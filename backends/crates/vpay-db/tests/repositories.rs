//! Integration tests for the OP-2 repository layer —
//! [`vpay_db::SqlClientAssertionStore`], the `disabled_clients` kill-switch
//! lookup, and the `oauth_signing_keys` repository — against a real Postgres
//! via testcontainers.
//!
//! Same technique as `tests/postgres.rs`: the container comes from
//! `vpay_testkit::containers::start_postgres_with_retry` (pinned image tag
//! and host-port-collision retry are documented there), and this file adds
//! only the pool and the migration run on top of it.
//!
//! One test per file-level concern below, each starting its own container.
//! The `oauth_signing_keys` section grew by three when
//! `ensure_active_signing_key` landed (the boot-time entry point: bootstrap,
//! idempotent re-boot, rotation, concurrency, and the rollback gap), which
//! is why the count no longer matches `tests/postgres.rs`'s three; each of
//! those three asserts something the others cannot, so none of them is
//! foldable into another without losing a proof.
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
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Starts a fresh, migrated Postgres 16 container and returns a pool bound
/// to it. The returned container guard must be kept alive for as long as the
/// pool is used — dropping it stops and removes the container.
///
/// The image request and its host-port-collision retry live in
/// `vpay_testkit::containers` — see that module for why the tag is pinned and
/// which errors are retried.
async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
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

/// `TIMESTAMPTZ` keeps microseconds; `OffsetDateTime::now_utc()` keeps
/// nanoseconds. Round-tripping a value through the database and comparing
/// it with `==` only works if the value went in at the precision the column
/// stores, so tests that read a timestamp back build it with this.
fn microsecond_precision(instant: time::OffsetDateTime) -> time::OffsetDateTime {
    let nanos = instant.nanosecond();
    instant
        .replace_nanosecond((nanos / 1_000) * 1_000)
        .unwrap_or(instant)
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

/// The boot-time entry point: `ensure_active_signing_key` must bootstrap an
/// empty table, then be a genuine no-op when the same `kid` is presented
/// again, then rotate exactly once when a *different* `kid` arrives.
///
/// The "no write at all" half is asserted on `updated_at`, not just on the
/// returned [`vpay_db::ActivationOutcome`]: a re-presented `kid` that
/// re-wrote its own row (an `ON CONFLICT DO UPDATE`, say) would still report
/// `AlreadyActive` while quietly touching the table on every replica's every
/// boot. Comparing the timestamp before and after is what makes the claim in
/// that variant's doc comment testable.
#[tokio::test]
async fn ensure_active_signing_key_bootstraps_is_idempotent_then_rotates_once() -> anyhow::Result<()>
{
    let (_container, pool) = migrated_postgres().await?;
    // Truncated to whole microseconds before it is written: `TIMESTAMPTZ`
    // stores microseconds, so a nanosecond-precision instant would never
    // compare equal to what Postgres hands back. This is the exact
    // mismatch that made this test fail on its first real run.
    let retire_at =
        microsecond_precision(time::OffsetDateTime::now_utc() + time::Duration::hours(24));

    let first = vpay_db::ensure_active_signing_key(
        &pool,
        "kid_boot_v1",
        &fixture_public_jwk("boot-v1"),
        retire_at,
    )
    .await
    .context("bootstrapping an empty table must succeed")?;
    assert_eq!(
        first,
        vpay_db::ActivationOutcome::Rotated { previous: None },
        "the first key this database ever holds is a rotation with no predecessor"
    );

    let updated_at_after_bootstrap: time::OffsetDateTime =
        sqlx::query_scalar("SELECT updated_at FROM oauth_signing_keys WHERE kid = 'kid_boot_v1'")
            .fetch_one(&pool)
            .await
            .context("reading the bootstrapped row's updated_at must succeed")?;

    // The second replica, holding the identical PEM and therefore the
    // identical thumbprint `kid`.
    let second = vpay_db::ensure_active_signing_key(
        &pool,
        "kid_boot_v1",
        &fixture_public_jwk("boot-v1"),
        retire_at,
    )
    .await
    .context("a second replica presenting the same kid must not error")?;
    assert_eq!(
        second,
        vpay_db::ActivationOutcome::AlreadyActive,
        "presenting the already-active kid must not rotate"
    );

    let updated_at_after_second: time::OffsetDateTime =
        sqlx::query_scalar("SELECT updated_at FROM oauth_signing_keys WHERE kid = 'kid_boot_v1'")
            .fetch_one(&pool)
            .await
            .context("re-reading updated_at must succeed")?;
    assert_eq!(
        updated_at_after_bootstrap, updated_at_after_second,
        "AlreadyActive must write nothing at all — not even an idempotent re-write of its own row"
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oauth_signing_keys")
        .fetch_one(&pool)
        .await
        .context("counting rows must succeed")?;
    assert_eq!(rows, 1, "an idempotent boot must not accumulate rows");

    // A deploy with a new Secret: a different thumbprint, so exactly one
    // rotation, naming the key it displaced.
    let third = vpay_db::ensure_active_signing_key(
        &pool,
        "kid_boot_v2",
        &fixture_public_jwk("boot-v2"),
        retire_at,
    )
    .await
    .context("rotating to a new kid must succeed")?;
    assert_eq!(
        third,
        vpay_db::ActivationOutcome::Rotated {
            previous: Some("kid_boot_v1".to_string())
        },
        "a new kid must rotate and report which key it displaced"
    );

    assert_eq!(
        vpay_db::active_signing_key_kid(&pool).await?,
        Some("kid_boot_v2".to_string())
    );
    let retired_expires_at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT expires_at FROM oauth_signing_keys WHERE kid = 'kid_boot_v1'")
            .fetch_one(&pool)
            .await
            .context("reading the retired key's expires_at must succeed")?;
    assert_eq!(
        retired_expires_at,
        Some(retire_at),
        "the displaced key must be retired at exactly the caller's overlap instant, so it keeps \
         publishing in /jwks.json until then"
    );

    Ok(())
}

/// The property the boot path actually depends on, and the one a sequential
/// test cannot prove: N replicas starting at the same moment with the *same*
/// key must produce exactly one rotation between them, not N.
///
/// Same shape of proof as
/// `concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result`
/// above. Without the transaction-scoped advisory lock in
/// `signing_keys::ensure_active_signing_key`, every task reads "no active
/// key" from its own snapshot and races to insert the same `kid`: the losers
/// fail on the `kid` primary key, so this test would report an error rather
/// than `AlreadyActive`.
#[tokio::test]
async fn concurrent_ensure_active_signing_key_calls_with_the_same_kid_rotate_exactly_once()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    // Truncated to whole microseconds before it is written: `TIMESTAMPTZ`
    // stores microseconds, so a nanosecond-precision instant would never
    // compare equal to what Postgres hands back. This is the exact
    // mismatch that made this test fail on its first real run.
    let retire_at =
        microsecond_precision(time::OffsetDateTime::now_utc() + time::Duration::hours(24));

    // Eight, not ten: `vpay_db::connect`'s pool caps at ten connections and
    // each task holds one for the whole of its transaction, so this stays
    // clear of the acquire timeout while still being genuinely concurrent.
    const REPLICAS: usize = 8;
    let mut handles = Vec::with_capacity(REPLICAS);
    for _ in 0..REPLICAS {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            vpay_db::ensure_active_signing_key(
                &pool,
                "kid_all_replicas_share",
                &fixture_public_jwk("shared"),
                retire_at,
            )
            .await
        }));
    }

    let mut rotated = 0;
    let mut already_active = 0;
    for handle in handles {
        let outcome = handle
            .await
            .context("a spawned ensure_active_signing_key task panicked")?
            .context("no replica's boot-time activation may error under contention")?;
        match outcome {
            vpay_db::ActivationOutcome::Rotated { previous } => {
                assert_eq!(previous, None, "there was no key before any of these");
                rotated += 1;
            }
            vpay_db::ActivationOutcome::AlreadyActive => already_active += 1,
        }
    }

    assert_eq!(
        rotated, 1,
        "exactly one of {REPLICAS} simultaneous replicas may write the key"
    );
    assert_eq!(already_active, REPLICAS - 1);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM oauth_signing_keys")
        .fetch_one(&pool)
        .await
        .context("counting rows must succeed")?;
    assert_eq!(rows, 1, "{REPLICAS} concurrent boots must leave one row");

    Ok(())
}

/// Pins the documented refusal on `ensure_active_signing_key`: rolling
/// *back* to a previously retired `kid` is an error, not a re-activation.
///
/// This test asserts today's behaviour rather than the behaviour someone
/// might want, so that whoever implements re-activation has to change it
/// deliberately — and so nobody reads the function's doc comment as
/// speculative. See `docs/roadmap.md`, "Open — signing-key rotation overlap
/// window", for why the policy this needs is not settled.
///
/// The **variant** is asserted, not merely `is_err()`. A rollback used to
/// come back as `DbError::Query(sqlx::Error::Database(..))` — a duplicate
/// key on `oauth_signing_keys_pkey` — which classifies as
/// `Category::Storage` and therefore told `vpay-server`'s supervisor to exit
/// `69` and keep restarting, waiting for a database that was working
/// perfectly. `is_err()` alone could not tell that apart from the fix, so it
/// would have gone on passing while the crash loop stayed.
#[tokio::test]
async fn ensure_active_signing_key_refuses_to_reactivate_a_retired_kid() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    // Truncated to whole microseconds before it is written: `TIMESTAMPTZ`
    // stores microseconds, so a nanosecond-precision instant would never
    // compare equal to what Postgres hands back. This is the exact
    // mismatch that made this test fail on its first real run.
    let retire_at =
        microsecond_precision(time::OffsetDateTime::now_utc() + time::Duration::hours(24));

    vpay_db::ensure_active_signing_key(&pool, "kid_old", &fixture_public_jwk("old"), retire_at)
        .await
        .context("bootstrapping the old key must succeed")?;
    vpay_db::ensure_active_signing_key(&pool, "kid_new", &fixture_public_jwk("new"), retire_at)
        .await
        .context("rotating to the new key must succeed")?;

    let rolled_back =
        vpay_db::ensure_active_signing_key(&pool, "kid_old", &fixture_public_jwk("old"), retire_at)
            .await;
    let error = rolled_back.expect_err(
        "re-activating a retired kid is not supported and must fail loudly at boot, not silently \
         republish a deliberately retired key",
    );
    match &error {
        vpay_db::DbError::SigningKeyRetired { kid, retired_at } => {
            assert_eq!(kid, "kid_old");
            // The retirement instant comes from the row the *previous*
            // rotation wrote, so it is in the past — a check that the field
            // carries `updated_at` and not the future `expires_at` the
            // caller passed in (`now + 24h`).
            assert!(
                *retired_at <= time::OffsetDateTime::now_utc(),
                "retired_at must be when the key was retired, not when it stops publishing: \
                 {retired_at}"
            );
        }
        other => panic!(
            "a rollback to a retired key must be its own error, not a raw SQL failure: {other:?}"
        ),
    }
    // The classification is what a supervisor acts on: 78 ("fix the deploy"),
    // never 69 ("wait for Postgres") — which is what the duplicate-key
    // `DbError::Query` this replaced produced.
    assert_eq!(
        vpay_core::Classify::category(&error),
        vpay_core::Category::Configuration
    );
    // And the sentence an operator reads out of the crash loop.
    let text = error.to_string();
    assert!(text.contains("kid_old"), "{text}");
    assert!(text.contains("generate a new key"), "{text}");

    // The failed transaction must have rolled back whole: the new key is
    // still the active one, and the database is not left with zero.
    assert_eq!(
        vpay_db::active_signing_key_kid(&pool).await?,
        Some("kid_new".to_string()),
        "a failed rollback attempt must leave the previously active key untouched"
    );

    Ok(())
}

// --- step 2: payment intents, charges, idempotency, provider requests -----
//
// Everything below covers migrations 0014-0018 and the repositories added
// with them. Same technique as the sections above — one container per test,
// via `migrated_postgres()` — and the same rule about helpers: they return
// `anyhow::Result` and propagate with `?`, because `clippy.toml`'s test
// exemption only covers a `#[test]`-attributed function body.

/// The reference data every payment-intent test needs, seeded through the
/// production path (`config_reconcile::reconcile`) rather than by hand-rolled
/// `INSERT`s — so a test that passes is evidence about the code that ships,
/// not about a fixture that resembles it.
async fn seed_reference_data(pool: &PgPool) -> anyhow::Result<()> {
    vpay_db::config_reconcile::reconcile(
        pool,
        &[vpay_db::CurrencySeed {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        &[vpay_db::ProviderSeed {
            code: "mtn_momo".to_owned(),
            display_name: "MTN MoMo".to_owned(),
            flow: "push".to_owned(),
            supports_refunds: false,
            supports_partial_refunds: false,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
            enabled: true,
        }],
    )
    .await
    .context("seeding currencies and providers must succeed")?;
    Ok(())
}

/// A `requires_payment_method` intent for `merchant_a` in the seeded
/// currency. Only the fields a test varies are parameters; everything else
/// is deliberately boring so an assertion failure points at the behaviour
/// under test rather than at fixture noise.
fn fixture_intent(id: &str, currency_code: &str) -> vpay_db::NewPaymentIntent {
    vpay_db::NewPaymentIntent {
        id: id.to_owned(),
        merchant_id: "merchant_a".to_owned(),
        livemode: false,
        amount: 5000,
        currency_code: currency_code.to_owned(),
        status: "requires_payment_method".to_owned(),
        last_payment_error_code: None,
        last_payment_error_message: None,
        payment_method_types: json!(["mtn_momo"]),
        metadata: json!({}),
        description: None,
        created_at: time::OffsetDateTime::now_utc(),
    }
}

/// A `submitting` charge against `payment_intent_id`, on the seeded rail.
fn fixture_charge(id: &str, payment_intent_id: &str) -> vpay_db::NewCharge {
    vpay_db::NewCharge {
        id: id.to_owned(),
        payment_intent_id: payment_intent_id.to_owned(),
        provider_code: "mtn_momo".to_owned(),
        provider_reference_id: uuid::Uuid::new_v4(),
        provider_ref_extra: None,
        redirect_url: None,
        return_url: None,
        state: "submitting".to_owned(),
        amount: 5000,
        currency_code: "XAF".to_owned(),
        payer_ref: Some("+237600000000".to_owned()),
        payer_ref_masked: Some("+2376••••000".to_owned()),
    }
}

/// The id of the nth paging fixture, zero-padded so lexical and insertion
/// order agree — computed on demand so the paging expectations never index
/// into a collected Vec (`clippy::indexing_slicing` is denied here too).
fn page_fixture_id(n: usize) -> String {
    format!("pi_{n:02}")
}

/// A 32-byte request hash whose bytes are all `seed`, so two fixtures either
/// match exactly or differ in all 32 bytes.
const fn fixture_hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

/// Migrations 0014-0018 apply to a clean database *and* leave the shape the
/// repositories below assume — in particular the hard cutover 0014 performs:
/// `last_payment_error` is **gone**, replaced by the code/message pair.
///
/// `migrated_postgres()` already proves the migrations apply (every test in
/// this file would fail otherwise). What this adds is that they applied to
/// the intended *shape*: a migration that silently no-ops (an `ADD COLUMN IF
/// NOT EXISTS` against an already-patched database, a file `sqlx` skipped
/// because the checksum matched an earlier version) applies just as
/// successfully as one that did the work.
#[tokio::test]
async fn migration_0014_replaces_last_payment_error_and_0015_to_0018_create_their_tables()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::TEXT FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'payment_intents'",
    )
    .fetch_all(&pool)
    .await
    .context("reading payment_intents' columns must succeed")?;

    for added in [
        "seq",
        "metadata",
        "description",
        "updated_at",
        "last_payment_error_code",
        "last_payment_error_message",
    ] {
        assert!(
            columns.iter().any(|c| c == added),
            "0014 must add payment_intents.{added}; found {columns:?}"
        );
    }
    assert!(
        !columns.iter().any(|c| c == "last_payment_error"),
        "0014 is a hard cutover: the free-text last_payment_error column must be DROPPED, not \
         left alongside its structured replacement — two sources for one fact is how they drift. \
         Found {columns:?}"
    );

    let charge_columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::TEXT FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'charges'",
    )
    .fetch_all(&pool)
    .await
    .context("reading charges' columns must succeed")?;
    assert!(
        charge_columns.iter().any(|c| c == "updated_at"),
        "0014 must add charges.updated_at; found {charge_columns:?}"
    );

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::TEXT FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_all(&pool)
    .await
    .context("listing public tables must succeed")?;
    for created in ["idempotency_keys", "provider_requests", "refunds", "events"] {
        assert!(
            tables.iter().any(|t| t == created),
            "0015-0018 must create {created}; found {tables:?}"
        );
    }

    // The partial index 0014 adds over live charge states. Named explicitly
    // because it is the one index whose *predicate* had to be transcribed
    // from `charge_state`'s real enum labels — a typo there is silent (the
    // index simply never matches) rather than an error.
    let live_index: Option<String> =
        sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE indexname = 'charges_live_idx'")
            .fetch_optional(&pool)
            .await
            .context("reading charges_live_idx must succeed")?;
    let live_index = live_index.context("0014 must create charges_live_idx")?;
    for label in ["submitting", "submitted", "pending", "unresolved"] {
        assert!(
            live_index.contains(label),
            "charges_live_idx must cover the non-terminal charge_state label {label}: \
             {live_index}"
        );
    }
    assert!(
        !live_index.contains("UNIQUE"),
        "charges_live_idx must NOT be unique — 0004's own comment explains that a unique index \
         scoped to live states would stop covering a failed charge and let a second one in: \
         {live_index}"
    );

    Ok(())
}

/// "One charge per intent, forever" (`AGENTS.md`), and — the half that is
/// easy to get wrong — the *variant* the second attempt comes back as.
///
/// `is_err()` would pass just as happily if this arrived as
/// `DbError::Query`, which classifies as `Category::Storage`: HTTP `503`,
/// `Retry::AfterBackoff`, and a public message telling the merchant vpay is
/// temporarily unavailable and to retry. On a duplicate charge that is
/// advice to charge the payer twice. So the constraint name and the
/// category are both asserted, and `classify_write` is what makes them
/// true.
#[tokio::test]
async fn a_second_charge_for_one_intent_is_refused_as_a_named_unique_violation()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_one_charge", "XAF"))
        .await
        .context("inserting the intent must succeed")?;

    let mut tx = pool.begin().await.context("first transaction begins")?;
    let charge =
        vpay_db::charges::insert_for_intent(&mut tx, &fixture_charge("ch_1", "pi_one_charge"))
            .await
            .context("the first charge for an intent must be accepted")?;
    tx.commit().await.context("first transaction commits")?;
    assert_eq!(charge.state, "submitting");
    assert_eq!(
        charge.currency_code, "XAF",
        "D2: the charge carries the intent's currency verbatim"
    );

    let mut tx = pool.begin().await.context("second transaction begins")?;
    let error =
        vpay_db::charges::insert_for_intent(&mut tx, &fixture_charge("ch_2", "pi_one_charge"))
            .await
            .expect_err("a second charge for the same intent must be refused by the database");

    match &error {
        vpay_db::DbError::UniqueViolation { constraint, .. } => assert_eq!(
            constraint, "one_charge_per_intent",
            "the refusal must name the rule that fired, so a handler can tell a duplicate charge \
             from a duplicate id"
        ),
        other => panic!("expected a named unique violation, got {other:?}"),
    }
    assert_eq!(
        vpay_core::Classify::category(&error),
        vpay_core::Category::Conflict,
        "a duplicate charge is a 409 the merchant can act on, never a 503 telling them to retry"
    );
    assert_eq!(vpay_core::Classify::retry(&error), vpay_core::Retry::Never);

    // The rolled-back attempt must leave exactly the one charge behind.
    drop(tx);
    let charges: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM charges")
        .fetch_one(&pool)
        .await
        .context("counting charges must succeed")?;
    assert_eq!(charges, 1);

    Ok(())
}

/// The two URL columns on `charges` are the only values in this schema that
/// end up in a *browser*, and migration `0019` constrains both.
///
/// `redirect_url` is the rail's own hosted-payment page — rendered as
/// `next_action.redirect_to_url.url` and followed by a payer — and until
/// the Step 3 security review nothing checked it at all: the Orange adapter
/// tested it for non-emptiness and wrote whatever came back. `return_url`
/// is the merchant's. A `javascript:` value in either is stored XSS in
/// whatever renders the intent.
///
/// The application refuses both before the write
/// (`vpay_adapter_orange_money::mapping::checked_redirect_url`,
/// `vpay_api::v1::payment_intents::checked_return_url`), which is what
/// makes those refusals a `400`/`Malformed` rather than the `503` a CHECK
/// violation becomes. This asserts the backstop underneath them, by writing
/// what the application refuses to write — a constraint nothing ever tries
/// to violate is a constraint nobody knows still exists.
#[tokio::test]
async fn a_charges_url_columns_refuse_a_scheme_a_browser_would_execute() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_urls", "XAF")).await?;

    // The happy path first, so the refusals below cannot be passing because
    // the insert was broken for some unrelated reason.
    let mut tx = pool.begin().await.context("transaction begins")?;
    let mut good = fixture_charge("ch_urls", "pi_urls");
    good.return_url = Some("https://shop.example/return".to_owned());
    vpay_db::charges::insert_for_intent(&mut tx, &good)
        .await
        .context("an http(s) return_url must be accepted")?;
    tx.commit().await.context("transaction commits")?;

    let violations: [(&str, &str, &str); 5] = [
        (
            "UPDATE charges SET redirect_url = $1 WHERE id = 'ch_urls'",
            "javascript:alert(document.cookie)",
            "redirect_url_is_a_bounded_web_url",
        ),
        (
            "UPDATE charges SET redirect_url = $1 WHERE id = 'ch_urls'",
            // 2049 characters: one over the ceiling.
            "",
            "redirect_url_is_a_bounded_web_url",
        ),
        (
            "UPDATE charges SET return_url = $1 WHERE id = 'ch_urls'",
            "data:text/html;base64,PHNjcmlwdD4=",
            "return_url_is_a_web_url",
        ),
        (
            "UPDATE charges SET return_url = $1 WHERE id = 'ch_urls'",
            "",
            "return_url_length",
        ),
        (
            "UPDATE charges SET redirect_url = $1 WHERE id = 'ch_urls'",
            "//evil.example/pay",
            "redirect_url_is_a_bounded_web_url",
        ),
    ];

    for (index, (statement, value, expected)) in violations.into_iter().enumerate() {
        // The two empty placeholders above are the over-length cases; built
        // here rather than in the table so the literal is not 2 KB wide.
        let value = if value.is_empty() {
            format!("https://p.example/{}", "x".repeat(2_048))
        } else {
            value.to_owned()
        };
        let error = sqlx::query(statement)
            .bind(&value)
            .execute(&pool)
            .await
            .expect_err(&format!("case {index}: the CHECK must refuse this"));
        assert_eq!(
            error
                .as_database_error()
                .and_then(|e| e.constraint())
                .unwrap_or_default(),
            expected,
            "case {index}: the refusal must name the rule that fired"
        );
    }

    // And the schemes really are case-insensitive on both columns, so a rail
    // shouting its scheme is accepted here exactly as the adapter accepts it.
    sqlx::query("UPDATE charges SET redirect_url = $1, return_url = $2 WHERE id = 'ch_urls'")
        .bind("HTTPS://webpayment.example/pay/tok")
        .bind("HTTP://shop.example/return")
        .execute(&pool)
        .await
        .context("an uppercase scheme is still an http(s) URL")?;

    Ok(())
}

/// A currency the deployment never seeded is the caller's mistake, not a
/// storage outage: `Category::InvalidRequest` (400), never `Storage` (503).
///
/// This is the second half of `classify_write`, and it matters for the same
/// reason as the first: `503` carries `Retry::AfterBackoff`, so a merchant
/// (or the worker) would be told to re-send a request that cannot ever
/// succeed.
#[tokio::test]
async fn an_intent_in_an_unseeded_currency_is_a_named_foreign_key_violation() -> anyhow::Result<()>
{
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    // Shape-valid (three uppercase letters, so `code_is_iso4217_shape` is
    // not what rejects it) but absent from `currencies`.
    let error = vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_bad_ccy", "ZZZ"))
        .await
        .expect_err("an intent in a currency this deployment does not know must be refused");

    match &error {
        vpay_db::DbError::ForeignKeyViolation { constraint, .. } => assert_eq!(
            constraint, "payment_intents_currency_code_fkey",
            "the refusal must name the reference that dangled"
        ),
        other => panic!("expected a named foreign key violation, got {other:?}"),
    }
    assert_eq!(
        vpay_core::Classify::category(&error),
        vpay_core::Category::InvalidRequest
    );
    assert_eq!(vpay_core::Classify::retry(&error), vpay_core::Retry::Never);

    Ok(())
}

/// `transition` is a compare-and-swap, and this is the case that proves it:
/// an `expected` status the row has already moved past updates nothing,
/// returns `Ok(None)`, and leaves every column — `updated_at` included —
/// exactly as it was.
///
/// Without the `AND status = $3` in the `UPDATE`'s own `WHERE`, this test
/// gets `Some(row)` back and the intent is dragged from `canceled` into
/// `processing`: a cancelled payment that starts processing because two
/// requests raced.
#[tokio::test]
async fn a_transition_from_a_stale_expected_status_changes_nothing() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_cas", "XAF"))
        .await
        .context("inserting the intent must succeed")?;

    let cancelled = vpay_db::payment_intents::cancel(&pool, "merchant_a", "pi_cas")
        .await
        .context("cancelling a requires_payment_method intent must succeed")?
        .context("cancel must return the updated row")?;
    assert_eq!(cancelled.status, "canceled");

    // The stale write: whoever issues it still believes the intent is
    // `requires_payment_method`.
    let stale = vpay_db::payment_intents::transition(
        &pool,
        "merchant_a",
        "pi_cas",
        "requires_payment_method",
        "processing",
    )
    .await
    .context("a stale transition must not error — it simply must not fire")?;
    assert_eq!(
        stale, None,
        "a compare-and-swap whose expected status no longer holds must report that it did nothing"
    );

    let after = vpay_db::payment_intents::get_for_merchant(&pool, "merchant_a", "pi_cas")
        .await?
        .context("the intent must still exist")?;
    assert_eq!(
        after, cancelled,
        "not one column may have moved, updated_at included"
    );

    // The same guard must also refuse a foreign merchant, and must not
    // reveal that the intent exists at all.
    let foreign = vpay_db::payment_intents::transition(
        &pool,
        "merchant_b",
        "pi_cas",
        "canceled",
        "processing",
    )
    .await?;
    assert_eq!(foreign, None);
    assert_eq!(
        vpay_db::payment_intents::get_for_merchant(&pool, "merchant_b", "pi_cas").await?,
        None,
        "another merchant's intent must read as missing, not as forbidden"
    );

    Ok(())
}

/// `cancel` refuses an intent the rail may still be acting on, and the
/// refusal is a property of the *write statement*.
///
/// The dangerous shape is specific and reachable today: a `confirm` commits
/// its charge in `submitting` before calling the rail and leaves the intent
/// `requires_payment_method` until it knows what happened, so a status-only
/// compare-and-swap happily cancels an intent whose reference a rail may
/// already hold. Deleting the `NOT EXISTS` from `cancel`'s `UPDATE` makes
/// the first assertion below return the cancelled row.
///
/// The second half is the other error: a charge in a *terminal* state is not
/// in flight, and blocking a cancel on it would leave the merchant with an
/// intent they can neither confirm again ("one charge per intent, forever")
/// nor cancel.
#[tokio::test]
async fn cancel_refuses_an_intent_with_a_live_charge_and_allows_one_with_a_terminal_charge()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_in_flight", "XAF"))
        .await
        .context("inserting the intent must succeed")?;

    let mut tx = pool.begin().await.context("the transaction begins")?;
    vpay_db::charges::insert_for_intent(&mut tx, &fixture_charge("ch_live", "pi_in_flight"))
        .await
        .context("the charge a confirm commits before submitting")?;
    tx.commit().await.context("the transaction commits")?;

    let before = vpay_db::payment_intents::get_for_merchant(&pool, "merchant_a", "pi_in_flight")
        .await?
        .context("the intent must exist")?;
    assert_eq!(
        before.status, "requires_payment_method",
        "the confirm left the status alone — which is exactly why the status is not enough"
    );

    assert_eq!(
        vpay_db::payment_intents::cancel(&pool, "merchant_a", "pi_in_flight").await?,
        None,
        "an intent whose charge may be live must not be cancellable"
    );
    let after = vpay_db::payment_intents::get_for_merchant(&pool, "merchant_a", "pi_in_flight")
        .await?
        .context("the intent must still exist")?;
    assert_eq!(after, before, "not one column may have moved");

    // Every live state blocks it, not just the one a confirm starts in.
    for state in ["submitted", "pending", "unresolved"] {
        sqlx::query("UPDATE charges SET state = $1::charge_state WHERE id = 'ch_live'")
            .bind(state)
            .execute(&pool)
            .await
            .context("moving the charge must succeed")?;
        assert_eq!(
            vpay_db::payment_intents::cancel(&pool, "merchant_a", "pi_in_flight").await?,
            None,
            "a charge in `{state}` is still one the rail may act on"
        );
    }

    // Terminal: nothing is in flight, and the intent can never get another
    // charge, so a cancel is the only thing left that can move it.
    sqlx::query("UPDATE charges SET state = 'failed'::charge_state WHERE id = 'ch_live'")
        .execute(&pool)
        .await
        .context("failing the charge must succeed")?;
    let cancelled = vpay_db::payment_intents::cancel(&pool, "merchant_a", "pi_in_flight")
        .await?
        .context("an intent whose only charge has failed must still be cancellable")?;
    assert_eq!(cancelled.status, "canceled");

    // And the guard is not a substitute for the tenancy filter or the
    // status guard: neither may be reached from another merchant, and a
    // second cancel does nothing.
    assert_eq!(
        vpay_db::payment_intents::cancel(&pool, "merchant_b", "pi_in_flight").await?,
        None
    );
    assert_eq!(
        vpay_db::payment_intents::cancel(&pool, "merchant_a", "pi_in_flight").await?,
        None
    );

    Ok(())
}

/// The `claim_id` a [`vpay_db::IdempotencyClaim::Fresh`] carries, or an
/// error naming what came back instead.
///
/// Every `store`/`release` below needs it, and reading it through one helper
/// is what keeps "the claim was fresh" and "this is the id it was fresh
/// under" from being asserted separately and drifting.
///
/// Returns an error rather than panicking because a free function in a test
/// *file* is not a test *function*, so `clippy.toml`'s
/// `allow-panic-in-tests` does not reach it — and `?` at the call sites
/// reports the same thing.
fn fresh_claim_id(claim: &vpay_db::IdempotencyClaim) -> anyhow::Result<uuid::Uuid> {
    match claim {
        vpay_db::IdempotencyClaim::Fresh { claim_id } => Ok(*claim_id),
        other => Err(anyhow::anyhow!("expected a fresh claim, got {other:?}")),
    }
}

/// The property the whole `Idempotency-Key` contract rests on, and the one
/// a sequential test cannot prove: N simultaneous retries of the same POST
/// must yield exactly one claim.
///
/// Same shape of proof as the `record_jti` and `ensure_active_signing_key`
/// races above. A check-then-insert `claim` passes every sequential test in
/// this file and fails here — every task reads "not claimed" from its own
/// snapshot, and the payer is charged eight times.
#[tokio::test]
async fn concurrent_claims_of_one_idempotency_key_yield_exactly_one_fresh() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    // Eight, not more: `vpay_db::connect`'s pool caps at ten connections.
    const RETRIES: usize = 8;
    let mut handles = Vec::with_capacity(RETRIES);
    for _ in 0..RETRIES {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            vpay_db::idempotency::claim(
                &pool,
                "merchant_a",
                "key-raced",
                "POST",
                "/v1/payment_intents",
                &fixture_hash(0xAB),
            )
            .await
        }));
    }

    let mut fresh = 0;
    let mut in_flight = 0;
    for handle in handles {
        let claim = handle
            .await
            .context("a spawned claim task panicked")?
            .context("no claim may error under contention")?;
        match claim {
            vpay_db::IdempotencyClaim::Fresh { .. } => fresh += 1,
            vpay_db::IdempotencyClaim::InFlight => in_flight += 1,
            other => panic!(
                "the same request under the same key is neither a mismatch nor a replay (nothing \
                 stored a response): {other:?}"
            ),
        }
    }

    assert_eq!(
        fresh, 1,
        "exactly one of {RETRIES} simultaneous retries may do the work"
    );
    assert_eq!(
        in_flight,
        RETRIES - 1,
        "every other retry must be told the request is already running — never Fresh, which \
         would execute the payment again"
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM idempotency_keys")
        .fetch_one(&pool)
        .await
        .context("counting idempotency keys must succeed")?;
    assert_eq!(rows, 1);

    Ok(())
}

/// Reusing a key with a *different* body is `Mismatch` — the `400
/// idempotency_key_in_use` case — and never a replay of the first request's
/// answer.
///
/// Handing back the first response would be the worst available outcome:
/// the merchant would believe their second, different request succeeded.
#[tokio::test]
async fn reusing_an_idempotency_key_with_a_different_request_is_a_mismatch() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let first = vpay_db::idempotency::claim(
        &pool,
        "merchant_a",
        "key-reused",
        "POST",
        "/v1/payment_intents",
        &fixture_hash(0x01),
    )
    .await?;
    let first = fresh_claim_id(&first)?;

    assert_eq!(
        vpay_db::idempotency::store(
            &pool,
            "merchant_a",
            "key-reused",
            first,
            200,
            &json!({"id": "pi_1"}),
        )
        .await
        .context("storing the first response must succeed")?,
        vpay_db::IdempotencyStoreOutcome::Stored
    );

    // Same key, same merchant, different body — so a different hash.
    let second = vpay_db::idempotency::claim(
        &pool,
        "merchant_a",
        "key-reused",
        "POST",
        "/v1/payment_intents",
        &fixture_hash(0x02),
    )
    .await?;
    assert_eq!(
        second,
        vpay_db::IdempotencyClaim::Mismatch,
        "a different request under a used key must be refused, never answered with the other \
         request's response"
    );

    // The key is scoped per merchant: the same key and the same *different*
    // body is simply a fresh claim for someone else.
    let other_merchant = vpay_db::idempotency::claim(
        &pool,
        "merchant_b",
        "key-reused",
        "POST",
        "/v1/payment_intents",
        &fixture_hash(0x02),
    )
    .await?;
    fresh_claim_id(&other_merchant)?;

    Ok(())
}

/// `store` then `claim` replays the stored status and body verbatim — and
/// `store` refuses to run twice.
///
/// The second half is the compare-and-swap on `state = 'in_flight'`. A
/// plain `UPDATE ... WHERE key` would let a late writer overwrite a
/// response a merchant has already been handed, so the replay a client gets
/// today would differ from the one it got a moment ago under the same key.
#[tokio::test]
async fn a_completed_idempotency_key_replays_its_stored_response() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let claim_id = fresh_claim_id(
        &vpay_db::idempotency::claim(
            &pool,
            "merchant_a",
            "key-replayed",
            "POST",
            "/v1/payment_intents",
            &fixture_hash(0x7F),
        )
        .await?,
    )?;

    let body = json!({"id": "pi_replayed", "object": "payment_intent", "amount": 5000});
    assert_eq!(
        vpay_db::idempotency::store(&pool, "merchant_a", "key-replayed", claim_id, 200, &body)
            .await
            .context("storing the response must succeed")?,
        vpay_db::IdempotencyStoreOutcome::Stored
    );

    let replay = vpay_db::idempotency::claim(
        &pool,
        "merchant_a",
        "key-replayed",
        "POST",
        "/v1/payment_intents",
        &fixture_hash(0x7F),
    )
    .await?;
    match replay {
        vpay_db::IdempotencyClaim::Replay(record) => {
            assert_eq!(record.state, "complete");
            assert_eq!(record.response_status, Some(200));
            assert_eq!(
                record.response_body,
                Some(body.clone()),
                "the replay must be the bytes that were sent, not a re-render"
            );
        }
        other => panic!("a completed key must replay its response, got {other:?}"),
    }

    // The *same* claim id: this is the caller completing one key twice,
    // which is its own invariant breaking — not the stale-claim case, which
    // the ABA test below covers and which is deliberately not an error.
    let twice = vpay_db::idempotency::store(
        &pool,
        "merchant_a",
        "key-replayed",
        claim_id,
        500,
        &json!({"error": "would clobber the answer already given"}),
    )
    .await
    .expect_err("completing an already-complete key must be refused, not silently applied");
    match &twice {
        vpay_db::DbError::WriteMatchedNoRow { table, .. } => {
            assert_eq!(*table, "idempotency_keys");
        }
        other => {
            panic!("expected the compare-and-swap to report that it matched nothing: {other:?}")
        }
    }
    assert_eq!(
        vpay_core::Classify::category(&twice),
        vpay_core::Category::Internal,
        "storing twice is vpay's invariant breaking, not the merchant's mistake"
    );

    // And the stored answer is untouched.
    let status: Option<i16> = sqlx::query_scalar(
        "SELECT response_status FROM idempotency_keys WHERE idempotency_key = 'key-replayed'",
    )
    .fetch_one(&pool)
    .await
    .context("re-reading the stored response must succeed")?;
    assert_eq!(status, Some(200));

    Ok(())
}

/// An expired `in_flight` row is claimable again; a live one is not.
///
/// This is the whole of what stops a key from being locked out permanently.
/// A request that claimed a key and then died — a crash, a `5xx` that is
/// deliberately not stored — leaves an `in_flight` row that no other code
/// path ever touches. Without the guarded `DO UPDATE` in `claim`, the next
/// request under that key is told `InFlight` **forever** rather than for the
/// 24 hours `expires_at` promises, and the merchant's key is dead for the
/// life of the deployment.
///
/// Both halves have to hold together: revert the `DO UPDATE` and the first
/// assertion fails; drop its `WHERE ... expires_at < now()` and the second
/// one does — and *that* failure would mean two live requests running under
/// one key, which is the exact double-charge the table exists to prevent.
#[tokio::test]
async fn an_expired_in_flight_key_is_reclaimable_and_a_live_one_is_not() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    for key in ["key-abandoned", "key-running"] {
        fresh_claim_id(
            &vpay_db::idempotency::claim(
                &pool,
                "merchant_a",
                key,
                "POST",
                "/v1/payment_intents",
                &fixture_hash(0x11),
            )
            .await?,
        )?;
    }

    // Age one row past its window, exactly as `sweep_expired`'s test does:
    // `expires_at` is a column default and nothing in the shipping API moves
    // it, so a test that wants an expired row has to write one.
    sqlx::query(
        "UPDATE idempotency_keys SET expires_at = now() - INTERVAL '1 hour' \
         WHERE idempotency_key = 'key-abandoned'",
    )
    .execute(&pool)
    .await
    .context("ageing the row must succeed")?;

    // A *different* request under the abandoned key: different path, and a
    // hash that differs in all 32 bytes. It must be `Fresh` and not
    // `Mismatch` — the reclaim starts a new request, so the row it leaves
    // has to describe that request rather than the abandoned one.
    let reclaim = vpay_db::idempotency::claim(
        &pool,
        "merchant_a",
        "key-abandoned",
        "POST",
        "/v1/payment_intents/pi_x/confirm",
        &fixture_hash(0x22),
    )
    .await?;
    assert!(
        matches!(reclaim, vpay_db::IdempotencyClaim::Fresh { .. }),
        "an expired in_flight row must be reclaimable, or the key is locked out permanently: \
         got {reclaim:?}"
    );

    // The row still `in_flight` inside its window is untouched by any of it.
    assert_eq!(
        vpay_db::idempotency::claim(
            &pool,
            "merchant_a",
            "key-running",
            "POST",
            "/v1/payment_intents",
            &fixture_hash(0x11),
        )
        .await?,
        vpay_db::IdempotencyClaim::InFlight,
        "a claim inside its window is still held; reclaiming it would let two requests run"
    );

    // The reclaimed row describes the new request, and its window restarted.
    let (path, expired): (String, bool) = sqlx::query_as(
        "SELECT request_path, expires_at < now() FROM idempotency_keys \
         WHERE idempotency_key = 'key-abandoned'",
    )
    .fetch_one(&pool)
    .await
    .context("re-reading the reclaimed row must succeed")?;
    assert_eq!(path, "/v1/payment_intents/pi_x/confirm");
    assert!(!expired, "the reclaim must restart the 24-hour window");

    Ok(())
}

/// `release` hands an in-flight key back, and refuses to touch a completed
/// one.
///
/// The first half is what makes a `5xx` retryable under the same key
/// (`vpay_api`'s `PostRequest::finish`): the row has to be *gone*, so the
/// next claim takes the ordinary insert path. The second half is the safety
/// on it — a release that deleted a `complete` row would throw away a
/// response the merchant is entitled to replay, and the retry would
/// re-execute a request that already succeeded. Zero rows deleted is the
/// correct, non-erroring answer there.
#[tokio::test]
async fn release_hands_back_an_in_flight_key_and_never_a_completed_one() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let claim = |key: &'static str| {
        let pool = pool.clone();
        async move {
            vpay_db::idempotency::claim(
                &pool,
                "merchant_a",
                key,
                "POST",
                "/v1/payment_intents",
                &fixture_hash(0x33),
            )
            .await
        }
    };

    let failed = fresh_claim_id(&claim("key-failed").await?)?;
    assert_eq!(
        vpay_db::idempotency::release(&pool, "merchant_a", "key-failed", failed).await?,
        1,
        "releasing an in-flight key must delete exactly its row"
    );
    let reclaimed = claim("key-failed").await?;
    assert!(
        matches!(reclaimed, vpay_db::IdempotencyClaim::Fresh { .. }),
        "a released key must be claimable again, or the merchant's retry is told `in progress` \
         for 24 hours: got {reclaimed:?}"
    );

    // A key that completed: release must not touch it.
    let done = fresh_claim_id(&claim("key-done").await?)?;
    let body = json!({"id": "pi_stored"});
    vpay_db::idempotency::store(&pool, "merchant_a", "key-done", done, 200, &body)
        .await
        .context("storing the response must succeed")?;
    assert_eq!(
        vpay_db::idempotency::release(&pool, "merchant_a", "key-done", done).await?,
        0,
        "a completed key is not in flight and must survive a release"
    );
    match claim("key-done").await? {
        vpay_db::IdempotencyClaim::Replay(record) => {
            assert_eq!(record.response_status, Some(200));
            assert_eq!(record.response_body, Some(body));
        }
        other => panic!("expected the stored response to still replay, got {other:?}"),
    }

    // And a key nobody ever claimed. The id is arbitrary because there is
    // no row for it to match — which is the answer being asserted.
    assert_eq!(
        vpay_db::idempotency::release(&pool, "merchant_a", "never-seen", uuid::Uuid::new_v4())
            .await?,
        0
    );

    Ok(())
}

/// The ABA a reclaimable row makes possible, and the `claim_id` that closes
/// it.
///
/// The sequence is the one a stalled request really produces: R1 claims the
/// key, takes longer than the 24-hour window (simulated by back-dating
/// `expires_at`, the same way the reclaim and sweep tests above age a row),
/// R2 reclaims it and starts doing R2's work — and *then* R1 wakes up.
///
/// Identified by `(merchant_id, idempotency_key)` and `state` alone, R1's
/// two closing calls would both hit R2's row: the `release` would delete a
/// live claim, freeing the key for a third request to run concurrently with
/// R2, and the `store` would overwrite it with R1's response, so a merchant
/// polling their key would be handed the answer to a request they had
/// already been told was superseded. Both are asserted here as *no-ops*, and
/// R2's own store is asserted to still succeed afterwards — without that
/// last part this test would also pass if `claim_id` simply broke `store`
/// for everyone.
///
/// Decisive: delete `AND claim_id = $3` from either statement in
/// `vpay_db::idempotency` and exactly one of the two halves fails.
#[tokio::test]
async fn a_reclaimed_key_is_not_writable_by_the_claim_it_replaced() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let r1 = fresh_claim_id(
        &vpay_db::idempotency::claim(
            &pool,
            "merchant_a",
            "key-aba",
            "POST",
            "/v1/payment_intents",
            &fixture_hash(0x41),
        )
        .await?,
    )?;

    sqlx::query(
        "UPDATE idempotency_keys SET expires_at = now() - INTERVAL '1 hour' \
         WHERE idempotency_key = 'key-aba'",
    )
    .execute(&pool)
    .await
    .context("ageing R1's claim past its window must succeed")?;

    let r2 = fresh_claim_id(
        &vpay_db::idempotency::claim(
            &pool,
            "merchant_a",
            "key-aba",
            "POST",
            "/v1/payment_intents",
            &fixture_hash(0x42),
        )
        .await?,
    )?;
    assert_ne!(
        r1, r2,
        "a reclaim must mint a new claim id, or the two requests are indistinguishable"
    );

    // R1 wakes up. Its release must not delete R2's claim...
    assert_eq!(
        vpay_db::idempotency::release(&pool, "merchant_a", "key-aba", r1).await?,
        0,
        "a superseded claim must delete nothing; deleting R2's row would let a third request run \
         under this key at the same time as R2"
    );

    // ...and its store must not overwrite R2's row, nor claim to have
    // stored anything.
    assert_eq!(
        vpay_db::idempotency::store(
            &pool,
            "merchant_a",
            "key-aba",
            r1,
            200,
            &json!({"id": "pi_r1", "note": "R1's answer, under R2's claim"}),
        )
        .await
        .context("a stale store must not be an error")?,
        vpay_db::IdempotencyStoreOutcome::StaleClaim,
        "a superseded claim must be told its claim is stale, never that the write succeeded"
    );

    // R2's row is untouched: still in flight, still R2's, still describing
    // R2's request.
    let (claim_id, state, response): (uuid::Uuid, String, Option<serde_json::Value>) =
        sqlx::query_as(
            "SELECT claim_id, state, response_body FROM idempotency_keys \
             WHERE merchant_id = 'merchant_a' AND idempotency_key = 'key-aba'",
        )
        .fetch_one(&pool)
        .await
        .context("re-reading the reclaimed row must succeed")?;
    assert_eq!(claim_id, r2);
    assert_eq!(state, "in_flight");
    assert_eq!(response, None);

    // And R2 can still complete normally — the guard narrows the write to
    // the live claim, it does not disable it.
    assert_eq!(
        vpay_db::idempotency::store(
            &pool,
            "merchant_a",
            "key-aba",
            r2,
            201,
            &json!({"id": "pi_r2"}),
        )
        .await
        .context("the live claim must still be able to store its response")?,
        vpay_db::IdempotencyStoreOutcome::Stored
    );
    match vpay_db::idempotency::claim(
        &pool,
        "merchant_a",
        "key-aba",
        "POST",
        "/v1/payment_intents",
        &fixture_hash(0x42),
    )
    .await?
    {
        vpay_db::IdempotencyClaim::Replay(record) => {
            assert_eq!(record.response_status, Some(201));
            assert_eq!(
                record.response_body,
                Some(json!({"id": "pi_r2"})),
                "the replayable answer must be R2's, never R1's"
            );
        }
        other => panic!("R2's completed key must replay R2's response, got {other:?}"),
    }

    Ok(())
}

/// The sweep deletes expired rows and *only* expired rows.
///
/// The half that matters is the second one: a sweep that took the whole
/// table would silently turn every in-flight idempotency guarantee off, and
/// the next retry of a request already running would be told `Fresh`.
#[tokio::test]
async fn sweep_expired_removes_only_the_rows_past_their_window() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    for key in ["key-old", "key-fresh"] {
        fresh_claim_id(
            &vpay_db::idempotency::claim(
                &pool,
                "merchant_a",
                key,
                "POST",
                "/v1/payment_intents",
                &fixture_hash(0x11),
            )
            .await?,
        )?;
    }

    // Age one row past its 24-hour window. Written directly because
    // `expires_at` has no setter — it is a column default, and nothing in
    // the shipping API may move it.
    sqlx::query(
        "UPDATE idempotency_keys SET expires_at = now() - INTERVAL '1 hour' \
         WHERE idempotency_key = 'key-old'",
    )
    .execute(&pool)
    .await
    .context("ageing the row must succeed")?;

    let swept = vpay_db::idempotency::sweep_expired(&pool)
        .await
        .context("the sweep must succeed")?;
    assert_eq!(swept, 1, "exactly the expired row may be deleted");

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT idempotency_key FROM idempotency_keys ORDER BY idempotency_key")
            .fetch_all(&pool)
            .await
            .context("listing the surviving keys must succeed")?;
    assert_eq!(remaining, vec!["key-fresh".to_owned()]);

    // A second sweep with nothing expired must delete nothing.
    assert_eq!(vpay_db::idempotency::sweep_expired(&pool).await?, 0);

    Ok(())
}

/// Cursor paging in both directions over 25 rows, with `has_more` correct
/// on the last page of each — D8's semantics end to end.
///
/// The two properties worth stating: `data` is newest-first *whichever*
/// cursor was used (so `ending_before` scans ascending and is reversed in
/// Rust — drop the reverse and the backward page comes back upside down),
/// and `ending_before` from the newest id of the last forward page returns
/// exactly the page before it, which is what makes "previous page" a real
/// operation rather than an approximation.
#[tokio::test]
async fn list_page_walks_forward_and_backward_over_twenty_five_intents() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    // Inserted oldest-first, so `page_fixture_id(24)` is the newest and every
    // page below is expressed against that order. The ids are computed
    // rather than collected so the expectations below can be written
    // directly, without indexing a Vec (which `clippy::indexing_slicing`
    // denies, tests included).
    for n in 0..25 {
        let row =
            vpay_db::payment_intents::insert(&pool, &fixture_intent(&page_fixture_id(n), "XAF"))
                .await
                .context("inserting a page fixture must succeed")?;
        assert_eq!(row.id, page_fixture_id(n));
    }
    // Another merchant's intents must never appear in any page below.
    let mut other = fixture_intent("pi_other_merchant", "XAF");
    other.merchant_id = "merchant_b".to_owned();
    vpay_db::payment_intents::insert(&pool, &other).await?;

    let ids = |rows: &[vpay_db::PaymentIntentRow]| -> Vec<String> {
        rows.iter().map(|r| r.id.clone()).collect()
    };
    let newest_first = |from: usize, to: usize| -> Vec<String> {
        (from..=to).rev().map(page_fixture_id).collect()
    };

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: None,
    };
    let (first, has_more) = vpay_db::payment_intents::list_page(&pool, "merchant_a", &page).await?;
    assert_eq!(
        ids(&first),
        newest_first(15, 24),
        "the default page is the newest 10"
    );
    assert!(has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some(page_fixture_id(15)),
        ending_before: None,
    };
    let (second, has_more) =
        vpay_db::payment_intents::list_page(&pool, "merchant_a", &page).await?;
    assert_eq!(ids(&second), newest_first(5, 14));
    assert!(has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some(page_fixture_id(5)),
        ending_before: None,
    };
    let (third, has_more) = vpay_db::payment_intents::list_page(&pool, "merchant_a", &page).await?;
    assert_eq!(ids(&third), newest_first(0, 4));
    assert!(
        !has_more,
        "the last forward page must report that nothing follows it"
    );

    // Backward from the newest id of the last page: exactly the page before
    // it, still newest-first.
    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: Some(page_fixture_id(4)),
    };
    let (back, has_more) = vpay_db::payment_intents::list_page(&pool, "merchant_a", &page).await?;
    assert_eq!(
        ids(&back),
        ids(&second),
        "ending_before must return the page before, in the same newest-first order the envelope \
         always promises"
    );
    assert!(has_more, "there are still 5 newer intents beyond this page");

    // Backward far enough that fewer than a full page remains: `has_more`
    // must be false, and the short page must still be newest-first.
    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: Some(page_fixture_id(15)),
    };
    let (tail, has_more) = vpay_db::payment_intents::list_page(&pool, "merchant_a", &page).await?;
    assert_eq!(ids(&tail), newest_first(16, 24));
    assert!(
        !has_more,
        "9 rows for a limit of 10 is the end of the range in this direction"
    );

    // An unknown cursor resolves to NULL and yields an empty page — never a
    // silent fallback to the newest rows, and never another merchant's.
    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some("pi_does_not_exist".to_owned()),
        ending_before: None,
    };
    let (none, has_more) = vpay_db::payment_intents::list_page(&pool, "merchant_a", &page).await?;
    assert!(none.is_empty() && !has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: None,
    };
    let (others, _) = vpay_db::payment_intents::list_page(&pool, "merchant_b", &page).await?;
    assert_eq!(
        ids(&others),
        vec!["pi_other_merchant".to_owned()],
        "every page is merchant-scoped in SQL"
    );

    Ok(())
}

/// The `providers` rows that matter to reconciliation, in a stable order —
/// a plain `async fn` rather than a closure so the borrow of `pool` is tied
/// to the call rather than to a captured environment.
async fn provider_snapshot(pool: &PgPool) -> anyhow::Result<Vec<(String, String, bool, bool)>> {
    let rows: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT code, flow::TEXT, supports_refunds, enabled FROM providers ORDER BY code",
    )
    .fetch_all(pool)
    .await
    .context("reading the providers table must succeed")?;
    Ok(rows)
}

/// Boot step 4 run twice must be a no-op the second time, and a provider
/// code dropped from the seed must be **disabled, not deleted**.
///
/// Deleting would break every historical `charges` row pointing at that
/// rail (and the foreign key would refuse), so "a rail that has ever taken
/// money stays nameable forever" is the invariant this pins. The exponent
/// refusal at the end is the other half: a currency's exponent is not a
/// per-deployment setting, and quietly upserting a new one would
/// reinterpret every amount already stored.
#[tokio::test]
async fn reconcile_is_idempotent_and_disables_a_dropped_provider_code() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let currencies = vec![
        vpay_db::CurrencySeed {
            code: "XAF".to_owned(),
            exponent: 0,
        },
        vpay_db::CurrencySeed {
            code: "EUR".to_owned(),
            exponent: 2,
        },
    ];
    let mtn = vpay_db::ProviderSeed {
        code: "mtn_momo".to_owned(),
        display_name: "MTN MoMo".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: false,
        delivers_callbacks: true,
        requires_ip_allowlist: false,
        enabled: true,
    };
    let orange = vpay_db::ProviderSeed {
        code: "orange_money".to_owned(),
        display_name: "Orange Money".to_owned(),
        flow: "redirect".to_owned(),
        supports_refunds: true,
        supports_partial_refunds: true,
        delivers_callbacks: false,
        requires_ip_allowlist: true,
        enabled: true,
    };

    vpay_db::config_reconcile::reconcile(&pool, &currencies, &[mtn.clone(), orange.clone()])
        .await
        .context("the first reconcile must succeed")?;

    let after_first = provider_snapshot(&pool)
        .await
        .context("reading providers must succeed")?;
    assert_eq!(
        after_first,
        vec![
            ("mtn_momo".to_owned(), "push".to_owned(), false, true),
            ("orange_money".to_owned(), "redirect".to_owned(), true, true),
        ]
    );

    vpay_db::config_reconcile::reconcile(&pool, &currencies, &[mtn.clone(), orange.clone()])
        .await
        .context("a second, identical reconcile must succeed")?;
    assert_eq!(
        provider_snapshot(&pool)
            .await
            .context("re-reading providers must succeed")?,
        after_first,
        "every replica runs boot step 4, so a repeat must be observably a no-op"
    );
    let currency_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM currencies")
        .fetch_one(&pool)
        .await
        .context("counting currencies must succeed")?;
    assert_eq!(
        currency_rows, 2,
        "a repeat must not duplicate reference data"
    );

    // The rail is removed from the deployment's configuration.
    vpay_db::config_reconcile::reconcile(&pool, &currencies, std::slice::from_ref(&mtn))
        .await
        .context("reconciling a shortened provider list must succeed")?;
    assert_eq!(
        provider_snapshot(&pool)
            .await
            .context("reading providers must succeed")?,
        vec![
            ("mtn_momo".to_owned(), "push".to_owned(), false, true),
            (
                "orange_money".to_owned(),
                "redirect".to_owned(),
                true,
                false
            ),
        ],
        "a dropped provider code must be disabled and KEPT — deleting it would orphan every \
         charge that ever used it"
    );

    // A currency whose exponent disagrees with the database is refused, and
    // the refusal rolls the whole transaction back.
    let wrong = vec![vpay_db::CurrencySeed {
        code: "XAF".to_owned(),
        exponent: 2,
    }];
    let error = vpay_db::config_reconcile::reconcile(&pool, &wrong, std::slice::from_ref(&mtn))
        .await
        .expect_err("changing a stored currency exponent must be refused at boot");
    match &error {
        vpay_db::DbError::CurrencyExponentConflict {
            code,
            stored,
            seeded,
        } => {
            assert_eq!(code, "XAF");
            assert_eq!(*stored, 0);
            assert_eq!(*seeded, 2);
        }
        other => panic!("expected a named exponent conflict, got {other:?}"),
    }
    assert_eq!(
        vpay_core::Classify::category(&error).exit_code(),
        78,
        "a misconfigured deployment must exit 78 (fix the deploy), never 69 (wait for Postgres)"
    );
    let stored_exponent: i32 =
        sqlx::query_scalar("SELECT exponent FROM currencies WHERE code = 'XAF'")
            .fetch_one(&pool)
            .await
            .context("re-reading the exponent must succeed")?;
    assert_eq!(
        stored_exponent, 0,
        "the refused transaction must have rolled back whole"
    );

    Ok(())
}

/// The two rails every reconcile test seeds, in the shape `boot_seeds`
/// produces them. A free function rather than a `const` because
/// `ProviderSeed` owns its `String`s.
fn reconcile_rails() -> (vpay_db::ProviderSeed, vpay_db::ProviderSeed) {
    (
        vpay_db::ProviderSeed {
            code: "mtn_momo".to_owned(),
            display_name: "Mtn Momo".to_owned(),
            flow: "push".to_owned(),
            supports_refunds: false,
            supports_partial_refunds: false,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
            enabled: true,
        },
        vpay_db::ProviderSeed {
            code: "orange_money".to_owned(),
            display_name: "Orange Money".to_owned(),
            flow: "redirect".to_owned(),
            supports_refunds: true,
            supports_partial_refunds: true,
            delivers_callbacks: false,
            requires_ip_allowlist: true,
            enabled: true,
        },
    )
}

/// `reconcile` takes `lock_keys::CONFIG_RECONCILE` before it touches a row,
/// and does not proceed while someone else holds it.
///
/// **This is the test that fails if the `pg_advisory_xact_lock` is removed
/// from `reconcile`.** Everything else about boot step 4 — idempotence, the
/// disable pass, the exponent refusal — passes with or without the lock,
/// because none of those observe another writer. Here the other writer is
/// this test: it holds the lock on its own transaction and asserts that a
/// `reconcile` started underneath it makes no progress at all, then that the
/// same call completes once the lock is released. Without the lock the first
/// assertion fails immediately (the reconcile finishes while the lock is
/// held), which is exactly the concurrency this is meant to pin.
///
/// The `pool` carries ten connections (`vpay_db::connect`), so holding one
/// for the blocker leaves the spawned reconcile a connection of its own —
/// what it waits on is the lock, not the pool.
#[tokio::test]
async fn reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released() -> anyhow::Result<()>
{
    use std::time::Duration;

    let (_container, pool) = migrated_postgres().await?;
    let (mtn, orange) = reconcile_rails();
    let currencies = vec![vpay_db::CurrencySeed {
        code: "XAF".to_owned(),
        exponent: 0,
    }];

    // A competing boot step 4, stopped at its first statement: the lock is
    // taken on a transaction this test controls the lifetime of.
    let mut blocker = pool
        .begin()
        .await
        .context("the blocker transaction begins")?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(vpay_db::lock_keys::CONFIG_RECONCILE)
        .execute(&mut *blocker)
        .await
        .context("taking the reconcile lock must succeed")?;

    let mut reconciling = tokio::spawn({
        let pool = pool.clone();
        let providers = vec![mtn.clone(), orange.clone()];
        let currencies = currencies.clone();
        async move { vpay_db::config_reconcile::reconcile(&pool, &currencies, &providers).await }
    });

    // Long enough that a reconcile which did not wait would have finished
    // several times over (the whole transaction is four statements against
    // a local container), short enough not to dominate the suite. `&mut` on
    // the handle so the same task can be awaited again after the release.
    let finished_while_locked = tokio::time::timeout(Duration::from_secs(3), &mut reconciling)
        .await
        .is_ok();
    assert!(
        !finished_while_locked,
        "reconcile completed while another transaction held \
         lock_keys::CONFIG_RECONCILE — boot step 4 is not taking the advisory lock, and two \
         replicas booting together can interleave their upserts"
    );

    // Nothing was written, either: the lock is taken *before* the first
    // upsert, not after it.
    let providers_seen: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&pool)
        .await
        .context("counting providers must succeed")?;
    assert_eq!(
        providers_seen, 0,
        "the lock must be the transaction's FIRST statement; rows appeared while it was held"
    );

    // Release it, and the blocked reconcile proceeds.
    blocker
        .rollback()
        .await
        .context("releasing the lock must succeed")?;
    tokio::time::timeout(Duration::from_secs(30), reconciling)
        .await
        .context("reconcile must proceed once the lock is released")?
        .context("the reconcile task must not panic")?
        .context("the reconcile itself must succeed")?;

    assert_eq!(
        provider_snapshot(&pool)
            .await
            .context("reading providers must succeed")?,
        vec![
            ("mtn_momo".to_owned(), "push".to_owned(), false, true),
            ("orange_money".to_owned(), "redirect".to_owned(), true, true),
        ],
        "the reconcile that waited must have done its whole job afterwards"
    );

    Ok(())
}

/// Two boot step 4s running at once, with the rails listed in **opposite**
/// orders, both succeed and leave the same rows.
///
/// The order is the point. Two deployments of the same config with the
/// `providers:` list formatted differently — or a rollout where the YAML was
/// reordered — take the same `providers` row locks in opposite orders, which
/// is the shape Postgres resolves by aborting one transaction with `40P01`
/// and the binary turns into a failed boot. `reconcile` closes it twice
/// over: it sorts its seeds by `code`, and it serialises on
/// `lock_keys::CONFIG_RECONCILE`. Both are load-bearing and neither is
/// sufficient alone — see that constant's doc comment.
#[tokio::test]
async fn two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    let (mtn, orange) = reconcile_rails();

    // Also in opposite orders, and with a currency each list mentions, so
    // the `currencies` upserts race the same way the `providers` ones do.
    let forward_currencies = vec![
        vpay_db::CurrencySeed {
            code: "EUR".to_owned(),
            exponent: 2,
        },
        vpay_db::CurrencySeed {
            code: "XAF".to_owned(),
            exponent: 0,
        },
    ];
    let mut reverse_currencies = forward_currencies.clone();
    reverse_currencies.reverse();

    let forward = vec![mtn.clone(), orange.clone()];
    let reverse = vec![orange.clone(), mtn.clone()];

    let (left, right) = tokio::join!(
        vpay_db::config_reconcile::reconcile(&pool, &forward_currencies, &forward),
        vpay_db::config_reconcile::reconcile(&pool, &reverse_currencies, &reverse),
    );
    left.context("the forward-ordered reconcile must succeed")?;
    right.context("the reverse-ordered reconcile must succeed")?;

    assert_eq!(
        provider_snapshot(&pool)
            .await
            .context("reading providers must succeed")?,
        vec![
            ("mtn_momo".to_owned(), "push".to_owned(), false, true),
            ("orange_money".to_owned(), "redirect".to_owned(), true, true),
        ],
        "the two orders must converge on identical rows"
    );
    let currency_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM currencies")
        .fetch_one(&pool)
        .await
        .context("counting currencies must succeed")?;
    assert_eq!(
        currency_rows, 2,
        "two concurrent reconciles must not duplicate reference data"
    );

    Ok(())
}

/// The write-before-network trail: an attempt is recorded *pending*, and an
/// attempt that never got an HTTP status stays `status_code IS NULL` /
/// `responded_at IS NULL` — which is exactly the row the documented `501`
/// from a not-implemented `submit` leaves behind on purpose.
///
/// The `response_is_paired` CHECK is exercised directly at the end. It is
/// what makes "unanswered attempt" a trustworthy query: a row carrying a
/// `responded_at` but no status would look answered to a human and be
/// invisible to the sweep that is supposed to find it.
#[tokio::test]
async fn provider_requests_record_attempts_and_keep_status_and_responded_at_in_step()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_attempts", "XAF")).await?;
    let mut tx = pool.begin().await.context("transaction begins")?;
    let charge =
        vpay_db::charges::insert_for_intent(&mut tx, &fixture_charge("ch_attempts", "pi_attempts"))
            .await
            .context("opening the charge must succeed")?;
    tx.commit().await.context("transaction commits")?;

    let first = vpay_db::provider_requests::insert_pending(
        &pool,
        &charge.id,
        "mtn_momo",
        "submit",
        charge.provider_reference_id,
        1,
    )
    .await
    .context("recording the pending attempt must succeed")?;

    let pending: (Option<i32>, Option<time::OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT status_code, responded_at, error_kind FROM provider_requests WHERE id = $1",
    )
    .bind(first)
    .fetch_one(&pool)
    .await
    .context("reading the pending attempt must succeed")?;
    assert_eq!(
        pending,
        (None, None, None),
        "the row is written before the call, so it starts unanswered"
    );

    // The not-implemented adapter: a failure with no HTTP status at all.
    vpay_db::provider_requests::record_response(&pool, first, None, Some("not_implemented"))
        .await
        .context("recording a status-less failure must succeed")?;
    let after: (Option<i32>, Option<time::OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT status_code, responded_at, error_kind FROM provider_requests WHERE id = $1",
    )
    .bind(first)
    .fetch_one(&pool)
    .await
    .context("re-reading the attempt must succeed")?;
    assert_eq!(
        (after.0, after.1),
        (None, None),
        "an attempt that got no HTTP status did not receive a response, and must not be stamped \
         as though it had — that is the row a recovery sweep looks for"
    );
    assert_eq!(after.2.as_deref(), Some("not_implemented"));

    // A second attempt for the same charge: this table is one row per
    // attempt, never one per charge.
    let second = vpay_db::provider_requests::insert_pending(
        &pool,
        &charge.id,
        "mtn_momo",
        "query_status",
        charge.provider_reference_id,
        2,
    )
    .await
    .context("a second attempt for the same charge must be accepted")?;
    assert_ne!(first, second);

    vpay_db::provider_requests::record_response(&pool, second, Some(200), None)
        .await
        .context("recording a real response must succeed")?;
    let answered: (Option<i32>, Option<time::OffsetDateTime>) =
        sqlx::query_as("SELECT status_code, responded_at FROM provider_requests WHERE id = $1")
            .bind(second)
            .fetch_one(&pool)
            .await
            .context("reading the answered attempt must succeed")?;
    assert_eq!(answered.0, Some(200));
    assert!(
        answered.1.is_some(),
        "an attempt with a status must carry the instant it was answered"
    );

    // Recording against an id that does not exist is vpay's invariant
    // breaking, not a merchant's mistake.
    let missing = vpay_db::provider_requests::record_response(&pool, 9_999_999, Some(200), None)
        .await
        .expect_err("completing an attempt that was never opened must be refused");
    assert_eq!(
        vpay_core::Classify::category(&missing),
        vpay_core::Category::Internal
    );

    // And the database itself refuses to let the two facts drift apart —
    // proven by writing what the repository refuses to write.
    let violation = sqlx::query(
        "UPDATE provider_requests SET responded_at = now() WHERE id = $1 AND status_code IS NULL",
    )
    .bind(first)
    .execute(&pool)
    .await
    .expect_err("a responded_at with no status_code must be rejected by response_is_paired");
    assert_eq!(
        violation
            .as_database_error()
            .and_then(|e| e.constraint())
            .unwrap_or_default(),
        "response_is_paired"
    );

    Ok(())
}

/// The write a rail's acceptance produces, and the guard that makes it a
/// state machine rather than a hope.
///
/// Three claims, and the third is the one worth the container: a
/// `mark_submitted` against a charge that is *no longer* `submitting`
/// matches nothing and says so, instead of dragging a settled charge back
/// into a live state. That is the collision a recovery pass (Step 4) will
/// actually create — it and a slow confirm can both hold the same charge —
/// and a blind `UPDATE … WHERE id = $1` would lose it silently.
#[tokio::test]
async fn a_submitted_charge_records_the_rails_material_and_only_from_submitting()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_submitted", "XAF")).await?;
    let mut tx = pool.begin().await.context("transaction begins")?;
    let mut new = fixture_charge("ch_submitted", "pi_submitted");
    // The merchant's own return destination, written before the rail is
    // called (migration 0019).
    new.return_url = Some("https://shop.example/order/1234/return".to_owned());
    let charge = vpay_db::charges::insert_for_intent(&mut tx, &new)
        .await
        .context("opening the charge must succeed")?;
    tx.commit().await.context("transaction commits")?;
    assert_eq!(
        charge.return_url.as_deref(),
        Some("https://shop.example/order/1234/return"),
        "the return_url is durable before anything is submitted"
    );
    assert_eq!(charge.redirect_url, None, "no rail has answered yet");

    let ref_extra = json!({ "pay_token": "tok_abc" });
    let mut tx = pool.begin().await.context("transaction begins")?;
    let submitted = vpay_db::charges::mark_submitted(
        &mut tx,
        &charge.id,
        "submitted",
        Some(&ref_extra),
        Some("https://webpayment.example/pay/tok_abc"),
    )
    .await
    .context("recording the rail's answer must succeed")?;
    tx.commit().await.context("transaction commits")?;

    assert_eq!(submitted.state, "submitted");
    assert_eq!(submitted.provider_ref_extra, Some(ref_extra));
    assert_eq!(
        submitted.redirect_url.as_deref(),
        Some("https://webpayment.example/pay/tok_abc")
    );
    assert_eq!(
        submitted.return_url, charge.return_url,
        "a rail's answer must not touch the merchant's return_url"
    );
    assert!(
        submitted.updated_at >= charge.updated_at,
        "the row records that it moved"
    );

    // Second time round the charge is no longer `submitting`, and the guard
    // is in the statement.
    let mut tx = pool.begin().await.context("transaction begins")?;
    let refused = vpay_db::charges::mark_submitted(
        &mut tx,
        &charge.id,
        "submitted",
        Some(&json!({ "pay_token": "tok_second" })),
        Some("https://webpayment.example/pay/tok_second"),
    )
    .await;
    tx.rollback().await.context("transaction rolls back")?;
    assert!(
        matches!(
            refused,
            Err(vpay_db::DbError::WriteMatchedNoRow {
                table: "charges",
                ..
            })
        ),
        "a charge that has left `submitting` must not be moved back into it, and the refusal \
         must name itself: got {refused:?}"
    );

    let unchanged: (String, Option<String>) =
        sqlx::query_as("SELECT state::TEXT, redirect_url FROM charges WHERE id = $1")
            .bind(&charge.id)
            .fetch_one(&pool)
            .await
            .context("re-reading the charge must succeed")?;
    assert_eq!(
        unchanged.1.as_deref(),
        Some("https://webpayment.example/pay/tok_abc"),
        "the refused write changed nothing"
    );

    Ok(())
}

/// A decline at submit, as both rows record it: the charge is terminal and
/// carries the taxonomy plus the rail's own words, and the intent keeps the
/// status it never left while gaining `last_payment_error`.
///
/// `docs/flows/payment-lifecycle.md` has no `failed` intent status, so the
/// pair below *is* what "the payment failed" means to a merchant — which is
/// why the `lpe_paired` CHECK and the `failure_code` enum are both exercised
/// here rather than assumed.
#[tokio::test]
async fn a_declined_charge_is_terminal_and_the_intent_keeps_its_status() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_declined", "XAF")).await?;
    let mut tx = pool.begin().await.context("transaction begins")?;
    let charge =
        vpay_db::charges::insert_for_intent(&mut tx, &fixture_charge("ch_declined", "pi_declined"))
            .await
            .context("opening the charge must succeed")?;
    tx.commit().await.context("transaction commits")?;

    let mut tx = pool.begin().await.context("transaction begins")?;
    let failed = vpay_db::charges::mark_failed(
        &mut tx,
        &charge.id,
        "insufficient_funds",
        "NOT_ENOUGH_FUNDS",
    )
    .await
    .context("failing the charge must succeed")?;
    let intent = vpay_db::payment_intents::record_payment_error(
        &mut tx,
        "merchant_a",
        "pi_declined",
        "requires_payment_method",
        "insufficient_funds",
        "The payment was declined (insufficient_funds).",
    )
    .await
    .context("recording the payment error must succeed")?
    .context("the intent is still requires_payment_method, so the guard must have matched")?;
    tx.commit().await.context("transaction commits")?;

    assert_eq!(failed.state, "failed");
    assert_eq!(failed.failure_code.as_deref(), Some("insufficient_funds"));
    assert_eq!(
        failed.failure_raw.as_deref(),
        Some("NOT_ENOUGH_FUNDS"),
        "the rail's own words survive for whoever fixes the mapping table"
    );
    assert_eq!(
        intent.status, "requires_payment_method",
        "a declined charge does not move the intent — there is no failed status"
    );
    assert_eq!(
        intent.last_payment_error_code.as_deref(),
        Some("insufficient_funds")
    );
    assert_eq!(
        intent.last_payment_error_message.as_deref(),
        Some("The payment was declined (insufficient_funds)."),
        "the merchant-facing sentence, never the rail's raw string"
    );

    // The status is a guard here too: an intent that has moved on does not
    // get a payment error stamped onto it.
    let mut tx = pool.begin().await.context("transaction begins")?;
    let stale = vpay_db::payment_intents::record_payment_error(
        &mut tx,
        "merchant_a",
        "pi_declined",
        "processing",
        "payer_timeout",
        "The payment was declined (payer_timeout).",
    )
    .await
    .context("a guarded write that matches nothing is not an error")?;
    tx.commit().await.context("transaction commits")?;
    assert!(
        stale.is_none(),
        "the intent is not `processing`, so the write must match nothing"
    );

    // A foreign merchant cannot stamp an error onto someone else's intent.
    let mut tx = pool.begin().await.context("transaction begins")?;
    let foreign = vpay_db::payment_intents::record_payment_error(
        &mut tx,
        "merchant_b",
        "pi_declined",
        "requires_payment_method",
        "payer_timeout",
        "The payment was declined (payer_timeout).",
    )
    .await
    .context("a tenancy-scoped write that matches nothing is not an error")?;
    tx.commit().await.context("transaction commits")?;
    assert!(foreign.is_none(), "every write in this module is scoped");

    Ok(())
}

/// The two rows a confirm's success moves, moved in **one** transaction: a
/// rollback leaves neither, so no reader can see an intent in
/// `requires_action` whose charge carries no redirect URL.
///
/// This is the database half of `docs/flows/crash-safety.md`'s "the commit
/// is the gate on the redirect". The API half — that the response is built
/// only after the commit — is
/// `redirect_confirm_commits_the_rails_material_before_it_answers` in
/// `backends/tests/integration/tests/confirm_rails.rs`.
#[tokio::test]
async fn the_charge_and_the_intent_move_together_or_not_at_all() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_reference_data(&pool).await?;

    vpay_db::payment_intents::insert(&pool, &fixture_intent("pi_atomic", "XAF")).await?;
    let mut tx = pool.begin().await.context("transaction begins")?;
    let charge =
        vpay_db::charges::insert_for_intent(&mut tx, &fixture_charge("ch_atomic", "pi_atomic"))
            .await
            .context("opening the charge must succeed")?;
    tx.commit().await.context("transaction commits")?;

    let mut tx = pool.begin().await.context("transaction begins")?;
    vpay_db::charges::mark_submitted(
        &mut tx,
        &charge.id,
        "submitted",
        Some(&json!({ "pay_token": "tok_rolled_back" })),
        Some("https://webpayment.example/pay/tok_rolled_back"),
    )
    .await
    .context("the charge update must succeed inside the transaction")?;
    vpay_db::payment_intents::transition_in_tx(
        &mut tx,
        "merchant_a",
        "pi_atomic",
        "requires_payment_method",
        "requires_action",
    )
    .await
    .context("the intent transition must succeed inside the transaction")?
    .context("the guard must have matched")?;
    // The crash: everything above is discarded.
    tx.rollback().await.context("transaction rolls back")?;

    let (state, redirect_url): (String, Option<String>) =
        sqlx::query_as("SELECT state::TEXT, redirect_url FROM charges WHERE id = $1")
            .bind(&charge.id)
            .fetch_one(&pool)
            .await
            .context("re-reading the charge must succeed")?;
    assert_eq!(
        state, "submitting",
        "the charge is back where a recovery pass expects to find it"
    );
    assert_eq!(redirect_url, None, "no URL survived the rollback");

    let status: String =
        sqlx::query_scalar("SELECT status::TEXT FROM payment_intents WHERE id = $1")
            .bind("pi_atomic")
            .fetch_one(&pool)
            .await
            .context("re-reading the intent must succeed")?;
    assert_eq!(
        status, "requires_payment_method",
        "an intent must never be left in requires_action with no URL to send the payer to"
    );

    Ok(())
}
