//! `/proc/net/dev` parsing and interface selection.

use super::parse_error;
use crate::{CollectError, NetCounters};

/// Interfaces skipped by auto-detection: loopback and container/bridge plumbing (ARCHITECTURE §4).
const SKIPPED_PREFIXES: &[&str] = &["veth", "docker", "br-"];

/// Parses `/proc/net/dev` into cumulative byte counters per interface.
pub fn parse_net_dev(text: &str) -> Result<Vec<NetCounters>, CollectError> {
    // The two header lines have no ':'; every interface line does.
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(iface, counters)| {
            let fields: Vec<&str> = counters.split_ascii_whitespace().collect();
            let field = |i: usize| {
                fields
                    .get(i)
                    .and_then(|f| f.parse::<u64>().ok())
                    .ok_or_else(|| {
                        parse_error(
                            "/proc/net/dev",
                            format!("bad counters for {}", iface.trim()),
                        )
                    })
            };
            Ok(NetCounters {
                iface: iface.trim().to_owned(),
                rx_bytes: field(0)?,
                tx_bytes: field(8)?,
            })
        })
        .collect()
}

/// Keeps the configured interfaces, or when none are configured, all but loopback and virtual ones.
pub fn select_interfaces(all: Vec<NetCounters>, configured: &[String]) -> Vec<NetCounters> {
    all.into_iter()
        .filter(|n| {
            if configured.is_empty() {
                n.iface != "lo" && !SKIPPED_PREFIXES.iter().any(|p| n.iface.starts_with(p))
            } else {
                configured.contains(&n.iface)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[NetCounters]) -> Vec<&str> {
        n.iter().map(|n| n.iface.as_str()).collect()
    }

    #[test]
    fn parses_and_auto_selects() {
        let all = parse_net_dev(include_str!(
            "../../tests/fixtures/debian-server/proc/net/dev"
        ))
        .expect("parses");
        assert_eq!(all.len(), 6);
        let picked = select_interfaces(all, &[]);
        assert_eq!(names(&picked), ["eno1", "wg0"]);
        assert_eq!(
            picked[0],
            NetCounters {
                iface: "eno1".into(),
                rx_bytes: 987_654_321_098,
                tx_bytes: 123_456_789_012,
            }
        );
    }

    #[test]
    fn configured_interfaces() {
        let all = parse_net_dev(include_str!(
            "../../tests/fixtures/debian-server/proc/net/dev"
        ))
        .expect("parses");
        let picked = select_interfaces(all, &["docker0".into(), "missing0".into()]);
        assert_eq!(names(&picked), ["docker0"]);
    }

    #[test]
    fn large_counters_glued_to_the_colon() {
        let all = parse_net_dev(include_str!("../../tests/fixtures/many-cores/proc/net/dev"))
            .expect("parses");
        assert_eq!(all[1].iface, "ens1f0");
        assert_eq!(all[1].rx_bytes, 123_456_789_012_345);
        assert_eq!(all[1].tx_bytes, 98_765_432_109_876);
    }

    #[test]
    fn captured_arch() {
        let all = parse_net_dev(include_str!(
            "../../tests/fixtures/arch-desktop/proc/net/dev"
        ))
        .expect("parses");
        assert!(names(&select_interfaces(all, &[])).contains(&"wlp15s0"));
    }

    #[test]
    fn truncated_line_is_an_error() {
        assert!(parse_net_dev("eth0: 1 2 3\n").is_err());
    }
}
