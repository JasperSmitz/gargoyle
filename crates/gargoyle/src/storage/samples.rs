//! Turning collected data into stored series points (ARCHITECTURE §5, series key table).
//!
//! Keys are `<metric>[:<label>]` and every metric has one fixed unit. These are pure functions so
//! the scheduler and the mock backfill produce exactly the same series.

use gargoyle_core::health::{disk_used_pct, memory_used_pct, swap_used_pct};
use gargoyle_core::model::{SystemSnapshot, UnitSnapshot};
use gargoyle_core::rates::{CpuUsage, NetRate};

pub const PERCENT: &str = "percent";
pub const LOAD: &str = "load";
pub const BYTES_PER_S: &str = "bytes_per_s";
pub const CELSIUS: &str = "celsius";
pub const BYTES: &str = "bytes";

/// One value of one series at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub key: String,
    pub unit: &'static str,
    /// Unix seconds.
    pub ts: i64,
    pub value: f64,
}

/// The points of one system tick. CPU and network values need a previous sample, so they are
/// missing on the first tick and after a counter reset.
pub fn system(snap: &SystemSnapshot, cpu: Option<CpuUsage>, net: &[NetRate]) -> Vec<Sample> {
    let mut out = Vec::with_capacity(8 + snap.disks.len() + 2 * net.len() + snap.temps.len());
    let mut push = |key: String, unit, value: Option<f64>| {
        // NaN cannot be stored (REAL NOT NULL), and ±inf would poison every average it touches.
        if let Some(value) = value.filter(|v| v.is_finite()) {
            out.push(Sample {
                key,
                unit,
                ts: snap.ts,
                value,
            });
        }
    };
    push("cpu.usage".into(), PERCENT, cpu.map(|c| c.usage));
    push("cpu.iowait".into(), PERCENT, cpu.map(|c| c.iowait));
    for (name, value) in ["load.1", "load.5", "load.15"].into_iter().zip(snap.load) {
        push(name.into(), LOAD, Some(value));
    }
    push("mem.used".into(), PERCENT, memory_used_pct(&snap.memory));
    push(
        "swap.used".into(),
        PERCENT,
        Some(swap_used_pct(&snap.memory)),
    );
    for d in &snap.disks {
        push(format!("disk.used:{}", d.mount), PERCENT, disk_used_pct(d));
    }
    for n in net {
        push(format!("net.rx:{}", n.iface), BYTES_PER_S, Some(n.rx_bps));
        push(format!("net.tx:{}", n.iface), BYTES_PER_S, Some(n.tx_bps));
    }
    for t in &snap.temps {
        push(format!("temp:{}", t.label), CELSIUS, Some(t.celsius));
    }
    out
}

/// Memory of the configured services in one unit tick, for units that report it.
pub fn services(ts: i64, units: &[UnitSnapshot], is_service: impl Fn(&str) -> bool) -> Vec<Sample> {
    units
        .iter()
        .filter(|u| is_service(&u.unit))
        .filter_map(|u| {
            Some(Sample {
                key: format!("service.memory:{}", u.unit),
                unit: BYTES,
                ts,
                value: u.memory_current? as f64,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use gargoyle_collector::MockCollector;
    use gargoyle_core::rates::system_rates;

    use super::*;

    #[test]
    fn keys_and_units_follow_the_series_table() {
        let mock = MockCollector::with_boot_time(1, 1_000);
        let a = mock.snapshot_at(100_000);
        let b = mock.snapshot_at(100_010);
        let rates = system_rates(Some(&a), &b);
        let samples = system(&b, rates.cpu, &rates.net);
        let pairs: Vec<(&str, &str)> = samples.iter().map(|s| (s.key.as_str(), s.unit)).collect();
        for expected in [
            ("cpu.usage", PERCENT),
            ("cpu.iowait", PERCENT),
            ("load.1", LOAD),
            ("load.5", LOAD),
            ("load.15", LOAD),
            ("mem.used", PERCENT),
            ("swap.used", PERCENT),
            ("disk.used:/", PERCENT),
            ("disk.used:/srv", PERCENT),
            ("temp:coretemp/Package id 0", CELSIUS),
        ] {
            assert!(pairs.contains(&expected), "{expected:?} in {pairs:?}");
        }
        assert!(
            pairs
                .iter()
                .any(|(k, u)| k.starts_with("net.rx:") && *u == BYTES_PER_S)
        );
        assert!(samples.iter().all(|s| s.ts == 100_010));
    }

    #[test]
    fn first_sample_has_no_rates_and_non_finite_values_are_dropped() {
        let mock = MockCollector::with_boot_time(1, 1_000);
        let mut snap = mock.snapshot_at(100_000);
        snap.load[0] = f64::NAN;
        let keys: Vec<String> = system(&snap, None, &[])
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(!keys.iter().any(|k| k.starts_with("cpu.") || k == "load.1"));
        assert!(keys.contains(&"load.5".to_owned()));
    }

    #[test]
    fn service_memory_only_for_configured_services_with_memory() {
        let mock = MockCollector::with_boot_time(1, 1_000);
        let reqs = ["a.service", "b.service"].map(gargoyle_core::model::UnitRequest::new);
        let mut units = mock.units_at(&reqs, 5_000);
        units[1].memory_current = None;
        let s = services(5_000, &units, |u| u == "a.service" || u == "b.service");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].key, "service.memory:a.service");
        assert_eq!(s[0].unit, BYTES);
        assert!(services(5_000, &units, |_| false).is_empty());
    }
}
