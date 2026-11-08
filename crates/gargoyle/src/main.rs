//! The `gargoyle` binary: CLI, scheduler, storage and HTTP server.

mod http;
mod scheduler;
mod state;
mod storage;
mod ui;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand};
use gargoyle_collector::{Collector, LinuxCollector, MockCollector};
use gargoyle_core::config::{AuthMode, CollectorMode, Config};
use tracing_subscriber::EnvFilter;

use crate::storage::{Storage, backfill_mock};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Read-only monitoring dashboard for a single Linux server"
)]
struct Cli {
    /// Path to the TOML config file.
    #[arg(
        long,
        short,
        global = true,
        value_name = "PATH",
        default_value = "/etc/gargoyle/gargoyle.toml"
    )]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the collector and web server (default).
    Run,
    /// Validate the config file, print every problem found, and exit.
    CheckConfig,
    /// Hash a password for `[auth] password_hash`.
    HashPassword,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing();

    let result = match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(&cli.config),
        Command::CheckConfig => check_config(&cli.config),
        Command::HashPassword => Err(anyhow::anyhow!(
            "hash-password is not implemented yet (ROADMAP M5.2)"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Logs go to stderr (journald captures them). `RUST_LOG` overrides the default `info` level.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        // No colour codes when stderr is journald or a file.
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

fn load_config(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config file {}", path.display()))?;
    Config::from_toml_str(&text).with_context(|| format!("config file {}", path.display()))
}

fn check_config(path: &Path) -> anyhow::Result<()> {
    load_config(path)?;
    println!("{}: OK", path.display());
    Ok(())
}

fn run(path: &Path) -> anyhow::Result<()> {
    // Validation includes the exposure guard: a non-loopback listen address without auth is
    // rejected here unless allow_unauthenticated_remote is set (ADR 0005).
    let config = load_config(path)?;
    let listen = config.server.listen;
    if !listen.ip().is_loopback() && config.auth.mode == AuthMode::None {
        tracing::warn!(
            %listen,
            "serving on a non-loopback address WITHOUT authentication (allow_unauthenticated_remote = true)"
        );
    }

    // An unusable database is a startup error (wrong path or permissions should be noticed);
    // failures after this point only cost history, never live data.
    let storage = Storage::open(&config.storage)?;
    let collector = build_collector(&config, &storage)?;
    let runtime = tokio::runtime::Runtime::new().context("cannot start async runtime")?;
    let result = runtime.block_on(async {
        let shared = scheduler::Shared::new(config, collector).with_storage(storage);
        // Without it, a restart logs every broken target again and journal backfill starts 30
        // days back; neither is worth refusing to start over.
        if let Err(e) = scheduler::restore(&shared).await {
            tracing::warn!(
                error = format!("{e:#}"),
                "cannot restore state from the database"
            );
        }
        let loops = scheduler::spawn(&shared);
        let result = http::serve(listen, shared).await;
        // No new ticks during shutdown, also when binding failed.
        for task in &loops {
            task.abort();
        }
        result
    });
    // Dropping the runtime would wait forever for a collector call that hung (stuck D-Bus, dead
    // NFS mount) and was left running on the blocking pool; give such calls 2s, then exit.
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    result
}

const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Fixed so `make dev` shows the same demo every run.
const MOCK_SEED: u64 = 0x6761_7267;

/// In mock mode, an empty database is first filled with a week of history (M2.4).
fn build_collector(config: &Config, storage: &Storage) -> anyhow::Result<Arc<dyn Collector>> {
    match config.collector.mode {
        CollectorMode::Mock => {
            let mock = MockCollector::new(MOCK_SEED);
            let started = Instant::now();
            let interval = config.collector.system_interval.as_secs();
            let interval = i64::try_from(interval).unwrap_or(i64::MAX);
            if let Some(samples) = backfill_mock(storage, &mock, scheduler::now(), interval)
                .context("cannot backfill mock history")?
            {
                tracing::info!(samples, elapsed = ?started.elapsed(), "backfilled mock history");
            }
            Ok(Arc::new(mock))
        }
        CollectorMode::Linux => Ok(Arc::new(LinuxCollector::new(config.system.clone()))),
    }
}
