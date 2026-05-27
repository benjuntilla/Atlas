//! `atlas keys [list|create|revoke]` — manage API keys for a project.

use crate::api::{ApiClient, KeyExpiry};
use crate::commands::Format;
use crate::config::AtlasConfig;
use anyhow::Result;
use clap::Subcommand;
use owo_colors::OwoColorize;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub enum KeysCommand {
    /// List all API keys for the current project.
    List,
    /// Create a new API key.
    Create {
        /// Friendly name for the key (e.g. "ci", "primary").
        name: String,
        /// Key expiry. One of: never | 30d | 90d | 1y.
        #[arg(long, default_value = "never")]
        expiry: String,
    },
    /// Revoke an existing key by its prefix.
    Revoke {
        /// Key prefix as shown by `atlas keys list` (e.g. "atl_live_abcd").
        prefix: String,
    },
}

pub async fn run(
    config_path: &Path,
    cmd: KeysCommand,
    format: Format,
    mock: bool,
    base_url: Option<String>,
) -> Result<()> {
    let cfg = AtlasConfig::load(config_path)?;
    let client = ApiClient::from_config(&cfg, mock, base_url);

    match cmd {
        KeysCommand::List => list(&client, &cfg, format).await,
        KeysCommand::Create { name, expiry } => {
            let exp: KeyExpiry = expiry.parse()?;
            create(&client, &cfg, &name, exp, format).await
        }
        KeysCommand::Revoke { prefix } => revoke(&client, &cfg, &prefix, format).await,
    }
}

async fn list(client: &ApiClient, cfg: &AtlasConfig, format: Format) -> Result<()> {
    let keys = client.list_keys(&cfg.project.name).await?;
    if format == Format::Json {
        println!("{}", serde_json::to_string_pretty(&keys)?);
        return Ok(());
    }
    println!(
        "{:<14} {:<22} {:<22} {:<22} {}",
        "NAME".dimmed(),
        "PREFIX".dimmed(),
        "CREATED".dimmed(),
        "LAST USED".dimmed(),
        "STATUS".dimmed()
    );
    for k in &keys {
        let status_cell = match k.status.as_str() {
            "active" => "active".green().to_string(),
            "revoked" => "revoked".red().to_string(),
            other => other.to_string(),
        };
        println!(
            "{:<14} {:<22} {:<22} {:<22} {}",
            k.name,
            k.prefix,
            k.created_at,
            k.last_used_at.as_deref().unwrap_or("—"),
            status_cell
        );
    }
    Ok(())
}

async fn create(
    client: &ApiClient,
    cfg: &AtlasConfig,
    name: &str,
    expiry: KeyExpiry,
    format: Format,
) -> Result<()> {
    let key = client.create_key(&cfg.project.name, name, expiry).await?;
    if format == Format::Json {
        println!("{}", serde_json::to_string_pretty(&key)?);
        return Ok(());
    }
    println!(
        "{} created key '{}' ({})",
        "✓".green(),
        key.name.bold(),
        key.prefix.cyan()
    );
    println!(
        "{}",
        "Save the full key now — it will not be shown again.".yellow()
    );
    Ok(())
}

async fn revoke(
    client: &ApiClient,
    cfg: &AtlasConfig,
    prefix: &str,
    format: Format,
) -> Result<()> {
    client.revoke_key(&cfg.project.name, prefix).await?;
    if format == Format::Json {
        println!("{}", serde_json::json!({"ok": true, "revoked": prefix}));
        return Ok(());
    }
    println!("{} revoked key {}", "✓".green(), prefix.cyan());
    Ok(())
}
