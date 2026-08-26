//! `atlas` — developer CLI for the Atlas platform.
//!
//! Reads `atlas.toml` from the current directory (or `--config`) and talks
//! to the Atlas control plane over REST. Use `--mock` to exercise commands
//! without a running backend.

mod api;
mod commands;
mod config;

use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::Format;
use owo_colors::OwoColorize;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "atlas",
    version,
    about = "Developer CLI for the Atlas platform.",
    long_about = "Manage Atlas projects from your terminal: validate atlas.toml, deploy, \
                  monitor service status, tail logs, and manage API keys."
)]
struct Cli {
    /// Path to atlas.toml.
    #[arg(long, default_value = "atlas.toml", global = true)]
    config: PathBuf,

    /// Emit machine-readable JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Hit the real control plane over HTTP. Defaults off while the
    /// control plane is not yet built — without `--live`, every command
    /// uses deterministic in-memory mock responses.
    #[arg(long, global = true)]
    live: bool,

    /// Override the control plane base URL. Only used with `--live`.
    #[arg(long, global = true)]
    base_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and validate atlas.toml.
    Validate,
    /// Validate and deploy the project to the Atlas control plane.
    Deploy,
    /// Print current service health for this project.
    Status,
    /// Tail logs for one or all services.
    Logs {
        /// Service to filter by: auth | geo | payments | events. Omit for all.
        service: Option<String>,
    },
    /// Manage API keys for this project.
    Keys {
        #[command(subcommand)]
        action: commands::keys::KeysCommand,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = if cli.json {
        Format::Json
    } else {
        Format::Human
    };

    let result: Result<()> = match cli.command {
        Command::Validate => commands::validate::run(&cli.config, format),
        Command::Deploy => {
            commands::deploy::run(&cli.config, format, !cli.live, cli.base_url).await
        }
        Command::Status => {
            commands::status::run(&cli.config, format, !cli.live, cli.base_url).await
        }
        Command::Logs { service } => {
            commands::logs::run(&cli.config, service, format, !cli.live, cli.base_url).await
        }
        Command::Keys { action } => {
            commands::keys::run(&cli.config, action, format, !cli.live, cli.base_url).await
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if format == Format::Json {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err.to_string(),
                });
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_default()
                );
            } else {
                eprintln!("{} {}", "error:".red().bold(), err);
                // Walk the error chain so the user sees the root cause.
                let mut source = err.source();
                while let Some(s) = source {
                    eprintln!("  {} {}", "caused by:".dimmed(), s);
                    source = s.source();
                }
            }
            ExitCode::FAILURE
        }
    }
}
