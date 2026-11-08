//! Unit state via `systemctl show` (ADR 0003).

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use super::parse_error;
use crate::{CollectError, UnitRequest, UnitSnapshot};

/// The only properties ever requested (ARCHITECTURE §9). Nothing here can carry secrets: no
/// `ExecStart*`, `Environment*` or similar. `Names` lists aliases so that a request for an alias
/// (Debian's `sshd.service` is `ssh.service`) still finds its block.
pub const PROPERTIES: &[&str] = &[
    "Id",
    "Names",
    "LoadState",
    "ActiveState",
    "SubState",
    "Description",
    "ActiveEnterTimestamp",
    "NRestarts",
    "Result",
    "ExecMainStatus",
    "ExecMainStartTimestamp",
    "ExecMainExitTimestamp",
    "InvocationID",
    "MemoryCurrent",
    "LastTriggerUSec",
    "NextElapseUSecRealtime",
    "Unit",
];

const SOURCE: &str = "systemctl show";

/// One unit's `Key=Value` block.
type Properties = HashMap<String, String>;

/// Collects snapshots for `requests` with at most two `show` calls through `show`.
///
/// The first call asks for every requested unit plus each timer's service: the configured one, or
/// the `<stem>.service` guess. Only a timer whose `Unit=` differs from that guess needs the second
/// call. Results are matched to requests by `Id`/`Names`, never by position.
pub fn collect(
    requests: &[UnitRequest],
    mut show: impl FnMut(&[String]) -> Result<String, CollectError>,
) -> Result<Vec<UnitSnapshot>, CollectError> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let mut first = Vec::new();
    for r in requests {
        push_unique(&mut first, r.unit.clone());
        if let Some(stem) = timer_stem(&r.unit) {
            push_unique(&mut first, service_guess(r, stem));
        }
    }
    let mut blocks = parse_show(&show(&first)?)?;

    let mut second = Vec::new();
    // Only timers emit their triggered unit; `.path` units also have `Unit=` but it is not used.
    for r in requests
        .iter()
        .filter(|r| r.service.is_none() && timer_stem(&r.unit).is_some())
    {
        if let Some(unit) = find(&blocks, &r.unit).and_then(trigger_unit)
            && find(&blocks, unit).is_none()
        {
            push_unique(&mut second, unit.to_owned());
        }
    }
    if !second.is_empty() {
        blocks.extend(parse_show(&show(&second)?)?);
    }

    let mut out: Vec<UnitSnapshot> = Vec::new();
    let mut emit = |name: &str| -> Result<(), CollectError> {
        if out.iter().any(|s| s.unit == name) {
            return Ok(());
        }
        let props = find(&blocks, name)
            .ok_or_else(|| parse_error(SOURCE, format!("no properties returned for {name}")))?;
        out.push(snapshot(name, props)?);
        Ok(())
    };
    for r in requests {
        emit(&r.unit)?;
        if let Some(stem) = timer_stem(&r.unit) {
            let service = match &r.service {
                Some(s) => s.clone(),
                None => find(&blocks, &r.unit)
                    .and_then(trigger_unit)
                    .map_or_else(|| service_guess(r, stem), str::to_owned),
            };
            emit(&service)?;
        }
    }
    Ok(out)
}

/// Runs the one allowed verb with the fixed property list. Unit names come after `--` and are
/// passed as separate arguments, never through a shell.
pub fn run_show(systemctl: &Path, units: &[String]) -> Result<String, CollectError> {
    let command_error = |message: String| CollectError::Command {
        program: "systemctl".into(),
        message,
    };
    let output = Command::new(systemctl)
        .args(["show", "--no-pager", "--timestamp=unix", "-p"])
        .arg(PROPERTIES.join(","))
        .arg("--")
        .args(units)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| command_error(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: String = stderr
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        return Err(command_error(format!(
            "{}: {}",
            output.status,
            detail.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Splits `systemctl show` output into its blank-line separated `Key=Value` blocks.
pub fn parse_show(text: &str) -> Result<Vec<Properties>, CollectError> {
    let mut blocks = Vec::new();
    let mut current = Properties::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| parse_error(SOURCE, format!("line without '=': {line:?}")))?;
        current.insert(key.to_owned(), value.to_owned());
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    if let Some(block) = blocks.iter().find(|b| !b.contains_key("Id")) {
        let keys: Vec<_> = block.keys().collect();
        return Err(parse_error(SOURCE, format!("block without Id: {keys:?}")));
    }
    Ok(blocks)
}

/// Builds the snapshot for the requested `unit` name from its property block.
pub fn snapshot(unit: &str, p: &Properties) -> Result<UnitSnapshot, CollectError> {
    let required = |key: &str| {
        p.get(key)
            .cloned()
            .ok_or_else(|| parse_error(SOURCE, format!("{unit}: {key} missing")))
    };
    let text = |key: &str| p.get(key).filter(|v| !is_unset(v)).cloned();
    let get = |key: &str| p.get(key).map(String::as_str);
    let timestamp = |key: &str| {
        let raw = get(key).map(|v| v.strip_prefix('@').unwrap_or(v));
        // An unset timestamp is sometimes printed as 0 rather than empty.
        optional::<i64>(unit, key, raw).filter(|t| *t != 0)
    };
    Ok(UnitSnapshot {
        unit: unit.to_owned(),
        load_state: required("LoadState")?,
        active_state: required("ActiveState")?,
        sub_state: required("SubState")?,
        description: p.get("Description").cloned().unwrap_or_default(),
        active_enter_ts: timestamp("ActiveEnterTimestamp"),
        n_restarts: optional(unit, "NRestarts", get("NRestarts")),
        result: text("Result"),
        exec_main_status: optional(unit, "ExecMainStatus", get("ExecMainStatus")),
        exec_main_start_ts: timestamp("ExecMainStartTimestamp"),
        exec_main_exit_ts: timestamp("ExecMainExitTimestamp"),
        invocation_id: text("InvocationID"),
        // Older systemd prints "unset" as u64::MAX.
        memory_current: optional::<u64>(unit, "MemoryCurrent", get("MemoryCurrent"))
            .filter(|m| *m != u64::MAX),
        last_trigger_ts: timestamp("LastTriggerUSec"),
        next_elapse_ts: timestamp("NextElapseUSecRealtime"),
        trigger_unit: text("Unit"),
    })
}

/// Optional properties never fail the batch: one odd value (say, a timestamp from a systemd that
/// ignores `--timestamp=unix`) would otherwise hide every unit. It is logged and treated as unset.
fn optional<T: std::str::FromStr>(unit: &str, key: &str, value: Option<&str>) -> Option<T> {
    let value = value.filter(|v| !is_unset(v))?;
    match value.parse() {
        Ok(parsed) => Some(parsed),
        Err(_) => {
            tracing::warn!(
                unit,
                property = key,
                value,
                "unparseable systemd property, ignoring it"
            );
            None
        }
    }
}

/// Spellings systemd uses for "no value".
fn is_unset(value: &str) -> bool {
    matches!(value, "" | "n/a" | "[not set]" | "infinity")
}

fn find<'a>(blocks: &'a [Properties], name: &str) -> Option<&'a Properties> {
    blocks.iter().find(|b| {
        b.get("Id").is_some_and(|id| id == name)
            || b.get("Names")
                .is_some_and(|n| n.split(' ').any(|n| n == name))
    })
}

fn trigger_unit(block: &Properties) -> Option<&str> {
    block
        .get("Unit")
        .map(String::as_str)
        .filter(|u| !is_unset(u))
}

fn timer_stem(unit: &str) -> Option<&str> {
    unit.strip_suffix(".timer")
}

fn service_guess(request: &UnitRequest, stem: &str) -> String {
    request
        .service
        .clone()
        .unwrap_or_else(|| format!("{stem}.service"))
}

fn push_unique(list: &mut Vec<String>, name: String) {
    if !list.contains(&name) {
        list.push(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($unit:literal) => {
            include_str!(concat!("../../tests/fixtures/systemctl/", $unit, ".txt"))
        };
    }

    fn one(text: &str, unit: &str) -> UnitSnapshot {
        let blocks = parse_show(text).expect("parses");
        assert_eq!(blocks.len(), 1);
        snapshot(unit, &blocks[0]).expect("maps")
    }

    #[test]
    fn whitelist_has_no_secret_bearing_properties() {
        for p in PROPERTIES {
            assert!(
                !p.starts_with("Exec") || p.starts_with("ExecMain"),
                "{p} may expose command lines"
            );
            assert!(!p.contains("Environment"), "{p}");
        }
    }

    #[test]
    fn running_service() {
        let s = one(fixture!("NetworkManager.service"), "NetworkManager.service");
        assert_eq!(
            s,
            UnitSnapshot {
                unit: "NetworkManager.service".into(),
                load_state: "loaded".into(),
                active_state: "active".into(),
                sub_state: "running".into(),
                description: "Network Manager".into(),
                active_enter_ts: Some(1_793_997_238),
                n_restarts: Some(0),
                result: Some("success".into()),
                exec_main_status: Some(0),
                exec_main_start_ts: Some(1_793_997_237),
                exec_main_exit_ts: None,
                invocation_id: Some("7b81da4cd23d4aba991b2ae2d0fddc73".into()),
                memory_current: s.memory_current,
                last_trigger_ts: None,
                next_elapse_ts: None,
                trigger_unit: None,
            }
        );
        assert!(s.memory_current.is_some_and(|m| m > 0));
    }

    #[test]
    fn failed_service() {
        let s = one(fixture!("backup.service"), "backup.service");
        assert_eq!(
            (s.active_state.as_str(), s.sub_state.as_str()),
            ("failed", "failed")
        );
        assert_eq!(s.result.as_deref(), Some("exit-code"));
        assert_eq!(s.exec_main_status, Some(2));
        assert_eq!(s.n_restarts, Some(3));
        assert_eq!(s.exec_main_exit_ts, Some(1_794_053_612));
        assert_eq!(s.memory_current, None);
    }

    #[test]
    fn inactive_and_oneshot_services() {
        let inactive = one(fixture!("shadow.service"), "shadow.service");
        assert_eq!(inactive.active_state, "inactive");
        assert_eq!(inactive.sub_state, "dead");
        assert_eq!(inactive.memory_current, None);

        let oneshot = one(
            fixture!("systemd-tmpfiles-setup.service"),
            "systemd-tmpfiles-setup.service",
        );
        assert_eq!(
            (oneshot.active_state.as_str(), oneshot.sub_state.as_str()),
            ("active", "exited")
        );
        assert_eq!(oneshot.exec_main_exit_ts, Some(1_793_997_235));
    }

    #[test]
    fn not_found_is_a_snapshot() {
        let s = one(fixture!("nonexistent.service"), "nonexistent.service");
        assert_eq!(s.load_state, "not-found");
        assert_eq!(s.active_enter_ts, None);
        assert_eq!(s.invocation_id, None);
    }

    #[test]
    fn timers() {
        let t = one(fixture!("shadow.timer"), "shadow.timer");
        assert_eq!(t.sub_state, "waiting");
        assert_eq!(t.trigger_unit.as_deref(), Some("shadow.service"));
        assert_eq!(t.last_trigger_ts, Some(1_794_092_408));
        assert_eq!(t.next_elapse_ts, Some(1_794_178_800));
        assert_eq!(t.n_restarts, None, "timers have no NRestarts");

        let t = one(
            fixture!("systemd-tmpfiles-clean.timer"),
            "systemd-tmpfiles-clean.timer",
        );
        assert_eq!(t.next_elapse_ts, None, "empty NextElapseUSecRealtime");
    }

    #[test]
    fn alias_found_through_names() {
        let blocks = parse_show(fixture!("dbus.service")).expect("parses");
        assert!(find(&blocks, "dbus.service").is_some());
        assert!(find(&blocks, "dbus-broker.service").is_some());
        assert!(find(&blocks, "other.service").is_none());
    }

    #[test]
    fn odd_values() {
        let text = "Id=a.service\nLoadState=loaded\nActiveState=active\nSubState=running\n\
                    Description=x=y\nMemoryCurrent=18446744073709551615\nNRestarts=\n\
                    ExecMainStatus=n/a\nActiveEnterTimestamp=@0\nExecMainStartTimestamp=infinity\n";
        let s = one(text, "a.service");
        assert_eq!(s.description, "x=y");
        assert_eq!(s.memory_current, None);
        assert_eq!(s.n_restarts, None);
        assert_eq!(s.exec_main_status, None);
        assert_eq!(s.active_enter_ts, None);
        assert_eq!(s.exec_main_start_ts, None);
    }

    #[test]
    fn malformed_output() {
        assert!(parse_show("garbage\n").is_err());
        assert!(parse_show("LoadState=loaded\n").is_err(), "no Id");
        let blocks = parse_show("Id=a.service\nLoadState=loaded\n").expect("parses");
        assert!(
            snapshot("a.service", &blocks[0]).is_err(),
            "ActiveState missing"
        );
    }

    #[test]
    fn unparseable_optional_values_are_unset_not_errors() {
        let text = "Id=a.service\nLoadState=loaded\nActiveState=active\nSubState=dead\n\
                    NRestarts=many\nExecMainStatus=-x\nMemoryCurrent=lots\n\
                    ActiveEnterTimestamp=Mon 2026-11-02 10:00:00 CET\nLastTriggerUSec=@soon\n";
        let s = one(text, "a.service");
        assert_eq!(s.active_state, "active");
        assert_eq!(s.n_restarts, None);
        assert_eq!(s.exec_main_status, None);
        assert_eq!(s.memory_current, None);
        assert_eq!(s.active_enter_ts, None);
        assert_eq!(s.last_trigger_ts, None);
    }

    /// Answers like systemctl from the fixtures, recording each call.
    fn fake_show<'a>(
        calls: &'a mut Vec<Vec<String>>,
    ) -> impl FnMut(&[String]) -> Result<String, CollectError> + 'a {
        move |units| {
            calls.push(units.to_vec());
            let blocks: Vec<String> = units
                .iter()
                .map(|u| match u.as_str() {
                    "music-sync.timer" => fixture!("music-sync.timer").to_owned(),
                    "music-sync-job.service" => fixture!("music-sync-job.service").to_owned(),
                    "shadow.timer" => fixture!("shadow.timer").to_owned(),
                    "shadow.service" => fixture!("shadow.service").to_owned(),
                    "dbus.service" => fixture!("dbus.service").to_owned(),
                    other => format!(
                        "Id={other}\nNames={other}\nDescription={other}\nLoadState=not-found\n\
                         ActiveState=inactive\nSubState=dead\n"
                    ),
                })
                .collect();
            Ok(blocks.join("\n"))
        }
    }

    fn units(snapshots: &[UnitSnapshot]) -> Vec<&str> {
        snapshots.iter().map(|s| s.unit.as_str()).collect()
    }

    #[test]
    fn one_call_when_unit_matches_the_stem() {
        let mut calls = Vec::new();
        let out = collect(
            &[
                UnitRequest::new("shadow.timer"),
                UnitRequest::new("dbus.service"),
            ],
            fake_show(&mut calls),
        )
        .expect("collects");
        assert_eq!(
            units(&out),
            ["shadow.timer", "shadow.service", "dbus.service"]
        );
        assert_eq!(calls, [["shadow.timer", "shadow.service", "dbus.service"]]);
    }

    #[test]
    fn second_call_for_a_timer_with_a_different_unit() {
        let mut calls = Vec::new();
        let out = collect(
            &[
                UnitRequest::new("music-sync.timer"),
                UnitRequest::new("nope.service"),
            ],
            fake_show(&mut calls),
        )
        .expect("collects");
        assert_eq!(
            units(&out),
            ["music-sync.timer", "music-sync-job.service", "nope.service"]
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1], ["music-sync-job.service"]);
        assert_eq!(out[1].sub_state, "dead");
        assert_eq!(out[2].load_state, "not-found");
    }

    #[test]
    fn configured_service_overrides_unit_and_dedups() {
        let mut calls = Vec::new();
        let out = collect(
            &[
                UnitRequest {
                    unit: "music-sync.timer".into(),
                    service: Some("shadow.service".into()),
                },
                UnitRequest::new("shadow.service"),
            ],
            fake_show(&mut calls),
        )
        .expect("collects");
        assert_eq!(units(&out), ["music-sync.timer", "shadow.service"]);
        assert_eq!(calls, [["music-sync.timer", "shadow.service"]]);
    }

    #[test]
    fn missing_timer_falls_back_to_stem_service() {
        let mut calls = Vec::new();
        let out =
            collect(&[UnitRequest::new("gone.timer")], fake_show(&mut calls)).expect("collects");
        assert_eq!(units(&out), ["gone.timer", "gone.service"]);
        assert_eq!(calls.len(), 1);
        assert!(out.iter().all(|s| s.load_state == "not-found"));
    }

    #[test]
    fn path_unit_with_unit_property_needs_no_second_call() {
        let mut calls = Vec::new();
        let mut show = fake_show(&mut calls);
        let out = collect(&[UnitRequest::new("watch.path")], |units: &[String]| {
            if units == ["watch.path"] {
                show(units)?;
                Ok(
                    "Id=watch.path\nLoadState=loaded\nActiveState=active\nSubState=waiting\n\
                    Unit=watch-handler.service\n"
                        .to_owned(),
                )
            } else {
                show(units)
            }
        })
        .expect("collects");
        drop(show);
        assert_eq!(units(&out), ["watch.path"]);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn no_requests_no_calls() {
        let mut calls = Vec::new();
        assert!(collect(&[], fake_show(&mut calls)).expect("ok").is_empty());
        assert!(calls.is_empty());
    }

    #[test]
    fn missing_block_is_an_error() {
        let result = collect(&[UnitRequest::new("a.service")], |_| Ok(String::new()));
        assert!(matches!(result, Err(CollectError::Parse { .. })));
    }
}
