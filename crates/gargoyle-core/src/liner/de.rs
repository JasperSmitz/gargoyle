//! Bounded deserializers. The report file is capped at 32 MiB, but a short JSON value can become
//! a much larger Rust value (`""` is 3 bytes of input and a 24-byte `String`). Long arrays are
//! therefore cut while parsing, not after, so memory stays proportional to what is kept.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::Issue;
use super::view::MAX_ISSUES;
use crate::api::Severity;

/// Distinct issue codes accepted in `summary.issues_by_code`. Liner has a few dozen rules; a
/// report with more is not one Liner wrote.
pub const MAX_CODES: usize = 1000;

/// The first `N` elements of a JSON array and how many it had. The rest are skipped without being
/// stored or type-checked.
#[derive(Debug)]
pub(super) struct Capped<T, const N: usize> {
    pub kept: Vec<T>,
    pub total: u64,
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Capped<T, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T, const N: usize>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for V<T, N> {
            type Value = Capped<T, N>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut kept = Vec::new();
                while kept.len() < N {
                    match seq.next_element()? {
                        Some(item) => kept.push(item),
                        None => break,
                    }
                }
                let mut total = kept.len() as u64;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    total += 1;
                }
                Ok(Capped { kept, total })
            }
        }

        d.deserialize_seq(V(PhantomData))
    }
}

/// `issues[]`, cut to the [`MAX_ISSUES`] that `build_view` keeps: the worst first, in report
/// order within a severity (what a stable sort by severity and a truncation would keep).
#[derive(Debug)]
pub(super) struct WorstIssues {
    /// Worst first.
    pub kept: Vec<Issue>,
    pub total: u64,
}

impl<'de> Deserialize<'de> for WorstIssues {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = WorstIssues;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of issues")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut by_severity: BTreeMap<Reverse<Severity>, Vec<Issue>> = BTreeMap::new();
                let (mut kept, mut total) = (0usize, 0u64);
                while let Some(issue) = seq.next_element::<Issue>()? {
                    total += 1;
                    by_severity
                        .entry(Reverse(issue.severity))
                        .or_default()
                        .push(issue);
                    if kept < MAX_ISSUES {
                        kept += 1;
                    } else if let Some(mut mildest) = by_severity.last_entry() {
                        // The latest issue of the mildest severity is the one a stable sort
                        // puts last; it may be the one just added.
                        mildest.get_mut().pop();
                        if mildest.get().is_empty() {
                            mildest.remove();
                        }
                    }
                }
                Ok(WorstIssues {
                    kept: by_severity.into_values().flatten().collect(),
                    total,
                })
            }
        }

        d.deserialize_seq(V)
    }
}

/// `summary.issues_by_code`, refused beyond [`MAX_CODES`] distinct codes.
pub(super) fn codes<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, u64>, D::Error> {
    struct V;

    impl<'de> Visitor<'de> for V {
        type Value = BTreeMap<String, u64>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a map of issue codes to counts")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut codes = BTreeMap::new();
            while let Some((code, count)) = map.next_entry::<String, u64>()? {
                if codes.len() >= MAX_CODES && !codes.contains_key(&code) {
                    return Err(de::Error::custom(format_args!(
                        "more than {MAX_CODES} issue codes"
                    )));
                }
                codes.insert(code, count);
            }
            Ok(codes)
        }
    }

    d.deserialize_map(V)
}

/// `schema_version` as the version probe sees it: a string, or anything else. Other values are
/// skipped rather than built (a `serde_json::Value` would turn `[[],[],…]` into far more memory
/// than the input).
#[derive(Debug, PartialEq)]
pub(super) enum ProbedVersion {
    Text(String),
    Other,
}

impl<'de> Deserialize<'de> for ProbedVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = ProbedVersion;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any value")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Text(v.to_owned()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Text(v))
            }

            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Other)
            }

            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Other)
            }

            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Other)
            }

            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Other)
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(ProbedVersion::Other)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(ProbedVersion::Other)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(ProbedVersion::Other)
            }
        }

        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_keeps_the_first_n_and_counts_all() {
        let c: Capped<String, 2> = serde_json::from_str(r#"["a","b","c",1,{}]"#).unwrap();
        assert_eq!((c.kept, c.total), (vec!["a".to_owned(), "b".to_owned()], 5));
        let c: Capped<String, 2> = serde_json::from_str("[]").unwrap();
        assert_eq!((c.kept.len(), c.total), (0, 0));
        // The kept part is still type-checked.
        assert!(serde_json::from_str::<Capped<String, 2>>("[1]").is_err());
    }

    #[test]
    fn too_many_codes_are_refused() {
        let map = |n: usize| {
            let entries: Vec<String> = (0..n).map(|i| format!("\"c{i}\":1")).collect();
            format!("{{{}}}", entries.join(","))
        };
        let parse = |text: &str| codes(&mut serde_json::Deserializer::from_str(text));
        assert_eq!(parse(&map(MAX_CODES)).unwrap().len(), MAX_CODES);
        let e = parse(&map(MAX_CODES + 1)).unwrap_err();
        assert!(e.to_string().contains("more than 1000 issue codes"), "{e}");
    }

    #[test]
    fn probed_versions() {
        let probe = |text: &str| serde_json::from_str::<ProbedVersion>(text).unwrap();
        assert_eq!(probe(r#""1.0.0""#), ProbedVersion::Text("1.0.0".into()));
        for other in [
            "1",
            "-1",
            "1.5",
            "true",
            "null",
            "[[],[1]]",
            r#"{"a":[{}]}"#,
        ] {
            assert_eq!(probe(other), ProbedVersion::Other, "{other}");
        }
    }
}
