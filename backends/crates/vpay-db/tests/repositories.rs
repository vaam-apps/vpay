//! Integration tests for the OP-2 repository layer —
//! [`vpay_db::SqlClientAssertionStore`], the `disabled_clients` kill-switch
//! lookup, and the `oauth_signing_keys` repository — against a real Postgres
//! via testcontainers.
//!
//! Same technique as `tests/postgres.rs` (deliberately duplicated rather
//! than shared — see that file's own comment): `testcontainers-modules`
//! 0.15 defaults to `postgres:11-alpine`, uncached and unpullable here, so
//! every container here is pinned `.with_tag("16-alpine")`.
//!
//! One test per file-level concern below, each starting its own container —
//! five tests total, comparable in count to `tests/postgres.rs`'s three.
//! This crate is not covered by `.config/nextest.toml`'s
//! `postgres-containers` concurrency cap (that file is owned by another
//! agent and scoped to `package(vpay-tests-integration)` only), so nextest's
//! default per-CPU parallelism applies here same as it already does to
//! `tests/postgres.rs`.

use std::sync::Arc;

use anyhow::Context;
use authkestra_op::client_assertion::ClientAssertionStore;
use authkestra_op::error::OpError;
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Starts a fresh, migrated Postgres 16 container and returns a pool bound
/// to it. The returned container guard must be kept alive for as long as the
/// pool is used — dropping it stops and removes the container.
async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
    let container = PostgresImage::default()
        .with_tag("16-alpine")
        .start()
        .await
        .context("postgres:16-alpine container starts (it is cached locally on this machine)")?;

    let host = container.get_host().await.context("container host")?;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .context("container port")?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = vpay_db::connect(&url)
        .await
        .context("connects to the freshly started container")?;
    vpay_db::run_migrations(&pool)
        .await
        .context("every migration under backends/migrations applies cleanly")?;

    Ok((container, pool))
}

// --- SqlClientAssertionStore -----------------------------------------------

/// A `jti` is fresh exactly once: the first `record_jti` call must accept
/// (return `Ok(true)`), and a second call with the same `jti` must be
/// rejected as a replay (`Ok(false)`) — not an error, per the trait's own
/// contract.
#[tokio::test]
async fn a_client_assertion_jti_is_fresh_once_then_replayed() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    let store = vpay_db::SqlClientAssertionStore::new(pool);
    let expires_at = Utc::now() + chrono::Duration::seconds(60);

    let first = store
        .record_jti("jti-fresh-then-replayed", expires_at)
        .await
        .context("the first presentation of a jti must not error")?;
    assert!(first, "the first presentation of a jti must be fresh");

    let second = store
        .record_jti("jti-fresh-then-replayed", expires_at)
        .await
        .context("a replayed jti must still return Ok, not error")?;
    assert!(
        !second,
        "a replayed jti must be rejected (Ok(false)), not accepted a second time"
    );

    Ok(())
}

/// Proves the replay guard is actually atomic, not merely correct when
/// called sequentially: fires 10 concurrent `record_jti` calls with the
/// *same* jti against a real Postgres and asserts exactly one reports fresh.
/// Mirrors `authkestra-op`'s own `sqlx_store::tests::test_postgres_concurrency`
/// (10 concurrent `consume_code` calls, exactly 1 success) — the same
/// TOCTOU race the migration's own header comment warns a separate
/// SELECT-then-INSERT would reintroduce, and the same reason `record_jti`'s
/// trait doc comment requires atomicity: two concurrent presentations of the
/// same captured assertion must not both observe "not yet seen."
#[tokio::test]
async fn concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    let store = Arc::new(vpay_db::SqlClientAssertionStore::new(pool));
    let expires_at = Utc::now() + chrono::Duration::seconds(60);

    const ATTEMPTS: usize = 10;
    let mut handles = Vec::with_capacity(ATTEMPTS);
    for _ in 0..ATTEMPTS {
        let store = Arc::clone(&store);
        handles.push(tokio::spawn(async move {
            store.record_jti("jti-concurrent-race", expires_at).await
        }));
    }

    let mut fresh = 0;
    let mut replayed = 0;
    for handle in handles {
        let result: Result<bool, OpError> =
            handle.await.context("a spawned record_jti task panicked")?;
        match result.context("record_jti must not error under concurrent contention")? {
            true => fresh += 1,
            false => replayed += 1,
        }
    }

    assert_eq!(
        fresh, 1,
        "exactly one of {ATTEMPTS} concurrent presentations of the same jti must be fresh"
    );
    assert_eq!(
        replayed,
        ATTEMPTS - 1,
        "every other concurrent presentation must be rejected as a replay"
    );

    Ok(())
}

// --- disabled_clients --------------------------------------------------

/// The kill-switch lookup itself: a client with no `disabled_clients` row
/// reports not-disabled; disabling it flips the lookup; re-enabling it flips
/// the lookup back — proving `is_client_disabled`, `disable_client` and
/// `enable_client` all observe the same underlying table consistently.
#[tokio::test]
async fn disabled_client_lookup_reflects_disable_and_enable() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    assert!(
        !vpay_db::is_client_disabled(&pool, "merchant_never_disabled").await?,
        "a client_id with no disabled_clients row must report not-disabled"
    );

    vpay_db::disable_client(&pool, "merchant_never_disabled", Some("key compromised"))
        .await
        .context("disabling a client must succeed")?;
    assert!(
        vpay_db::is_client_disabled(&pool, "merchant_never_disabled").await?,
        "a client_id with a disabled_clients row must report disabled"
    );

    // disable_client must be idempotent at this layer — a second call for an
    // already-disabled client must not error (the module's own doc comment
    // argues for INSERT ... ON CONFLICT DO UPDATE specifically so an
    // operator re-running "disable this client" never has to check first).
    vpay_db::disable_client(
        &pool,
        "merchant_never_disabled",
        Some("re-confirmed compromised"),
    )
    .await
    .context("a second disable of the same client_id must not error")?;
    assert!(vpay_db::is_client_disabled(&pool, "merchant_never_disabled").await?);

    vpay_db::enable_client(&pool, "merchant_never_disabled")
        .await
        .context("re-enabling a client must succeed")?;
    assert!(
        !vpay_db::is_client_disabled(&pool, "merchant_never_disabled").await?,
        "a client_id must report not-disabled again after enable_client removes its row"
    );

    // enable_client on a client_id that was never disabled must be a no-op,
    // not an error.
    vpay_db::enable_client(&pool, "merchant_was_never_disabled_at_all")
        .await
        .context("enabling a client with no disabled_clients row must not error")?;

    Ok(())
}

// --- oauth_signing_keys --------------------------------------------------

/// Test fixture JWK — a syntactically valid public-key shape; nothing in
/// this repository layer inspects its contents (that is `authkestra_engine`
/// / attestation code, out of this crate's scope), so a fixed fixture value
/// is sufficient across every test that needs "some JWK."
fn fixture_public_jwk(label: &str) -> serde_json::Value {
    json!({
        "kty": "RSA",
        "alg": "RS256",
        "use": "sig",
        "n": format!("fixture-modulus-{label}"),
        "e": "AQAB",
    })
}

/// `publishable_signing_keys` (`WHERE active OR expires_at > now()`) must
/// return the active key and a retired-but-not-yet-expired key, and must
/// exclude a key retired long enough ago that its `expires_at` has already
/// passed.
#[tokio::test]
async fn publishable_signing_keys_includes_active_and_unexpired_retired_but_excludes_expired()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    sqlx::query(
        "INSERT INTO oauth_signing_keys (kid, public_jwk, active) VALUES ($1, $2::jsonb, true)",
    )
    .bind("key_active")
    .bind(fixture_public_jwk("active"))
    .execute(&pool)
    .await
    .context("inserting the active key must succeed")?;

    sqlx::query(
        "INSERT INTO oauth_signing_keys (kid, public_jwk, active, expires_at) \
         VALUES ($1, $2::jsonb, false, now() + interval '30 minutes')",
    )
    .bind("key_retired_not_yet_expired")
    .bind(fixture_public_jwk("retired-not-yet-expired"))
    .execute(&pool)
    .await
    .context("inserting the not-yet-expired retired key must succeed")?;

    // `expiry_after_creation` (migration 0007) requires `expires_at >
    // created_at`, and `created_at` is not settable from this INSERT (it
    // defaults to `now()` at insert time) — so an "already expired" row
    // cannot be inserted with a past `expires_at` outright; the constraint
    // would reject it as backwards. Instead, insert with `expires_at` a
    // moment in the future (satisfying the constraint at insert time), then
    // let real time carry it into the past before querying below.
    sqlx::query(
        "INSERT INTO oauth_signing_keys (kid, public_jwk, active, expires_at) \
         VALUES ($1, $2::jsonb, false, now() + interval '1 millisecond')",
    )
    .bind("key_expired")
    .bind(fixture_public_jwk("expired"))
    .execute(&pool)
    .await
    .context("inserting the soon-to-expire key must succeed")?;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut kids: Vec<String> = vpay_db::publishable_signing_keys(&pool)
        .await
        .context("publishable_signing_keys query must succeed")?
        .into_iter()
        .map(|key| key.kid)
        .collect();
    kids.sort();

    assert_eq!(
        kids,
        vec![
            "key_active".to_string(),
            "key_retired_not_yet_expired".to_string(),
        ],
        "publishable_signing_keys must include the active key and the not-yet-expired retired \
         key, and must exclude the already-expired key"
    );

    Ok(())
}

/// `rotate_signing_key` must, in one transaction: retire whatever was
/// active before it and insert the new key as the sole active row —
/// proven by reading `active_signing_key_kid` (backed by the same
/// `one_active_signing_key` partial unique index the repository's own doc
/// comment explains the statement ordering around) before and after.
#[tokio::test]
async fn rotate_signing_key_leaves_exactly_one_active_key() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    // Bootstrap case first: rotating with no key currently active must not
    // error (the UPDATE affects zero rows and the INSERT becomes an
    // ordinary bootstrap insert, per the function's own doc comment).
    vpay_db::rotate_signing_key(
        &pool,
        "key_v1",
        &fixture_public_jwk("v1"),
        time::OffsetDateTime::now_utc() + time::Duration::minutes(30),
    )
    .await
    .context("rotating with no prior active key (bootstrap) must succeed")?;

    assert_eq!(
        vpay_db::active_signing_key_kid(&pool).await?,
        Some("key_v1".to_string()),
        "after the bootstrap rotation, key_v1 must be the sole active key"
    );

    // Real rotation: an active key already exists (key_v1); rotating to
    // key_v2 must retire key_v1 and leave key_v2 as the only active row.
    vpay_db::rotate_signing_key(
        &pool,
        "key_v2",
        &fixture_public_jwk("v2"),
        time::OffsetDateTime::now_utc() + time::Duration::minutes(30),
    )
    .await
    .context("rotating from an existing active key must succeed")?;

    assert_eq!(
        vpay_db::active_signing_key_kid(&pool).await?,
        Some("key_v2".to_string()),
        "after rotation, key_v2 must be the sole active key"
    );

    let active_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM oauth_signing_keys WHERE active")
            .fetch_one(&pool)
            .await
            .context("counting active rows directly must succeed")?;
    assert_eq!(
        active_count, 1,
        "exactly one row must be active after rotation, proving the transaction left the \
         one_active_signing_key invariant intact"
    );

    let retired_kids: Vec<String> = vpay_db::publishable_signing_keys(&pool)
        .await
        .context("publishable_signing_keys after rotation must succeed")?
        .into_iter()
        .map(|key| key.kid)
        .collect();
    assert!(
        retired_kids.contains(&"key_v1".to_string()),
        "key_v1 must still be publishable (within its retirement overlap window) after rotation"
    );
    assert!(retired_kids.contains(&"key_v2".to_string()));

    Ok(())
}
