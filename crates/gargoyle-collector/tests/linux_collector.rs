//! `LinuxCollector` against fixture trees and a fake `systemctl` (ARCHITECTURE §11).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gargoyle_collector::linux::{JOURNAL_FIELDS, SYSTEMCTL_PROPERTIES};
use gargoyle_collector::{CollectError, Collector, LinuxCollector, UnitRequest};
use gargoyle_core::config::SystemConfig;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn collector(case: &str, system: SystemConfig) -> LinuxCollector {
    LinuxCollector {
        root: fixtures().join(case),
        systemctl: PathBuf::from("/nonexistent/systemctl"),
        journalctl: PathBuf::from("/nonexistent/journalctl"),
        system,
    }
}

#[test]
fn host_info_debian_server() {
    let info = collector("debian-server", SystemConfig::default())
        .host_info()
        .expect("host info");
    assert_eq!(info.hostname, "server");
    assert_eq!(info.os, "Debian GNU/Linux 13 (trixie)");
    assert_eq!(info.kernel, "6.12.48+deb13-amd64");
    assert_eq!(info.cpu_model, "Intel(R) Core(TM) i5-8500T CPU @ 2.10GHz");
    assert_eq!(info.cores, 4);
    assert_eq!(info.boot_time, 1_793_300_000);
}

#[test]
fn host_info_arm_and_many_cores() {
    let arm = collector("arm-no-hwmon", SystemConfig::default())
        .host_info()
        .expect("host info");
    assert_eq!(arm.cpu_model, "Raspberry Pi 4 Model B Rev 1.4");
    assert_eq!(arm.cores, 4);

    let many = collector("many-cores", SystemConfig::default())
        .host_info()
        .expect("host info");
    assert_eq!(many.cores, 64);
    assert_eq!(
        many.os, "Debian GNU/Linux 12 (bookworm)",
        "/usr/lib/os-release fallback"
    );
}

#[test]
fn system_snapshot_debian_server() {
    let snap = collector("debian-server", SystemConfig::default())
        .system_snapshot()
        .expect("snapshot");
    assert!(snap.ts > 1_700_000_000);
    assert_eq!(snap.uptime_s, 698_123.45);
    assert_eq!(snap.cpu.per_core.len(), 4);
    assert_eq!(snap.load, [0.42, 0.37, 0.31]);
    assert_eq!(snap.memory.total, 16_303_912 * 1024);

    let mounts: Vec<_> = snap.disks.iter().map(|d| d.mount.as_str()).collect();
    assert_eq!(mounts, ["/", "/boot/efi", "/srv/music library"]);
    for d in &snap.disks {
        // statvfs sees the filesystem holding the fixture tree; only sanity-check the maths.
        assert!(d.total > 0 && d.used + d.avail <= d.total, "{d:?}");
    }
    assert_eq!(snap.disks[2].fs_type, "ext4");

    let ifaces: Vec<_> = snap.net.iter().map(|n| n.iface.as_str()).collect();
    assert_eq!(ifaces, ["eno1", "wg0"]);
    assert_eq!(snap.temps.len(), 6);
}

#[test]
fn system_snapshot_honours_config() {
    let system = SystemConfig {
        disks: vec!["/srv/music library".into(), "/missing".into()],
        interfaces: vec!["docker0".into()],
        temperatures: false,
    };
    let snap = collector("debian-server", system)
        .system_snapshot()
        .expect("snapshot");
    let mounts: Vec<_> = snap.disks.iter().map(|d| d.mount.as_str()).collect();
    assert_eq!(mounts, ["/srv/music library"]);
    assert_eq!(snap.net.len(), 1);
    assert_eq!(snap.net[0].iface, "docker0");
    assert!(snap.temps.is_empty());
}

#[test]
fn system_snapshot_without_sensors_or_mount_dirs() {
    // many-cores has no /sys and its mount points do not exist in the tree: no temperatures and
    // the unreadable disks are skipped rather than failing the snapshot.
    let snap = collector("many-cores", SystemConfig::default())
        .system_snapshot()
        .expect("snapshot");
    assert!(snap.temps.is_empty());
    assert_eq!(snap.cpu.per_core.len(), 64);
    assert_eq!(snap.memory.swap_total, 0);
    assert_eq!(
        snap.disks
            .iter()
            .map(|d| d.mount.as_str())
            .collect::<Vec<_>>(),
        ["/"]
    );

    let arm = collector("arm-no-hwmon", SystemConfig::default())
        .system_snapshot()
        .expect("snapshot");
    assert_eq!(arm.temps.len(), 1);
    assert_eq!(arm.temps[0].label, "thermal/cpu-thermal");
}

#[test]
fn missing_proc_file_is_an_io_error() {
    let err = collector("does-not-exist", SystemConfig::default())
        .system_snapshot()
        .expect_err("no tree");
    assert!(matches!(err, CollectError::Io { .. }), "{err:?}");
}

// --- systemd ------------------------------------------------------------------------------------

/// Writing an executable and running it while another thread forks can fail with ETXTBSY, so the
/// tests that create and run fake scripts take turns.
static SCRIPTS: Mutex<()> = Mutex::new(());

/// A fake `systemctl` that checks it was called exactly as `systemctl show` with the whitelist,
/// logs the unit names of each call, and prints `fixtures/systemctl/<unit>.txt` per unit (or a
/// not-found block, as systemd does).
fn fake_systemctl(dir: &Path) -> PathBuf {
    let script = format!(
        r#"#!/bin/sh
expected='show --no-pager --timestamp=unix -p {props} --'
actual="$1 $2 $3 $4 $5 $6"
if [ "$actual" != "$expected" ]; then
    echo "unexpected arguments: $*" >&2
    exit 64
fi
shift 6
echo "$*" >> '{log}'
first=1
for unit in "$@"; do
    [ "$first" = 1 ] || echo
    first=0
    if [ -f '{fixtures}'/"$unit".txt ]; then
        cat '{fixtures}'/"$unit".txt
    else
        printf 'Id=%s\nNames=%s\nDescription=%s\nLoadState=not-found\nActiveState=inactive\nSubState=dead\nActiveEnterTimestamp=\nInvocationID=\nResult=success\nNRestarts=0\nExecMainStatus=0\nMemoryCurrent=[not set]\n' "$unit" "$unit" "$unit"
    fi
done
"#,
        props = SYSTEMCTL_PROPERTIES.join(","),
        log = dir.join("calls.log").display(),
        fixtures = fixtures().join("systemctl").display(),
    );
    write_script(dir, "systemctl", &script)
}

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write script");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn units_through_fake_systemctl() {
    let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut c = collector("debian-server", SystemConfig::default());
    c.systemctl = fake_systemctl(dir.path());

    let requests = [
        UnitRequest::new("NetworkManager.service"),
        UnitRequest::new("backup.service"),
        UnitRequest::new("systemd-tmpfiles-setup.service"),
        UnitRequest::new("dbus.service"),
        UnitRequest::new("missing.service"),
        UnitRequest::new("shadow.timer"),
        UnitRequest::new("music-sync.timer"),
    ];
    let snaps = c.units(&requests).expect("units");

    let summary: Vec<_> = snaps
        .iter()
        .map(|s| {
            format!(
                "{} {} {} {}",
                s.unit, s.load_state, s.active_state, s.sub_state
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            "NetworkManager.service loaded active running",
            "backup.service loaded failed failed",
            "systemd-tmpfiles-setup.service loaded active exited",
            "dbus.service loaded active running",
            "missing.service not-found inactive dead",
            "shadow.timer loaded active waiting",
            "shadow.service loaded inactive dead",
            "music-sync.timer loaded active waiting",
            "music-sync-job.service loaded inactive dead",
        ]
    );
    let timer = &snaps[7];
    assert_eq!(
        timer.trigger_unit.as_deref(),
        Some("music-sync-job.service")
    );
    assert_eq!(timer.last_trigger_ts, Some(1_794_106_800));
    assert_eq!(timer.next_elapse_ts, Some(1_794_193_200));
    assert_eq!(snaps[1].exec_main_status, Some(2));

    // One batched call, plus one for the timer whose Unit= is not <stem>.service.
    assert_eq!(
        calls(dir.path()),
        [
            "NetworkManager.service backup.service systemd-tmpfiles-setup.service dbus.service \
             missing.service shadow.timer shadow.service music-sync.timer music-sync.service",
            "music-sync-job.service",
        ]
    );
}

#[test]
fn systemctl_failures_are_command_errors() {
    let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut c = collector("debian-server", SystemConfig::default());
    let request = [UnitRequest::new("a.service")];

    c.systemctl = write_script(
        dir.path(),
        "failing",
        "#!/bin/sh\necho 'Failed to connect to bus' >&2\nexit 1\n",
    );
    match c.units(&request) {
        Err(CollectError::Command { program, message }) => {
            assert_eq!(program, "systemctl");
            assert!(message.contains("Failed to connect to bus"), "{message}");
        }
        other => panic!("expected a command error, got {other:?}"),
    }

    c.systemctl = dir.path().join("not-installed");
    assert!(matches!(
        c.units(&request),
        Err(CollectError::Command { .. })
    ));
}

#[test]
fn unit_names_are_arguments_not_shell() {
    let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut c = collector("debian-server", SystemConfig::default());
    c.systemctl = fake_systemctl(dir.path());
    // Config validation rejects this name; the collector must still never interpret it.
    let evil = "x.service; touch pwned";
    let snaps = c.units(&[UnitRequest::new(evil)]).expect("units");
    assert_eq!(snaps[0].unit, evil);
    assert!(!dir.path().join("pwned").exists());
    assert!(!Path::new("pwned").exists());
}

// --- journal ------------------------------------------------------------------------------------

/// A fake `journalctl` that checks the fixed argument vector, logs it, and prints
/// `fixtures/journal/handmade.json`.
fn fake_journalctl(dir: &Path) -> PathBuf {
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> '{log}'
[ "$1" = '--unit=backup.service' ] || {{ echo "bad unit: $1" >&2; exit 64; }}
[ "$2 $3 $4" = '--output=json --since=@1794000000 --no-pager' ] || {{ echo "bad args: $*" >&2; exit 64; }}
[ "$6" = '--output-fields={fields}' ] || {{ echo "bad fields: $6" >&2; exit 64; }}
[ "$7" = '_PID=1' ] || exit 64
cat '{fixture}'
"#,
        log = dir.join("calls.log").display(),
        fields = JOURNAL_FIELDS.join(","),
        fixture = fixtures().join("journal/handmade.json").display(),
    );
    write_script(dir, "journalctl", &script)
}

#[test]
fn timer_history_through_fake_journalctl() {
    let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut c = collector("debian-server", SystemConfig::default());
    c.journalctl = fake_journalctl(dir.path());

    let runs = c
        .timer_history("backup.timer", "backup.service", 1_794_000_000)
        .expect("history");
    let results: Vec<_> = runs.iter().map(|r| r.result.as_deref()).collect();
    assert_eq!(
        results,
        [Some("timeout"), Some("core-dump"), Some("success"), None]
    );
    assert!(runs.iter().all(|r| r.timer == "backup.timer"));
    assert_eq!(calls(dir.path()).len(), 1);
}

#[test]
fn timer_history_without_journal_access_is_empty() {
    let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut c = collector("debian-server", SystemConfig::default());
    c.journalctl = write_script(
        dir.path(),
        "journalctl",
        "#!/bin/sh\necho 'No journal files were opened due to insufficient permissions.' >&2\nexit 1\n",
    );
    for _ in 0..2 {
        assert_eq!(c.timer_history("a.timer", "a.service", 0), Ok(Vec::new()));
    }
    // Not installed at all.
    c.journalctl = dir.path().join("not-installed");
    assert_eq!(c.timer_history("a.timer", "a.service", 0), Ok(Vec::new()));
}

#[test]
fn timer_history_refuses_invalid_service_names() {
    let _turn = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut c = collector("debian-server", SystemConfig::default());
    c.journalctl = fake_journalctl(dir.path());
    for bad in ["x.service; touch pwned", "*.service", "a.timer", "--help"] {
        assert!(
            matches!(
                c.timer_history("a.timer", bad, 0),
                Err(CollectError::Command { .. })
            ),
            "{bad}"
        );
    }
    assert!(calls(dir.path()).is_empty(), "journalctl never ran");
}

// --- live host (make test-live) -----------------------------------------------------------------

#[test]
#[ignore = "reads the real host; run with `make test-live`"]
fn live_host() {
    let c = LinuxCollector::new(SystemConfig::default());
    let info = c.host_info().expect("host info");
    assert!(!info.hostname.is_empty() && info.cores > 0 && info.boot_time > 0);

    let snap = c.system_snapshot().expect("snapshot");
    assert_eq!(snap.cpu.per_core.len(), info.cores as usize);
    assert!(snap.memory.total > 0 && snap.memory.available <= snap.memory.total);
    assert!(
        snap.disks.iter().any(|d| d.mount == "/"),
        "{:?}",
        snap.disks
    );

    let snaps = c
        .units(&[
            UnitRequest::new("systemd-journald.service"),
            UnitRequest::new("gargoyle-does-not-exist.service"),
        ])
        .expect("units");
    assert_eq!(snaps.len(), 2);
    assert_eq!(snaps[0].active_state, "active");
    assert_eq!(snaps[1].load_state, "not-found");

    // Works with or without journal access; with access, runs are well-formed.
    let since = info.boot_time - 30 * 86_400;
    let runs = c
        .timer_history("shadow.timer", "shadow.service", since)
        .expect("history");
    for r in &runs {
        assert!(r.started_at >= since, "{r:?}");
        assert!(r.finished_at.is_none_or(|f| f >= r.started_at), "{r:?}");
        assert!(r.invocation_id.is_some());
    }
}
