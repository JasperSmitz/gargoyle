//! [`LinuxCollector`]: reads `/proc`, `/sys` and `systemctl show` on the local host (ARCHITECTURE §4).
//!
//! Every path is resolved under `root`, so tests point it at a fixture tree. Parsing lives in pure
//! `fn(&str) -> Result<T, CollectError>` functions in the submodules (ADR 0006).

mod mounts;
mod net;
mod procfs;
mod sensors;
mod systemctl;

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gargoyle_core::config::SystemConfig;

use crate::{
    CollectError, Collector, DiskUsage, HostInfo, NetCounters, SystemSnapshot, UnitRequest,
    UnitSnapshot,
};

pub use systemctl::PROPERTIES as SYSTEMCTL_PROPERTIES;

/// Collector for the host it runs on.
#[derive(Debug, Clone)]
pub struct LinuxCollector {
    /// Filesystem root under which `proc/`, `sys/` and `etc/` are read; `/` in production.
    pub root: PathBuf,
    /// The `systemctl` binary. Only ever run as `systemctl show`.
    pub systemctl: PathBuf,
    /// Which disks and interfaces to report, and whether to read temperatures.
    pub system: SystemConfig,
}

impl LinuxCollector {
    /// A collector for the real host.
    pub fn new(system: SystemConfig) -> Self {
        Self {
            root: PathBuf::from("/"),
            systemctl: PathBuf::from("/usr/bin/systemctl"),
            system,
        }
    }

    /// Reads `rel` (e.g. `proc/stat`) under `root`.
    fn read(&self, rel: &str) -> Result<String, CollectError> {
        let path = self.root.join(rel);
        std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))
    }

    /// Like [`Self::read`], but a missing file is `None`.
    fn read_optional(&self, rel: &str) -> Result<Option<String>, CollectError> {
        let path = self.root.join(rel);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(&path, &e)),
        }
    }

    fn os_name(&self) -> Result<String, CollectError> {
        for rel in ["etc/os-release", "usr/lib/os-release"] {
            if let Some(name) = self
                .read_optional(rel)?
                .and_then(|t| procfs::parse_os_release(&t))
            {
                return Ok(name);
            }
        }
        Ok("Linux".into())
    }

    fn disks(&self) -> Result<Vec<DiskUsage>, CollectError> {
        let mounts = mounts::parse_mountinfo(&self.read("proc/self/mountinfo")?)?;
        let selected = mounts::select_mounts(&mounts, &self.system.disks);
        for missing in self
            .system
            .disks
            .iter()
            .filter(|d| !selected.iter().any(|m| &m.mount_point == *d))
        {
            tracing::warn!(mount = %missing, "configured disk is not mounted");
        }
        Ok(selected
            .into_iter()
            .filter_map(|m| {
                let path = self.root.join(m.mount_point.trim_start_matches('/'));
                match rustix::fs::statvfs(&path) {
                    Ok(st) => Some(DiskUsage {
                        total: st.f_blocks.saturating_mul(st.f_frsize),
                        used: st
                            .f_blocks
                            .saturating_sub(st.f_bfree)
                            .saturating_mul(st.f_frsize),
                        avail: st.f_bavail.saturating_mul(st.f_frsize),
                        mount: m.mount_point,
                        fs_type: m.fs_type,
                    }),
                    // One unreadable filesystem must not hide every other metric.
                    Err(e) => {
                        tracing::warn!(mount = %m.mount_point, error = %e, "statvfs failed");
                        None
                    }
                }
            })
            .collect())
    }

    /// Missing `/proc/net/dev` (no network namespace access) is an empty list, not an error.
    fn net(&self) -> Result<Vec<NetCounters>, CollectError> {
        let Some(text) = self.read_optional("proc/net/dev")? else {
            return Ok(Vec::new());
        };
        Ok(net::select_interfaces(
            net::parse_net_dev(&text)?,
            &self.system.interfaces,
        ))
    }
}

impl Collector for LinuxCollector {
    fn host_info(&self) -> Result<HostInfo, CollectError> {
        let stat = procfs::parse_stat(&self.read("proc/stat")?)?;
        let cpu = procfs::parse_cpuinfo(&self.read("proc/cpuinfo")?);
        let cores = if cpu.processors > 0 {
            cpu.processors
        } else {
            u32::try_from(stat.cpu.per_core.len()).unwrap_or(u32::MAX)
        };
        Ok(HostInfo {
            hostname: self.read("proc/sys/kernel/hostname")?.trim().to_owned(),
            os: self.os_name()?,
            kernel: self.read("proc/sys/kernel/osrelease")?.trim().to_owned(),
            cpu_model: cpu.model,
            cores,
            boot_time: stat
                .boot_time
                .ok_or_else(|| parse_error("/proc/stat", "btime missing"))?,
        })
    }

    fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        Ok(SystemSnapshot {
            ts,
            uptime_s: procfs::parse_uptime(&self.read("proc/uptime")?)?,
            cpu: procfs::parse_stat(&self.read("proc/stat")?)?.cpu,
            load: procfs::parse_loadavg(&self.read("proc/loadavg")?)?,
            memory: procfs::parse_meminfo(&self.read("proc/meminfo")?)?,
            disks: self.disks()?,
            net: self.net()?,
            temps: if self.system.temperatures {
                sensors::read_temps(&self.root)
            } else {
                Vec::new()
            },
        })
    }

    fn units(&self, units: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError> {
        systemctl::collect(units, |names| systemctl::run_show(&self.systemctl, names))
    }
}

fn io_error(path: &Path, e: &std::io::Error) -> CollectError {
    CollectError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

fn parse_error(source: &str, message: impl Into<String>) -> CollectError {
    CollectError::Parse {
        source_name: source.to_owned(),
        message: message.into(),
    }
}
