//! Health of the system metrics (§8, "Metric threshold" row).

use super::{Assessment, Health, crossed, num, threshold};
use crate::config::{Levels, Thresholds};
use crate::model::{DiskUsage, MemoryInfo, SystemSnapshot};

/// Per-metric health of one system sample. `disks` and `temps` follow the snapshot's order.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemHealth {
    pub cpu: Assessment,
    pub load: Assessment,
    pub memory: Assessment,
    pub swap: Assessment,
    pub disks: Vec<Assessment>,
    pub temps: Vec<Assessment>,
    /// Mounts listed in `[system] disks` but absent from the sample (not mounted, or statvfs
    /// failed), each critical, in config order.
    pub missing_disks: Vec<(String, Assessment)>,
}

impl SystemHealth {
    /// `system.health`: the worst metric, with every non-healthy metric's message.
    pub fn overall(&self) -> Assessment {
        Assessment::combine(
            [&self.cpu, &self.load, &self.memory, &self.swap]
                .into_iter()
                .chain(&self.disks)
                .chain(self.missing_disks.iter().map(|(_, a)| a))
                .chain(&self.temps),
        )
    }

    /// The API reports memory and swap as one card, so it shows the worse of the two.
    pub fn memory_card(&self) -> Health {
        self.memory.health.worst_of(self.swap.health)
    }
}

/// Used memory in percent (`total - available`), or `None` if the total is 0.
pub fn memory_used_pct(m: &MemoryInfo) -> Option<f64> {
    (m.total > 0).then(|| m.total.saturating_sub(m.available) as f64 / m.total as f64 * 100.0)
}

/// Used swap in percent; 0 when the host has no swap.
pub fn swap_used_pct(m: &MemoryInfo) -> f64 {
    if m.swap_total == 0 {
        0.0
    } else {
        m.swap_total.saturating_sub(m.swap_free) as f64 / m.swap_total as f64 * 100.0
    }
}

/// Used space in percent as `df` shows it: `used / (used + avail)`. Blocks reserved for root count
/// as unavailable, because that is the point where ordinary services start failing to write.
/// `None` for a filesystem reporting no space at all.
pub fn disk_used_pct(d: &DiskUsage) -> Option<f64> {
    let usable = d.used.saturating_add(d.avail);
    (usable > 0).then(|| d.used as f64 / usable as f64 * 100.0)
}

/// The 1-minute load divided by the number of cores, which is what the load threshold is
/// expressed in. `None` if the core count is unknown (0).
pub fn load_per_core(load1: f64, cores: u32) -> Option<f64> {
    (cores > 0).then(|| load1 / f64::from(cores))
}

/// Evaluates one system sample.
///
/// `cpu_mean` and `load_mean` are the [`super::Window`] means of CPU usage (percent) and
/// [`load_per_core`] over their configured windows; every other metric uses the latest sample.
/// `None` means no sample yet (CPU needs two snapshots), which is `unknown`.
///
/// `expected_disks` is `[system] disks`. Each entry missing from the sample is critical. An empty
/// list means auto-detection, where nothing is expected and so nothing can be missing.
pub fn evaluate_system(
    t: &Thresholds,
    expected_disks: &[String],
    snap: &SystemSnapshot,
    cpu_mean: Option<f64>,
    load_mean: Option<f64>,
) -> SystemHealth {
    let cpu = match cpu_mean {
        None => Assessment::unknown("no CPU sample yet"),
        Some(v) => metric(v, levels(t.cpu.warning, t.cpu.critical), |h, l| {
            format!(
                "cpu at {}% {}",
                num(v),
                crossed(h, l, "%", Some(t.cpu.window))
            )
        }),
    };

    let load = match load_mean {
        None => Assessment::unknown("no load sample yet"),
        Some(v) => metric(v, levels(t.load.warning, t.load.critical), |h, l| {
            format!(
                "load {} per core {}",
                num(v),
                crossed(h, l, "", Some(t.load.window))
            )
        }),
    };

    let memory = match memory_used_pct(&snap.memory) {
        None => Assessment::unknown("memory total is 0"),
        Some(v) => metric(v, t.memory, |h, l| {
            format!("memory at {}% {}", num(v), crossed(h, l, "%", None))
        }),
    };

    let swap_pct = swap_used_pct(&snap.memory);
    let swap = metric(swap_pct, t.swap, |h, l| {
        format!("swap at {}% {}", num(swap_pct), crossed(h, l, "%", None))
    });

    let disks = snap
        .disks
        .iter()
        .map(|d| {
            let l = t.disk_mounts.get(&d.mount).copied().unwrap_or(t.disk);
            match disk_used_pct(d) {
                None => Assessment::unknown(format!("disk {} reports no space", d.mount)),
                Some(v) => metric(v, l, |h, l| {
                    format!(
                        "disk {} at {}% {}",
                        d.mount,
                        num(v),
                        crossed(h, l, "%", None)
                    )
                }),
            }
        })
        .collect();

    let temps = snap
        .temps
        .iter()
        .map(|r| {
            metric(r.celsius, t.temperature, |h, l| {
                format!(
                    "temperature {} at {}°C {}",
                    r.label,
                    num(r.celsius),
                    crossed(h, l, "°C", None)
                )
            })
        })
        .collect();

    let missing_disks = expected_disks
        .iter()
        .filter(|m| !snap.disks.iter().any(|d| &d.mount == *m))
        .map(|m| {
            let a = Assessment::new(Health::Critical, format!("disk {m} not mounted"));
            (m.clone(), a)
        })
        .collect();

    SystemHealth {
        cpu,
        load,
        memory,
        swap,
        disks,
        temps,
        missing_disks,
    }
}

fn levels(warning: f64, critical: f64) -> Levels {
    Levels { warning, critical }
}

/// Applies [`threshold`]; `describe` builds the message for warning/critical.
fn metric(value: f64, l: Levels, describe: impl FnOnce(Health, Levels) -> String) -> Assessment {
    match threshold(value, l) {
        Health::Healthy => Assessment::healthy(),
        Health::Unknown => Assessment::unknown("no valid reading"),
        h => Assessment::new(h, describe(h, l)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Thresholds;
    use crate::model::{CpuCounters, CpuTimes, TempReading};
    use Health::*;

    const GIB: u64 = 1 << 30;

    fn disk(mount: &str, used: u64, avail: u64) -> DiskUsage {
        DiskUsage {
            mount: mount.into(),
            fs_type: "ext4".into(),
            total: used + avail,
            used,
            avail,
        }
    }

    fn temp(label: &str, celsius: f64) -> TempReading {
        TempReading {
            label: label.into(),
            celsius,
        }
    }

    fn mem(total: u64, available: u64, swap_total: u64, swap_free: u64) -> MemoryInfo {
        MemoryInfo {
            total,
            available,
            swap_total,
            swap_free,
        }
    }

    /// A sample where everything is comfortably healthy.
    fn snap() -> SystemSnapshot {
        SystemSnapshot {
            ts: 1000,
            uptime_s: 100.0,
            cpu: CpuCounters {
                total: CpuTimes::default(),
                per_core: vec![],
            },
            load: [0.1, 0.1, 0.1],
            memory: mem(100, 50, 100, 100),
            disks: vec![disk("/", 50, 50)],
            net: vec![],
            temps: vec![temp("coretemp/Package id 0", 40.0)],
        }
    }

    fn eval(snap: &SystemSnapshot, cpu: Option<f64>, load: Option<f64>) -> SystemHealth {
        evaluate_system(&Thresholds::default(), &[], snap, cpu, load)
    }

    fn assert_assessment(a: &Assessment, health: Health, message: Option<&str>, case: &str) {
        assert_eq!(a.health, health, "{case}");
        assert_eq!(a.message.as_deref(), message, "{case}");
    }

    #[test]
    fn all_healthy() {
        let h = eval(&snap(), Some(10.0), Some(0.2));
        assert_eq!(h.overall(), Assessment::healthy());
        assert_eq!(h.memory_card(), Healthy);
    }

    // Defaults: cpu 85/95 over 2m.
    #[test]
    fn cpu_rows() {
        let cases: &[(Option<f64>, Health, Option<&str>)] = &[
            (Some(0.0), Healthy, None),
            (Some(84.99), Healthy, None),
            (
                Some(85.0),
                Warning,
                Some("cpu at 85% (warning ≥ 85%, 2m average)"),
            ),
            (
                Some(94.99),
                Warning,
                Some("cpu at 94.9% (warning ≥ 85%, 2m average)"),
            ),
            (
                Some(95.0),
                Critical,
                Some("cpu at 95% (critical ≥ 95%, 2m average)"),
            ),
            (None, Unknown, Some("no CPU sample yet")),
            (Some(f64::NAN), Unknown, Some("no valid reading")),
        ];
        for (v, health, msg) in cases {
            let h = eval(&snap(), *v, Some(0.1));
            assert_assessment(&h.cpu, *health, *msg, &format!("cpu {v:?}"));
            assert_eq!(h.overall().health, *health, "overall for cpu {v:?}");
        }
    }

    // Defaults: load 1.5/3.0 per core over 5m.
    #[test]
    fn load_rows() {
        let cases: &[(Option<f64>, Health, Option<&str>)] = &[
            (Some(1.49), Healthy, None),
            (
                Some(1.5),
                Warning,
                Some("load 1.5 per core (warning ≥ 1.5, 5m average)"),
            ),
            (
                Some(3.0),
                Critical,
                Some("load 3 per core (critical ≥ 3, 5m average)"),
            ),
            (None, Unknown, Some("no load sample yet")),
        ];
        for (v, health, msg) in cases {
            let h = eval(&snap(), Some(1.0), *v);
            assert_assessment(&h.load, *health, *msg, &format!("load {v:?}"));
        }
    }

    #[test]
    fn load_per_core_divides_by_cores() {
        assert_eq!(load_per_core(6.0, 4), Some(1.5));
        assert_eq!(load_per_core(1.0, 0), None);
    }

    // Defaults: memory 85/95.
    #[test]
    fn memory_rows() {
        let cases: &[(MemoryInfo, Health, Option<&str>)] = &[
            (mem(100, 16, 0, 0), Healthy, None),
            (
                mem(100, 15, 0, 0),
                Warning,
                Some("memory at 85% (warning ≥ 85%)"),
            ),
            (
                mem(100, 5, 0, 0),
                Critical,
                Some("memory at 95% (critical ≥ 95%)"),
            ),
            (
                mem(100, 0, 0, 0),
                Critical,
                Some("memory at 100% (critical ≥ 95%)"),
            ),
            (mem(0, 0, 0, 0), Unknown, Some("memory total is 0")),
        ];
        for (m, health, msg) in cases {
            let mut s = snap();
            s.memory = *m;
            let h = eval(&s, Some(1.0), Some(0.1));
            assert_assessment(&h.memory, *health, *msg, &format!("{m:?}"));
            assert_eq!(h.memory_card(), *health, "card {m:?}");
        }
    }

    // Defaults: swap 50/90.
    #[test]
    fn swap_rows() {
        let cases: &[(MemoryInfo, Health, Option<&str>)] = &[
            (mem(100, 100, 0, 0), Healthy, None), // no swap at all
            (mem(100, 100, 100, 51), Healthy, None),
            (
                mem(100, 100, 100, 50),
                Warning,
                Some("swap at 50% (warning ≥ 50%)"),
            ),
            (
                mem(100, 100, 100, 10),
                Critical,
                Some("swap at 90% (critical ≥ 90%)"),
            ),
        ];
        for (m, health, msg) in cases {
            let mut s = snap();
            s.memory = *m;
            let h = eval(&s, Some(1.0), Some(0.1));
            assert_assessment(&h.swap, *health, *msg, &format!("{m:?}"));
            assert_eq!(h.memory_card(), *health, "card {m:?}");
        }
    }

    // Defaults: disk 80/90; /srv overridden to 90/95.
    #[test]
    fn disk_rows_with_mount_override() {
        let mut t = Thresholds::default();
        t.disk_mounts.insert(
            "/srv".into(),
            Levels {
                warning: 90.0,
                critical: 95.0,
            },
        );
        let cases: &[(DiskUsage, Health, Option<&str>)] = &[
            (disk("/", 79, 21), Healthy, None),
            (
                disk("/", 84, 16),
                Warning,
                Some("disk / at 84% (warning ≥ 80%)"),
            ),
            (
                disk("/", 90, 10),
                Critical,
                Some("disk / at 90% (critical ≥ 90%)"),
            ),
            (disk("/srv", 85, 15), Healthy, None),
            (
                disk("/srv", 90, 10),
                Warning,
                Some("disk /srv at 90% (warning ≥ 90%)"),
            ),
            (
                disk("/srv", 96, 4),
                Critical,
                Some("disk /srv at 96% (critical ≥ 95%)"),
            ),
            (
                disk("/empty", 0, 0),
                Unknown,
                Some("disk /empty reports no space"),
            ),
        ];
        for (d, health, msg) in cases {
            let mut s = snap();
            s.disks = vec![d.clone()];
            let h = evaluate_system(&t, &[], &s, Some(1.0), Some(0.1));
            assert_assessment(&h.disks[0], *health, *msg, &d.mount);
        }
    }

    #[test]
    fn disk_pct_excludes_reserved_blocks() {
        // 100 GiB fs, 5 GiB reserved for root: 80 used + 15 avail = 84.2% as df shows it.
        let d = DiskUsage {
            mount: "/".into(),
            fs_type: "ext4".into(),
            total: 100 * GIB,
            used: 80 * GIB,
            avail: 15 * GIB,
        };
        let pct = disk_used_pct(&d).expect("non-empty");
        assert!((pct - 84.21).abs() < 0.01, "{pct}");
    }

    // Defaults: temperature 75/90.
    #[test]
    fn temperature_rows() {
        let label = "coretemp/Package id 0";
        let cases: &[(f64, Health, Option<&str>)] = &[
            (74.9, Healthy, None),
            (
                75.0,
                Warning,
                Some("temperature coretemp/Package id 0 at 75°C (warning ≥ 75°C)"),
            ),
            (
                91.5,
                Critical,
                Some("temperature coretemp/Package id 0 at 91.5°C (critical ≥ 90°C)"),
            ),
            (f64::NAN, Unknown, Some("no valid reading")),
        ];
        for (c, health, msg) in cases {
            let mut s = snap();
            s.temps = vec![temp(label, *c)];
            let h = eval(&s, Some(1.0), Some(0.1));
            assert_assessment(&h.temps[0], *health, *msg, &format!("{c}°C"));
        }
    }

    #[test]
    fn no_sensors_and_no_disks_are_fine() {
        let mut s = snap();
        s.temps.clear();
        s.disks.clear();
        let h = eval(&s, Some(1.0), Some(0.1));
        assert!(h.temps.is_empty() && h.disks.is_empty());
        assert_eq!(h.overall(), Assessment::healthy());
    }

    #[test]
    fn overall_is_worst_with_all_messages() {
        let mut s = snap();
        s.disks = vec![disk("/", 84, 16), disk("/data", 95, 5)];
        s.memory = mem(100, 10, 0, 0);
        let h = eval(&s, None, Some(0.1));
        let o = h.overall();
        assert_eq!(o.health, Critical);
        assert_eq!(
            o.message.as_deref(),
            Some(
                "disk /data at 95% (critical ≥ 90%); memory at 90% (warning ≥ 85%); \
                 disk / at 84% (warning ≥ 80%); no CPU sample yet"
            )
        );
    }

    #[test]
    fn overall_unknown_when_only_cpu_missing() {
        let h = eval(&snap(), None, Some(0.1));
        assert_eq!(h.overall().health, Unknown);
    }

    #[test]
    fn configured_disks_must_be_present() {
        let mut s = snap(); // has "/" only
        s.disks.push(disk("/srv", 10, 90));
        let expected = |m: &[&str]| m.iter().map(|m| (*m).to_owned()).collect::<Vec<_>>();
        let cases: &[(&[&str], &[&str])] = &[
            (&[], &[]), // auto-detect: nothing expected
            (&["/", "/srv"], &[]),
            (&["/srv", "/backup"], &["/backup"]),
            (&["/data", "/", "/backup"], &["/data", "/backup"]),
        ];
        for (configured, missing) in cases {
            let h = evaluate_system(
                &Thresholds::default(),
                &expected(configured),
                &s,
                Some(1.0),
                Some(0.1),
            );
            let got: Vec<&str> = h.missing_disks.iter().map(|(m, _)| m.as_str()).collect();
            assert_eq!(got, *missing, "{configured:?}");
            for (m, a) in &h.missing_disks {
                assert_assessment(a, Critical, Some(&format!("disk {m} not mounted")), m);
            }
        }
    }

    #[test]
    fn missing_disk_makes_the_system_critical() {
        let h = evaluate_system(
            &Thresholds::default(),
            &["/srv".to_owned()],
            &snap(),
            Some(1.0),
            Some(0.1),
        );
        assert_eq!(
            h.overall(),
            Assessment::new(Critical, "disk /srv not mounted")
        );
    }
}
