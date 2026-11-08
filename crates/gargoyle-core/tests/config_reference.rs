//! docs/CONFIG.md must document exactly the keys the config structs accept (ROADMAP M6.4).
//!
//! The key list is not maintained by hand: it is read from the serde derives themselves. A
//! probing `Deserializer` asks `Config::deserialize` for its struct fields (serde passes them to
//! `deserialize_struct`, renames applied) and walks down one key at a time into nested structs,
//! sequences (`[[service]]`), options and maps (`disk_mounts`, shown as `*`). A new field
//! anywhere in the config therefore fails this test until CONFIG.md has a row for it, and a row
//! for a removed field fails it too.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use gargoyle_core::config::Config;
use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, forward_to_deserialize_any};

/// Placeholder for a map key (a mount point in `thresholds.disk_mounts`).
const MAP_KEY: &str = "*";

#[derive(Debug)]
struct Stop;

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("probe stopped")
    }
}

impl std::error::Error for Stop {}

impl de::Error for Stop {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Stop
    }
}

/// Stands for the value at the end of `route` (relative to where it is used). At the end of the
/// route it records the keys that value has, if any; every scalar ends the probe with `Stop`.
#[derive(Clone, Copy)]
struct Probe<'a> {
    route: &'a [String],
    keys: &'a RefCell<Vec<String>>,
}

impl<'de> de::Deserializer<'de> for Probe<'_> {
    type Error = Stop;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Stop> {
        Err(Stop)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        match self.route.split_first() {
            None => {
                self.keys
                    .borrow_mut()
                    .extend(fields.iter().map(|f| f.to_string()));
                Err(Stop)
            }
            Some((key, rest)) => visitor.visit_map(OneEntry::new(key, rest, self.keys)),
        }
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        match self.route.split_first() {
            None => {
                self.keys.borrow_mut().push(MAP_KEY.to_owned());
                Err(Stop)
            }
            Some((_, rest)) => visitor.visit_map(OneEntry::new("/probe", rest, self.keys)),
        }
    }

    /// Sequences are transparent: `[[service]]` entries have the keys `service.unit`, ...
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        visitor.visit_seq(OneElement(Some(self)))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Stop> {
        visitor.visit_newtype_struct(self)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit
        unit_struct tuple tuple_struct enum identifier ignored_any
    }
}

/// A map with a single entry: `key` and a probe for the rest of the route.
struct OneEntry<'a> {
    key: Option<&'a str>,
    value: Probe<'a>,
}

impl<'a> OneEntry<'a> {
    fn new(key: &'a str, route: &'a [String], keys: &'a RefCell<Vec<String>>) -> Self {
        Self {
            key: Some(key),
            value: Probe { route, keys },
        }
    }
}

impl<'de> MapAccess<'de> for OneEntry<'_> {
    type Error = Stop;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Stop> {
        self.key
            .take()
            .map(|k| seed.deserialize(k.into_deserializer()))
            .transpose()
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Stop> {
        seed.deserialize(self.value)
    }
}

struct OneElement<'a>(Option<Probe<'a>>);

impl<'de> SeqAccess<'de> for OneElement<'_> {
    type Error = Stop;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Stop> {
        self.0.take().map(|p| seed.deserialize(p)).transpose()
    }
}

/// The keys directly below `route` (empty for a scalar).
fn keys_below(route: &[String]) -> Vec<String> {
    let keys = RefCell::new(Vec::new());
    // The probe always ends in `Stop`; only what it recorded on the way matters.
    let _ = Config::deserialize(Probe { route, keys: &keys });
    keys.into_inner()
}

/// Every scalar key as a dotted path, e.g. `server.listen` or `thresholds.disk_mounts.*.warning`.
fn config_keys() -> BTreeSet<String> {
    let mut leaves = BTreeSet::new();
    let mut todo = vec![Vec::new()];
    while let Some(route) = todo.pop() {
        let below = keys_below(&route);
        if below.is_empty() {
            leaves.insert(route.join("."));
        }
        for key in below {
            let mut child = route.clone();
            child.push(key);
            todo.push(child);
        }
    }
    leaves
}

/// The first column of every table row whose first cell is a single code span: `` | `key` | ``.
fn documented_keys(markdown: &str) -> BTreeSet<String> {
    markdown
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once("` |"))
        .map(|(key, _)| key.to_owned())
        .filter(|key| !key.contains('`'))
        .collect()
}

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn probe_finds_known_keys() {
    let keys = config_keys();
    for key in [
        "server.listen",
        "auth.password_hash_file",
        "thresholds.cpu.window",
        "thresholds.disk_mounts.*.critical",
        "service.unit",
        "timer.max_age",
        "check.latency_warning",
        "system.disks",
    ] {
        assert!(keys.contains(key), "{key} missing from {keys:#?}");
    }
    assert!(!keys.contains("server"), "sections are not leaf keys");
}

#[test]
fn config_md_documents_every_key() {
    let documented = documented_keys(&repo_file("docs/CONFIG.md"));
    let keys = config_keys();
    let missing: Vec<_> = keys.difference(&documented).collect();
    let unknown: Vec<_> = documented.difference(&keys).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "docs/CONFIG.md is out of date.\n  keys without a `| `key` |` row: {missing:?}\n  \
         rows for keys the config does not have: {unknown:?}"
    );
}

/// The complete example in CONFIG.md (the first ```toml block) must be a valid config.
#[test]
fn config_md_example_is_valid() {
    let md = repo_file("docs/CONFIG.md");
    let start = md
        .find("```toml\n")
        .expect("CONFIG.md has a ```toml example")
        + 8;
    let len = md[start..]
        .find("```")
        .expect("the example block is closed");
    if let Err(e) = Config::from_toml_str(&md[start..start + len]) {
        panic!(
            "CONFIG.md example: {e}: {:?}",
            std::error::Error::source(&e)
        );
    }
}
