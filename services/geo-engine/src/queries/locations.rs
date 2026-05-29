//! Location ping inserts and proximity queries.
//!
//! Why these queries use `query()` / `query_as()` (the runtime path) rather
//! than `query!()` / `query_as!()` (compile-time checked against a live DB):
//! the compile-time variants require `DATABASE_URL` to be set at build
//! time, which breaks CI and offline builds. The runtime path keeps the
//! crate buildable without a running Postgres.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Insert a location ping. `expires_at` defaults to `NOW() + 24h` via the
/// column default in migration 0020.
pub async fn insert_location(
    pool: &PgPool,
    user_id: Uuid,
    lat: f64,
    lng: f64,
    recorded_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO geo.locations (user_id, position, recorded_at)
        VALUES (
            $1,
            ST_SetSRID(ST_MakePoint($2, $3), 4326),
            $4
        )
        "#,
    )
    .bind(user_id)
    // PostGIS ST_MakePoint takes (lng, lat) — easy to flip; we keep the
    // bind order matching the SQL parameter order for readability.
    .bind(lng)
    .bind(lat)
    .bind(recorded_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub struct NearbyRow {
    pub user_id: Uuid,
    pub lng: f64,
    pub lat: f64,
    pub distance_m: f64,
    pub safety_score: f64,
}

/// Find users near (lat, lng) within `radius_m` meters, excluding the
/// requester. Joined to safety_ratings within 200m for an aggregate
/// safety score; defaults to 1500.0 (the ELO neutral score) when no
/// nearby ratings exist.
pub async fn nearby(
    pool: &PgPool,
    lat: f64,
    lng: f64,
    radius_m: f64,
    requester_user_id: Uuid,
    limit: i64,
) -> Result<Vec<NearbyRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, f64, f64, f64, f64)>(
        r#"
        SELECT
            l.user_id,
            ST_X(l.position::geometry) AS lng,
            ST_Y(l.position::geometry) AS lat,
            ST_Distance(
                l.position::geometry,
                ST_SetSRID(ST_MakePoint($1, $2), 4326)::geometry
            ) AS distance_m,
            COALESCE(AVG(sr.elo_score), 1500.0) AS safety_score
        FROM geo.locations l
        LEFT JOIN geo.safety_ratings sr
            ON ST_DWithin(sr.segment_geom, l.position::geometry, 200)
        WHERE
            ST_DWithin(
                l.position::geometry,
                ST_SetSRID(ST_MakePoint($1, $2), 4326)::geometry,
                $3
            )
            AND l.expires_at > NOW()
            AND l.user_id != $4
        GROUP BY l.user_id, l.position
        ORDER BY distance_m ASC
        LIMIT $5
        "#,
    )
    .bind(lng)
    .bind(lat)
    .bind(radius_m)
    .bind(requester_user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(user_id, lng, lat, distance_m, safety_score)| NearbyRow {
            user_id,
            lng,
            lat,
            distance_m,
            safety_score,
        })
        .collect())
}
