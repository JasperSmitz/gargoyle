//! Configuration model, TOML parsing and validation (ARCHITECTURE §7).
//!
//! Parsing is two-stage so that `check-config` can report *every* problem at once:
//! 1. [`toml`] + serde turn the text into typed structs. Syntax errors, wrong types and unknown
//!    keys stop here, because serde cannot continue past them.
//! 2. [`Config::validate`] checks the semantic rules and collects all violations.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Suffixes accepted by the unit-name rule in ARCHITECTURE §4.
const UNIT_SUFFIXES: &[&str] = &["service", "timer", "socket", "mount", "target", "path"];

/// systemd's `UNIT_NAME_MAX`.
const MAX_UNIT_NAME_LEN: usize = 255;

const MIN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    // No `{0}` here: the toml error is the `source()`, and callers print the chain.
    #[error("invalid config file")]
    Parse(#[from] toml::de::Error),
    #[error("{} config error(s):{}", .0.len(), ValidationErrors(.0))]
    Invalid(Vec<ValidationError>),
}

/// One semantic problem, located by a TOML-style path such as `service[1].unit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

struct ValidationErrors<'a>(&'a [ValidationError]);

impl fmt::Display for ValidationErrors<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in self.0 {
            write!(f, "\n  - {e}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub storage: StorageConfig,
    pub collector: CollectorConfig,
    pub system: SystemConfig,
    pub thresholds: Thresholds,
    #[serde(rename = "service")]
    pub services: Vec<ServiceConfig>,
    #[serde(rename = "timer")]
    pub timers: Vec<TimerConfig>,
    #[serde(rename = "check")]
    pub checks: Vec<CheckConfig>,
    /// `None` without a `[library]` section: the feature is off (ADR 0007).
    pub library: Option<LibraryConfig>,
}

/// Two 5m rollup buckets: the open one plus the grace before it is rolled up, with margin.
const MIN_RAW_RETENTION: Duration = Duration::from_secs(600);

impl Config {
    /// Parses and validates a config file's contents.
    pub fn from_toml_str(text: &str) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(text)?;
        let errors = config.validate();
        if errors.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError::Invalid(errors))
        }
    }

    /// Returns every rule violation; an empty list means the config is valid.
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut v = Validator::default();
        self.validate_server_and_auth(&mut v);
        self.validate_storage(&mut v);
        self.validate_collector(&mut v);
        self.validate_system(&mut v);
        self.thresholds.validate(&mut v);
        self.validate_services(&mut v);
        self.validate_timers(&mut v);
        self.validate_checks(&mut v);
        if let Some(library) = &self.library {
            library.validate(&mut v);
        }
        v.errors
    }

    fn validate_server_and_auth(&self, v: &mut Validator) {
        let auth = &self.auth;
        if auth.mode == AuthMode::Basic {
            if auth.username.is_empty() {
                v.err("auth.username", "must not be empty when mode = \"basic\"");
            }
            match (&auth.password_hash, &auth.password_hash_file) {
                (None, None) => v.err(
                    "auth",
                    "mode = \"basic\" requires password_hash or password_hash_file",
                ),
                (Some(_), Some(_)) => v.err(
                    "auth",
                    "set only one of password_hash and password_hash_file",
                ),
                (Some(hash), None) if !hash.starts_with("$argon2id$") => v.err(
                    "auth.password_hash",
                    "must be an argon2id PHC string (generate one with `gargoyle hash-password`)",
                ),
                _ => {}
            }
        }

        let listen = self.server.listen;
        if listen.port() == 0 {
            v.err("server.listen", "port must not be 0");
        }
        if !listen.ip().is_loopback()
            && auth.mode == AuthMode::None
            && !self.server.allow_unauthenticated_remote
        {
            v.err(
                "server.listen",
                format!(
                    "{listen} is not a loopback address and [auth] mode is \"none\"; refusing to \
                     expose the dashboard without authentication. Set [auth] mode = \"basic\", or \
                     set server.allow_unauthenticated_remote = true if the network itself is \
                     trusted (e.g. a VPN interface)"
                ),
            );
        }
    }

    fn validate_storage(&self, v: &mut Validator) {
        let s = &self.storage;
        // Raw rows must outlive the open 5m bucket and the rollup grace, or they are deleted
        // before being rolled up.
        if s.raw_retention < MIN_RAW_RETENTION {
            v.err("storage.raw_retention", "must be at least 10m");
        }
        v.min_interval("storage.rollup_retention", s.rollup_retention);
        if s.raw_retention > s.rollup_retention {
            v.err(
                "storage.raw_retention",
                "must not be longer than rollup_retention",
            );
        }
    }

    fn validate_collector(&self, v: &mut Validator) {
        let c = &self.collector;
        v.min_interval("collector.system_interval", c.system_interval);
        v.min_interval("collector.units_interval", c.units_interval);
    }

    fn validate_system(&self, v: &mut Validator) {
        for (i, mount) in self.system.disks.iter().enumerate() {
            if !mount.starts_with('/') {
                v.err(
                    format!("system.disks[{i}]"),
                    format!("{mount:?} must be an absolute mount point"),
                );
            }
        }
    }

    fn validate_services(&self, v: &mut Validator) {
        let mut seen = HashSet::new();
        for (i, s) in self.services.iter().enumerate() {
            let path = format!("service[{i}].unit");
            v.unit_name(&path, &s.unit, None);
            if !seen.insert(s.unit.as_str()) {
                v.err(&path, format!("duplicate service {:?}", s.unit));
            }
        }
    }

    fn validate_timers(&self, v: &mut Validator) {
        let mut seen = HashSet::new();
        for (i, t) in self.timers.iter().enumerate() {
            let path = format!("timer[{i}].unit");
            v.unit_name(&path, &t.unit, Some("timer"));
            if !seen.insert(t.unit.as_str()) {
                v.err(&path, format!("duplicate timer {:?}", t.unit));
            }
            if let Some(service) = &t.service {
                v.unit_name(&format!("timer[{i}].service"), service, Some("service"));
            }
        }
    }

    fn validate_checks(&self, v: &mut Validator) {
        let mut seen = HashSet::new();
        for (i, c) in self.checks.iter().enumerate() {
            let at = |field: &str| format!("check[{i}].{field}");
            if c.name.trim().is_empty() {
                v.err(at("name"), "must not be empty");
            } else if c.name.trim() != c.name {
                v.err(at("name"), "must not start or end with whitespace");
            } else if !seen.insert(c.name.as_str()) {
                v.err(at("name"), format!("duplicate check name {:?}", c.name));
            }

            match c.kind {
                CheckKind::Http => {
                    match &c.url {
                        None => v.err(at("url"), "required for kind = \"http\""),
                        Some(url) => {
                            if let Some(problem) = target::url_problem(url) {
                                v.err(at("url"), problem);
                            }
                        }
                    }
                    if c.address.is_some() {
                        v.err(at("address"), "only valid for kind = \"tcp\"");
                    }
                    if let Some(status) = c.expect_status
                        && !(100..=599).contains(&status)
                    {
                        v.err(at("expect_status"), "must be an HTTP status code (100-599)");
                    }
                }
                CheckKind::Tcp => {
                    match &c.address {
                        None => v.err(at("address"), "required for kind = \"tcp\""),
                        Some(addr) if !is_host_port(addr) => {
                            v.err(at("address"), "must be host:port, e.g. \"127.0.0.1:25565\"")
                        }
                        Some(_) => {}
                    }
                    if c.url.is_some() {
                        v.err(at("url"), "only valid for kind = \"http\"");
                    }
                    if c.expect_status.is_some() {
                        v.err(at("expect_status"), "only valid for kind = \"http\"");
                    }
                }
            }

            v.min_interval(&at("interval"), c.interval);
            if c.timeout.is_zero() {
                v.err(at("timeout"), "must be greater than 0");
            } else if c.timeout > c.interval {
                v.err(at("timeout"), "must not be longer than interval");
            }
            if c.latency_warning.is_some_and(|d| d.is_zero()) {
                v.err(at("latency_warning"), "must be greater than 0");
            }
        }
    }
}

#[derive(Default)]
struct Validator {
    errors: Vec<ValidationError>,
}

impl Validator {
    fn err(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ValidationError {
            path: path.into(),
            message: message.into(),
        });
    }

    fn min_interval(&mut self, path: &str, d: Duration) {
        if d < MIN_INTERVAL {
            self.err(path, "must be at least 1s");
        }
    }

    fn unit_name(&mut self, path: &str, name: &str, required_suffix: Option<&str>) {
        if !is_valid_unit_name(name) {
            self.err(
                path,
                format!(
                    "{name:?} is not a valid unit name (letters, digits and :_.@- followed by \
                     .service, .timer, .socket, .mount, .target or .path)"
                ),
            );
        } else if let Some(suffix) = required_suffix
            && !name.ends_with(&format!(".{suffix}"))
        {
            self.err(path, format!("{name:?} must be a .{suffix} unit"));
        }
    }
}

/// Implements `^[A-Za-z0-9:_.@\-]+\.(service|timer|socket|mount|target|path)$` from ARCHITECTURE §4,
/// plus the systemd rules that the regex alone lets through: at most 255 characters, no leading
/// `@`, and no empty template instance (`foo@.service` names a template, which cannot be
/// queried).
///
/// Unit names end up as `systemctl` arguments, so this is a safety check, not just hygiene. It
/// also keeps the batched `systemctl show` working: one name systemd rejects fails the whole call,
/// every tick.
pub fn is_valid_unit_name(name: &str) -> bool {
    let Some((stem, suffix)) = name.rsplit_once('.') else {
        return false;
    };
    name.len() <= MAX_UNIT_NAME_LEN
        && !stem.is_empty()
        && !stem.starts_with('@')
        && !stem.ends_with('@')
        && UNIT_SUFFIXES.contains(&suffix)
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '@' | '-'))
}

/// `host:port`. The host may not contain `@`, `/` or whitespace: an address is echoed in the API
/// as is, so it must not be able to carry URL-style credentials.
fn is_host_port(addr: &str) -> bool {
    match addr.rsplit_once(':') {
        Some((host, port)) => {
            !host.is_empty()
                && !host.contains(|c: char| c == '@' || c == '/' || c.is_whitespace())
                && port.parse::<u16>().is_ok_and(|p| p != 0)
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub allow_unauthenticated_remote: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 9077)),
            allow_unauthenticated_remote: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    None,
    Basic,
}

#[derive(Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    pub mode: AuthMode,
    pub username: String,
    pub password_hash: Option<String>,
    pub password_hash_file: Option<PathBuf>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::None,
            username: "admin".to_owned(),
            password_hash: None,
            password_hash_file: None,
        }
    }
}

/// Hand-written so the password hash can never reach a log line via `{:?}`.
impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("mode", &self.mode)
            .field("username", &self.username)
            .field(
                "password_hash",
                &self.password_hash.as_ref().map(|_| "<redacted>"),
            )
            .field("password_hash_file", &self.password_hash_file)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub path: PathBuf,
    #[serde(with = "humantime_serde")]
    pub raw_retention: Duration,
    #[serde(with = "humantime_serde")]
    pub rollup_retention: Duration,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("/var/lib/gargoyle/gargoyle.db"),
            raw_retention: hours(48),
            rollup_retention: hours(400 * 24),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectorMode {
    #[default]
    Linux,
    Mock,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CollectorConfig {
    pub mode: CollectorMode,
    #[serde(with = "humantime_serde")]
    pub system_interval: Duration,
    #[serde(with = "humantime_serde")]
    pub units_interval: Duration,
    pub journal_backfill: bool,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            mode: CollectorMode::Linux,
            system_interval: Duration::from_secs(10),
            units_interval: Duration::from_secs(30),
            journal_backfill: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
    /// Empty means auto-detect real filesystems.
    pub disks: Vec<String>,
    /// Empty means all non-loopback, non-virtual interfaces.
    pub interfaces: Vec<String>,
    pub temperatures: bool,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            disks: Vec::new(),
            interfaces: Vec::new(),
            temperatures: true,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    pub warning: f64,
    pub critical: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowedLevels {
    pub warning: f64,
    pub critical: f64,
    /// Averaging window applied before evaluation.
    pub window: Duration,
}

/// Global thresholds. Each table in the file may set only some keys (`cpu = { warning = 80.0 }`);
/// the rest keep their defaults. `disk_mounts` entries inherit missing keys from `disk`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawThresholds")]
pub struct Thresholds {
    pub cpu: WindowedLevels,
    /// 1-minute load per core.
    pub load: WindowedLevels,
    pub memory: Levels,
    pub swap: Levels,
    pub disk: Levels,
    pub temperature: Levels,
    pub disk_mounts: BTreeMap<String, Levels>,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            cpu: windowed(85.0, 95.0, Duration::from_secs(120)),
            load: windowed(1.5, 3.0, Duration::from_secs(300)),
            memory: levels(85.0, 95.0),
            swap: levels(50.0, 90.0),
            disk: levels(80.0, 90.0),
            temperature: levels(75.0, 90.0),
            disk_mounts: BTreeMap::new(),
        }
    }
}

impl Thresholds {
    fn validate(&self, v: &mut Validator) {
        let mut check = |path: String, warning: f64, critical: f64| {
            // `partial_cmp` rather than `>=` so NaN (valid TOML: `nan`) is rejected too.
            if warning.partial_cmp(&critical) != Some(std::cmp::Ordering::Less) {
                v.err(
                    path,
                    format!("warning ({warning}) must be less than critical ({critical})"),
                );
            }
        };
        check("thresholds.cpu".into(), self.cpu.warning, self.cpu.critical);
        check(
            "thresholds.load".into(),
            self.load.warning,
            self.load.critical,
        );
        for (name, l) in [
            ("memory", self.memory),
            ("swap", self.swap),
            ("disk", self.disk),
            ("temperature", self.temperature),
        ] {
            check(format!("thresholds.{name}"), l.warning, l.critical);
        }
        for (mount, l) in &self.disk_mounts {
            check(
                format!("thresholds.disk_mounts.{mount:?}"),
                l.warning,
                l.critical,
            );
        }
        for (name, w) in [("cpu", self.cpu.window), ("load", self.load.window)] {
            v.min_interval(&format!("thresholds.{name}.window"), w);
        }
        for mount in self.disk_mounts.keys() {
            if !mount.starts_with('/') {
                v.err(
                    format!("thresholds.disk_mounts.{mount:?}"),
                    "must be an absolute mount point",
                );
            }
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawThresholds {
    cpu: RawWindowed,
    load: RawWindowed,
    memory: RawLevels,
    swap: RawLevels,
    disk: RawLevels,
    temperature: RawLevels,
    disk_mounts: BTreeMap<String, RawLevels>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawLevels {
    warning: Option<f64>,
    critical: Option<f64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawWindowed {
    warning: Option<f64>,
    critical: Option<f64>,
    #[serde(with = "humantime_serde")]
    window: Option<Duration>,
}

impl RawLevels {
    fn over(&self, base: Levels) -> Levels {
        levels(
            self.warning.unwrap_or(base.warning),
            self.critical.unwrap_or(base.critical),
        )
    }
}

impl RawWindowed {
    fn over(&self, base: WindowedLevels) -> WindowedLevels {
        windowed(
            self.warning.unwrap_or(base.warning),
            self.critical.unwrap_or(base.critical),
            self.window.unwrap_or(base.window),
        )
    }
}

impl From<RawThresholds> for Thresholds {
    fn from(raw: RawThresholds) -> Self {
        let d = Thresholds::default();
        let disk = raw.disk.over(d.disk);
        Thresholds {
            cpu: raw.cpu.over(d.cpu),
            load: raw.load.over(d.load),
            memory: raw.memory.over(d.memory),
            swap: raw.swap.over(d.swap),
            disk,
            temperature: raw.temperature.over(d.temperature),
            disk_mounts: raw
                .disk_mounts
                .into_iter()
                .map(|(mount, l)| (mount, l.over(disk)))
                .collect(),
        }
    }
}

fn levels(warning: f64, critical: f64) -> Levels {
    Levels { warning, critical }
}

fn windowed(warning: f64, critical: f64, window: Duration) -> WindowedLevels {
    WindowedLevels {
        warning,
        critical,
        window,
    }
}

fn hours(h: u64) -> Duration {
    Duration::from_secs(h * 3600)
}

// ---------------------------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Expect {
    /// ActiveState=active and SubState=running.
    #[default]
    Running,
    /// ActiveState=active, e.g. a oneshot with RemainAfterExit.
    Active,
    /// Anything but failed.
    Any,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    pub unit: String,
    /// Display name; the unit name when absent.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub expect: Expect,
    #[serde(default)]
    pub memory_warning_mb: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerConfig {
    pub unit: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Triggered service; the timer's `Unit=` property when absent.
    #[serde(default)]
    pub service: Option<String>,
    /// Warn if there has been no successful run within this age.
    #[serde(default, with = "humantime_serde")]
    pub max_age: Option<Duration>,
}

/// Shared by config (`kind = "http"`) and the API (`"kind": "http"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckKind {
    Http,
    Tcp,
}

/// `Debug` is written by hand so that the URL shows sanitized, like the password hash shows
/// redacted: a stray `{config:?}` in a log line must not leak check credentials.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    pub name: String,
    pub kind: CheckKind,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    /// Any 2xx when absent.
    #[serde(default)]
    pub expect_status: Option<u16>,
    #[serde(default = "default_check_interval", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default = "default_check_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    #[serde(default, with = "humantime_serde")]
    pub latency_warning: Option<Duration>,
}

impl CheckConfig {
    /// The configured URL or address as the API and logs show it (§6): a URL without query
    /// string and fragment, and only `scheme://host[:port]` if it has userinfo. Never use the raw
    /// `url` for display.
    pub fn display_target(&self) -> String {
        match self.kind {
            CheckKind::Http => self
                .url
                .as_deref()
                .map_or_else(String::new, target::display_url),
            CheckKind::Tcp => self
                .address
                .as_deref()
                .map_or_else(String::new, target::display_address),
        }
    }
}

impl fmt::Debug for CheckConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckConfig")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("url", &self.url.as_deref().map(target::display_url))
            .field(
                "address",
                &self.address.as_deref().map(target::display_address),
            )
            .field("expect_status", &self.expect_status)
            .field("interval", &self.interval)
            .field("timeout", &self.timeout)
            .field("latency_warning", &self.latency_warning)
            .finish()
    }
}

/// Shared by config (`warn_on = "error"`) and the API (`"warnOn": "error"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WarnOn {
    #[default]
    Error,
    Warning,
    Info,
    /// Only the report's age warns.
    Never,
}

/// `[library]`: the Liner report Gargoyle reads (ADR 0007). Gargoyle never runs Liner.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig {
    /// The report file. Never shown in the API or in messages (§9.4).
    pub report: PathBuf,
    #[serde(default = "default_library_name")]
    pub name: String,
    /// Warn when the report's `generated_at` is older than this.
    #[serde(default = "default_library_max_age", with = "humantime_serde")]
    pub max_age: Duration,
    /// The lowest issue severity that warns.
    #[serde(default)]
    pub warn_on: WarnOn,
    /// How often the file is checked for changes.
    #[serde(default = "default_library_interval", with = "humantime_serde")]
    pub interval: Duration,
}

impl LibraryConfig {
    fn validate(&self, v: &mut Validator) {
        if self.report.as_os_str().is_empty() {
            v.err("library.report", "must not be empty");
        }
        if self.name.trim().is_empty() {
            v.err("library.name", "must not be empty");
        } else if self.name.trim() != self.name {
            v.err("library.name", "must not start or end with whitespace");
        }
        if self.max_age < Duration::from_secs(60) {
            v.err("library.max_age", "must be at least 1m");
        }
        v.min_interval("library.interval", self.interval);
    }
}

fn default_library_name() -> String {
    "Music library".to_owned()
}

fn default_library_max_age() -> Duration {
    hours(26)
}

fn default_library_interval() -> Duration {
    Duration::from_secs(60)
}

fn default_check_interval() -> Duration {
    Duration::from_secs(60)
}

fn default_check_timeout() -> Duration {
    Duration::from_secs(5)
}

mod target;

#[cfg(test)]
mod tests;
