//! Temperatures from `/sys/class/hwmon`, falling back to `/sys/class/thermal`.
//!
//! Sensors are flaky (missing drivers, `EIO` from sleeping devices, chips without temperature
//! inputs), so this never fails: anything unreadable is skipped and no sensors is an empty list.

use std::fs;
use std::path::Path;

use crate::TempReading;

/// Reads all temperature sensors under `root`.
pub fn read_temps(root: &Path) -> Vec<TempReading> {
    let mut temps = read_hwmon(&root.join("sys/class/hwmon"));
    if temps.is_empty() {
        temps = read_thermal_zones(&root.join("sys/class/thermal"));
    }
    disambiguate(&mut temps);
    temps
}

/// `hwmonN/name` is the chip; each `tempK_input` is a sensor, labelled by `tempK_label` if present.
fn read_hwmon(dir: &Path) -> Vec<TempReading> {
    let mut temps = Vec::new();
    for (_, chip_dir) in numbered_entries(dir, "hwmon", "") {
        let chip = read_trimmed(&chip_dir.join("name")).unwrap_or_else(|| "hwmon".into());
        for (k, input) in numbered_entries(&chip_dir, "temp", "_input") {
            let Some(celsius) = read_trimmed(&input).and_then(|v| parse_millidegrees(&v)) else {
                continue;
            };
            let sensor = read_trimmed(&chip_dir.join(format!("temp{k}_label")))
                .unwrap_or_else(|| format!("temp{k}"));
            temps.push(TempReading {
                label: format!("{chip}/{sensor}"),
                celsius,
            });
        }
    }
    temps
}

/// `thermal_zoneN/{type,temp}`; labelled `thermal/<type>`.
fn read_thermal_zones(dir: &Path) -> Vec<TempReading> {
    numbered_entries(dir, "thermal_zone", "")
        .into_iter()
        .filter_map(|(n, zone)| {
            let celsius = read_trimmed(&zone.join("temp")).and_then(|v| parse_millidegrees(&v))?;
            let kind = read_trimmed(&zone.join("type")).unwrap_or_else(|| format!("zone{n}"));
            Some(TempReading {
                label: format!("thermal/{kind}"),
                celsius,
            })
        })
        .collect()
}

/// Entries of `dir` named `<prefix><number><suffix>`, sorted by number (so `temp10` follows
/// `temp9`). A missing or unreadable directory yields nothing.
fn numbered_entries(dir: &Path, prefix: &str, suffix: &str) -> Vec<(u32, std::path::PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name();
            let n = name
                .to_str()?
                .strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse()
                .ok()?;
            Some((n, e.path()))
        })
        .collect();
    found.sort_by_key(|(n, _)| *n);
    found
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Sysfs temperatures are integer millidegrees Celsius.
fn parse_millidegrees(text: &str) -> Option<f64> {
    text.trim().parse::<i64>().ok().map(|m| m as f64 / 1000.0)
}

/// Labels identify a series, so they must be unique. Two identical chips (say, two NVMe drives)
/// produce the same label; later ones get ` #2`, ` #3`, ... in discovery order.
fn disambiguate(temps: &mut [TempReading]) {
    for i in 1..temps.len() {
        let base = temps[i].label.clone();
        let mut n = 1;
        while temps[..i].iter().any(|t| t.label == temps[i].label) {
            n += 1;
            temps[i].label = format!("{base} #{n}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(case)
    }

    fn labels(t: &[TempReading]) -> Vec<&str> {
        t.iter().map(|t| t.label.as_str()).collect()
    }

    #[test]
    fn hwmon_with_labels_numeric_order_and_missing_inputs() {
        let temps = read_temps(&fixture("debian-server"));
        assert_eq!(
            labels(&temps),
            [
                "acpitz/temp1",
                "coretemp/Package id 0",
                "coretemp/Core 0",
                "coretemp/Core 1",
                "coretemp/Core 2",
                "nvme/Composite",
            ]
        );
        assert_eq!(temps[1].celsius, 45.0);
        assert_eq!(temps[5].celsius, 38.85);
    }

    #[test]
    fn captured_arch_has_duplicate_chips() {
        let temps = read_temps(&fixture("arch-desktop"));
        let l = labels(&temps);
        assert!(l.contains(&"k10temp/Tctl"), "{l:?}");
        assert!(l.contains(&"nvme/Composite"), "{l:?}");
        assert!(l.contains(&"nvme/Composite #2"), "{l:?}");
        assert!(!l.iter().any(|l| l.starts_with("thermal/")), "hwmon wins");
    }

    #[test]
    fn falls_back_to_thermal_zones() {
        let temps = read_temps(&fixture("arm-no-hwmon"));
        assert_eq!(
            temps,
            [TempReading {
                label: "thermal/cpu-thermal".into(),
                celsius: 51.54,
            }]
        );
    }

    #[test]
    fn no_sensors_is_empty() {
        assert!(read_temps(&fixture("many-cores")).is_empty());
        assert!(read_temps(Path::new("/nonexistent/gargoyle")).is_empty());
    }

    #[test]
    fn millidegrees() {
        assert_eq!(parse_millidegrees("-5500\n"), Some(-5.5));
        assert_eq!(parse_millidegrees("hot"), None);
    }
}
