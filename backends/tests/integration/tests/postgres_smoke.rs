//! Integration tests against a real Postgres via testcontainers.
//!
//! Every test spins up its own `postgres:16-alpine` container (cached
//! locally on this machine — see the note below), runs every migration under
//! `backends/migrations` against it with `sqlx::migrate!`, then asserts
//! against the live database. No mock, fake, or in-memory substitute is used
//! anywhere in this file (ADR-0006 forbids that for a database boundary in
//! any case).
//!
//! `testcontainers-modules` 0.15 defaults to image `postgres:11-alpine`,
//! which is not cached on this machine and this machine cannot reach Docker
//! Hub to pull it. `16-alpine` IS cached locally, and it is also the more
//! correct choice regardless: `compose.yml` runs Postgres 16, so testing
//! against 11 would itself be a version mismatch. Hence the explicit
//! `.with_tag("16-alpine")` below.
//!
//! Helper functions here return `anyhow::Result` and propagate with `?`
//! rather than `.expect`/`.unwrap`, matching the workspace lint policy:
//! `expect_used`/`unwrap_used`/`panic` are only exempted *inside* a
//! `#[test]`-attributed function body (`clippy.toml`), not in a plain helper
//! a test happens to call.

use anyhow::Context;
use sqlx::{PgPool, Row};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;

/// Starts a fresh, migrated Postgres 16 container and returns a pool bound to
/// it. The returned container guard must be kept alive for as long as the
/// pool is used — dropping it stops the container.
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

    let pool = PgPool::connect(&url)
        .await
        .context("connects to the freshly started container")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("every migration under backends/migrations applies cleanly")?;

    Ok((container, pool))
}

/// Inserts the two reference currencies vpay_core::Currency models (XAF, EUR)
/// — see backends/crates/vpay-core/src/money.rs.
async fn seed_currencies(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('XAF', 0), ('EUR', 2)")
        .execute(pool)
        .await
        .context("seeding currencies")?;
    Ok(())
}

/// Inserts the two real adapters' declared `Capabilities`, verbatim from
/// backends/crates/vpay-adapter-mtn-momo/src/lib.rs and
/// backends/crates/vpay-adapter-orange-money/src/lib.rs, so FK-dependent
/// fixtures below have a coherent `providers` row to point at.
async fn seed_providers(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds, delivers_callbacks, requires_ip_allowlist) \
         VALUES \
            ('mtn_momo', 'MTN MoMo', 'push'::provider_flow, true, true, true, true), \
            ('orange_money', 'Orange Money', 'redirect'::provider_flow, false, false, true, false)",
    )
    .execute(pool)
    .await
    .context("seeding providers")?;
    Ok(())
}

async fn insert_payment_intent(
    pool: &PgPool,
    id: &str,
    amount: i64,
    amount_refunded: i64,
    amount_refund_pending: i64,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO payment_intents \
            (id, merchant_id, livemode, amount, amount_refunded, amount_refund_pending, currency_code, status, payment_method_types) \
         VALUES ($1, 'merchant_1', false, $2, $3, $4, 'XAF', 'requires_payment_method'::intent_status, '[]'::jsonb)",
    )
    .bind(id)
    .bind(amount)
    .bind(amount_refunded)
    .bind(amount_refund_pending)
    .execute(pool)
    .await
}

async fn insert_charge(
    pool: &PgPool,
    id: &str,
    payment_intent_id: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO charges \
            (id, payment_intent_id, provider_code, provider_reference_id, state, amount, currency_code) \
         VALUES ($1, $2, 'mtn_momo', $3, 'submitting'::charge_state, 5000, 'XAF')",
    )
    .bind(id)
    .bind(payment_intent_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
}

/// MVP requirement #1 (docs/status.md): the schema must migrate cleanly on an
/// empty database. This also exercises every `CREATE TYPE`/`CREATE
/// TABLE`/`CREATE INDEX`/`CHECK` statement in `backends/migrations` at once —
/// a syntax error anywhere in the five migration files fails this test.
#[tokio::test]
async fn schema_migrates_cleanly_on_an_empty_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    // The migrator's own bookkeeping table is the simplest proof every
    // migration was recorded as applied, not merely that `.run()` returned
    // `Ok` without actually running anything.
    let applied: i64 = sqlx::query("SELECT COUNT(*) AS n FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .context("querying sqlx's own migration bookkeeping table")?
        .get("n");
    assert_eq!(
        applied, 8,
        "all eight migrations under backends/migrations should be recorded as applied"
    );

    // And the tables they create are genuinely queryable.
    for table in [
        "currencies",
        "providers",
        "payment_intents",
        "charges",
        "ledger_transactions",
        "ledger_entries",
        "authkestra.oauth_clients",
        "authkestra.oauth_codes",
        "authkestra.oauth_refresh_tokens",
        "authkestra.oauth_device_codes",
        "oauth_signing_keys",
        "merchant_api_keys",
    ] {
        sqlx::query(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .with_context(|| format!("table {table} should exist and be queryable"))?;
    }

    Ok(())
}

/// AGENTS.md: "One charge per intent, forever. Enforced by a plain unique
/// index." — `one_charge_per_intent` in
/// `backends/migrations/0004_create-charges.sql`.
#[tokio::test]
async fn one_charge_per_intent_is_enforced_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_currencies(&pool).await?;
    seed_providers(&pool).await?;
    insert_payment_intent(&pool, "pi_one_charge", 5_000, 0, 0)
        .await
        .context("seeding the payment intent")?;

    insert_charge(&pool, "ch_first", "pi_one_charge")
        .await
        .context("the first charge on this intent must succeed")?;

    let err = insert_charge(&pool, "ch_second", "pi_one_charge")
        .await
        .expect_err("a second charge on the same payment_intent_id must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("one_charge_per_intent"),
        "the rejection must come from the one_charge_per_intent unique index specifically, not some other constraint"
    );

    Ok(())
}

/// `Provider.supports_partial_refunds ⇒ Provider.supports_refunds`.
///
/// `schemas/vpay.cstack`'s GAP comment on `Provider` says CrateStack's grammar
/// cannot express this as a CHECK. Raw SQL can — see the
/// `partial_refunds_imply_refunds` constraint in
/// `backends/migrations/0002_create-providers.sql`. This test proves it
/// actually rejects an incoherent row, not merely that the SQL parses.
#[tokio::test]
async fn partial_refunds_without_refunds_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds) \
         VALUES ('incoherent_provider', 'Incoherent Provider', 'push'::provider_flow, false, true)",
    )
    .execute(&pool)
    .await
    .expect_err("supports_partial_refunds=true with supports_refunds=false must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("partial_refunds_imply_refunds"),
        "the rejection must come from the coherence CHECK specifically"
    );

    Ok(())
}

/// `amount_refunded + amount_refund_pending <= amount` on `PaymentIntent`.
///
/// `docs/flows/ledger.md` was corrected to say no database constraint
/// provides this; `no_over_refund` in
/// `backends/migrations/0003_create-payment-intents.sql` now does. This test
/// proves it fires.
#[tokio::test]
async fn over_refund_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_currencies(&pool).await?;

    // amount 1000, refunded 600 + pending 500 = 1100 > 1000.
    let err = insert_payment_intent(&pool, "pi_over_refund", 1_000, 600, 500)
        .await
        .expect_err("amount_refunded + amount_refund_pending > amount must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("no_over_refund"),
        "the rejection must come from the over-refund CHECK specifically"
    );

    Ok(())
}

/// Non-negative amounts: `Money::new` rejects a negative amount in Rust
/// (`docs/flows/money.md` invariant 1); the database must reject one too,
/// independent of whatever validated it on the way in.
#[tokio::test]
async fn negative_amount_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_currencies(&pool).await?;

    let err = insert_payment_intent(&pool, "pi_negative", -100, 0, 0)
        .await
        .expect_err("a negative amount must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("amount_non_negative"),
        "the rejection must come from the non-negative-amount CHECK specifically"
    );

    Ok(())
}

/// FK integrity: a charge cannot reference a payment intent that does not
/// exist. Cheap to prove and it is one of the invariants the task explicitly
/// calls out ("Non-negative amounts, currency exponent sanity, and FK
/// integrity throughout").
#[tokio::test]
async fn a_charge_referencing_a_nonexistent_payment_intent_is_rejected_by_the_database()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_currencies(&pool).await?;
    seed_providers(&pool).await?;

    let err = insert_charge(&pool, "ch_orphan", "pi_does_not_exist")
        .await
        .expect_err("a charge referencing a nonexistent payment_intent_id must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("charges_payment_intent_id_fkey"),
        "the rejection must come from the payment_intent_id foreign key specifically"
    );

    Ok(())
}

/// Currency exponent sanity: `docs/flows/money.md` — the exponent is a
/// property of the currency (XAF=0, EUR=2); the schema bounds it to a
/// plausible range (0..=4) rather than accepting an arbitrary integer that
/// would silently corrupt `Money::to_provider_string`'s output.
#[tokio::test]
async fn an_out_of_range_currency_exponent_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('JPY', 5)")
        .execute(&pool)
        .await
        .expect_err("an exponent outside 0..=4 must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("exponent_in_range"),
        "the rejection must come from the exponent range CHECK specifically"
    );

    Ok(())
}

// --- migration 0006 (authkestra-op tables, transcribed) -------------------

/// `authkestra.oauth_codes.client_id` is `NOT NULL REFERENCES
/// authkestra.oauth_clients(client_id)` — transcribed verbatim from
/// `SqlxOpStore::migrate()` (backends/migrations/0006_create-authkestra-op-tables.sql).
/// This proves the FK actually fires on our copy, not just that it parses.
#[tokio::test]
async fn an_authkestra_oauth_code_referencing_a_nonexistent_client_is_rejected_by_the_database()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        "INSERT INTO authkestra.oauth_codes \
            (code, client_id, redirect_uri, scope, identity, expires_at) \
         VALUES ('code_orphan', 'client_does_not_exist', 'https://dash.example/callback', 'openid', '{}'::jsonb, now() + interval '60 seconds')",
    )
    .execute(&pool)
    .await
    .expect_err("a code referencing a nonexistent client_id must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("oauth_codes_client_id_fkey"),
        "the rejection must come from the oauth_codes -> oauth_clients foreign key specifically"
    );

    Ok(())
}

// --- migration 0007 (oauth_signing_keys, vpay-owned) -----------------------

async fn insert_signing_key(
    pool: &PgPool,
    id: &str,
    active: bool,
    expires_at_clause: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(&format!(
        "INSERT INTO oauth_signing_keys (id, private_key_pem, active, expires_at) \
         VALUES ($1, '-----BEGIN PRIVATE KEY-----\nMIIBVgIBADANBgkqhkiG9w0BAQEFAASCAUAw\n-----END PRIVATE KEY-----\n', $2, {expires_at_clause})"
    ))
    .bind(id)
    .bind(active)
    .execute(pool)
    .await
}

/// "At most one active key at a time" — the `one_active_signing_key` partial
/// unique index (`WHERE active`) in
/// `backends/migrations/0007_create-oauth-signing-keys.sql`. Proves a second
/// active key is genuinely rejected, not merely that the index was created.
#[tokio::test]
async fn only_one_active_signing_key_is_enforced_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    insert_signing_key(&pool, "key_first", true, "NULL")
        .await
        .context("the first active signing key must succeed")?;

    let err = insert_signing_key(&pool, "key_second", true, "NULL")
        .await
        .expect_err("a second active signing key must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("one_active_signing_key"),
        "the rejection must come from the one_active_signing_key partial unique index specifically"
    );

    // A second *inactive* key must be perfectly fine — the partial index
    // only constrains `active = true` rows.
    insert_signing_key(&pool, "key_retired", false, "now() + interval '30 minutes'")
        .await
        .context("an inactive (retired) key must not trip the partial unique index")?;

    Ok(())
}

/// `active_key_has_no_expiry`: an active key must not carry a scheduled
/// expiry — rotation is supposed to set `active = false` and `expires_at`
/// together, never one without the other (this migration's own
/// justification for why this is stricter than the vsms precedent, which has
/// no such CHECK).
#[tokio::test]
async fn an_active_signing_key_with_an_expiry_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = insert_signing_key(&pool, "key_bad", true, "now() + interval '30 minutes'")
        .await
        .expect_err("an active key with a non-null expires_at must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("active_key_has_no_expiry"),
        "the rejection must come from the active_key_has_no_expiry CHECK specifically"
    );

    Ok(())
}

/// `private_key_pem_looks_like_pem`: a sanity floor, not a full PEM parser
/// (Postgres CHECK is the wrong tool for that) — but it must still actually
/// reject an obviously-wrong value like a plain string.
#[tokio::test]
async fn a_non_pem_shaped_signing_key_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        "INSERT INTO oauth_signing_keys (id, private_key_pem, active) \
         VALUES ('key_not_pem', 'definitely-not-a-pem-key', true)",
    )
    .execute(&pool)
    .await
    .expect_err("a non-PEM-shaped value must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("private_key_pem_looks_like_pem"),
        "the rejection must come from the PEM-shape CHECK specifically"
    );

    Ok(())
}

// --- migration 0008 (merchant_api_keys) ------------------------------------

/// Instant, structural uniqueness on `key_digest` — a digest collision must
/// be impossible, not merely unlikely (see this migration's header comment).
#[tokio::test]
async fn a_duplicate_merchant_api_key_digest_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    let digest = "a".repeat(64);

    sqlx::query(
        "INSERT INTO merchant_api_keys (id, merchant_id, livemode, key_prefix, key_digest) \
         VALUES ('mak_first', 'merchant_1', true, 'sk_live_abc', $1)",
    )
    .bind(&digest)
    .execute(&pool)
    .await
    .context("the first key with this digest must succeed")?;

    let err = sqlx::query(
        "INSERT INTO merchant_api_keys (id, merchant_id, livemode, key_prefix, key_digest) \
         VALUES ('mak_second', 'merchant_2', true, 'sk_live_xyz', $1)",
    )
    .bind(&digest)
    .execute(&pool)
    .await
    .expect_err("a second key with the same digest must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("merchant_api_keys_key_digest_idx"),
        "the rejection must come from the key_digest uniqueness index specifically"
    );

    Ok(())
}

/// `key_digest_is_sha256_hex`: catches an accidental non-hex or
/// wrong-length write (e.g. base64, or a raw binary digest) at INSERT time.
#[tokio::test]
async fn a_malformed_merchant_api_key_digest_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        "INSERT INTO merchant_api_keys (id, merchant_id, livemode, key_prefix, key_digest) \
         VALUES ('mak_bad', 'merchant_1', true, 'sk_live_abc', 'not-a-sha256-digest')",
    )
    .execute(&pool)
    .await
    .expect_err("a malformed digest must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("key_digest_is_sha256_hex"),
        "the rejection must come from the key_digest shape CHECK specifically"
    );

    Ok(())
}
