//! `atlas status` — show current service health for the configured project.

use crate::api::ApiClient;
use crate::commands::Format;
use crate::config::AtlasConfig;
use anyhow::Result;
use owo_colors::OwoColorize;
use std::path::Path;

pub async fn run(
    config_path: &Path,
    format: Format,
    mock: bool,
    base_url: Option<String>,
) -> Result<()> {
    let cfg = AtlasConfig::load(config_path)?;
    let client = ApiClient::from_config(&cfg, mock, base_url);
    let resp = client.status(&cfg.project.name).await?;

    if format == Format::Json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    println!("{} ({})", resp.project_name.bold(), cfg.project.region);
    println!();
    println!(
        "{:<10} {:<8} {:>10} {:>14} {:>12}",
        "SERVICE".dimmed(),
        "STATUS".dimmed(),
        "P95 (ms)".dimmed(),
        "REQ / 24H".dimmed(),
        "ERR RATE".dimmed()
    );
    for s in &resp.services {
        let status_cell = if s.healthy {
            "healthy".green().to_string()
        } else {
            "DOWN".red().to_string()
        };
        println!(
            "{:<10} {:<8} {:>10} {:>14} {:>11.3}%",
            s.name,
            status_cell,
            s.p95_latency_ms,
            s.requests_24h,
            s.error_rate * 100.0
        );
    }
    Ok(())
}
