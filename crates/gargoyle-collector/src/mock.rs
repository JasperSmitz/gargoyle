//! Deterministic fake host for development and tests (ARCHITECTURE §4, `collector.mode = "mock"`).
//!
//! Gauges (memory, load, disks, temperatures) are pure functions of `(seed, ts)`: slow sine waves
//! around plausible baselines plus seeded noise. CPU and network counters must be cumulative and
//! monotonic, so they are integrated from the same kind of curve and kept in a small mutex-guarded
//! state. Given the same seed and the same sequence of sample times, the output is identical.
//!
//! Unit states are derived from a hash of the unit name only, so a given name always shows the
//! same state; names containing `fail` are always failed.

use std::f64::consts::TAU;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    CollectError, Collector, CpuCounters, CpuTimes, DiskUsage, HostInfo, MemoryInfo, NetCounters,
    SystemSnapshot, TempReading, UnitRequest, UnitSnapshot,
};

const CORES: usize = 4;
/// `USER_HZ`: jiffies per second per core.
const HZ: f64 = 100.0;
/// CPU and network rates are constant within a slot; one slot is one default system interval.
const SLOT_S: i64 = 10;
/// Gaps longer than this are filled with the average rate instead of slot by slot.
const MAX_STEPPED_GAP_S: i64 = 8 * 24 * 3600;
const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
const HOUR: f64 = 3600.0;
const DAY: f64 = 24.0 * HOUR;

/// Mean CPU usage (percent) and network rates (bytes/s) used for gap filling.
const AVG_CPU_PCT: f64 = 18.0;
const AVG_IOWAIT_PCT: f64 = 1.0;
const AVG_RX_BPS: f64 = 60_000.0;
const AVG_TX_BPS: f64 = 12_000.0;

pub struct MockCollector {
    seed: u64,
    boot_time: i64,
    counters: Mutex<Counters>,
}

/// Fractional accumulators; snapshots report them floored, which stays monotonic.
#[derive(Debug, Default)]
struct Counters {
    last_ts: Option<i64>,
    cores: [CoreAcc; CORES],
    rx: f64,
    tx: f64,
}

#[derive(Debug, Default, Clone, Copy)]
struct CoreAcc {
    user: f64,
    system: f64,
    softirq: f64,
    idle: f64,
    iowait: f64,
}

impl MockCollector {
    /// A mock host that booted a few days before now.
    pub fn new(seed: u64) -> Self {
        let boot_time = now() - 3 * 86_400 - (mix(seed, 1) % 86_400) as i64;
        Self::with_boot_time(seed, boot_time)
    }

    /// A mock host with a fixed boot time, for reproducible tests and history backfill.
    pub fn with_boot_time(seed: u64, boot_time: i64) -> Self {
        Self {
            seed,
            boot_time,
            counters: Mutex::new(Counters::default()),
        }
    }

    /// The system sample at `ts` (unix seconds). Counters advance from the previous call; a `ts`
    /// earlier than the previous one returns the counters unchanged.
    pub fn snapshot_at(&self, ts: i64) -> SystemSnapshot {
        let (cpu, net) = self.advance(ts);
        let total_mem = 16 * GIB;
        let mem_used = 0.35 + 0.08 * wave(ts, DAY, 0.0) + 0.02 * self.noise(10, ts / SLOT_S);
        let swap_total = 2 * GIB;
        let swap_used = 0.04 + 0.02 * wave(ts, DAY, 0.3);

        let usage = self.cpu_pct(ts);
        let load = |lag: i64| {
            (CORES as f64 * self.cpu_pct(ts - lag) / 100.0 * 1.2
                + 0.1 * self.noise(11, ts / SLOT_S))
            .max(0.0)
        };

        SystemSnapshot {
            ts,
            uptime_s: (ts - self.boot_time).max(0) as f64,
            cpu,
            load: [load(0), load(150), load(450)],
            memory: MemoryInfo {
                total: total_mem,
                available: frac(total_mem, 1.0 - mem_used),
                swap_total,
                swap_free: frac(swap_total, 1.0 - swap_used),
            },
            disks: vec![
                disk(
                    "/",
                    "ext4",
                    250_000_000_000,
                    0.55 + 0.01 * wave(ts, DAY, 0.0),
                ),
                disk(
                    "/srv",
                    "ext4",
                    2_000_000_000_000,
                    0.76 + 0.05 * wave(ts, DAY, 0.5),
                ),
            ],
            net,
            temps: vec![
                TempReading {
                    label: "coretemp/Package id 0".into(),
                    celsius: round1(40.0 + usage * 0.35 + self.noise(12, ts / SLOT_S)),
                },
                TempReading {
                    label: "nvme/Composite".into(),
                    celsius: round1(36.0 + 3.0 * wave(ts, DAY, 0.2)),
                },
            ],
        }
    }

    /// Unit snapshots as seen at `now` (unix seconds).
    pub fn units_at(&self, units: &[UnitRequest], now: i64) -> Vec<UnitSnapshot> {
        let mut out = Vec::with_capacity(units.len());
        for req in units {
            if let Some(stem) = req.unit.strip_suffix(".timer") {
                let service = req
                    .service
                    .clone()
                    .unwrap_or_else(|| format!("{stem}.service"));
                let (timer, svc) = self.timer(&req.unit, &service, now);
                out.push(timer);
                out.push(svc);
            } else {
                out.push(self.service(&req.unit, now));
            }
        }
        out
    }

    /// Integrates CPU and network counters up to `ts` and returns them.
    fn advance(&self, ts: i64) -> (CpuCounters, Vec<NetCounters>) {
        let mut c = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        let from = c.last_ts.unwrap_or(self.boot_time);
        if ts > from {
            let stepped_from = from.max(ts - MAX_STEPPED_GAP_S);
            self.add_average(&mut c, (stepped_from - from) as f64);
            let mut t = stepped_from;
            while t < ts {
                let slot_end = (t.div_euclid(SLOT_S) + 1) * SLOT_S;
                let end = slot_end.min(ts);
                self.add_slot(&mut c, t, (end - t) as f64);
                t = end;
            }
        }
        c.last_ts = Some(ts.max(c.last_ts.unwrap_or(ts)));

        let per_core: Vec<CpuTimes> = c.cores.iter().map(CoreAcc::times).collect();
        let total = per_core.iter().fold(CpuTimes::default(), |a, b| CpuTimes {
            user: a.user + b.user,
            system: a.system + b.system,
            softirq: a.softirq + b.softirq,
            idle: a.idle + b.idle,
            iowait: a.iowait + b.iowait,
            ..a
        });
        let net = vec![NetCounters {
            iface: "enp1s0".into(),
            rx_bytes: c.rx as u64,
            tx_bytes: c.tx as u64,
        }];
        (CpuCounters { total, per_core }, net)
    }

    fn add_average(&self, c: &mut Counters, secs: f64) {
        if secs <= 0.0 {
            return;
        }
        for core in &mut c.cores {
            core.add(AVG_CPU_PCT, AVG_IOWAIT_PCT, secs * HZ);
        }
        c.rx += AVG_RX_BPS * secs;
        c.tx += AVG_TX_BPS * secs;
    }

    /// Adds `secs` seconds at the rates of the slot containing `t`.
    fn add_slot(&self, c: &mut Counters, t: i64, secs: f64) {
        let slot = t.div_euclid(SLOT_S);
        let usage = self.cpu_pct(t);
        for (i, core) in c.cores.iter_mut().enumerate() {
            // Spread load unevenly across cores while keeping the mean at `usage`.
            let skew = 1.0 + 0.3 * self.noise(20 + i as u64, slot);
            let core_usage = (usage * skew).clamp(0.0, 100.0);
            let iowait = (AVG_IOWAIT_PCT + 0.8 * self.noise(30 + i as u64, slot)).max(0.0);
            core.add(core_usage, iowait, secs * HZ);
        }
        let traffic = 1.0 + 0.6 * wave(t, HOUR, 0.1) + 0.3 * self.noise(40, slot);
        c.rx += AVG_RX_BPS * traffic.max(0.0) * secs;
        c.tx += AVG_TX_BPS * traffic.max(0.0) * secs;
    }

    /// CPU usage in percent at `t`: daily and hourly waves, noise, and a spike to ~97% for the
    /// first 3 minutes of roughly one in twelve 10-minute blocks (so ~every 2h), long enough to
    /// cross the default 2-minute window.
    fn cpu_pct(&self, t: i64) -> f64 {
        let block = t.div_euclid(600);
        if mix(self.seed, block as u64) % 12 == 0 && t.rem_euclid(600) < 180 {
            return 97.0;
        }
        let v = AVG_CPU_PCT
            + 8.0 * wave(t, DAY, 0.0)
            + 6.0 * wave(t, HOUR, 0.25)
            + 4.0 * self.noise(1, t.div_euclid(SLOT_S));
        v.clamp(1.0, 100.0)
    }

    /// Seeded noise in [-1, 1] for a `(stream, slot)` pair.
    fn noise(&self, stream: u64, slot: i64) -> f64 {
        let h = mix(
            self.seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            slot as u64,
        );
        (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }

    fn service(&self, unit: &str, now: i64) -> UnitSnapshot {
        let h = fnv1a(unit);
        let kind = if unit.contains("fail") {
            Kind::Failed
        } else {
            match h % 10 {
                // The order is tuned so `config/gargoyle.dev.toml` shows healthy, warning
                // (nginx: activating) and critical services.
                0..=5 => Kind::Running,
                6 => Kind::Failed,
                7 => Kind::Inactive,
                8 => Kind::Activating,
                _ => Kind::NotFound,
            }
        };
        let started = self.boot_time + 30 + (h % 600) as i64;
        let base = UnitSnapshot {
            description: describe(unit),
            invocation_id: Some(format!("{h:016x}{:016x}", mix(h, 1))),
            exec_main_start_ts: Some(started),
            active_enter_ts: Some(started),
            n_restarts: Some((h % 3) as u32),
            result: Some("success".into()),
            exec_main_status: Some(0),
            ..unit_snapshot(unit, "loaded", "active", "running")
        };
        match kind {
            Kind::Running => UnitSnapshot {
                memory_current: Some(frac(
                    (64 + h % 900) * MIB,
                    1.0 + 0.1 * wave(now, HOUR, (h % 100) as f64 / 100.0),
                )),
                ..base
            },
            Kind::Activating => UnitSnapshot {
                active_state: "activating".into(),
                sub_state: "auto-restart".into(),
                active_enter_ts: None,
                ..base
            },
            Kind::Inactive => UnitSnapshot {
                active_state: "inactive".into(),
                sub_state: "dead".into(),
                exec_main_exit_ts: Some(started + 60),
                ..base
            },
            Kind::Failed => UnitSnapshot {
                active_state: "failed".into(),
                sub_state: "failed".into(),
                result: Some("exit-code".into()),
                exec_main_status: Some(1),
                exec_main_exit_ts: Some(started + 5),
                ..base
            },
            Kind::NotFound => UnitSnapshot {
                description: unit.into(),
                ..unit_snapshot(unit, "not-found", "inactive", "dead")
            },
        }
    }

    /// A timer and its service. Runs are periodic (hourly, 6-hourly or daily, by name hash); one in
    /// ten runs fails, and every run of a timer whose name contains `fail` fails.
    fn timer(&self, timer: &str, service: &str, now: i64) -> (UnitSnapshot, UnitSnapshot) {
        let h = fnv1a(timer);
        let period = [3600, 6 * 3600, 86_400][(h % 3) as usize];
        let offset = (h % period as u64) as i64;
        let run = (now - offset).div_euclid(period);
        let last = run * period + offset;
        let duration = 20 + (h % 60) as i64;
        let fails = timer.contains("fail") || mix(h ^ self.seed, run as u64) % 10 == 0;
        let running = now < last + duration;

        let timer_snap = UnitSnapshot {
            description: describe(timer),
            active_enter_ts: Some(self.boot_time + 5),
            last_trigger_ts: Some(last),
            next_elapse_ts: Some(last + period),
            trigger_unit: Some(service.into()),
            ..unit_snapshot(timer, "loaded", "active", "waiting")
        };
        let base = UnitSnapshot {
            description: describe(service),
            invocation_id: Some(format!("{:016x}{:016x}", mix(h, run as u64), h)),
            exec_main_start_ts: Some(last),
            n_restarts: Some(0),
            ..unit_snapshot(service, "loaded", "inactive", "dead")
        };
        let service_snap = if running {
            UnitSnapshot {
                active_state: "activating".into(),
                sub_state: "start".into(),
                memory_current: Some((32 + h % 200) * MIB),
                ..base
            }
        } else if fails {
            UnitSnapshot {
                active_state: "failed".into(),
                sub_state: "failed".into(),
                result: Some("exit-code".into()),
                exec_main_status: Some(1),
                exec_main_exit_ts: Some(last + duration),
                ..base
            }
        } else {
            UnitSnapshot {
                result: Some("success".into()),
                exec_main_status: Some(0),
                exec_main_exit_ts: Some(last + duration),
                ..base
            }
        };
        (timer_snap, service_snap)
    }
}

impl Collector for MockCollector {
    fn host_info(&self) -> Result<HostInfo, CollectError> {
        Ok(HostInfo {
            hostname: "gargoyle-mock".into(),
            os: "Debian GNU/Linux 13 (trixie)".into(),
            kernel: "6.12.0-mock-amd64".into(),
            cpu_model: "Mock CPU @ 2.00GHz".into(),
            cores: CORES as u32,
            boot_time: self.boot_time,
        })
    }

    fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
        Ok(self.snapshot_at(now()))
    }

    fn units(&self, units: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
        Ok(self.units_at(units, now()))
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Running,
    Activating,
    Inactive,
    Failed,
    NotFound,
}

impl CoreAcc {
    /// Adds `jiffies` split by usage and iowait percent; busy time is 70% user, 25% system and 5%
    /// softirq.
    fn add(&mut self, usage_pct: f64, iowait_pct: f64, jiffies: f64) {
        let busy = jiffies * usage_pct / 100.0;
        let iowait = (jiffies * iowait_pct / 100.0).min(jiffies - busy);
        self.user += busy * 0.70;
        self.system += busy * 0.25;
        self.softirq += busy * 0.05;
        self.iowait += iowait;
        self.idle += jiffies - busy - iowait;
    }

    fn times(&self) -> CpuTimes {
        CpuTimes {
            user: self.user as u64,
            system: self.system as u64,
            softirq: self.softirq as u64,
            idle: self.idle as u64,
            iowait: self.iowait as u64,
            ..CpuTimes::default()
        }
    }
}

fn unit_snapshot(unit: &str, load: &str, active: &str, sub: &str) -> UnitSnapshot {
    UnitSnapshot {
        unit: unit.into(),
        load_state: load.into(),
        active_state: active.into(),
        sub_state: sub.into(),
        description: String::new(),
        active_enter_ts: None,
        n_restarts: None,
        result: None,
        exec_main_status: None,
        exec_main_start_ts: None,
        exec_main_exit_ts: None,
        invocation_id: None,
        memory_current: None,
        last_trigger_ts: None,
        next_elapse_ts: None,
        trigger_unit: None,
    }
}

/// `music-sync.timer` → `Mock music-sync`.
fn describe(unit: &str) -> String {
    let stem = unit.rsplit_once('.').map_or(unit, |(stem, _)| stem);
    format!("Mock {stem}")
}

fn disk(mount: &str, fs_type: &str, total: u64, used_frac: f64) -> DiskUsage {
    let used = frac(total, used_frac);
    DiskUsage {
        mount: mount.into(),
        fs_type: fs_type.into(),
        total,
        used,
        avail: total - used,
    }
}

/// `sin` over `period` seconds, phase as a fraction of the period; in [-1, 1].
fn wave(t: i64, period: f64, phase: f64) -> f64 {
    (TAU * (t as f64 / period + phase)).sin()
}

/// `total * f`, with `f` clamped to 0–1.
fn frac(total: u64, f: f64) -> u64 {
    (total as f64 * f.clamp(0.0, 1.0)) as u64
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// SplitMix64 finalizer over `seed` and `x`: a stable, well-mixed hash without a dependency.
fn mix(seed: u64, x: u64) -> u64 {
    let mut z = seed ^ x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a: stable across Rust versions and platforms, unlike `DefaultHasher`.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gargoyle_core::rates::{cpu_usage, system_rates};

    const BOOT: i64 = 1_794_000_000;
    const T0: i64 = BOOT + 86_400;

    fn mock() -> MockCollector {
        MockCollector::with_boot_time(42, BOOT)
    }

    fn req(unit: &str) -> UnitRequest {
        UnitRequest::new(unit)
    }

    #[test]
    fn same_seed_same_samples() {
        let (a, b) = (mock(), mock());
        for i in 0..20 {
            let ts = T0 + i * 10;
            assert_eq!(a.snapshot_at(ts), b.snapshot_at(ts));
        }
    }

    #[test]
    fn different_seeds_differ() {
        let a = mock().snapshot_at(T0);
        let b = MockCollector::with_boot_time(7, BOOT).snapshot_at(T0);
        assert_ne!(a.memory, b.memory);
    }

    #[test]
    fn counters_are_monotonic_and_rates_plausible() {
        let m = mock();
        let mut prev = m.snapshot_at(T0);
        for i in 1..=2000 {
            let cur = m.snapshot_at(T0 + i * 10);
            let r = system_rates(Some(&prev), &cur);
            let cpu = r.cpu.expect("monotonic counters always give a CPU rate");
            assert!((0.0..=100.0).contains(&cpu.usage), "{cpu:?}");
            assert!((0.0..=100.0).contains(&cpu.iowait), "{cpu:?}");
            for (p, c) in prev.cpu.per_core.iter().zip(&cur.cpu.per_core) {
                assert!(cpu_usage(p, c).is_some(), "per-core counters monotonic");
            }
            assert_eq!(r.net.len(), 1);
            assert!(r.net[0].rx_bps > 0.0 && r.net[0].tx_bps > 0.0);
            prev = cur;
        }
    }

    #[test]
    fn total_is_sum_of_cores() {
        let s = mock().snapshot_at(T0);
        assert_eq!(s.cpu.per_core.len(), CORES);
        let sum: u64 = s.cpu.per_core.iter().map(CpuTimes::total).sum();
        assert_eq!(s.cpu.total.total(), sum);
        // Roughly uptime × cores × HZ jiffies (floors lose at most a few).
        let expected = (T0 - BOOT) as f64 * CORES as f64 * HZ;
        assert!((sum as f64 - expected).abs() < 100.0, "{sum} vs {expected}");
    }

    #[test]
    fn cpu_spikes_occur_but_are_rare() {
        let m = mock();
        let mut prev = m.snapshot_at(T0);
        let (mut spiky, mut total) = (0, 0);
        // Two days at 10s.
        for i in 1..=17_280 {
            let cur = m.snapshot_at(T0 + i * 10);
            let usage = system_rates(Some(&prev), &cur).cpu.map_or(0.0, |c| c.usage);
            if usage > 90.0 {
                spiky += 1;
            }
            total += 1;
            prev = cur;
        }
        let share = f64::from(spiky) / f64::from(total);
        assert!(spiky > 0, "no spike in two days");
        assert!(share < 0.1, "spikes too frequent: {share}");
    }

    #[test]
    fn going_back_in_time_does_not_decrease_counters() {
        let m = mock();
        let later = m.snapshot_at(T0 + 100);
        let earlier = m.snapshot_at(T0);
        assert_eq!(later.cpu, earlier.cpu);
        assert_eq!(later.net, earlier.net);
    }

    #[test]
    fn long_gap_is_filled_without_stepping_forever() {
        let m = mock();
        let a = m.snapshot_at(T0);
        let b = m.snapshot_at(T0 + 365 * 86_400);
        assert!(system_rates(Some(&a), &b).cpu.is_some());
    }

    #[test]
    fn gauges_are_plausible() {
        let m = mock();
        for i in 0..500 {
            let s = m.snapshot_at(T0 + i * 977);
            assert!(s.memory.available < s.memory.total);
            assert!(s.memory.swap_free <= s.memory.swap_total);
            assert!(s.load.iter().all(|l| *l >= 0.0));
            for d in &s.disks {
                assert_eq!(d.used + d.avail, d.total);
            }
            assert!(s.temps.iter().all(|t| (20.0..110.0).contains(&t.celsius)));
            assert_eq!(s.uptime_s, (s.ts - BOOT) as f64);
        }
    }

    #[test]
    fn host_info_is_static() {
        let h = mock().host_info().expect("mock never fails");
        assert_eq!(h.cores, CORES as u32);
        assert_eq!(h.boot_time, BOOT);
    }

    #[test]
    fn unit_states_are_stable_and_cover_every_kind() {
        let m = mock();
        let names: Vec<String> = (0..200).map(|i| format!("svc{i}.service")).collect();
        let reqs: Vec<_> = names.iter().map(|n| req(n)).collect();
        let a = m.units_at(&reqs, T0);
        let b = MockCollector::with_boot_time(99, BOOT).units_at(&reqs, T0 + 5000);
        let states = |v: &[UnitSnapshot]| {
            v.iter()
                .map(|u| (u.load_state.clone(), u.active_state.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(states(&a), states(&b), "state depends on the name only");
        for (load, active) in [
            ("loaded", "active"),
            ("loaded", "failed"),
            ("loaded", "inactive"),
            ("loaded", "activating"),
            ("not-found", "inactive"),
        ] {
            assert!(
                a.iter()
                    .any(|u| u.load_state == load && u.active_state == active),
                "missing {load}/{active}"
            );
        }
    }

    #[test]
    fn fail_names_always_fail() {
        let m = mock();
        for name in ["backup-fail.service", "fail.service", "x-failover.service"] {
            let u = &m.units_at(&[req(name)], T0)[0];
            assert_eq!(u.active_state, "failed", "{name}");
            assert_eq!(u.result.as_deref(), Some("exit-code"));
        }
    }

    #[test]
    fn not_found_unit_has_no_properties() {
        let m = mock();
        let u = (0..)
            .map(|i| m.units_at(&[req(&format!("nf{i}.service"))], T0).remove(0))
            .find(|u| u.load_state == "not-found")
            .expect("some name hashes to not-found");
        assert_eq!(u.active_state, "inactive");
        assert_eq!(u.memory_current, None);
        assert_eq!(u.n_restarts, None);
    }

    #[test]
    fn dev_config_services_show_several_states() {
        let m = mock();
        let reqs: Vec<_> = [
            "navidrome.service",
            "minecraft.service",
            "nginx.service",
            "backup-fail.service",
        ]
        .into_iter()
        .map(req)
        .collect();
        let units = m.units_at(&reqs, T0);
        let mut states: Vec<_> = units.iter().map(|u| u.active_state.as_str()).collect();
        states.sort_unstable();
        states.dedup();
        assert!(states.len() >= 3, "only {states:?}");
    }

    #[test]
    fn timer_returns_timer_and_service() {
        let m = mock();
        let units = m.units_at(&[req("music-sync.timer")], T0);
        assert_eq!(units.len(), 2);
        let (timer, svc) = (&units[0], &units[1]);
        assert_eq!(timer.unit, "music-sync.timer");
        assert_eq!(svc.unit, "music-sync.service");
        assert_eq!(timer.trigger_unit.as_deref(), Some("music-sync.service"));
        let (last, next) = (
            timer.last_trigger_ts.expect("has run"),
            timer.next_elapse_ts.expect("scheduled"),
        );
        assert!(last <= T0 && T0 < next);
        assert_eq!(svc.exec_main_start_ts, Some(last));
    }

    #[test]
    fn timer_with_explicit_service() {
        let m = mock();
        let r = UnitRequest {
            unit: "backup.timer".into(),
            service: Some("restic.service".into()),
        };
        let units = m.units_at(&[r], T0);
        assert_eq!(units[1].unit, "restic.service");
        assert_eq!(units[0].trigger_unit.as_deref(), Some("restic.service"));
    }

    #[test]
    fn timers_run_periodically() {
        let m = mock();
        let mut triggers = Vec::new();
        for i in 0..(3 * 24 * 6) {
            let units = m.units_at(&[req("music-sync.timer")], T0 + i * 600);
            triggers.push(units[0].last_trigger_ts.expect("has run"));
            assert!(["failed", "inactive", "activating"].contains(&units[1].active_state.as_str()));
        }
        triggers.dedup();
        assert!(triggers.len() >= 3, "{triggers:?}");
    }

    #[test]
    fn timer_runs_fail_occasionally() {
        let m = mock();
        // Sample each of many timers well after its run finished.
        let failed = (0..200)
            .filter(|i| {
                let units = m.units_at(&[req(&format!("t{i}.timer"))], T0);
                let last = units[0].last_trigger_ts.expect("has run");
                let after = m.units_at(&[req(&format!("t{i}.timer"))], last + 120);
                after[1].active_state == "failed"
            })
            .count();
        assert!((1..60).contains(&failed), "{failed} of 200");
        let units = m.units_at(&[req("nightly-fail.timer")], T0);
        let last = units[0].last_trigger_ts.expect("has run");
        let after = m.units_at(&[req("nightly-fail.timer")], last + 120);
        assert_eq!(after[1].active_state, "failed");
        assert_eq!(after[1].result.as_deref(), Some("exit-code"));
    }
}
