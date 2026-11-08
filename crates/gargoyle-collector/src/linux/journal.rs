//! Timer run history from the journal (M3.4, ARCHITECTURE §4).
//!
//! Runs are rebuilt from the structured messages the service manager (PID 1) logs about the
//! service unit, never from the service's own output. Each message carries the unit's
//! `INVOCATION_ID`, so the messages of one run are grouped by it. Verified against systemd 262;
//! every field and message ID used here exists since systemd 240, below the documented minimum
//! (252, Debian 12):
//!
//! | `MESSAGE_ID` (sd-messages.h) | Logged when | Fields used |
//! |---|---|---|
//! | `7d4958e8…` `UNIT_STARTING` | start job begins ("Starting X...") | — |
//! | `39f53479…` `UNIT_STARTED` | start job done ("Started X." / "Finished X.") | `JOB_RESULT=done` |
//! | `be02cf68…` `UNIT_FAILED` | start job failed ("Failed to start X.") | `JOB_RESULT` |
//! | `7ad2d189…` `UNIT_SUCCESS` | unit deactivated successfully | — |
//! | `d9b373ed…` `UNIT_FAILURE_RESULT` | unit failed ("Failed with result 'exit-code'.") | `UNIT_RESULT` |
//! | `98e32220…` `UNIT_PROCESS_EXIT` | a main or control process exited unsuccessfully | `EXIT_STATUS`, `COMMAND` |
//!
//! Plus `UNIT` (the unit the message is about), `_PID` (trusted, set by journald) and
//! `__REALTIME_TIMESTAMP` (µs, always present in `-o json`).
//!
//! Quirks this has to handle:
//! - `Type=simple`: no "Starting" message (the job finishes at once); `UNIT_STARTED` marks the
//!   start and `UNIT_SUCCESS` the end.
//! - `Type=oneshot`: `UNIT_STARTING` marks the start; `UNIT_STARTED` ("Finished X.") comes *after*
//!   the run ended, so it is not a start time.
//! - A rejected start (`start-limit-hit`) logs a failure under the previous invocation, even when
//!   that run succeeded; the earliest end message of an invocation is the result of that run.
//! - `UNIT_PROCESS_EXIT` is also logged for control processes (`ExecStopPost=`...). `COMMAND`
//!   (`ExecStart`, `ExecStopPost`, ...; an enum, not the command line) picks the main process
//!   where present (systemd 262 has it, older entries may not); otherwise the first exit wins.
//! - A run with a start but no end that is followed by a newer run was cut short (power loss,
//!   crash): it is finished at the next run's start with result `unknown`.
//! - A clean exit logs no `UNIT_PROCESS_EXIT`; a successful run gets exit status 0.
//! - Runs whose start is outside the queried window (or cut off) are dropped rather than given a
//!   wrong start time.
//! - A `RemainAfterExit=yes` oneshot logs no end until it is stopped, so its last run stays
//!   unfinished. Such services only ever run once per activation, so they make no sense behind a
//!   timer.
//!
//! `MESSAGE` is never requested: it is free text and could carry anything.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::{CollectError, TimerRunRecord};

const UNIT_STARTING: &str = "7d4958e842da4a758f6c1cdc7b36dcc5";
const UNIT_STARTED: &str = "39f53479d3a045ac8e11786248231fbf";
const UNIT_FAILED: &str = "be02cf6855d2428ba40df7e9d022f03d";
const UNIT_SUCCESS: &str = "7ad2d189f7e94e70a38c781354912448";
const UNIT_FAILURE_RESULT: &str = "d9b373ed55a64feb8242e02dbe79a49c";
const UNIT_PROCESS_EXIT: &str = "98e322203f7a4ed290d09fe03c09fe15";

const MESSAGE_IDS: &[&str] = &[
    UNIT_STARTING,
    UNIT_STARTED,
    UNIT_FAILED,
    UNIT_SUCCESS,
    UNIT_FAILURE_RESULT,
    UNIT_PROCESS_EXIT,
];

/// The only fields ever requested. None of them can carry secrets.
pub const FIELDS: &[&str] = &[
    "MESSAGE_ID",
    "INVOCATION_ID",
    "JOB_TYPE",
    "JOB_RESULT",
    "UNIT_RESULT",
    "EXIT_CODE",
    "EXIT_STATUS",
    "UNIT",
    "_PID",
    "COMMAND",
];

const PROGRAM: &str = "journalctl";

/// Bounds on one `journalctl` call, so a huge journal or a hung process cannot stall a blocking
/// thread or exhaust memory.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Kill `journalctl` after this long. The scheduler's own timeout cannot cancel a blocking
    /// thread, so the child must not outlive this.
    pub timeout: Duration,
    /// At most this many entries (`--lines`, newest kept). About four per run.
    pub max_entries: usize,
    /// Stop reading stdout after this many bytes and kill the process.
    pub max_bytes: usize,
}

pub const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(10),
    max_entries: 5000,
    max_bytes: 8 << 20,
};

const MAX_STDERR_BYTES: u64 = 16 << 10;

/// What one `journalctl` call produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetch {
    Output {
        stdout: String,
        /// A stderr line saying some journal files could not be read: the output may be partial.
        notice: Option<String>,
        /// Output hit `max_bytes`; the newest entries are missing.
        truncated: bool,
    },
    /// No journal, or no permission to read it.
    Unavailable(String),
}

/// The argument vector: `--unit` plus field matches (`_PID=1`, any of the message IDs) so only
/// the service manager's messages about this unit come back.
pub fn args(service: &str, since: i64, limits: &Limits) -> Vec<String> {
    let mut args = vec![
        format!("--unit={service}"),
        "--output=json".to_owned(),
        format!("--since=@{since}"),
        "--no-pager".to_owned(),
        format!("--lines={}", limits.max_entries),
        format!("--output-fields={}", FIELDS.join(",")),
        "_PID=1".to_owned(),
    ];
    args.extend(MESSAGE_IDS.iter().map(|id| format!("MESSAGE_ID={id}")));
    args
}

/// Runs `journalctl` with [`args`] within `limits`. Never through a shell.
pub fn fetch(
    journalctl: &Path,
    service: &str,
    since: i64,
    limits: &Limits,
) -> Result<Fetch, CollectError> {
    let child = Command::new(journalctl)
        .args(args(service, since, limits))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Fetch::Unavailable(format!(
                "{} not found",
                journalctl.display()
            )));
        }
        Err(e) => return Err(command_error(e.to_string())),
    };
    let output = read_bounded(&mut child, limits);
    // Reap in every case; a process killed for timeout or truncation exits promptly.
    if output.is_err() || output.as_ref().is_ok_and(|o| o.truncated) {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| command_error(e.to_string()));
    let output = output?;
    let status = status?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if output.truncated {
        return Ok(Fetch::Output {
            stdout: complete_lines(&stdout).to_owned(),
            notice: access_notice(&stderr),
            truncated: true,
        });
    }
    if !status.success() {
        if let Some(reason) = unavailable_reason(&stderr) {
            return Ok(Fetch::Unavailable(reason));
        }
        return Err(command_error(format!("{status}: {}", first_line(&stderr))));
    }
    Ok(Fetch::Output {
        stdout: stdout.into_owned(),
        notice: access_notice(&stderr),
        truncated: false,
    })
}

struct Bounded {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

enum Stream {
    Out(io::Result<Vec<u8>>),
    Err(io::Result<Vec<u8>>),
}

/// Reads stdout (up to `max_bytes`) and stderr on helper threads so neither pipe can fill up and
/// block the child, giving up at the deadline.
fn read_bounded(child: &mut Child, limits: &Limits) -> Result<Bounded, CollectError> {
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(command_error("stdio pipes missing".into()));
    };
    let (tx, rx) = mpsc::channel();
    let out_tx = tx.clone();
    let cap = limits.max_bytes as u64 + 1;
    thread::spawn(move || {
        let mut buf = Vec::new();
        let r = stdout.take(cap).read_to_end(&mut buf).map(|_| buf);
        let _ = out_tx.send(Stream::Out(r));
    });
    thread::spawn(move || {
        let mut stderr = stderr;
        let mut buf = Vec::new();
        let r = (&mut stderr)
            .take(MAX_STDERR_BYTES)
            .read_to_end(&mut buf)
            // Keep draining so a chatty stderr cannot block the child.
            .and_then(|_| io::copy(&mut stderr, &mut io::sink()))
            .map(|_| buf);
        let _ = tx.send(Stream::Err(r));
    });

    let deadline = Instant::now() + limits.timeout;
    let (mut out, mut err) = (None, None);
    while out.is_none() || err.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Stream::Out(r)) => {
                let mut buf = r.map_err(|e| command_error(e.to_string()))?;
                if buf.len() > limits.max_bytes {
                    buf.truncate(limits.max_bytes);
                    // The child may be blocked writing; the caller kills it, which closes stderr.
                    return Ok(Bounded {
                        stdout: buf,
                        stderr: Vec::new(),
                        truncated: true,
                    });
                }
                out = Some(buf);
            }
            Ok(Stream::Err(r)) => err = Some(r.unwrap_or_default()),
            Err(_) => {
                return Err(command_error(format!(
                    "timed out after {}s",
                    limits.timeout.as_secs_f32()
                )));
            }
        }
    }
    Ok(Bounded {
        stdout: out.unwrap_or_default(),
        stderr: err.unwrap_or_default(),
        truncated: false,
    })
}

/// journalctl exits non-zero with these when it cannot open any journal file.
fn unavailable_reason(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .find(|l| {
            l.contains("insufficient permissions")
                || l.contains("No journal files")
                || l.contains("Permission denied")
        })
        .map(|l| l.trim().to_owned())
}

/// journalctl exits 0 but warns when it could open only some files (e.g. a user outside the
/// `systemd-journal`/`adm` groups sees only their own journal, without PID 1's messages).
fn access_notice(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .find(|l| l.contains("not seeing messages") || l.contains("insufficient permissions"))
        .map(|l| l.trim().to_owned())
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect()
}

/// Drops a trailing partial line.
fn complete_lines(text: &str) -> &str {
    text.rfind('\n').map_or("", |i| &text[..=i])
}

fn command_error(message: String) -> CollectError {
    CollectError::Command {
        program: PROGRAM.into(),
        message,
    }
}

static WARNED: AtomicBool = AtomicBool::new(false);

/// Logs that the journal cannot be (fully) read, once per process: it will not change between
/// calls and must not flood the log. Returns whether it logged.
pub fn warn_once(reason: &str) -> bool {
    warn_once_with(&WARNED, reason)
}

fn warn_once_with(flag: &AtomicBool, reason: &str) -> bool {
    if flag.swap(true, Ordering::Relaxed) {
        return false;
    }
    tracing::warn!(
        reason,
        "cannot read the system journal; timer run history will not be backfilled \
         (add the gargoyle user to the systemd-journal group); logged once"
    );
    true
}

/// Runs rebuilt from `journalctl -o json` output, plus how many lines were unusable.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Oldest first.
    pub runs: Vec<TimerRunRecord>,
    /// Lines that were not a JSON object with a timestamp.
    pub skipped: usize,
}

/// One invocation's messages, reduced to what a run needs. Timestamps in µs.
#[derive(Debug, Default)]
struct Invocation {
    starting: Option<i64>,
    started: Option<i64>,
    success: Option<i64>,
    failure: Option<(i64, String)>,
    job_failed: Option<(i64, String)>,
    /// First `UNIT_PROCESS_EXIT` of any process, and of the main process (`COMMAND=ExecStart`).
    first_exit: Option<i32>,
    main_exit: Option<i32>,
}

/// Rebuilds the runs of `service` from journal entries. Entries not logged by PID 1 about
/// `service`, or without an invocation ID, are ignored; unparseable lines are counted.
pub fn parse_runs(text: &str, timer: &str, service: &str) -> Parsed {
    let mut skipped = 0;
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Invocation> = HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some((ts, entry)) = parse_entry(line) else {
            skipped += 1;
            continue;
        };
        let field = |k: &str| entry.get(k).and_then(Value::as_str);
        if field("_PID") != Some("1") || field("UNIT") != Some(service) {
            continue;
        }
        let (Some(id), Some(message_id)) = (field("INVOCATION_ID"), field("MESSAGE_ID")) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let inv = by_id.entry(id.to_owned()).or_insert_with(|| {
            order.push(id.to_owned());
            Invocation::default()
        });
        match message_id {
            UNIT_STARTING => {
                inv.starting.get_or_insert(ts);
            }
            UNIT_STARTED if field("JOB_RESULT") == Some("done") => {
                inv.started.get_or_insert(ts);
            }
            UNIT_SUCCESS => {
                inv.success.get_or_insert(ts);
            }
            UNIT_FAILURE_RESULT => {
                if let Some(result) = field("UNIT_RESULT") {
                    inv.failure.get_or_insert((ts, result.to_owned()));
                }
            }
            UNIT_FAILED => {
                if let Some(result) = field("JOB_RESULT") {
                    inv.job_failed.get_or_insert((ts, result.to_owned()));
                }
            }
            UNIT_PROCESS_EXIT => {
                if let Some(status) = field("EXIT_STATUS").and_then(exit_status) {
                    inv.first_exit.get_or_insert(status);
                    if field("COMMAND") == Some("ExecStart") {
                        inv.main_exit.get_or_insert(status);
                    }
                }
            }
            _ => {}
        }
    }

    let mut runs: Vec<TimerRunRecord> = order
        .into_iter()
        .filter_map(|id| {
            let inv = by_id.remove(&id)?;
            inv.into_run(timer, service, id)
        })
        .collect();
    runs.sort_by_key(|r| r.started_at);
    // A unit has one invocation at a time: an unfinished run followed by a newer one ended
    // without systemd logging it.
    let next_starts: Vec<i64> = runs.iter().skip(1).map(|r| r.started_at).collect();
    for (run, next_start) in runs.iter_mut().zip(next_starts) {
        if run.finished_at.is_none() {
            run.finished_at = Some(next_start);
            run.result = Some("unknown".to_owned());
            run.exit_status = None;
        }
    }
    Parsed { runs, skipped }
}

impl Invocation {
    fn into_run(self, timer: &str, service: &str, id: String) -> Option<TimerRunRecord> {
        // The earliest end wins: a later `start-limit-hit` under the same invocation describes a
        // rejected new start, not this run. On a tie the failure wins.
        let success = self.success.map(|ts| (ts, "success".to_owned()));
        let end = match (self.failure, success) {
            (Some(f), Some(s)) => Some(if f.0 <= s.0 { f } else { s }),
            (f, s) => f.or(s).or(self.job_failed),
        };
        let start = match (self.starting, self.started) {
            (Some(ts), _) => ts,
            // `UNIT_STARTED` is the start only if nothing ended before it (Type=simple). For a
            // oneshot it is logged after the run and the real start is outside the window.
            (None, Some(ts)) if end.as_ref().is_none_or(|(end, _)| *end >= ts) => ts,
            _ => return None,
        };
        let exit_status = match (&end, self.main_exit.or(self.first_exit)) {
            (Some((_, result)), _) if result == "success" => Some(0),
            (_, Some(status)) => Some(status),
            (Some((_, result)), None) if result == "success" => Some(0),
            _ => None,
        };
        Some(TimerRunRecord {
            timer: timer.to_owned(),
            service: service.to_owned(),
            invocation_id: Some(id),
            started_at: micros_to_secs(start),
            finished_at: end.as_ref().map(|(ts, _)| micros_to_secs(*ts)),
            result: end.map(|(_, result)| result),
            exit_status,
        })
    }
}

fn parse_entry(line: &str) -> Option<(i64, Map<String, Value>)> {
    let entry: Map<String, Value> = serde_json::from_str(line).ok()?;
    let ts = entry
        .get("__REALTIME_TIMESTAMP")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    Some((ts, entry))
}

fn micros_to_secs(us: i64) -> i64 {
    us.div_euclid(1_000_000)
}

/// `EXIT_STATUS` is a number for `EXIT_CODE=exited` and a signal name (`TERM`) for
/// `killed`/`dumped`. Signals map to their number, as `ExecMainStatus` reports them.
fn exit_status(value: &str) -> Option<i32> {
    if let Ok(n) = value.parse() {
        return Some(n);
    }
    let name = value.strip_prefix("SIG").unwrap_or(value);
    let name = match name {
        "IOT" => "ABRT",
        "POLL" => "IO",
        other => other,
    };
    SIGNALS
        .iter()
        .position(|s| *s == name)
        .and_then(|i| i32::try_from(i + 1).ok())
}

/// Linux signal names 1–31 (x86 and arm numbering), as systemd's `signal_to_string` prints them.
const SIGNALS: [&str; 31] = [
    "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "BUS", "FPE", "KILL", "USR1", "SEGV", "USR2",
    "PIPE", "ALRM", "TERM", "STKFLT", "CHLD", "CONT", "STOP", "TSTP", "TTIN", "TTOU", "URG",
    "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "IO", "PWR", "SYS",
];

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!("../../tests/fixtures/journal/", $name, ".json"))
        };
    }

    /// `(started_at, finished_at, result, exit_status)`.
    type Row<'a> = (i64, Option<i64>, Option<&'a str>, Option<i32>);

    fn summary(p: &Parsed) -> Vec<Row<'_>> {
        p.runs
            .iter()
            .map(|r| {
                (
                    r.started_at,
                    r.finished_at,
                    r.result.as_deref(),
                    r.exit_status,
                )
            })
            .collect()
    }

    #[test]
    fn fields_carry_no_free_text() {
        assert!(!FIELDS.contains(&"MESSAGE"));
        assert!(
            FIELDS
                .iter()
                .all(|f| !f.contains("CMDLINE") && !f.contains("ENV"))
        );
    }

    #[test]
    fn args_are_fixed() {
        let a = args("music-sync.service", 1_794_000_000, &LIMITS);
        assert_eq!(a[0], "--unit=music-sync.service");
        assert!(a.contains(&"--output=json".to_owned()));
        assert!(a.contains(&"--since=@1794000000".to_owned()));
        assert!(a.contains(&"--lines=5000".to_owned()));
        assert!(a.contains(&"_PID=1".to_owned()));
        assert_eq!(a.iter().filter(|x| x.starts_with("MESSAGE_ID=")).count(), 6);
        assert!(a.contains(&format!("--output-fields={}", FIELDS.join(","))));
        // A leading dash cannot turn the unit into an option.
        assert_eq!(args("-x.service", 0, &LIMITS)[0], "--unit=-x.service");
    }

    #[test]
    fn oneshot_successes() {
        let p = parse_runs(fixture!("man-db"), "man-db.timer", "man-db.service");
        assert_eq!(p.skipped, 0);
        assert_eq!(p.runs.len(), 5);
        let first = &p.runs[0];
        assert_eq!(first.timer, "man-db.timer");
        assert_eq!(first.service, "man-db.service");
        assert_eq!(
            first.invocation_id.as_deref(),
            Some("2d371a06f8f44b4f8789b756218100a5")
        );
        // Start from "Starting", end from "Deactivated successfully", not from "Finished".
        assert_eq!(
            summary(&p)[0],
            (1_793_393_774, Some(1_793_393_775), Some("success"), Some(0))
        );
        assert!(p.runs.windows(2).all(|w| w[0].started_at < w[1].started_at));
        assert!(
            p.runs
                .iter()
                .all(|r| r.result.as_deref() == Some("success"))
        );
    }

    #[test]
    fn simple_service_starts_at_started() {
        let p = parse_runs(fixture!("shadow"), "shadow.timer", "shadow.service");
        assert_eq!(
            summary(&p),
            [
                (1_793_886_144, Some(1_793_886_144), Some("success"), Some(0)),
                (1_793_974_991, Some(1_793_974_991), Some("success"), Some(0)),
                (1_794_006_006, Some(1_794_006_006), Some("success"), Some(0)),
                (1_794_092_408, Some(1_794_092_408), Some("success"), Some(0)),
            ]
        );
    }

    #[test]
    fn failed_runs_with_exit_status() {
        let p = parse_runs(
            fixture!("pcrproduct-failed"),
            "x.timer",
            "systemd-pcrproduct.service",
        );
        // The first invocation's "Starting" is before the window: dropped, not misdated.
        assert_eq!(
            summary(&p),
            [
                (
                    1_791_511_115,
                    Some(1_791_511_115),
                    Some("exit-code"),
                    Some(1)
                ),
                (
                    1_791_511_115,
                    Some(1_791_511_115),
                    Some("exit-code"),
                    Some(1)
                ),
                (
                    1_791_511_116,
                    Some(1_791_511_116),
                    Some("exit-code"),
                    Some(1)
                ),
            ]
        );
    }

    #[test]
    fn start_limit_hit_does_not_overwrite_the_run_result() {
        let p = parse_runs(
            fixture!("pcrproduct-start-limit"),
            "x.timer",
            "systemd-pcrproduct.service",
        );
        assert_eq!(p.runs.len(), 4);
        let last = p.runs.last().expect("runs");
        assert_eq!(
            last.invocation_id.as_deref(),
            Some("1f7b8b0b77a54274a764ef1f473d0a82")
        );
        assert_eq!(last.result.as_deref(), Some("exit-code"));
        assert_eq!(last.finished_at, Some(1_792_588_421));
    }

    #[test]
    fn oneshot_cut_off_at_the_start_is_dropped() {
        let p = parse_runs(
            fixture!("tmpfiles-clean-cut"),
            "systemd-tmpfiles-clean.timer",
            "systemd-tmpfiles-clean.service",
        );
        // Only "Deactivated" + "Finished" of the first run are in the window.
        assert_eq!(p.runs.len(), 2);
        assert!(p.runs.iter().all(|r| r.started_at > 1_793_887_068));
    }

    #[test]
    fn hand_made_edge_cases() {
        let p = parse_runs(fixture!("handmade"), "backup.timer", "backup.service");
        assert_eq!(p.skipped, 2, "malformed line and entry without timestamp");
        let ids: Vec<_> = p
            .runs
            .iter()
            .map(|r| r.invocation_id.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(ids, ["a-timeout", "b-coredump", "c-ok", "d-running"]);
        assert_eq!(
            summary(&p),
            [
                (
                    1_794_100_000,
                    Some(1_794_100_090),
                    Some("timeout"),
                    Some(15)
                ),
                (
                    1_794_103_600,
                    Some(1_794_103_601),
                    Some("core-dump"),
                    Some(11)
                ),
                (1_794_107_200, Some(1_794_107_242), Some("success"), Some(0)),
                (1_794_110_800, None, None, None),
            ]
        );
    }

    /// One journal line from PID 1 about `backup.service`.
    fn line(ts_s: f64, id: &str, msg: &str, extra: &str) -> String {
        let ts = (ts_s * 1e6) as i64;
        format!(
            r#"{{"__REALTIME_TIMESTAMP":"{ts}","_PID":"1","UNIT":"backup.service","MESSAGE_ID":"{msg}","INVOCATION_ID":"{id}"{extra}}}"#
        ) + "\n"
    }

    fn parse(lines: &[String]) -> Parsed {
        parse_runs(&lines.concat(), "backup.timer", "backup.service")
    }

    #[test]
    fn success_then_start_limit_hit_stays_success() {
        let p = parse(&[
            line(100.0, "ok", UNIT_STARTING, ""),
            line(130.0, "ok", UNIT_SUCCESS, ""),
            line(130.1, "ok", UNIT_STARTED, r#","JOB_RESULT":"done""#),
            line(
                3700.0,
                "ok",
                UNIT_FAILURE_RESULT,
                r#","UNIT_RESULT":"start-limit-hit""#,
            ),
            line(3700.1, "ok", UNIT_FAILED, r#","JOB_RESULT":"failed""#),
        ]);
        assert_eq!(summary(&p), [(100, Some(130), Some("success"), Some(0))]);
    }

    #[test]
    fn interrupted_run_is_finished_by_the_next_one() {
        let p = parse(&[
            line(100.0, "stale", UNIT_STARTING, ""),
            line(3600.0, "ok", UNIT_STARTING, ""),
            line(3630.0, "ok", UNIT_SUCCESS, ""),
            line(7200.0, "running", UNIT_STARTING, ""),
        ]);
        assert_eq!(
            summary(&p),
            [
                (100, Some(3600), Some("unknown"), None),
                (3600, Some(3630), Some("success"), Some(0)),
                (7200, None, None, None),
            ]
        );
    }

    #[test]
    fn exit_status_is_the_main_process_or_the_first() {
        let exit = |status: &str, command: &str| {
            format!(r#","EXIT_CODE":"exited","EXIT_STATUS":"{status}"{command}"#)
        };
        let failure = r#","UNIT_RESULT":"exit-code""#;
        // Without COMMAND (older systemd): the first exit, not a later ExecStopPost.
        let p = parse(&[
            line(100.0, "a", UNIT_STARTING, ""),
            line(110.0, "a", UNIT_PROCESS_EXIT, &exit("1", "")),
            line(111.0, "a", UNIT_PROCESS_EXIT, &exit("7", "")),
            line(111.1, "a", UNIT_FAILURE_RESULT, failure),
        ]);
        assert_eq!(p.runs[0].exit_status, Some(1));
        // With COMMAND: the main process, even after a failed ExecStartPre-style control exit.
        let p = parse(&[
            line(100.0, "b", UNIT_STARTING, ""),
            line(
                105.0,
                "b",
                UNIT_PROCESS_EXIT,
                &exit("3", r#","COMMAND":"ExecCondition""#),
            ),
            line(
                110.0,
                "b",
                UNIT_PROCESS_EXIT,
                &exit("2", r#","COMMAND":"ExecStart""#),
            ),
            line(
                111.0,
                "b",
                UNIT_PROCESS_EXIT,
                &exit("7", r#","COMMAND":"ExecStopPost""#),
            ),
            line(111.1, "b", UNIT_FAILURE_RESULT, failure),
        ]);
        assert_eq!(p.runs[0].exit_status, Some(2));
    }

    #[test]
    fn signal_names_map_to_numbers() {
        assert_eq!(exit_status("2"), Some(2));
        assert_eq!(exit_status("TERM"), Some(15));
        assert_eq!(exit_status("SIGKILL"), Some(9));
        assert_eq!(exit_status("XCPU"), Some(24));
        assert_eq!(exit_status("IOT"), Some(6));
        assert_eq!(exit_status("PWR"), Some(30));
        assert_eq!(exit_status("SYS"), Some(31));
        assert_eq!(exit_status("RTMIN+3"), None);
    }

    #[test]
    fn empty_and_garbage_input() {
        assert_eq!(parse_runs("", "a.timer", "a.service"), Parsed::default());
        let p = parse_runs("not json\n[1,2]\n\n", "a.timer", "a.service");
        assert_eq!((p.runs.len(), p.skipped), (0, 2));
    }

    #[test]
    fn complete_lines_drops_partial_tail() {
        assert_eq!(complete_lines("a\nb\nc"), "a\nb\n");
        assert_eq!(complete_lines("abc"), "");
        assert_eq!(complete_lines("a\n"), "a\n");
    }

    #[test]
    fn stderr_classification() {
        assert!(
            unavailable_reason("No journal files were opened due to insufficient permissions.\n")
                .is_some()
        );
        assert!(unavailable_reason("No journal files were found.\n").is_some());
        assert!(unavailable_reason("Failed to add match: Invalid argument\n").is_none());
        let hint = "Hint: You are currently not seeing messages from other users and the system.\n      Users in groups 'adm', 'systemd-journal' can see all messages.\n";
        assert_eq!(
            access_notice(hint).as_deref(),
            Some("Hint: You are currently not seeing messages from other users and the system.")
        );
        assert!(access_notice("").is_none());
    }

    #[test]
    fn warns_only_once() {
        let flag = AtomicBool::new(false);
        assert!(warn_once_with(&flag, "denied"));
        assert!(!warn_once_with(&flag, "denied"));
        assert!(!warn_once_with(&flag, "other"));
    }

    // --- fetch against fake scripts ---

    /// Writing an executable while another thread forks can fail with ETXTBSY; take turns.
    static SCRIPTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("journalctl");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    fn fetch_with(body: &str, limits: &Limits) -> Result<Fetch, CollectError> {
        let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        fetch(&script(dir.path(), body), "a.service", 0, limits)
    }

    const FAST: Limits = Limits {
        timeout: Duration::from_millis(500),
        max_entries: 10,
        max_bytes: 1000,
    };

    #[test]
    fn hung_journalctl_is_killed_at_the_deadline() {
        let start = Instant::now();
        let r = fetch_with("exec sleep 30", &FAST);
        assert!(
            matches!(&r, Err(CollectError::Command { message, .. }) if message.contains("timed out")),
            "{r:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn huge_output_is_truncated_to_complete_lines() {
        let r = fetch_with("exec yes '{\"_PID\":\"1\"}'", &FAST).expect("fetches");
        let Fetch::Output {
            stdout, truncated, ..
        } = r
        else {
            panic!("expected output, got {r:?}");
        };
        assert!(truncated);
        assert!(stdout.len() <= FAST.max_bytes && stdout.ends_with('\n'));
        assert!(stdout.lines().all(|l| l == "{\"_PID\":\"1\"}"));
    }

    #[test]
    fn no_permission_or_journal_is_unavailable() {
        for msg in [
            "No journal files were opened due to insufficient permissions.",
            "No journal files were found.",
        ] {
            let r = fetch_with(&format!("echo '{msg}' >&2; exit 1"), &FAST).expect("fetches");
            assert_eq!(r, Fetch::Unavailable(msg.into()));
        }
        let missing = fetch(Path::new("/nonexistent/journalctl"), "a.service", 0, &FAST);
        assert!(matches!(missing, Ok(Fetch::Unavailable(_))), "{missing:?}");
    }

    #[test]
    fn partial_access_is_a_notice() {
        let r = fetch_with(
            "echo 'Hint: You are currently not seeing messages from other users and the system.' >&2; echo '{}'",
            &FAST,
        )
        .expect("fetches");
        assert_eq!(
            r,
            Fetch::Output {
                stdout: "{}\n".into(),
                notice: Some(
                    "Hint: You are currently not seeing messages from other users and the system."
                        .into()
                ),
                truncated: false,
            }
        );
    }

    #[test]
    fn other_failures_are_errors() {
        let r = fetch_with(
            "echo 'Failed to add match: Invalid argument' >&2; exit 1",
            &FAST,
        );
        assert!(
            matches!(&r, Err(CollectError::Command { message, .. }) if message.contains("Invalid argument")),
            "{r:?}"
        );
    }
}
