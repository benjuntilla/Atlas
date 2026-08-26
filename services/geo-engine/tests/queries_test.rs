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

/// Create an account and a project to hang test rows off. Every geo row
/// belongs to a project, and the tests need real ones rather than the
/// bootstrap default so that scoping can be asserted rather than assumed.
async fn seed_project(pool: &PgPool) -> Uuid {
    let account = Uuid::new_v4();
    let project = Uuid::new_v4();
    sqlx::query("INSERT INTO control.accounts (id, email) VALUES ($1, $2)")
        .bind(account)
        .bind(format!("{account}@geo.test"))
        .execute(pool)
        .await
        .expect("seed control.accounts");
    sqlx::query(
        "INSERT INTO control.projects (id, account_id, name, region, environment, endpoint)
         VALUES ($1, $2, $3, 'local', 'development', '')",
    )
    .bind(project)
    .bind(account)
    .bind(format!("geo-{project}"))
    .execute(pool)
    .await
    .expect("seed control.projects");
    project
}

/// Insert an auth.users row so the FK on geo.* tables resolves.
/// Uses the columns from migration 0010_auth.sql plus the project_id
/// added by 0050.
async fn seed_user(pool: &PgPool, project_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO auth.users (id, project_id, email, password_hash)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(id)
    .bind(project_id)
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
    let project = seed_project(&pool).await;
    let owner = seed_user(&pool, project).await;
    let other = seed_user(&pool, project).await;

    // Insert a ping for `other` at Phoenix coordinates.
    let (lat, lng) = (33.4484, -112.0740);
    atlas_geo_engine::queries::locations::insert_location(
        &pool,
        project,
        other,
        lat,
        lng,
        Utc::now(),
    )
    .await
    .unwrap();

    // owner queries nearby within 1km of the same point. Should find `other`.
    let rows =
        atlas_geo_engine::queries::locations::nearby(&pool, project, lat, lng, 1000.0, owner, 50)
            .await
            .unwrap();

    assert!(
        rows.iter().any(|r| r.user_id == other),
        "other not in nearby results"
    );
}

/// A coordinate no other test writes to, derived from a random UUID.
///
/// Tests here share one never-truncated database, which is fine for
/// assertions scoped to a user_id but not for one that asserts *absence*
/// of rows at a location. This keeps such a test independent of what
/// every other test has already inserted.
fn unique_coordinate(seed: Uuid) -> (f64, f64) {
    let b = seed.as_bytes();
    let lat = -60.0 + (u16::from_be_bytes([b[0], b[1]]) as f64 / 65_535.0) * 120.0;
    let lng = -180.0 + (u16::from_be_bytes([b[2], b[3]]) as f64 / 65_535.0) * 360.0;
    (lat, lng)
}

#[tokio::test]
#[ignore]
async fn nearby_with_zero_radius_returns_empty() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let owner = seed_user(&pool, project).await;
    let other = seed_user(&pool, project).await;

    // A fixed coordinate would make this test order-dependent:
    // `update_then_nearby_returns_the_row` pings Phoenix, and
    // ST_DWithin(p, p, 0) is TRUE for an exactly coincident point, so
    // whichever test ran first decided whether this one passed.
    let (lat, lng) = unique_coordinate(owner);

    // ~44m north. Close enough that any nonzero radius would find it,
    // which is what makes the zero-radius assertion meaningful rather
    // than vacuous.
    atlas_geo_engine::queries::locations::insert_location(
        &pool,
        project,
        other,
        lat + 0.0004,
        lng,
        Utc::now(),
    )
    .await
    .unwrap();

    // No radius defaulting in queries — that's a service-layer concern.
    // Passing radius=0 to the SQL should return nothing.
    let rows =
        atlas_geo_engine::queries::locations::nearby(&pool, project, lat, lng, 0.0, owner, 50)
            .await
            .unwrap();
    assert!(
        rows.is_empty(),
        "radius 0 must exclude a ping 44m away, got {} row(s)",
        rows.len()
    );
}

/// Guards the degrees-vs-meters bug: `radius_m` must be interpreted as
/// meters, not as SRID 4326 degrees. Before the `::geography` casts went
/// in, a 250m search matched a ping 144km away.
#[tokio::test]
#[ignore]
async fn nearby_radius_is_meters_not_degrees() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let owner = seed_user(&pool, project).await;
    let other = seed_user(&pool, project).await;

    let (lat, lng) = unique_coordinate(owner);
    // ~1.3 degrees away, which is ~144km — inside a 250-degree radius but
    // very much outside a 250-metre one.
    atlas_geo_engine::queries::locations::insert_location(
        &pool,
        project,
        other,
        lat + 1.05,
        lng - 0.93,
        Utc::now(),
    )
    .await
    .unwrap();

    let rows =
        atlas_geo_engine::queries::locations::nearby(&pool, project, lat, lng, 250.0, owner, 50)
            .await
            .unwrap();
    assert!(
        !rows.iter().any(|r| r.user_id == other),
        "a ping ~144km away must not match a 250m radius"
    );

    // And the same ping is found once the radius genuinely covers it.
    let wide = atlas_geo_engine::queries::locations::nearby(
        &pool, project, lat, lng, 200_000.0, owner, 50,
    )
    .await
    .unwrap();
    let found = wide
        .iter()
        .find(|r| r.user_id == other)
        .expect("ping must be found within 200km");
    // distance_m must be meters too, not degrees (~1.4 would mean degrees).
    assert!(
        found.distance_m > 100_000.0 && found.distance_m < 200_000.0,
        "distance_m should be ~144000 meters, got {}",
        found.distance_m
    );
}

// --- routes -----------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn score_route_is_neutral_where_nobody_has_voted() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    // A short route in the middle of the ocean — nobody has voted here.
    let route = vec![(0.0, 0.0), (0.001, 0.001)];
    let scored = atlas_geo_engine::queries::safety::score_route_line(&pool, project, &route)
        .await
        .unwrap();
    assert_eq!(scored.score, 1500.0);
    assert_eq!(
        scored.vote_count, 0,
        "the neutral score must come with the evidence that there is none"
    );
}

// --- geofences --------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn create_list_check_delete_geofence_round_trip() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let user = seed_user(&pool, project).await;

    let g = atlas_geo_engine::queries::geofences::create(
        &pool, project, user, "downtown", 33.4484, -112.0740, 250.0,
    )
    .await
    .unwrap();
    assert_eq!(g.user_id, user);
    assert_eq!(g.label, "downtown");
    assert!(g.active);

    // List returns it.
    let listed = atlas_geo_engine::queries::geofences::list(&pool, project, user, true)
        .await
        .unwrap();
    assert!(listed.iter().any(|f| f.id == g.id));

    // Standing at the center: membership check returns this fence.
    let inside = atlas_geo_engine::queries::geofences::check_membership(
        &pool, project, user, 33.4484, -112.0740,
    )
    .await
    .unwrap();
    assert!(inside.contains(&g.id));

    // Standing 100km away: not inside.
    let outside =
        atlas_geo_engine::queries::geofences::check_membership(&pool, project, user, 34.5, -113.0)
            .await
            .unwrap();
    assert!(!outside.contains(&g.id));

    // Soft-delete and re-check: not in active-only list, not inside.
    let deleted = atlas_geo_engine::queries::geofences::deactivate(&pool, project, user, g.id)
        .await
        .unwrap();
    assert!(deleted);

    let listed_active = atlas_geo_engine::queries::geofences::list(&pool, project, user, true)
        .await
        .unwrap();
    assert!(!listed_active.iter().any(|f| f.id == g.id));

    let listed_all = atlas_geo_engine::queries::geofences::list(&pool, project, user, false)
        .await
        .unwrap();
    assert!(listed_all.iter().any(|f| f.id == g.id && !f.active));
}

// --- tenancy ----------------------------------------------------------------
//
// Every test above uses a single project, so all of them would pass just
// as happily against the unscoped queries these replaced. These would not.

/// The leak this scoping exists to close.
///
/// `nearby` searches by POSITION, not by user id, so before the project
/// filter it returned every Atlas user near a point no matter which
/// customer's application asked — a cross-tenant read of people's
/// locations. Both users below sit on the same coordinates.
#[tokio::test]
#[ignore]
async fn nearby_does_not_return_another_projects_users() {
    let pool = pool().await;
    let project_a = seed_project(&pool).await;
    let project_b = seed_project(&pool).await;
    let asker = seed_user(&pool, project_a).await;
    let mine = seed_user(&pool, project_a).await;
    let theirs = seed_user(&pool, project_b).await;

    // A coordinate this test owns, so a shared database cannot make the
    // assertions below pass or fail by accident.
    let (lat, lng) = unique_coords(asker);

    atlas_geo_engine::queries::locations::insert_location(
        &pool,
        project_a,
        mine,
        lat,
        lng,
        Utc::now(),
    )
    .await
    .expect("insert mine");
    atlas_geo_engine::queries::locations::insert_location(
        &pool,
        project_b,
        theirs,
        lat,
        lng,
        Utc::now(),
    )
    .await
    .expect("insert theirs");

    let rows =
        atlas_geo_engine::queries::locations::nearby(&pool, project_a, lat, lng, 1000.0, asker, 50)
            .await
            .expect("nearby");

    let found: Vec<Uuid> = rows.iter().map(|r| r.user_id).collect();
    assert!(
        found.contains(&mine),
        "the asker's own project's user must be returned"
    );
    assert!(
        !found.contains(&theirs),
        "a user from another project must not appear in nearby results"
    );
}

/// Geofences are read by user id, so a cross-tenant read would need a
/// leaked user UUID — but the scoping is asserted anyway, because
/// "unreachable through the current API" is a weaker guarantee than
/// "returns nothing".
#[tokio::test]
#[ignore]
async fn geofences_are_invisible_to_another_project() {
    let pool = pool().await;
    let project_a = seed_project(&pool).await;
    let project_b = seed_project(&pool).await;
    let user = seed_user(&pool, project_a).await;

    let g = atlas_geo_engine::queries::geofences::create(
        &pool, project_a, user, "home", 33.4484, -112.0740, 250.0,
    )
    .await
    .expect("create");

    let mine = atlas_geo_engine::queries::geofences::list(&pool, project_a, user, true)
        .await
        .expect("list mine");
    assert!(mine.iter().any(|f| f.id == g.id));

    let theirs = atlas_geo_engine::queries::geofences::list(&pool, project_b, user, true)
        .await
        .expect("list theirs");
    assert!(
        !theirs.iter().any(|f| f.id == g.id),
        "another project must not see this geofence"
    );

    let inside = atlas_geo_engine::queries::geofences::check_membership(
        &pool, project_b, user, 33.4484, -112.0740,
    )
    .await
    .expect("check theirs");
    assert!(
        inside.is_empty(),
        "another project's membership check must not match this fence"
    );
}

/// The IDOR fix. A geofence id is a UUID, but ids leak — create returns
/// one, list echoes them, alerts carry them. Holding one must not be
/// enough to delete a fence you do not own.
#[tokio::test]
#[ignore]
async fn a_geofence_cannot_be_deleted_by_another_user_or_project() {
    let pool = pool().await;
    let project_a = seed_project(&pool).await;
    let project_b = seed_project(&pool).await;
    let owner = seed_user(&pool, project_a).await;
    let neighbour = seed_user(&pool, project_a).await;
    let stranger = seed_user(&pool, project_b).await;

    let g = atlas_geo_engine::queries::geofences::create(
        &pool, project_a, owner, "home", 33.4484, -112.0740, 250.0,
    )
    .await
    .expect("create");

    // Another user in the SAME project, holding the real id.
    assert!(
        !atlas_geo_engine::queries::geofences::deactivate(&pool, project_a, neighbour, g.id)
            .await
            .expect("deactivate as neighbour"),
        "a different user must not be able to delete this fence"
    );

    // A user in a DIFFERENT project, holding the real id.
    assert!(
        !atlas_geo_engine::queries::geofences::deactivate(&pool, project_b, stranger, g.id)
            .await
            .expect("deactivate as stranger"),
        "a different project must not be able to delete this fence"
    );

    // And after both attempts it is still active.
    let still = atlas_geo_engine::queries::geofences::list(&pool, project_a, owner, true)
        .await
        .expect("list");
    assert!(
        still.iter().any(|f| f.id == g.id),
        "the fence must survive both attempts"
    );

    // The owner can, and can do it twice.
    assert!(
        atlas_geo_engine::queries::geofences::deactivate(&pool, project_a, owner, g.id)
            .await
            .expect("deactivate as owner")
    );
    assert!(
        atlas_geo_engine::queries::geofences::deactivate(&pool, project_a, owner, g.id)
            .await
            .expect("deactivate again"),
        "deleting your own fence twice stays idempotent"
    );
}

/// Derive a coordinate pair unique to this test run from a UUID, so tests
/// sharing a database cannot see each other's rows. Kept well inside
/// valid lat/lng bounds.
fn unique_coords(seed: Uuid) -> (f64, f64) {
    let bytes = seed.as_bytes();
    let lat = -60.0 + (bytes[0] as f64) * (120.0 / 255.0);
    let lng = -170.0 + (bytes[1] as f64) * (340.0 / 255.0);
    (lat, lng)
}

// --- safety votes -----------------------------------------------------------

/// The whole point of the feature: before this, `safety_score` was
/// `COALESCE(AVG(elo_score), 1500.0)` over a table nothing ever wrote to,
/// so it was 1500.0 for every user on the platform. A vote has to move it.
#[tokio::test]
#[ignore]
async fn votes_move_the_score_away_from_neutral() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let voter = seed_user(&pool, project).await;
    let (lat, lng) = unique_coords(voter);

    let before = atlas_geo_engine::queries::safety::score_at(&pool, project, lat, lng)
        .await
        .expect("score before");
    assert_eq!(before.score, 1500.0, "somewhere unvoted starts neutral");
    assert_eq!(before.vote_count, 0);

    let after = atlas_geo_engine::queries::safety::cast_vote(
        &pool,
        project,
        voter,
        lat,
        lng,
        atlas_geo_engine::queries::safety::Verdict::Unsafe,
    )
    .await
    .expect("cast vote");

    assert!(
        after.score < 1500.0,
        "an unsafe vote must lower the score, got {}",
        after.score
    );
    assert_eq!(after.vote_count, 1);
}

/// Smoothing is the property that makes a score readable: one voter must
/// not be able to drive a place to an extreme, and many agreeing voters
/// must get further than one.
#[tokio::test]
#[ignore]
async fn more_agreeing_votes_move_the_score_further() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let anchor = seed_user(&pool, project).await;
    let (lat, lng) = unique_coords(anchor);

    let mut scores = Vec::new();
    for _ in 0..5 {
        let voter = seed_user(&pool, project).await;
        let s = atlas_geo_engine::queries::safety::cast_vote(
            &pool,
            project,
            voter,
            lat,
            lng,
            atlas_geo_engine::queries::safety::Verdict::Safe,
        )
        .await
        .expect("cast vote");
        scores.push(s.score);
    }

    assert!(
        scores.windows(2).all(|w| w[1] > w[0]),
        "each additional agreeing vote must move the score further: {scores:?}"
    );
    assert!(
        scores[0] < 1700.0,
        "one voter must not swing a place to an extreme, got {}",
        scores[0]
    );
    assert!(
        *scores.last().unwrap() < 2000.0,
        "the score is bounded below 2000 no matter the agreement"
    );
}

/// Re-voting corrects rather than accumulates. Without the DISTINCT ON,
/// one determined user could vote a hundred times and outweigh everybody.
#[tokio::test]
#[ignore]
async fn a_user_voting_twice_counts_once_and_the_latest_wins() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let voter = seed_user(&pool, project).await;
    let (lat, lng) = unique_coords(voter);

    let first = atlas_geo_engine::queries::safety::cast_vote(
        &pool,
        project,
        voter,
        lat,
        lng,
        atlas_geo_engine::queries::safety::Verdict::Safe,
    )
    .await
    .expect("first vote");
    assert!(first.score > 1500.0);
    assert_eq!(first.vote_count, 1);

    // Same user, opposite verdict.
    let second = atlas_geo_engine::queries::safety::cast_vote(
        &pool,
        project,
        voter,
        lat,
        lng,
        atlas_geo_engine::queries::safety::Verdict::Unsafe,
    )
    .await
    .expect("second vote");

    assert_eq!(
        second.vote_count, 1,
        "one user is one voter however often they vote"
    );
    assert!(
        second.score < 1500.0,
        "the latest verdict must win, got {}",
        second.score
    );
}

/// Safety data is per tenant. One customer's users voting a street unsafe
/// must not change what another customer's users are told about it.
#[tokio::test]
#[ignore]
async fn votes_do_not_cross_projects() {
    let pool = pool().await;
    let project_a = seed_project(&pool).await;
    let project_b = seed_project(&pool).await;
    let voter = seed_user(&pool, project_a).await;
    let (lat, lng) = unique_coords(voter);

    for _ in 0..5 {
        let v = seed_user(&pool, project_a).await;
        atlas_geo_engine::queries::safety::cast_vote(
            &pool,
            project_a,
            v,
            lat,
            lng,
            atlas_geo_engine::queries::safety::Verdict::Unsafe,
        )
        .await
        .expect("vote in A");
    }

    let a = atlas_geo_engine::queries::safety::score_at(&pool, project_a, lat, lng)
        .await
        .expect("score in A");
    let b = atlas_geo_engine::queries::safety::score_at(&pool, project_b, lat, lng)
        .await
        .expect("score in B");

    assert!(a.score < 1500.0, "project A sees its own votes");
    assert_eq!(
        b.score, 1500.0,
        "project B must see neutral at the same coordinates"
    );
    assert_eq!(b.vote_count, 0);
}

/// Votes are positional: a verdict about one street says nothing about a
/// street a hundred kilometres away.
#[tokio::test]
#[ignore]
async fn a_vote_does_not_affect_a_distant_place() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let voter = seed_user(&pool, project).await;
    let (lat, lng) = unique_coords(voter);

    atlas_geo_engine::queries::safety::cast_vote(
        &pool,
        project,
        voter,
        lat,
        lng,
        atlas_geo_engine::queries::safety::Verdict::Unsafe,
    )
    .await
    .expect("vote");

    // ~1.1 km north — well outside the 200m scoring radius.
    let far = atlas_geo_engine::queries::safety::score_at(&pool, project, lat + 0.01, lng)
        .await
        .expect("score far away");
    assert_eq!(far.score, 1500.0);
    assert_eq!(far.vote_count, 0);
}

/// `nearby` is where the score is actually consumed, so the wiring gets
/// its own test rather than being assumed from the unit-level ones.
#[tokio::test]
#[ignore]
async fn nearby_reports_the_score_of_each_users_own_position() {
    let pool = pool().await;
    let project = seed_project(&pool).await;
    let asker = seed_user(&pool, project).await;
    let subject = seed_user(&pool, project).await;
    let (lat, lng) = unique_coords(subject);

    atlas_geo_engine::queries::locations::insert_location(
        &pool,
        project,
        subject,
        lat,
        lng,
        Utc::now(),
    )
    .await
    .expect("insert location");

    let before =
        atlas_geo_engine::queries::locations::nearby(&pool, project, lat, lng, 500.0, asker, 50)
            .await
            .expect("nearby before");
    let row = before
        .iter()
        .find(|r| r.user_id == subject)
        .expect("subject is nearby");
    assert_eq!(row.safety_score, 1500.0);
    assert_eq!(row.safety_vote_count, 0);

    for _ in 0..4 {
        let v = seed_user(&pool, project).await;
        atlas_geo_engine::queries::safety::cast_vote(
            &pool,
            project,
            v,
            lat,
            lng,
            atlas_geo_engine::queries::safety::Verdict::Safe,
        )
        .await
        .expect("vote");
    }

    let after =
        atlas_geo_engine::queries::locations::nearby(&pool, project, lat, lng, 500.0, asker, 50)
            .await
            .expect("nearby after");
    let row = after
        .iter()
        .find(|r| r.user_id == subject)
        .expect("subject is still nearby");
    assert!(
        row.safety_score > 1500.0,
        "nearby must reflect the votes, got {}",
        row.safety_score
    );
    assert_eq!(row.safety_vote_count, 4);
}
