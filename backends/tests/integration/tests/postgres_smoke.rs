//! Integration tests against a real Postgres via testcontainers.
//!
//! Every test spins up its own `postgres:16-alpine` container (cached
//! locally on this machine — see the note below), runs every migration under
//! `backends/migrations` against it with `sqlx::migrate!`, then asserts
//! against the live database. No mock, fake, or in-memory substitute is used
//! anywhere in this file (ADR-0006 forbids that for a database boundary in
//! any case).
//!
//! The container is started by
//! `vpay_testkit::containers::start_postgres_with_retry` — one helper shared
//! by every Postgres-backed suite in the workspace, which is where the
//! pinned `16-alpine` tag (`testcontainers-modules` 0.15 would otherwise
//! give us `postgres:11-alpine`, which `compose.yml` does not run) and the
//! retry on a host-port collision are explained.
//!
//! Helper functions here return `anyhow::Result` and propagate with `?`
//! rather than `.expect`/`.unwrap`, matching the workspace lint policy:
//! `expect_used`/`unwrap_used`/`panic` are only exempted *inside* a
//! `#[test]`-attributed function body (`clippy.toml`), not in a plain helper
//! a test happens to call.

use anyhow::Context;
use sqlx::{PgPool, Row};
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;

/// Starts a fresh, migrated Postgres 16 container and returns a pool bound to
/// it. The returned container guard must be kept alive for as long as the
/// pool is used — dropping it stops the container.
///
/// The container itself comes from
/// `vpay_testkit::containers::start_postgres_with_retry`, which is where the
/// pinned tag and the host-port-collision retry are documented.
async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
    let (container, pool, _url) = migrated_postgres_with_url().await?;
    Ok((container, pool))
}

/// The same container, additionally handing back the connection string it is
/// reachable on.
///
/// This exists because one test in this file drives a tool that is not linked
/// into the process — `cratestack migrate baseline` is a separate binary and
/// takes a `--database-url` — and building a second URL from the container
/// guard at that call site would be a second spelling of the credentials this
/// helper already chose. `migrated_postgres` delegates here rather than the
/// other way round so there is exactly one place that starts a container and
/// runs `backends/migrations` against it.
async fn migrated_postgres_with_url()
-> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool, String)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
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

    Ok((container, pool, url))
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
        // All EIGHT columns, including `enabled`, since migration 0033: the
        // five capability booleans have no column default any more, so a
        // statement that omits one is a `23502` rather than a row with an
        // invented capability. See that migration's operator note and
        // `a_hand_written_provider_insert_must_now_name_every_capability_column`.
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds, delivers_callbacks, requires_ip_allowlist, enabled) \
         VALUES \
            ('mtn_momo', 'MTN MoMo', 'push', true, true, true, true, true), \
            ('orange_money', 'Orange Money', 'redirect', false, false, true, false, true)",
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
        // `client_secret_suffix` is NOT NULL as of migration 0026 and has no
        // default, deliberately (see that file): a writer that can omit it is
        // a writer that can create an intent no browser can ever address. It
        // is spelled here as a literal rather than through
        // `vpay_core::ids::client_secret_suffix` because this suite's subject
        // is what *the database* enforces — the constraint under test in each
        // case below is a different one, and a generated value would make
        // these inserts depend on a Rust function agreeing with a CHECK.
        "INSERT INTO payment_intents \
            (id, merchant_id, livemode, amount, amount_refunded, amount_refund_pending, currency_code, status, payment_method_types, client_secret_suffix) \
         VALUES ($1, 'merchant_1', false, $2, $3, $4, 'XAF', 'requires_payment_method'::intent_status, '[]'::jsonb, \
                 replace(gen_random_uuid()::text, '-', ''))",
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
/// a syntax error anywhere in the migration files fails this test.
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
        applied, 33,
        "all thirty-three migrations under backends/migrations should be recorded as applied \
         (0001-0008 plus 0009 drop merchant_api_keys, 0010 reshape oauth_signing_keys, \
         0011 oauth_client_assertion_jtis, 0012 disabled_clients, \
         0013 add-authkestra-op-0-7-columns, Step 2's 0014 payment-intent API fields, \
         0015 idempotency_keys, 0016 provider_requests, 0017 refunds, 0018 events, \
         Step 3's 0019 charges.return_url and 0020 provider_requests.status_code comment, \
         Step 4's 0021 jobs + charges.provider_txn_id, \
         and Step 5's 0022 webhook_deliveries + the reopened jobs.kind_is_known, \
         0023 jobs.kind_is_known reopened again for scan_deliveries, \
         0024 events.fanout_attempts + the 'failed' fanout_state, \
         Step 5b's 0025 idempotency_keys.response_retry, the column that lets a \
         replayed response re-emit the stripe-should-retry the original carried, \
         Step 5c's 0026 payment_intents.client_secret_suffix, the payer credential \
         /v1/browser authenticates with, and Step 8's 0027 \
         charges_provider_reference_idx, which keeps the unauthenticated \
         POST /provider/{{code}}/callback lookup off a sequential scan, \
         Step 9's 0028 checkout_sessions, the hosted/embedded checkout \
         object with its two payer credentials and the partial unique index \
         that is what actually enforces one open session per intent, \
         0029 the checkout.session.expired event type, \
         0030 checkout_sessions_intent_seq_idx, which keeps the confirm \
         path's find_latest_by_intent off a sequential scan for the same \
         reason 0027 keeps the rail callback off one, \
         and issue #46's 0031 refunds.fee, the nullable column behind the \
         refund object's tenth field — nullable so that 'the rail reported \
         no fee' stays distinguishable from 'the movement was free', \
         and 0032, which brings `currencies` and `providers` into the shape \
         schemas/vpay.cstack projects: exponent INT -> BIGINT, the two \
         hand-named currencies CHECKs renamed to the generator's \
         <table>_<column>_<validator>_check spelling, and the FIRST of \
         vpay's seven native enums -- providers.flow -- converted to TEXT \
         plus providers_flow_enum_check, because CrateStack's generated \
         row decoders read an enum column with try_get::<String>() and a \
         native enum column therefore fails to decode on every read, \
         and 0033, which drops the column DEFAULTs on providers' five \
         capability booleans so that CrateStack's generated upsert input \
         can carry them -- the maintainer's D7, 2026-09-06)"
    );

    // And the tables they create are genuinely queryable. merchant_api_keys
    // is deliberately absent: 0009 drops it (the abandoned API-key design),
    // and if that DROP had silently failed this migration run itself would
    // have failed already, so there is nothing further to assert about its
    // absence here.
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
        // Migration 0013's own table. **Added 2026-09-05 by the review of the
        // branch that deleted `authkestra_op_smoke.rs`**: that file's
        // `sqlx_op_store_records_a_dpop_jti_once_against_migration_0013s_table`
        // was the only test in the repository that named this table, and it
        // went with the `SqlxOpStore` it drove. What it proved beyond
        // existence — that the store's `INSERT … ON CONFLICT` matches this
        // DDL — is genuinely gone with the store, and `docs/status.md` says
        // so; that the table 0013 creates is *there* is a claim worth keeping
        // checked, and is all that survives being checkable.
        "authkestra.oauth_dpop_jti",
        "oauth_signing_keys",
        "oauth_client_assertion_jtis",
        "disabled_clients",
        // Step 2's five. `refunds` and `events` have no reader or writer yet
        // (`docs/status.md`) — the schema landing ahead of the code is
        // deliberate, and listing them here is what keeps "the table exists"
        // from being an unchecked claim.
        "idempotency_keys",
        "provider_requests",
        "refunds",
        "events",
        // Step 9's. Unlike `refunds` and `events` above, this one has both a
        // reader and a writer from the day it landed
        // (`vpay_db::checkout_sessions`, `vpay_api::v1::checkout_sessions`),
        // so listing it here proves the table the code queries is the table
        // the migration creates.
        "checkout_sessions",
    ] {
        // sqlx 0.9 (sqlx#3723) wants the injection audit written down. The
        // only interpolation is `table`, bound by the `for` above to one of
        // the string literals in that list — an identifier cannot be a bind
        // parameter, which is why this is a `format!` at all.
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(&pool)
            .await
            .with_context(|| format!("table {table} should exist and be queryable"))?;
    }

    // Migration 0013's three *columns*, for the same reason as its table
    // above: `authkestra_op_smoke.rs` proved each of them by reading it back
    // through `SqlxOpStore`'s own SQL, and nothing has named them since that
    // file was deleted on 2026-09-05. `ALTER TABLE … ADD COLUMN` failing would
    // fail the migration run, so this is not about the DDL executing — it is
    // about a later migration dropping or renaming one of them without a test
    // anywhere going red, which is exactly the state 0013's header comment
    // says must not happen ("DO NOT EDIT THIS DDL INDEPENDENTLY OF THE PINNED
    // `authkestra-op` VERSION").
    for (table, column) in [
        ("oauth_refresh_tokens", "jkt"),
        ("oauth_clients", "token_endpoint_auth_method"),
        ("oauth_clients", "jwks"),
    ] {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = 'authkestra' AND table_name = $1 AND column_name = $2)",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&pool)
        .await
        .with_context(|| format!("looking up authkestra.{table}.{column}"))?;
        assert!(
            present,
            "authkestra.{table}.{column} is one of the three columns migration 0013 adds; \
             it is unread by any code path today, but dropping or renaming it is a schema \
             change nobody has decided to make"
        );
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
        // Every column named, because migration 0033 left the five capability
        // booleans without a default: an omitted one would make this a
        // not-null violation and the assertion below would be about the
        // wrong refusal.
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds, \
             delivers_callbacks, requires_ip_allowlist, enabled) \
         VALUES ('incoherent_provider', 'Incoherent Provider', 'push', false, true, \
             false, false, true)",
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

// --- migration 0032 (the cratestack shape for currencies and providers) ----
//
// Three constraints, one of which is a *replacement* for a type that no
// longer exists. Every one of them is a rename or a re-expression of
// something migrations 0001 and 0002 already enforced, so what these tests
// are really asking is whether the enforcement survived the move — a
// renamed-away CHECK and a dropped one look identical from anywhere except
// an INSERT that ought to fail.

/// `currencies_code_iso4217_check` (migration 0032) fires, under its new
/// name and with the predicate `@iso4217` renders.
///
/// It was `code_is_iso4217_shape` until 0032. The predicate did not change —
/// `code ~ '^[A-Z]{3}$'`, which is byte-identical to what
/// `cratestack-migrate`'s `emit/postgres/checks.rs` produces for `@iso4217`
/// — only the name did, to `naming.rs::check_name`'s
/// `<table>_<column>_<validator>_check`. Asserting on `constraint()` rather
/// than merely on "the insert failed" is what makes this a test of the
/// renamed constraint and not of the primary key.
#[tokio::test]
async fn a_currency_code_that_is_not_three_uppercase_letters_is_rejected_by_the_database()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('xa1', 0)")
        .execute(&pool)
        .await
        .expect_err("a code that is not three uppercase letters must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("currencies_code_iso4217_check"),
        "the rejection must come from 0032's renamed ISO-4217 shape CHECK specifically. If this \
         says `code_is_iso4217_shape`, 0032 did not run; if it says None, the constraint was \
         dropped rather than renamed"
    );

    Ok(())
}

/// `providers_flow_enum_check` (migration 0032) refuses what the
/// `provider_flow` enum type used to refuse — and the type is gone.
///
/// This is the first of vpay's seven native Postgres enums to be converted
/// to `TEXT` + CHECK, and the conversion had to happen for a runtime reason
/// no drift report can see: `cratestack`'s generated row decoders read an
/// enum column with `try_get::<String>()` and `.parse()`, so a native enum
/// column fails to decode on every read through that layer (upstream issue
/// #228). `vpay-db`'s own
/// `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` is
/// the test that proves the read now works; this one proves the conversion
/// did not cost the *validation* the native type was providing.
///
/// The `DROP TYPE` assertion is the half that would otherwise go unnoticed:
/// a leftover `provider_flow` type with no column using it is dead weight
/// that a later reader has to prove is dead before touching anything.
#[tokio::test]
async fn an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type()
-> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        // Eight columns for migration 0033's reason (see `seed_providers`),
        // and here it is load-bearing for what this test asserts: with the
        // capability columns omitted, Postgres would refuse the row for a
        // missing `supports_refunds` and the constraint assertion below would
        // read `None` instead of the flow CHECK.
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds, \
             delivers_callbacks, requires_ip_allowlist, enabled) \
         VALUES ('typo_rail', 'Typo Rail', 'redirekt', false, false, false, false, true)",
    )
    .execute(&pool)
    .await
    .expect_err("a flow that is neither push nor redirect must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("providers_flow_enum_check"),
        "the rejection must come from 0032's membership CHECK. Before 0032 this insert failed \
         with an invalid-input-value error carrying NO constraint name, because the native enum \
         type refused it at cast time"
    );

    // Both legal values still are.
    for flow in ["push", "redirect"] {
        sqlx::query(
            "INSERT INTO providers \
                (code, display_name, flow, supports_refunds, supports_partial_refunds, \
                 delivers_callbacks, requires_ip_allowlist, enabled) \
             VALUES ($1, $2, $3, false, false, false, false, true)",
        )
        .bind(format!("ok_{flow}"))
        .bind("Fine")
        .bind(flow)
        .execute(&pool)
        .await
        .context("push and redirect must both still be accepted")?;
    }

    // The type itself is gone. `to_regtype` returns NULL for a type that
    // does not exist, which is a cheaper and less brittle question than
    // parsing `pg_type`.
    let leftover: Option<String> = sqlx::query_scalar("SELECT to_regtype('provider_flow')::text")
        .fetch_one(&pool)
        .await
        .context("asking whether the provider_flow type still exists must succeed")?;
    assert_eq!(
        leftover, None,
        "migration 0032 drops the now-unused `provider_flow` enum type"
    );

    Ok(())
}

// --- migration 0033 (the providers capability defaults) --------------------

/// After migration 0033, a hand-written `INSERT` that names only the three
/// identity columns is **refused**, where it used to succeed and invent five
/// capabilities.
///
/// This is the whole cost of the maintainer's D7 (2026-09-06), asserted
/// rather than described. `cratestack-macros` drops every `@default(...)`
/// field from `Create{Model}Input` and from `upsert_update_columns`, so the
/// five `@default(...)` on `model Provider` were what kept
/// `ConfigReconcile::reconcile`'s provider pass on hand-written SQL: the
/// generated upsert could write `code`, `display_name` and `flow` and nothing
/// else. Dropping the five `@default(...)` alone puts five
/// `column ... default value differs` lines in the drift report, so the
/// schema half and this DDL half are one change.
///
/// What is being traded away is a convenience for a writer that is not
/// `reconcile` — and `reconcile` is the only writer of these columns, always
/// writes all five, and cannot be helped by a default. What is bought is that
/// nobody else can half-fill a row: before this migration,
/// `INSERT INTO providers (code, display_name, flow) VALUES (...)` produced a
/// rail recorded as not refunding, not delivering callbacks, and **enabled**,
/// with no error and nothing to notice. The refusal below is the honest
/// version of that.
///
/// Reverting the migration turns this test red at test time (not at compile
/// time — nothing in Rust names a column default), together with
/// `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`, whose
/// count would go 84 -> 89.
#[tokio::test]
async fn a_hand_written_provider_insert_must_now_name_every_capability_column() -> anyhow::Result<()>
{
    let (_container, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        "INSERT INTO providers (code, display_name, flow) \
         VALUES ('half_filled_rail', 'Half Filled Rail', 'push')",
    )
    .execute(&pool)
    .await
    .expect_err(
        "with no column defaults, an INSERT that names only the three identity columns must be \
         refused rather than silently completed",
    );

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.code().as_deref(),
        Some("23502"),
        "a not-null violation is the refusal migration 0033 creates. Any other SQLSTATE means \
         something else refused this row first"
    );

    // Which column is named matters: `enabled` had `DEFAULT TRUE`, so before
    // 0033 this row would have been created OPEN FOR CHARGES. The four
    // `DEFAULT FALSE` columns are checked through the catalog below rather
    // than by four more inserts, because Postgres reports only the first
    // not-null violation it meets.
    assert!(
        db_err.message().contains("supports_refunds"),
        "the first missing column Postgres meets is `supports_refunds`: {db_err}"
    );

    // Every one of the five, read from the catalog, so a migration that
    // dropped four defaults and missed one is a failure here and not a
    // surprise in production. `pg_attrdef` holds a row only for a column that
    // HAS a default, so the assertion is that none of the five is in it.
    let still_defaulted: Vec<String> = sqlx::query_scalar(
        "SELECT a.attname::text FROM pg_attribute a \
         JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
         WHERE a.attrelid = 'providers'::regclass \
         ORDER BY a.attname",
    )
    .fetch_all(&pool)
    .await
    .context("reading providers' column defaults must succeed")?;
    assert!(
        still_defaulted.is_empty(),
        "migration 0033 drops the column default on all five capability booleans; these still \
         have one: {still_defaulted:?}"
    );

    // And the row a caller names in full is still accepted, so what changed
    // is what may be OMITTED and not what may be written.
    sqlx::query(
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds, \
             delivers_callbacks, requires_ip_allowlist, enabled) \
         VALUES ('fully_named_rail', 'Fully Named Rail', 'push', false, false, false, false, \
             false)",
    )
    .execute(&pool)
    .await
    .context("an INSERT naming all eight columns must still succeed")?;

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
///
/// The constraint was `exponent_in_range` with the predicate `exponent
/// BETWEEN 0 AND 4` until migration 0032 renamed it to
/// `currencies_exponent_range_check` and re-spelled the predicate as
/// `exponent >= 0 AND exponent <= 4` — the same accepted set, written the
/// way `@range(min: 0, max: 4)` renders it, under the name
/// `naming.rs::check_name` generates. The value 5 rather than something huge
/// is deliberate: the *range* has to be what refuses it, not the column
/// width, which 0032 widened at the same time.
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
        Some("currencies_exponent_range_check"),
        "the rejection must come from the exponent range CHECK specifically. If this says \
         `exponent_in_range`, migration 0032 did not run; if it says None, the rename dropped \
         the constraint instead of replacing it"
    );

    // The column 0032 also widened, asserted here rather than in a test of
    // its own because a widening that silently dropped the CHECK above is
    // exactly the failure worth catching in one place. `cratestack`'s `Int`
    // always emits `int8` and its introspector deliberately refuses to map
    // `int4` back onto it, so an `int4` here is not a narrower `Int` — it is
    // a column `migrate baseline` cannot compare at all, which is what the
    // `EXPECTED_UNMAPPABLE_COLUMNS` 18 -> 17 move below records.
    let data_type: String = sqlx::query_scalar(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_name = 'currencies' AND column_name = 'exponent'",
    )
    .fetch_one(&pool)
    .await
    .context("reading the exponent column's type must succeed")?;
    assert_eq!(
        data_type, "bigint",
        "migration 0032 widens currencies.exponent to BIGINT"
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

// --- migration 0007 + 0010 (oauth_signing_keys, vpay-owned, reshaped) ------

/// A minimal but shape-plausible public JWK, matching what
/// `authkestra_engine::token::jwk::Jwk` actually derives (`kty`/`alg`/`n`/`e`/
/// `kid`) — see migration 0010's header comment. `oauth_signing_keys.
/// public_jwk` has no shape CHECK (unlike the dropped `private_key_pem`
/// column), so any JSON object would satisfy `NOT NULL`; this fixture is
/// realistic rather than minimal-to-pass, so these tests exercise the column
/// the way real code will fill it.
const FIXTURE_PUBLIC_JWK: &str =
    r#"{"kty":"RSA","alg":"RS256","kid":"test-kid","n":"vGb-fixture-n","e":"AQAB"}"#;

async fn insert_signing_key(
    pool: &PgPool,
    kid: &str,
    active: bool,
    expires_at_clause: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    // sqlx 0.9 (sqlx#3723) wants the injection audit written down.
    // `expires_at_clause` is a *SQL expression*, not a value — `NULL` or
    // `now() + interval '…'` — which is exactly why it cannot be a bind
    // parameter. Every one of the four call sites below passes a literal
    // written in this file; nothing here reads a value from outside the test.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO oauth_signing_keys (kid, public_jwk, active, expires_at) \
         VALUES ($1, $2::jsonb, $3, {expires_at_clause})"
    )))
    .bind(kid)
    .bind(FIXTURE_PUBLIC_JWK)
    .bind(active)
    .execute(pool)
    .await
}

/// "At most one active key at a time" — the `one_active_signing_key` partial
/// unique index (`WHERE active`) in
/// `backends/migrations/0007_create-oauth-signing-keys.sql`, carried forward
/// untouched by migration 0010's reshape. Proves a second active key is
/// genuinely rejected, not merely that the index was created — and that it
/// still fires after `id`/`private_key_pem` became `kid`/`public_jwk`.
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
/// together, never one without the other. Carried forward untouched by
/// migration 0010's reshape; this proves it still fires afterward.
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

// --- migration 0011 (oauth_client_assertion_jtis) --------------------------

/// The `jti` primary key is the atomic single-use guard for `private_key_jwt`
/// replay protection (migration 0011's header comment). A plain duplicate
/// INSERT must be rejected by the database.
#[tokio::test]
async fn a_duplicate_client_assertion_jti_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    sqlx::query(
        "INSERT INTO oauth_client_assertion_jtis (jti, expires_at) \
         VALUES ('jti_first', now() + interval '5 minutes')",
    )
    .execute(&pool)
    .await
    .context("the first insert of this jti must succeed")?;

    let err = sqlx::query(
        "INSERT INTO oauth_client_assertion_jtis (jti, expires_at) \
         VALUES ('jti_first', now() + interval '5 minutes')",
    )
    .execute(&pool)
    .await
    .expect_err("a duplicate jti must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("oauth_client_assertion_jtis_pkey"),
        "the rejection must come from the jti primary key specifically"
    );

    Ok(())
}

/// The Rust side is specified to use `INSERT ... ON CONFLICT (jti) DO
/// NOTHING` and read `rows_affected()` rather than check-then-insert
/// (migration 0011's header comment — a TOCTOU race would defeat the whole
/// point of replay protection). Proves that pattern actually reports 1 row
/// affected on first use and 0 on a replay, rather than erroring or silently
/// reporting 1 both times.
#[tokio::test]
async fn on_conflict_do_nothing_reports_zero_rows_affected_for_a_replayed_jti() -> anyhow::Result<()>
{
    let (_container, pool) = migrated_postgres().await?;

    let first = sqlx::query(
        "INSERT INTO oauth_client_assertion_jtis (jti, expires_at) \
         VALUES ('jti_conflict', now() + interval '5 minutes') \
         ON CONFLICT (jti) DO NOTHING",
    )
    .execute(&pool)
    .await
    .context("the first ON CONFLICT DO NOTHING insert must succeed")?;
    assert_eq!(
        first.rows_affected(),
        1,
        "the first presentation of a jti must report exactly 1 row affected (accept)"
    );

    let replay = sqlx::query(
        "INSERT INTO oauth_client_assertion_jtis (jti, expires_at) \
         VALUES ('jti_conflict', now() + interval '5 minutes') \
         ON CONFLICT (jti) DO NOTHING",
    )
    .execute(&pool)
    .await
    .context("a replayed ON CONFLICT DO NOTHING insert must still succeed as a statement")?;
    assert_eq!(
        replay.rows_affected(),
        0,
        "a replayed jti must report exactly 0 rows affected (reject) rather than erroring"
    );

    Ok(())
}

// --- migration 0012 (disabled_clients) -------------------------------------

/// The basic kill-switch write path: an operator disabling a client_id must
/// simply succeed.
#[tokio::test]
async fn disabled_clients_accepts_an_insert() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    sqlx::query(
        "INSERT INTO disabled_clients (client_id, reason) \
         VALUES ('merchant_compromised', 'key compromised, ticket INC-123')",
    )
    .execute(&pool)
    .await
    .context("disabling a client_id must succeed")?;

    Ok(())
}

/// `client_id` is the primary key: a client can only be disabled once (a
/// second disable attempt for the same client_id must be rejected, not
/// silently accepted as a second row) — the operator-facing "disable" action
/// should be idempotent at the application layer, not double-insert at the
/// database layer.
#[tokio::test]
async fn a_duplicate_disabled_client_id_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    sqlx::query("INSERT INTO disabled_clients (client_id) VALUES ('merchant_dupe')")
        .execute(&pool)
        .await
        .context("the first disable of this client_id must succeed")?;

    let err = sqlx::query("INSERT INTO disabled_clients (client_id) VALUES ('merchant_dupe')")
        .execute(&pool)
        .await
        .expect_err("a duplicate client_id must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("disabled_clients_pkey"),
        "the rejection must come from the client_id primary key specifically"
    );

    Ok(())
}

// --- migration 0030 (checkout_sessions_intent_seq_idx) ---------------------

/// The confirm path's session lookup is served by an index, and not by a scan
/// of `checkout_sessions`.
///
/// `vpay_db::CheckoutSessions::find_latest_by_intent` — "the newest session on
/// this intent, whatever its `status`" — is asked once per confirm by
/// `vpay_api::v1::return_trip`, **including** for the majority of confirms
/// that have no checkout session at all. 0028 gave this table only a
/// *partial* lookup by intent (`WHERE status = 'open'`), which this query
/// cannot use, because dropping that predicate is the entire point of it: it
/// has to tell "no session was ever created" from "the session that was
/// created is over". Without 0030 the planner's only choices are a sequential
/// scan of the table or a full backward scan of `checkout_sessions_seq_key`,
/// and the case with no matching row — the common one — never stops early.
///
/// Measured on this image with 200,000 sessions and an intent that has none:
/// a parallel sequential scan removing 200,000 rows, 11.7 ms, against
/// 0.047 ms through this index.
///
/// # Why the plan and not only the index
///
/// `pg_indexes` alone would pass for an index of the wrong shape — one that
/// carried a `WHERE`, or that led on `seq` — so the plan is asserted too.
/// `enable_seqscan = off` is what makes that assertion meaningful on an empty
/// table: the planner would otherwise pick a sequential scan over any index
/// here whatever the schema says, and what is being pinned is that *an index
/// can serve this query at all*. Delete 0030 and the same query plans as an
/// `Index Scan Backward using checkout_sessions_seq_key` with
/// `payment_intent_id` demoted to a filter — which is the defect, and which
/// this test then fails on.
#[tokio::test]
async fn the_confirm_paths_session_lookup_is_served_by_an_index() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;

    let definition: Option<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE indexname = 'checkout_sessions_intent_seq_idx'",
    )
    .fetch_optional(&pool)
    .await
    .context("reading checkout_sessions_intent_seq_idx must succeed")?;
    let definition = definition.context("0030 must create checkout_sessions_intent_seq_idx")?;
    assert!(
        definition.contains("(payment_intent_id, seq DESC)"),
        "the index must lead on payment_intent_id and carry seq DESC, or `ORDER BY seq DESC \
         LIMIT 1` is a sort rather than a first entry: {definition}"
    );
    assert!(
        !definition.contains(" WHERE "),
        "a partial index is what created this gap — 0028's checkout_sessions_open_by_intent_idx \
         is unusable here precisely because it carries a predicate the query does not: \
         {definition}"
    );

    // One connection for both statements: `SET` is session-scoped, and a
    // pool hands the `EXPLAIN` a different connection otherwise.
    let mut connection = pool
        .acquire()
        .await
        .context("taking one connection for the SET and the EXPLAIN")?;
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *connection)
        .await
        .context("disabling sequential scans for this session must succeed")?;

    // The query `find_latest_by_intent` builds, in shape.
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT id FROM checkout_sessions \
         WHERE payment_intent_id = 'pi_00000000000000000000000001' \
         ORDER BY seq DESC LIMIT 1",
    )
    .fetch_all(&mut *connection)
    .await
    .context("explaining the confirm path's session lookup must succeed")?;
    let plan = plan.join("\n");
    assert!(
        plan.contains("checkout_sessions_intent_seq_idx"),
        "the lookup must be servable by 0030's index; without it the planner falls back to a \
         full backward scan of checkout_sessions_seq_key with payment_intent_id demoted to a \
         filter, which is the defect this migration exists for: {plan}"
    );

    Ok(())
}

// -------------------------------------------- schemas/vpay.cstack drift ----
//
// Everything below measures one thing: how far `schemas/vpay.cstack` is from
// the database `backends/migrations/*.sql` actually builds. Until this test
// landed, that distance was a paragraph in `docs/status.md` written from
// reading both files — "content remains a design sketch" — and nothing ran
// that could have contradicted it.
//
// The tool is `cratestack migrate baseline --strict`
// (https://cratestack.dev/tooling/migrate-baseline). Its normal job is
// adoption: introspect a live database, report how it differs from the
// schema, write a snapshot from what it found. `--strict` inverts the exit
// condition — "Fail (non-zero exit, no writes) if any drift is found … For
// teams that want baselining to double as a 'prove the schema already
// matches' CI gate" — which is what makes it usable as a measurement here.
// vpay is not adopting anything: `backends/migrations/*.sql` is the
// authoritative schema and the `.cstack` file is excluded from the build
// graph (`docs/status.md`, "CrateStack"). What this test wants is the
// report, and `--strict`'s promise that producing it changed nothing.

/// Pending drift changes between `schemas/vpay.cstack` and a freshly migrated
/// vpay database, as counted by `cratestack migrate baseline --strict`.
///
/// **Measured, not chosen**, against `cratestack-cli 0.11.1` and
/// `postgres:16-alpine` — `docs/plans/exp13-notes/opus.md` has the full
/// transcript for the original 2026-09-05 measurement. **This number must
/// move when the schema grows.** It is the size of the gap between a design
/// sketch and the migrations that outran it, so a commit that closes part of
/// that gap and leaves this constant alone has changed the schema without
/// measuring the change; the test failing is the intended way to find that
/// out.
///
/// **86 -> 85 on 2026-09-06**, when `schemas/vpay.cstack` gained
/// `model DisabledClient` and `vpay-db` started reading `disabled_clients`
/// through CrateStack. One `table … is not declared in the schema` line went
/// away and nothing replaced it, so `disabled_clients` left the report
/// entirely and `EXPECTED_DRIFTED_RELATIONS` fell with it. That the count
/// moved at all is a fact about the *default*, and it was the second thing
/// tried: `@default(dbgenerated())` on `disabled_at` swaps the missing-table
/// line for a `column disabled_at default value differs` line and leaves the
/// total at exactly 86 — a whole table entering the schema, invisible here.
/// `@default(now())` compares equal to the live `now()` and is why this
/// number moved (`docs/plans/exp14-notes/opus.md`). The set assertion below
/// is what caught the first spelling; keep both.
///
/// **Unmoved by migration 0031 on 2026-09-06**, re-measured with this test on
/// the tree rebased onto issue #46. A stationary number here can mean either
/// "the schema did not change" or "the report stopped looking", so the reason
/// is recorded rather than assumed: `0031_refunds-fee.sql` adds `refunds.fee`
/// to a table `schemas/vpay.cstack` does not declare at all, and an undeclared
/// table contributes exactly one `table ... is not declared in the schema`
/// line whatever its column count. The schema therefore grew by a column and
/// this total did not move. `refunds.fee` is `numeric`, which cratestack maps,
/// so it did not enter the unmappable block either.
///
/// **85 -> 84 on 2026-09-06**, by migration 0032, and *which* of that
/// migration's three changes moved it is the finding worth carrying, because
/// two of the three moved nothing:
///
///   * `currencies.exponent` `INT` -> `BIGINT` is the whole of the -1. The
///     column was in the trailing "could not confidently map" block, so the
///     schema's declaration of it read as `column exponent is declared in
///     the schema but does not exist in the live database`. `Int` emits
///     `int8` and the introspector refuses to map `int4` back onto it
///     deliberately, so an `INT` column is not a narrower `Int` — it is a
///     column the comparison cannot see. `EXPECTED_UNMAPPABLE_COLUMNS` falls
///     by one for the same reason, which is the direction that matters: the
///     count fell because the report is comparing *more*, not less.
///   * Renaming the two hand-named `currencies` CHECKs to the generator's
///     `<table>_<column>_<validator>_check` spelling moved **nothing**.
///     `diff/checks.rs` matches by name first and then compares kinds, and
///     introspection reports every validator-derived CHECK as
///     `CheckKind::Raw(<deparsed text>)` — it reconstructs only
///     `CheckKind::Enum`, and never `Iso4217` or `Range`
///     (`ir/checks.rs`: "Rather than guess which validator (if any) produced
///     it, introspection always reports it as opaque text"). So two
///     unrelated lines became a same-named drop-and-add pair: a clearer
///     report, the same number. Not a reason to revert the rename — the
///     names are the half that *can* converge at 0.11.1 — but a reason not
///     to expect a validator rename to move this constant.
///   * Converting `providers.flow` from the native `provider_flow` enum to
///     `TEXT` + `providers_flow_enum_check` also moved **nothing**, and
///     `providers` reports exactly the same four lines it did before.
///     `introspect/postgres/enums.rs` already synthesised that CHECK from
///     `pg_enum` for the native column, and `resolve_column` projects a
///     native enum and a TEXT column onto the same `Scalar("String")`. The
///     conversion is real and load-bearing — it is what lets `cratestack`
///     decode the column at all — and this report is structurally blind to
///     it, exactly as it is blind to the ten multi-column CHECKs. See
///     `docs/plans/exp17-notes/opus.md`.
///
/// It is deliberately not, and must never become, `0`. A zero here would not
/// mean the schema had caught up — it would mean the report stopped finding
/// things, which is the failure mode `--strict` is easiest to misread as
/// success in.
const EXPECTED_DRIFT_CHANGES: u32 = 84;

/// Tables and views the drift above is spread across. Reported on the same
/// header line as the change count and pinned for the same reason: 85 changes
/// concentrated in three relations and 85 spread over sixteen are different
/// facts about the schema, and only one of them is true.
///
/// **17 -> 16 on 2026-09-06**: `disabled_clients` is now modelled exactly and
/// contributes no change at all, so it is not merely a table whose drift
/// shrank — it is off the list.
///
/// **Still 16 after migration 0031 (2026-09-06):** the column 0031 adds
/// lands on `refunds`, which was already on this list as an undeclared table
/// and stays exactly one entry on it.
///
/// **Still 16 after migration 0032 (2026-09-06):** `currencies` lost one of
/// its five lines and `providers` lost none, so both tables are still on the
/// list. This is the assertion that says so: a -1 in the change count with
/// this number unmoved means a line went away, not a table.
const EXPECTED_DRIFTED_RELATIONS: u32 = 16;

/// Live columns `cratestack` declines to compare because it cannot map their
/// Postgres type onto a `.cstack` scalar, which it reports as a trailing
/// "review manually" block.
///
/// Pinned because it is a *blind spot in the measurement itself*, not part of
/// it: these columns are excluded from the 86 above, so the drift on them —
/// whatever it is — is unmeasured. Every one is a `jsonb`, an `int2`/`int4`
/// or a `bytea`. If this number grows, the report is comparing less than it
/// was, and `EXPECTED_DRIFT_CHANGES` can fall for a reason that has nothing
/// to do with the schema improving.
///
/// **Still 18 after migration 0031 (2026-09-06):** `refunds.fee` is `numeric`,
/// which cratestack maps onto a `.cstack` scalar, so it is compared rather
/// than excluded. The 18 still include `refunds.metadata` (`jsonb`).
///
/// **18 -> 17 on 2026-09-06**, by migration 0032: `currencies.exponent` was
/// the one `int4` in this block that a migration could remove, and it is
/// `BIGINT` now. This is the *good* direction for this constant and the
/// reason `EXPECTED_DRIFT_CHANGES` may fall alongside it — the comparison
/// grew by a column and the drift it found on that column was zero. Every
/// remaining entry is a `jsonb`, a `bytea`, or an `int2`/`int4` on a table
/// `schemas/vpay.cstack` does not model at all; `jsonb` and `bytea` do not
/// round-trip at 0.11.1 (they are emitted but not read back), so this number
/// cannot reach zero by schema work alone.
const EXPECTED_UNMAPPABLE_COLUMNS: u32 = 17;

/// The `--out-dir` handed to `migrate baseline`, removed when it goes out of
/// scope.
///
/// A `remove_dir_all` as the test's last statement is skipped by every `?`
/// and every failed assertion above it — which is precisely the set of runs
/// that leave something worth not leaving behind. Measured on 2026-09-05
/// while reviewing this test: seven orphaned `vpay-cstack-baseline-*`
/// directories had accumulated in `$TMPDIR` over one afternoon's mutation
/// runs, and the one from a run with `--strict` removed still held the
/// snapshot the tool had written — the failure case where the leaked
/// directory is not empty is exactly the one the old cleanup could not
/// reach.
///
/// Same shape and same reason as `worker_kill9.rs`'s `Workspace`; this file
/// is a separate test binary, so the type is repeated rather than shared.
struct OutDir(std::path::PathBuf);

impl Drop for OutDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Absolute path to the repository root, derived from this test crate's own
/// manifest directory (`backends/tests/integration`) rather than from the
/// process's working directory, which `cargo nextest` does not promise.
fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .context("resolving the repository root from CARGO_MANIFEST_DIR")
}

/// The `cratestack` version string on `PATH`, e.g. `"0.11.1"`.
///
/// # Errors
///
/// Fails if the binary is absent. That is deliberate and matches
/// `just check-schema`, whose own comment gives the reason: a check that
/// downgrades itself to a skip "reports success for a run in which nothing
/// was checked, in a log indistinguishable from one in which everything
/// passed". There is no `#[ignore]` and no early `return Ok(())` below.
fn cratestack_version() -> anyhow::Result<String> {
    let output = std::process::Command::new("cratestack")
        .arg("--version")
        .output()
        .context(
            "the `cratestack` CLI must be on PATH for this test — it is a red failure, not a \
             skip, exactly as `just check-schema` treats the same absence. Install the pinned \
             release with `cargo install cratestack-cli --locked --version 0.11.1` (the version \
             pin lives in `justfile` as `cratestack_version`)",
        )?;
    anyhow::ensure!(
        output.status.success(),
        "`cratestack --version` exited {}",
        output.status
    );
    // "cratestack <semver>"
    let stdout = String::from_utf8(output.stdout).context("`cratestack --version` prints UTF-8")?;
    let version = stdout
        .split_whitespace()
        .nth(1)
        .context("`cratestack --version` should print `cratestack <semver>`")?;
    Ok(version.to_owned())
}

/// The `cratestack_version` the `justfile` pins, read out of the `justfile`.
///
/// The banner below used to name `0.11.1` as a literal beside the version it
/// had actually run. That is a claim about a file this test can read, and a
/// literal keeps printing `0.11.1` after the pin moves — a false sentence in
/// the log of the one test whose entire subject is where a number came from.
///
/// # Errors
///
/// Fails if the `justfile` stops declaring the pin, rather than falling back
/// to a default: a silent default is how the banner would go stale a second
/// time.
fn pinned_cratestack_version(root: &std::path::Path) -> anyhow::Result<String> {
    let justfile =
        std::fs::read_to_string(root.join("justfile")).context("reading the justfile")?;
    let pin = justfile
        .lines()
        .find_map(|line| line.strip_prefix("cratestack_version := "))
        .context("`justfile` should declare `cratestack_version := \"<semver>\"` at column 0")?;
    Ok(pin.trim().trim_matches('"').to_owned())
}

/// Parses `migrate baseline`'s header — `drift detected in 17
/// table(s)/view(s) (86 change(s) total):` — into `(relations, changes)`.
///
/// Tokenised rather than split on `(`, because `table(s)/view(s)` and
/// `change(s)` both contain parentheses and the first draft of this function
/// read `86 change` out of the wrong one.
fn parse_drift_header(stdout: &str) -> anyhow::Result<(u32, u32)> {
    let line = stdout
        .lines()
        .find(|line| line.starts_with("drift detected in "))
        .context(
            "`migrate baseline --strict` printed no `drift detected in …` header. Either the \
             report format changed or the run found no drift at all — and no drift would mean \
             the schema now matches the migrations, which is a claim for a human to check \
             rather than for this test to accept",
        )?;
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let relations = tokens
        .iter()
        .skip_while(|token| **token != "in")
        .nth(1)
        .context("no relation count after `in` in the drift header")?
        .parse()
        .context("the relation count in the drift header is not a number")?;
    let changes = tokens
        .iter()
        .find_map(|token| token.strip_prefix('('))
        .context("no `(N change(s) total)` group in the drift header")?
        .parse()
        .context("the change count in the drift header is not a number")?;
    Ok((relations, changes))
}

/// Every table the report names as present in the live database and absent
/// from `schemas/vpay.cstack`, in the report's own words.
///
/// Keyed on the sentence, not on the `[lossy]` severity label that precedes
/// it. `Op::DropTable` is unconditionally `Destructiveness::Lossy` in
/// `cratestack-migrate` 0.11.1 (`src/ir.rs`), so pinning the label was
/// correct — but it is a second fact about the tool riding on an assertion
/// about vpay's schema, and if the label ever changed this would quietly
/// return an empty set. The test would still fail, and would fail saying the
/// schema now declares every table in the database: the wrong diagnosis, from
/// a helper that had stopped reading the report rather than a schema that had
/// grown.
fn tables_missing_from_the_schema(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let (_severity, rest) = line.trim().strip_prefix('[')?.split_once("] table `")?;
            let (name, tail) = rest.split_once('`')?;
            tail.starts_with(" exists in the live database but is not declared in the schema")
                .then(|| name.to_owned())
        })
        .collect()
}

/// `schemas/vpay.cstack` measured against the database it claims to mirror.
///
/// The schema's own header lists what it "deliberately does NOT invent shapes
/// for". This test does not take that list on trust — it measures, and the
/// measured set is **larger** than the header's: `oauth_signing_keys` and
/// `oauth_client_assertion_jtis` are lumped under "the authkestra tables" in
/// that prose, but they live in `public` rather than in the `authkestra`
/// schema, so they are two more tables the file omits and not a category it
/// already accounted for. `disabled_clients` was a third until 2026-09-06,
/// when it became the first table this schema models *and* `vpay-db` reads
/// through CrateStack.
///
/// See `docs/plans/exp13-notes/opus.md` for the full CLI transcript and
/// `docs/status.md`, "CrateStack", for what this number means.
#[tokio::test]
async fn the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount() -> anyhow::Result<()> {
    let root = repo_root()?;

    // Printed unconditionally: the grammar that answered is part of the
    // measurement, and `justfile`'s `cratestack_version` pin is reported
    // rather than enforced locally for the reason `check-schema` gives —
    // "CI installs the pin exactly and CI is the gate of record, and blocking
    // every contributor whose PATH carries a newer release is how a gate
    // acquires a local opt-out". The warning on a mismatch is the other half
    // of that recipe's rule and was missing here: measured 2026-09-05, a shim
    // printing `cratestack 9.9.9-review-fake` ran the whole measurement and
    // said nothing about it. Whether a mismatch should instead FAIL this test
    // — the pinned 86 is a number about one grammar — is a maintainer's call,
    // and this matches `check-schema` rather than taking it.
    let pinned = pinned_cratestack_version(&root)?;
    let version = cratestack_version()?;
    eprintln!("cratestack CLI under test: {version} (justfile pins {pinned})");
    if version != pinned {
        eprintln!(
            "WARNING: cratestack {version} on PATH, this repository pins {pinned}. The \
             measurement below still ran in full, but against the {version} grammar, and \
             every constant it asserts was measured against {pinned}."
        );
    }
    let schema = root.join("schemas/vpay.cstack");
    let schema_before = std::fs::read(&schema).context("reading schemas/vpay.cstack")?;

    let (_container, pool, url) = migrated_postgres_with_url().await?;

    // `--out-dir` is outside the checkout, so a `--strict` run that wrote
    // anything despite its documented "no writes" would land here and not in
    // the repository. It is created empty first, so "nothing was written" is
    // an assertion about a directory that exists rather than about one that
    // may simply never have been reached.
    let out_dir =
        OutDir(std::env::temp_dir().join(format!("vpay-cstack-baseline-{}", Uuid::new_v4())));
    std::fs::create_dir_all(&out_dir.0).context("creating the out-dir for migrate baseline")?;

    let output = std::process::Command::new("cratestack")
        .current_dir(&root)
        .args(["migrate", "baseline"])
        .arg("--schema")
        .arg(&schema)
        .arg("--database-url")
        .arg(&url)
        .arg("--out-dir")
        .arg(&out_dir.0)
        .arg("--strict")
        .output()
        .context("running `cratestack migrate baseline --strict`")?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    eprintln!("--- migrate baseline --strict stdout ---\n{stdout}");
    eprintln!("--- migrate baseline --strict stderr ---\n{stderr}");

    // `--strict` exits non-zero when it finds drift. A zero exit would mean
    // the schema and the migrations agree, which is the one outcome this test
    // must not silently accept — see EXPECTED_DRIFT_CHANGES.
    assert_eq!(
        output.status.code(),
        Some(1),
        "`migrate baseline --strict` must exit 1 on a drifted database"
    );

    let (relations, changes) = parse_drift_header(&stdout)?;
    assert_eq!(
        changes, EXPECTED_DRIFT_CHANGES,
        "the drift between schemas/vpay.cstack and backends/migrations changed: the report \
         counts {changes} pending change(s), this test pins {EXPECTED_DRIFT_CHANGES}. If the \
         schema grew, move the constant and docs/status.md's CrateStack section in the same \
         commit and say what closed. If it shrank without an edit to the schema, find out what \
         the report stopped seeing before moving anything"
    );
    assert_eq!(
        relations, EXPECTED_DRIFTED_RELATIONS,
        "the drift is spread over {relations} table(s)/view(s), this test pins \
         {EXPECTED_DRIFTED_RELATIONS}"
    );

    // The exact set, sorted, because the count above is not sufficient on its
    // own and that was measured rather than assumed: adding a `DisabledClient`
    // model for the existing `disabled_clients` table swaps one change for
    // another — the "table … is not declared" line goes away and a "column
    // `disabled_at` default value differs" line takes its place — and the
    // total stays at exactly 86. A whole table entering the schema was
    // invisible to `EXPECTED_DRIFT_CHANGES`. This assertion is what caught
    // it. (`docs/plans/exp13-notes/opus.md`, mutation 2.)
    //
    // That model landed on 2026-09-06 and `disabled_clients` is gone from the
    // list below — but with `@default(now())` rather than
    // `@default(dbgenerated())`, so the replacement line the paragraph above
    // describes never appeared and the count moved after all (86 -> 85).
    // The paragraph stays because the mutation it records is still the one
    // this assertion exists for: the *next* table modelled with a default
    // this tool renders differently will swap a line for a line, and only
    // this list will notice.
    let mut missing = tables_missing_from_the_schema(&stdout);
    missing.sort();
    assert_eq!(
        missing,
        [
            // sqlx's own bookkeeping table. Not a vpay design object and not
            // something `schemas/vpay.cstack` should ever declare — but it is
            // genuinely in the database `sqlx::migrate!` produces, so it is
            // honestly part of the drift rather than something to filter out.
            "_sqlx_migrations",
            "checkout_sessions",
            "events",
            "idempotency_keys",
            "jobs",
            // These two are `public` tables, not `authkestra` ones. The
            // schema header's "and the authkestra tables" does not cover
            // them; the `authkestra.*` tables really are invisible here,
            // because baseline introspects the connection's own schema.
            "oauth_client_assertion_jtis",
            "oauth_signing_keys",
            "provider_requests",
            "refunds",
            "webhook_deliveries",
        ],
        "the set of tables the migrations build and the schema does not declare"
    );

    // ---- the cross-column CHECKs, and what the report does with them ----
    //
    // This is the evidence for the `@@check(expr)` ask recorded in
    // `schemas/vpay.cstack`'s header and in docs/status.md, and the finding is
    // sharper than "the report names them as things the schema cannot
    // express": **it does not name them at all.** Every CHECK the report
    // lists is a single-column one. Every multi-column CHECK in the database
    // is invisible to it, in both directions.
    //
    // The claim being made here is "the report cannot see these", not "these
    // strings do not appear in the report" — and those come apart the moment
    // somebody deletes a constraint, which is the case that matters most.
    // So the live database is read first, from `pg_constraint`, and the
    // absence below is asserted only about constraints that demonstrably
    // exist. Measured 2026-09-05: ten multi-column CHECKs, zero of them
    // reported.
    //
    // `cardinality(conkey) > 1` is the complement of the tool's own filter,
    // and the tool documents it: `cratestack-migrate` 0.11.1's
    // `src/introspect/postgres/mod.rs` lists "Multi-column and zero-column
    // CHECK constraints are skipped … `AddCheck` ties to exactly one column"
    // under Known gaps, and `constraints.rs::introspect_checks` selects
    // `contype = 'c' AND array_length(c.conkey, 1) = 1`. Two consequences for
    // this query. It is scoped to `public` because that is the only schema
    // baseline introspects, so an `authkestra.*` constraint could not be
    // "reported" either way (there are none that are multi-column; measured).
    // And it does NOT cover the *zero*-column case the same upstream rule
    // skips: `cardinality(NULL) > 1` is NULL, so a `CHECK` referencing no
    // column at all would be missing from both sides of this comparison.
    // There are none on a `public` table here — the only two in the database,
    // `cardinal_number_domain_check` and `yes_or_no_check`, are
    // `information_schema` domain constraints — which is why the list stays
    // "multi-column" rather than growing a second branch for a case vpay does
    // not have.
    let multi_column_checks: Vec<(String, String)> = sqlx::query(
        "SELECT conrelid::regclass::text AS tbl, conname AS name \
         FROM pg_constraint \
         WHERE contype = 'c' AND connamespace = 'public'::regnamespace \
           AND cardinality(conkey) > 1 \
         ORDER BY tbl, name",
    )
    .fetch_all(&pool)
    .await
    .context("reading the live database's multi-column CHECK constraints")?
    .iter()
    .map(|row| (row.get("tbl"), row.get("name")))
    .collect();
    let observed: Vec<(&str, &str)> = multi_column_checks
        .iter()
        .map(|(table, name)| (table.as_str(), name.as_str()))
        .collect();
    assert_eq!(
        observed,
        [
            ("checkout_sessions", "urls_match_ui_mode"),
            ("idempotency_keys", "complete_has_a_response"),
            ("jobs", "lock_is_paired"),
            ("oauth_signing_keys", "active_key_has_no_expiry"),
            ("oauth_signing_keys", "expiry_after_creation"),
            ("payment_intents", "lpe_paired"),
            // The over-refund guard.
            ("payment_intents", "no_over_refund"),
            ("provider_requests", "response_is_paired"),
            // The refund-capability coherence rule.
            ("providers", "partial_refunds_imply_refunds"),
            ("refunds", "failure_paired"),
        ],
        "the multi-column CHECK constraints backends/migrations builds. This list is read from \
         the live database rather than from the report precisely because the report cannot see \
         it — so if a constraint is deleted from a migration, this assertion is what notices, \
         and the drift count below cannot"
    );

    // None of the ten reaches the report.
    //
    // Matched as the shape the report renders a CHECK in — ``CHECK `name` ``,
    // from `cratestack-cli` 0.11.1's `src/migrate/drift_report.rs::describe`,
    // which is the only way an `Op::AddCheck`/`Op::DropCheck` is ever printed
    // — rather than as a bare substring of the whole report. Detection is
    // identical and the false-positive class goes away: this repository names
    // constraints by pattern, so `amount_non_negative` already appears on
    // three tables and `id_length` on two, and a bare `contains` would trip on
    // a multi-column CHECK that merely shares a name with a reported
    // single-column one on some other table. It would trip red, not green —
    // but with a message accusing the tool of a change it had not made.
    for (table, name) in &observed {
        let rendered = format!("CHECK `{name}`");
        assert!(
            !stdout.contains(&rendered),
            "`{table}.{name}` is a multi-column CHECK that `migrate baseline` did not mention on \
             2026-09-05, at cratestack 0.11.1. It does now. That is a change in what the tool \
             can see — very likely the cross-column CHECK support this repository has been \
             asking for — and it wants recording in docs/status.md and schemas/vpay.cstack's \
             header rather than a constant bump: {stdout}"
        );
    }

    // Two of the ten sit on tables the schema *does* model, so their absence
    // is not the table being skipped: a single-column CHECK on each of those
    // very tables is reported, and pinning both lines is what separates "the
    // tool cannot see cross-column CHECKs" from "the tool said nothing about
    // `providers` or `payment_intents` at all".
    //
    // Why this matters beyond the `@@check` ask: a future `--strict` run that
    // exits 0 would say nothing whatever about the over-refund guard or the
    // refund-capability rule — the two constraints the migrations added
    // *because* the grammar could not express them are exactly the two a
    // green drift report cannot vouch for. Deleting `CONSTRAINT
    // no_over_refund` from migration 0003 leaves the count below at 86,
    // measured; `over_refund_is_rejected_by_the_database` above is what
    // catches that, and the `pg_constraint` assertion here is what catches it
    // in this test.
    for reported in [
        // providers, alongside the invisible partial_refunds_imply_refunds
        "CHECK `code_length` exists in the live database but is not declared in the schema",
        // payment_intents, alongside the invisible no_over_refund
        "CHECK `amount_refunded_non_negative` exists in the live database but is not declared in \
         the schema",
    ] {
        assert!(
            stdout.contains(reported),
            "expected the report to carry `{reported}` — a single-column CHECK on the same table \
             as one of the invisible cross-column ones: {stdout}"
        );
    }

    // The report's own account of what it declined to compare.
    let unmappable: u32 = stdout
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .next()
                .filter(|_| line.contains("could not confidently map"))
                .and_then(|count| count.parse().ok())
        })
        .context("the report should carry a `N column(s) … could not confidently map` line")?;
    assert_eq!(
        unmappable, EXPECTED_UNMAPPABLE_COLUMNS,
        "the number of columns excluded from the comparison changed; see \
         EXPECTED_UNMAPPABLE_COLUMNS for why that invalidates the count above rather than \
         merely accompanying it"
    );

    // `--strict` documents "no snapshot was written and no baseline row was
    // recorded". Both halves are checked below — the snapshot against the
    // out-dir and the repository, the baseline row against the database.
    assert_eq!(
        stderr.trim(),
        format!(
            "Error: migrate baseline: --strict refuses to baseline with {EXPECTED_DRIFT_CHANGES} \
             pending drift change(s); resolve the drift above (or drop --strict) and try again. \
             No snapshot was written and no baseline row was recorded."
        ),
        "the strict refusal should name the same count the report did"
    );
    let written: Vec<_> = std::fs::read_dir(&out_dir.0)
        .context("listing the out-dir after the run")?
        .collect::<Result<Vec<_>, _>>()
        .context("listing the out-dir after the run")?
        .iter()
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        written.is_empty(),
        "`--strict` promises no writes when it finds drift, and wrote {written:?}"
    );
    assert!(
        !root.join("migrations").exists(),
        "`--out-dir` was pointed outside the checkout; a `migrations/` directory at the \
         repository root means the flag was ignored and the default was used"
    );
    assert_eq!(
        std::fs::read(&schema).context("re-reading schemas/vpay.cstack")?,
        schema_before,
        "measuring the schema must not edit it"
    );

    // The other half of the promise, and the half nothing else here could
    // notice. A recorded baseline row means a `cratestack_migrations` table
    // (`cratestack-cli` 0.11.1, `src/migrate/baseline_cmd.rs`), and
    // introspection excludes that table from its own table list
    // (`cratestack-migrate` 0.11.1, `src/introspect/postgres/tables.rs`) — so
    // if `--strict` did write one, the 86 above would not move and the set of
    // undeclared tables would not grow. docs/status.md asserted this from a
    // run someone did by hand; it is read from the database now.
    let baseline_row_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.cratestack_migrations')::text")
            .fetch_one(&pool)
            .await
            .context("checking whether migrate baseline recorded a baseline row")?;
    assert_eq!(
        baseline_row_table, None,
        "`--strict` promises no baseline row was recorded, and a `cratestack_migrations` table \
         exists. Nothing else in this test could see that: the drift report never lists that \
         table"
    );

    Ok(())
}

// --- migration 0031 (refunds.fee) ------------------------------------------

/// Inserts a `refunds` row (migration `0017`) with a given `fee` (migration
/// `0031`).
///
/// `Option<i64>` and not `i64`, because the distinction between `NULL` and `0`
/// is the entire subject of the two tests below. Written as raw SQL for the
/// reason the header gives for every other insert in this file: `vpay_db`'s
/// refunds repository has a read and no write, so there is nothing to route an
/// insert through, and the subject here is what *the database* enforces.
async fn insert_refund(
    pool: &PgPool,
    id: &str,
    payment_intent_id: &str,
    fee: Option<i64>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO refunds \
            (id, payment_intent_id, amount, currency_code, status, fee) \
         VALUES ($1, $2, 1000, 'XAF', 'pending'::refund_status, $3)",
    )
    .bind(id)
    .bind(payment_intent_id)
    .bind(fee)
    .execute(pool)
    .await
}

/// Issue #46's whole point, at the layer that would otherwise quietly lose it:
/// an unknown fee and a zero fee must be two different rows, and neither may
/// become the other on the way in or out.
///
/// A `DEFAULT 0` on the column, or a writer that mapped `None` to `0`, would
/// still let every other assertion about `refunds.fee` pass — the column would
/// exist, be `BIGINT`, and round-trip a number. This is the case that fails.
#[tokio::test]
async fn an_unreported_refund_fee_stays_null_and_never_becomes_zero() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_currencies(&pool).await?;
    insert_payment_intent(&pool, "pi_fee", 5_000, 0, 0).await?;

    insert_refund(&pool, "re_unknown", "pi_fee", None).await?;
    insert_refund(&pool, "re_free", "pi_fee", Some(0)).await?;
    insert_refund(&pool, "re_charged", "pi_fee", Some(250)).await?;

    // And the case a `DEFAULT 0` would quietly rewrite: a writer that does
    // not mention the column at all. This is the shape a refunds repository
    // written before it knew about `fee` would have, and binding NULL
    // explicitly above does not exercise it — measured 2026-09-05, adding
    // `DEFAULT 0` to migration 0031 left every other assertion here green.
    sqlx::query(
        "INSERT INTO refunds (id, payment_intent_id, amount, currency_code, status) \
         VALUES ('re_omitted', 'pi_fee', 1000, 'XAF', 'pending'::refund_status)",
    )
    .execute(&pool)
    .await
    .context("inserting a refund whose INSERT does not mention `fee` at all")?;

    let read = |id: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<i64>>("SELECT fee FROM refunds WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
        }
    };

    assert_eq!(
        read("re_unknown").await?,
        None,
        "a refund written without a fee must read back as unknown, not as free — the column has \
         no DEFAULT for exactly this reason"
    );
    assert_eq!(
        read("re_free").await?,
        Some(0),
        "a rail that reported the movement cost nothing must stay distinguishable from one that \
         reported nothing at all"
    );
    assert_eq!(read("re_charged").await?, Some(250));
    assert_eq!(
        read("re_omitted").await?,
        None,
        "an INSERT that never mentions `fee` must leave it unknown — the column must have no \
         DEFAULT, or every refund vpay ever writes claims to have been free"
    );

    Ok(())
}

/// `fee_non_negative` (migration `0031`) fires.
///
/// A negative fee would be a rebate, which vpay has no concept of; rendered
/// onto the `refund` object it would show a merchant a cost that credits them.
/// The CHECK is what stops it, and this proves the CHECK rather than the SQL
/// merely parsing.
#[tokio::test]
async fn a_negative_refund_fee_is_rejected_by_the_database() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    seed_currencies(&pool).await?;
    insert_payment_intent(&pool, "pi_fee_negative", 5_000, 0, 0).await?;

    let err = insert_refund(&pool, "re_negative", "pi_fee_negative", Some(-1))
        .await
        .expect_err("a negative fee must be rejected");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.constraint(),
        Some("fee_non_negative"),
        "the rejection must come from 0031's CHECK specifically"
    );

    Ok(())
}
