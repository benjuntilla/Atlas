//! Tenant-isolation guarantees enforced by the schema itself.
//!
//! Marked `#[ignore]` so `cargo test` stays green without a database. Run
//! with:
//!
//!     docker compose up -d postgres
//!     cargo run -p atlas-migrator -- run
//!     cargo test -p atlas-migrator -- --include-ignored
//!
//! # Why these live in SQL and not in a service
//!
//! Every property below could be enforced in application code, and every
//! one of them would then be one forgotten `WHERE project_id = $1` away
//! from being silently untrue. A constraint in the database fails the
//! write; a missing WHERE clause returns the wrong rows and looks like a
//! success. For the money path in particular that difference is the whole
//! point: a cross-tenant transfer should be impossible to express, not
//! merely unreachable through the current API surface.
//!
//! Each test creates its own account and two projects under random UUIDs
//! and rolls everything back, so the suite is order-independent and leaves
//! no rows behind.

use sqlx::{Connection, PgConnection, Postgres, Transaction};
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("ATLAS_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://atlas:atlas_dev@localhost:5432/atlas".to_string())
}

/// Two projects under one account, created inside the caller's
/// transaction so nothing survives the rollback.
struct Tenants {
    a: Uuid,
    b: Uuid,
}

async fn two_tenants(tx: &mut Transaction<'_, Postgres>) -> Tenants {
    let account = Uuid::new_v4();
    sqlx::query("INSERT INTO control.accounts (id, email) VALUES ($1, $2)")
        .bind(account)
        .bind(format!("{account}@atlas.test"))
        .execute(&mut **tx)
        .await
        .expect("create account");

    let mut ids = Vec::new();
    for label in ["a", "b"] {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO control.projects (id, account_id, name, region, environment, endpoint)
             VALUES ($1, $2, $3, 'local', 'development', '')",
        )
        .bind(id)
        .bind(account)
        .bind(format!("t-{label}-{id}"))
        .execute(&mut **tx)
        .await
        .expect("create project");
        ids.push(id);
    }

    Tenants {
        a: ids[0],
        b: ids[1],
    }
}

async fn user_in(tx: &mut Transaction<'_, Postgres>, project: Uuid, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO auth.users (id, project_id, email, password_hash)
         VALUES ($1, $2, $3, 'not-a-real-hash')",
    )
    .bind(id)
    .bind(project)
    .bind(email)
    .execute(&mut **tx)
    .await
    .expect("create user");
    id
}

async fn wallet_for(tx: &mut Transaction<'_, Postgres>, project: Uuid, user: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO payments.wallets (id, project_id, user_id) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(project)
        .bind(user)
        .execute(&mut **tx)
        .await
        .expect("create wallet");
    id
}

async fn connect() -> PgConnection {
    PgConnection::connect(&database_url())
        .await
        .expect("connect to the test database — is postgres up and migrated?")
}

/// The constraint that made a second customer impossible rather than
/// merely unscoped: `auth.users.email` used to be globally UNIQUE, so the
/// first tenant to sign up alice@example.com took that address away from
/// every other tenant on the platform.
#[tokio::test]
#[ignore]
async fn the_same_email_can_exist_in_two_tenants() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    user_in(&mut tx, t.a, "alice@example.com").await;
    user_in(&mut tx, t.b, "alice@example.com").await;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth.users
         WHERE email = 'alice@example.com' AND project_id IN ($1, $2)",
    )
    .bind(t.a)
    .bind(t.b)
    .fetch_one(&mut *tx)
    .await
    .expect("count");
    assert_eq!(
        count, 2,
        "both tenants should own the address independently"
    );

    tx.rollback().await.expect("rollback");
}

/// Scoping the constraint must not weaken it: within one tenant an email
/// is still exactly one account.
#[tokio::test]
#[ignore]
async fn a_duplicate_email_within_one_tenant_is_still_rejected() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    user_in(&mut tx, t.a, "alice@example.com").await;

    let err = sqlx::query(
        "INSERT INTO auth.users (project_id, email, password_hash)
         VALUES ($1, 'alice@example.com', 'x')",
    )
    .bind(t.a)
    .execute(&mut *tx)
    .await
    .expect_err("a second alice in the same tenant must be rejected");

    assert!(
        err.to_string().contains("users_project_email_key"),
        "expected the scoped unique constraint to fire, got: {err}"
    );

    tx.rollback().await.expect("rollback");
}

/// Idempotency keys are chosen by the caller, so "order-1" is not a
/// distinctive string — it is the string every integration picks first.
/// While the unique index was global, the second tenant to use it did not
/// get an error: it got the FIRST tenant's transaction handed back as a
/// successful idempotent replay.
#[tokio::test]
#[ignore]
async fn the_same_idempotency_key_can_be_used_by_two_tenants() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    for project in [t.a, t.b] {
        let user = user_in(
            &mut tx,
            project,
            &format!("u-{}@example.com", Uuid::new_v4()),
        )
        .await;
        let wallet = wallet_for(&mut tx, project, user).await;
        sqlx::query(
            "INSERT INTO payments.transactions
                 (project_id, to_wallet, amount_cents, idempotency_key, kind)
             VALUES ($1, $2, 100, 'order-1', 'deposit')",
        )
        .bind(project)
        .bind(wallet)
        .execute(&mut *tx)
        .await
        .expect("both tenants must be able to use the same key");
    }

    tx.rollback().await.expect("rollback");
}

/// Reusing a key within one tenant is still the idempotency signal it has
/// always been.
#[tokio::test]
#[ignore]
async fn a_reused_idempotency_key_within_one_tenant_still_conflicts() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    let user = user_in(&mut tx, t.a, &format!("u-{}@example.com", Uuid::new_v4())).await;
    let wallet = wallet_for(&mut tx, t.a, user).await;

    let insert = "INSERT INTO payments.transactions
                      (project_id, to_wallet, amount_cents, idempotency_key, kind)
                  VALUES ($1, $2, 100, 'order-1', 'deposit')";
    sqlx::query(insert)
        .bind(t.a)
        .bind(wallet)
        .execute(&mut *tx)
        .await
        .expect("first insert");

    let err = sqlx::query(insert)
        .bind(t.a)
        .bind(wallet)
        .execute(&mut *tx)
        .await
        .expect_err("the same key twice in one tenant must conflict");
    assert!(
        err.to_string()
            .contains("transactions_project_idempotency_key"),
        "expected the scoped idempotency constraint to fire, got: {err}"
    );

    tx.rollback().await.expect("rollback");
}

/// The one that matters most: money must not be able to cross a tenant
/// boundary even if a query somewhere forgets to scope itself.
///
/// This is expressed as composite foreign keys — (wallet, project) pairs —
/// rather than as a check in the payments service, so it holds for every
/// writer including migrations, backfills, and a psql session.
#[tokio::test]
#[ignore]
async fn a_cross_tenant_transfer_is_rejected_by_the_database() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    let user_a = user_in(&mut tx, t.a, &format!("a-{}@example.com", Uuid::new_v4())).await;
    let user_b = user_in(&mut tx, t.b, &format!("b-{}@example.com", Uuid::new_v4())).await;
    let wallet_a = wallet_for(&mut tx, t.a, user_a).await;
    let wallet_b = wallet_for(&mut tx, t.b, user_b).await;

    let err = sqlx::query(
        "INSERT INTO payments.transactions
             (project_id, from_wallet, to_wallet, amount_cents, idempotency_key)
         VALUES ($1, $2, $3, 5000, 'cross-tenant')",
    )
    .bind(t.a)
    .bind(wallet_a)
    .bind(wallet_b)
    .execute(&mut *tx)
    .await
    .expect_err("draining a wallet into another tenant must be impossible");

    assert!(
        err.to_string()
            .contains("transactions_to_wallet_project_fkey"),
        "expected the composite wallet/project foreign key to fire, got: {err}"
    );

    tx.rollback().await.expect("rollback");
}

/// The mirror of the test above: the composite keys must not have broken
/// ordinary transfers, and a deposit — `from_wallet IS NULL`, money
/// arriving from outside — must still be expressible. MATCH SIMPLE skips
/// the check when a referencing column is NULL, which is what makes that
/// work.
#[tokio::test]
#[ignore]
async fn same_tenant_transfers_and_deposits_still_work() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    let payer = user_in(&mut tx, t.a, &format!("p-{}@example.com", Uuid::new_v4())).await;
    let payee = user_in(&mut tx, t.a, &format!("q-{}@example.com", Uuid::new_v4())).await;
    let from = wallet_for(&mut tx, t.a, payer).await;
    let to = wallet_for(&mut tx, t.a, payee).await;

    sqlx::query(
        "INSERT INTO payments.transactions
             (project_id, from_wallet, to_wallet, amount_cents, idempotency_key)
         VALUES ($1, $2, $3, 5000, $4)",
    )
    .bind(t.a)
    .bind(from)
    .bind(to)
    .bind(format!("transfer-{}", Uuid::new_v4()))
    .execute(&mut *tx)
    .await
    .expect("a transfer within one tenant must still be accepted");

    sqlx::query(
        "INSERT INTO payments.transactions
             (project_id, to_wallet, amount_cents, idempotency_key, kind)
         VALUES ($1, $2, 5000, $3, 'deposit')",
    )
    .bind(t.a)
    .bind(to)
    .bind(format!("deposit-{}", Uuid::new_v4()))
    .execute(&mut *tx)
    .await
    .expect("a deposit has no from_wallet and must still be accepted");

    tx.rollback().await.expect("rollback");
}

/// Deleting a project must take its data with it. Without the cascade,
/// off-boarding a customer would leave their users, wallets and locations
/// behind as orphans that no API can reach and no one can find.
#[tokio::test]
#[ignore]
async fn deleting_a_project_removes_its_data() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.expect("begin");
    let t = two_tenants(&mut tx).await;

    let doomed = user_in(&mut tx, t.a, &format!("d-{}@example.com", Uuid::new_v4())).await;
    wallet_for(&mut tx, t.a, doomed).await;
    let survivor = user_in(&mut tx, t.b, &format!("s-{}@example.com", Uuid::new_v4())).await;

    sqlx::query("DELETE FROM control.projects WHERE id = $1")
        .bind(t.a)
        .execute(&mut *tx)
        .await
        .expect("delete project");

    let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth.users WHERE id = $1")
        .bind(doomed)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(gone, 0, "the deleted project's users must be gone");

    let wallets: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM payments.wallets WHERE project_id = $1")
            .bind(t.a)
            .fetch_one(&mut *tx)
            .await
            .expect("count");
    assert_eq!(wallets, 0, "the deleted project's wallets must be gone");

    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth.users WHERE id = $1")
        .bind(survivor)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(kept, 1, "the other tenant must be untouched");

    tx.rollback().await.expect("rollback");
}
