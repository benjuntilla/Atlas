//! Atlas safety-consumer entrypoint.
//!
//! Reads `atlas.location.updates`, and writes geofence crossings to
//! `atlas.safety.alerts`.

use anyhow::Context;
use atlas_safety_consumer::{config, consumer, metrics, producer};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = config::Config::from_env();
    info!(
        input = %cfg.topic_location_updates,
        output = %cfg.topic_safety_alerts,
        group = %cfg.consumer_group,
        "starting atlas-safety-consumer"
    );

    let metrics_handle = metrics::install_recorder();
    metrics::serve(cfg.metrics_addr, metrics_handle).await?;

    let pool = PgPoolOptions::new()
        .max_connections(cfg.database_pool_size)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&cfg.database_url)
        .await
        .context("postgres connection")?;
    info!("postgres pool ready");

    let publisher =
        producer::KafkaAlertPublisher::new(&cfg.kafka_brokers, cfg.topic_safety_alerts.clone())
            .context("kafka producer")?;

    let kafka = consumer::build(&cfg.kafka_brokers, &cfg.consumer_group)
        .context("building kafka consumer")?;

    consumer::run(
        kafka,
        pool,
        publisher,
        &cfg.topic_location_updates,
        shutdown_signal(),
    )
    .await;

    info!("atlas-safety-consumer exited cleanly");
    Ok(())
}

/// Wait for SIGTERM (k8s rolling deploy) or SIGINT (local ctrl-c).
async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
