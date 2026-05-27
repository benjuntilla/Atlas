//! `atlas validate` — parse and validate `atlas.toml`. No network calls.

use crate::commands::Format;
use crate::config::AtlasConfig;
use anyhow::Result;
use owo_colors::OwoColorize;
use serde_json::json;
use std::path::Path;

pub fn run(config_path: &Path, format: Format) -> Result<()> {
    let cfg = AtlasConfig::load(config_path)?;

    if format == Format::Json {
        let dangling = cfg.dangling_sections();
        let summary = json!({
            "ok": true,
            "project": {
                "name": cfg.project.name,
                "environment": cfg.project.environment.as_str(),
                "region": cfg.project.region,
                "api_key": cfg.masked_api_key(),
            },
            "services": {
                "auth": cfg.services.auth,
                "geo": cfg.services.geo,
                "payments": cfg.services.payments,
                "events": cfg.services.events,
            },
            "warnings": dangling.iter().map(|s| format!("[{}] section is present but services.{} = false", s, s)).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    // Human output. One line per enabled service, then a summary.
    println!(
        "{} project: {} ({})",
        "✓".green(),
        cfg.project.name.bold(),
        cfg.project.environment.as_str()
    );

    if cfg.services.auth {
        if let Some(a) = &cfg.auth {
            println!(
                "{} auth: enabled (geo claims {}, {}s expiry)",
                "✓".green(),
                if a.geo_claims { "on" } else { "off" },
                a.token_expiry_seconds
            );
        } else {
            println!("{} auth: enabled (defaults)", "✓".green());
        }
    }

    if cfg.services.geo {
        if let Some(g) = &cfg.geo {
            println!(
                "{} geo: enabled ({}m default radius, ELO cron `{}`)",
                "✓".green(),
                g.default_radius_m,
                g.elo_recompute_cron
            );
        } else {
            println!("{} geo: enabled (defaults)", "✓".green());
        }
    }

    if cfg.services.payments {
        if let Some(p) = &cfg.payments {
            let mode = match p.settlement_mode {
                crate::config::SettlementMode::Auto => "auto",
                crate::config::SettlementMode::Manual => "manual",
            };
            println!(
                "{} payments: enabled ({}, {} settlement)",
                "✓".green(),
                p.currency,
                mode
            );
        } else {
            println!("{} payments: enabled (defaults)", "✓".green());
        }
    }

    if cfg.services.events {
        if let Some(e) = &cfg.events {
            println!(
                "{} events: enabled ({} topic{}, {}h retention)",
                "✓".green(),
                e.topics.len(),
                if e.topics.len() == 1 { "" } else { "s" },
                e.retention_hours
            );
        }
    }

    let warnings = cfg.dangling_sections();
    for w in &warnings {
        println!(
            "{} [{}] section is present but services.{} = false",
            "!".yellow(),
            w,
            w
        );
    }

    if warnings.is_empty() {
        println!("\n{}", "All good.".green().bold());
    } else {
        println!(
            "\n{}",
            format!("Valid, with {} warning(s).", warnings.len())
                .yellow()
                .bold()
        );
    }
    Ok(())
}
