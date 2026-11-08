use super::*;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

fn parse(text: &str) -> Result<Config, ConfigError> {
    Config::from_toml_str(text)
}

fn errors(text: &str) -> Vec<ValidationError> {
    match parse(text) {
        Err(ConfigError::Invalid(errors)) => errors,
        other => panic!("expected validation errors, got {other:?}"),
    }
}

fn error_paths(text: &str) -> Vec<String> {
    errors(text).into_iter().map(|e| e.path).collect()
}

#[test]
fn empty_file_gives_documented_defaults() {
    let c = parse("").unwrap();
    assert_eq!(c, Config::default());
    assert_eq!(c.server.listen, "127.0.0.1:9077".parse().unwrap());
    assert!(!c.server.allow_unauthenticated_remote);
    assert_eq!(c.auth.mode, AuthMode::None);
    assert_eq!(c.auth.username, "admin");
    assert_eq!(
        c.storage.path,
        PathBuf::from("/var/lib/gargoyle/gargoyle.db")
    );
    assert_eq!(c.storage.raw_retention, Duration::from_secs(48 * 3600));
    assert_eq!(c.storage.rollup_retention, Duration::from_secs(400 * 86400));
    assert_eq!(c.collector.mode, CollectorMode::Linux);
    assert_eq!(c.collector.system_interval, Duration::from_secs(10));
    assert_eq!(c.collector.units_interval, Duration::from_secs(30));
    assert!(c.collector.journal_backfill);
    assert!(c.system.disks.is_empty() && c.system.interfaces.is_empty());
    assert!(c.system.temperatures);
    let t = &c.thresholds;
    assert_eq!(
        (t.cpu.warning, t.cpu.critical, t.cpu.window),
        (85.0, 95.0, Duration::from_secs(120))
    );
    assert_eq!(
        (t.load.warning, t.load.critical, t.load.window),
        (1.5, 3.0, Duration::from_secs(300))
    );
    assert_eq!(t.memory, levels(85.0, 95.0));
    assert_eq!(t.swap, levels(50.0, 90.0));
    assert_eq!(t.disk, levels(80.0, 90.0));
    assert_eq!(t.temperature, levels(75.0, 90.0));
    assert!(c.services.is_empty() && c.timers.is_empty() && c.checks.is_empty());
}

#[test]
fn architecture_example_parses() {
    let text = format!(
        r#"
[server]
listen = "127.0.0.1:9077"
allow_unauthenticated_remote = false

[auth]
mode = "basic"
username = "admin"
password_hash = "{HASH}"

[storage]
path = "/var/lib/gargoyle/gargoyle.db"
raw_retention = "48h"
rollup_retention = "400d"

[collector]
mode = "linux"
system_interval = "10s"
units_interval = "30s"
journal_backfill = true

[system]
disks = []
interfaces = []
temperatures = true

[thresholds]
cpu       = {{ warning = 85.0, critical = 95.0, window = "2m" }}
load      = {{ warning = 1.5,  critical = 3.0,  window = "5m" }}
memory    = {{ warning = 85.0, critical = 95.0 }}
swap      = {{ warning = 50.0, critical = 90.0 }}
disk      = {{ warning = 80.0, critical = 90.0 }}
temperature = {{ warning = 75.0, critical = 90.0 }}
disk_mounts = {{ "/srv" = {{ warning = 90.0, critical = 95.0 }} }}

[[service]]
unit = "navidrome.service"
name = "Navidrome"
expect = "running"
memory_warning_mb = 2048

[[service]]
unit = "minecraft.service"
name = "Minecraft"

[[timer]]
unit = "music-sync.timer"
name = "Music sync"
max_age = "26h"

[[check]]
name = "Music HTTP"
kind = "http"
url = "http://127.0.0.1:4533/ping"
expect_status = 200
interval = "60s"
timeout = "5s"
latency_warning = "1s"

[[check]]
name = "Game port"
kind = "tcp"
address = "127.0.0.1:25565"
"#
    );
    let c = parse(&text).unwrap();
    assert_eq!(c.auth.mode, AuthMode::Basic);
    assert_eq!(c.thresholds.disk_mounts["/srv"], levels(90.0, 95.0));
    assert_eq!(c.services.len(), 2);
    assert_eq!(c.services[0].expect, Expect::Running);
    assert_eq!(c.services[0].memory_warning_mb, Some(2048));
    assert_eq!(c.services[1].name.as_deref(), Some("Minecraft"));
    assert_eq!(c.timers[0].max_age, Some(Duration::from_secs(26 * 3600)));
    assert_eq!(c.timers[0].service, None);
    assert_eq!(c.checks[0].kind, CheckKind::Http);
    assert_eq!(c.checks[0].latency_warning, Some(Duration::from_secs(1)));
    assert_eq!(c.checks[1].kind, CheckKind::Tcp);
    assert_eq!(c.checks[1].interval, Duration::from_secs(60));
    assert_eq!(c.checks[1].timeout, Duration::from_secs(5));
}

#[test]
fn partial_threshold_tables_keep_other_defaults() {
    let c = parse(
        r#"
[thresholds]
cpu = { warning = 70.0 }
load = { window = "10m" }
disk = { critical = 97.0 }
disk_mounts = { "/srv" = { warning = 93.0 } }
"#,
    )
    .unwrap();
    let t = &c.thresholds;
    assert_eq!(t.cpu, windowed(70.0, 95.0, Duration::from_secs(120)));
    assert_eq!(t.load, windowed(1.5, 3.0, Duration::from_secs(600)));
    assert_eq!(t.disk, levels(80.0, 97.0));
    // Missing keys in a mount override come from the (overridden) global disk thresholds.
    assert_eq!(t.disk_mounts["/srv"], levels(93.0, 97.0));
}

#[test]
fn durations_accept_humantime_forms() {
    let c = parse(
        r#"
[storage]
raw_retention = "1day 12h"
rollup_retention = "2y"
[collector]
system_interval = "1500ms"
"#,
    )
    .unwrap();
    assert_eq!(c.storage.raw_retention, Duration::from_secs(36 * 3600));
    assert!(c.storage.rollup_retention > Duration::from_secs(700 * 86400));
    assert_eq!(c.collector.system_interval, Duration::from_millis(1500));
}

#[test]
fn bad_duration_is_a_parse_error() {
    let err = parse("[collector]\nsystem_interval = \"ten seconds\"").unwrap_err();
    assert!(matches!(err, ConfigError::Parse(_)), "{err}");
}

#[test]
fn unknown_keys_are_rejected() {
    for text in [
        "[server]\nlisten_addr = \"127.0.0.1:1\"",
        "[bogus]\nx = 1",
        "[thresholds]\nmemory = { warning = 1.0, window = \"1m\" }",
        "[[service]]\nunit = \"a.service\"\nexpected = \"running\"",
    ] {
        let err = parse(text).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "{text}: {err}");
    }
}

#[test]
fn unit_name_rule() {
    let cases = [
        ("nginx.service", true),
        ("music-sync.timer", true),
        ("getty@tty1.service", true),
        ("systemd-fsck@dev-disk-by\\x2duuid.service", false),
        ("dev-sda1.mount", true),
        ("a:b_c.d.socket", true),
        ("multi-user.target", true),
        ("x.path", true),
        (".service", false),
        ("nginx", false),
        ("nginx.service ", false),
        ("nginx.scope", false),
        ("nginx.SERVICE", false),
        ("foo bar.service", false),
        ("foo;rm.service", false),
        ("foo/bar.service", false),
        ("", false),
        ("@.service", false),
        ("@foo.service", false),
        ("foo@.service", false),
        ("foo@bar@.service", false),
        ("foo@bar@baz.service", true),
    ];
    for (name, ok) in cases {
        assert_eq!(is_valid_unit_name(name), ok, "{name:?}");
    }

    // Length limit: 255 characters in total, suffix included.
    let longest = format!("{}.service", "a".repeat(255 - ".service".len()));
    assert_eq!(longest.len(), 255);
    assert!(is_valid_unit_name(&longest));
    let too_long = format!("a{longest}");
    assert!(!is_valid_unit_name(&too_long));
}

#[test]
fn broken_config_reports_every_error() {
    let text = r#"
[server]
listen = "0.0.0.0:9077"

[collector]
system_interval = "0s"
units_interval = "500ms"

[system]
disks = ["srv"]

[thresholds]
cpu = { warning = 95.0, critical = 90.0 }
memory = { warning = 90.0, critical = 90.0 }
disk_mounts = { "/srv" = { warning = 99.0, critical = 98.0 } }

[[service]]
unit = "nginx"

[[service]]
unit = "ok.service"

[[service]]
unit = "ok.service"

[[timer]]
unit = "backup.service"
service = "backup.timer"

[[check]]
name = "web"
kind = "http"
url = "ftp://example"
expect_status = 42

[[check]]
name = "web"
kind = "tcp"
address = "no-port"
interval = "10s"
timeout = "30s"
"#;
    let got = error_paths(text);
    let expected = [
        "server.listen",
        "collector.system_interval",
        "collector.units_interval",
        "system.disks[0]",
        "thresholds.cpu",
        "thresholds.memory",
        "thresholds.disk_mounts.\"/srv\"",
        "service[0].unit",
        "service[2].unit",
        "timer[0].unit",
        "timer[0].service",
        "check[0].url",
        "check[0].expect_status",
        "check[1].name",
        "check[1].address",
        "check[1].timeout",
    ];
    assert_eq!(got, expected);

    // The Display form lists them all, one per line.
    let err = parse(text).unwrap_err();
    let rendered = err.to_string();
    assert!(rendered.starts_with(&format!("{} config error(s):", expected.len())));
    assert_eq!(rendered.lines().count(), expected.len() + 1);
}

#[test]
fn non_loopback_without_auth_is_refused() {
    for listen in ["0.0.0.0:9077", "192.168.1.5:9077", "[::]:9077"] {
        let errs = errors(&format!("[server]\nlisten = \"{listen}\""));
        assert_eq!(errs.len(), 1, "{listen}");
        assert_eq!(errs[0].path, "server.listen");
        assert!(errs[0].message.contains("refusing"), "{}", errs[0].message);
    }
}

#[test]
fn non_loopback_allowed_with_auth_or_override() {
    let with_auth = format!(
        "[server]\nlisten = \"0.0.0.0:9077\"\n[auth]\nmode = \"basic\"\npassword_hash = \"{HASH}\""
    );
    assert!(parse(&with_auth).is_ok());
    assert!(
        parse("[server]\nlisten = \"0.0.0.0:9077\"\nallow_unauthenticated_remote = true").is_ok()
    );
    assert!(parse("[server]\nlisten = \"[::1]:9077\"").is_ok());
    assert!(parse("[server]\nlisten = \"127.0.0.2:9077\"").is_ok());
}

#[test]
fn basic_auth_requires_exactly_one_hash_source() {
    assert_eq!(error_paths("[auth]\nmode = \"basic\""), ["auth"]);
    let both = format!(
        "[auth]\nmode = \"basic\"\npassword_hash = \"{HASH}\"\npassword_hash_file = \"/etc/gargoyle/pw\""
    );
    assert_eq!(error_paths(&both), ["auth"]);
    assert_eq!(
        error_paths("[auth]\nmode = \"basic\"\npassword_hash = \"hunter2\""),
        ["auth.password_hash"]
    );
    assert_eq!(
        error_paths(&format!(
            "[auth]\nmode = \"basic\"\nusername = \"\"\npassword_hash = \"{HASH}\""
        )),
        ["auth.username"]
    );
    assert!(parse("[auth]\nmode = \"basic\"\npassword_hash_file = \"/etc/gargoyle/pw\"").is_ok());
}

#[test]
fn http_and_tcp_check_fields_must_match_kind() {
    let text = r#"
[[check]]
name = "a"
kind = "http"
address = "127.0.0.1:80"

[[check]]
name = "b"
kind = "tcp"
url = "http://x"
expect_status = 200
"#;
    assert_eq!(
        error_paths(text),
        [
            "check[0].url",
            "check[0].address",
            "check[1].address",
            "check[1].url",
            "check[1].expect_status",
        ]
    );
}

#[test]
fn storage_retentions_and_listen_port() {
    assert_eq!(
        error_paths("[storage]\nraw_retention = \"0s\"\nrollup_retention = \"0s\""),
        ["storage.raw_retention", "storage.rollup_retention"]
    );
    assert_eq!(
        error_paths("[storage]\nraw_retention = \"500d\""),
        ["storage.raw_retention"]
    );
    assert!(parse("[storage]\nraw_retention = \"400d\"").is_ok());
    assert_eq!(
        error_paths("[storage]\nraw_retention = \"9m 59s\""),
        ["storage.raw_retention"]
    );
    assert!(parse("[storage]\nraw_retention = \"10m\"").is_ok());
    assert_eq!(
        error_paths("[server]\nlisten = \"127.0.0.1:0\""),
        ["server.listen"]
    );
}

#[test]
fn check_names_and_latency_warning() {
    let text = r#"
[[check]]
name = "a "
kind = "tcp"
address = "127.0.0.1:1"

[[check]]
name = "  "
kind = "tcp"
address = "127.0.0.1:1"

[[check]]
name = "b"
kind = "tcp"
address = "127.0.0.1:1"
latency_warning = "0s"
"#;
    assert_eq!(
        error_paths(text),
        ["check[0].name", "check[1].name", "check[2].latency_warning"]
    );
}

#[test]
fn nan_thresholds_are_rejected() {
    assert_eq!(
        error_paths("[thresholds]\nswap = { warning = nan }"),
        ["thresholds.swap"]
    );
}

#[test]
fn debug_output_redacts_password_hash() {
    let c = parse(&format!(
        "[auth]\nmode = \"basic\"\npassword_hash = \"{HASH}\""
    ))
    .unwrap();
    let debug = format!("{c:?}");
    assert!(!debug.contains("argon2id"), "{debug}");
    assert!(debug.contains("<redacted>"));
}
