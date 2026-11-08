//! Periodic collection (ARCHITECTURE §3): the system tick and the unit tick.
//!
//! Collector calls block (they read `/proc` or spawn `systemctl`), so each runs in
//! `spawn_blocking`. A failed collection is logged and recorded; it never stops the loop.
//!
//! Calls can also hang: `systemctl` on a stuck D-Bus, `statvfs` on a dead NFS mount. Each call is
//! awaited for at most [`call_limit`], and while a timed-out call of one kind is still running on
//! the blocking pool, further ticks of that kind are skipped, so hung threads cannot pile up.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gargoyle_collector::{CollectError, Collector};
use gargoyle_core::config::Config;
use gargoyle_core::model::UnitRequest;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::state::State;

/// Everything the scheduler and the HTTP handlers share.
#[derive(Clone)]
pub struct Shared {
    pub config: Arc<Config>,
    pub collector: Arc<dyn Collector>,
    pub state: Arc<RwLock<State>>,
    system_busy: Busy,
    units_busy: Busy,
}

impl Shared {
    pub fn new(config: Config, collector: Arc<dyn Collector>) -> Self {
        let state = State::new(&config);
        Self {
            config: Arc::new(config),
            collector,
            state: Arc::new(RwLock::new(state)),
            system_busy: Busy::default(),
            units_busy: Busy::default(),
        }
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

/// Starts the system and unit loops. Each ticks immediately, then every configured interval.
pub fn spawn(shared: &Shared) -> Vec<JoinHandle<()>> {
    let c = &shared.config.collector;
    vec![
        spawn_loop(shared.clone(), c.system_interval, system_tick),
        spawn_loop(shared.clone(), c.units_interval, unit_tick),
    ]
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
/// it succeeds; after that only uptime changes, and that comes from the system sample.
pub async fn system_tick(shared: Shared) {
    let limit = call_limit(shared.config.collector.system_interval);
    if !shared.read(State::has_host_info) {
        let collector = shared.collector.clone();
        match call(&shared.system_busy, limit, move || collector.host_info()).await {
            Ok(info) => shared.write(|s| s.record_host_info(info)),
            Err(e) => tracing::warn!(error = %e, "cannot read host info"),
        }
    }

    let collector = shared.collector.clone();
    match call(&shared.system_busy, limit, move || {
        collector.system_snapshot()
    })
    .await
    {
        Ok(snap) => shared.write(|s| s.record_system(snap)),
        // Not stored: the system summary has no message, and it turns `unknown` once its last
        // sample is stale.
        Err(e) => tracing::warn!(error = %e, "system collection failed"),
    }
}

/// One batched poll of every configured unit.
pub async fn unit_tick(shared: Shared) {
    let requests: Vec<UnitRequest> = shared
        .config
        .services
        .iter()
        .map(|s| UnitRequest::new(&s.unit))
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
        Ok(units) => shared.write(|s| s.record_units(now(), units)),
        Err(e) => {
            tracing::warn!(error = %e, "unit collection failed");
            shared.write(|s| s.record_units_error(e.to_string()));
        }
    }
}

/// The longest one collector call is awaited: one interval, but never more than 20s.
pub fn call_limit(interval: Duration) -> Duration {
    interval.min(Duration::from_secs(20))
}

/// Set while a call of one kind is running on the blocking pool, including after its caller gave
/// up waiting.
#[derive(Clone, Default)]
struct Busy(Arc<AtomicBool>);

/// Clears the [`Busy`] flag when the blocking call ends, however it ends.
struct ClearOnDrop(Arc<AtomicBool>);

impl Drop for ClearOnDrop {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
enum CallError {
    /// The previous call of this kind has not returned yet; this tick was skipped.
    Busy,
    TimedOut(Duration),
    Panicked,
    Collect(CollectError),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Busy => f.write_str("previous collection is still running"),
            CallError::TimedOut(d) => write!(f, "timed out after {}s", d.as_secs()),
            CallError::Panicked => f.write_str("collector crashed"),
            CallError::Collect(e) => e.fmt(f),
        }
    }
}

/// Runs a collector call on the blocking pool, waiting at most `limit`, unless the previous call
/// guarded by `busy` is still running. A panic in the call becomes an error, so one bad sample
/// does not take the loop down.
async fn call<T, F>(busy: &Busy, limit: Duration, f: F) -> Result<T, CallError>
where
    F: FnOnce() -> Result<T, CollectError> + Send + 'static,
    T: Send + 'static,
{
    if busy.0.swap(true, Ordering::AcqRel) {
        return Err(CallError::Busy);
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
            Some("collection failed: previous collection is still running")
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
        let handles = spawn(&shared);
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
}
