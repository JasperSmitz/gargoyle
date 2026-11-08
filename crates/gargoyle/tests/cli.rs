//! Runs the real binary: config checking and the startup exposure guard.

#![allow(clippy::unwrap_used)] // test code

use std::path::PathBuf;
use std::process::{Command, Output};

fn gargoyle(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gargoyle"))
        .args(args)
        .env("RUST_LOG", "warn")
        .output()
        .expect("binary runs")
}

fn repo_file(rel: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
        .display()
        .to_string()
}

/// Writes `contents` to a per-test file in the system temp dir and returns its path.
fn temp_config(name: &str, contents: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("gargoyle-test-{}-{name}.toml", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn shipped_configs_are_valid() {
    for rel in [
        "config/gargoyle.dev.toml",
        "config/gargoyle.example.toml",
        "config/gargoyle.local.toml",
    ] {
        let out = gargoyle(&["--config", &repo_file(rel), "check-config"]);
        assert!(
            out.status.success(),
            "{rel}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));
    }
}

#[test]
fn check_config_reports_all_errors() {
    let path = temp_config(
        "broken",
        r#"
[collector]
system_interval = "0s"

[thresholds]
disk = { warning = 95.0, critical = 90.0 }

[[service]]
unit = "not a unit"

[[check]]
name = "x"
kind = "tcp"
"#,
    );
    let out = gargoyle(&["check-config", "--config", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("4 config error(s)"), "{stderr}");
    for needle in [
        "collector.system_interval",
        "thresholds.disk",
        "service[0].unit",
        "check[0].address",
    ] {
        assert!(stderr.contains(needle), "missing {needle} in:\n{stderr}");
    }
}

#[test]
fn run_refuses_non_loopback_without_auth() {
    let path = temp_config("exposed", "[server]\nlisten = \"0.0.0.0:9077\"\n");
    // `run` is the default command; validation fails before any socket is bound.
    let out = gargoyle(&["--config", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("server.listen"), "{stderr}");
    assert!(stderr.contains("refusing"), "{stderr}");
}

#[test]
fn parse_errors_are_printed_once() {
    let path = temp_config("syntax", "[server]\nlisten = \"localhost:9077\"\n");
    let out = gargoyle(&["check-config", "--config", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid config file"), "{stderr}");
    assert_eq!(stderr.matches("TOML parse error").count(), 1, "{stderr}");
}

#[test]
fn missing_config_file_is_a_clear_error() {
    let out = gargoyle(&["--config", "/nonexistent/gargoyle.toml", "check-config"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot read config file"), "{stderr}");
}

#[test]
fn run_refuses_an_unreadable_password_hash_file() {
    let path = temp_config(
        "hashfile",
        "[auth]\nmode = \"basic\"\npassword_hash_file = \"/nonexistent/gargoyle-password\"\n",
    );
    // The hash is loaded before the database is opened or a socket is bound, and check-config
    // reads it too.
    for args in [&[][..], &["check-config"]] {
        let mut args = args.to_vec();
        args.extend(["--config", path.to_str().unwrap()]);
        let out = gargoyle(&args);
        assert!(!out.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("password_hash_file"), "{args:?}: {stderr}");
    }
    std::fs::remove_file(&path).ok();
}
