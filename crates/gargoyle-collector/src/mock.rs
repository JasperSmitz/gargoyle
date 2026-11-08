//! Deterministic fake host for development and tests (ARCHITECTURE §4, `collector.mode = "mock"`).
//!
//! Gauges (memory, load, disks, temperatures) are pure functions of `(seed, ts)`: slow sine waves
//! around plausible baselines plus seeded noise. CPU and network counters must be cumulative and
//! monotonic, so they are integrated from the same kind of curve and kept in a small mutex-guarded
//! state. Given the same seed and the same sequence of sample times, the output is identical.
//!
//! Unit states are derived from a hash of the unit name only, so a given name always shows the
//! same state; names containing `fail` are always failed. Timers run on a schedule derived from
//! their name (see `Schedule`); `units_at` and `timer_history_at` describe the same runs, and a
//! `*-quick.timer` alternates failed and successful runs every 2 minutes for demos.

use std::f64::consts::TAU;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    CollectError, Collector, CpuCounters, CpuTimes, DiskUsage, HostInfo, MemoryInfo, NetCounters,
    SystemSnapshot, TempReading, TimerRunRecord, UnitRequest, UnitSnapshot,
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
/// Period of `*-quick.timer` demo timers.
const QUICK_PERIOD_S: i64 = 120;
/// [`MockCollector::timer_history_at`] returns at most this many (newest) runs, like the journal
/// backfill's entry cap.
const MAX_HISTORY_RUNS: i64 = 1000;

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
    /// A mock host that booted 8–9 days before now, so a 7-day history backfill never samples
    /// before boot.
    pub fn new(seed: u64) -> Self {
        let boot_time = now() - 8 * 86_400 - (mix(seed, 1) % 86_400) as i64;
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
    /// earlier than the previous one returns the counters unchanged. Before `boot_time` the host
    /// is treated as just booted: uptime and counters stay at 0.
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
        // Nothing accumulates before boot, so uptime and counters stay consistent.
        let from = c.last_ts.unwrap_or(self.boot_time).max(self.boot_time);
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

    /// Past and current runs of `timer` whose start is at or after `since` (and after boot), as
    /// the journal would report them at `now`: oldest first, at most [`MAX_HISTORY_RUNS`] (the
    /// newest), the run in progress unfinished. Consistent with [`Self::units_at`].
    pub fn timer_history_at(
        &self,
        timer: &str,
        service: &str,
        since: i64,
        now: i64,
    ) -> Vec<TimerRunRecord> {
        let schedule = Schedule::of(timer);
        let current = schedule.index_at(now);
        let first = schedule
            .index_at(since.max(self.boot_time) - 1)
            .saturating_add(1)
            .max(current.saturating_sub(MAX_HISTORY_RUNS - 1));
        (first..=current)
            .map(|i| self.timer_run(timer, &schedule, i))
            .filter(|run| run.start <= now)
            .map(|run| {
                let finished = run.end() <= now;
                TimerRunRecord {
                    timer: timer.into(),
                    service: service.into(),
                    invocation_id: Some(run.invocation_id.clone()),
                    started_at: run.start,
                    finished_at: finished.then(|| run.end()),
                    result: finished.then(|| run.result().into()),
                    exit_status: finished.then_some(run.exit_status),
                }
            })
            .collect()
    }

    /// Run `index` of `timer`. Every run gets its own invocation ID and a slightly different
    /// duration. One in ten runs fails at random; see [`Schedule`] for the fixed rules.
    fn timer_run(&self, timer: &str, schedule: &Schedule, index: i64) -> MockRun {
        let h = fnv1a(timer);
        let r = mix(h ^ self.seed, index as u64);
        let fails = match schedule.kind {
            TimerKind::AlwaysFails => true,
            TimerKind::Alternating => index.rem_euclid(2) == 0,
            TimerKind::Normal => r % 10 == 0,
        };
        MockRun {
            start: schedule.start(index),
            duration: schedule.duration + ((r >> 8) % 10) as i64,
            fails,
            exit_status: if fails { 1 + ((r >> 16) % 2) as i32 } else { 0 },
            invocation_id: format!("{:016x}{:016x}", mix(h, index as u64), h),
        }
    }

    /// A timer and its service at `now`. The service is `activating` while a run is in progress,
    /// otherwise `failed` or `inactive` with the last run's result.
    fn timer(&self, timer: &str, service: &str, now: i64) -> (UnitSnapshot, UnitSnapshot) {
        let schedule = Schedule::of(timer);
        let index = schedule.index_at(now);
        let run = self.timer_run(timer, &schedule, index);
        let h = fnv1a(timer);

        let timer_snap = UnitSnapshot {
            description: describe(timer),
            active_enter_ts: Some(self.boot_time + 5),
            last_trigger_ts: Some(run.start),
            next_elapse_ts: Some(schedule.start(index + 1)),
            trigger_unit: Some(service.into()),
            ..unit_snapshot(timer, "loaded", "active", "waiting")
        };
        let base = UnitSnapshot {
            description: describe(service),
            invocation_id: Some(run.invocation_id.clone()),
            exec_main_start_ts: Some(run.start),
            n_restarts: Some(0),
            ..unit_snapshot(service, "loaded", "inactive", "dead")
        };
        let service_snap = if now < run.end() {
            UnitSnapshot {
                active_state: "activating".into(),
                sub_state: "start".into(),
                memory_current: Some((32 + h % 200) * MIB),
                ..base
            }
        } else {
            let (active, sub) = if run.fails {
                ("failed", "failed")
            } else {
                ("inactive", "dead")
            };
            UnitSnapshot {
                active_state: active.into(),
                sub_state: sub.into(),
                result: Some(run.result().into()),
                exec_main_status: Some(run.exit_status),
                exec_main_exit_ts: Some(run.end()),
                ..base
            }
        };
        (timer_snap, service_snap)
    }
}

/// When a mock timer fires and how its runs end, derived from its name:
///
/// - `*-quick.timer`: every 2 minutes, runs ~20s, and alternates failed → success, so a failed
///   run turning `critical` and the next success returning `healthy` can be watched live.
/// - names containing `fail`: every run fails (period by hash as below).
/// - otherwise hourly, 6-hourly or daily by name hash, runs 20–90s, one in ten runs fails.
#[derive(Debug, Clone, Copy)]
struct Schedule {
    period: i64,
    offset: i64,
    /// Base duration; each run adds 0–9s.
    duration: i64,
    kind: TimerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    Normal,
    AlwaysFails,
    Alternating,
}

impl Schedule {
    fn of(timer: &str) -> Self {
        let h = fnv1a(timer);
        let kind = if timer.contains("fail") {
            TimerKind::AlwaysFails
        } else if timer.ends_with("-quick.timer") {
            TimerKind::Alternating
        } else {
            TimerKind::Normal
        };
        let (period, duration) = if timer.ends_with("-quick.timer") {
            (QUICK_PERIOD_S, 15)
        } else {
            (
                [3600, 6 * 3600, 86_400][(h % 3) as usize],
                20 + (h % 60) as i64,
            )
        };
        Self {
            period,
            offset: (h % period as u64) as i64,
            duration,
            kind,
        }
    }

    /// Index of the last run started at or before `t`.
    fn index_at(&self, t: i64) -> i64 {
        (t - self.offset).div_euclid(self.period)
    }

    fn start(&self, index: i64) -> i64 {
        index * self.period + self.offset
    }
}

struct MockRun {
    start: i64,
    duration: i64,
    fails: bool,
    exit_status: i32,
    invocation_id: String,
}

impl MockRun {
    fn end(&self) -> i64 {
        self.start + self.duration
    }

    fn result(&self) -> &'static str {
        if self.fails { "exit-code" } else { "success" }
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

    fn timer_history(
        &self,
        timer: &str,
        service: &str,
        since: i64,
    ) -> Result<Vec<TimerRunRecord>, CollectError> {
        Ok(self.timer_history_at(timer, service, since, now()))
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
    fn nothing_accumulates_before_boot() {
        let m = mock();
        let a = m.snapshot_at(BOOT - 86_400);
        let b = m.snapshot_at(BOOT - 10);
        assert_eq!(a.cpu.total.total(), 0);
        assert_eq!(b.cpu.total.total(), 0);
        assert_eq!(b.uptime_s, 0.0);
        let c = m.snapshot_at(BOOT + 100);
        let expected = 100.0 * CORES as f64 * HZ;
        assert!((c.cpu.total.total() as f64 - expected).abs() < 10.0);
    }

    #[test]
    fn default_boot_time_allows_a_week_of_backfill() {
        let m = MockCollector::new(5);
        assert!(m.boot_time < now() - 8 * 86_400);
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

    fn service_at(m: &MockCollector, timer: &str, t: i64) -> UnitSnapshot {
        m.units_at(&[req(timer)], t).remove(1)
    }

    #[test]
    fn quick_timer_alternates_failed_and_success_every_two_minutes() {
        let m = mock();
        let timer = "demo-quick.timer";
        let first = m.units_at(&[req(timer)], T0)[0]
            .next_elapse_ts
            .expect("scheduled");
        let mut results = Vec::new();
        for i in 0..6 {
            let start = first + i * QUICK_PERIOD_S;
            let timer_snap = &m.units_at(&[req(timer)], start)[0];
            assert_eq!(timer_snap.last_trigger_ts, Some(start));
            assert_eq!(timer_snap.next_elapse_ts, Some(start + QUICK_PERIOD_S));
            // Finished well before the next run.
            let svc = service_at(&m, timer, start + 60);
            results.push((svc.result.clone(), svc.exec_main_status));
            assert_eq!(
                svc.active_state,
                if svc.result.as_deref() == Some("success") {
                    "inactive"
                } else {
                    "failed"
                }
            );
        }
        for pair in results.windows(2) {
            assert_ne!(pair[0].0, pair[1].0, "{results:?}");
        }
        assert!(
            results
                .iter()
                .any(|(r, s)| r.as_deref() == Some("exit-code") && *s != Some(0))
        );
        assert!(results.contains(&(Some("success".into()), Some(0))));
    }

    #[test]
    fn running_service_is_activating_with_a_new_invocation() {
        let m = mock();
        let timer = "demo-quick.timer";
        let start = m.units_at(&[req(timer)], T0)[0]
            .next_elapse_ts
            .expect("scheduled");
        let before = service_at(&m, timer, start - 1);
        let running = service_at(&m, timer, start + 1);
        let after = service_at(&m, timer, start + 60);

        assert_eq!(running.active_state, "activating");
        assert_eq!(running.sub_state, "start");
        assert_eq!(running.exec_main_start_ts, Some(start));
        assert_eq!(running.exec_main_exit_ts, None);
        assert!(running.invocation_id.is_some());
        assert_ne!(running.invocation_id, before.invocation_id);
        assert_eq!(running.invocation_id, after.invocation_id);
        assert_eq!(after.exec_main_start_ts, Some(start));
        assert!(after.exec_main_exit_ts.is_some_and(|e| e > start));
    }

    #[test]
    fn history_matches_units_at() {
        let m = mock();
        for timer in ["demo-quick.timer", "music-sync.timer", "nightly-fail.timer"] {
            for k in 0..200 {
                let t = T0 + k * 37;
                let svc = service_at(&m, timer, t);
                let history = m.timer_history_at(timer, "svc.service", T0 - 3 * 86_400, t);
                let last = history.last().expect("runs in three days");
                assert_eq!(last.invocation_id, svc.invocation_id, "{timer} at {t}");
                assert_eq!(Some(last.started_at), svc.exec_main_start_ts);
                assert_eq!(last.finished_at, svc.exec_main_exit_ts);
                assert_eq!(last.result, svc.result);
                assert_eq!(last.exit_status, svc.exec_main_status);
                assert_eq!(last.service, "svc.service");
                assert_eq!(last.timer, timer);
            }
        }
    }

    #[test]
    fn history_window_order_and_ids() {
        let m = mock();
        let timer = "demo-quick.timer";
        let now = T0 + 3600;
        let since = now - 1800;
        let runs = m.timer_history_at(timer, "demo-quick.service", since, now);
        assert!((14..=16).contains(&runs.len()), "{}", runs.len());
        assert!(
            runs.iter()
                .all(|r| r.started_at >= since && r.started_at <= now)
        );
        assert!(
            runs.windows(2)
                .all(|w| w[1].started_at - w[0].started_at == QUICK_PERIOD_S)
        );
        let mut ids: Vec<_> = runs.iter().map(|r| r.invocation_id.clone()).collect();
        ids.dedup();
        assert_eq!(ids.len(), runs.len(), "a new invocation ID per run");
        // Only the newest run can be unfinished, and every finished run has a duration.
        for r in &runs[..runs.len() - 1] {
            let f = r.finished_at.expect("finished");
            assert!((15..25).contains(&(f - r.started_at)), "{r:?}");
        }
        // A window starting exactly at a run includes it.
        let exact = m.timer_history_at(timer, "s", runs[3].started_at, now);
        assert_eq!(exact[0].started_at, runs[3].started_at);
        assert!(m.timer_history_at(timer, "s", now + 10, now).is_empty());
    }

    #[test]
    fn history_starts_at_boot_and_is_capped() {
        let m = mock();
        let runs = m.timer_history_at("demo-quick.timer", "s", 0, BOOT + 600);
        assert!(runs.first().is_some_and(|r| r.started_at >= BOOT));
        let runs = m.timer_history_at("demo-quick.timer", "s", 0, BOOT + 7 * 86_400);
        assert_eq!(runs.len(), MAX_HISTORY_RUNS as usize);
        assert!(
            runs.last()
                .is_some_and(|r| r.started_at > BOOT + 7 * 86_400 - 120)
        );
    }

    #[test]
    fn history_failures() {
        let m = mock();
        let fail = m.timer_history_at("nightly-fail.timer", "s", T0 - 7 * 86_400, T0);
        assert!(!fail.is_empty());
        assert!(
            fail.iter()
                .filter(|r| r.finished_at.is_some())
                .all(|r| r.result.as_deref() == Some("exit-code")
                    && r.exit_status.is_some_and(|s| s > 0))
        );
        let hourly: Vec<_> = (0..50)
            .flat_map(|i| m.timer_history_at(&format!("t{i}.timer"), "s", T0 - 7 * 86_400, T0))
            .filter(|r| r.finished_at.is_some())
            .collect();
        let failed = hourly
            .iter()
            .filter(|r| r.result.as_deref() == Some("exit-code"))
            .count();
        let share = failed as f64 / hourly.len() as f64;
        assert!((0.03..0.2).contains(&share), "{failed} of {}", hourly.len());
        assert!(
            hourly
                .iter()
                .filter(|r| r.result.as_deref() == Some("success"))
                .all(|r| r.exit_status == Some(0))
        );
    }

    #[test]
    fn collector_trait_returns_history() {
        let m = MockCollector::new(3);
        let runs = m
            .timer_history("demo-quick.timer", "demo-quick.service", now() - 3600)
            .expect("mock never fails");
        assert!((29..=31).contains(&runs.len()), "{}", runs.len());
    }
}
