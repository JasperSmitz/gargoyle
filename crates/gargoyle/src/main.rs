//! The `gargoyle` binary: CLI, HTTP server and (in later milestones) scheduler and storage.

mod http;
mod ui;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use gargoyle_core::config::{AuthMode, Config};
use tracing_subscriber::EnvFilter;

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

    let runtime = tokio::runtime::Runtime::new().context("cannot start async runtime")?;
    runtime.block_on(http::serve(listen))
}
