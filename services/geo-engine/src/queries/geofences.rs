//! Geofence CRUD plus the synchronous "is this user inside any fence?"
//! check.
//!
//! Soft delete: DeleteGeofence flips `active = FALSE` rather than
//! removing the row. Two reasons:
//!   1. Audit trail — a developer can see which fences they previously had.
//!   2. The safety-consumer (Phase 6) may have in-flight events
//!      referencing a fence_id; the FK stays valid.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub struct GeofenceRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub label: String,
    pub center_lat: f64,
    pub center_lng: f64,
    pub radius_m: f64,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// The column tuple every geofence query projects, in SELECT order.
/// `create` and `list` share the same projection, so they share one name
/// for it rather than repeating an eight-element tuple type each.
type GeofenceTuple = (Uuid, Uuid, String, f64, f64, f64, bool, DateTime<Utc>);

impl From<GeofenceTuple> for GeofenceRow {
    fn from(t: GeofenceTuple) -> Self {
        Self {
            id: t.0,
            user_id: t.1,
            label: t.2,
            center_lat: t.3,
            center_lng: t.4,
            radius_m: t.5,
            active: t.6,
            created_at: t.7,
        }
    }
}

pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    label: &str,
    center_lat: f64,
    center_lng: f64,
    radius_m: f64,
) -> Result<GeofenceRow, sqlx::Error> {
    // NULLIF turns an empty string into SQL NULL so the column distinction
    // between "unlabeled" and "labeled with empty string" stays clean.
    let row: GeofenceTuple = sqlx::query_as(
        r#"
        INSERT INTO geo.geofences (user_id, label, center, radius_m)
        VALUES (
            $1,
            NULLIF($2, ''),
            ST_SetSRID(ST_MakePoint($3, $4), 4326),
            $5
        )
        RETURNING id, user_id, COALESCE(label, '') AS label,
                  ST_Y(center::geometry) AS center_lat,
                  ST_X(center::geometry) AS center_lng,
                  radius_m, active, created_at
        "#,
    )
    .bind(user_id)
    .bind(label)
    .bind(center_lng)
    .bind(center_lat)
    .bind(radius_m)
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

pub async fn list(
    pool: &PgPool,
    user_id: Uuid,
    active_only: bool,
) -> Result<Vec<GeofenceRow>, sqlx::Error> {
    let rows: Vec<GeofenceTuple> = sqlx::query_as(
        r#"
        SELECT id, user_id, COALESCE(label, '') AS label,
               ST_Y(center::geometry) AS center_lat,
               ST_X(center::geometry) AS center_lng,
               radius_m, active, created_at
        FROM geo.geofences
        WHERE user_id = $1
          AND ($2 = FALSE OR active = TRUE)
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .bind(active_only)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Soft-delete. Returns true if a row was found and deactivated.
pub async fn deactivate(pool: &PgPool, geofence_id: Uuid) -> Result<bool, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"
        UPDATE geo.geofences
        SET active = FALSE
        WHERE id = $1
        RETURNING id
        "#,
    )
    .bind(geofence_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Return the IDs of all active geofences the user is currently inside.
///
/// `radius_m` is meters, so both operands cast to `geography` — see the
/// note on `queries::locations::nearby`. Comparing the raw `GEOMETRY(Point,
/// 4326)` values would treat `radius_m` as degrees and report every fence
/// as containing every user.
pub async fn check_membership(
    pool: &PgPool,
    user_id: Uuid,
    lat: f64,
    lng: f64,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM geo.geofences
        WHERE user_id = $1
          AND active = TRUE
          AND ST_DWithin(
              center::geography,
              ST_SetSRID(ST_MakePoint($2, $3), 4326)::geography,
              radius_m
          )
        "#,
    )
    .bind(user_id)
    .bind(lng)
    .bind(lat)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
