//! Project-key resolution against a real `control.api_keys` table.
//!
//! Marked `#[ignore]` so `cargo test` stays green without a database. Run
//! with:
//!
//!     docker compose up -d postgres
//!     cargo run -p atlas-migrator -- run
//!     cargo test -p atlas-gateway -- --include-ignored
//!
//! `router_test.rs` covers what the gateway decides before it talks to
//! anything and injects a tenant to do so. This file covers the other
//! half: given a key and a database, does the right project come back —
//! and, more to the point, do revoked, expired, and account-scoped keys
//! come back as nothing.
//!
//! Every test creates its own account, project and keys under random
//! UUIDs and deletes the account afterwards, which cascades the rest away.

use atlas_gateway::tenant::{resolve, ProjectCache};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("ATLAS_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://atlas:atlas_dev@localhost:5432/atlas".to_string())
}

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url())
        .await
        .expect("connect to the test database — is postgres up and migrated?")
}

struct Fixture {
    account_id: Uuid,
    project_id: Uuid,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Self {
        let account_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        sqlx::query("INSERT INTO control.accounts (id, email) VALUES ($1, $2)")
            .bind(account_id)
            .bind(format!("{account_id}@gateway.test"))
            .execute(pool)
            .await
            .expect("create account");
        sqlx::query(
            "INSERT INTO control.projects (id, account_id, name, region, environment, endpoint)
             VALUES ($1, $2, $3, 'local', 'production', '')",
        )
        .bind(project_id)
        .bind(account_id)
        .bind(format!("p-{project_id}"))
        .execute(pool)
        .await
        .expect("create project");
        Self {
            account_id,
            project_id,
        }
    }

    /// Mint a key exactly as the control plane does, returning the
    /// plaintext the caller would present.
    async fn key(&self, pool: &PgPool, opts: KeyOpts) -> String {
        let generated = atlas_keys::generate("production");
        sqlx::query(
            "INSERT INTO control.api_keys
                 (account_id, project_id, name, key_prefix, key_hash, status, expires_at)
             VALUES ($1, $2, 'test', $3, $4, $5, $6)",
        )
        .bind(self.account_id)
        .bind(if opts.account_scoped {
            None
        } else {
            Some(self.project_id)
        })
        .bind(&generated.prefix)
        .bind(&generated.hash)
        .bind(opts.status)
        .bind(opts.expires_at)
        .execute(pool)
        .await
        .expect("create key");
        generated.plaintext
    }

    async fn cleanup(&self, pool: &PgPool) {
        sqlx::query("DELETE FROM control.accounts WHERE id = $1")
            .bind(self.account_id)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

struct KeyOpts {
    status: &'static str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    account_scoped: bool,
}

impl Default for KeyOpts {
    fn default() -> Self {
        Self {
            status: "active",
            expires_at: None,
            account_scoped: false,
        }
    }
}

fn cache() -> std::sync::Arc<ProjectCache> {
    ProjectCache::new(Duration::from_secs(30))
}

#[tokio::test]
#[ignore]
async fn a_valid_key_resolves_to_its_project() {
    let pool = pool().await;
    let fixture = Fixture::create(&pool).await;
    let key = fixture.key(&pool, KeyOpts::default()).await;

    let tenant = resolve(&pool, &cache(), &key)
        .await
        .expect("a live key must resolve");
    assert_eq!(tenant.project_id, fixture.project_id);
    assert_eq!(tenant.account_id, fixture.account_id);
    assert_eq!(tenant.environment, "production");

    fixture.cleanup(&pool).await;
}

/// `atlas keys revoke` has to actually revoke. The status filter is in
/// SQL, so a revoked key never becomes a tenant value at all.
#[tokio::test]
#[ignore]
async fn a_revoked_key_does_not_resolve() {
    let pool = pool().await;
    let fixture = Fixture::create(&pool).await;
    let key = fixture
        .key(
            &pool,
            KeyOpts {
                status: "revoked",
                ..KeyOpts::default()
            },
        )
        .await;

    assert!(
        resolve(&pool, &cache(), &key).await.is_err(),
        "a revoked key must not resolve"
    );

    fixture.cleanup(&pool).await;
}

#[tokio::test]
#[ignore]
async fn an_expired_key_does_not_resolve() {
    let pool = pool().await;
    let fixture = Fixture::create(&pool).await;
    let key = fixture
        .key(
            &pool,
            KeyOpts {
                expires_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
                ..KeyOpts::default()
            },
        )
        .await;

    assert!(
        resolve(&pool, &cache(), &key).await.is_err(),
        "an expired key must not resolve"
    );

    fixture.cleanup(&pool).await;
}

/// A key with a future expiry is still live — the filter must be a
/// comparison, not merely a null check.
#[tokio::test]
#[ignore]
async fn a_key_expiring_later_still_resolves() {
    let pool = pool().await;
    let fixture = Fixture::create(&pool).await;
    let key = fixture
        .key(
            &pool,
            KeyOpts {
                expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                ..KeyOpts::default()
            },
        )
        .await;

    let tenant = resolve(&pool, &cache(), &key).await.expect("still live");
    assert_eq!(tenant.project_id, fixture.project_id);

    fixture.cleanup(&pool).await;
}

/// Account-scoped keys (`project_id IS NULL`) exist so `atlas deploy` can
/// create a project that does not exist yet. They name no project, so
/// there is no tenant they could resolve to, and accepting one on the data
/// plane would mean guessing which of an account's projects the caller
/// meant.
#[tokio::test]
#[ignore]
async fn an_account_scoped_key_is_not_usable_on_the_data_plane() {
    let pool = pool().await;
    let fixture = Fixture::create(&pool).await;
    let key = fixture
        .key(
            &pool,
            KeyOpts {
                account_scoped: true,
                ..KeyOpts::default()
            },
        )
        .await;

    assert!(
        resolve(&pool, &cache(), &key).await.is_err(),
        "an account-scoped key must not resolve to a project"
    );

    fixture.cleanup(&pool).await;
}

#[tokio::test]
#[ignore]
async fn an_unknown_key_does_not_resolve() {
    let pool = pool().await;
    let key = atlas_keys::generate("production").plaintext;
    assert!(resolve(&pool, &cache(), &key).await.is_err());
}

/// The cache is the reason this is one query per key per TTL rather than
/// one per request. Deleting the row and resolving again proves the second
/// answer came from memory.
#[tokio::test]
#[ignore]
async fn a_resolved_key_is_cached_across_calls() {
    let pool = pool().await;
    let fixture = Fixture::create(&pool).await;
    let key = fixture.key(&pool, KeyOpts::default()).await;
    let warm = cache();

    let first = resolve(&pool, &warm, &key).await.expect("first resolve");
    fixture.cleanup(&pool).await;

    let second = resolve(&pool, &warm, &key)
        .await
        .expect("second resolve must be served from cache");
    assert_eq!(first.project_id, second.project_id);

    // A cold cache now sees the deleted row and rejects, which is what
    // makes the assertion above meaningful rather than accidental.
    assert!(
        resolve(&pool, &cache(), &key).await.is_err(),
        "a cold cache must reject a key whose project is gone"
    );
}
