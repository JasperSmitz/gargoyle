//! Pure parsers for the `/proc` text files and os-release (ADR 0006).

use super::parse_error;
use crate::{CollectError, CpuCounters, CpuTimes, MemoryInfo};

/// What `/proc/stat` tells us.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcStat {
    pub cpu: CpuCounters,
    /// `btime`: boot time, unix seconds.
    pub boot_time: Option<i64>,
}

/// Parses `/proc/stat`: the aggregate `cpu` line, the `cpuN` lines in order, and `btime`.
pub fn parse_stat(text: &str) -> Result<ProcStat, CollectError> {
    let mut total = None;
    let mut per_core = Vec::new();
    let mut boot_time = None;
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let Some(key) = fields.next() else { continue };
        if key == "cpu" {
            total = Some(parse_cpu_times(fields)?);
        } else if key.starts_with("cpu") && key[3..].bytes().all(|b| b.is_ascii_digit()) {
            per_core.push(parse_cpu_times(fields)?);
        } else if key == "btime" {
            let value = fields.next().unwrap_or_default();
            boot_time = Some(
                value
                    .parse()
                    .map_err(|_| parse_error("/proc/stat", format!("invalid btime {value:?}")))?,
            );
        }
    }
    let total = total.ok_or_else(|| parse_error("/proc/stat", "no aggregate cpu line"))?;
    Ok(ProcStat {
        cpu: CpuCounters { total, per_core },
        boot_time,
    })
}

/// The columns are user, nice, system, idle, iowait, irq, softirq, steal (guest columns follow and
/// are ignored). Very old kernels print only the first four, so missing trailing columns are 0.
fn parse_cpu_times<'a>(fields: impl Iterator<Item = &'a str>) -> Result<CpuTimes, CollectError> {
    let values = fields
        .take(8)
        .map(|f| {
            f.parse::<u64>()
                .map_err(|_| parse_error("/proc/stat", format!("invalid cpu counter {f:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 4 {
        return Err(parse_error(
            "/proc/stat",
            "cpu line has fewer than 4 columns",
        ));
    }
    let at = |i: usize| values.get(i).copied().unwrap_or(0);
    Ok(CpuTimes {
        user: at(0),
        nice: at(1),
        system: at(2),
        idle: at(3),
        iowait: at(4),
        irq: at(5),
        softirq: at(6),
        steal: at(7),
    })
}

/// Parses `/proc/meminfo`. `MemTotal` and `MemAvailable` are required; swap is 0 when absent.
pub fn parse_meminfo(text: &str) -> Result<MemoryInfo, CollectError> {
    let mut total = None;
    let mut available = None;
    let mut swap_total = 0;
    let mut swap_free = 0;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        match key {
            "MemTotal" => total = Some(meminfo_bytes(key, rest)?),
            "MemAvailable" => available = Some(meminfo_bytes(key, rest)?),
            "SwapTotal" => swap_total = meminfo_bytes(key, rest)?,
            "SwapFree" => swap_free = meminfo_bytes(key, rest)?,
            _ => {}
        }
    }
    Ok(MemoryInfo {
        total: total.ok_or_else(|| parse_error("/proc/meminfo", "MemTotal missing"))?,
        available: available.ok_or_else(|| parse_error("/proc/meminfo", "MemAvailable missing"))?,
        swap_total,
        swap_free,
    })
}

fn meminfo_bytes(key: &str, rest: &str) -> Result<u64, CollectError> {
    let mut parts = rest.split_ascii_whitespace();
    let value = parts
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| parse_error("/proc/meminfo", format!("invalid value for {key}")))?;
    // The kernel's "kB" is KiB.
    let factor = match parts.next() {
        Some("kB") => 1024,
        None => 1,
        Some(unit) => {
            return Err(parse_error(
                "/proc/meminfo",
                format!("unknown unit {unit:?} for {key}"),
            ));
        }
    };
    Ok(value.saturating_mul(factor))
}

/// Parses `/proc/loadavg` into the 1-, 5- and 15-minute load averages.
pub fn parse_loadavg(text: &str) -> Result<[f64; 3], CollectError> {
    let mut fields = text.split_ascii_whitespace();
    let mut load = [0.0; 3];
    for slot in &mut load {
        *slot = fields
            .next()
            .and_then(|f| f.parse().ok())
            .ok_or_else(|| parse_error("/proc/loadavg", "expected three load averages"))?;
    }
    Ok(load)
}

/// Parses `/proc/uptime` into seconds since boot.
pub fn parse_uptime(text: &str) -> Result<f64, CollectError> {
    text.split_ascii_whitespace()
        .next()
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| parse_error("/proc/uptime", "expected uptime seconds"))
}

/// CPU facts from `/proc/cpuinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuInfo {
    pub model: String,
    /// Number of `processor` entries; 0 if the file lists none.
    pub processors: u32,
}

/// Keys that name the CPU, best first. x86 has `model name`; arm64 has no per-CPU model at all,
/// so we fall back to the board (`Model`, e.g. a Raspberry Pi) or the older ARM/MIPS keys.
const MODEL_KEYS: &[&str] = &["model name", "Processor", "cpu model", "Model", "Hardware"];

/// Parses `/proc/cpuinfo`. Never fails: an unknown layout yields model `unknown`.
pub fn parse_cpuinfo(text: &str) -> CpuInfo {
    let mut processors = 0;
    let mut best: Option<(usize, &str)> = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "processor" {
            processors += 1;
        } else if let Some(rank) = MODEL_KEYS.iter().position(|k| *k == key)
            && !value.is_empty()
            && best.is_none_or(|(r, _)| rank < r)
        {
            best = Some((rank, value));
        }
    }
    CpuInfo {
        model: best.map_or("unknown", |(_, v)| v).to_owned(),
        processors,
    }
}

/// Returns `PRETTY_NAME` from an os-release file, falling back to `NAME`.
pub fn parse_os_release(text: &str) -> Option<String> {
    let mut name = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        match key {
            "PRETTY_NAME" => return Some(unquote(value)).filter(|v| !v.is_empty()),
            "NAME" => name = Some(unquote(value)),
            _ => {}
        }
    }
    name.filter(|v| !v.is_empty())
}

/// os-release values follow shell quoting: optional single or double quotes, and backslash
/// escapes inside double quotes.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return inner.to_owned();
    }
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($case:literal, $file:literal) => {
            include_str!(concat!("../../tests/fixtures/", $case, "/", $file))
        };
    }

    #[test]
    fn stat_debian() {
        let stat = parse_stat(fixture!("debian-server", "proc/stat")).expect("parses");
        assert_eq!(stat.boot_time, Some(1_793_300_000));
        assert_eq!(stat.cpu.per_core.len(), 4);
        assert_eq!(
            stat.cpu.total,
            CpuTimes {
                user: 1_529_370,
                nice: 2310,
                system: 498_124,
                idle: 61_935_612,
                iowait: 41_228,
                irq: 0,
                softirq: 9120,
                steal: 0,
            }
        );
        assert_eq!(stat.cpu.per_core[3].user, 380_895);
    }

    #[test]
    fn stat_captured_and_many_cores() {
        let arch = parse_stat(fixture!("arch-desktop", "proc/stat")).expect("parses");
        assert_eq!(arch.cpu.per_core.len(), 16);
        assert!(arch.boot_time.is_some());

        let many = parse_stat(fixture!("many-cores", "proc/stat")).expect("parses");
        assert_eq!(many.cpu.per_core.len(), 64);
        assert_eq!(many.cpu.total.steal, 1280);
        assert_eq!(many.cpu.per_core[63].steal, 20);
    }

    #[test]
    fn stat_old_kernel_with_four_columns() {
        let stat = parse_stat("cpu 1 2 3 4\ncpu0 1 2 3 4\n").expect("parses");
        assert_eq!(stat.cpu.total.idle, 4);
        assert_eq!(stat.cpu.total.steal, 0);
        assert_eq!(stat.boot_time, None);
    }

    #[test]
    fn stat_errors() {
        assert!(parse_stat("cpu0 1 2 3 4\n").is_err(), "no aggregate line");
        assert!(parse_stat("cpu 1 2 3\n").is_err(), "too few columns");
        assert!(parse_stat("cpu 1 2 x 4\n").is_err(), "not a number");
        assert!(parse_stat("cpu 1 2 3 4\nbtime soon\n").is_err());
    }

    #[test]
    fn meminfo() {
        let m = parse_meminfo(fixture!("debian-server", "proc/meminfo")).expect("parses");
        assert_eq!(
            m,
            MemoryInfo {
                total: 16_303_912 * 1024,
                available: 11_234_567 * 1024,
                swap_total: 1_003_516 * 1024,
                swap_free: 991_228 * 1024,
            }
        );
        let arch = parse_meminfo(fixture!("arch-desktop", "proc/meminfo")).expect("parses");
        assert!(arch.available <= arch.total);
    }

    #[test]
    fn meminfo_without_swap_lines_and_errors() {
        let m = parse_meminfo("MemTotal: 100 kB\nMemAvailable: 50 kB\n").expect("parses");
        assert_eq!((m.swap_total, m.swap_free), (0, 0));
        assert!(parse_meminfo("MemTotal: 100 kB\n").is_err());
        assert!(parse_meminfo("MemTotal: lots kB\nMemAvailable: 50 kB\n").is_err());
    }

    #[test]
    fn loadavg_and_uptime() {
        assert_eq!(
            parse_loadavg(fixture!("debian-server", "proc/loadavg")).expect("parses"),
            [0.42, 0.37, 0.31]
        );
        assert!(parse_loadavg("0.1 0.2\n").is_err());
        assert_eq!(
            parse_uptime(fixture!("debian-server", "proc/uptime")).expect("parses"),
            698_123.45
        );
        assert!(parse_uptime("").is_err());
    }

    #[test]
    fn cpuinfo_x86() {
        assert_eq!(
            parse_cpuinfo(fixture!("debian-server", "proc/cpuinfo")),
            CpuInfo {
                model: "Intel(R) Core(TM) i5-8500T CPU @ 2.10GHz".into(),
                processors: 4,
            }
        );
        let arch = parse_cpuinfo(fixture!("arch-desktop", "proc/cpuinfo"));
        assert_eq!(arch.processors, 16);
        assert!(arch.model.starts_with("AMD Ryzen"), "{}", arch.model);
        assert_eq!(
            parse_cpuinfo(fixture!("many-cores", "proc/cpuinfo")).processors,
            64
        );
    }

    #[test]
    fn cpuinfo_arm_without_model_name() {
        assert_eq!(
            parse_cpuinfo(fixture!("arm-no-hwmon", "proc/cpuinfo")),
            CpuInfo {
                model: "Raspberry Pi 4 Model B Rev 1.4".into(),
                processors: 4,
            }
        );
        assert_eq!(parse_cpuinfo("processor : 0\n").model, "unknown");
    }

    #[test]
    fn os_release() {
        assert_eq!(
            parse_os_release(fixture!("debian-server", "etc/os-release")).as_deref(),
            Some("Debian GNU/Linux 13 (trixie)")
        );
        assert_eq!(
            parse_os_release(fixture!("arch-desktop", "etc/os-release")).as_deref(),
            Some("Arch Linux")
        );
        assert_eq!(
            parse_os_release("NAME='Foo'\n").as_deref(),
            Some("Foo"),
            "falls back to NAME"
        );
        assert_eq!(
            parse_os_release(r#"PRETTY_NAME="Say \"hi\"""#).as_deref(),
            Some(r#"Say "hi""#)
        );
        assert_eq!(parse_os_release("ID=x\n"), None);
    }
}
