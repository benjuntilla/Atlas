//! End-to-end transition tests against the docker-compose Postgres.
//!
//! Marked #[ignore] by default, same convention as geo-engine's tests, so
//! `cargo test` stays green without Docker. Run explicitly with:
//!
//!     docker compose up -d postgres
//!     cargo test -p atlas-safety-consumer -- --include-ignored
//!
//! These cover what the pure `diff` unit tests cannot: that the geography
//! maths picks the right fences, that membership actually persists
//! between pings, and that a failed publish rolls the membership back.

use atlas_safety_consumer::alerts;
use atlas_safety_consumer::pb::events::safety_alert_event::AlertType;
use atlas_safety_consumer::producer::RecordingAlertPublisher;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_DATABASE_URL: &str = "postgres://atlas:atlas_dev@localhost:5432/atlas";
const NOW: i64 = 1_700_000_000;

/// Phoenix, and a point ~40km away that no 250m fence can contain.
const CENTER: (f64, f64) = (33.4484, -112.0740);
const FAR_AWAY: (f64, f64) = (33.8000, -112.0740);

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(TEST_DATABASE_URL)
        .await
        .expect("connect to local postgres — is docker compose up?")
}

/// A real project to hang test rows off. Geofences and users both belong
/// to one, and the tests need genuine ids rather than the bootstrap
/// default so scoping can be asserted rather than assumed.
async fn seed_project(pool: &PgPool) -> Uuid {
    let account = Uuid::new_v4();
    let project = Uuid::new_v4();
    sqlx::query("INSERT INTO control.accounts (id, email) VALUES ($1, $2)")
        .bind(account)
        .bind(format!("{account}@safety.test"))
        .execute(pool)
        .await
        .expect("seed control.accounts");
    sqlx::query(
        "INSERT INTO control.projects (id, account_id, name, region, environment, endpoint)
         VALUES ($1, $2, $3, 'local', 'development', '')",
    )
    .bind(project)
    .bind(account)
    .bind(format!("safety-{project}"))
    .execute(pool)
    .await
    .expect("seed control.projects");
    project
}

async fn seed_user(pool: &PgPool, project_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO auth.users (id, project_id, email, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(project_id)
    .bind(format!("safety-{id}@atlas.dev"))
    .bind("$2a$04$abcdefghijklmnopqrstuv")
    .execute(pool)
    .await
    .expect("seed auth.users");
    id
}

async fn seed_fence(
    pool: &PgPool,
    project_id: Uuid,
    user_id: Uuid,
    lat: f64,
    lng: f64,
    radius_m: f64,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO geo.geofences (project_id, user_id, label, center, radius_m)
        VALUES ($5, $1, 'test', ST_SetSRID(ST_MakePoint($2, $3), 4326), $4)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(lng)
    .bind(lat)
    .bind(radius_m)
    .bind(project_id)
    .fetch_one(pool)
    .await
    .expect("seed geofence");
    row.0
}

#[tokio::test]
#[ignore]
async fn entering_then_leaving_emits_one_alert_each() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let user = seed_user(&pool, project).await;
    let fence = seed_fence(&pool, project, user, CENTER.0, CENTER.1, 250.0).await;
    let publisher = RecordingAlertPublisher::default();

    // Arrive at the centre.
    let t = alerts::apply_position(&pool, &publisher, project, user, CENTER.0, CENTER.1, NOW)
        .await
        .expect("apply arrival");
    assert_eq!(t.entered, vec![fence]);
    assert!(t.exited.is_empty());

    // Ping again without moving: no second alert.
    let t = alerts::apply_position(
        &pool,
        &publisher,
        project,
        user,
        CENTER.0,
        CENTER.1,
        NOW + 10,
    )
    .await
    .expect("apply stationary ping");
    assert!(
        t.is_empty(),
        "a stationary ping must not re-alert, got {t:?}"
    );

    // Leave.
    let t = alerts::apply_position(
        &pool,
        &publisher,
        project,
        user,
        FAR_AWAY.0,
        FAR_AWAY.1,
        NOW + 20,
    )
    .await
    .expect("apply departure");
    assert_eq!(t.exited, vec![fence]);
    assert!(t.entered.is_empty());

    let published = publisher.published();
    assert_eq!(published.len(), 2, "expected exactly ENTERED then EXITED");
    assert_eq!(published[0].alert_type, AlertType::GeofenceEntered as i32);
    assert_eq!(published[0].geofence_id, fence.to_string());
    assert_eq!(published[0].triggered_at, NOW);
    assert_eq!(published[1].alert_type, AlertType::GeofenceExited as i32);
    assert_eq!(published[1].triggered_at, NOW + 20);
}

/// The radius is meters. A 250m fence must not contain a point 40km away
/// — the degrees-vs-meters bug would make it.
#[tokio::test]
#[ignore]
async fn fence_radius_is_meters() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let user = seed_user(&pool, project).await;
    seed_fence(&pool, project, user, CENTER.0, CENTER.1, 250.0).await;

    let inside = alerts::fences_containing(&pool, project, user, CENTER.0, CENTER.1)
        .await
        .expect("query inside");
    assert_eq!(inside.len(), 1);

    let outside = alerts::fences_containing(&pool, project, user, FAR_AWAY.0, FAR_AWAY.1)
        .await
        .expect("query outside");
    assert!(
        outside.is_empty(),
        "a 250m fence must not contain a point 40km away"
    );
}

/// Fences belong to a user. One user's crossing must not consult, or
/// alert on, another user's fences.
#[tokio::test]
#[ignore]
async fn fences_are_scoped_to_their_owner() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let owner = seed_user(&pool, project).await;
    let stranger = seed_user(&pool, project).await;
    seed_fence(&pool, project, owner, CENTER.0, CENTER.1, 250.0).await;

    let publisher = RecordingAlertPublisher::default();
    let t = alerts::apply_position(
        &pool, &publisher, project, stranger, CENTER.0, CENTER.1, NOW,
    )
    .await
    .expect("apply");

    assert!(
        t.is_empty(),
        "standing inside someone else's fence is not a crossing"
    );
    assert!(publisher.published().is_empty());
}

/// A soft-deleted fence stops matching, so the next ping reports an exit
/// rather than leaving a membership row stranded forever.
#[tokio::test]
#[ignore]
async fn deactivating_a_fence_exits_its_members() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let user = seed_user(&pool, project).await;
    let fence = seed_fence(&pool, project, user, CENTER.0, CENTER.1, 250.0).await;
    let publisher = RecordingAlertPublisher::default();

    alerts::apply_position(&pool, &publisher, project, user, CENTER.0, CENTER.1, NOW)
        .await
        .expect("enter");

    sqlx::query("UPDATE geo.geofences SET active = FALSE WHERE id = $1")
        .bind(fence)
        .execute(&pool)
        .await
        .expect("deactivate");

    let t = alerts::apply_position(
        &pool,
        &publisher,
        project,
        user,
        CENTER.0,
        CENTER.1,
        NOW + 10,
    )
    .await
    .expect("ping after deactivation");
    assert_eq!(t.exited, vec![fence]);
}

/// The ordering guarantee: if the publish fails, the membership change
/// must roll back, so the retry re-emits the alert instead of swallowing
/// it.
#[tokio::test]
#[ignore]
async fn failed_publish_rolls_back_membership() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let user = seed_user(&pool, project).await;
    let fence = seed_fence(&pool, project, user, CENTER.0, CENTER.1, 250.0).await;

    let failing = RecordingAlertPublisher::failing();
    let result =
        alerts::apply_position(&pool, &failing, project, user, CENTER.0, CENTER.1, NOW).await;
    assert!(result.is_err(), "publish failure must surface as an error");

    let recorded = alerts::recorded_memberships(&pool, user)
        .await
        .expect("read memberships");
    assert!(
        recorded.is_empty(),
        "membership must not persist when the alert was never published"
    );

    // The retry succeeds and emits the alert that was nearly lost.
    let ok = RecordingAlertPublisher::default();
    let t = alerts::apply_position(&pool, &ok, project, user, CENTER.0, CENTER.1, NOW)
        .await
        .expect("retry");
    assert_eq!(t.entered, vec![fence]);
    assert_eq!(ok.published().len(), 1);
}
