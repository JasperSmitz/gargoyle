//! Periodic work (ARCHITECTURE §3): the system tick, the unit tick, one tick per check, the
//! library tick (`crate::library`) and the maintenance tick, plus the one-off startup work:
//! restoring state from storage and journal backfill.
//!
//! After each system, unit or check tick, the targets it covers are judged and every health
//! transition is stored as an event (see `gargoyle_core::health::Transitions` for the rules). The
//! unit tick also stores new and finished timer runs.
//!
//! Collector calls block (they read `/proc` or spawn `systemctl`), so each runs in
//! `spawn_blocking`. A failed collection is logged and recorded; it never stops the loop.
//!
//! Calls can also hang: `systemctl` on a stuck D-Bus, `statvfs` on a dead NFS mount. Each call is
//! awaited for at most [`call_limit`], and while a timed-out call of one kind is still running on
//! the blocking pool, further ticks of that kind are skipped, so hung threads cannot pile up.
//!
//! Storage is best effort: a failed write is logged and the live state keeps updating, so the
//! dashboard keeps working on a full or broken disk; only history is lost.

use std::fmt;
use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gargoyle_collector::{CollectError, Collector};
use gargoyle_core::api::RunSource;
use gargoyle_core::config::{CheckConfig, Config};
use gargoyle_core::health::{ProbeOutcome, Transition};
use gargoyle_core::model::{TimerRunRecord, UnitRequest};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::checks::Prober;
use crate::state::State;
use crate::storage::{Sample, Storage, samples};

/// How often rollup and retention run (§3).
pub const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(300);

/// The longest one timer's journal backfill (one `journalctl` run) is awaited.
const JOURNAL_LIMIT: Duration = Duration::from_secs(60);

/// Everything the scheduler and the HTTP handlers share.
#[derive(Clone)]
pub struct Shared {
    pub config: Arc<Config>,
    pub collector: Arc<dyn Collector>,
    pub state: Arc<RwLock<State>>,
    /// `None` only in tests that do not need history.
    pub storage: Option<Storage>,
    /// Set while writes fail, so a broken disk logs one warning instead of one per tick.
    storage_failing: Arc<StorageFailing>,
    /// Host info has its own flag, so a hung host-info read never costs a system sample.
    host_busy: Busy,
    system_busy: Busy,
    units_busy: Busy,
    journal_busy: Busy,
    /// How long one `timer_history` call is awaited; [`JOURNAL_LIMIT`] except in tests.
    journal_limit: Duration,
    /// Set once journal backfill has been started, after the first successful unit tick (which
    /// tells each timer's service, from its `Unit=` property).
    journal_started: Arc<AtomicBool>,
    /// Host info is retried every system tick until it works; only the first failure is a warning.
    host_info_warned: Arc<AtomicBool>,
}

impl Shared {
    pub fn new(config: Config, collector: Arc<dyn Collector>) -> Self {
        let state = State::new(&config);
        Self {
            config: Arc::new(config),
            collector,
            state: Arc::new(RwLock::new(state)),
            storage: None,
            storage_failing: Arc::default(),
            host_busy: Busy::default(),
            system_busy: Busy::default(),
            units_busy: Busy::default(),
            journal_busy: Busy::default(),
            journal_limit: JOURNAL_LIMIT,
            journal_started: Arc::default(),
            host_info_warned: Arc::default(),
        }
    }

    pub fn with_storage(mut self, storage: Storage) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Runs `f` with the state locked for writing. A panic while holding the lock cannot leave
    /// the plain data in it half-updated in a way that matters, so a poisoned lock is reused.
    pub fn write<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut guard = self.state.write().unwrap_or_else(PoisonError::into_inner);
        f(&mut guard)
    }

    pub fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        let guard = self.state.read().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }
}

/// Restores what storage knows from before this process (the last logged state of every target,
/// each timer's newest run and success) into `State`. Call before [`spawn`], so the first ticks
/// compare against it.
pub async fn restore(shared: &Shared) -> anyhow::Result<()> {
    let Some(storage) = &shared.storage else {
        return Ok(());
    };
    let states = storage.last_states().await?;
    let timers = shared
        .config
        .timers
        .iter()
        .map(|t| t.unit.clone())
        .collect();
    let timers = storage.timer_summaries(timers).await?;
    shared.write(|s| s.restore(states, timers));
    Ok(())
}

/// Starts the system, unit, check, library (with `[library]`) and (with storage) maintenance
/// loops. The system, unit and maintenance loops tick immediately, then every interval; the
/// immediate maintenance tick catches up on rollups missed while stopped. Each check starts after
/// its own [`check_jitter`], the library 1s after startup.
pub fn spawn(shared: &Shared) -> anyhow::Result<Vec<JoinHandle<()>>> {
    let c = &shared.config.collector;
    let mut loops = vec![
        spawn_loop(shared.clone(), c.system_interval, system_tick),
        spawn_loop(shared.clone(), c.units_interval, unit_tick),
    ];
    if shared.storage.is_some() {
        loops.push(spawn_loop(
            shared.clone(),
            MAINTENANCE_INTERVAL,
            maintenance_tick,
        ));
    }
    if !shared.config.checks.is_empty() {
        let prober = Prober::new()?;
        for index in 0..shared.config.checks.len() {
            loops.push(spawn_check_loop(shared.clone(), prober.clone(), index));
        }
    }
    loops.extend(crate::library::spawn(shared));
    Ok(loops)
}

/// The longest a check waits before its first probe, beyond [`CHECK_MIN_DELAY`].
const CHECK_MAX_JITTER: Duration = Duration::from_secs(10);
/// Every check waits at least this long, so a check of Gargoyle's own API does not race the HTTP
/// server binding its port.
const CHECK_MIN_DELAY: Duration = Duration::from_secs(1);

/// A random delay before a check's first probe, so checks with the same interval do not all fire
/// in the same instant: [`CHECK_MIN_DELAY`] plus up to `min(interval, CHECK_MAX_JITTER)`.
fn check_jitter(interval: Duration) -> Duration {
    let span = u64::try_from(interval.min(CHECK_MAX_JITTER).as_millis()).unwrap_or(u64::MAX);
    // A freshly keyed `RandomState` is a random source from std, which is all this needs.
    let random = RandomState::new().hash_one(0u8);
    CHECK_MIN_DELAY + Duration::from_millis(random % span.max(1))
}

fn spawn_check_loop(shared: Shared, prober: Prober, index: usize) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(cfg) = shared.config.checks.get(index).cloned() else {
            return;
        };
        let start = tokio::time::Instant::now() + check_jitter(cfg.interval);
        let mut interval = tokio::time::interval_at(start, cfg.interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            check_tick(&shared, &prober, &cfg).await;
        }
    })
}

/// One probe of one check. The probe is bounded by the check's timeout, which config validation
/// keeps within its interval, so probes of one check never overlap.
pub async fn check_tick(shared: &Shared, prober: &Prober, cfg: &CheckConfig) {
    let outcome = prober.probe(cfg).await;
    let ts = now();
    if let ProbeOutcome::Failure { reason, .. } = &outcome {
        // The target is shown sanitized; the raw URL is never logged.
        tracing::debug!(check = %cfg.name, target = %cfg.display_target(), %reason, "check failed");
    }
    let sample = outcome
        .latency_ms()
        .and_then(|ms| samples::check(ts, &cfg.name, ms));
    let events = shared.write(|s| {
        s.record_check(&cfg.name, ts, outcome);
        let target = s.check_target(cfg, ts);
        s.transitions(vec![target])
    });
    store(shared, sample.into_iter().collect()).await;
    store_events(shared, ts, events).await;
}

fn spawn_loop<F, Fut>(shared: Shared, every: Duration, tick: F) -> JoinHandle<()>
where
    F: Fn(Shared) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(every);
        // After a stall (e.g. suspend), sample once and keep the cadence rather than bursting.
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            tick(shared.clone()).await;
        }
    })
}

/// One system sample. Host info is collected on the first tick, and retried on later ticks until
/// it succeeds; after that only uptime changes, and that comes from the system sample. A failed
/// host-info read does not stop the sample.
pub async fn system_tick(shared: Shared) {
    let limit = call_limit(shared.config.collector.system_interval);
    if !shared.read(State::has_host_info) {
        let collector = shared.collector.clone();
        match call(&shared.host_busy, limit, move || collector.host_info()).await {
            Ok(info) => shared.write(|s| s.record_host_info(info)),
            Err(e) if !shared.host_info_warned.swap(true, Ordering::Relaxed) => {
                tracing::warn!(error = %e, "cannot read host info; will keep retrying quietly");
            }
            Err(e) => tracing::debug!(error = %e, "cannot read host info"),
        }
    }

    let collector = shared.collector.clone();
    match call(&shared.system_busy, limit, move || {
        collector.system_snapshot()
    })
    .await
    {
        Ok(snap) => {
            let samples = shared.write(|s| {
                s.record_system(snap);
                s.system_samples()
            });
            store(&shared, samples).await;
        }
        // The system summary has no message; the error reaches the messages of system events
        // once the last good sample is stale.
        Err(e) => {
            tracing::warn!(error = %e, "system collection failed");
            shared.write(|s| s.record_system_error(e.to_string()));
        }
    }
    let now = now();
    let events = shared.write(|s| {
        let targets = s.system_targets(&shared.config, now);
        s.transitions(targets)
    });
    store_events(&shared, now, events).await;
}

/// One batched poll of every configured service and timer.
pub async fn unit_tick(shared: Shared) {
    let config = &shared.config;
    let requests: Vec<UnitRequest> = config
        .services
        .iter()
        .map(|s| UnitRequest::new(&s.unit))
        .chain(config.timers.iter().map(|t| UnitRequest {
            unit: t.unit.clone(),
            service: t.service.clone(),
        }))
        .collect();
    if requests.is_empty() {
        return;
    }
    let limit = call_limit(shared.config.collector.units_interval);
    let collector = shared.collector.clone();
    match call(&shared.units_busy, limit, move || {
        collector.units(&requests)
    })
    .await
    {
        Ok(units) => {
            let now = now();
            let samples = samples::services(now, &units, |unit| {
                config.services.iter().any(|s| s.unit == unit)
            });
            let runs = shared.write(|s| {
                s.record_units(now, units);
                s.observe_timer_runs(config)
            });
            store(&shared, samples).await;
            store_runs(&shared, runs).await;
            if config.collector.journal_backfill
                && shared.storage.is_some()
                && !config.timers.is_empty()
                && !shared.journal_started.swap(true, Ordering::Relaxed)
            {
                tokio::spawn(backfill_journal(shared.clone()));
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "unit collection failed");
            shared.write(|s| s.record_units_error(e.to_string()));
        }
    }
    let now = now();
    let events = shared.write(|s| {
        let targets = s.unit_targets(config, now);
        s.transitions(targets)
    });
    store_events(&shared, now, events).await;
}

/// Imports past runs of every configured timer from the journal (M3.4), from the newest run
/// stored before startup, or 30 days back. Runs one timer at a time; an error is logged (one
/// warning, the rest at debug) and the next timer is tried, while a timeout stops the backfill.
/// Collectors without journal access return no runs.
pub async fn backfill_journal(shared: Shared) {
    let Some(storage) = shared.storage.clone() else {
        return;
    };
    let requests = shared.read(|s| s.journal_requests(&shared.config, now()));
    // Usually the same cause (e.g. no journal access) for every timer: warn once.
    let mut warned = false;
    for (timer, service, since) in requests {
        let collector = shared.collector.clone();
        let (t, svc) = (timer.clone(), service.clone());
        let records = match call(&shared.journal_busy, shared.journal_limit, move || {
            collector.timer_history(&t, &svc, since)
        })
        .await
        {
            Ok(records) => records,
            // The hung `journalctl` keeps the busy flag set, so every further timer would fail
            // the same way: stop with one warning.
            Err(e @ (CallError::TimedOut(_) | CallError::Busy(_))) => {
                tracing::warn!(%timer, error = %e, "journal backfill stopped");
                return;
            }
            Err(e) => {
                if warned {
                    tracing::debug!(%timer, error = %e, "journal backfill failed");
                } else {
                    tracing::warn!(%timer, error = %e, "journal backfill failed");
                    warned = true;
                }
                continue;
            }
        };
        if records.is_empty() {
            continue;
        }
        // Stored under the configured timer name, whatever the collector called it.
        let records: Vec<TimerRunRecord> = records
            .into_iter()
            .map(|r| TimerRunRecord {
                timer: timer.clone(),
                ..r
            })
            .collect();
        tracing::info!(%timer, runs = records.len(), "imported timer runs from the journal");
        shared.write(|s| s.record_journal_runs(&timer, &records));
        if let Err(e) = storage.upsert_runs(records, RunSource::Journal).await {
            tracing::warn!(%timer, error = format!("{e:#}"), "cannot store journal runs");
        }
    }
}

/// Stores observed timer runs. If that fails, they are handed out again on the next tick.
async fn store_runs(shared: &Shared, runs: Vec<TimerRunRecord>) {
    let Some(storage) = &shared.storage else {
        return;
    };
    let result = storage.upsert_runs(runs, RunSource::Observed).await;
    if !report_storage(&shared.storage_failing.runs, "timer runs", result) {
        shared.write(State::forget_written_runs);
    }
}

/// Stores health transitions. A failed write loses those events (the live state is unaffected).
pub(crate) async fn store_events(shared: &Shared, ts: i64, events: Vec<Transition>) {
    for e in &events {
        tracing::info!(
            target_kind = ?e.kind,
            target = %e.target,
            from = e.from.map(|h| h.as_str()),
            to = %e.to,
            message = e.message.as_deref(),
            "health changed"
        );
    }
    let Some(storage) = &shared.storage else {
        return;
    };
    let result = storage.insert_events(ts, events).await;
    report_storage(&shared.storage_failing.events, "events", result);
}

/// Appends points to history.
async fn store(shared: &Shared, samples: Vec<Sample>) {
    let Some(storage) = &shared.storage else {
        return;
    };
    let result = storage.append(samples).await;
    report_storage(&shared.storage_failing.points, "history", result);
}

/// One flag per kind of write, so one kind failing while another works does not flap the logs.
#[derive(Default)]
struct StorageFailing {
    points: AtomicBool,
    runs: AtomicBool,
    events: AtomicBool,
}

/// Logs a failed write of `what` (once until the next success); never fatal. Returns whether
/// it worked.
fn report_storage(failing: &AtomicBool, what: &str, result: anyhow::Result<()>) -> bool {
    match result {
        Ok(()) => {
            if failing.swap(false, Ordering::Relaxed) {
                tracing::info!("storing {what} works again");
            }
            true
        }
        Err(e) => {
            if failing.swap(true, Ordering::Relaxed) {
                tracing::debug!(error = format!("{e:#}"), "cannot store {what}");
            } else {
                tracing::warn!(
                    error = format!("{e:#}"),
                    "cannot store {what}; live data is unaffected"
                );
            }
            false
        }
    }
}

/// Rollup, retention and the daily optimize.
pub async fn maintenance_tick(shared: Shared) {
    let Some(storage) = &shared.storage else {
        return;
    };
    match storage.maintain(now()).await {
        Ok(m) => tracing::debug!(?m, "storage maintenance done"),
        Err(e) => tracing::warn!(error = format!("{e:#}"), "storage maintenance failed"),
    }
}

/// The longest one collector call is awaited: one interval, but never more than 20s.
pub fn call_limit(interval: Duration) -> Duration {
    interval.min(Duration::from_secs(20))
}

/// Set while a call of one kind is running on the blocking pool, including after its caller gave
/// up waiting.
#[derive(Clone, Default)]
pub(crate) struct Busy(Arc<AtomicBool>);

/// Clears the [`Busy`] flag when the blocking call ends, however it ends.
struct ClearOnDrop(Arc<AtomicBool>);

impl Drop for ClearOnDrop {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
pub(crate) enum CallError {
    /// The previous call of this kind has not returned yet, so this tick was skipped. That call
    /// was given up on after the carried limit (calls of one kind are otherwise sequential).
    Busy(Duration),
    TimedOut(Duration),
    Panicked,
    Collect(CollectError),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Busy(d) => {
                write!(f, "still running (timed out after {}s)", d.as_secs())
            }
            CallError::TimedOut(d) => write!(f, "timed out after {}s", d.as_secs()),
            CallError::Panicked => f.write_str("collector crashed"),
            CallError::Collect(e) => e.fmt(f),
        }
    }
}

/// Runs a collector call on the blocking pool, waiting at most `limit`, unless the previous call
/// guarded by `busy` is still running. A panic in the call becomes an error, so one bad sample
/// does not take the loop down.
pub(crate) async fn call<T, F>(busy: &Busy, limit: Duration, f: F) -> Result<T, CallError>
where
    F: FnOnce() -> Result<T, CollectError> + Send + 'static,
    T: Send + 'static,
{
    if busy.0.swap(true, Ordering::AcqRel) {
        return Err(CallError::Busy(limit));
    }
    let guard = ClearOnDrop(busy.0.clone());
    let task = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        f()
    });
    match tokio::time::timeout(limit, task).await {
        // The task keeps running and keeps `busy` set until it returns.
        Err(_) => Err(CallError::TimedOut(limit)),
        Ok(Err(_join)) => Err(CallError::Panicked),
        Ok(Ok(result)) => result.map_err(CallError::Collect),
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use gargoyle_collector::MockCollector;
    use gargoyle_core::health::Health;
    use gargoyle_core::model::{HostInfo, SystemSnapshot, UnitSnapshot};

    use super::*;
    use crate::checks::Prober;
    use crate::storage::Storage;

    fn config() -> Config {
        Config::from_toml_str(
            "[collector]\nmode = \"mock\"\n[[service]]\nunit = \"backup-fail.service\"\n",
        )
        .expect("valid")
    }

    /// Fails every call.
    struct Broken;

    impl Collector for Broken {
        fn host_info(&self) -> Result<HostInfo, CollectError> {
            Err(err())
        }
        fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
            Err(err())
        }
        fn units(&self, _: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
            Err(err())
        }
    }

    fn err() -> CollectError {
        CollectError::Command {
            program: "systemctl".into(),
            message: "boom".into(),
        }
    }

    #[tokio::test]
    async fn ticks_fill_state() {
        let shared = Shared::new(config(), Arc::new(MockCollector::new(1)));
        system_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        let o = shared.read(|s| s.overview(&shared.config, now()));
        assert_eq!(o.host, "gargoyle-mock");
        assert_eq!(o.system.disks.len(), 2);
        assert_eq!(o.services[0].health, Health::Critical);
    }

    #[tokio::test]
    async fn failures_are_recorded_not_fatal() {
        let shared = Shared::new(config(), Arc::new(Broken));
        system_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        let o = shared.read(|s| s.overview(&shared.config, now()));
        assert_eq!(o.host, "unknown");
        assert_eq!(o.system.health, Health::Unknown);
        assert_eq!(
            o.services[0].message.as_deref(),
            Some("collection failed: systemctl failed: boom")
        );
    }

    /// `units()` blocks until released, counting calls.
    #[derive(Default)]
    struct Hanging {
        released: AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl Collector for Hanging {
        fn host_info(&self) -> Result<HostInfo, CollectError> {
            Err(err())
        }
        fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
            Err(err())
        }
        fn units(&self, _: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            while !self.released.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(Vec::new())
        }
    }

    /// Host info always fails; system snapshots come from the mock.
    struct NoHostInfo(MockCollector);

    impl Collector for NoHostInfo {
        fn host_info(&self) -> Result<HostInfo, CollectError> {
            Err(err())
        }
        fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
            self.0.system_snapshot()
        }
        fn units(&self, u: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
            self.0.units(u)
        }
    }

    #[tokio::test]
    async fn host_info_failure_does_not_stop_the_sample() {
        let shared = Shared::new(config(), Arc::new(NoHostInfo(MockCollector::new(1))));
        system_tick(shared.clone()).await;
        system_tick(shared.clone()).await;
        let o = shared.read(|s| s.overview(&shared.config, now()));
        assert_eq!(o.host, "unknown");
        assert_eq!(o.system.disks.len(), 2);
        assert!(shared.host_info_warned.load(Ordering::Relaxed));
    }

    #[test]
    fn call_limit_is_interval_capped_at_20s() {
        assert_eq!(call_limit(Duration::from_secs(10)), Duration::from_secs(10));
        assert_eq!(call_limit(Duration::from_secs(30)), Duration::from_secs(20));
    }

    #[tokio::test]
    async fn hung_call_times_out_and_blocks_further_calls_until_it_returns() {
        let config = Config::from_toml_str(
            "[collector]\nmode = \"mock\"\nunits_interval = \"1s\"\n\
             [[service]]\nunit = \"a.service\"\n",
        )
        .expect("valid");
        let hanging = Arc::new(Hanging::default());
        let shared = Shared::new(config, hanging.clone());
        let message = |shared: &Shared| {
            shared.read(|s| s.services_view(&shared.config, now())[0].message.clone())
        };

        unit_tick(shared.clone()).await;
        assert_eq!(hanging.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            message(&shared).as_deref(),
            Some("collection failed: timed out after 1s")
        );

        // Still hanging: the tick is skipped without a second call.
        unit_tick(shared.clone()).await;
        assert_eq!(hanging.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            message(&shared).as_deref(),
            Some("collection failed: still running (timed out after 1s)")
        );

        hanging.released.store(true, Ordering::SeqCst);
        for _ in 0..200 {
            if !shared.units_busy.0.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        unit_tick(shared.clone()).await;
        assert_eq!(hanging.calls.load(Ordering::SeqCst), 2);
        // Succeeded, but returned nothing for the unit.
        assert_eq!(
            message(&shared).as_deref(),
            Some("no data from collector for this unit")
        );
    }

    #[tokio::test]
    async fn panicking_collector_clears_busy() {
        struct Panics;
        impl Collector for Panics {
            fn host_info(&self) -> Result<HostInfo, CollectError> {
                Err(err())
            }
            fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
                Err(err())
            }
            fn units(&self, _: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
                panic!("collector bug")
            }
        }
        let shared = Shared::new(config(), Arc::new(Panics));
        unit_tick(shared.clone()).await;
        assert!(!shared.units_busy.0.load(Ordering::SeqCst));
        let msg = shared.read(|s| s.services_view(&shared.config, now())[0].message.clone());
        assert_eq!(msg.as_deref(), Some("collection failed: collector crashed"));
    }

    #[tokio::test]
    async fn spawned_loops_tick_immediately() {
        let shared = Shared::new(config(), Arc::new(MockCollector::new(1)));
        let handles = spawn(&shared).unwrap();
        for _ in 0..200 {
            if shared.read(|s| s.has_host_info()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(shared.read(|s| s.has_host_info()));
        for h in handles {
            h.abort();
        }
    }

    fn with_storage(dir: &tempfile::TempDir) -> Shared {
        let mut config = config();
        config.storage.path = dir.path().join("g.db");
        let storage = Storage::open(&config.storage).unwrap();
        Shared::new(config, Arc::new(MockCollector::new(1))).with_storage(storage)
    }

    #[tokio::test]
    async fn ticks_append_history() {
        let dir = tempfile::tempdir().unwrap();
        let shared = with_storage(&dir);
        system_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        let keys: Vec<String> = shared
            .storage
            .as_ref()
            .unwrap()
            .series()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.key)
            .collect();
        // The first system tick has no rates yet, so no cpu.* or net.* series.
        assert!(keys.contains(&"mem.used".to_owned()), "{keys:?}");
        assert!(keys.contains(&"load.15".to_owned()), "{keys:?}");
        assert!(!keys.iter().any(|k| k.starts_with("cpu.")), "{keys:?}");
        // backup-fail.service is failed, so it has no memory, but it reports NRestarts.
        assert!(
            !keys.iter().any(|k| k.starts_with("service.memory:")),
            "{keys:?}"
        );
        assert!(
            keys.contains(&"service.restarts:backup-fail.service".to_owned()),
            "{keys:?}"
        );
        maintenance_tick(shared.clone()).await;
    }

    #[tokio::test]
    async fn broken_storage_does_not_stop_collection() {
        let dir = tempfile::tempdir().unwrap();
        let shared = with_storage(&dir);
        // Break the schema behind the writer's back.
        let conn = rusqlite::Connection::open(dir.path().join("g.db")).unwrap();
        conn.execute_batch("DROP TABLE points_raw").unwrap();
        system_tick(shared.clone()).await;
        assert!(shared.storage_failing.points.load(Ordering::Relaxed));
        system_tick(shared.clone()).await;
        let o = shared.read(|s| s.overview(&shared.config, now()));
        assert_eq!(o.system.disks.len(), 2);
        assert!(o.system.ts > 0, "the sample was recorded");
        maintenance_tick(shared.clone()).await;
    }

    /// Units come from a script the test edits between ticks; system samples from the mock.
    #[derive(Default)]
    struct Scripted {
        units: std::sync::Mutex<Vec<UnitSnapshot>>,
        history: std::sync::Mutex<Vec<TimerRunRecord>>,
        /// `(timer, service, since)` of every `timer_history` call.
        history_calls: std::sync::Mutex<Vec<(String, String, i64)>>,
    }

    impl Scripted {
        fn set_service(&self, svc: UnitSnapshot) {
            let mut units = self.units.lock().unwrap();
            units.retain(|u| u.unit != svc.unit);
            units.push(svc);
        }
    }

    impl Collector for Scripted {
        fn host_info(&self) -> Result<HostInfo, CollectError> {
            MockCollector::new(1).host_info()
        }
        fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
            Err(err())
        }
        fn units(&self, _: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
            Ok(self.units.lock().unwrap().clone())
        }
        fn timer_history(
            &self,
            timer: &str,
            service: &str,
            since: i64,
        ) -> Result<Vec<TimerRunRecord>, CollectError> {
            self.history_calls
                .lock()
                .unwrap()
                .push((timer.into(), service.into(), since));
            Ok(self.history.lock().unwrap().clone())
        }
    }

    fn unit(name: &str, active: &str, sub: &str) -> UnitSnapshot {
        UnitSnapshot {
            unit: name.into(),
            load_state: "loaded".into(),
            active_state: active.into(),
            sub_state: sub.into(),
            description: String::new(),
            active_enter_ts: None,
            n_restarts: Some(0),
            result: Some("success".into()),
            exec_main_status: Some(0),
            exec_main_start_ts: None,
            exec_main_exit_ts: None,
            invocation_id: None,
            memory_current: None,
            last_trigger_ts: None,
            next_elapse_ts: None,
            trigger_unit: None,
        }
    }

    /// `sync.timer` triggers `sync-job.service` (via `Unit=`, not the default name).
    fn timer_unit() -> UnitSnapshot {
        UnitSnapshot {
            next_elapse_ts: Some(now() + 3600),
            trigger_unit: Some("sync-job.service".into()),
            ..unit("sync.timer", "active", "waiting")
        }
    }

    fn running(start: i64, id: &str) -> UnitSnapshot {
        UnitSnapshot {
            exec_main_start_ts: Some(start),
            invocation_id: Some(id.into()),
            ..unit("sync-job.service", "activating", "start")
        }
    }

    fn finished(start: i64, id: &str, status: i32) -> UnitSnapshot {
        let (active, result) = if status == 0 {
            ("inactive", "success")
        } else {
            ("failed", "exit-code")
        };
        UnitSnapshot {
            exec_main_start_ts: Some(start),
            exec_main_exit_ts: Some(start + 30),
            invocation_id: Some(id.into()),
            result: Some(result.into()),
            exec_main_status: Some(status),
            ..unit(
                "sync-job.service",
                active,
                if status == 0 { "dead" } else { "failed" },
            )
        }
    }

    fn timer_config(dir: &tempfile::TempDir, journal: bool) -> Config {
        let mut config = Config::from_toml_str(&format!(
            "[collector]\nmode = \"mock\"\njournal_backfill = {journal}\n\
             [[timer]]\nunit = \"sync.timer\"\n"
        ))
        .expect("valid");
        config.storage.path = dir.path().join("g.db");
        config
    }

    /// A process over the database in `dir`, restored from it as at startup.
    async fn start(dir: &tempfile::TempDir, collector: Arc<Scripted>, journal: bool) -> Shared {
        let config = timer_config(dir, journal);
        let storage = Storage::open(&config.storage).unwrap();
        let shared = Shared::new(config, collector).with_storage(storage);
        restore(&shared).await.unwrap();
        shared
    }

    /// `(started_at, finished_at, result, exit_status, source)` of every stored run, oldest first.
    type RunRow = (i64, Option<i64>, Option<String>, Option<i32>, String);

    fn stored_runs(dir: &tempfile::TempDir) -> Vec<RunRow> {
        let conn = rusqlite::Connection::open(dir.path().join("g.db")).unwrap();
        conn.prepare(
            "SELECT started_at, finished_at, result, exit_status, source FROM timer_runs
             WHERE timer = 'sync.timer' AND service = 'sync-job.service' ORDER BY started_at",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    /// `(target, from, to)` of every stored event, oldest first.
    fn stored_events(dir: &tempfile::TempDir) -> Vec<(String, Option<String>, String)> {
        let conn = rusqlite::Connection::open(dir.path().join("g.db")).unwrap();
        conn.prepare("SELECT target, from_state, to_state FROM events ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn timer_health(shared: &Shared) -> (Health, Option<String>) {
        let t = shared.read(|s| s.timers_view(&shared.config, now())[0].clone());
        (t.health, t.message)
    }

    #[tokio::test]
    async fn timer_runs_are_detected_completed_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let script = Arc::new(Scripted::default());
        let t0 = now() - 7200;
        script.units.lock().unwrap().push(timer_unit());

        let shared = start(&dir, script.clone(), false).await;
        // Never run since boot: no run, timer unknown.
        script.set_service(unit("sync-job.service", "inactive", "dead"));
        unit_tick(shared.clone()).await;
        assert!(stored_runs(&dir).is_empty());
        assert_eq!(
            timer_health(&shared),
            (Health::Unknown, Some("never run".into()))
        );

        // A run starts: stored without an end; the timer is healthy while it runs.
        script.set_service(running(t0, "aaa"));
        unit_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        assert_eq!(
            stored_runs(&dir),
            [(t0, None, None, None, "observed".into())]
        );
        assert_eq!(timer_health(&shared).0, Health::Healthy);
        assert!(shared.read(|s| s.timers_view(&shared.config, now())[0].running));

        // It fails: the same row is completed.
        script.set_service(finished(t0, "aaa", 1));
        unit_tick(shared.clone()).await;
        let failed = (
            t0,
            Some(t0 + 30),
            Some("exit-code".into()),
            Some(1),
            "observed".into(),
        );
        assert_eq!(stored_runs(&dir), std::slice::from_ref(&failed));
        assert_eq!(
            timer_health(&shared),
            (
                Health::Critical,
                Some("last run failed: exit-code (status 1)".into())
            )
        );

        // Restart: the same snapshot again inserts nothing new, and the stored run is the
        // timer's last run before the first tick.
        drop(shared);
        let shared = start(&dir, script.clone(), false).await;
        let last = shared.read(|s| s.timers_view(&shared.config, now())[0].last_run.clone());
        assert_eq!(last.map(|r| r.started_at), Some(t0));
        unit_tick(shared.clone()).await;
        assert_eq!(stored_runs(&dir), std::slice::from_ref(&failed));

        // A new invocation is a new row; it succeeds.
        script.set_service(running(t0 + 3600, "bbb"));
        unit_tick(shared.clone()).await;
        script.set_service(finished(t0 + 3600, "bbb", 0));
        unit_tick(shared.clone()).await;
        assert_eq!(
            stored_runs(&dir),
            [
                failed,
                (
                    t0 + 3600,
                    Some(t0 + 3630),
                    Some("success".into()),
                    Some(0),
                    "observed".into()
                )
            ]
        );
        assert_eq!(timer_health(&shared), (Health::Healthy, None));
    }

    #[tokio::test]
    async fn timer_failure_and_recovery_are_events_once() {
        let dir = tempfile::tempdir().unwrap();
        let script = Arc::new(Scripted::default());
        let t0 = now() - 7200;
        script.units.lock().unwrap().push(timer_unit());
        script.set_service(finished(t0, "aaa", 0));

        let shared = start(&dir, script.clone(), false).await;
        unit_tick(shared.clone()).await;
        // Healthy at first sight: nothing logged.
        assert!(stored_events(&dir).is_empty());

        script.set_service(finished(t0 + 3600, "bbb", 1));
        unit_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        let to_critical = (
            "sync.timer".to_owned(),
            Some("healthy".to_owned()),
            "critical".to_owned(),
        );
        assert_eq!(stored_events(&dir), std::slice::from_ref(&to_critical));

        // A restart does not log the critical state again.
        drop(shared);
        let shared = start(&dir, script.clone(), false).await;
        unit_tick(shared.clone()).await;
        assert_eq!(stored_events(&dir), std::slice::from_ref(&to_critical));

        script.set_service(finished(t0 + 7200, "ccc", 0));
        unit_tick(shared.clone()).await;
        // Held down until seen twice.
        assert_eq!(stored_events(&dir).len(), 1);
        unit_tick(shared.clone()).await;
        let events = stored_events(&dir);
        assert_eq!(
            events,
            [
                to_critical,
                (
                    "sync.timer".to_owned(),
                    Some("critical".to_owned()),
                    "healthy".to_owned()
                )
            ]
        );
        let detail = shared
            .storage
            .as_ref()
            .unwrap()
            .events(crate::storage::EventFilter {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            detail[1].message.as_deref(),
            Some("last run failed: exit-code (status 1)")
        );
        assert_eq!(detail[0].message, None);
    }

    #[tokio::test]
    async fn failing_unit_collection_logs_unknown_after_staleness() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = timer_config(&dir, false);
        config.services.push(
            Config::from_toml_str("[[service]]\nunit = \"backup-fail.service\"\n")
                .unwrap()
                .services
                .remove(0),
        );
        let storage = Storage::open(&config.storage).unwrap();
        let shared = Shared::new(config, Arc::new(MockCollector::new(1))).with_storage(storage);
        unit_tick(shared.clone()).await;
        assert!(stored_events(&dir).is_empty(), "held down");
        unit_tick(shared.clone()).await;
        let first = stored_events(&dir);
        assert!(first.contains(&("backup-fail.service".into(), None, "critical".into())));
        // Pretend the last good tick was long ago and collection now fails.
        shared.write(|s| {
            s.record_units(now() - 3600, Vec::new());
            s.record_units_error("systemctl failed: boom".into());
        });
        let evaluate = || {
            shared.write(|s| {
                let targets = s.unit_targets(&shared.config, now());
                s.transitions(targets)
            })
        };
        assert!(evaluate().is_empty(), "held down");
        let events = evaluate();
        assert!(events.iter().all(|e| e.to == Health::Unknown), "{events:?}");
        assert!(events.iter().any(|e| e.target == "backup-fail.service"
            && e.from == Some(Health::Critical)
            && e.message.as_deref() == Some("collection failed: systemctl failed: boom")));
    }

    #[tokio::test]
    async fn journal_backfill_imports_runs_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let script = Arc::new(Scripted::default());
        let t0 = now() - 86_400;
        script.units.lock().unwrap().push(timer_unit());
        script.set_service(finished(t0 + 7200, "ccc", 0));
        let record = |start: i64, status: i32| TimerRunRecord {
            timer: "whatever-the-journal-says".into(),
            service: "sync-job.service".into(),
            invocation_id: None,
            started_at: start,
            finished_at: Some(start + 30),
            result: Some(if status == 0 { "success" } else { "exit-code" }.into()),
            exit_status: Some(status),
        };
        // The journal also has the run the unit tick observes.
        *script.history.lock().unwrap() =
            vec![record(t0, 1), record(t0 + 3600, 0), record(t0 + 7200, 0)];

        // With journal_backfill on, the first unit tick starts the backfill in the background.
        let shared = start(&dir, script.clone(), true).await;
        unit_tick(shared.clone()).await;
        for _ in 0..200 {
            if stored_runs(&dir).len() == 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let calls = script.history_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        let (timer, service, since) = &calls[0];
        assert_eq!(
            (timer.as_str(), service.as_str()),
            ("sync.timer", "sync-job.service")
        );
        let thirty_days = now() - 30 * 86_400;
        assert!((thirty_days - 5..=thirty_days).contains(since), "{since}");
        let sources: Vec<String> = stored_runs(&dir).into_iter().map(|r| r.4).collect();
        // The observed run keeps its source.
        assert_eq!(sources, ["journal", "journal", "observed"]);
        // A second tick does not start it again.
        unit_tick(shared.clone()).await;
        assert_eq!(script.history_calls.lock().unwrap().len(), 1);

        // After a restart it continues from the newest stored run, and inserts no duplicates.
        drop(shared);
        let shared = start(&dir, script.clone(), true).await;
        unit_tick(shared.clone()).await;
        backfill_journal(shared.clone()).await;
        assert_eq!(stored_runs(&dir).len(), 3);
        let calls = script.history_calls.lock().unwrap().clone();
        assert!(calls[1..].iter().all(|c| c.2 == t0 + 7200), "{calls:?}");
    }

    /// `timer_history` hangs for a while, counting calls.
    #[derive(Default)]
    struct SlowJournal(std::sync::atomic::AtomicUsize);

    impl Collector for SlowJournal {
        fn host_info(&self) -> Result<HostInfo, CollectError> {
            Err(err())
        }
        fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
            Err(err())
        }
        fn units(&self, _: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
            Ok(Vec::new())
        }
        fn timer_history(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<Vec<TimerRunRecord>, CollectError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(300));
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn journal_timeout_stops_the_backfill() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = timer_config(&dir, false);
        config.timers.push(config.timers[0].clone());
        config.timers[1].unit = "other.timer".into();
        let storage = Storage::open(&config.storage).unwrap();
        let slow = Arc::new(SlowJournal::default());
        let mut shared = Shared::new(config, slow.clone()).with_storage(storage);
        shared.journal_limit = Duration::from_millis(20);
        backfill_journal(shared.clone()).await;
        // The second timer is not tried behind the hung call.
        assert_eq!(slow.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn journal_runs_feed_timer_health() {
        let dir = tempfile::tempdir().unwrap();
        let script = Arc::new(Scripted::default());
        let t0 = now() - 86_400;
        script.units.lock().unwrap().push(timer_unit());
        // Not run since boot, but the journal knows a failed run.
        script.set_service(unit("sync-job.service", "inactive", "dead"));
        *script.history.lock().unwrap() = vec![TimerRunRecord {
            timer: "sync.timer".into(),
            service: "sync-job.service".into(),
            invocation_id: Some("x".into()),
            started_at: t0,
            finished_at: Some(t0 + 5),
            result: Some("exit-code".into()),
            exit_status: Some(2),
        }];
        let shared = start(&dir, script.clone(), false).await;
        unit_tick(shared.clone()).await;
        assert_eq!(timer_health(&shared).0, Health::Unknown);
        backfill_journal(shared.clone()).await;
        assert_eq!(
            timer_health(&shared),
            (
                Health::Critical,
                Some("last run failed: exit-code (status 2)".into())
            )
        );
        assert_eq!(stored_runs(&dir)[0].4, "journal");
    }

    /// A fake `systemctl show` that answers for every requested unit until the file `fail`
    /// exists next to it, then exits 1 like systemctl without a bus.
    fn fake_systemctl(dir: &std::path::Path, run_start: i64) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            r#"#!/bin/sh
if [ -e '{flag}' ]; then
    echo 'Failed to connect to bus: No such file or directory' >&2
    exit 1
fi
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done
shift
first=1
for unit in "$@"; do
    [ "$first" = 1 ] || echo
    first=0
    case "$unit" in
    *.timer) printf 'Id=%s\nLoadState=loaded\nActiveState=active\nSubState=waiting\nUnit=sync-job.service\nNextElapseUSecRealtime=@{next}\n' "$unit" ;;
    sync-job.service) printf 'Id=%s\nLoadState=loaded\nActiveState=inactive\nSubState=dead\nResult=success\nExecMainStartTimestamp=@{start}\nExecMainExitTimestamp=@{end}\nExecMainStatus=0\nInvocationID=abc\n' "$unit" ;;
    *) printf 'Id=%s\nLoadState=loaded\nActiveState=active\nSubState=running\nNRestarts=0\n' "$unit" ;;
    esac
done
"#,
            flag = dir.join("fail").display(),
            next = run_start + 7200,
            start = run_start,
            end = run_start + 5,
        );
        let path = dir.join("systemctl");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// ROADMAP M4 acceptance: when `systemctl` starts failing, services and timers turn
    /// `unknown` with the collection error as their message within 3 unit intervals, and that is
    /// logged as an event.
    #[tokio::test]
    async fn failing_systemctl_turns_units_unknown_within_three_intervals() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::from_toml_str(
            "[collector]\nunits_interval = \"1s\"\njournal_backfill = false\n\
             [[service]]\nunit = \"web.service\"\n\
             [[timer]]\nunit = \"sync.timer\"\nservice = \"sync-job.service\"\n",
        )
        .expect("valid");
        config.storage.path = dir.path().join("g.db");
        let collector = gargoyle_collector::LinuxCollector {
            root: dir.path().to_owned(),
            systemctl: fake_systemctl(dir.path(), now() - 600),
            journalctl: dir.path().join("no-journalctl"),
            system: config.system.clone(),
        };
        let storage = Storage::open(&config.storage).unwrap();
        let shared = Shared::new(config, Arc::new(collector)).with_storage(storage);
        let health_at = |at: i64| {
            shared.read(|s| {
                let services = s.services_view(&shared.config, at);
                let timers = s.timers_view(&shared.config, at);
                [
                    (services[0].health, services[0].message.clone()),
                    (timers[0].health, timers[0].message.clone()),
                ]
            })
        };

        // The good tick happens between `before` and `t0`, whatever second boundaries it spans.
        let before = now();
        unit_tick(shared.clone()).await;
        let t0 = now();
        let healthy = [(Health::Healthy, None), (Health::Healthy, None)];
        assert_eq!(health_at(t0), healthy);

        std::fs::write(dir.path().join("fail"), "").unwrap();
        unit_tick(shared.clone()).await;
        // The last good data still counts within 3 intervals...
        assert_eq!(health_at(before + 3), healthy);
        // ...and not after.
        let after = health_at(t0 + 4);
        for (health, message) in &after {
            assert_eq!(*health, Health::Unknown, "{after:?}");
            let message = message.as_deref().unwrap_or_default();
            assert!(
                message.starts_with("collection failed: systemctl")
                    && message.contains("Failed to connect to bus"),
                "{message}"
            );
        }

        // Transitions as the unit ticks at those times would see them (held down for 2).
        let evaluate = || {
            shared.write(|s| {
                let targets = s.unit_targets(&shared.config, t0 + 4);
                s.transitions(targets)
            })
        };
        assert!(evaluate().is_empty(), "held down");
        let events = evaluate();
        let mut seen: Vec<_> = events
            .iter()
            .map(|e| (e.target.as_str(), e.from, e.to))
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("sync.timer", Some(Health::Healthy), Health::Unknown),
                ("web.service", Some(Health::Healthy), Health::Unknown)
            ]
        );
        assert!(events.iter().all(|e| e.message == after[0].1));
    }

    #[tokio::test]
    async fn system_collection_errors_reach_system_event_messages() {
        let shared = Shared::new(config(), Arc::new(MockCollector::new(1)));
        system_tick(shared.clone()).await;
        shared.write(|s| s.record_system_error("cannot read /proc/stat: gone".into()));
        let targets = shared.read(|s| s.system_targets(&shared.config, now() + 31));
        assert!(!targets.is_empty());
        for (_, target, a) in targets {
            assert_eq!(a.health, Health::Unknown, "{target}");
            assert_eq!(
                a.message.as_deref(),
                Some("collection failed: cannot read /proc/stat: gone"),
                "{target}"
            );
        }
    }

    #[tokio::test]
    async fn failing_system_tick_records_the_error() {
        let shared = Shared::new(config(), Arc::new(NoHostInfo(MockCollector::new(1))));
        system_tick(shared.clone()).await;
        let broken = Shared {
            collector: Arc::new(Broken),
            ..shared.clone()
        };
        system_tick(broken).await;
        let targets = shared.read(|s| s.system_targets(&shared.config, now() + 31));
        assert!(targets.iter().all(|(_, _, a)| {
            a.message.as_deref() == Some("collection failed: systemctl failed: boom")
        }));
        // A good sample clears it.
        system_tick(shared.clone()).await;
        let targets = shared.read(|s| s.system_targets(&shared.config, now() + 31));
        assert!(targets.iter().all(|(_, _, a)| {
            a.message
                .as_deref()
                .is_some_and(|m| m.starts_with("no data for"))
        }));
    }

    /// Reviewer's scenario: a check goes critical, Gargoyle restarts while the target is still
    /// down. The restart must log nothing, and the check must stay critical from its first probe.
    #[tokio::test]
    async fn restart_of_a_failing_check_logs_nothing_new() {
        let dir = tempfile::tempdir().unwrap();
        let closed = crate::checks::tests::closed_port().await;
        let start = || async {
            let mut config = Config::from_toml_str(&format!(
                "[collector]\nmode = \"mock\"\njournal_backfill = false\n\
                 [[check]]\nname = \"Game port\"\nkind = \"tcp\"\naddress = \"{closed}\"\n"
            ))
            .unwrap();
            config.storage.path = dir.path().join("g.db");
            let storage = Storage::open(&config.storage).unwrap();
            let shared = Shared::new(config, Arc::new(MockCollector::new(1))).with_storage(storage);
            restore(&shared).await.unwrap();
            shared
        };
        let prober = Prober::new().unwrap();
        let probe = |shared: Shared| {
            let prober = prober.clone();
            async move {
                let cfg = shared.config.checks[0].clone();
                check_tick(&shared, &prober, &cfg).await;
                shared.read(|s| s.checks_view(&shared.config, now())[0].clone())
            }
        };
        let critical = vec![
            ("Game port".to_owned(), None, "warning".to_owned()),
            (
                "Game port".to_owned(),
                Some("warning".to_owned()),
                "critical".to_owned(),
            ),
        ];

        let shared = start().await;
        for _ in 0..3 {
            probe(shared.clone()).await;
        }
        // No hold-down for checks: warning at failure 1, critical at failure 3.
        assert_eq!(stored_events(&dir), critical);

        drop(shared);
        let shared = start().await;
        let before = shared.read(|s| s.checks_view(&shared.config, now())[0].clone());
        assert_eq!(
            (before.health, before.consecutive_failures),
            (Health::Unknown, 0),
            "not yet run in this process"
        );
        for _ in 0..4 {
            let c = probe(shared.clone()).await;
            assert_eq!(c.health, Health::Critical, "{c:?}");
        }
        assert_eq!(
            stored_events(&dir),
            critical,
            "nothing new after the restart"
        );
    }
}
