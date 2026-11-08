//! Contract test (ARCHITECTURE §6/§11): every `docs/api/examples/*.json` file deserializes into
//! its DTO and serializes back to the same JSON value, so no field is lost, renamed or omitted.

#![allow(clippy::unwrap_used)] // test code

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use gargoyle_core::api::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/api/examples")
}

fn round_trip<T: DeserializeOwned + Serialize>(file: &str) {
    let path = examples_dir().join(file);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let original: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
    let dto: T = serde_json::from_value(original.clone())
        .unwrap_or_else(|e| panic!("{file} does not match {}: {e}", std::any::type_name::<T>()));
    let again = serde_json::to_value(&dto).unwrap();
    assert_eq!(
        again,
        original,
        "{file} changed after a round trip through {}",
        std::any::type_name::<T>()
    );
}

/// File name → round-trip function. Every example file must be listed here.
type Case = (&'static str, fn(&str));

const CASES: &[Case] = &[
    ("health.json", round_trip::<HealthResponse>),
    ("error.json", round_trip::<ApiError>),
    ("overview.json", round_trip::<Overview>),
    ("services.json", round_trip::<ServiceList>),
    ("service-detail.json", round_trip::<ServiceDetail>),
    ("timers.json", round_trip::<TimerList>),
    ("timer-detail.json", round_trip::<TimerDetail>),
    ("checks.json", round_trip::<CheckList>),
    ("check-detail.json", round_trip::<CheckDetail>),
    ("metrics.json", round_trip::<MetricList>),
    ("metrics-query.json", round_trip::<MetricsQuery>),
    ("events.json", round_trip::<EventList>),
    ("library.json", round_trip::<LibraryResponse>),
    ("library-unconfigured.json", round_trip::<LibraryResponse>),
];

#[test]
fn every_example_round_trips() {
    for (file, check) in CASES {
        check(file);
    }
}

#[test]
fn every_example_file_is_covered() {
    let on_disk: BTreeSet<String> = fs::read_dir(examples_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".json"))
        .collect();
    let covered: BTreeSet<String> = CASES.iter().map(|(f, _)| (*f).to_owned()).collect();
    assert_eq!(on_disk, covered, "add new example files to CASES");
}

#[test]
fn optional_fields_serialize_as_null() {
    let check = CheckSummary {
        name: "x".into(),
        kind: CheckKind::Tcp,
        target: "127.0.0.1:1".into(),
        health: gargoyle_core::Health::Unknown,
        message: None,
        last_check: None,
        latency_ms: None,
        consecutive_failures: 0,
    };
    let v = serde_json::to_value(&check).unwrap();
    for key in ["message", "lastCheck", "latencyMs"] {
        assert_eq!(
            v.get(key),
            Some(&Value::Null),
            "{key} must be present as null"
        );
    }
}
