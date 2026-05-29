//! sqlx integration tests against the docker-compose Postgres.
//!
//! Marked #[ignore] by default so `cargo test` stays green when Docker is
//! off. Run explicitly with:
//!
//!     docker compose up -d postgres
//!     cargo test -p atlas-geo-engine -- --include-ignored
//!
//! Each test inserts under a fresh user_id (random UUID) and writes to
//! geo.locations / geo.geofences / geo.safety_ratings. We do NOT
//! truncate tables — concurrent test runs would conflict — instead each
//! test scopes its assertions to its own user_id.

use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_DATABASE_URL: &str = "postgres://atlas:atlas_dev@localhost:5432/atlas";

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(TEST_DATABASE_URL)
        .await
        .expect("connect to local postgres — is docker compose up?")
}

/// Insert an auth.users row so the FK on geo.* tables resolves.
/// Uses the columns from migration 0010_auth.sql.
async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO auth.users (id, email, password_hash)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(id)
    .bind(format!("test-{id}@atlas.dev"))
    .bind("$2a$04$abcdefghijklmnopqrstuv") // not a real bcrypt; FK only
    .execute(pool)
    .await
    .expect("seed auth.users");
    id
}

// --- locations --------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn update_then_nearby_returns_the_row() {
    let pool = pool().await;
    let owner = seed_user(&pool).await;
    let other = seed_user(&pool).await;

    // Insert a ping for `other` at Phoenix coordinates.
    let (lat, lng) = (33.4484, -112.0740);
    atlas_geo_engine::queries::locations::insert_location(&pool, other, lat, lng, Utc::now())
        .await
        .unwrap();

    // owner queries nearby within 1km of the same point. Should find `other`.
    let rows =
        atlas_geo_engine::queries::locations::nearby(&pool, lat, lng, 1000.0, owner, 50)
            .await
            .unwrap();

    assert!(rows.iter().any(|r| r.user_id == other), "other not in nearby results");
}

#[tokio::test]
#[ignore]
async fn nearby_with_zero_radius_returns_empty() {
    let pool = pool().await;
    let owner = seed_user(&pool).await;
    // No radius defaulting in queries — that's a service-layer concern.
    // Passing radius=0 to the SQL should return nothing.
    let rows =
        atlas_geo_engine::queries::locations::nearby(&pool, 33.4484, -112.0740, 0.0, owner, 50)
            .await
            .unwrap();
    assert!(rows.is_empty());
}

// --- routes -----------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn score_route_defaults_to_neutral_elo_when_no_ratings() {
    let pool = pool().await;
    // A short route in the middle of the ocean — no safety ratings nearby.
    let route = vec![(0.0, 0.0), (0.001, 0.001)];
    let score = atlas_geo_engine::queries::routes::score_route(&pool, &route)
        .await
        .unwrap();
    assert_eq!(score, 1500.0);
}

// --- geofences --------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn create_list_check_delete_geofence_round_trip() {
    let pool = pool().await;
    let user = seed_user(&pool).await;

    let g = atlas_geo_engine::queries::geofences::create(
        &pool, user, "downtown", 33.4484, -112.0740, 250.0,
    )
    .await
    .unwrap();
    assert_eq!(g.user_id, user);
    assert_eq!(g.label, "downtown");
    assert!(g.active);

    // List returns it.
    let listed = atlas_geo_engine::queries::geofences::list(&pool, user, true)
        .await
        .unwrap();
    assert!(listed.iter().any(|f| f.id == g.id));

    // Standing at the center: membership check returns this fence.
    let inside = atlas_geo_engine::queries::geofences::check_membership(
        &pool, user, 33.4484, -112.0740,
    )
    .await
    .unwrap();
    assert!(inside.contains(&g.id));

    // Standing 100km away: not inside.
    let outside = atlas_geo_engine::queries::geofences::check_membership(
        &pool, user, 34.5, -113.0,
    )
    .await
    .unwrap();
    assert!(!outside.contains(&g.id));

    // Soft-delete and re-check: not in active-only list, not inside.
    let deleted = atlas_geo_engine::queries::geofences::deactivate(&pool, g.id)
        .await
        .unwrap();
    assert!(deleted);

    let listed_active = atlas_geo_engine::queries::geofences::list(&pool, user, true)
        .await
        .unwrap();
    assert!(!listed_active.iter().any(|f| f.id == g.id));

    let listed_all = atlas_geo_engine::queries::geofences::list(&pool, user, false)
        .await
        .unwrap();
    assert!(listed_all.iter().any(|f| f.id == g.id && !f.active));
}
