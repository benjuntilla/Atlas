//! The retention sweep for `geo.locations`.
//!
//! Migration 0020 gives every row a 24h `expires_at`, and `nearby`
//! already filters expired rows out of results. This deletes them for
//! real, which is what keeps the GIST index and the table from growing
//! without bound.

use sqlx::PgPool;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Delete one batch of expired rows. Returns how many went.
///
/// The `ctid IN (SELECT ... LIMIT n)` shape is deliberate. A bare
/// `DELETE ... WHERE expires_at < NOW()` would take row locks on every
/// expired row in one transaction — after an outage backlog that can be
/// millions of rows, holding locks and bloating WAL while the ingest path
/// waits. Batching by physical row id bounds each transaction instead.
pub async fn sweep_once(pool: &PgPool, batch_size: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM geo.locations
        WHERE ctid IN (
            SELECT ctid
            FROM geo.locations
            WHERE expires_at < NOW()
            LIMIT $1
        )
        "#,
    )
    .bind(batch_size)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Sweep until nothing expired remains, then return the total deleted.
///
/// Capped at `max_batches` so a pathological backlog cannot monopolise
/// the task forever — whatever is left is picked up on the next tick.
pub async fn sweep_until_clear(
    pool: &PgPool,
    batch_size: i64,
    max_batches: u32,
) -> Result<u64, sqlx::Error> {
    let mut total = 0;
    for _ in 0..max_batches {
        let deleted = sweep_once(pool, batch_size).await?;
        total += deleted;
        // A short batch means the backlog is drained.
        if (deleted as i64) < batch_size {
            break;
        }
    }
    Ok(total)
}

/// Run the sweep on a timer until cancelled.
pub async fn run(pool: PgPool, interval: Duration, batch_size: i64) {
    const MAX_BATCHES_PER_TICK: u32 = 100;
    let mut ticker = tokio::time::interval(interval);
    // The first tick fires immediately; that is wanted, so a restart after
    // a long outage starts draining rather than waiting out a full period.
    loop {
        ticker.tick().await;
        match sweep_until_clear(&pool, batch_size, MAX_BATCHES_PER_TICK).await {
            Ok(0) => debug!("retention sweep: nothing expired"),
            Ok(n) => {
                metrics::counter!("atlas_location_rows_reaped_total").increment(n);
                info!(deleted = n, "retention sweep complete");
            }
            Err(e) => {
                // A failed sweep must not kill the task — the next tick
                // retries, and rows stay queryable-but-filtered meanwhile.
                metrics::counter!("atlas_location_reap_errors_total").increment(1);
                warn!(error = %e, "retention sweep failed; will retry next tick");
            }
        }
    }
}
