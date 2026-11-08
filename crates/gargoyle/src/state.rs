//! The in-memory `State` (ARCHITECTURE §1, §3): the latest collected facts, plus the little
//! history health needs (CPU/load windows, restart trackers).
//!
//! The scheduler writes it through the `record_*` methods; HTTP handlers read it through the
//! `*_view` methods, which evaluate health at read time so that staleness is judged against the
//! moment of the request, not the moment of the last sample.

use std::collections::HashMap;

use gargoyle_core::api::{
    CpuSummary, DiskSummary, HostInfo as HostInfoDto, LoadSummary, MemorySummary, NetSummary,
    Overview, ServiceSummary, SystemSummary, TempSummary,
};
use gargoyle_core::config::{Config, ServiceConfig};
use gargoyle_core::health::{
    self, Assessment, Health, RestartTracker, SystemHealth, Window, disk_used_pct,
    evaluate_service, evaluate_system, is_stale, load_per_core, memory_used_pct,
};
use gargoyle_core::model::{HostInfo, SystemSnapshot, UnitSnapshot};
use gargoyle_core::rates::{CpuUsage, NetRate, system_rates};

use crate::storage::samples::{self, Sample};

/// Shown as `host` until host info has been collected once.
const UNKNOWN_HOST: &str = "unknown";

#[derive(Debug)]
pub struct State {
    host_info: Option<HostInfo>,
    system: SystemState,
    units: UnitState,
}

#[derive(Debug)]
struct SystemState {
    last: Option<SystemSnapshot>,
    /// From the last pair of samples; `None` on the first sample or after a counter reset.
    cpu: Option<CpuUsage>,
    net: Vec<NetRate>,
    cpu_window: Window,
    load_window: Window,
    /// Unix seconds of the last successful sample. A failed sample only gets logged: the system
    /// summary has no message field, so failures show as `unknown` once this goes stale.
    last_ok: Option<i64>,
}

#[derive(Debug, Default)]
struct UnitState {
    by_unit: HashMap<String, UnitSnapshot>,
    restarts: HashMap<String, RestartTracker>,
    last_ok: Option<i64>,
    error: Option<String>,
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
            },
            units: UnitState::default(),
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

    /// The `host` field of every response.
    pub fn host(&self) -> &str {
        self.host_info
            .as_ref()
            .map_or(UNKNOWN_HOST, |h| h.hostname.as_str())
    }

    pub fn overview(&self, config: &Config, now: i64) -> Overview {
        let system = self.system_view(config, now);
        let services = self.services_view(config, now);
        let parts: Vec<Health> = std::iter::once(system.health)
            .chain(services.iter().map(|s| s.health))
            .collect();
        Overview {
            host: self.host().to_owned(),
            ts: now,
            health: health::worst(parts.iter().copied()),
            host_info: self.host_info_view(now),
            system,
            counts: health::tally(parts),
            services,
            // Timer health is M3, checks are M4; storage (and so events) is M2.
            timers: Vec::new(),
            checks: Vec::new(),
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

    pub fn service_view(&self, config: &Config, cfg: &ServiceConfig, now: i64) -> ServiceSummary {
        let u = &self.units;
        let snap = u.by_unit.get(&cfg.unit);
        let assessment = if is_stale(u.last_ok, now, config.collector.units_interval) {
            Assessment::unknown(stale_message(u.last_ok, u.error.as_deref(), now))
        } else {
            match snap {
                Some(snap) => {
                    let restarted = u
                        .restarts
                        .get(&cfg.unit)
                        .is_some_and(|t| t.restarted_recently(now));
                    evaluate_service(cfg, snap, restarted)
                }
                None => Assessment::unknown("no data from collector for this unit"),
            }
        };
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
        let h: SystemHealth = evaluate_system(
            &config.thresholds,
            &config.system.disks,
            snap,
            s.cpu_window.mean(),
            s.load_window.mean(),
        );
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
