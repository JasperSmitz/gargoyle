//! The in-memory `State` (ARCHITECTURE §1, §3): the latest collected facts, plus the little
//! history health needs (CPU/load windows, restart trackers, timer runs, the last state of every
//! target for transition events).
//!
//! The scheduler writes it through the `record_*` methods; HTTP handlers read it through the
//! `*_view` methods, which evaluate health at read time so that staleness is judged against the
//! moment of the request, not the moment of the last sample.

use std::collections::HashMap;

use gargoyle_core::api::{
    CheckSummary, CpuSummary, DiskSummary, HostInfo as HostInfoDto, LibraryReport, LibrarySummary,
    LoadSummary, MemorySummary, NetSummary, Overview, ServiceSummary, SystemSummary, TargetKind,
    TempSummary, TimerSummary,
};
use gargoyle_core::config::{CheckConfig, Config, LibraryConfig, ServiceConfig, TimerConfig};
use gargoyle_core::health::{
    self, Assessment, CheckTracker, Health, LibraryStatus, ProbeOutcome, ReportFacts,
    RestartTracker, SystemHealth, TimerRuns, Transition, Transitions, Window, disk_used_pct,
    evaluate_check, evaluate_library, evaluate_service, evaluate_system, evaluate_timer, is_stale,
    load_per_core, memory_used_pct, run_of, service_is_running, service_run, timer_service,
};
use gargoyle_core::liner::ReadFailure;
use gargoyle_core::model::{HostInfo, SystemSnapshot, TimerRunRecord, UnitSnapshot};
use gargoyle_core::rates::{CpuUsage, NetRate, system_rates};

use crate::storage::samples::{self, Sample};

/// Shown as `host` until host info has been collected once.
const UNKNOWN_HOST: &str = "unknown";

/// The fixed target name of the music library in events (§6).
pub const LIBRARY_TARGET: &str = "library";

/// How far back journal backfill looks for a timer with no stored runs.
pub const JOURNAL_BACKFILL_S: i64 = 30 * 86_400;

#[derive(Debug)]
pub struct State {
    host_info: Option<HostInfo>,
    system: SystemState,
    units: UnitState,
    /// Per configured check name.
    checks: HashMap<String, CheckTracker>,
    library: LibraryState,
    transitions: Transitions,
    /// Per timer: start of the newest run stored before this process started, from where journal
    /// backfill continues. Captured at restore, before live runs are stored.
    journal_since: HashMap<String, i64>,
}

#[derive(Debug)]
struct SystemState {
    last: Option<SystemSnapshot>,
    /// From the last pair of samples; `None` on the first sample or after a counter reset.
    cpu: Option<CpuUsage>,
    net: Vec<NetRate>,
    cpu_window: Window,
    load_window: Window,
    /// Unix seconds of the last successful sample. Failures show as `unknown` once this goes
    /// stale.
    last_ok: Option<i64>,
    /// The last collection error since the last successful sample. The system summary has no
    /// message field, so it only reaches the messages of system events.
    error: Option<String>,
}

#[derive(Debug, Default)]
struct UnitState {
    by_unit: HashMap<String, UnitSnapshot>,
    restarts: HashMap<String, RestartTracker>,
    /// Per configured timer unit.
    timer_runs: HashMap<String, TimerRuns>,
    /// Per configured timer unit: the observed run last handed out for storing, so an unchanged
    /// run is not written every tick.
    written_runs: HashMap<String, TimerRunRecord>,
    last_ok: Option<i64>,
    error: Option<String>,
}

/// The last poll of the library report. Only used with `[library]`.
#[derive(Debug, Default)]
struct LibraryState {
    /// When the last poll completed.
    checked_at: Option<i64>,
    /// `None` until the first read. A failed read drops the previous report (ADR 0007).
    read: Option<Result<Box<LoadedReport>, ReadFailure>>,
}

/// A valid report, parsed once per change into the API view.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedReport {
    /// Unix seconds.
    pub generated_at: i64,
    pub view: LibraryReport,
}

impl LoadedReport {
    fn facts(&self) -> ReportFacts {
        let s = &self.view.summary;
        ReportFacts {
            generated_at: self.generated_at,
            issue_counts: s.issue_counts,
            unreadable_files: s.unreadable_files,
            albums: s.albums,
        }
    }
}

/// The result of one poll of the library report.
#[derive(Debug)]
pub enum LibraryPoll {
    /// The file has not changed since the last read; its outcome stands.
    Unchanged,
    Read(Result<Box<LoadedReport>, ReadFailure>),
}

impl State {
    pub fn new(config: &Config) -> Self {
        Self {
            host_info: None,
            system: SystemState {
                last: None,
                cpu: None,
                net: Vec::new(),
                cpu_window: Window::new(config.thresholds.cpu.window),
                load_window: Window::new(config.thresholds.load.window),
                last_ok: None,
                error: None,
            },
            units: UnitState::default(),
            checks: HashMap::new(),
            library: LibraryState::default(),
            transitions: Transitions::default(),
            journal_since: HashMap::new(),
        }
    }

    /// Restores what storage knows from before this process: the last logged state of every
    /// target, and each timer's newest run and success.
    pub fn restore(
        &mut self,
        states: Vec<(TargetKind, String, Health)>,
        timers: Vec<(String, TimerRuns)>,
    ) {
        for (kind, target, health) in states {
            self.transitions.restore(kind, &target, health);
            if kind == TargetKind::Check {
                self.checks.insert(target, CheckTracker::restored(health));
            }
        }
        for (timer, runs) in timers {
            if let Some(last) = &runs.last {
                self.journal_since.insert(timer.clone(), last.started_at);
            }
            self.units.timer_runs.insert(timer, runs);
        }
    }

    pub fn has_host_info(&self) -> bool {
        self.host_info.is_some()
    }

    pub fn record_host_info(&mut self, info: HostInfo) {
        self.host_info = Some(info);
    }

    /// Stores a successful system sample, derives rates against the previous one and feeds the
    /// CPU and load windows.
    pub fn record_system(&mut self, snap: SystemSnapshot) {
        let s = &mut self.system;
        let rates = system_rates(s.last.as_ref(), &snap);
        if let Some(cpu) = rates.cpu {
            s.cpu_window.push(snap.ts, cpu.usage);
        }
        s.cpu = rates.cpu;
        s.net = rates.net;

        let cores = self
            .host_info
            .as_ref()
            .map(|h| h.cores)
            .filter(|&c| c > 0)
            .unwrap_or_else(|| u32::try_from(snap.cpu.per_core.len()).unwrap_or(u32::MAX));
        if let Some(per_core) = load_per_core(snap.load[0], cores) {
            s.load_window.push(snap.ts, per_core);
        }

        s.last_ok = Some(snap.ts);
        s.last = Some(snap);
        s.error = None;
    }

    pub fn record_system_error(&mut self, error: String) {
        self.system.error = Some(error);
    }

    /// Records the outcome of one probe of the check called `name`, finished at `ts`.
    pub fn record_check(&mut self, name: &str, ts: i64, outcome: ProbeOutcome) {
        self.checks
            .entry(name.to_owned())
            .or_default()
            .record(ts, outcome);
    }

    /// Records one completed poll of the library report at `ts`.
    pub fn record_library(&mut self, ts: i64, poll: LibraryPoll) {
        self.library.checked_at = Some(ts);
        if let LibraryPoll::Read(read) = poll {
            self.library.read = Some(read);
        }
    }

    /// Stores the unit snapshots of one successful unit tick at `now`.
    pub fn record_units(&mut self, now: i64, snaps: Vec<UnitSnapshot>) {
        let u = &mut self.units;
        u.by_unit.clear();
        for snap in snaps {
            u.restarts
                .entry(snap.unit.clone())
                .or_default()
                .observe(now, &snap);
            u.by_unit.insert(snap.unit.clone(), snap);
        }
        u.last_ok = Some(now);
        u.error = None;
    }

    /// The series points of the latest system sample (empty before the first one).
    pub fn system_samples(&self) -> Vec<Sample> {
        let s = &self.system;
        s.last
            .as_ref()
            .map(|snap| samples::system(snap, s.cpu, &s.net))
            .unwrap_or_default()
    }

    pub fn record_units_error(&mut self, error: String) {
        self.units.error = Some(error);
    }

    /// Folds the current or last run of every configured timer's service, as the latest unit
    /// tick saw it, into the timer's runs. Returns the runs that are new or changed (started,
    /// or finished since the last call) for storing.
    pub fn observe_timer_runs(&mut self, config: &Config) -> Vec<TimerRunRecord> {
        let u = &mut self.units;
        let mut changed = Vec::new();
        for cfg in &config.timers {
            let service = timer_service(cfg, u.by_unit.get(&cfg.unit));
            let Some(svc) = u.by_unit.get(&service) else {
                continue;
            };
            let Some(run) = service_run(svc) else {
                continue;
            };
            u.timer_runs
                .entry(cfg.unit.clone())
                .or_default()
                .record(&run);
            let record = TimerRunRecord {
                timer: cfg.unit.clone(),
                service,
                invocation_id: svc.invocation_id.clone(),
                started_at: run.started_at,
                finished_at: run.finished_at,
                result: run.result,
                exit_status: run.exit_status,
            };
            if u.written_runs.get(&cfg.unit) != Some(&record) {
                u.written_runs.insert(cfg.unit.clone(), record.clone());
                changed.push(record);
            }
        }
        changed
    }

    /// After a failed write: hand every observed run out again on the next tick.
    pub fn forget_written_runs(&mut self) {
        self.units.written_runs.clear();
    }

    /// Folds runs from the journal into `timer`'s runs.
    pub fn record_journal_runs(&mut self, timer: &str, records: &[TimerRunRecord]) {
        let runs = self.units.timer_runs.entry(timer.to_owned()).or_default();
        for r in records {
            runs.record(&run_of(r));
        }
    }

    /// `(timer, service, since)` for journal backfill of every configured timer: from the newest
    /// run stored before startup, or [`JOURNAL_BACKFILL_S`] back.
    pub fn journal_requests(&self, config: &Config, now: i64) -> Vec<(String, String, i64)> {
        config
            .timers
            .iter()
            .map(|cfg| {
                let service = timer_service(cfg, self.units.by_unit.get(&cfg.unit));
                let since = self
                    .journal_since
                    .get(&cfg.unit)
                    .copied()
                    .unwrap_or(now - JOURNAL_BACKFILL_S);
                (cfg.unit.clone(), service, since)
            })
            .collect()
    }

    /// Records the current state of `targets` and returns the transitions among them.
    pub fn transitions(
        &mut self,
        targets: Vec<(TargetKind, String, Assessment)>,
    ) -> Vec<Transition> {
        targets
            .into_iter()
            .filter_map(|(kind, target, a)| self.transitions.observe(kind, &target, &a))
            .collect()
    }

    /// The health of every system target (`cpu`, `load`, `memory`, `swap`, `disk:<mount>`,
    /// `temp:<label>`) at `now`, judged as the overview judges them. Empty before the first
    /// sample.
    pub fn system_targets(
        &self,
        config: &Config,
        now: i64,
    ) -> Vec<(TargetKind, String, Assessment)> {
        let s = &self.system;
        let Some(snap) = &s.last else {
            return Vec::new();
        };
        let h = self.system_health(config, snap);
        let stale = is_stale(s.last_ok, now, config.collector.system_interval);
        let judged = |a: &Assessment| {
            if stale {
                Assessment::unknown(stale_message(s.last_ok, s.error.as_deref(), now))
            } else {
                a.clone()
            }
        };
        let mut out = vec![
            ("cpu".to_owned(), judged(&h.cpu)),
            ("load".to_owned(), judged(&h.load)),
            ("memory".to_owned(), judged(&h.memory)),
            ("swap".to_owned(), judged(&h.swap)),
        ];
        out.extend(
            snap.disks
                .iter()
                .zip(&h.disks)
                .map(|(d, a)| (format!("disk:{}", d.mount), judged(a))),
        );
        // Missing configured disks stay critical when stale, as in the overview.
        out.extend(
            h.missing_disks
                .iter()
                .map(|(mount, a)| (format!("disk:{mount}"), a.clone())),
        );
        out.extend(
            snap.temps
                .iter()
                .zip(&h.temps)
                .map(|(t, a)| (format!("temp:{}", t.label), judged(a))),
        );
        out.into_iter()
            .map(|(target, a)| (TargetKind::System, target, a))
            .collect()
    }

    /// The health of every configured service and timer at `now`.
    pub fn unit_targets(&self, config: &Config, now: i64) -> Vec<(TargetKind, String, Assessment)> {
        let services = config.services.iter().map(|cfg| {
            (
                TargetKind::Service,
                cfg.unit.clone(),
                self.service_assessment(config, cfg, now),
            )
        });
        let timers = config.timers.iter().map(|cfg| {
            (
                TargetKind::Timer,
                cfg.unit.clone(),
                self.timer_assessment(config, cfg, now),
            )
        });
        services.chain(timers).collect()
    }

    /// The health of one check at `now`, for its transitions after each probe.
    pub fn check_target(&self, cfg: &CheckConfig, now: i64) -> (TargetKind, String, Assessment) {
        (
            TargetKind::Check,
            cfg.name.clone(),
            self.check_assessment(cfg, now),
        )
    }

    /// The health of the library at `now`, for its transitions after each poll. `None` without
    /// `[library]`.
    pub fn library_target(
        &self,
        config: &Config,
        now: i64,
    ) -> Option<(TargetKind, String, Assessment)> {
        let cfg = config.library.as_ref()?;
        Some((
            TargetKind::Library,
            LIBRARY_TARGET.to_owned(),
            self.library_assessment(cfg, now),
        ))
    }

    fn library_assessment(&self, cfg: &LibraryConfig, now: i64) -> Assessment {
        let l = &self.library;
        let status = match &l.read {
            None => LibraryStatus::NotRead,
            Some(Err(failure)) => LibraryStatus::Failed(failure),
            Some(Ok(report)) => LibraryStatus::Read(report.facts()),
        };
        evaluate_library(cfg, status, l.checked_at, now)
    }

    /// `overview.library` and `library` in `GET /library`; `None` without `[library]`. The age
    /// is computed at `now`.
    pub fn library_view(&self, config: &Config, now: i64) -> Option<LibrarySummary> {
        let cfg = config.library.as_ref()?;
        let a = self.library_assessment(cfg, now);
        let facts = self.loaded_report().map(LoadedReport::facts);
        Some(LibrarySummary {
            name: cfg.name.clone(),
            health: a.health,
            message: a.message,
            checked_at: self.library.checked_at,
            generated_at: facts.map(|f| f.generated_at),
            age: facts.map(|f| now.saturating_sub(f.generated_at).max(0)),
            max_age: cfg.max_age.as_secs(),
            warn_on: cfg.warn_on,
            issue_counts: facts.map(|f| f.issue_counts),
            unreadable_files: facts.map(|f| f.unreadable_files),
            albums: facts.map(|f| f.albums),
        })
    }

    /// The view of the last valid report, if the last read was one.
    pub fn library_report(&self) -> Option<LibraryReport> {
        self.loaded_report().map(|r| r.view.clone())
    }

    fn loaded_report(&self) -> Option<&LoadedReport> {
        self.library.read.as_ref().and_then(|r| r.as_deref().ok())
    }

    /// The `host` field of every response.
    pub fn host(&self) -> &str {
        self.host_info
            .as_ref()
            .map_or(UNKNOWN_HOST, |h| h.hostname.as_str())
    }

    pub fn overview(&self, config: &Config, now: i64) -> Overview {
        let system = self.system_view(config, now);
        let services = self.services_view(config, now);
        let timers = self.timers_view(config, now);
        let checks = self.checks_view(config, now);
        let library = self.library_view(config, now);
        let parts: Vec<Health> = std::iter::once(system.health)
            .chain(services.iter().map(|s| s.health))
            .chain(timers.iter().map(|t| t.health))
            .chain(checks.iter().map(|c| c.health))
            .chain(library.iter().map(|l| l.health))
            .collect();
        Overview {
            host: self.host().to_owned(),
            ts: now,
            health: health::worst(parts.iter().copied()),
            host_info: self.host_info_view(now),
            system,
            counts: health::tally(parts),
            services,
            timers,
            checks,
            library,
            // Events come from storage; the HTTP handler adds them.
            recent_events: Vec::new(),
        }
    }

    pub fn services_view(&self, config: &Config, now: i64) -> Vec<ServiceSummary> {
        config
            .services
            .iter()
            .map(|cfg| self.service_view(config, cfg, now))
            .collect()
    }

    fn service_assessment(&self, config: &Config, cfg: &ServiceConfig, now: i64) -> Assessment {
        let u = &self.units;
        if let Some(stale) = self.units_stale(config, now) {
            return stale;
        }
        match u.by_unit.get(&cfg.unit) {
            Some(snap) => {
                let restarted = u
                    .restarts
                    .get(&cfg.unit)
                    .is_some_and(|t| t.restarted_recently(now));
                evaluate_service(cfg, snap, restarted)
            }
            None => no_unit_data(),
        }
    }

    /// `Some(unknown)` when the unit data is stale.
    fn units_stale(&self, config: &Config, now: i64) -> Option<Assessment> {
        let u = &self.units;
        is_stale(u.last_ok, now, config.collector.units_interval)
            .then(|| Assessment::unknown(stale_message(u.last_ok, u.error.as_deref(), now)))
    }

    pub fn service_view(&self, config: &Config, cfg: &ServiceConfig, now: i64) -> ServiceSummary {
        let snap = self.units.by_unit.get(&cfg.unit);
        let assessment = self.service_assessment(config, cfg, now);
        let text = |f: fn(&UnitSnapshot) -> &String| {
            snap.map_or_else(|| "unknown".to_owned(), |s| f(s).clone())
        };
        ServiceSummary {
            unit: cfg.unit.clone(),
            name: cfg.name.clone().unwrap_or_else(|| cfg.unit.clone()),
            description: snap.map(|s| s.description.clone()).unwrap_or_default(),
            health: assessment.health,
            message: assessment.message,
            load_state: text(|s| &s.load_state),
            active_state: text(|s| &s.active_state),
            sub_state: text(|s| &s.sub_state),
            since: snap.and_then(|s| s.active_enter_ts),
            restarts: snap.and_then(|s| s.n_restarts).unwrap_or(0),
            memory: snap.and_then(|s| s.memory_current),
        }
    }

    pub fn timers_view(&self, config: &Config, now: i64) -> Vec<TimerSummary> {
        config
            .timers
            .iter()
            .map(|cfg| self.timer_view(config, cfg, now))
            .collect()
    }

    /// Whether the timer's service is running, judged from the latest unit tick.
    fn timer_running(&self, cfg: &TimerConfig) -> bool {
        let by_unit = &self.units.by_unit;
        let service = timer_service(cfg, by_unit.get(&cfg.unit));
        by_unit.get(&service).is_some_and(service_is_running)
    }

    fn timer_assessment(&self, config: &Config, cfg: &TimerConfig, now: i64) -> Assessment {
        if let Some(stale) = self.units_stale(config, now) {
            return stale;
        }
        let Some(snap) = self.units.by_unit.get(&cfg.unit) else {
            return no_unit_data();
        };
        let runs = self
            .units
            .timer_runs
            .get(&cfg.unit)
            .cloned()
            .unwrap_or_default();
        evaluate_timer(cfg, snap, self.timer_running(cfg), &runs, now)
    }

    pub fn timer_view(&self, config: &Config, cfg: &TimerConfig, now: i64) -> TimerSummary {
        let snap = self.units.by_unit.get(&cfg.unit);
        let assessment = self.timer_assessment(config, cfg, now);
        TimerSummary {
            unit: cfg.unit.clone(),
            name: cfg.name.clone().unwrap_or_else(|| cfg.unit.clone()),
            service: timer_service(cfg, snap),
            health: assessment.health,
            message: assessment.message,
            active: snap.is_some_and(|s| s.active_state == "active"),
            last_run: self
                .units
                .timer_runs
                .get(&cfg.unit)
                .and_then(|r| r.last.clone()),
            next_run: snap.and_then(|s| s.next_elapse_ts),
            running: self.timer_running(cfg),
        }
    }

    pub fn checks_view(&self, config: &Config, now: i64) -> Vec<CheckSummary> {
        config
            .checks
            .iter()
            .map(|cfg| self.check_view(cfg, now))
            .collect()
    }

    fn check_assessment(&self, cfg: &CheckConfig, now: i64) -> Assessment {
        let empty = CheckTracker::default();
        evaluate_check(cfg, self.checks.get(&cfg.name).unwrap_or(&empty), now)
    }

    pub fn check_view(&self, cfg: &CheckConfig, now: i64) -> CheckSummary {
        let tracker = self.checks.get(&cfg.name);
        let last = tracker.and_then(|t| t.last.as_ref());
        let assessment = self.check_assessment(cfg, now);
        CheckSummary {
            name: cfg.name.clone(),
            kind: cfg.kind,
            target: cfg.display_target(),
            health: assessment.health,
            message: assessment.message,
            last_check: last.map(|(ts, _)| *ts),
            latency_ms: last
                .and_then(|(_, o)| o.latency_ms())
                .map(|ms| round(ms, 1)),
            // A count seeded at restore is not shown before the first probe of this process.
            consecutive_failures: tracker
                .filter(|t| t.last.is_some())
                .map_or(0, |t| t.consecutive_failures),
        }
    }

    fn system_health(&self, config: &Config, snap: &SystemSnapshot) -> SystemHealth {
        evaluate_system(
            &config.thresholds,
            &config.system.disks,
            snap,
            self.system.cpu_window.mean(),
            self.system.load_window.mean(),
        )
    }

    fn host_info_view(&self, now: i64) -> HostInfoDto {
        let Some(h) = &self.host_info else {
            return HostInfoDto {
                hostname: UNKNOWN_HOST.to_owned(),
                os: String::new(),
                kernel: String::new(),
                cpu_model: String::new(),
                cores: 0,
                boot_time: 0,
                uptime: 0,
            };
        };
        // Prefer the kernel's uptime from the latest sample; fall back to the boot time.
        let uptime = self
            .system
            .last
            .as_ref()
            .map_or((now - h.boot_time) as f64, |s| s.uptime_s);
        HostInfoDto {
            hostname: h.hostname.clone(),
            os: h.os.clone(),
            kernel: h.kernel.clone(),
            cpu_model: h.cpu_model.clone(),
            cores: h.cores,
            boot_time: h.boot_time,
            uptime: uptime.max(0.0) as u64,
        }
    }

    fn system_view(&self, config: &Config, now: i64) -> SystemSummary {
        let s = &self.system;
        let Some(snap) = &s.last else {
            return empty_system();
        };
        let h = self.system_health(config, snap);
        // Stale data keeps its values, so the UI can still show them, but every metric judged
        // from it is unknown (§8). Missing configured disks stay critical: they are reported
        // whether or not samples arrive.
        let stale = is_stale(s.last_ok, now, config.collector.system_interval);
        let judged = |health: Health| if stale { Health::Unknown } else { health };
        let overall = if stale {
            health::worst(
                std::iter::once(Health::Unknown)
                    .chain(h.missing_disks.iter().map(|(_, a)| a.health)),
            )
        } else {
            h.overall().health
        };
        let mem = &snap.memory;
        SystemSummary {
            health: overall,
            ts: snap.ts,
            cpu: CpuSummary {
                usage: round(s.cpu.map_or(0.0, |c| c.usage), 1),
                iowait: round(s.cpu.map_or(0.0, |c| c.iowait), 1),
                health: judged(h.cpu.health),
            },
            load: LoadSummary {
                one: round(snap.load[0], 2),
                five: round(snap.load[1], 2),
                fifteen: round(snap.load[2], 2),
                health: judged(h.load.health),
            },
            memory: MemorySummary {
                total: mem.total,
                used: mem.total.saturating_sub(mem.available),
                used_pct: round(memory_used_pct(mem).unwrap_or(0.0), 1),
                swap_total: mem.swap_total,
                swap_used: mem.swap_total.saturating_sub(mem.swap_free),
                health: judged(h.memory_card()),
            },
            disks: snap
                .disks
                .iter()
                .zip(&h.disks)
                .map(|(d, a)| DiskSummary {
                    mount: d.mount.clone(),
                    fs_type: d.fs_type.clone(),
                    total: d.total,
                    used: d.used,
                    used_pct: round(disk_used_pct(d).unwrap_or(0.0), 1),
                    health: judged(a.health),
                })
                // A configured disk that is not mounted still gets a (zeroed, critical) row, so
                // it cannot silently disappear from the dashboard.
                .chain(h.missing_disks.iter().map(|(mount, a)| DiskSummary {
                    mount: mount.clone(),
                    fs_type: String::new(),
                    total: 0,
                    used: 0,
                    used_pct: 0.0,
                    health: a.health,
                }))
                .collect(),
            net: s
                .net
                .iter()
                .map(|n| NetSummary {
                    iface: n.iface.clone(),
                    rx_bps: round(n.rx_bps, 1),
                    tx_bps: round(n.tx_bps, 1),
                })
                .collect(),
            temps: snap
                .temps
                .iter()
                .zip(&h.temps)
                .map(|(t, a)| TempSummary {
                    label: t.label.clone(),
                    celsius: t.celsius,
                    health: judged(a.health),
                })
                .collect(),
        }
    }
}

/// The system before the first successful sample: all zeros, unknown. `ts` is 0 rather than now,
/// so nothing mistakes the placeholder for a fresh sample.
fn empty_system() -> SystemSummary {
    SystemSummary {
        health: Health::Unknown,
        ts: 0,
        cpu: CpuSummary {
            usage: 0.0,
            iowait: 0.0,
            health: Health::Unknown,
        },
        load: LoadSummary {
            one: 0.0,
            five: 0.0,
            fifteen: 0.0,
            health: Health::Unknown,
        },
        memory: MemorySummary {
            total: 0,
            used: 0,
            used_pct: 0.0,
            swap_total: 0,
            swap_used: 0,
            health: Health::Unknown,
        },
        disks: Vec::new(),
        net: Vec::new(),
        temps: Vec::new(),
    }
}

fn no_unit_data() -> Assessment {
    Assessment::unknown("no data from collector for this unit")
}

/// The message for a stale target: the last collection error if there was one.
fn stale_message(last_ok: Option<i64>, error: Option<&str>, now: i64) -> String {
    match (last_ok, error) {
        (_, Some(e)) => format!("collection failed: {e}"),
        (None, None) => "waiting for first sample".to_owned(),
        (Some(ts), None) => format!("no data for {}s", now.saturating_sub(ts)),
    }
}

fn round(v: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (v * f).round() / f
}

#[cfg(test)]
mod tests {
    use gargoyle_collector::MockCollector;
    use gargoyle_core::model::UnitRequest;

    use super::*;

    const BOOT: i64 = 1_794_000_000;
    const T0: i64 = BOOT + 86_400;

    fn config() -> Config {
        Config::from_toml_str(
            r#"
            [collector]
            mode = "mock"
            [[service]]
            unit = "navidrome.service"
            name = "Navidrome"
            [[service]]
            unit = "backup-fail.service"
            [[service]]
            unit = "nginx.service"
            "#,
        )
        .expect("valid config")
    }

    fn mock() -> MockCollector {
        MockCollector::with_boot_time(42, BOOT)
    }

    fn requests(config: &Config) -> Vec<UnitRequest> {
        config
            .services
            .iter()
            .map(|s| UnitRequest::new(&s.unit))
            .collect()
    }

    /// State after `n` system ticks (10s apart, ending at `T0`) and one unit tick at `T0`.
    fn populated(n: i64) -> (Config, State) {
        let config = config();
        let m = mock();
        let mut state = State::new(&config);
        state.record_host_info(
            gargoyle_collector::Collector::host_info(&m).expect("mock never fails"),
        );
        for i in (0..n).rev() {
            state.record_system(m.snapshot_at(T0 - i * 10));
        }
        state.record_units(T0, m.units_at(&requests(&config), T0));
        (config, state)
    }

    #[test]
    fn empty_state_is_unknown_with_placeholders() {
        let config = config();
        let state = State::new(&config);
        let o = state.overview(&config, T0);
        assert_eq!(o.host, "unknown");
        assert_eq!(o.system.health, Health::Unknown);
        assert_eq!(o.system.ts, 0);
        assert_eq!(o.health, Health::Unknown);
        assert_eq!(o.services.len(), 3);
        for s in &o.services {
            assert_eq!(s.health, Health::Unknown);
            assert_eq!(s.message.as_deref(), Some("waiting for first sample"));
        }
        assert_eq!(o.counts.unknown, 4);
    }

    #[test]
    fn first_sample_has_no_cpu_yet() {
        let (config, state) = populated(1);
        let o = state.overview(&config, T0);
        assert_eq!(o.system.cpu.health, Health::Unknown);
        assert_eq!(o.system.cpu.usage, 0.0);
        assert!(o.system.net.is_empty());
    }

    #[test]
    fn populated_overview() {
        let (config, state) = populated(12);
        let o = state.overview(&config, T0 + 5);
        assert_eq!(o.host, "gargoyle-mock");
        assert_eq!(o.host_info.uptime, (T0 - BOOT) as u64);
        assert_eq!(o.system.ts, T0);
        assert_ne!(o.system.cpu.health, Health::Unknown);
        assert!(o.system.cpu.usage > 0.0);
        assert_eq!(o.system.disks.len(), 2);
        assert_eq!(o.system.net.len(), 1);
        assert!(o.system.memory.used_pct > 0.0);

        let by_unit = |u: &str| {
            o.services
                .iter()
                .find(|s| s.unit == u)
                .expect("configured")
                .clone()
        };
        let nav = by_unit("navidrome.service");
        assert_eq!(nav.name, "Navidrome");
        assert_eq!(nav.health, Health::Healthy);
        assert_eq!(nav.message, None);
        let fail = by_unit("backup-fail.service");
        assert_eq!(fail.name, "backup-fail.service");
        assert_eq!(fail.health, Health::Critical);
        assert_eq!(
            fail.message.as_deref(),
            Some("failed: exit-code (status 1)")
        );
        assert_eq!(by_unit("nginx.service").health, Health::Warning);

        let total = o.counts.healthy + o.counts.warning + o.counts.critical + o.counts.unknown;
        assert_eq!(total, 4, "system + 3 services");
        assert_eq!(o.health, Health::Critical);
        assert!(o.timers.is_empty() && o.checks.is_empty() && o.recent_events.is_empty());
    }

    #[test]
    fn stale_targets_become_unknown() {
        let (config, state) = populated(12);
        // system interval 10s, units 30s: 3× is 30s and 90s.
        let fresh = state.overview(&config, T0 + 30);
        assert_ne!(fresh.system.health, Health::Unknown);
        assert_ne!(fresh.system.cpu.health, Health::Unknown);

        let o = state.overview(&config, T0 + 31);
        let sys = &o.system;
        assert_eq!(sys.health, Health::Unknown);
        assert_eq!(sys.cpu.health, Health::Unknown);
        assert_eq!(sys.load.health, Health::Unknown);
        assert_eq!(sys.memory.health, Health::Unknown);
        assert!(sys.disks.iter().all(|d| d.health == Health::Unknown));
        assert!(sys.temps.iter().all(|t| t.health == Health::Unknown));
        // The last values stay visible.
        assert_eq!(sys.ts, fresh.system.ts);
        assert_eq!(sys.disks, {
            let mut d = fresh.system.disks.clone();
            d.iter_mut().for_each(|d| d.health = Health::Unknown);
            d
        });
        assert_eq!(sys.memory.used, fresh.system.memory.used);
        assert!(o.services.iter().all(|s| s.health != Health::Unknown));

        let o = state.overview(&config, T0 + 91);
        for s in &o.services {
            assert_eq!(s.health, Health::Unknown);
            assert_eq!(s.message.as_deref(), Some("no data for 91s"));
        }
    }

    #[test]
    fn collection_error_becomes_the_stale_message() {
        let (config, mut state) = populated(3);
        state.record_units_error("systemctl failed: exit status 1".into());
        // Not stale yet: the last good data still counts.
        let o = state.overview(&config, T0 + 5);
        assert_ne!(o.services[0].health, Health::Unknown);

        let o = state.overview(&config, T0 + 100);
        assert_eq!(
            o.services[0].message.as_deref(),
            Some("collection failed: systemctl failed: exit status 1")
        );
    }

    #[test]
    fn unit_missing_from_collector_output_is_unknown() {
        let (config, mut state) = populated(3);
        state.record_units(T0, Vec::new());
        let s = &state.services_view(&config, T0)[0];
        assert_eq!(s.health, Health::Unknown);
        assert_eq!(s.load_state, "unknown");
        assert_eq!(
            s.message.as_deref(),
            Some("no data from collector for this unit")
        );
    }

    #[test]
    fn restart_increase_warns_for_an_hour() {
        let (config, mut state) = populated(3);
        let mut snaps = mock().units_at(&requests(&config), T0);
        let nav = snaps
            .iter_mut()
            .find(|s| s.unit == "navidrome.service")
            .expect("requested");
        nav.n_restarts = nav.n_restarts.map(|n| n + 1);
        state.record_units(T0 + 30, snaps.clone());
        let view = |state: &State, now| state.services_view(&config, now)[0].clone();
        let s = view(&state, T0 + 30);
        assert_eq!(s.health, Health::Warning);
        assert!(
            s.message
                .as_deref()
                .is_some_and(|m| m.starts_with("restarted in the last hour")),
            "{s:?}"
        );
        // Still the same count an hour and a bit later: healthy again.
        state.record_units(T0 + 3700, snaps);
        assert_eq!(view(&state, T0 + 3700).health, Health::Healthy);
    }

    #[test]
    fn rounding() {
        assert_eq!(round(12.345, 1), 12.3);
        assert_eq!(round(0.4249, 2), 0.42);
    }

    #[test]
    fn unmounted_configured_disk_is_a_critical_row() {
        let mut config = config();
        config.system.disks = vec!["/".into(), "/backup".into()];
        let m = mock();
        let mut state = State::new(&config);
        state.record_system(m.snapshot_at(T0 - 10));
        state.record_system(m.snapshot_at(T0));
        let o = state.overview(&config, T0);
        let backup = o
            .system
            .disks
            .iter()
            .find(|d| d.mount == "/backup")
            .expect("missing disk still listed");
        assert_eq!(
            (
                backup.total,
                backup.used,
                backup.used_pct,
                backup.fs_type.as_str()
            ),
            (0, 0, 0.0, "")
        );
        assert_eq!(backup.health, Health::Critical);
        assert_eq!(o.system.health, Health::Critical);
        // "/srv" is mounted but not configured: still shown, as the collector reported it.
        assert_eq!(o.system.disks.len(), 3);

        // Stale: the mounted disks become unknown, the missing one stays critical.
        let o = state.overview(&config, T0 + 3600);
        let healths: Vec<_> = o
            .system
            .disks
            .iter()
            .map(|d| (d.mount.as_str(), d.health))
            .collect();
        assert_eq!(
            healths,
            [
                ("/", Health::Unknown),
                ("/srv", Health::Unknown),
                ("/backup", Health::Critical)
            ]
        );
        assert_eq!(o.system.health, Health::Critical);
    }
}
