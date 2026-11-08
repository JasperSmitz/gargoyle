//! Per-second values from cumulative counters (ARCHITECTURE §3, §4).
//!
//! The collector returns counters as the kernel reports them. A counter that goes *down* between
//! two samples means it was reset: the host rebooted, a NIC driver was reloaded, or a 32-bit
//! counter wrapped. A wrap cannot be told apart from a reset reliably, so both are treated the
//! same way: that interval has no rate, and the next pair of samples produces one again. Showing a
//! gap is better than inventing a huge or negative spike.

use crate::model::{CpuTimes, NetCounters, SystemSnapshot};

/// CPU utilisation over one interval, in percent of all cores (0–100).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuUsage {
    /// Everything except idle and iowait.
    pub usage: f64,
    /// Time spent idle while waiting for IO.
    pub iowait: f64,
}

/// Throughput of one interface, in bytes per second.
#[derive(Debug, Clone, PartialEq)]
pub struct NetRate {
    pub iface: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
}

/// Rates between two consecutive system samples.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemRates {
    /// `None` on the first sample or after a counter reset.
    pub cpu: Option<CpuUsage>,
    /// Only interfaces present in both samples with non-decreasing counters.
    pub net: Vec<NetRate>,
}

/// CPU usage between two readings of the same counters, or `None` if any counter went backwards
/// or no time passed.
pub fn cpu_usage(prev: &CpuTimes, cur: &CpuTimes) -> Option<CpuUsage> {
    let fields = |t: &CpuTimes| {
        [
            t.user, t.nice, t.system, t.idle, t.iowait, t.irq, t.softirq, t.steal,
        ]
    };
    if fields(cur).iter().zip(fields(prev)).any(|(c, p)| *c < p) {
        return None;
    }
    let total = cur.total() - prev.total();
    if total == 0 {
        return None;
    }
    let idle = (cur.idle - prev.idle) as f64;
    let iowait = (cur.iowait - prev.iowait) as f64;
    let total = total as f64;
    Some(CpuUsage {
        usage: pct(total - idle - iowait, total),
        iowait: pct(iowait, total),
    })
}

/// Bytes per second for one interface, or `None` after a reset/wrap or when `dt_s` is not
/// positive.
pub fn net_rate(prev: &NetCounters, cur: &NetCounters, dt_s: f64) -> Option<NetRate> {
    if dt_s <= 0.0 {
        return None;
    }
    let rx = cur.rx_bytes.checked_sub(prev.rx_bytes)?;
    let tx = cur.tx_bytes.checked_sub(prev.tx_bytes)?;
    Some(NetRate {
        iface: cur.iface.clone(),
        rx_bps: rx as f64 / dt_s,
        tx_bps: tx as f64 / dt_s,
    })
}

/// Rates from the previous sample (if any) to the current one.
///
/// A reboot between samples (uptime went down) or a clock that did not move forward yields no
/// rates at all, even for counters that happen to look monotonic.
pub fn system_rates(prev: Option<&SystemSnapshot>, cur: &SystemSnapshot) -> SystemRates {
    let Some(prev) = prev else {
        return SystemRates::default();
    };
    if cur.uptime_s < prev.uptime_s || cur.ts <= prev.ts {
        return SystemRates::default();
    }
    let dt = (cur.ts - prev.ts) as f64;
    let net = cur
        .net
        .iter()
        .filter_map(|c| {
            let p = prev.net.iter().find(|p| p.iface == c.iface)?;
            net_rate(p, c, dt)
        })
        .collect();
    SystemRates {
        cpu: cpu_usage(&prev.cpu.total, &cur.cpu.total),
        net,
    }
}

/// `part / whole` in percent, clamped to 0–100 so rounding in the kernel's counters never shows
/// as 100.4%.
fn pct(part: f64, whole: f64) -> f64 {
    (part / whole * 100.0).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CpuCounters, MemoryInfo};

    /// (usage, iowait) or (rx, tx).
    type Pair = (f64, f64);

    fn times(user: u64, system: u64, idle: u64, iowait: u64) -> CpuTimes {
        CpuTimes {
            user,
            system,
            idle,
            iowait,
            ..CpuTimes::default()
        }
    }

    fn iface(name: &str, rx: u64, tx: u64) -> NetCounters {
        NetCounters {
            iface: name.into(),
            rx_bytes: rx,
            tx_bytes: tx,
        }
    }

    fn snap(ts: i64, uptime_s: f64, cpu: CpuTimes, net: Vec<NetCounters>) -> SystemSnapshot {
        SystemSnapshot {
            ts,
            uptime_s,
            cpu: CpuCounters {
                total: cpu,
                per_core: vec![cpu],
            },
            load: [0.0; 3],
            memory: MemoryInfo {
                total: 1,
                available: 1,
                swap_total: 0,
                swap_free: 0,
            },
            disks: vec![],
            net,
            temps: vec![],
        }
    }

    #[test]
    fn cpu_usage_cases() {
        let base = times(100, 100, 700, 100);
        let cases: &[(&str, CpuTimes, Option<Pair>)] = &[
            // +25 user, +25 system, +40 idle, +10 iowait over 100 jiffies.
            ("mixed", times(125, 125, 740, 110), Some((50.0, 10.0))),
            ("all idle", times(100, 100, 800, 100), Some((0.0, 0.0))),
            ("all busy", times(150, 150, 700, 100), Some((100.0, 0.0))),
            ("all iowait", times(100, 100, 700, 200), Some((0.0, 100.0))),
            ("no time passed", base, None),
            ("reset after reboot", times(5, 5, 50, 1), None),
            ("one counter backwards", times(200, 200, 699, 200), None),
        ];
        for (name, cur, expected) in cases {
            let got = cpu_usage(&base, cur).map(|u| (u.usage, u.iowait));
            assert_eq!(got, *expected, "{name}");
        }
    }

    #[test]
    fn cpu_usage_counts_irq_softirq_steal_nice_as_busy() {
        let prev = CpuTimes::default();
        let cur = CpuTimes {
            nice: 10,
            irq: 10,
            softirq: 10,
            steal: 10,
            idle: 60,
            ..CpuTimes::default()
        };
        assert_eq!(cpu_usage(&prev, &cur).map(|u| u.usage), Some(40.0));
    }

    #[test]
    fn net_rate_cases() {
        let prev = iface("eth0", 1_000, 2_000);
        let cases: &[(&str, NetCounters, f64, Option<Pair>)] = &[
            (
                "steady",
                iface("eth0", 11_000, 2_500),
                10.0,
                Some((1000.0, 50.0)),
            ),
            ("idle", iface("eth0", 1_000, 2_000), 10.0, Some((0.0, 0.0))),
            ("rx reset", iface("eth0", 10, 3_000), 10.0, None),
            ("tx wrap", iface("eth0", 2_000, 5), 10.0, None),
            ("zero dt", iface("eth0", 2_000, 3_000), 0.0, None),
            ("negative dt", iface("eth0", 2_000, 3_000), -1.0, None),
        ];
        for (name, cur, dt, expected) in cases {
            let got = net_rate(&prev, cur, *dt).map(|r| (r.rx_bps, r.tx_bps));
            assert_eq!(got, *expected, "{name}");
        }
    }

    #[test]
    fn system_rates_first_sample_has_none() {
        let cur = snap(10, 100.0, times(1, 1, 1, 1), vec![iface("eth0", 1, 1)]);
        assert_eq!(system_rates(None, &cur), SystemRates::default());
    }

    #[test]
    fn system_rates_normal_interval() {
        let prev = snap(
            1000,
            500.0,
            times(0, 0, 0, 0),
            vec![iface("eth0", 0, 0), iface("wlan0", 0, 0)],
        );
        let cur = snap(
            1010,
            510.0,
            times(50, 0, 50, 0),
            // wlan0 disappeared, eth1 is new: neither has a rate.
            vec![iface("eth0", 10_000, 100), iface("eth1", 5, 5)],
        );
        let r = system_rates(Some(&prev), &cur);
        assert_eq!(
            r.cpu,
            Some(CpuUsage {
                usage: 50.0,
                iowait: 0.0
            })
        );
        assert_eq!(
            r.net,
            vec![NetRate {
                iface: "eth0".into(),
                rx_bps: 1000.0,
                tx_bps: 10.0
            }]
        );
    }

    #[test]
    fn system_rates_reboot_drops_everything() {
        let prev = snap(1000, 5000.0, times(0, 0, 0, 0), vec![iface("eth0", 0, 0)]);
        // Uptime went down; counters coincidentally higher must not produce rates.
        let cur = snap(1010, 5.0, times(50, 0, 50, 0), vec![iface("eth0", 10, 10)]);
        assert_eq!(system_rates(Some(&prev), &cur), SystemRates::default());
    }

    #[test]
    fn system_rates_clock_not_advancing_drops_everything() {
        let prev = snap(1000, 100.0, times(0, 0, 0, 0), vec![]);
        let same = snap(1000, 110.0, times(50, 0, 50, 0), vec![]);
        let back = snap(990, 110.0, times(50, 0, 50, 0), vec![]);
        assert_eq!(system_rates(Some(&prev), &same), SystemRates::default());
        assert_eq!(system_rates(Some(&prev), &back), SystemRates::default());
    }

    #[test]
    fn system_rates_net_reset_on_one_iface_keeps_others() {
        let prev = snap(
            0,
            1.0,
            times(0, 0, 0, 0),
            vec![iface("a", 100, 100), iface("b", 100, 100)],
        );
        let cur = snap(
            1,
            2.0,
            times(0, 0, 1, 0),
            vec![iface("a", 0, 0), iface("b", 200, 200)],
        );
        let r = system_rates(Some(&prev), &cur);
        assert_eq!(r.net.len(), 1);
        assert_eq!(r.net[0].iface, "b");
    }
}
