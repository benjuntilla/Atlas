//! Route scoring against the safety graph.
//!
//! Each candidate route is a list of (lat, lng) points. We build a
//! LineString in SQL via jsonb_array_elements and average the elo_score
//! of every safety rating within 30m of any segment of that line. A route
//! with no nearby ratings gets the neutral ELO score of 1500.0.

use serde_json::json;
use sqlx::PgPool;

/// Score one route candidate. The route is passed as `[{lat, lng}, ...]`
/// and built into a LineString in SQL. Returns the average ELO; defaults
/// to 1500.0 when no safety ratings are within 30m of the line.
pub async fn score_route(pool: &PgPool, points: &[(f64, f64)]) -> Result<f64, sqlx::Error> {
    // Build the JSON payload up front so the SQL stays simple. ::float8
    // casts are critical — without them ST_MakePoint fails because
    // jsonb's ->> returns text.
    let json_points: Vec<serde_json::Value> = points
        .iter()
        .map(|(lat, lng)| json!({ "lat": lat, "lng": lng }))
        .collect();
    let payload = serde_json::Value::Array(json_points);

    let row: (Option<f64>,) = sqlx::query_as(
        r#"
        WITH route_segment AS (
            SELECT ST_MakeLine(
                ARRAY(
                    SELECT ST_SetSRID(
                        ST_MakePoint(
                            (p->>'lng')::float8,
                            (p->>'lat')::float8
                        ),
                        4326
                    )::geometry
                    FROM jsonb_array_elements($1::jsonb) p
                )
            ) AS geom
        )
        SELECT AVG(sr.elo_score)::float8 AS score
        FROM route_segment rs
        LEFT JOIN geo.safety_ratings sr
            ON ST_DWithin(sr.segment_geom, rs.geom, 30)
        "#,
    )
    .bind(payload)
    .fetch_one(pool)
    .await?;

    // COALESCE in SQL would also work; doing it in Rust keeps the SQL
    // self-documenting about the join behavior.
    Ok(row.0.unwrap_or(1500.0))
}
