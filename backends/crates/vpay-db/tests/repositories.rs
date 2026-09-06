//! Integration tests for the OP-2 repository layer —
//! [`vpay_db::client_assertion_store`], the `disabled_clients` kill-switch
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
use vpay_db::{
    Charges, Events, Idempotency, Jobs, PaymentIntents, Repositories, TxOutcome, UnitOfWork as _,
};

/// Starts a fresh, migrated Postgres 16 container and returns the
/// repositories bound to it, plus a plain `sqlx` pool for the assertions that
/// read the schema itself (column lists, index definitions, CHECK
/// violations) — statements no repository method owns, and none should. The
/// returned container guard must be kept alive for as long as either is used
/// — dropping it stops and removes the container.
///
/// The image request and its host-port-collision retry live in
/// `vpay_testkit::containers` — see that module for why the tag is pinned and
/// which errors are retried.
async fn migrated_postgres()
-> anyhow::Result<(ContainerAsync<PostgresImage>, Arc<dyn Repositories>, PgPool)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
        .await
        .context("postgres:16-alpine container starts (it is cached locally on this machine)")?;

    let host = container.get_host().await.context("container host")?;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .context("container port")?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let repositories = vpay_db::connect(&url)
        .await
        .context("connects to the freshly started container")?;
    repositories
        .run_migrations()
        .await
        .context("every migration under backends/migrations applies cleanly")?;
    let pool = PgPool::connect(&url)
        .await
        .context("the raw-SQL pool connects to the same container")?;

    Ok((container, repositories, pool))
}

/// The transactional writes, each in its own committed transaction.
///
/// `vpay_db` offers no pooled variant of any of them, deliberately: every one
/// exists to commit *beside* another write, and a caller that could run one
/// on its own is the bug [`vpay_db::UnitOfWork`] closes. A test building a
/// fixture has no such second write to be in step with, so it opens a
/// transaction of its own — which is all these are. The tests that are
/// *about* a transaction boundary (atomicity, a deliberate rollback, the
/// metrics that must not be counted for a rolled-back insert) spell the
/// closure out inline instead.
mod one_tx {
    use vpay_db::{DbError, Repositories, TxOutcome, UnitOfWork as _};

    pub(super) async fn insert_for_intent(
        repositories: &dyn Repositories,
        new: &vpay_db::NewCharge,
    ) -> Result<vpay_db::ChargeRow, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(tx.insert_for_intent(new).await?))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn mark_submitted(
        repositories: &dyn Repositories,
        id: &str,
        state: &str,
        provider_ref_extra: Option<&serde_json::Value>,
        redirect_url: Option<&str>,
    ) -> Result<vpay_db::ChargeRow, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(
                        tx.mark_submitted(id, state, provider_ref_extra, redirect_url)
                            .await?,
                    ))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn insert_in_tx(
        repositories: &dyn Repositories,
        new: &vpay_db::NewEvent,
    ) -> Result<vpay_db::EventRow, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(
                    async move { Ok::<_, DbError>(TxOutcome::Commit(tx.insert_in_tx(new).await?)) },
                )
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn enqueue_in_tx(
        repositories: &dyn Repositories,
        kind: &str,
        dedupe_key: &str,
        payload: &serde_json::Value,
        run_at: time::OffsetDateTime,
    ) -> Result<bool, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(
                        tx.enqueue_in_tx(kind, dedupe_key, payload, run_at).await?,
                    ))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn pull_forward_in_tx(
        repositories: &dyn Repositories,
        dedupe_key: &str,
        floor: std::time::Duration,
    ) -> Result<bool, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(
                        tx.pull_forward_in_tx(dedupe_key, floor).await?,
                    ))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn record_payment_error(
        repositories: &dyn Repositories,
        merchant_id: &str,
        id: &str,
        expected: &str,
        code: &str,
        message: &str,
    ) -> Result<Option<vpay_db::PaymentIntentRow>, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(
                        tx.record_payment_error(merchant_id, id, expected, code, message)
                            .await?,
                    ))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn create_in_tx(
        repositories: &dyn Repositories,
        event_id: &str,
        endpoint_id: &str,
        url: &str,
    ) -> Result<Option<uuid::Uuid>, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(
                        tx.create_in_tx(event_id, endpoint_id, url).await?,
                    ))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }

    pub(super) async fn mark_fanned_out_in_tx(
        repositories: &dyn Repositories,
        event_id: &str,
    ) -> Result<bool, DbError> {
        repositories
            .transaction(|tx| {
                Box::pin(async move {
                    Ok::<_, DbError>(TxOutcome::Commit(tx.mark_fanned_out_in_tx(event_id).await?))
                })
            })
            .await
            .map(TxOutcome::into_inner)
    }
}

// --- client_assertion_store -----------------------------------------------

/// A `jti` is fresh exactly once: the first `record_jti` call must accept
/// (return `Ok(true)`), and a second call with the same `jti` must be
/// rejected as a replay (`Ok(false)`) — not an error, per the trait's own
/// contract.
#[tokio::test]
async fn a_client_assertion_jti_is_fresh_once_then_replayed() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let store = vpay_db::client_assertion_store(repositories.op_store_pool());
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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let store = Arc::new(vpay_db::client_assertion_store(
        repositories.op_store_pool(),
    ));
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
    let (_container, repositories, _pool) = migrated_postgres().await?;

    assert!(
        !repositories
            .is_client_disabled("merchant_never_disabled")
            .await?,
        "a client_id with no disabled_clients row must report not-disabled"
    );

    repositories
        .disable_client("merchant_never_disabled", Some("key compromised"))
        .await
        .context("disabling a client must succeed")?;
    assert!(
        repositories
            .is_client_disabled("merchant_never_disabled")
            .await?,
        "a client_id with a disabled_clients row must report disabled"
    );

    // disable_client must be idempotent at this layer — a second call for an
    // already-disabled client must not error (the module's own doc comment
    // argues for INSERT ... ON CONFLICT DO UPDATE specifically so an
    // operator re-running "disable this client" never has to check first).
    repositories
        .disable_client("merchant_never_disabled", Some("re-confirmed compromised"))
        .await
        .context("a second disable of the same client_id must not error")?;
    assert!(
        repositories
            .is_client_disabled("merchant_never_disabled")
            .await?
    );

    repositories
        .enable_client("merchant_never_disabled")
        .await
        .context("re-enabling a client must succeed")?;
    assert!(
        !repositories
            .is_client_disabled("merchant_never_disabled")
            .await?,
        "a client_id must report not-disabled again after enable_client removes its row"
    );

    // enable_client on a client_id that was never disabled must be a no-op,
    // not an error.
    repositories
        .enable_client("merchant_was_never_disabled_at_all")
        .await
        .context("enabling a client with no disabled_clients row must not error")?;

    Ok(())
}

/// The CrateStack read and a plain `SELECT` must answer identically, for a
/// row that exists and for one that does not.
///
/// **This is the test that makes the whole CrateStack adoption falsifiable**,
/// and it is written as a comparison rather than as an assertion about
/// `is_client_disabled` alone for one specific reason. A model policy is
/// compiled into the `WHERE` clause of the generated read
/// (`cratestack-sqlx`'s `push_action_policy_query`), and a model with no
/// `@@allow` is deny-by-default — so a policy mistake does not raise an
/// error, it silently returns zero rows. `is_client_disabled` would go on
/// answering `false` for every client, `disabled_client_lookup_reflects_
/// disable_and_enable` above would still pass its "not disabled" half, and
/// the kill-switch would be off with nothing red anywhere.
///
/// So: seed the row with a statement CrateStack had nothing to do with, then
/// read the same key three ways — the repository method (CrateStack), a
/// direct `SELECT EXISTS` (the statement this method used to be), and a
/// direct `SELECT client_id` — and require all three to agree. The second
/// read is the control: it is what proves the row is really there when the
/// first one says it is not.
///
/// **The seed changed on 2026-09-06 and the reason it had to is the point of
/// this paragraph.** Until then this test wrote through
/// `DisabledClients::disable_client`, and the sentence below the write said
/// "the sqlx write that deliberately did NOT move to CrateStack in this
/// change — so this asserts the two layers see one table, not that one layer
/// is self-consistent". That write is now a CrateStack `upsert`, so calling
/// it here would have made exactly the claim that sentence disclaimed: a
/// generated write read back by a generated read, agreeing with itself. The
/// `INSERT` and `DELETE` are inline in this test now, deliberately not
/// factored into a helper — they exist to be *unlike* the code under test,
/// and a helper shared with the CrateStack-write test below would be the
/// first step back towards one path checking itself.
///
/// **Decisive mutation, DESIGNED AND NOT YET RUN.** Delete
/// `@@allow("read", auth().isSystem())` from `model DisabledClient` in
/// `schemas/vpay.cstack` and this test should fail on the "must agree"
/// assertion with `CrateStack says false, sqlx says true`. This doc comment
/// claimed until 2026-09-06 that the mutation had been run and cited a
/// transcript in `docs/plans/exp14-notes/opus.md`; there is no such
/// transcript, and that file's own § 7 lists this mutation — and this test —
/// among the cases the authoring host could not execute, because its Docker
/// daemon was dead. **Both halves are owed to CI**, and until they are paid
/// "the CrateStack read returns what the sqlx read returns" is read out of
/// `cratestack-sqlx`'s query builder rather than measured.
///
/// What *has* been measured, on 2026-09-06 and without a container, is the
/// half that makes the mutation worth running: with the `@@allow` line
/// deleted, `just check-schema`, `cargo build`, `just clippy` and all ten
/// `just verify` gates stay green (`docs/plans/exp14-notes/opus-review.md`,
/// M8). So this test really is the only thing standing between a deleted
/// policy line and a kill-switch that answers "not disabled" for every
/// client.
#[tokio::test]
async fn a_disabled_client_reads_the_same_through_both_paths() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    /// Reads `disabled_clients` with a statement that has no policy layer
    /// and no generated code in it, so a disagreement with the repository
    /// method can only be the CrateStack path's doing.
    async fn sqlx_says_disabled(pool: &PgPool, client_id: &str) -> anyhow::Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM disabled_clients WHERE client_id = $1)",
        )
        .bind(client_id)
        .fetch_one(pool)
        .await
        .context("the control SELECT must run")?;

        // A second spelling of the same question. `SELECT EXISTS` and
        // `SELECT <column>` fail differently if the table or the column is
        // not what this test thinks it is, and a control that can be wrong
        // in the same way as the thing it controls is not a control.
        let row: Option<String> =
            sqlx::query_scalar("SELECT client_id FROM disabled_clients WHERE client_id = $1")
                .bind(client_id)
                .fetch_optional(pool)
                .await
                .context("the second control SELECT must run")?;

        assert_eq!(
            exists,
            row.is_some(),
            "the two control reads of `{client_id}` disagree with each other"
        );
        Ok(exists)
    }

    // Absent: both paths must say "not disabled", and the CrateStack read
    // must reach `Ok(None)` rather than the `NotFound` error a different
    // builder would have produced.
    let absent = "merchant_absent_from_the_table";
    assert!(!sqlx_says_disabled(&pool, absent).await?);
    assert_eq!(
        repositories.is_client_disabled(absent).await?,
        sqlx_says_disabled(&pool, absent).await?,
        "an absent client_id must read the same through CrateStack and through sqlx"
    );

    // Present, seeded with a hand-written statement — no generated code, no
    // policy layer, nothing this test is testing. So this asserts the two
    // layers see one table, not that one layer is self-consistent.
    let present = "merchant_disabled_by_a_hand_written_insert";
    sqlx::query("INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2)")
        .bind(present)
        .bind("key compromised, ticket INC-123")
        .execute(&pool)
        .await
        .context("the control INSERT must succeed")?;

    let via_sqlx = sqlx_says_disabled(&pool, present).await?;
    let via_cratestack = repositories.is_client_disabled(present).await?;
    assert!(
        via_sqlx,
        "the control read must find the row the write made"
    );
    assert_eq!(
        via_cratestack, via_sqlx,
        "a row written by the sqlx path must be visible to the CrateStack read: CrateStack says \
         {via_cratestack}, sqlx says {via_sqlx}. If CrateStack says false and sqlx says true, the \
         model's `@@allow(\"read\", auth().isSystem())` clause is missing or the context this \
         crate reads under stopped being a SystemContext — the read is compiled into the WHERE \
         clause, so a denied row is indistinguishable from an absent one and the kill-switch is \
         silently OFF"
    );

    // And back again: the removal must be visible to the CrateStack read too,
    // so the agreement is not an artefact of a row that was never removed.
    // Hand-written for the same reason the INSERT above is.
    sqlx::query("DELETE FROM disabled_clients WHERE client_id = $1")
        .bind(present)
        .execute(&pool)
        .await
        .context("the control DELETE must succeed")?;
    assert_eq!(
        repositories.is_client_disabled(present).await?,
        sqlx_says_disabled(&pool, present).await?,
        "a row deleted by the sqlx path must be gone from the CrateStack read too"
    );
    assert!(!repositories.is_client_disabled(present).await?);

    Ok(())
}

/// The twin of the test above, for the two writes — which moved to CrateStack
/// on 2026-09-06, a day after the read.
///
/// Same shape and the same reason: write through the code under test, read
/// back through a statement that shares nothing with it, and require the two
/// to agree. What is asserted here beyond "it did not error" is everything
/// the trait's doc comments promise and a generated builder could quietly
/// stop doing:
///
/// - the row carries the `reason` that was passed, so `reason` really is in
///   the insert list rather than left to a default;
/// - a second `disable_client` **updates** the reason and does **not** move
///   `disabled_at`, which is the `ON CONFLICT … DO UPDATE SET reason` half of
///   the contract and the reason `upsert_update_columns` excluding
///   `@default(...)` columns matters (`disabled_clients.rs`'s unit tests
///   assert the rendered SQL; this asserts the row);
/// - passing `None` clears the reason rather than preserving the old one,
///   because the generated `SET` is `EXCLUDED.reason` and not a `COALESCE`;
/// - `enable_client` removes the row, visibly to both paths;
/// - `enable_client` on an id that was never disabled is `Ok`.
///
/// **Decisive mutations, all three run on 2026-09-06** (`docs/plans/exp16-notes/opus.md`):
///
/// 1. Delete `@@allow("create", auth().isSystem())` from `model
///    DisabledClient` → this test FAILS at the *first* `disable_client`, with
///    a `PersistenceError::Denied` classified `Category::Internal` — **an
///    error, not a silently absent row.** That asymmetry with the read is the
///    single most useful thing measured here: `upsert_exec.rs` evaluates the
///    create policy in Rust before it builds any SQL, so an empty allow list
///    is a `CratestackError::Forbidden` rather than a `WHERE FALSE`.
/// 2. Delete `@@allow("update", …)` → this test FAILS at the *second*
///    `disable_client` and not the first, because only the conflict branch
///    consults the update policy (`upsert_resolve.rs::gate_update_policy`).
///    A test that disabled a client once and stopped would have passed.
/// 3. Delete `@@allow("delete", …)` → this test FAILS at the `enable_client`
///    assertion, and it fails the way a *read* mutation does: the call
///    returns `Ok` and the row is still there. `delete_many` puts its policy
///    in the `WHERE`, so zero rows deleted is indistinguishable from nothing
///    to delete. This is why `enable_client` is asserted by reading the table
///    back rather than by trusting its return.
#[tokio::test]
async fn a_client_disabled_through_cratestack_is_visible_to_both_paths() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    /// The whole row, read with no generated code and no policy layer in the
    /// statement. `Option` because "no row" is an answer this test asserts.
    async fn row(
        pool: &PgPool,
        client_id: &str,
    ) -> anyhow::Result<Option<(chrono::DateTime<Utc>, Option<String>)>> {
        sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<String>)>(
            "SELECT disabled_at, reason FROM disabled_clients WHERE client_id = $1",
        )
        .bind(client_id)
        .fetch_optional(pool)
        .await
        .context("the control SELECT must run")
    }

    let client = "merchant_disabled_by_the_cratestack_write";

    assert!(
        row(&pool, client).await?.is_none(),
        "the fixture must start with no row for `{client}`"
    );

    repositories
        .disable_client(client, Some("key compromised, ticket INC-123"))
        .await
        .context(
            "the CrateStack upsert must succeed. A `Denied`/`Internal` here means \
             `@@allow(\"create\", auth().isSystem())` is missing from `model DisabledClient` in \
             schemas/vpay.cstack — unlike the read, the write says so rather than doing nothing",
        )?;

    let (first_disabled_at, reason) = row(&pool, client)
        .await?
        .context("the row the CrateStack upsert wrote must be visible to a plain SELECT")?;
    assert_eq!(
        reason.as_deref(),
        Some("key compromised, ticket INC-123"),
        "the operator note must reach the column, not a default"
    );
    assert!(
        repositories.is_client_disabled(client).await?,
        "and the CrateStack read must see the row the CrateStack write made"
    );

    // Second disable: updates the reason, leaves `disabled_at` alone. Both
    // halves are the trait's documented contract and both are properties of
    // `upsert_update_columns` rather than of anything this crate writes.
    repositories
        .disable_client(client, Some("re-confirmed compromised"))
        .await
        .context(
            "a second disable must not error. A `Denied` here and not on the first call means \
             `@@allow(\"update\", auth().isSystem())` is missing: only the conflict branch \
             consults the update policy",
        )?;

    let (second_disabled_at, reason) = row(&pool, client)
        .await?
        .context("the row must still exist after a second disable")?;
    assert_eq!(
        reason.as_deref(),
        Some("re-confirmed compromised"),
        "a second disable must overwrite the reason — `DO UPDATE SET reason = EXCLUDED.reason`"
    );
    assert_eq!(
        second_disabled_at, first_disabled_at,
        "a second disable must NOT move `disabled_at`: it records when the client was FIRST \
         disabled, and an upsert that assigned it would silently rewrite that history"
    );

    // `None` clears the note. `EXCLUDED.reason`, not `COALESCE(EXCLUDED.reason, reason)`.
    repositories
        .disable_client(client, None)
        .await
        .context("disabling with no reason must succeed")?;
    let (_, reason) = row(&pool, client)
        .await?
        .context("the row must still exist")?;
    assert_eq!(
        reason, None,
        "passing no reason must clear the column, not preserve the previous note"
    );

    // Enable: the row goes, and both paths must see it go. Asserted by
    // reading the table, never by trusting the return value — a missing
    // `@@allow("delete", …)` makes `delete_many` remove zero rows and return
    // `Ok`, which is the one silent failure mode this write has.
    repositories
        .enable_client(client)
        .await
        .context("the CrateStack delete must succeed")?;
    assert!(
        row(&pool, client).await?.is_none(),
        "`enable_client` returned Ok and the row is still there. That is what a missing \
         `@@allow(\"delete\", auth().isSystem())` looks like: `delete_many` compiles the policy \
         into the WHERE, so a denied delete removes nothing and reports success"
    );
    assert!(
        !repositories.is_client_disabled(client).await?,
        "and the CrateStack read must agree the row is gone"
    );

    // Idempotent in the other direction too: enabling something that was
    // never disabled is a no-op, not an error. This is the contract that
    // forced `delete_many` over `delete` — `DeleteRecord` reports a row it
    // did not match as `Forbidden`, and cannot tell that from a policy
    // refusal.
    repositories
        .enable_client("merchant_that_was_never_disabled")
        .await
        .context("enabling a client with no row must be a no-op, not an error")?;

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
    let (_container, repositories, pool) = migrated_postgres().await?;

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

    let mut kids: Vec<String> = repositories
        .publishable_signing_keys()
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
    let (_container, repositories, pool) = migrated_postgres().await?;

    // Bootstrap case first: rotating with no key currently active must not
    // error (the UPDATE affects zero rows and the INSERT becomes an
    // ordinary bootstrap insert, per the function's own doc comment).
    repositories
        .rotate_signing_key(
            "key_v1",
            &fixture_public_jwk("v1"),
            time::OffsetDateTime::now_utc() + time::Duration::minutes(30),
        )
        .await
        .context("rotating with no prior active key (bootstrap) must succeed")?;

    assert_eq!(
        repositories.active_signing_key_kid().await?,
        Some("key_v1".to_string()),
        "after the bootstrap rotation, key_v1 must be the sole active key"
    );

    // Real rotation: an active key already exists (key_v1); rotating to
    // key_v2 must retire key_v1 and leave key_v2 as the only active row.
    repositories
        .rotate_signing_key(
            "key_v2",
            &fixture_public_jwk("v2"),
            time::OffsetDateTime::now_utc() + time::Duration::minutes(30),
        )
        .await
        .context("rotating from an existing active key must succeed")?;

    assert_eq!(
        repositories.active_signing_key_kid().await?,
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

    let retired_kids: Vec<String> = repositories
        .publishable_signing_keys()
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    // Truncated to whole microseconds before it is written: `TIMESTAMPTZ`
    // stores microseconds, so a nanosecond-precision instant would never
    // compare equal to what Postgres hands back. This is the exact
    // mismatch that made this test fail on its first real run.
    let retire_at =
        microsecond_precision(time::OffsetDateTime::now_utc() + time::Duration::hours(24));

    let first = repositories
        .ensure_active_signing_key("kid_boot_v1", &fixture_public_jwk("boot-v1"), retire_at)
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
    let second = repositories
        .ensure_active_signing_key("kid_boot_v1", &fixture_public_jwk("boot-v1"), retire_at)
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
    let third = repositories
        .ensure_active_signing_key("kid_boot_v2", &fixture_public_jwk("boot-v2"), retire_at)
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
        repositories.active_signing_key_kid().await?,
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
    let (_container, repositories, pool) = migrated_postgres().await?;
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
        let repositories = Arc::clone(&repositories);
        handles.push(tokio::spawn(async move {
            repositories
                .ensure_active_signing_key(
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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    // Truncated to whole microseconds before it is written: `TIMESTAMPTZ`
    // stores microseconds, so a nanosecond-precision instant would never
    // compare equal to what Postgres hands back. This is the exact
    // mismatch that made this test fail on its first real run.
    let retire_at =
        microsecond_precision(time::OffsetDateTime::now_utc() + time::Duration::hours(24));

    repositories
        .ensure_active_signing_key("kid_old", &fixture_public_jwk("old"), retire_at)
        .await
        .context("bootstrapping the old key must succeed")?;
    repositories
        .ensure_active_signing_key("kid_new", &fixture_public_jwk("new"), retire_at)
        .await
        .context("rotating to the new key must succeed")?;

    let rolled_back = repositories
        .ensure_active_signing_key("kid_old", &fixture_public_jwk("old"), retire_at)
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
        repositories.active_signing_key_kid().await?,
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
async fn seed_reference_data(repositories: &dyn Repositories) -> anyhow::Result<()> {
    repositories
        .reconcile(
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
        // Through the real generator, so these rows satisfy migration
        // `0026`'s `client_secret_suffix_length` CHECK the same way a
        // `/v1`-created intent does.
        client_secret_suffix: vpay_core::ids::client_secret_suffix(),
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
    let (_container, _repositories, pool) = migrated_postgres().await?;

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
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_one_charge", "XAF"))
        .await
        .context("inserting the intent must succeed")?;

    let charge = one_tx::insert_for_intent(
        repositories.as_ref(),
        &fixture_charge("ch_1", "pi_one_charge"),
    )
    .await
    .context("the first charge for an intent must be accepted")?;
    assert_eq!(charge.state, "submitting");
    assert_eq!(
        charge.currency_code, "XAF",
        "D2: the charge carries the intent's currency verbatim"
    );

    let error = one_tx::insert_for_intent(
        repositories.as_ref(),
        &fixture_charge("ch_2", "pi_one_charge"),
    )
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_urls", "XAF"))
        .await?;

    // The happy path first, so the refusals below cannot be passing because
    // the insert was broken for some unrelated reason.
    let mut good = fixture_charge("ch_urls", "pi_urls");
    good.return_url = Some("https://shop.example/return".to_owned());
    one_tx::insert_for_intent(repositories.as_ref(), &good)
        .await
        .context("an http(s) return_url must be accepted")?;

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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    // Shape-valid (three uppercase letters, so `code_is_iso4217_shape` is
    // not what rejects it) but absent from `currencies`.
    let error = repositories
        .insert(&fixture_intent("pi_bad_ccy", "ZZZ"))
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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_cas", "XAF"))
        .await
        .context("inserting the intent must succeed")?;

    let cancelled = repositories
        .cancel("merchant_a", "pi_cas")
        .await
        .context("cancelling a requires_payment_method intent must succeed")?
        .context("cancel must return the updated row")?;
    assert_eq!(cancelled.status, "canceled");

    // The stale write: whoever issues it still believes the intent is
    // `requires_payment_method`.
    let stale = repositories
        .transition(
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

    let after = PaymentIntents::get_for_merchant(repositories.as_ref(), "merchant_a", "pi_cas")
        .await?
        .context("the intent must still exist")?;
    assert_eq!(
        after, cancelled,
        "not one column may have moved, updated_at included"
    );

    // The same guard must also refuse a foreign merchant, and must not
    // reveal that the intent exists at all.
    let foreign = repositories
        .transition("merchant_b", "pi_cas", "canceled", "processing")
        .await?;
    assert_eq!(foreign, None);
    assert_eq!(
        PaymentIntents::get_for_merchant(repositories.as_ref(), "merchant_b", "pi_cas").await?,
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_in_flight", "XAF"))
        .await
        .context("inserting the intent must succeed")?;

    one_tx::insert_for_intent(
        repositories.as_ref(),
        &fixture_charge("ch_live", "pi_in_flight"),
    )
    .await
    .context("the charge a confirm commits before submitting")?;

    let before =
        PaymentIntents::get_for_merchant(repositories.as_ref(), "merchant_a", "pi_in_flight")
            .await?
            .context("the intent must exist")?;
    assert_eq!(
        before.status, "requires_payment_method",
        "the confirm left the status alone — which is exactly why the status is not enough"
    );

    assert_eq!(
        repositories.cancel("merchant_a", "pi_in_flight").await?,
        None,
        "an intent whose charge may be live must not be cancellable"
    );
    let after =
        PaymentIntents::get_for_merchant(repositories.as_ref(), "merchant_a", "pi_in_flight")
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
            repositories.cancel("merchant_a", "pi_in_flight").await?,
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
    let cancelled = repositories
        .cancel("merchant_a", "pi_in_flight")
        .await?
        .context("an intent whose only charge has failed must still be cancellable")?;
    assert_eq!(cancelled.status, "canceled");

    // And the guard is not a substitute for the tenancy filter or the
    // status guard: neither may be reached from another merchant, and a
    // second cancel does nothing.
    assert_eq!(
        repositories.cancel("merchant_b", "pi_in_flight").await?,
        None
    );
    assert_eq!(
        repositories.cancel("merchant_a", "pi_in_flight").await?,
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
    let (_container, repositories, pool) = migrated_postgres().await?;

    // Eight, not more: `vpay_db::connect`'s pool caps at ten connections.
    const RETRIES: usize = 8;
    let mut handles = Vec::with_capacity(RETRIES);
    for _ in 0..RETRIES {
        let repositories = Arc::clone(&repositories);
        handles.push(tokio::spawn(async move {
            Idempotency::claim(
                repositories.as_ref(),
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
    let (_container, repositories, _pool) = migrated_postgres().await?;

    let first = Idempotency::claim(
        repositories.as_ref(),
        "merchant_a",
        "key-reused",
        "POST",
        "/v1/payment_intents",
        &fixture_hash(0x01),
    )
    .await?;
    let first = fresh_claim_id(&first)?;

    assert_eq!(
        repositories
            .store(
                "merchant_a",
                "key-reused",
                first,
                200,
                &json!({"id": "pi_1"}),
                None,
            )
            .await
            .context("storing the first response must succeed")?,
        vpay_db::IdempotencyStoreOutcome::Stored
    );

    // Same key, same merchant, different body — so a different hash.
    let second = Idempotency::claim(
        repositories.as_ref(),
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
    let other_merchant = Idempotency::claim(
        repositories.as_ref(),
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
    let (_container, repositories, pool) = migrated_postgres().await?;

    let claim_id = fresh_claim_id(
        &Idempotency::claim(
            repositories.as_ref(),
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
        repositories
            .store("merchant_a", "key-replayed", claim_id, 200, &body, None,)
            .await
            .context("storing the response must succeed")?,
        vpay_db::IdempotencyStoreOutcome::Stored
    );

    let replay = Idempotency::claim(
        repositories.as_ref(),
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
    let twice = repositories
        .store(
            "merchant_a",
            "key-replayed",
            claim_id,
            500,
            &json!({"error": "would clobber the answer already given"}),
            Some("false"),
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

/// The `stripe-should-retry` advisory survives a round trip through
/// `idempotency_keys`, and migration `0025`'s CHECK refuses anything that is
/// not one of the header's two values.
///
/// Both halves matter, and for different reasons. The round trip is what
/// makes `v1::payment_intents::replay` able to re-emit the advisory the
/// original response carried instead of working it out again from the stored
/// status — the drift ADR-0011 exists to prevent. The CHECK is what stops
/// that column from becoming a place a caller can put arbitrary text that
/// would then be written into a response header.
///
/// `None` is asserted separately from `'false'` because they are different
/// answers: a stored `2xx` never passed through the error renderer and its
/// replay must emit no header at all, which `NOT NULL DEFAULT 'false'` would
/// have quietly turned into "do retry your successful create".
#[tokio::test]
async fn the_retry_advisory_round_trips_and_0025_refuses_anything_else() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    for (key, hash, status, advisory) in [
        ("key-advice-false", 0x51_u8, 409_u16, Some("false")),
        ("key-advice-true", 0x52, 400, Some("true")),
        ("key-advice-none", 0x53, 200, None),
    ] {
        let claim_id = fresh_claim_id(
            &Idempotency::claim(
                repositories.as_ref(),
                "merchant_a",
                key,
                "POST",
                "/v1/payment_intents",
                &fixture_hash(hash),
            )
            .await?,
        )?;
        assert_eq!(
            repositories
                .store(
                    "merchant_a",
                    key,
                    claim_id,
                    status,
                    &json!({"stored": key}),
                    advisory,
                )
                .await
                .context("storing the response must succeed")?,
            vpay_db::IdempotencyStoreOutcome::Stored
        );

        match Idempotency::claim(
            repositories.as_ref(),
            "merchant_a",
            key,
            "POST",
            "/v1/payment_intents",
            &fixture_hash(hash),
        )
        .await?
        {
            vpay_db::IdempotencyClaim::Replay(record) => {
                assert_eq!(record.response_status, Some(status.cast_signed()));
                assert_eq!(
                    record.response_retry.as_deref(),
                    advisory,
                    "{key}: the advisory a replay reads back must be the one that was stored"
                );
            }
            other => panic!("{key}: a completed key must replay, got {other:?}"),
        }
    }

    // The CHECK, proven by making it fire. `'maybe'` is what a re-derivation
    // from some future third `Retry` variant would most plausibly try to
    // write; the column refuses it rather than letting it reach a header.
    let refused = sqlx::query(
        "UPDATE idempotency_keys SET response_retry = 'maybe' \
         WHERE merchant_id = 'merchant_a' AND idempotency_key = 'key-advice-false'",
    )
    .execute(&pool)
    .await
    .expect_err("response_retry_is_an_advisory must refuse a third value");
    assert!(
        refused
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::constraint)
            .is_some_and(|name| name == "response_retry_is_an_advisory"),
        "expected the named CHECK to fire, got {refused}"
    );

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
    let (_container, repositories, pool) = migrated_postgres().await?;

    for key in ["key-abandoned", "key-running"] {
        fresh_claim_id(
            &Idempotency::claim(
                repositories.as_ref(),
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
    let reclaim = Idempotency::claim(
        repositories.as_ref(),
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
        Idempotency::claim(
            repositories.as_ref(),
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
    let (_container, repositories, _pool) = migrated_postgres().await?;

    let claim = |key: &'static str| {
        let repositories = Arc::clone(&repositories);
        async move {
            Idempotency::claim(
                repositories.as_ref(),
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
        repositories
            .release("merchant_a", "key-failed", failed)
            .await?,
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
    repositories
        .store("merchant_a", "key-done", done, 200, &body, None)
        .await
        .context("storing the response must succeed")?;
    assert_eq!(
        repositories.release("merchant_a", "key-done", done).await?,
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
        repositories
            .release("merchant_a", "never-seen", uuid::Uuid::new_v4())
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
    let (_container, repositories, pool) = migrated_postgres().await?;

    let r1 = fresh_claim_id(
        &Idempotency::claim(
            repositories.as_ref(),
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
        &Idempotency::claim(
            repositories.as_ref(),
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
        repositories.release("merchant_a", "key-aba", r1).await?,
        0,
        "a superseded claim must delete nothing; deleting R2's row would let a third request run \
         under this key at the same time as R2"
    );

    // ...and its store must not overwrite R2's row, nor claim to have
    // stored anything.
    assert_eq!(
        repositories
            .store(
                "merchant_a",
                "key-aba",
                r1,
                200,
                &json!({"id": "pi_r1", "note": "R1's answer, under R2's claim"}),
                None,
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
        repositories
            .store(
                "merchant_a",
                "key-aba",
                r2,
                201,
                &json!({"id": "pi_r2"}),
                None,
            )
            .await
            .context("the live claim must still be able to store its response")?,
        vpay_db::IdempotencyStoreOutcome::Stored
    );
    match Idempotency::claim(
        repositories.as_ref(),
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
    let (_container, repositories, pool) = migrated_postgres().await?;

    for key in ["key-old", "key-fresh"] {
        fresh_claim_id(
            &Idempotency::claim(
                repositories.as_ref(),
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

    let swept = repositories
        .sweep_expired()
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
    assert_eq!(repositories.sweep_expired().await?, 0);

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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    // Inserted oldest-first, so `page_fixture_id(24)` is the newest and every
    // page below is expressed against that order. The ids are computed
    // rather than collected so the expectations below can be written
    // directly, without indexing a Vec (which `clippy::indexing_slicing`
    // denies, tests included).
    for n in 0..25 {
        let row = repositories
            .insert(&fixture_intent(&page_fixture_id(n), "XAF"))
            .await
            .context("inserting a page fixture must succeed")?;
        assert_eq!(row.id, page_fixture_id(n));
    }
    // Another merchant's intents must never appear in any page below.
    let mut other = fixture_intent("pi_other_merchant", "XAF");
    other.merchant_id = "merchant_b".to_owned();
    repositories.insert(&other).await?;

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
    let (first, has_more) =
        PaymentIntents::list_page(repositories.as_ref(), "merchant_a", &page).await?;
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
        PaymentIntents::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert_eq!(ids(&second), newest_first(5, 14));
    assert!(has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some(page_fixture_id(5)),
        ending_before: None,
    };
    let (third, has_more) =
        PaymentIntents::list_page(repositories.as_ref(), "merchant_a", &page).await?;
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
    let (back, has_more) =
        PaymentIntents::list_page(repositories.as_ref(), "merchant_a", &page).await?;
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
    let (tail, has_more) =
        PaymentIntents::list_page(repositories.as_ref(), "merchant_a", &page).await?;
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
    let (none, has_more) =
        PaymentIntents::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert!(none.is_empty() && !has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: None,
    };
    let (others, _) = PaymentIntents::list_page(repositories.as_ref(), "merchant_b", &page).await?;
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
        // No `flow::TEXT` cast any more: migration 0032 made `flow` a plain
        // `TEXT` column guarded by `providers_flow_enum_check`, because
        // cratestack's generated row decoders read an enum column with
        // `try_get::<String>()` and a native Postgres enum fails that on
        // every read. The cast was doing real work while `provider_flow`
        // existed; leaving it would now be a no-op that reads as though the
        // column were still an enum.
        "SELECT code, flow, supports_refunds, enabled FROM providers ORDER BY code",
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
    let (_container, repositories, pool) = migrated_postgres().await?;

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

    repositories
        .reconcile(&currencies, &[mtn.clone(), orange.clone()])
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

    repositories
        .reconcile(&currencies, &[mtn.clone(), orange.clone()])
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
    repositories
        .reconcile(&currencies, std::slice::from_ref(&mtn))
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
    let error = repositories
        .reconcile(&wrong, std::slice::from_ref(&mtn))
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
    // `i64`, because migration 0032 widened `currencies.exponent` to
    // `BIGINT`. An `i32` here decodes as "mismatched types; Rust type `i32`
    // (as SQL type `INT4`) is not compatible with SQL type `INT8`" — sqlx
    // refuses the narrowing rather than performing it, which is why this is
    // a compile-and-run change and not a silent one.
    let stored_exponent: i64 =
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

/// A currency row this deployment did **not** write is still refused when
/// its exponent disagrees — which is the case the `find_unique(...)
/// .for_update()` read exists for, and the one the statement it replaced
/// could not have been asked.
///
/// `reconcile_is_idempotent_and_disables_a_dropped_provider_code` above
/// reaches the same refusal, but it seeds `XAF` *through `reconcile` itself*
/// first. That leaves one thing unproven: whether the guard reads what is
/// actually in the table, or merely remembers what this process put there.
/// Here the row is inserted by hand — the shape an operator's `psql`, an
/// older release, or a replica that booted with a different configuration
/// leaves behind — and then boot step 4 runs against it.
///
/// Why that distinction is worth a test of its own: since 2026-09-06 the
/// comparison is a separate `SELECT ... FOR UPDATE` rather than the
/// `RETURNING` of a no-op `ON CONFLICT DO UPDATE`, because CrateStack's
/// `upsert` renders `SET exponent = EXCLUDED.exponent` and would have
/// **overwritten** the stored value (pinned by
/// `the_currency_upsert_would_overwrite_a_stored_exponent_on_its_own` in
/// `vpay-db`'s own unit tests). If that read is ever dropped, or if
/// `@@allow("read", auth().isSystem())` leaves `model Currency` — which
/// compiles into the `WHERE` clause and turns "the row is there" into "no
/// row" silently — this test is what fails, and the assertion at the end is
/// what makes it fail for the right reason: the stored exponent is still 0.
#[tokio::test]
async fn a_hand_seeded_currency_exponent_is_read_back_and_refused_not_overwritten()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    // Not through `reconcile`: this is the row somebody else left.
    sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('XAF', 0)")
        .execute(&pool)
        .await
        .context("seeding XAF at exponent 0 by hand must succeed")?;

    let (mtn, _orange) = reconcile_rails();
    let error = repositories
        .reconcile(
            &[vpay_db::CurrencySeed {
                code: "XAF".to_owned(),
                exponent: 2,
            }],
            std::slice::from_ref(&mtn),
        )
        .await
        .expect_err("a deployment that disagrees with the stored exponent must be refused");

    match &error {
        vpay_db::DbError::CurrencyExponentConflict {
            code,
            stored,
            seeded,
        } => {
            assert_eq!(code, "XAF");
            assert_eq!(*stored, 0, "the refusal must name what the DATABASE holds");
            assert_eq!(*seeded, 2, "and what this deployment asked for");
        }
        other => panic!("expected a named exponent conflict, got {other:?}"),
    }
    assert_eq!(
        vpay_core::Classify::category(&error),
        vpay_core::Category::Configuration,
        "the fix is a corrected deployment, never a retry"
    );
    assert_eq!(
        vpay_core::Classify::category(&error).exit_code(),
        78,
        "a misconfigured deployment must exit 78 (fix the deploy), never 69 (wait for Postgres)"
    );

    // The whole transaction rolled back: the exponent is untouched AND the
    // provider the same call would have written is absent. The second half
    // is what says the refusal happened before any other write landed, not
    // merely that this one row survived.
    let stored_exponent: i64 =
        sqlx::query_scalar("SELECT exponent FROM currencies WHERE code = 'XAF'")
            .fetch_one(&pool)
            .await
            .context("re-reading the exponent must succeed")?;
    assert_eq!(
        stored_exponent, 0,
        "the stored exponent must be untouched — every amount already recorded in XAF is a \
         count of minor units at exponent 0"
    );
    let providers_written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&pool)
        .await
        .context("counting providers must succeed")?;
    assert_eq!(
        providers_written, 0,
        "the refusal must roll the whole transaction back, not just skip the currency"
    );

    Ok(())
}

/// The currency write goes into **vpay's** transaction, so vpay's rollback
/// takes it back — asserted in the direction that was not covered.
///
/// `config_reconcile`'s module doc leads with "Every statement below runs in
/// one transaction, so a failure part-way through leaves the tables exactly
/// as they were". Since 2026-09-06 half of those statements belong to an
/// external crate, and `run_in_tx` versus `run` is the *only* difference
/// between joining this transaction and quietly opening a private one. A
/// `.run(&ctx)` there would still compile, still return `Ok`, and still
/// write the right row — and the row would survive a rollback that was
/// supposed to erase it.
///
/// `a_hand_seeded_currency_exponent_is_read_back_and_refused_not_overwritten`
/// above proves the other direction: a currency refusal leaves no provider
/// behind. Nothing proved this one.
///
/// What the review of 2026-09-06 measured when it changed that `run_in_tx`
/// to `.run(&ctx)` is worth stating exactly, because it is *not* "the suite
/// stayed green" and it is not a plain rollback failure either:
///
///   * this test fails in **1.2 s**, on the assertion below;
///   * `a_hand_seeded_currency_exponent_is_read_back_and_refused_not_overwritten`
///     still passes — it never reaches the upsert;
///   * `reconcile_is_idempotent_and_disables_a_dropped_provider_code`
///     **hangs forever**. Not fails: hangs. `upsert`'s own conflict probe is
///     `SELECT … FOR UPDATE` (`upsert_sql.rs::select_for_update_by_conflict_target`),
///     so off the transaction it waits on the row lock the transaction
///     itself is holding, and the transaction is waiting on it. nextest
///     reported `SLOW [>480.000s]` and was still reporting it when the run
///     was killed.
///
/// So `run_in_tx` is not only about atomicity here — with `.for_update()`
/// above it, it is what keeps boot from deadlocking against itself. This
/// test exists so that mistake reports as a red assertion naming the cause
/// rather than as a boot that never returns, which is what the same mistake
/// would do in production. See docs/plans/exp17-notes/opus-review.md.
///
/// It is *this* case rather than the idempotence one that stays fast under
/// the mutation because the currency here is an INSERT: the conflict probe
/// finds no row, so it locks nothing and blocks on nothing.
///
/// The failure is arranged so the currency lands *first* and something later
/// in the same transaction fails: seeds are iterated in sorted `code` order
/// with every currency before every provider, and this rail sets
/// `supports_partial_refunds` without `supports_refunds`, which migration
/// 0002's `partial_refunds_imply_refunds` CHECK refuses. So `EUR` is written
/// through CrateStack, then the provider upsert raises `23514`, then `tx`
/// drops. If `EUR` is still there afterwards, the write was never in this
/// transaction at all.
///
/// The provider half has an assertion of its own since 2026-09-06 —
/// `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
/// — because that statement is a CrateStack upsert now too and needs the same
/// question asked of it.
#[tokio::test]
async fn a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    let incoherent = vpay_db::ProviderSeed {
        code: "incoherent_rail".to_owned(),
        display_name: "Incoherent Rail".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: true,
        delivers_callbacks: false,
        requires_ip_allowlist: false,
        enabled: true,
    };
    let error = repositories
        .reconcile(
            &[vpay_db::CurrencySeed {
                code: "EUR".to_owned(),
                exponent: 2,
            }],
            std::slice::from_ref(&incoherent),
        )
        .await
        .expect_err("a rail that refunds partially but not at all must be refused by the CHECK");

    // Named, so this test cannot pass on some *other* failure that happened
    // to occur before the currency was written — which would make the
    // assertion below vacuous.
    //
    // `Persistence(Check { .. })` rather than the `Query(sqlx_error)` this
    // read until 2026-09-06: the provider pass is a CrateStack upsert now, so
    // the `23514` arrives as a `CratestackError` and
    // `persistence::classify_cratestack` turns it into the twin of what
    // `error::classify_write` produced. Same constraint name, same
    // `Category::Internal`; a different variant, which is exactly the
    // "a caller matching the variant would have silently stopped matching"
    // the module doc warns about, caught here rather than in production.
    let vpay_db::DbError::Persistence(vpay_db::PersistenceError::Check { constraint, .. }) = &error
    else {
        panic!(
            "expected the constraint violation to surface as \
             DbError::Persistence(PersistenceError::Check), got {error:?}"
        );
    };
    assert_eq!(
        constraint, "partial_refunds_imply_refunds",
        "the transaction must have got as far as the provider upsert and failed THERE; if it \
         failed earlier, the currency was never written and this test proves nothing"
    );

    let currencies_written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM currencies")
        .fetch_one(&pool)
        .await
        .context("counting currencies must succeed")?;
    assert_eq!(
        currencies_written, 0,
        "the CrateStack currency upsert must have been rolled back with everything else. A 1 \
         here means it ran on its own connection instead of joining this transaction — check \
         that `reconcile`'s upsert still says `run_in_tx(&mut tx, &ctx)` and not `run(&ctx)`"
    );

    Ok(())
}

/// Every column of one `providers` row, in declaration order, so an
/// assertion can be about the whole row rather than about the two fields
/// `provider_snapshot` happens to select.
///
/// It exists for `a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile`
/// below, whose whole subject is the five columns migration 0033 made
/// writable: reading only some of them would leave exactly the fields that
/// used to be silently wrong unchecked.
async fn full_provider_row(
    pool: &PgPool,
    code: &str,
) -> anyhow::Result<(String, String, String, bool, bool, bool, bool, bool)> {
    sqlx::query_as(
        "SELECT code, display_name, flow, supports_refunds, supports_partial_refunds, \
         delivers_callbacks, requires_ip_allowlist, enabled FROM providers WHERE code = $1",
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .with_context(|| format!("reading the whole {code} row must succeed"))
}

/// A rail the deployment turns **off** stays off, and every other capability
/// it declares is carried to the stored row — the decisive test for migration
/// 0033 and for the provider pass moving onto CrateStack's `upsert`.
///
/// This is the case that was impossible to pass until 2026-09-06, and the
/// reason is worth stating as a fact about the *previous* code rather than as
/// a description of this one. `cratestack-macros` drops every `@default(...)`
/// field from `Create{Model}Input` **and** from `upsert_update_columns`, and
/// `model Provider` carried a `@default(...)` on all five capability booleans
/// because migration 0002's table did. So a CrateStack upsert would have
/// rendered
///
/// ```text
/// INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
/// ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
/// ```
///
/// and this test would have failed twice over: the first reconcile would have
/// stored `delivers_callbacks = false` where the seed says `true` (the column
/// default winning over the configuration), and the second — the one that
/// matters — would have left `enabled = true` on a rail the deployment had
/// just turned off, because `enabled` is in neither the insert list nor the
/// update list. A rail taking money after an operator disabled it is the
/// worst shape of `AGENTS.md`'s second rule: a plausible-looking success
/// storing the wrong value.
///
/// The third reconcile is not decoration. `enabled = false` is also what the
/// *disable pass* writes for a rail that has left the configuration
/// altogether, so a run that flipped the rail off and then flipped it back on
/// the next boot would be a genuinely different bug with the same first-boot
/// symptom; asserting the row again after an identical repeat is what tells
/// "reconcile wrote what it was told" apart from "reconcile happened to agree
/// once".
#[tokio::test]
async fn a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    let currencies = vec![vpay_db::CurrencySeed {
        code: "XAF".to_owned(),
        exponent: 0,
    }];

    // Deliberately not the column defaults: `delivers_callbacks` is the one
    // that disagrees with `DEFAULT FALSE` on the very first insert, so this
    // seed catches the insert half without needing a second reconcile.
    let live = vpay_db::ProviderSeed {
        code: "mtn_momo".to_owned(),
        display_name: "MTN MoMo".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: false,
        delivers_callbacks: true,
        requires_ip_allowlist: false,
        enabled: true,
    };
    repositories
        .reconcile(&currencies, std::slice::from_ref(&live))
        .await
        .context("the first reconcile must succeed")?;
    assert_eq!(
        full_provider_row(&pool, "mtn_momo").await?,
        (
            "mtn_momo".to_owned(),
            "MTN MoMo".to_owned(),
            "push".to_owned(),
            false,
            false,
            true,
            false,
            true,
        ),
        "the INSERT branch must store the deployment's capabilities. A `delivers_callbacks` of \
         `false` here is the column default winning over the configuration, which is what a \
         `@default(...)` on `model Provider` used to cause"
    );

    // The deployment turns the rail off and, in the same edit, changes every
    // other capability. Same rail, still configured — so the disable pass
    // (`WHERE code <> ALL($1)`) does not touch it and the UPDATE branch of
    // the upsert is the only thing that can produce the row below.
    let disabled = vpay_db::ProviderSeed {
        code: "mtn_momo".to_owned(),
        display_name: "MTN MoMo (paused)".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: true,
        supports_partial_refunds: true,
        delivers_callbacks: false,
        requires_ip_allowlist: true,
        enabled: false,
    };
    repositories
        .reconcile(&currencies, std::slice::from_ref(&disabled))
        .await
        .context("reconciling a disabled rail must succeed")?;
    let expected = (
        "mtn_momo".to_owned(),
        "MTN MoMo (paused)".to_owned(),
        "push".to_owned(),
        true,
        true,
        false,
        true,
        false,
    );
    assert_eq!(
        full_provider_row(&pool, "mtn_momo").await?,
        expected,
        "a capability change must reach an EXISTING row. `enabled = true` here means the rail \
         the deployment disabled is still open for new charges; any other field wrong means the \
         upsert's `DO UPDATE SET` list lost a column — check `model Provider` in \
         schemas/vpay.cstack for a returned `@default(...)`, and \
         `the_provider_upsert_carries_all_eight_columns`"
    );

    // And a repeat is observably a no-op, so the row above is what reconcile
    // *writes* rather than a state it passes through.
    repositories
        .reconcile(&currencies, std::slice::from_ref(&disabled))
        .await
        .context("a second, identical reconcile must succeed")?;
    assert_eq!(
        full_provider_row(&pool, "mtn_momo").await?,
        expected,
        "every replica runs boot step 4; a disabled rail must not come back on the next one"
    );

    // And the other direction, which is a different claim rather than the
    // same one read backwards. Everything above is satisfied by a `reconcile`
    // that can only ever turn capabilities OFF — the disable pass writes
    // `enabled = false` and four of the five column defaults were `false`, so
    // "the row matches the config" and "the row is the pessimistic value"
    // agree on every assertion so far. Turning the rail back ON, with every
    // other capability flipped back with it, is where they stop agreeing: the
    // only statement that can produce this row is the upsert's `DO UPDATE
    // SET`, and the disable pass (`WHERE code <> ALL($1) AND enabled`) has to
    // leave a configured code alone for it to survive to the commit.
    let restored = vpay_db::ProviderSeed {
        code: "mtn_momo".to_owned(),
        display_name: "MTN MoMo".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: false,
        delivers_callbacks: true,
        requires_ip_allowlist: false,
        enabled: true,
    };
    repositories
        .reconcile(&currencies, std::slice::from_ref(&restored))
        .await
        .context("re-enabling a rail must succeed")?;
    assert_eq!(
        full_provider_row(&pool, "mtn_momo").await?,
        (
            "mtn_momo".to_owned(),
            "MTN MoMo".to_owned(),
            "push".to_owned(),
            false,
            false,
            true,
            false,
            true,
        ),
        "an operator who turns a rail back on must get it back. An `enabled` of `false` here \
         means reconcile can disable a rail but not re-enable one — check that the disable pass \
         still excludes configured codes (`WHERE code <> ALL($1)`) and that it still runs AFTER \
         the upserts, not before them"
    );

    Ok(())
}

/// The provider write goes into **vpay's** transaction — the same assertion
/// `a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
/// makes for the currency half, made for the half that moved on 2026-09-06.
///
/// `run_in_tx` versus `run` is the only difference between joining this
/// transaction and quietly opening a private one, and a `.run(&ctx)` here
/// would still compile, still return `Ok`, and still write the right row.
/// The row would then survive the rollback that a later failure triggers, and
/// boot step 4 would report a failure having left a rail behind that nothing
/// in this deployment believes it created.
///
/// Arranged so a valid rail lands *first* and an invalid one fails after it:
/// seeds are iterated in sorted `code` order, `alpha_rail` sorts before
/// `zulu_rail`, and `zulu_rail` sets `supports_partial_refunds` without
/// `supports_refunds`, which migration 0002's `partial_refunds_imply_refunds`
/// CHECK refuses. So `alpha_rail` is written through CrateStack, then the
/// second upsert raises `23514`, then `tx` drops. If `alpha_rail` is still
/// there afterwards, the write was never in this transaction at all.
///
/// **Why this fails fast rather than hanging**, which is the property that
/// makes it useful: exp17's review measured that swapping the *currency*
/// upsert to `.run(&ctx)` made `reconcile_is_idempotent_and_disables_a_
/// dropped_provider_code` hang rather than fail — `upsert`'s own conflict
/// probe is `SELECT … FOR UPDATE`, and off the transaction it waits on the
/// row lock `find_unique(...).for_update()` is holding. The provider pass has
/// no such read (see `config_reconcile`'s comment on that loop for why it
/// needs none), so nothing in this transaction holds a `providers` row lock
/// when the mutated call runs, and both rails here are INSERTs whose conflict
/// probe finds and locks nothing. Measured 2026-09-06: under `.run(&ctx)`
/// this test fails on the assertion below in about a second, and no test in
/// the crate hangs.
#[tokio::test]
async fn a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    let valid = vpay_db::ProviderSeed {
        code: "alpha_rail".to_owned(),
        display_name: "Alpha Rail".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: true,
        supports_partial_refunds: true,
        delivers_callbacks: true,
        requires_ip_allowlist: false,
        enabled: true,
    };
    let incoherent = vpay_db::ProviderSeed {
        code: "zulu_rail".to_owned(),
        display_name: "Zulu Rail".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: true,
        delivers_callbacks: false,
        requires_ip_allowlist: false,
        enabled: true,
    };

    let error = repositories
        .reconcile(&[], &[valid.clone(), incoherent.clone()])
        .await
        .expect_err("a rail that refunds partially but not at all must be refused by the CHECK");

    // Named, so the assertion below cannot be satisfied by a run that failed
    // before `alpha_rail` was ever written.
    let vpay_db::DbError::Persistence(vpay_db::PersistenceError::Check { constraint, .. }) = &error
    else {
        panic!(
            "expected the constraint violation to surface as \
             DbError::Persistence(PersistenceError::Check), got {error:?}"
        );
    };
    assert_eq!(
        constraint, "partial_refunds_imply_refunds",
        "the transaction must have got as far as the SECOND provider upsert and failed there; if \
         it failed earlier, `alpha_rail` was never written and this test proves nothing"
    );

    // The category, pinned because it MOVED when this statement became a
    // CrateStack upsert and nothing said so. `error::classify_write` leaves a
    // `23514` in the unclassified `DbError::Query` bucket -> `Storage` ->
    // exit 69; `persistence::classify_cratestack` gives it
    // `PersistenceError::Check` -> `Internal` -> exit 1. So a boot against an
    // adapter whose declared `Capabilities` are incoherent told a supervisor
    // "wait for Postgres" until 2026-09-06 and pages someone now. Whether
    // `Configuration` (78) would be better still is a maintainer's call
    // (docs/status.md); this assertion is what makes the next move
    // deliberate instead of accidental.
    assert_eq!(
        vpay_core::Classify::category(&error),
        vpay_core::Category::Internal,
        "a CHECK the application was supposed to satisfy is vpay's own bug, not a storage \
         outage. If this is `Storage` again, the provider pass has gone back to raw sqlx"
    );
    assert_eq!(vpay_core::Classify::category(&error).exit_code(), 1);

    let providers_written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&pool)
        .await
        .context("counting providers must succeed")?;
    assert_eq!(
        providers_written, 0,
        "the CrateStack provider upsert must have been rolled back with everything else. A 1 \
         here is `alpha_rail`, written on its own connection instead of joining this transaction \
         — check that `reconcile`'s provider upsert still says `run_in_tx(&mut tx, &ctx)` and \
         not `run(&ctx)`"
    );

    Ok(())
}

/// `reconcile` itself refuses a flow label the schema's enum cannot name, and
/// has written nothing when it does — asserted **through the public trait,
/// against a real database**.
///
/// It exists because the no-container
/// `an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement` in
/// `config_reconcile.rs` cannot make this claim: that test constructs
/// `DbError::ProviderFlowUnknown` by hand and asserts how it *classifies*. It
/// never calls `reconcile`, so it stays green whatever `reconcile` does with
/// a bad label.
///
/// **The mutation this exists for, measured on 2026-09-06 before it was
/// written.** Replace `reconcile`'s `.parse().map_err(...)` with
/// `.parse().unwrap_or_default()` — the exact trap the comment above that
/// call warns about, because `cratestack-macros` marks the FIRST variant of
/// every generated enum `#[default]` (`types/enums.rs::variant_tokens`) and
/// the first variant of `ProviderFlow` is `push`. The whole `vpay-db` suite
/// passed under that mutation, 112/112: boot recorded `typo_rail` as a
/// **push rail** and returned `Ok`. That is `AGENTS.md`'s second rule
/// exactly — a plausible-looking success storing the wrong value — and the
/// provider count below is what turns it red.
///
/// The currency count is not decoration either. The parse is inside the
/// provider loop, which runs *after* every currency has been upserted, so
/// `XAF` is written and must then be rolled back with everything else; a
/// non-zero there would be boot step 4 left half-applied by a refusal.
#[tokio::test]
async fn a_flow_label_the_schema_cannot_name_is_refused_by_reconcile_before_any_row_is_written()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    let currencies = vec![vpay_db::CurrencySeed {
        code: "XAF".to_owned(),
        exponent: 0,
    }];
    let typo = vpay_db::ProviderSeed {
        code: "typo_rail".to_owned(),
        display_name: "Typo Rail".to_owned(),
        flow: "redirekt".to_owned(),
        supports_refunds: true,
        supports_partial_refunds: false,
        delivers_callbacks: false,
        requires_ip_allowlist: false,
        enabled: true,
    };

    let error = repositories
        .reconcile(&currencies, std::slice::from_ref(&typo))
        .await
        .expect_err("a flow that is neither `push` nor `redirect` must be refused");

    let vpay_db::DbError::ProviderFlowUnknown { code, flow } = &error else {
        panic!(
            "expected DbError::ProviderFlowUnknown, got {error:?}. If this is an `Ok`, \
             `reconcile` swallowed the label — check that the parse is a `map_err` and not \
             `unwrap_or_default()`, whose default is the first variant, `push`"
        );
    };
    assert_eq!(code, "typo_rail");
    assert_eq!(flow, "redirekt");

    // Exit 78 ("fix the deploy"), never 69 ("wait for Postgres"): the
    // database is healthy and no amount of waiting turns `redirekt` into a
    // flow. Both binaries' `exit_code_for` reads exactly this category out of
    // the `anyhow` chain with `find_in_chain::<DbError>`, so pinning the
    // category here pins the code a supervisor sees.
    assert_eq!(
        vpay_core::Classify::category(&error),
        vpay_core::Category::Configuration,
        "a typo in a deployment must not be reported as a storage outage"
    );
    assert_eq!(vpay_core::Classify::category(&error).exit_code(), 78);
    assert_eq!(vpay_core::Classify::retry(&error), vpay_core::Retry::Never);

    let providers_written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&pool)
        .await
        .context("counting providers must succeed")?;
    assert_eq!(
        providers_written, 0,
        "a 1 here is `typo_rail` stored with the FIRST `ProviderFlow` variant — `push` — which \
         is precisely what `.unwrap_or_default()` in `reconcile` does, silently and with an `Ok`"
    );
    let currencies_written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM currencies")
        .fetch_one(&pool)
        .await
        .context("counting currencies must succeed")?;
    assert_eq!(
        currencies_written, 0,
        "the refusal happens inside the transaction the currency pass has already written to, \
         so it has to roll that back as well: boot step 4 is all-or-nothing"
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

/// `reconcile` reads the stored exponent **under a row lock**, so a writer
/// that is not another `reconcile` cannot slip a change in between the read
/// and the upsert.
///
/// **This is the test that fails if `.for_update()` is removed from the
/// currency read**, and it is deliberately not
/// `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`
/// below. That one passes with or without the row lock — measured on
/// 2026-09-06, and recorded in `docs/plans/exp17-notes/opus.md` — because the
/// advisory lock is what serialises boot against boot. So does every other
/// test in this file: the whole suite was green with `.for_update()` deleted
/// until this case was written. Two guards, two tests, and neither covers the
/// other's.
///
/// The interleaving is deterministic rather than raced, which is what makes
/// this a test rather than a flake:
///
/// 1. `XAF` is seeded at exponent 0 by hand.
/// 2. A transaction this test controls issues `UPDATE ... SET exponent = 3`
///    and does **not** commit. It now holds the row's write lock, and the
///    value 3 is invisible to every other snapshot.
/// 3. `reconcile` starts with a seed of `XAF` at exponent 0 — a seed that
///    *agrees* with what is committed and *disagrees* with what is about to
///    be. It takes the advisory lock (free), then blocks on the row.
/// 4. The blocker commits. The read unblocks and sees 3.
///
/// With `.for_update()` the read is what blocked, so it returns the
/// *post-commit* 3 and boot refuses: `CurrencyExponentConflict { stored: 3,
/// seeded: 0 }`. Without it the plain `SELECT` would have returned the
/// pre-commit 0 immediately, the comparison would have passed, and the
/// upsert — whose own internal probe blocks on the same row — would then have
/// written `exponent = 0` over the committed 3, silently. That is the race
/// the row lock closes, and the assertion on the stored value at the end is
/// what distinguishes the two outcomes rather than merely observing that
/// something was refused.
#[tokio::test]
async fn reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer()
-> anyhow::Result<()> {
    use std::time::Duration;

    let (_container, repositories, pool) = migrated_postgres().await?;
    let (mtn, _orange) = reconcile_rails();

    sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('XAF', 0)")
        .execute(&pool)
        .await
        .context("seeding XAF at exponent 0 must succeed")?;

    // A writer that is not a `reconcile`, so the advisory lock does not bind
    // it: an operator's `psql`, a data fix, a future admin surface.
    let mut blocker = pool
        .begin()
        .await
        .context("the blocker transaction begins")?;
    sqlx::query("UPDATE currencies SET exponent = 3 WHERE code = 'XAF'")
        .execute(&mut *blocker)
        .await
        .context("the uncommitted competing update must succeed")?;

    let mut reconciling = tokio::spawn({
        let repositories = Arc::clone(&repositories);
        let providers = vec![mtn.clone()];
        let currencies = vec![vpay_db::CurrencySeed {
            code: "XAF".to_owned(),
            exponent: 0,
        }];
        async move { repositories.reconcile(&currencies, &providers).await }
    });

    // It must not get past the currency row while the write lock is held.
    // Same window and same reasoning as the boot-lock test below.
    let finished_while_locked = tokio::time::timeout(Duration::from_secs(3), &mut reconciling)
        .await
        .is_ok();
    assert!(
        !finished_while_locked,
        "reconcile finished while another transaction held the `currencies` row's write lock; \
         it cannot have read the row under `FOR UPDATE`"
    );

    blocker
        .commit()
        .await
        .context("releasing the row lock must succeed")?;

    let outcome = reconciling
        .await
        .context("the reconcile task must not panic")?;
    let error = outcome.expect_err(
        "the read must see the committed 3 and refuse a seed of 0. If this returned `Ok`, the \
         read did not block on the row and boot has just overwritten another writer's value",
    );
    match &error {
        vpay_db::DbError::CurrencyExponentConflict {
            code,
            stored,
            seeded,
        } => {
            assert_eq!(code, "XAF");
            assert_eq!(
                *stored, 3,
                "the refusal must name the value the OTHER writer committed, which is only \
                 visible to a read that waited for it"
            );
            assert_eq!(*seeded, 0);
        }
        other => panic!("expected a named exponent conflict, got {other:?}"),
    }

    let stored_exponent: i64 =
        sqlx::query_scalar("SELECT exponent FROM currencies WHERE code = 'XAF'")
            .fetch_one(&pool)
            .await
            .context("re-reading the exponent must succeed")?;
    assert_eq!(
        stored_exponent, 3,
        "the other writer's value must survive. A 0 here is the exact silent clobber this row \
         lock exists to prevent: boot read a stale 0, agreed with itself, and wrote it back over \
         a committed 3"
    );

    Ok(())
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

    let (_container, repositories, pool) = migrated_postgres().await?;
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
        let repositories = Arc::clone(&repositories);
        let providers = vec![mtn.clone(), orange.clone()];
        let currencies = currencies.clone();
        async move { repositories.reconcile(&currencies, &providers).await }
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

/// A `reconcile` that waited for `lock_keys::CONFIG_RECONCILE` converges on
/// **its own** configuration over the row the holder committed while it
/// waited — on all eight columns.
///
/// This is the question the absent `find_unique(...).for_update()` on the
/// provider pass raises, asked as an assertion rather than answered in a
/// comment. The currency pass reads under a row lock because it has to
/// *compare* before it writes; the provider pass has nothing to compare, so
/// the only thing standing between two boots of different configurations is
/// the advisory lock and the conflict probe `upsert` takes for itself
/// (`upsert_exec.rs::run_upsert_in_tx` → `select_for_update_by_conflict_
/// target` on `tx`). `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_
/// is_released` above proves the waiting half but releases the lock by
/// **rollback**, so it never asks what the waiter does with state that
/// actually landed.
///
/// The blocker here is deliberately *not* another `reconcile`: it is a plain
/// writer holding the same advisory lock, which is the case the module
/// comment describes the currency row lock as binding. What this proves is
/// that the provider pass needs no such lock — the row it finds is the
/// committed one, the upsert takes its UPDATE branch, and the eight columns
/// that come out are the waiter's configuration and not a mixture of the two.
///
/// **The mutation it kills, measured 2026-09-06.** Add `SET TRANSACTION
/// ISOLATION LEVEL REPEATABLE READ` after `reconcile`'s `pool.begin()` — a
/// plausible "make boot safer" edit. This test then FAILS in 4.2 s with
/// `could not serialize access due to concurrent update` (`40001`), because
/// the transaction's snapshot is taken when the `pg_advisory_xact_lock`
/// statement *starts*, which is before the holder commits, so the upsert's
/// conflict probe cannot see the row and the INSERT collides with it. Every
/// other reconcile case passes under that mutation, including
/// `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` and
/// `two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge`.
/// So boot step 4 depends on **READ COMMITTED**, and this is the only test
/// that says so.
///
/// If this ever goes red on the `enabled` column alone, suspect the disable
/// pass; if it goes red as a *mixture* of the two configurations, the
/// `DO UPDATE SET` list has lost a column and
/// `the_provider_upsert_carries_all_eight_columns` is the faster witness.
#[tokio::test]
async fn a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed()
-> anyhow::Result<()> {
    use std::time::Duration;

    let (_container, repositories, pool) = migrated_postgres().await?;
    let currencies = vec![vpay_db::CurrencySeed {
        code: "XAF".to_owned(),
        exponent: 0,
    }];

    // The holder: the reconcile lock plus a row of its own, uncommitted.
    let mut blocker = pool
        .begin()
        .await
        .context("the blocker transaction begins")?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(vpay_db::lock_keys::CONFIG_RECONCILE)
        .execute(&mut *blocker)
        .await
        .context("taking the reconcile lock must succeed")?;
    sqlx::query(
        "INSERT INTO providers \
            (code, display_name, flow, supports_refunds, supports_partial_refunds, \
             delivers_callbacks, requires_ip_allowlist, enabled) \
         VALUES ('mtn_momo', 'Stale Name', 'redirect', true, true, true, true, true)",
    )
    .execute(&mut *blocker)
    .await
    .context("the blocker's own write must succeed")?;

    // The waiter, whose configuration disagrees with the holder's row on
    // every one of the eight columns except the primary key.
    let seed = vpay_db::ProviderSeed {
        code: "mtn_momo".to_owned(),
        display_name: "MTN MoMo".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: false,
        delivers_callbacks: false,
        requires_ip_allowlist: false,
        enabled: false,
    };
    let mut reconciling = tokio::spawn({
        let repositories = Arc::clone(&repositories);
        let currencies = currencies.clone();
        let providers = vec![seed.clone()];
        async move { repositories.reconcile(&currencies, &providers).await }
    });

    let finished_while_locked = tokio::time::timeout(Duration::from_secs(3), &mut reconciling)
        .await
        .is_ok();
    assert!(
        !finished_while_locked,
        "reconcile completed while another writer held lock_keys::CONFIG_RECONCILE"
    );

    // Release by COMMIT, not rollback: the waiter must now meet a row that
    // exists.
    blocker
        .commit()
        .await
        .context("committing the blocker must succeed")?;
    tokio::time::timeout(Duration::from_secs(30), reconciling)
        .await
        .context("reconcile must proceed once the lock is released")?
        .context("the reconcile task must not panic")?
        .context("the reconcile itself must succeed against a row that now exists")?;

    assert_eq!(
        full_provider_row(&pool, "mtn_momo").await?,
        (
            "mtn_momo".to_owned(),
            "MTN MoMo".to_owned(),
            "push".to_owned(),
            false,
            false,
            false,
            false,
            false,
        ),
        "the reconcile that waited must have overwritten every column of the committed row with \
         its own configuration. Any value from the blocker's row surviving here is the provider \
         pass reading, or writing, something other than what this deployment configured"
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
    let (_container, repositories, pool) = migrated_postgres().await?;
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
        repositories.reconcile(&forward_currencies, &forward),
        repositories.reconcile(&reverse_currencies, &reverse),
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_attempts", "XAF"))
        .await?;
    let charge = one_tx::insert_for_intent(
        repositories.as_ref(),
        &fixture_charge("ch_attempts", "pi_attempts"),
    )
    .await
    .context("opening the charge must succeed")?;

    let first = repositories
        .insert_pending(
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
    repositories
        .record_response(first, None, Some("not_implemented"))
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
    let second = repositories
        .insert_pending(
            &charge.id,
            "mtn_momo",
            "query_status",
            charge.provider_reference_id,
            2,
        )
        .await
        .context("a second attempt for the same charge must be accepted")?;
    assert_ne!(first, second);

    repositories
        .record_response(second, Some(200), None)
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
    let missing = repositories
        .record_response(9_999_999, Some(200), None)
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

/// A resubmit that answers with nothing must not erase the rail key material
/// an earlier answer left.
///
/// `charges.provider_ref_extra` is `vpay_provider::RefExtra` — on a redirect
/// rail the `pay_token` in it is the only thing that can ever query the
/// charge again (`docs/flows/crash-safety.md`). `vpay_worker`'s
/// `resubmit_charge` calls `mark_submitted` with whatever the *second* submit
/// answered, and a push rail answers with an empty map; assigning that map
/// would replace key material with `{}` and leave a charge nobody can ask
/// about. So the write is a merge.
///
/// Driven through `mark_submitted` twice against two different charges rather
/// than one, because the state guard (`submitting`) means one charge can only
/// take that write once — which is also why this is defence against the next
/// caller rather than a bug that is reachable today.
#[tokio::test]
async fn mark_submitted_merges_ref_extra_and_never_erases_it() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    repositories
        .insert(&fixture_intent("pi_merge", "XAF"))
        .await?;

    // A charge that already holds a rail's key material when the write lands
    // — the shape a callback repair, or a second attempt on a redirect rail,
    // would present.
    let mut with_token = fixture_charge("ch_merge", "pi_merge");
    with_token.provider_ref_extra = Some(json!({ "pay_token": "tok_abc" }));
    one_tx::insert_for_intent(repositories.as_ref(), &with_token)
        .await
        .context("opening the charge must succeed")?;

    // The empty answer a push rail gives a resubmit.
    let merged = one_tx::mark_submitted(
        repositories.as_ref(),
        "ch_merge",
        "submitted",
        Some(&json!({})),
        None,
    )
    .await
    .context("recording the resubmit's answer must succeed")?;
    assert_eq!(
        merged.provider_ref_extra,
        Some(json!({ "pay_token": "tok_abc" })),
        "an empty answer erased the rail key material; that charge can never be queried \
         again"
    );

    // A `None` argument — "the answer carried nothing at all" — is also not
    // an erasure.
    let mut none_arg = fixture_charge("ch_merge_none", "pi_merge_none");
    none_arg.provider_ref_extra = Some(json!({ "pay_token": "tok_xyz" }));
    repositories
        .insert(&fixture_intent("pi_merge_none", "XAF"))
        .await?;
    let untouched = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.insert_for_intent(&none_arg)
                    .await
                    .context("opening the charge must succeed")?;
                let untouched = tx
                    .mark_submitted("ch_merge_none", "submitted", None, None)
                    .await
                    .context("recording an answer with no key material must succeed")?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(untouched))
            })
        })
        .await?
        .into_inner();
    assert_eq!(
        untouched.provider_ref_extra,
        Some(json!({ "pay_token": "tok_xyz" })),
        "a NULL argument must leave the column alone rather than nulling it"
    );

    // And a real answer still wins, per key: this is a merge, not a refusal
    // to write.
    let mut fresh = fixture_charge("ch_merge_new", "pi_merge_new");
    fresh.provider_ref_extra = Some(json!({ "pay_token": "tok_old", "keep": "me" }));
    repositories
        .insert(&fixture_intent("pi_merge_new", "XAF"))
        .await?;
    let replaced = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.insert_for_intent(&fresh)
                    .await
                    .context("opening the charge must succeed")?;
                let replaced = tx
                    .mark_submitted(
                        "ch_merge_new",
                        "submitted",
                        Some(&json!({ "pay_token": "tok_new" })),
                        None,
                    )
                    .await
                    .context("recording a fresh token must succeed")?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(replaced))
            })
        })
        .await?
        .into_inner();
    assert_eq!(
        replaced.provider_ref_extra,
        Some(json!({ "pay_token": "tok_new", "keep": "me" })),
        "the rail's newer value must win for the keys it names, and only those"
    );

    Ok(())
}

/// A second answer that carries no URL must not blank the one a payer is
/// standing on.
///
/// `charges.redirect_url` is what `GET /v1/payment_intents/{id}` renders as
/// `next_action` (`vpay_api::v1::payment_intents`), and on a redirect rail the
/// payer is handed it strictly *after* it is committed
/// (`docs/flows/crash-safety.md`). Assigning `$4` blindly would let a
/// resubmit whose answer had no URL leave an intent in `requires_action` with
/// nothing to act on — the charge still live, the address gone. So the write
/// is `COALESCE($4, redirect_url)`: `NULL` means "this answer carried no
/// URL", never "there is no URL".
///
/// Same shape as [`mark_submitted_merges_ref_extra_and_never_erases_it`], and
/// for the same reason it uses a charge per claim: the `submitting` guard
/// means one charge can only take this write once.
#[tokio::test]
async fn mark_submitted_never_erases_the_redirect_url() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    // A charge that already carries the rail's URL when a second answer
    // lands — the shape a resubmit on a redirect rail would present.
    repositories
        .insert(&fixture_intent("pi_url_keep", "XAF"))
        .await?;
    let mut holding_url = fixture_charge("ch_url_keep", "pi_url_keep");
    holding_url.redirect_url = Some("https://pay.example/abc".to_owned());
    let kept = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.insert_for_intent(&holding_url)
                    .await
                    .context("opening the charge must succeed")?;
                let kept = tx
                    .mark_submitted("ch_url_keep", "submitted", None, None)
                    .await
                    .context("recording an answer with no URL must succeed")?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(kept))
            })
        })
        .await?
        .into_inner();
    assert_eq!(
        kept.redirect_url.as_deref(),
        Some("https://pay.example/abc"),
        "a NULL argument erased the only address the payer can pay at; the intent would          read `requires_action` with no next_action"
    );

    // A real answer still wins: this is a merge, not a refusal to write.
    repositories
        .insert(&fixture_intent("pi_url_new", "XAF"))
        .await?;
    let mut replacing = fixture_charge("ch_url_new", "pi_url_new");
    replacing.redirect_url = Some("https://pay.example/old".to_owned());
    let replaced = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.insert_for_intent(&replacing)
                    .await
                    .context("opening the charge must succeed")?;
                let replaced = tx
                    .mark_submitted(
                        "ch_url_new",
                        "submitted",
                        None,
                        Some("https://pay.example/new"),
                    )
                    .await
                    .context("recording a fresh URL must succeed")?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(replaced))
            })
        })
        .await?
        .into_inner();
    assert_eq!(
        replaced.redirect_url.as_deref(),
        Some("https://pay.example/new"),
        "the rail's newer URL must win; COALESCE must not become a refusal to update"
    );

    // And the ordinary push-rail confirm: nothing stored, nothing answered,
    // nothing written.
    repositories
        .insert(&fixture_intent("pi_url_none", "XAF"))
        .await?;
    let still_none = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.insert_for_intent(&fixture_charge("ch_url_none", "pi_url_none"))
                    .await
                    .context("opening the charge must succeed")?;
                let still_none = tx
                    .mark_submitted("ch_url_none", "submitted", None, None)
                    .await
                    .context("recording a push rail's answer must succeed")?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(still_none))
            })
        })
        .await?
        .into_inner();
    assert_eq!(
        still_none.redirect_url, None,
        "a push rail has no URL and must not acquire one"
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_submitted", "XAF"))
        .await?;
    let mut new = fixture_charge("ch_submitted", "pi_submitted");
    // The merchant's own return destination, written before the rail is
    // called (migration 0019).
    new.return_url = Some("https://shop.example/order/1234/return".to_owned());
    let charge = one_tx::insert_for_intent(repositories.as_ref(), &new)
        .await
        .context("opening the charge must succeed")?;
    assert_eq!(
        charge.return_url.as_deref(),
        Some("https://shop.example/order/1234/return"),
        "the return_url is durable before anything is submitted"
    );
    assert_eq!(charge.redirect_url, None, "no rail has answered yet");

    let ref_extra = json!({ "pay_token": "tok_abc" });
    let submitted = one_tx::mark_submitted(
        repositories.as_ref(),
        &charge.id,
        "submitted",
        Some(&ref_extra),
        Some("https://webpayment.example/pay/tok_abc"),
    )
    .await
    .context("recording the rail's answer must succeed")?;

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
    let refused = one_tx::mark_submitted(
        repositories.as_ref(),
        &charge.id,
        "submitted",
        Some(&json!({ "pay_token": "tok_second" })),
        Some("https://webpayment.example/pay/tok_second"),
    )
    .await;
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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_declined", "XAF"))
        .await?;
    let charge = one_tx::insert_for_intent(
        repositories.as_ref(),
        &fixture_charge("ch_declined", "pi_declined"),
    )
    .await
    .context("opening the charge must succeed")?;

    let (failed, intent) = repositories
        .transaction(|tx| {
            let charge = &charge;
            Box::pin(async move {
                let failed = tx
                    .mark_failed(&charge.id, "insufficient_funds", "NOT_ENOUGH_FUNDS")
                    .await
                    .context("failing the charge must succeed")?;
                let intent = tx
                    .record_payment_error(
                        "merchant_a",
                        "pi_declined",
                        "requires_payment_method",
                        "insufficient_funds",
                        "The payment was declined (insufficient_funds).",
                    )
                    .await
                    .context("recording the payment error must succeed")?
                    .context(
                        "the intent is still requires_payment_method, so the guard must have \
                         matched",
                    )?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit((failed, intent)))
            })
        })
        .await?
        .into_inner();

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
    let stale = one_tx::record_payment_error(
        repositories.as_ref(),
        "merchant_a",
        "pi_declined",
        "processing",
        "payer_timeout",
        "The payment was declined (payer_timeout).",
    )
    .await
    .context("a guarded write that matches nothing is not an error")?;
    assert!(
        stale.is_none(),
        "the intent is not `processing`, so the write must match nothing"
    );

    // A foreign merchant cannot stamp an error onto someone else's intent.
    let foreign = one_tx::record_payment_error(
        repositories.as_ref(),
        "merchant_b",
        "pi_declined",
        "requires_payment_method",
        "payer_timeout",
        "The payment was declined (payer_timeout).",
    )
    .await
    .context("a tenancy-scoped write that matches nothing is not an error")?;
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_atomic", "XAF"))
        .await?;
    let charge = one_tx::insert_for_intent(
        repositories.as_ref(),
        &fixture_charge("ch_atomic", "pi_atomic"),
    )
    .await
    .context("opening the charge must succeed")?;

    repositories
        .transaction(|tx| {
            let charge = &charge;
            Box::pin(async move {
                tx.mark_submitted(
                    &charge.id,
                    "submitted",
                    Some(&json!({ "pay_token": "tok_rolled_back" })),
                    Some("https://webpayment.example/pay/tok_rolled_back"),
                )
                .await
                .context("the charge update must succeed inside the transaction")?;
                tx.transition_in_tx(
                    "merchant_a",
                    "pi_atomic",
                    "requires_payment_method",
                    "requires_action",
                )
                .await
                .context("the intent transition must succeed inside the transaction")?
                .context("the guard must have matched")?;
                // The crash: everything above is discarded.
                Ok::<_, anyhow::Error>(TxOutcome::Abandon(()))
            })
        })
        .await?;

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

// --- jobs and settlement (migration 0021, Step 4) --------------------------

/// A live charge on a confirmed intent, in whatever pair of states the test
/// under discussion needs.
///
/// The two are set directly rather than driven through `confirm`, because
/// what is under test here is the *settlement* half of the lifecycle and the
/// shapes it has to cope with — `submitted`/`processing` (a push rail),
/// `pending`/`requires_action` (a redirect rail whose payer has been sent
/// away), and the invariant-violating pairs a broken database could present.
/// Reaching those through the API would test the API.
async fn live_charge(
    repositories: &dyn Repositories,
    intent_id: &str,
    charge_id: &str,
    intent_status: &str,
    charge_state: &str,
) -> anyhow::Result<()> {
    let mut intent = fixture_intent(intent_id, "XAF");
    intent.status = intent_status.to_owned();
    repositories
        .insert(&intent)
        .await
        .context("inserting the intent must succeed")?;

    let mut charge = fixture_charge(charge_id, intent_id);
    charge.state = charge_state.to_owned();
    one_tx::insert_for_intent(repositories, &charge)
        .await
        .context("opening the charge must succeed")?;

    Ok(())
}

/// Enqueues one job in its own committed transaction — the pooled
/// convenience `vpay_db::jobs` deliberately does not offer, because a
/// *caller* enqueueing outside the transaction that creates the work is the
/// bug that module exists to prevent. A test setting up a queue has no such
/// work to be in step with.
async fn enqueue(
    repositories: &dyn Repositories,
    kind: &str,
    dedupe_key: &str,
    run_at: time::OffsetDateTime,
) -> anyhow::Result<bool> {
    let inserted = one_tx::enqueue_in_tx(
        repositories,
        kind,
        dedupe_key,
        &json!({ "charge_id": "ch_x" }),
        run_at,
    )
    .await
    .context("enqueueing must succeed")?;
    Ok(inserted)
}

/// How many `events` rows name this object. The settlement transaction's
/// central claim is "exactly one", and a count is the only assertion that
/// can catch the failure that matters (a second delivery to a merchant).
async fn event_count(pool: &PgPool, object_id: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE object_id = $1")
        .bind(object_id)
        .fetch_one(pool)
        .await
        .context("counting events must succeed")
}

/// Reads a charge's stored state without going through the repository, so an
/// assertion about what was *committed* cannot be satisfied by whatever the
/// writer happened to return.
async fn charge_state(pool: &PgPool, charge_id: &str) -> anyhow::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT state::TEXT FROM charges WHERE id = $1")
        .bind(charge_id)
        .fetch_one(pool)
        .await
        .context("re-reading the charge must succeed")
}

/// `claim` takes the *oldest* runnable job, and leaves a job whose `run_at`
/// has not arrived alone. Without the second half, a poll ladder would be
/// decorative: every rescheduled job would be claimed again immediately.
#[tokio::test]
async fn claim_takes_the_earliest_runnable_job_and_leaves_the_future_one() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();

    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:newer",
        now - time::Duration::seconds(10),
    )
    .await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:older",
        now - time::Duration::seconds(60),
    )
    .await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:future",
        now + time::Duration::hours(1),
    )
    .await?;

    let first = Jobs::claim(repositories.as_ref(), "worker-1")
        .await?
        .context("a runnable job must be claimable")?;
    assert_eq!(
        first.dedupe_key, "poll:older",
        "the oldest runnable job wins"
    );
    assert_eq!(first.attempts, 1, "the claim itself counts the attempt");
    assert_eq!(first.locked_by.as_deref(), Some("worker-1"));

    let second = Jobs::claim(repositories.as_ref(), "worker-1")
        .await?
        .context("the second runnable job must be claimable")?;
    assert_eq!(second.dedupe_key, "poll:newer");

    assert!(
        Jobs::claim(repositories.as_ref(), "worker-1")
            .await?
            .is_none(),
        "a job scheduled an hour out must not be claimable now"
    );

    Ok(())
}

/// Eight workers racing for **one** job: exactly one may win. Two workers
/// running the same `poll_charge` would query the rail twice and race each
/// other into the settlement transaction.
#[tokio::test]
async fn eight_concurrent_claims_over_one_job_yield_exactly_one_claim() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:contended",
        time::OffsetDateTime::now_utc(),
    )
    .await?;

    const WORKERS: usize = 8;
    let mut handles = Vec::with_capacity(WORKERS);
    for worker in 0..WORKERS {
        let repositories = Arc::clone(&repositories);
        handles.push(tokio::spawn(async move {
            Jobs::claim(repositories.as_ref(), &format!("worker-{worker}")).await
        }));
    }

    let mut claimed = Vec::new();
    for handle in handles {
        if let Some(job) = handle.await.context("the claim task must not panic")?? {
            claimed.push(job);
        }
    }

    assert_eq!(
        claimed.len(),
        1,
        "exactly one of {WORKERS} concurrent claims may take the single job; got {claimed:?}"
    );

    Ok(())
}

/// The other half of the same property, and the one that fails if
/// `FOR UPDATE SKIP LOCKED` is dropped: eight workers racing for **eight**
/// jobs must take eight *different* ones and none must come away empty.
///
/// A plain `WHERE locked_at IS NULL … LIMIT 1` subquery makes every worker
/// pick the same candidate row, block on it, and then match nothing once the
/// winner commits — so seven of the eight claim nothing while seven jobs sit
/// runnable. That is not a correctness failure a "exactly one winner" test
/// can see (it also returns one winner); it is a queue that does not scale
/// past one worker, and this is the test that catches it.
#[tokio::test]
async fn eight_concurrent_claims_over_eight_jobs_take_eight_distinct_jobs() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();

    const WORKERS: usize = 8;
    for n in 0..WORKERS {
        enqueue(
            repositories.as_ref(),
            "poll_charge",
            &format!("poll:{n}"),
            now - time::Duration::seconds(i64::try_from(n).unwrap_or(0)),
        )
        .await?;
    }

    let mut handles = Vec::with_capacity(WORKERS);
    for worker in 0..WORKERS {
        let repositories = Arc::clone(&repositories);
        handles.push(tokio::spawn(async move {
            Jobs::claim(repositories.as_ref(), &format!("worker-{worker}")).await
        }));
    }

    let mut keys = std::collections::BTreeSet::new();
    let mut empty = 0;
    for handle in handles {
        match handle.await.context("the claim task must not panic")?? {
            Some(job) => {
                assert!(
                    keys.insert(job.dedupe_key.clone()),
                    "two workers claimed {}",
                    job.dedupe_key
                );
            }
            None => empty += 1,
        }
    }

    assert_eq!(
        empty, 0,
        "no worker may come away empty while {WORKERS} jobs are runnable — SKIP LOCKED is what \
         makes a contended claim take the *next* job rather than nothing"
    );
    assert_eq!(
        keys.len(),
        WORKERS,
        "every job must have been claimed exactly once"
    );

    Ok(())
}

/// A leased job is invisible to `claim` until something releases it. This is
/// the property the lease exists for, stated on its own so a change to the
/// claim predicate cannot pass by accident.
#[tokio::test]
async fn a_leased_job_is_invisible_to_claim() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    enqueue(
        repositories.as_ref(),
        "sweep_expired",
        "sweep:expired",
        time::OffsetDateTime::now_utc(),
    )
    .await?;

    let job = Jobs::claim(repositories.as_ref(), "worker-1")
        .await?
        .context("the job must be claimable once")?;

    assert!(
        Jobs::claim(repositories.as_ref(), "worker-2")
            .await?
            .is_none(),
        "a job another worker holds must not be claimable"
    );

    assert!(
        repositories.release_all("worker-1").await? == 1,
        "the drain path must hand the lease back"
    );
    let reclaimed = Jobs::claim(repositories.as_ref(), "worker-2")
        .await?
        .context("a released job must be claimable again")?;
    assert_eq!(reclaimed.id, job.id);
    assert_eq!(
        reclaimed.attempts, 2,
        "attempts count claims, so a job that is handed back and retaken is visibly on its \
         second attempt"
    );

    Ok(())
}

/// `finish` is guarded on the lease holder. A worker whose lease was reaped
/// as stale must not delete the job the worker that took it over is still
/// running — that is the ABA the `locked_by` guard closes.
#[tokio::test]
async fn finish_with_the_wrong_worker_id_deletes_nothing() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:aba",
        time::OffsetDateTime::now_utc(),
    )
    .await?;

    let job = Jobs::claim(repositories.as_ref(), "worker-1")
        .await?
        .context("the job must be claimable")?;

    assert!(
        !repositories.finish(job.id, "worker-2").await?,
        "a worker that does not hold the lease must not be told it finished the job"
    );
    let survivors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE id = $1")
        .bind(job.id)
        .fetch_one(&pool)
        .await
        .context("counting the job must succeed")?;
    assert_eq!(survivors, 1, "the job must still be there");

    assert!(
        repositories.finish(job.id, "worker-1").await?,
        "the lease holder finishes it"
    );
    let survivors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE id = $1")
        .bind(job.id)
        .fetch_one(&pool)
        .await
        .context("counting the job must succeed")?;
    assert_eq!(survivors, 0, "a finished job is deleted, not flagged");

    Ok(())
}

/// `reschedule` releases the lease and moves `run_at` in one write, and
/// records a `last_error` bounded to the column's 2000 characters — the
/// bound matters because a refused write here would leave the job leased
/// with nothing saying why it did not finish.
#[tokio::test]
async fn reschedule_clears_the_lease_and_moves_run_at_into_the_future() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:ladder",
        time::OffsetDateTime::now_utc(),
    )
    .await?;

    let job = Jobs::claim(repositories.as_ref(), "worker-1")
        .await?
        .context("the job must be claimable")?;

    let over_long = "e".repeat(3_000);
    assert!(
        !repositories
            .reschedule(
                job.id,
                "worker-2",
                std::time::Duration::from_secs(60),
                Some(&over_long)
            )
            .await?,
        "a worker that does not hold the lease must not be able to reschedule the job"
    );

    assert!(
        repositories
            .reschedule(
                job.id,
                "worker-1",
                std::time::Duration::from_secs(60),
                Some(&over_long)
            )
            .await?,
        "the lease holder reschedules it"
    );

    let (run_at, locked_at, locked_by, last_error): (
        time::OffsetDateTime,
        Option<time::OffsetDateTime>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as("SELECT run_at, locked_at, locked_by, last_error FROM jobs WHERE id = $1")
        .bind(job.id)
        .fetch_one(&pool)
        .await
        .context("re-reading the job must succeed")?;

    assert!(
        run_at > time::OffsetDateTime::now_utc(),
        "a rescheduled job must not be runnable yet"
    );
    assert_eq!(locked_at, None, "the lease is released");
    assert_eq!(locked_by, None, "…and so is its holder — lock_is_paired");
    assert_eq!(
        last_error.map(|error| error.chars().count()),
        Some(2_000),
        "the error is truncated to the column's ceiling rather than refused"
    );

    assert!(
        Jobs::claim(repositories.as_ref(), "worker-1")
            .await?
            .is_none(),
        "the rescheduled job must not be claimable before its new run_at"
    );

    Ok(())
}

/// The reaper frees a lease whose worker died, and *only* that one: reaping
/// a lease that is merely slow hands a running job to a second worker.
#[tokio::test]
async fn reap_expired_leases_frees_only_the_stale_lease() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:stale",
        now - time::Duration::seconds(60),
    )
    .await?;
    enqueue(repositories.as_ref(), "poll_charge", "poll:fresh", now).await?;

    let stale = Jobs::claim(repositories.as_ref(), "worker-dead")
        .await?
        .context("the stale job must be claimable")?;
    let fresh = Jobs::claim(repositories.as_ref(), "worker-alive")
        .await?
        .context("the fresh job must be claimable")?;

    // The crash: a worker that took a lease ten minutes ago and never came
    // back. Written directly because there is no other way to produce it —
    // waiting ten minutes is not a test.
    sqlx::query("UPDATE jobs SET locked_at = now() - INTERVAL '10 minutes' WHERE id = $1")
        .bind(stale.id)
        .execute(&pool)
        .await
        .context("backdating the lease must succeed")?;

    let reaped = repositories
        .reap_expired_leases(std::time::Duration::from_secs(300))
        .await
        .context("reaping must succeed")?;
    assert_eq!(
        reaped, 1,
        "only the ten-minute-old lease is stale at a five-minute lease"
    );

    let (locked_by, last_error): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT locked_by, last_error FROM jobs WHERE id = $1")
            .bind(stale.id)
            .fetch_one(&pool)
            .await
            .context("re-reading the reaped job must succeed")?;
    assert_eq!(locked_by, None, "the stale lease is freed");
    assert_eq!(last_error.as_deref(), Some("lease expired"), "and says why");

    let still_held: Option<String> = sqlx::query_scalar("SELECT locked_by FROM jobs WHERE id = $1")
        .bind(fresh.id)
        .fetch_one(&pool)
        .await
        .context("re-reading the live job must succeed")?;
    assert_eq!(
        still_held.as_deref(),
        Some("worker-alive"),
        "a lease that is merely young must not be reaped"
    );

    Ok(())
}

/// The gauge's queue-age number: the oldest job that anything could actually
/// claim.
///
/// Three exclusions, and each one is a way the number could lie to whoever is
/// watching for a backlog. A **leased** job is being worked on, not waiting.
/// A **future-dated** job is on the poll ladder, which is the ladder working
/// correctly rather than a queue falling behind. A **parked** job
/// (`run_at = 'infinity'`) is not backlog at all — counting it would peg the
/// gauge at "infinitely behind" from the first dead letter onwards and, more
/// bluntly, `'infinity'` has no `OffsetDateTime`, so decoding one fails the
/// query instead of answering it.
///
/// The empty answer is `None` and not zero, because "the queue is empty" and
/// "the queue is zero seconds behind" are different facts and an operator
/// acts differently on each.
#[tokio::test]
async fn oldest_runnable_run_at_ignores_leased_future_and_parked_jobs() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();

    assert_eq!(
        repositories
            .oldest_runnable_run_at()
            .await
            .context("an empty queue must be readable, not an error")?,
        None,
        "an empty queue has no age; zero would read as `caught up`"
    );

    let parked_at = now - time::Duration::hours(3);
    let leased_at = now - time::Duration::hours(2);
    let runnable_at = now - time::Duration::hours(1);
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:parked",
        parked_at,
    )
    .await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:leased",
        leased_at,
    )
    .await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:runnable",
        runnable_at,
    )
    .await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:later",
        now + time::Duration::hours(1),
    )
    .await?;

    // The oldest row of all, parked by the same write the loop uses — so the
    // exclusion is asserted against `dead_letter`'s own output rather than
    // against a hand-written `'infinity'`.
    let parked = Jobs::claim(repositories.as_ref(), "worker-parking")
        .await?
        .context("the oldest job must be claimable")?;
    assert_eq!(parked.dedupe_key, "poll:parked");
    assert!(
        repositories
            .dead_letter(parked.id, "worker-parking", "poisoned: no such charge")
            .await?,
        "the lease holder parks it"
    );

    // The next-oldest, held by a worker that is still working on it.
    let leased = Jobs::claim(repositories.as_ref(), "worker-busy")
        .await?
        .context("the next job must be claimable")?;
    assert_eq!(leased.dedupe_key, "poll:leased");

    let oldest = repositories
        .oldest_runnable_run_at()
        .await
        .context("reading the queue age must succeed")?
        .context("three of the four rows are excluded, but `poll:runnable` is not")?;
    // Compared with a tolerance, not for equality: `timestamptz` is
    // microsecond-precision and `OffsetDateTime` is nanosecond, so a
    // round-trip truncates. An hour of slack would hide the bug; a
    // millisecond cannot.
    assert!(
        (oldest - runnable_at).abs() < time::Duration::milliseconds(1),
        "the age must come from the oldest job nothing is doing and nothing has parked; \
         got {oldest} for a row written at {runnable_at}"
    );

    // And a queue whose only rows are excluded is indistinguishable from an
    // empty one, which is the honest answer: there is no backlog.
    sqlx::query("DELETE FROM jobs WHERE dedupe_key IN ('poll:runnable', 'poll:later')")
        .execute(&pool)
        .await
        .context("removing the claimable rows must succeed")?;
    assert_eq!(
        repositories.oldest_runnable_run_at().await?,
        None,
        "a parked job and a leased one are not a backlog"
    );

    Ok(())
}

/// `dedupe_key` names the *work*, so a second enqueue of the same work is a
/// no-op — and specifically not an upsert: a backstop scan must not be able
/// to drag a job already scheduled an hour out back to now.
#[tokio::test]
async fn enqueue_in_tx_dedupes_on_dedupe_key() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let later = time::OffsetDateTime::now_utc() + time::Duration::hours(1);

    assert!(
        enqueue(repositories.as_ref(), "poll_charge", "poll:ch_once", later).await?,
        "the first enqueue inserts"
    );

    let inserted = one_tx::enqueue_in_tx(
        repositories.as_ref(),
        "resubmit_charge",
        "poll:ch_once",
        &json!({ "charge_id": "ch_other" }),
        time::OffsetDateTime::now_utc(),
    )
    .await
    .context("a duplicate enqueue must not error")?;
    assert!(
        !inserted,
        "the second enqueue of the same work inserts nothing"
    );

    let (kind, payload, run_at): (String, serde_json::Value, time::OffsetDateTime) =
        sqlx::query_as("SELECT kind, payload, run_at FROM jobs WHERE dedupe_key = $1")
            .bind("poll:ch_once")
            .fetch_one(&pool)
            .await
            .context("exactly one row must carry the dedupe key")?;
    assert_eq!(kind, "poll_charge", "the first enqueue's kind survives");
    assert_eq!(payload, json!({ "charge_id": "ch_x" }), "and its payload");
    assert!(
        run_at > time::OffsetDateTime::now_utc(),
        "a duplicate enqueue must not pull a scheduled job forward — DO NOTHING, not DO UPDATE"
    );

    Ok(())
}

/// The write a rail callback is worth anything because of: a job sitting a
/// rung or two out is brought back to now, and the four states it must
/// **not** touch are left alone.
///
/// The first assertion is the callback's whole value — without it a rail
/// telling us about a payment changes nothing at all, because
/// `enqueue_in_tx` is `DO NOTHING` and the test directly above pins that it
/// stays that way. The others are the reasons this is a separate, opt-in
/// write rather than that enqueue growing an upsert: a leased job is being
/// run right now, a parked job is a dead letter a human owns, an
/// already-claimable job needs nothing — and a job due **within the floor**
/// is about to run anyway, which is the guard that stops an unauthenticated
/// caller turning a POST about a freshly confirmed charge into a rail
/// request (`vpay_api::provider_callback::PULL_FORWARD_FLOOR`).
#[tokio::test]
async fn pull_forward_moves_a_job_past_the_floor_and_leaves_near_leased_parked_and_due_alone()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();
    // The poll ladder's first rung, which is the floor the one caller passes
    // (`vpay_worker::poll_delay(0)`, spelled in `vpay_api::provider_callback`
    // because the dependency runs the other way).
    let floor = std::time::Duration::from_secs(10);
    let a_few_rungs_out = now + time::Duration::seconds(30);

    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:ch_rung",
        a_few_rungs_out,
    )
    .await?;
    // Inside the floor: the queue is about to ask anyway.
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:ch_soon",
        now + time::Duration::seconds(5),
    )
    .await?;
    // Already claimable: nothing to do.
    enqueue(repositories.as_ref(), "poll_charge", "poll:ch_due", now).await?;
    // A worker is running this one right now.
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:ch_leased",
        a_few_rungs_out,
    )
    .await?;
    sqlx::query("UPDATE jobs SET locked_at = now(), locked_by = 'worker-1' WHERE dedupe_key = $1")
        .bind("poll:ch_leased")
        .execute(&pool)
        .await
        .context("leasing the job must succeed")?;
    // Parked by `dead_letter`: `run_at = 'infinity'`, lease cleared.
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:ch_parked",
        a_few_rungs_out,
    )
    .await?;
    sqlx::query("UPDATE jobs SET run_at = 'infinity'::TIMESTAMPTZ WHERE dedupe_key = $1")
        .bind("poll:ch_parked")
        .execute(&pool)
        .await
        .context("parking the job must succeed")?;

    let soon_before = run_at_of(&pool, "poll:ch_soon").await?;

    assert!(
        one_tx::pull_forward_in_tx(repositories.as_ref(), "poll:ch_rung", floor).await?,
        "a job past the floor, unleased and unparked, is exactly what a callback exists \
         to move"
    );
    let moved: time::OffsetDateTime = run_at_of(&pool, "poll:ch_rung").await?;
    assert!(
        moved <= time::OffsetDateTime::now_utc(),
        "the job must be claimable now, not at its rung; run_at is {moved}"
    );

    for (key, why) in [
        (
            "poll:ch_soon",
            "a job due inside the floor is about to run; moving it buys the rail nothing \
             and lets an anonymous caller spend a rail request",
        ),
        ("poll:ch_due", "a job whose time has come needs no help"),
        (
            "poll:ch_leased",
            "a leased job is being polled right now; that poll will see the rail's answer",
        ),
        (
            "poll:ch_parked",
            "a dead letter stays parked — un-parking one is a human's UPDATE",
        ),
    ] {
        assert!(
            !one_tx::pull_forward_in_tx(repositories.as_ref(), key, floor).await?,
            "{why}"
        );
    }

    // The refusal has to be about the *row*, not only about the answer: a
    // statement that moved the job and then reported `false` would pass the
    // loop above.
    assert_eq!(
        run_at_of(&pool, "poll:ch_soon").await?,
        soon_before,
        "a job inside the floor must be left exactly where it was"
    );

    // And the parked row is *still* parked. The boolean above would also be
    // `false` for a write that matched nothing because it had already moved
    // the row, so the state itself is what the assertion has to be about.
    // Counted rather than selected: `'infinity'` has no `OffsetDateTime`
    // representation, so decoding `run_at` here would fail the query instead
    // of answering it (`Jobs::oldest_runnable_run_at` says the same).
    let still_parked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE dedupe_key = 'poll:ch_parked' AND run_at = 'infinity'",
    )
    .fetch_one(&pool)
    .await
    .context("counting the parked row must succeed")?;
    assert_eq!(still_parked, 1, "the dead letter must still be parked");

    // A key nothing has ever queued is not an error either — the callback
    // route calls this immediately after an enqueue that may have inserted.
    assert!(!one_tx::pull_forward_in_tx(repositories.as_ref(), "poll:ch_absent", floor).await?);

    // A floor of zero is the old behaviour, and it is what proves the guard
    // above is the floor doing work rather than the row being unmovable for
    // some other reason.
    assert!(
        one_tx::pull_forward_in_tx(
            repositories.as_ref(),
            "poll:ch_soon",
            std::time::Duration::ZERO
        )
        .await?,
        "with no floor the same job moves, so the refusal above was the floor's"
    );

    Ok(())
}

/// `run_at` for one dedupe key, read straight off the table so an assertion
/// about what was committed cannot be satisfied by what the writer returned.
async fn run_at_of(pool: &PgPool, dedupe_key: &str) -> anyhow::Result<time::OffsetDateTime> {
    sqlx::query_scalar::<_, time::OffsetDateTime>("SELECT run_at FROM jobs WHERE dedupe_key = $1")
        .bind(dedupe_key)
        .fetch_one(pool)
        .await
        .context("reading the job's run_at must succeed")
}

/// The lookup behind the unauthenticated callback route: a charge is found by
/// the reference vpay generated, and **only** under the rail that generated
/// it.
///
/// The second half is the security property, not a tidiness one. The rail is
/// named by a path segment and the reference by a body anyone who can reach
/// the URL could have written, so a lookup that ignored `provider_code` would
/// let a POST to `/provider/orange_money/callback` name an MTN charge.
#[tokio::test]
async fn get_by_provider_reference_finds_the_charge_and_only_under_its_own_rail()
-> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    repositories
        .insert(&fixture_intent("pi_cb", "XAF"))
        .await
        .context("inserting the intent must succeed")?;
    let mut charge = fixture_charge("ch_cb", "pi_cb");
    let reference = uuid::Uuid::new_v4();
    charge.provider_reference_id = reference;
    one_tx::insert_for_intent(repositories.as_ref(), &charge)
        .await
        .context("opening the charge must succeed")?;

    let found = repositories
        .get_by_provider_reference("mtn_momo", reference)
        .await?
        .context("the charge must be found by the reference vpay generated")?;
    assert_eq!(found.id, "ch_cb");
    assert_eq!(found.provider_reference_id, reference);

    assert!(
        repositories
            .get_by_provider_reference("orange_money", reference)
            .await?
            .is_none(),
        "a callback posted to one rail's path must not be able to name another rail's charge"
    );
    assert!(
        repositories
            .get_by_provider_reference("mtn_momo", uuid::Uuid::new_v4())
            .await?
            .is_none(),
        "a reference this deployment never generated names nothing"
    );

    Ok(())
}

/// The push-rail settlement: a `submitted` charge on a `processing` intent.
/// Charge and intent both reach `succeeded`, `amount_received` becomes the
/// full amount, and **one** `payment_intent.succeeded` event is queued for
/// fan-out.
#[tokio::test]
async fn apply_succeeded_settles_a_submitted_charge_and_emits_one_event() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_push",
        "ch_push",
        "processing",
        "submitted",
    )
    .await?;

    let data = json!({ "id": "pi_push", "object": "payment_intent", "status": "succeeded" });
    let (charge, intent) = repositories
        .apply_succeeded("ch_push", Some("MTN-TXN-4242"), "evt_push_succeeded", &data)
        .await
        .context("settling must succeed")?
        .context("a live charge must settle")?;

    assert_eq!(charge.state, "succeeded");
    assert_eq!(charge.provider_txn_id.as_deref(), Some("MTN-TXN-4242"));
    assert_eq!(intent.status, "succeeded");
    assert_eq!(
        intent.amount_received, intent.amount,
        "a settled push-rail payment collected the whole amount"
    );
    assert_eq!(intent.amount_received, 5_000);

    assert_eq!(
        charge_state(&pool, "ch_push").await?,
        "succeeded",
        "committed, not just returned"
    );
    assert_eq!(event_count(&pool, "pi_push").await?, 1);

    let pending = repositories
        .pending_page(10)
        .await
        .context("the fan-out backlog must be readable")?;
    let event = pending
        .first()
        .context("the settlement's event must be in the backlog")?;
    assert_eq!(event.event_type, "payment_intent.succeeded");
    assert_eq!(event.object_id, "pi_push");
    assert_eq!(
        event.merchant_id, "merchant_a",
        "copied from the intent, not joined"
    );
    assert!(!event.livemode);
    assert_eq!(event.fanout_state, "pending");
    assert_eq!(
        event.data, data,
        "the wire object is snapshotted, not re-derived"
    );

    Ok(())
}

/// **The decisive test for the `from` label on a settlement**, and therefore
/// for the `RETURNING` sub-select `vpay_db::settlement` adds to the two
/// statements that take a charge terminal.
///
/// Those statements guard on a *set* of live states, so `from` cannot come
/// from the `WHERE` clause the way `mark_submitted`'s can. It comes from
/// `(SELECT prev.state FROM charges prev WHERE prev.id = charges.id)` in the
/// `RETURNING` list, which reads the statement's own snapshot and therefore
/// the state before the update. Break that — drop the sub-select, or let it
/// see the new row — and this reads `succeeded → succeeded`.
///
/// Two charges, two different starting rungs, in one recorder, so the label
/// is proven to follow the row rather than being a constant that happens to
/// be right once.
///
/// A plain `#[test]` with its own runtime, unlike every other case in this
/// file: `metrics::with_local_recorder` installs a **thread-local** recorder
/// and takes a synchronous closure, so the settlement has to be driven
/// inside that closure and on that thread. `#[tokio::test]` would put the
/// awaited work on a runtime whose worker threads cannot see the recorder,
/// and the scrape below would be empty — a test that passed by asserting on
/// a document nothing wrote.
#[test]
fn a_settlement_counts_the_transition_it_actually_made() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("a current-thread runtime builds")?;
    // The container's `Drop` calls `Handle::current()` to schedule its
    // teardown, so the runtime has to be *entered* on this thread and has to
    // outlive the container. Declaration order is what guarantees both:
    // locals drop in reverse, so the container goes first, then this guard,
    // then the runtime.
    let _entered = runtime.enter();

    let (_container, repositories, _pool) = runtime.block_on(async {
        let (container, repositories, pool) = migrated_postgres().await?;
        seed_reference_data(repositories.as_ref()).await?;
        live_charge(
            repositories.as_ref(),
            "pi_from_submitted",
            "ch_from_submitted",
            "processing",
            "submitted",
        )
        .await?;
        live_charge(
            repositories.as_ref(),
            "pi_from_pending",
            "ch_from_pending",
            "requires_action",
            "pending",
        )
        .await?;
        anyhow::Ok((container, repositories, pool))
    })?;

    // The shipping exporter under a *local* recorder: `set_global_recorder`
    // succeeds once per process, and this suite is 60-odd tests in one
    // binary under a plain `cargo test`.
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let data = json!({ "id": "x", "object": "payment_intent", "status": "succeeded" });

    let settled = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            repositories
                .apply_succeeded("ch_from_submitted", None, "evt_from_submitted", &data)
                .await?;
            repositories
                .apply_failed(
                    "ch_from_pending",
                    "insufficient_funds",
                    "NOT_ENOUGH_FUNDS",
                    "The payment was declined (insufficient_funds).",
                    "evt_from_pending",
                    &data,
                )
                .await
        })
    });
    settled.context("settling must succeed")?;
    let scrape = handle.render();

    assert!(
        scrape.contains(
            r#"vpay_charge_transitions_total{provider="mtn_momo",from="submitted",to="succeeded"} 1"#
        ),
        "the previous-state sub-select must name the rung the charge came from:\n{scrape}"
    );
    assert!(
        scrape.contains(
            r#"vpay_charge_transitions_total{provider="mtn_momo",from="pending",to="failed"} 1"#
        ),
        "and it must follow the row rather than being a constant:\n{scrape}"
    );

    Ok(())
}

/// **The counter is a record of committed transitions.**
/// `charges::insert_for_intent` runs inside a caller's transaction, so a
/// second write in that transaction failing must leave
/// `vpay_charge_transitions_total` untouched — the charge does not exist.
///
/// This is the exact shape of the confirm path
/// (`vpay_api::v1::payment_intents::insert_charge`): open the charge, then
/// enqueue its poll job in the same transaction. Here the enqueue names a
/// `kind` outside migration 0021's `kind_is_known` CHECK, so Postgres aborts
/// the transaction — no test double, no injected failure, a real constraint
/// on a real database refusing a real write.
///
/// Both halves matter and the second is what stops this passing vacuously:
/// the rolled-back insert counts nothing, *and* the committed one counts
/// exactly one. Move `record_opened` back inside `insert_for_intent` and the
/// first assertion fails; delete the call from this test's own caller and the
/// second does.
///
/// The *production* caller is pinned somewhere else, and has to be: this
/// crate cannot see `vpay_api`. Deleting `charges::record_opened` from
/// `vpay_api::v1::payment_intents::insert_charge` — or
/// `record_left_submitting` from `persist_submitted` — fails
/// `a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`
/// (`backends/tests/integration/tests/worker_e2e.rs`), which scrapes the
/// running server's `/metrics` and asserts all four edges of one charge's
/// walk. That is the cost of moving the recording out to the commit's owner,
/// and it is paid rather than assumed: verified by deleting the call and
/// watching that test fail on the `from="",to="submitting"` edge.
///
/// A plain `#[test]` with its own current-thread runtime, for the reason
/// `a_settlement_counts_the_transition_it_actually_made` gives above.
#[test]
fn a_rolled_back_charge_insert_counts_nothing_and_a_committed_one_counts_once() -> anyhow::Result<()>
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("a current-thread runtime builds")?;
    let _entered = runtime.enter();

    let (_container, repositories, _pool) = runtime.block_on(async {
        let (container, repositories, pool) = migrated_postgres().await?;
        seed_reference_data(repositories.as_ref()).await?;
        repositories
            .insert(&fixture_intent("pi_rolled_back", "XAF"))
            .await?;
        repositories
            .insert(&fixture_intent("pi_committed", "XAF"))
            .await?;
        anyhow::Ok((container, repositories, pool))
    })?;

    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();

    let rolled_back = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let charge = repositories
                .transaction(|tx| {
                    Box::pin(async move {
                        let charge = tx
                            .insert_for_intent(&fixture_charge("ch_rolled_back", "pi_rolled_back"))
                            .await
                            .context("the insert itself must succeed")?;
                        // The confirm path's second write, with a kind the
                        // CHECK refuses.
                        let enqueued = tx
                            .enqueue_in_tx(
                                "not_a_job_kind",
                                "poll:ch_rolled_back",
                                &json!({ "charge_id": "ch_rolled_back" }),
                                time::OffsetDateTime::now_utc(),
                            )
                            .await;
            assert!(
                enqueued.is_err(),
                "`kind_is_known` must refuse this write, or the transaction never aborts \
                 and this test proves nothing"
            );
                        Ok::<_, anyhow::Error>(TxOutcome::Abandon(charge))
                    })
                })
                .await?
                .into_inner();
            anyhow::Ok(charge)
        })
    })?;
    assert_eq!(rolled_back.state, "submitting");

    let scrape = handle.render();
    assert!(
        !scrape.contains("vpay_charge_transitions_total"),
        "a charge whose transaction rolled back must not appear in the counter:\n{scrape}"
    );
    assert!(
        runtime
            .block_on(repositories.get_for_intent("pi_rolled_back"))?
            .is_none(),
        "the rollback must have removed the row — otherwise the assertion above is \
         about a charge that still exists"
    );

    // The committed half, in a second recorder so the first render stays a
    // statement about the rollback alone.
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let charge = repositories
                .transaction(|tx| {
                    Box::pin(async move {
                        let charge = tx
                            .insert_for_intent(&fixture_charge("ch_committed", "pi_committed"))
                            .await?;
                        tx.enqueue_in_tx(
                            "poll_charge",
                            "poll:ch_committed",
                            &json!({ "charge_id": "ch_committed" }),
                            time::OffsetDateTime::now_utc(),
                        )
                        .await?;
                        Ok::<_, anyhow::Error>(TxOutcome::Commit(charge))
                    })
                })
                .await?
                .into_inner();
            repositories.record_opened(&charge);
            anyhow::Ok(())
        })
    })?;

    let scrape = handle.render();
    assert!(
        scrape.contains(
            r#"vpay_charge_transitions_total{provider="mtn_momo",from="",to="submitting"} 1"#
        ),
        "a committed charge is still counted, exactly once:\n{scrape}"
    );

    Ok(())
}

/// The redirect-rail settlement: a `pending` charge on a `requires_action`
/// intent. `requires_action → succeeded` is a legal settlement precisely
/// because the payer acting at the rail is what the status means; the guard
/// names both confirmed statuses for that reason.
#[tokio::test]
async fn apply_succeeded_settles_a_pending_charge_from_a_requires_action_intent()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_redirect",
        "ch_redirect",
        "requires_action",
        "pending",
    )
    .await?;

    let (charge, intent) = repositories
        .apply_succeeded(
            "ch_redirect",
            None,
            "evt_redirect_succeeded",
            &json!({ "id": "pi_redirect" }),
        )
        .await?
        .context("a pending charge on a requires_action intent must settle")?;

    assert_eq!(charge.state, "succeeded");
    assert_eq!(
        charge.provider_txn_id, None,
        "a rail that named no transaction id leaves the column NULL rather than inventing one"
    );
    assert_eq!(intent.status, "succeeded");
    assert_eq!(intent.amount_received, 5_000);
    assert_eq!(event_count(&pool, "pi_redirect").await?, 1);

    Ok(())
}

/// Re-running a settled job — the normal outcome when a worker dies between
/// committing the settlement and deleting its job — must change nothing and
/// must not queue a second webhook.
#[tokio::test]
async fn a_second_apply_succeeded_returns_none_and_writes_no_second_event() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_twice",
        "ch_twice",
        "processing",
        "submitted",
    )
    .await?;

    repositories
        .apply_succeeded(
            "ch_twice",
            Some("TXN-1"),
            "evt_first",
            &json!({ "id": "pi_twice" }),
        )
        .await?
        .context("the first settlement must fire")?;

    let second = repositories
        .apply_succeeded(
            "ch_twice",
            Some("TXN-2"),
            "evt_second",
            &json!({ "id": "pi_twice" }),
        )
        .await
        .context("a re-run must not error — it is a normal outcome, not a failure")?;
    assert!(
        second.is_none(),
        "an already-settled charge reports nothing to do"
    );

    assert_eq!(
        event_count(&pool, "pi_twice").await?,
        1,
        "a second event would be a second webhook for one payment, under a different evt_ id \
         that a merchant deduping on it cannot catch"
    );

    let txn: Option<String> =
        sqlx::query_scalar("SELECT provider_txn_id FROM charges WHERE id = $1")
            .bind("ch_twice")
            .fetch_one(&pool)
            .await
            .context("re-reading the charge must succeed")?;
    assert_eq!(
        txn.as_deref(),
        Some("TXN-1"),
        "the re-run overwrote nothing"
    );

    Ok(())
}

/// A decline the poll discovered: the charge goes terminal, and the intent
/// goes *back* to `requires_payment_method` carrying the error pair — the
/// transition `docs/flows/payment-lifecycle.md` describes and that
/// `record_payment_error` deliberately does not perform.
#[tokio::test]
async fn apply_failed_returns_the_intent_to_requires_payment_method_with_the_error_pair()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_declined",
        "ch_declined",
        "processing",
        "pending",
    )
    .await?;

    let (charge, intent) = repositories
        .apply_failed(
            "ch_declined",
            "payer_declined",
            "PAYER_REJECTED: the payer declined the prompt",
            "The payer declined the payment request.",
            "evt_declined",
            &json!({ "id": "pi_declined", "status": "requires_payment_method" }),
        )
        .await?
        .context("a live charge must be failable")?;

    assert_eq!(charge.state, "failed");
    assert_eq!(charge.failure_code.as_deref(), Some("payer_declined"));
    assert_eq!(
        charge.failure_raw.as_deref(),
        Some("PAYER_REJECTED: the payer declined the prompt"),
        "the rail's own words survive"
    );
    assert_eq!(
        intent.status, "requires_payment_method",
        "there is no `failed` intent status — the diagram's failed box is this status plus the \
         error pair"
    );
    assert_eq!(
        intent.last_payment_error_code.as_deref(),
        Some("payer_declined")
    );
    assert_eq!(
        intent.last_payment_error_message.as_deref(),
        Some("The payer declined the payment request."),
    );
    assert_eq!(intent.amount_received, 0, "nothing was collected");

    let pending = repositories.pending_page(10).await?;
    let event = pending
        .first()
        .context("the failure must be queued for fan-out")?;
    assert_eq!(event.event_type, "payment_intent.payment_failed");
    assert_eq!(event_count(&pool, "pi_declined").await?, 1);

    assert!(
        repositories
            .apply_failed(
                "ch_declined",
                "payer_declined",
                "raw",
                "message",
                "evt_declined_again",
                &json!({ "id": "pi_declined" }),
            )
            .await?
            .is_none(),
        "a charge that is already terminal reports nothing to do"
    );
    assert_eq!(event_count(&pool, "pi_declined").await?, 1);

    Ok(())
}

/// The settlement of a confirm that crashed before it could move the intent.
///
/// `confirm` commits the charge and its poll job in one transaction *before*
/// calling the rail, and moves the intent only afterwards, in
/// `persist_submitted` (`vpay_api::v1::payment_intents`,
/// `docs/flows/crash-safety.md`). So all three of that document's kill points
/// leave exactly this pairing: a live charge against an intent that still
/// reads `requires_payment_method`.
///
/// This case is the one that caught the bug. While the intent guard named
/// only `processing`/`requires_action`, the charge compare-and-swap fired,
/// the intent write matched nothing, and the settlement raised
/// `WriteMatchedNoRow` — `Category::Internal`, `Retry::Never`,
/// `Decision::DeadLetter`. The charge the rail had just collected was left
/// live with its poll job parked at `run_at = 'infinity'`, and the merchant's
/// intent said no payment had ever been attempted. Reverting
/// `SETTLEABLE_STATUSES` to the two confirmed statuses turns this test back
/// into that failure, which is what makes it worth its container.
///
/// The charge is the record of whether a confirm happened; the intent's
/// status is not. The compare-and-swap above has already proven a live charge
/// exists, and only a confirm writes one.
#[tokio::test]
async fn a_settlement_after_a_crashed_confirm_still_moves_the_intent() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_crashed",
        "ch_crashed",
        "requires_payment_method",
        "submitted",
    )
    .await?;

    let (charge, intent) = repositories
        .apply_succeeded(
            "ch_crashed",
            Some("TXN-CRASHED"),
            "evt_crashed",
            &json!({ "id": "pi_crashed", "status": "succeeded" }),
        )
        .await
        .context("a crashed confirm's charge must be settleable, not a WriteMatchedNoRow")?
        .context("the live charge must settle")?;

    assert_eq!(charge.state, "succeeded");
    assert_eq!(charge.provider_txn_id.as_deref(), Some("TXN-CRASHED"));
    assert_eq!(
        intent.status, "succeeded",
        "the intent must move even though the confirm never moved it out of \
         requires_payment_method"
    );
    assert_eq!(
        intent.amount_received, intent.amount,
        "amount_received is written by the same statement, whatever the intent's previous \
         status was"
    );
    assert_eq!(
        charge_state(&pool, "ch_crashed").await?,
        "succeeded",
        "committed, not just returned"
    );
    assert_eq!(event_count(&pool, "pi_crashed").await?, 1);

    Ok(())
}

/// The same crash, declined instead of paid: the status does not move (it is
/// already `requires_payment_method`) and the write is the error pair alone —
/// but it must still count as **applied**.
///
/// Zero rows matched is what `vpay_db::settlement` reports as a broken
/// invariant, and one row matched with nothing to change is not that. The
/// distinction is the whole difference between "the merchant is told why the
/// payment failed" and "the poll job is parked forever".
#[tokio::test]
async fn a_decline_after_a_crashed_confirm_stamps_the_error_without_moving_the_status()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_crash_declined",
        "ch_crash_declined",
        "requires_payment_method",
        "submitted",
    )
    .await?;

    let (charge, intent) = repositories
        .apply_failed(
            "ch_crash_declined",
            "insufficient_funds",
            "NOT_ENOUGH_FUNDS",
            "The payer's account did not have enough funds.",
            "evt_crash_declined",
            &json!({ "id": "pi_crash_declined", "status": "requires_payment_method" }),
        )
        .await
        .context("a crashed confirm's decline must apply, not raise WriteMatchedNoRow")?
        .context("the live charge must fail")?;

    assert_eq!(charge.state, "failed");
    assert_eq!(charge.failure_code.as_deref(), Some("insufficient_funds"));
    assert_eq!(
        intent.status, "requires_payment_method",
        "there was nowhere to move it: that is already the status a decline returns to"
    );
    assert_eq!(
        intent.last_payment_error_code.as_deref(),
        Some("insufficient_funds"),
        "the error pair is the entire write here, and it is what the merchant reads"
    );
    assert!(intent.last_payment_error_message.is_some());
    assert_eq!(event_count(&pool, "pi_crash_declined").await?, 1);

    Ok(())
}

/// The intent half of the settlement is a compare-and-swap too, and it is
/// the one that keeps a broken database from becoming a wrong balance: a
/// live charge against a `canceled` intent is an invariant violation, and the
/// settlement refuses it outright rather than moving an intent the merchant
/// withdrew to `succeeded`.
///
/// `canceled` and not `requires_payment_method`: that one is now a legal
/// settlement source (see above), while this one is genuinely unreachable —
/// `payment_intents::cancel`'s `NOT EXISTS` refuses to cancel an intent with
/// a live charge, so the pairing can only be produced by a bug. `succeeded`
/// is the other unreachable one, kept out by "one charge per intent, forever".
///
/// The whole transaction must roll back — the charge stays live, no event is
/// queued — so a retry finds exactly the state it expects.
#[tokio::test]
async fn apply_succeeded_refuses_a_canceled_intent_and_commits_nothing() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    // Not reachable through the API (cancel refuses while a charge is live) —
    // which is the point: this is the shape a bug elsewhere would present,
    // and it must not settle.
    live_charge(
        repositories.as_ref(),
        "pi_broken",
        "ch_broken",
        "canceled",
        "submitted",
    )
    .await?;

    let error = repositories
        .apply_succeeded(
            "ch_broken",
            Some("TXN-BROKEN"),
            "evt_broken",
            &json!({ "id": "pi_broken" }),
        )
        .await
        .expect_err("settling against a canceled intent must not report success");
    assert!(
        matches!(
            error,
            vpay_db::DbError::WriteMatchedNoRow {
                table: "payment_intents",
                ..
            }
        ),
        "the refusal must name the row that did not move, and classify as Internal: {error}"
    );

    assert_eq!(
        charge_state(&pool, "ch_broken").await?,
        "submitted",
        "the charge half must have rolled back with the rest"
    );
    let status: String =
        sqlx::query_scalar("SELECT status::TEXT FROM payment_intents WHERE id = $1")
            .bind("pi_broken")
            .fetch_one(&pool)
            .await
            .context("re-reading the intent must succeed")?;
    assert_eq!(status, "canceled", "the intent must not have moved");
    assert_eq!(
        event_count(&pool, "pi_broken").await?,
        0,
        "and nothing may be announced"
    );

    Ok(())
}

/// The non-terminal rungs of the poll ladder are a compare-and-swap on the
/// state the caller believed the charge was in, so a job running twice
/// cannot walk a charge backwards.
#[tokio::test]
async fn set_live_state_moves_a_charge_only_from_the_expected_state() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_live",
        "ch_live",
        "processing",
        "submitted",
    )
    .await?;

    assert!(
        !repositories
            .set_live_state("ch_live", "pending", "unresolved")
            .await?,
        "a swap from a state the charge is not in must not fire"
    );
    assert_eq!(charge_state(&pool, "ch_live").await?, "submitted");

    assert!(
        repositories
            .set_live_state("ch_live", "submitted", "pending")
            .await?,
        "the expected state swaps"
    );
    assert_eq!(charge_state(&pool, "ch_live").await?, "pending");

    assert!(
        !repositories
            .set_live_state("ch_live", "submitted", "pending")
            .await?,
        "re-running the same swap is a no-op, not a second move"
    );

    Ok(())
}

/// The read the worker polls with carries **Postgres' own clock** beside the
/// row, and the age that pair implies moves with `created_at`.
///
/// Everything the worker decides about a `submitting` charge is that age:
/// whether the state is evidence of a crash or of a confirm still inside its
/// rail call (`vpay_worker::recovery_step`, sixty seconds), and whether the
/// charge is past the 24-hour escalation horizon. `created_at` is written by
/// Postgres; before this method existed the other operand was the worker
/// host's `OffsetDateTime::now_utc()`, so the subtraction spanned two clocks
/// and a worker running a minute fast measured every charge as a minute
/// older than it was. Both halves come off one statement now, which is what
/// this asserts — and the second half asserts the answer is a real
/// measurement rather than a constant: move `created_at` back by a minute and
/// a second, and the age moves with it, across the window the worker's guard
/// compares against.
#[tokio::test]
async fn the_charge_read_carries_the_databases_own_clock_beside_the_row() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_as_of",
        "ch_as_of",
        "processing",
        "submitting",
    )
    .await?;

    let as_of = Charges::get_by_id_as_of(repositories.as_ref(), "ch_as_of")
        .await?
        .context("the charge must be readable by its own id")?;
    assert_eq!(
        Some(&as_of.charge),
        Charges::get_by_id(repositories.as_ref(), "ch_as_of")
            .await?
            .as_ref(),
        "`get_by_id` is this read with the clock dropped; two statements that answered \
         with different rows would be the drift the delegation exists to prevent"
    );

    let fresh = as_of.db_now - as_of.charge.created_at;
    assert!(
        fresh >= time::Duration::ZERO && fresh < time::Duration::seconds(5),
        "a charge opened a moment ago must read as seconds old; got {fresh}"
    );

    // The same lever the integration suite's `age_the_crash` pulls, and the
    // one number the recovery guard compares against: a minute and a second.
    sqlx::query("UPDATE charges SET created_at = now() - INTERVAL '61 seconds' WHERE id = $1")
        .bind("ch_as_of")
        .execute(&pool)
        .await
        .context("backdating the charge")?;

    let aged = Charges::get_by_id_as_of(repositories.as_ref(), "ch_as_of")
        .await?
        .context("the backdated charge must still be readable")?;
    let age = aged.db_now - aged.charge.created_at;
    assert!(
        age >= time::Duration::seconds(61) && age < time::Duration::seconds(66),
        "the age must be the database's own measurement of `now() - created_at`; got {age}"
    );

    Ok(())
}

/// The recovery table reads the *latest* submit, ignoring the poll ladder's
/// own `query_status` rows — and reads the `status_code`/`responded_at` pair
/// that tells it whether an answer was ever received.
#[tokio::test]
async fn latest_submit_attempt_returns_the_newest_submit_and_ignores_query_status()
-> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_attempts",
        "ch_attempts",
        "processing",
        "submitting",
    )
    .await?;

    assert!(
        repositories
            .latest_submit_attempt("ch_attempts")
            .await?
            .is_none(),
        "a charge whose rail was never called has no attempt — the resubmit branch"
    );

    let charge = Charges::get_by_id(repositories.as_ref(), "ch_attempts")
        .await?
        .context("the charge must be readable by its own id")?;

    repositories
        .insert_pending(
            "ch_attempts",
            "mtn_momo",
            "submit",
            charge.provider_reference_id,
            1,
        )
        .await?;
    let second = repositories
        .insert_pending(
            "ch_attempts",
            "mtn_momo",
            "submit",
            charge.provider_reference_id,
            2,
        )
        .await?;
    // Inserted *last*, and must still not win: the recovery table asks about
    // submits, and a poll of a charge that was never answered would otherwise
    // hide the unanswered submit behind its own row.
    repositories
        .insert_pending(
            "ch_attempts",
            "mtn_momo",
            "query_status",
            charge.provider_reference_id,
            1,
        )
        .await?;

    let latest = repositories
        .latest_submit_attempt("ch_attempts")
        .await?
        .context("the submit attempts must be visible")?;
    assert_eq!(latest.id, second, "the newest submit wins");
    assert_eq!(latest.attempt, 2);
    assert_eq!(latest.provider_reference_id, charge.provider_reference_id);
    assert_eq!(
        (latest.status_code, latest.responded_at),
        (None, None),
        "no answer was received — the pair the poll branch keys on"
    );

    repositories
        .record_response(second, Some(202), None)
        .await?;
    let answered = repositories
        .latest_submit_attempt("ch_attempts")
        .await?
        .context("the attempt must still be there")?;
    assert_eq!(answered.status_code, Some(202));
    assert!(
        answered.responded_at.is_some(),
        "response_is_paired holds in both directions"
    );

    Ok(())
}

/// The backstop scan sees live charges that have gone quiet, and nothing
/// else: not a terminal charge, and not one that moved a moment ago.
#[tokio::test]
async fn live_charges_stale_since_honours_the_cutoff_and_the_live_state_set() -> anyhow::Result<()>
{
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;

    live_charge(
        repositories.as_ref(),
        "pi_a",
        "ch_stale_old",
        "processing",
        "submitted",
    )
    .await?;
    live_charge(
        repositories.as_ref(),
        "pi_b",
        "ch_stale_older",
        "processing",
        "pending",
    )
    .await?;
    live_charge(
        repositories.as_ref(),
        "pi_c",
        "ch_terminal",
        "succeeded",
        "succeeded",
    )
    .await?;
    live_charge(
        repositories.as_ref(),
        "pi_d",
        "ch_recent",
        "processing",
        "submitted",
    )
    .await?;

    for (id, minutes) in [
        ("ch_stale_old", 20),
        ("ch_stale_older", 60),
        ("ch_terminal", 60),
    ] {
        sqlx::query(
            "UPDATE charges SET updated_at = now() - make_interval(mins => $2) WHERE id = $1",
        )
        .bind(id)
        .bind(minutes)
        .execute(&pool)
        .await
        .context("backdating the charge must succeed")?;
    }

    let cutoff = time::OffsetDateTime::now_utc() - time::Duration::minutes(10);
    let stale = repositories.live_charges_stale_since(cutoff, 10).await?;

    assert_eq!(
        stale,
        vec!["ch_stale_older".to_owned(), "ch_stale_old".to_owned()],
        "oldest first, terminal charges excluded, and a charge that moved inside the window left \
         alone"
    );

    assert_eq!(
        repositories.live_charges_stale_since(cutoff, 1).await?,
        vec!["ch_stale_older".to_owned()],
        "the limit bounds the page"
    );

    Ok(())
}

/// `provider_txn_id` round-trips through the settlement transaction, and the
/// `provider_txn_id_length` CHECK refuses the two values that would be lies:
/// an empty string (reads as "there is an identifier", carries none) and an
/// unbounded blob from a misparsed response. Both must roll the settlement
/// back rather than half-applying it.
#[tokio::test]
async fn provider_txn_id_round_trips_and_its_check_refuses_empty_and_over_long()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_txn",
        "ch_txn",
        "processing",
        "submitted",
    )
    .await?;

    for bad in ["", &"9".repeat(129)] {
        let error = repositories
            .apply_succeeded(
                "ch_txn",
                Some(bad),
                "evt_txn_bad",
                &json!({ "id": "pi_txn" }),
            )
            .await
            .expect_err("the CHECK must refuse this transaction id");
        assert!(
            matches!(error, vpay_db::DbError::Query(_)),
            "a CHECK violation is a vpay bug, not a merchant's: {error}"
        );
        assert_eq!(
            charge_state(&pool, "ch_txn").await?,
            "submitted",
            "the refused settlement must leave the charge exactly where a retry expects it"
        );
        assert_eq!(event_count(&pool, "pi_txn").await?, 0);
    }

    repositories
        .apply_succeeded(
            "ch_txn",
            Some("0123456789"),
            "evt_txn_ok",
            &json!({ "id": "pi_txn" }),
        )
        .await?
        .context("a well-formed transaction id must settle")?;

    let charge = Charges::get_by_id(repositories.as_ref(), "ch_txn")
        .await?
        .context("the charge must be readable")?;
    assert_eq!(charge.provider_txn_id.as_deref(), Some("0123456789"));

    Ok(())
}

/// The worker's unscoped read of an intent: it is reached from a charge's
/// foreign key, so there is no merchant to scope by — and it must still
/// answer `None` for an id that names nothing, rather than erroring, because
/// the caller turns that into a poisoned job with a name in it.
#[tokio::test]
async fn payment_intents_get_by_id_reads_without_a_merchant_scope() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    seed_reference_data(repositories.as_ref()).await?;
    live_charge(
        repositories.as_ref(),
        "pi_unscoped",
        "ch_unscoped",
        "processing",
        "submitted",
    )
    .await?;

    let row = PaymentIntents::get_by_id(repositories.as_ref(), "pi_unscoped")
        .await?
        .context("the worker must be able to read the intent its charge names")?;
    assert_eq!(row.id, "pi_unscoped");
    assert_eq!(row.status, "processing");
    assert_eq!(row.merchant_id, "merchant_a");

    assert!(
        PaymentIntents::get_by_id(repositories.as_ref(), "pi_does_not_exist")
            .await?
            .is_none(),
        "an id that names nothing is None, not an error"
    );

    Ok(())
}

/// The recovery table's per-job bookkeeping — the `not_found_streak` — is
/// carried in the payload, and writing it is guarded on the lease like every
/// other write that follows a claim.
#[tokio::test]
async fn set_payload_writes_only_for_the_lease_holder() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    enqueue(
        repositories.as_ref(),
        "poll_charge",
        "poll:streak",
        time::OffsetDateTime::now_utc(),
    )
    .await?;

    let job = Jobs::claim(repositories.as_ref(), "worker-1")
        .await?
        .context("the job must be claimable")?;
    let updated = json!({ "charge_id": "ch_x", "not_found_streak": 2 });

    assert!(
        !repositories
            .set_payload(job.id, "worker-2", &updated)
            .await?,
        "a worker that does not hold the lease must not rewrite the payload"
    );
    assert!(
        repositories
            .set_payload(job.id, "worker-1", &updated)
            .await?,
        "the lease holder records its bookkeeping"
    );

    let (payload, run_at): (serde_json::Value, time::OffsetDateTime) =
        sqlx::query_as("SELECT payload, run_at FROM jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&pool)
            .await
            .context("re-reading the job must succeed")?;
    assert_eq!(payload, updated);
    assert_eq!(
        run_at, job.run_at,
        "recording bookkeeping must not move the schedule — that is reschedule's job"
    );

    Ok(())
}

// --- webhook deliveries and the events read API (migration 0022, Step 5) ---
//
// Same technique as every section above — one container per test, via
// `migrated_postgres()`. The fan-out and delivery *handlers* live in
// `vpay-worker` and are not exercised here: what these tests pin is the
// persistence contract those handlers are written against, and every claim
// below is about what a real Postgres committed, re-read rather than
// inferred from what a writer returned.

/// A `payment_intent.succeeded` event for `merchant_id`. The type and the
/// data are deliberately boring — nothing below asserts anything about
/// either — so a failure points at fan-out or paging rather than at fixture
/// noise.
fn fixture_event(id: &str, merchant_id: &str) -> vpay_db::NewEvent {
    vpay_db::NewEvent {
        id: id.to_owned(),
        merchant_id: merchant_id.to_owned(),
        livemode: false,
        event_type: "payment_intent.succeeded".to_owned(),
        object_id: "pi_event_fixture".to_owned(),
        data: json!({ "id": "pi_event_fixture", "object": "payment_intent" }),
    }
}

/// Commits one event in its own transaction. `events::insert_in_tx` has no
/// pooled variant on purpose (an event must commit with the transition it
/// describes); a test setting up a backlog has no such transition to be in
/// step with.
async fn insert_event(
    repositories: &dyn Repositories,
    new: &vpay_db::NewEvent,
) -> anyhow::Result<vpay_db::EventRow> {
    let row = one_tx::insert_in_tx(repositories, new)
        .await
        .context("inserting the event must succeed")?;
    Ok(row)
}

/// Creates one delivery in its own committed transaction, the way the
/// fan-out pass would — and returns exactly what `create_in_tx` said, so a
/// caller can assert on the `None` that means "already fanned out".
async fn create_delivery(
    repositories: &dyn Repositories,
    event_id: &str,
    endpoint_id: &str,
    url: &str,
) -> anyhow::Result<Option<uuid::Uuid>> {
    let created = one_tx::create_in_tx(repositories, event_id, endpoint_id, url)
        .await
        .context("creating the delivery must succeed")?;
    Ok(created)
}

/// One delivery, re-read through the repository. Every assertion below goes
/// through this rather than through the `bool` a writer returned, because
/// "the statement matched a row" and "the row now holds what it should" are
/// different claims and only the second one is the contract.
async fn delivery(
    repositories: &dyn Repositories,
    id: uuid::Uuid,
) -> anyhow::Result<vpay_db::DeliveryRow> {
    repositories
        .get(id)
        .await
        .context("reading the delivery must succeed")?
        .context("the delivery must still exist")
}

/// The three timestamp columns `DeliveryRow` deliberately does not carry.
/// Read raw, because the pairing they encode — `status_code IS NULL` with
/// `responded_at IS NULL` and a `sent_at` set is a transport failure — is a
/// property of the row that no Rust type in this crate spells.
async fn delivery_timestamps(
    pool: &PgPool,
    id: uuid::Uuid,
) -> anyhow::Result<(bool, bool, Option<String>)> {
    let row: (bool, bool, Option<String>) = sqlx::query_as(
        "SELECT sent_at IS NOT NULL, responded_at IS NOT NULL, state \
         FROM webhook_deliveries WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .context("re-reading the delivery's timestamps must succeed")?;
    Ok(row)
}

/// An event's stored `fanout_state`, read without the repository so an
/// assertion about what was *committed* cannot be satisfied by whatever the
/// writer happened to return.
async fn fanout_state(pool: &PgPool, event_id: &str) -> anyhow::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT fanout_state FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .context("re-reading the event must succeed")
}

/// The id of the nth events paging fixture, zero-padded so lexical and
/// insertion order agree — same reasoning as `page_fixture_id`.
fn event_page_id(n: usize) -> String {
    format!("evt_{n:02}")
}

/// Migration 0022 applies to a database that already has 0021's four job
/// kinds, and it leaves the shape Step 5 needs: the two new kinds are
/// enqueueable, an invented one is still refused, and `webhook_deliveries`
/// exists with its state vocabulary closed.
///
/// The refusals are the half that matters. 0021 closed `kind_is_known`
/// *specifically* so a `deliver_webhook` job could not be enqueued before a
/// handler existed; a migration that reopened it permissively — or one that
/// dropped the constraint and forgot to re-add it — would apply just as
/// cleanly as this one and would leave nothing to catch a typo'd kind that
/// no worker will ever run.
#[tokio::test]
async fn migration_0022_reopens_the_job_kinds_and_closes_the_delivery_states() -> anyhow::Result<()>
{
    let (_container, repositories, pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();

    for kind in ["fan_out_events", "deliver_webhook"] {
        assert!(
            enqueue(repositories.as_ref(), kind, &format!("dedupe:{kind}"), now).await?,
            "0022 must make {kind} enqueueable — the handlers land in this step"
        );
    }
    // And 0021's four are untouched by the drop-and-re-add.
    assert!(
        enqueue(
            repositories.as_ref(),
            "poll_charge",
            "poll:ch_still_ok",
            now
        )
        .await?
    );

    let refused = one_tx::enqueue_in_tx(
        repositories.as_ref(),
        "deliver_webhooks",
        "webhook:typo",
        &json!({}),
        now,
    )
    .await;
    assert!(
        matches!(refused, Err(vpay_db::DbError::Query(_))),
        "a kind no handler exists for must still be refused by the database, not merely by \
         convention: {refused:?}"
    );

    // The delivery table exists, and its own vocabulary is closed too — a
    // state outside `state_is_known` is as unrunnable as a job kind is.
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_state_check", "merchant_a"),
    )
    .await?;
    let id = create_delivery(
        repositories.as_ref(),
        &event.id,
        "ep_live",
        "https://example.test/hook",
    )
    .await?
    .context("the first delivery for a pair must be created")?;

    let bad_state = sqlx::query("UPDATE webhook_deliveries SET state = 'retrying' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await;
    assert!(
        bad_state.is_err(),
        "state_is_known must refuse a state no reader knows how to interpret"
    );

    // A delivery for an event that does not exist is refused by the foreign
    // key, so a fan-out bug cannot leave a delivery pointing at nothing.
    //
    // The variant is `PersistenceError::ForeignKey` rather than
    // `DbError::ForeignKeyViolation` since 2026-09-06, because `create_in_tx`
    // runs through CrateStack — the constraint is the same one, reported by
    // the same SQLSTATE, through the other of vpay's two persistence paths.
    // `schemas/vpay.cstack` deliberately does not declare this foreign key
    // (`model WebhookDelivery`, `event_id`), which is exactly why the
    // database still has to be the thing that refuses.
    let dangling = one_tx::create_in_tx(
        repositories.as_ref(),
        "evt_nonexistent",
        "ep_live",
        "https://example.test/hook",
    )
    .await;
    let Err(dangling) = dangling else {
        panic!("event_id is a real foreign key: {dangling:?}");
    };
    assert!(
        matches!(
            &dangling,
            vpay_db::DbError::Persistence(vpay_db::PersistenceError::ForeignKey {
                constraint,
                ..
            }) if constraint == "webhook_deliveries_event_id_fkey"
        ),
        "the operator log has to name the constraint that fired: {dangling:?}"
    );
    // And it must reach a caller as the same thing the sqlx path produced
    // before the move — asserted against that path's own answer rather than
    // a literal, so changing one and not the other fails here. This is the
    // property that makes the swap invisible above `vpay-db`.
    {
        use vpay_core::Classify as _;
        let via_sqlx = vpay_db::DbError::ForeignKeyViolation {
            constraint: "webhook_deliveries_event_id_fkey".to_owned(),
            source: sqlx::Error::RowNotFound,
        };
        assert_eq!(dangling.category(), via_sqlx.category(), "{dangling:?}");
        assert_eq!(dangling.code(), via_sqlx.code(), "{dangling:?}");
        assert_eq!(dangling.retry(), via_sqlx.retry(), "{dangling:?}");
    }

    Ok(())
}

/// Migration 0023 makes `scan_deliveries` enqueueable, leaves 0022's six
/// alone, and still refuses a seventh nothing dispatches.
///
/// The last assertion is the one that matters: `kind_is_known` is dropped and
/// re-added on every one of these migrations, and a re-add that widened to
/// `kind IS NOT NULL` would pass every other check in this file while making
/// the constraint stop doing its job — which is to refuse, at the insert, a
/// job kind no worker knows how to dispatch.
#[tokio::test]
async fn migration_0023_opens_scan_deliveries_and_keeps_the_vocabulary_closed() -> anyhow::Result<()>
{
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let now = time::OffsetDateTime::now_utc();

    assert!(
        enqueue(
            repositories.as_ref(),
            "scan_deliveries",
            "scan:deliveries",
            now
        )
        .await?,
        "0023 must make scan_deliveries enqueueable — the backstop handler lands with it"
    );
    for kind in [
        "poll_charge",
        "resubmit_charge",
        "sweep_expired",
        "scan_live_charges",
        "fan_out_events",
        "deliver_webhook",
    ] {
        assert!(
            enqueue(
                repositories.as_ref(),
                kind,
                &format!("still-known:{kind}"),
                now
            )
            .await?,
            "the drop-and-re-add must not have lost {kind}"
        );
    }

    let refused = one_tx::enqueue_in_tx(
        repositories.as_ref(),
        "scan_delivery",
        "scan:deliveries:typo",
        &json!({}),
        now,
    )
    .await;
    assert!(
        matches!(refused, Err(vpay_db::DbError::Query(_))),
        "a kind no handler exists for must still be refused by the database: {refused:?}"
    );

    Ok(())
}

/// `endpoint_id_length` and `url_length` really do refuse an out-of-bounds
/// value — and since 2026-09-06 so does the generated input validator, one
/// layer earlier.
///
/// This is the failure `vpay_config`'s boot bounds exist to move to boot, and
/// it is also the mechanism the fan-out isolation test uses to make exactly
/// one event fail. Both claims rest on the bound firing, so it is asserted
/// rather than assumed from the migration's text.
///
/// # Why this asserts BOTH layers
///
/// `create_in_tx` moved to `WebhookDelivery.upsert(..).do_nothing()`, and
/// `schemas/vpay.cstack` declares `@length` on both columns without
/// `@db_enforce`. `run_upsert_do_nothing_in_tx` calls `input.validate()`
/// before any SQL runs, so the refusal now arrives as
/// `PersistenceError::Invalid` and the CHECK is never reached on this path.
///
/// Asserting only the new answer would quietly stop testing the CHECK, and
/// the CHECK is the half that still binds every writer that is not this
/// function — `psql`, a runbook, a future repository method. So the raw
/// inserts below go around CrateStack entirely and prove the constraint is
/// still live in the database. Deleting `CONSTRAINT endpoint_id_length` from
/// migration 0022 must fail this test, and it does: measured 2026-09-06.
#[tokio::test]
async fn a_delivery_outside_the_length_checks_is_refused_by_the_database() -> anyhow::Result<()> {
    use vpay_core::{Category, Classify as _, Retry};

    let (_container, repositories, pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_bounds", "merchant_a"),
    )
    .await?;

    // ---- layer 1: the generated validator, before the transaction writes ----
    let too_long_id = one_tx::create_in_tx(
        repositories.as_ref(),
        &event.id,
        &"p".repeat(65),
        "https://example.test/hook",
    )
    .await;
    let Err(error) = too_long_id else {
        panic!("endpoint_id_length caps endpoint_id at 64 characters: {too_long_id:?}");
    };
    assert!(
        matches!(
            &error,
            vpay_db::DbError::Persistence(vpay_db::PersistenceError::Invalid { model, .. })
                if *model == "WebhookDelivery"
        ),
        "the input validator refuses this before any SQL runs, and the error must name the \
         model that refused: {error:?}"
    );
    // The assertion that earns the variant. `Backend` — the wildcard arm
    // this would otherwise have landed on — is `Category::Storage`, which is
    // retryable, and a 65-character endpoint id is exactly as long on every
    // retry. A worker that retried this would retry it forever.
    assert_eq!(error.category(), Category::Internal, "{error:?}");
    assert_eq!(error.retry(), Retry::Never, "{error:?}");

    let too_long_url = one_tx::create_in_tx(
        repositories.as_ref(),
        &event.id,
        "ep_bounds",
        &format!("https://example.test/{}", "p".repeat(2049)),
    )
    .await;
    assert!(
        matches!(
            too_long_url,
            Err(vpay_db::DbError::Persistence(
                vpay_db::PersistenceError::Invalid { .. }
            ))
        ),
        "url_length caps url at 2048 characters: {too_long_url:?}"
    );

    // ---- layer 2: the CHECKs themselves, reached by going around the layer ----
    //
    // Raw sqlx, so the generated validator has no chance to answer first.
    // This is what still binds `psql` and every future writer.
    let raw_id = sqlx::query(
        "INSERT INTO webhook_deliveries (event_id, endpoint_id, url) VALUES ($1, $2, $3)",
    )
    .bind(&event.id)
    .bind("p".repeat(65))
    .bind("https://example.test/hook")
    .execute(&pool)
    .await;
    assert!(
        raw_id.is_err(),
        "endpoint_id_length must still refuse a 65-character id in the database itself"
    );

    let raw_url = sqlx::query(
        "INSERT INTO webhook_deliveries (event_id, endpoint_id, url) VALUES ($1, $2, $3)",
    )
    .bind(&event.id)
    .bind("ep_bounds")
    .bind(format!("https://example.test/{}", "p".repeat(2049)))
    .execute(&pool)
    .await;
    assert!(
        raw_url.is_err(),
        "url_length must still refuse an over-long url in the database itself"
    );

    Ok(())
}

/// The fan-out is at-least-once by construction: a pass that crashes before
/// committing re-runs the whole event. `create_in_tx` is what makes that
/// harmless — the second creation for the same (event, endpoint) pair
/// reports `None` and writes nothing, so the merchant is not told twice.
///
/// The `None` is the assertion that matters. A repository that returned the
/// existing row's id instead would read identically at the call site and
/// would make the caller enqueue a second `deliver_webhook` job for work
/// already queued.
#[tokio::test]
async fn a_second_delivery_for_one_event_and_endpoint_is_not_created() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_dedupe", "merchant_a"),
    )
    .await?;

    let first = create_delivery(
        repositories.as_ref(),
        &event.id,
        "ep_one",
        "https://example.test/one",
    )
    .await?
    .context("the first fan-out pass must create the delivery")?;
    let second = create_delivery(
        repositories.as_ref(),
        &event.id,
        "ep_one",
        "https://example.test/one",
    )
    .await?;
    assert!(
        second.is_none(),
        "a re-run of the fan-out must report that it created nothing, so the caller enqueues no \
         second delivery job"
    );

    // A *different* endpoint is different work, and is created.
    let other = create_delivery(
        repositories.as_ref(),
        &event.id,
        "ep_two",
        "https://example.test/two",
    )
    .await?
    .context("a second endpoint is a second delivery")?;
    assert_ne!(first, other);

    let rows = repositories.for_event(&event.id).await?;
    assert_eq!(
        rows.iter()
            .map(|row| row.endpoint_id.clone())
            .collect::<Vec<_>>(),
        vec!["ep_one".to_owned(), "ep_two".to_owned()],
        "exactly one delivery per configured endpoint, however many times the drain ran"
    );
    assert!(
        rows.iter()
            .all(|row| row.state == "pending" && row.attempt == 0 && row.next_attempt_at.is_none()),
        "a freshly created delivery is owed an attempt and has never had one: {rows:?}"
    );

    Ok(())
}

/// The other half of `the_events_insert_cannot_move_until_a_json_column_can_be_modelled`:
/// the statement CrateStack would generate for `Event.create(..)` really is
/// refused by the database, and refused for the reason claimed.
///
/// That unit test pins the *shape* with no database — `data` is absent from
/// the rendered INSERT. This one runs exactly that rendered statement and
/// observes what Postgres does with it, because "a five-column INSERT into a
/// table with a sixth `NOT NULL` column would fail" is a claim about the
/// database, and this repository does not accept those from reading.
///
/// It is deliberately raw `sqlx` rather than a call through CrateStack: the
/// generated delegate is private to `vpay-db` (`mod schema` is not `pub`,
/// ADR-0016 standard 5), and the interesting thing is the statement, not the
/// path that would have issued it. The column list below is copied from that
/// unit test's own assertion, so the two cannot drift silently — if the
/// generator starts emitting a different list, the unit test goes red first.
#[tokio::test]
async fn a_generated_events_insert_is_refused_by_the_not_null_on_data() -> anyhow::Result<()> {
    let (_container, _repositories, pool) = migrated_postgres().await?;

    let err = sqlx::query(
        "INSERT INTO events (id, merchant_id, livemode, type, object_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind("evt_no_data")
    .bind("merchant_a")
    .bind(false)
    .bind("payment_intent.succeeded")
    .bind("pi_1")
    .execute(&pool)
    .await
    .expect_err("events.data is NOT NULL with no DEFAULT, so this statement cannot succeed");

    let db_err = err.as_database_error().expect("a database-level error");
    eprintln!("observed rejection: {db_err}");
    assert_eq!(
        db_err.code().as_deref(),
        Some("23502"),
        "the refusal must be the NOT NULL specifically. A `23514` here would mean a CHECK \
         fired first and this test is not measuring what it claims; a success would mean \
         `data` acquired a DEFAULT, in which case `insert_in_tx` can move to CrateStack and \
         `the_events_insert_cannot_move_until_a_json_column_can_be_modelled` should be deleted \
         with it: {db_err}"
    );

    // Named, so a future `NOT NULL` added to some other column cannot make
    // this pass for the wrong reason.
    assert!(
        db_err.message().contains("data"),
        "the column the NOT NULL names must be `data`: {db_err}"
    );

    // And the same INSERT with `data` supplied succeeds — which is what says
    // the five columns above are otherwise complete and correct, rather than
    // the statement being malformed in some way that would mask the point.
    sqlx::query(
        "INSERT INTO events (id, merchant_id, livemode, type, object_id, data) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind("evt_with_data")
    .bind("merchant_a")
    .bind(false)
    .bind("payment_intent.succeeded")
    .bind("pi_1")
    .bind(serde_json::json!({}))
    .execute(&pool)
    .await
    .context("the same insert with data must succeed")?;

    Ok(())
}

/// **The transaction-seam test.** Both CrateStack writes in the fan-out's
/// transaction are undone by vpay's own rollback, together, and with nothing
/// left behind.
///
/// # What this is for
///
/// `create_in_tx` and `mark_fanned_out_in_tx` moved to CrateStack on
/// 2026-09-06 (`WebhookDelivery.upsert(..).do_nothing()` and
/// `Event.update_many(..)`), and they are the first vpay writes to run
/// through `run_in_tx` on a transaction **`vpay-db` did not open for
/// CrateStack's benefit** — `UnitOfWork::transaction` opened it,
/// `vpay_worker::webhooks::fan_out_one` fills it, and `TxOutcome` decides its
/// fate. `run_in_tx`'s own signature cannot tell whose transaction it has
/// been handed, which is precisely what makes the seam possible and precisely
/// what makes it unprovable by reading the code.
///
/// `docs/flows/webhooks.md`'s two-step outbox rests on this and on nothing
/// else: the drain is at-least-once, so a pass that loses the race to another
/// drain must leave **no** delivery row and **no** `deliver_webhook` job
/// behind. `mark_fanned_out_in_tx` returning `false` is how it finds out, and
/// `TxOutcome::Abandon` is how it acts on it. If either write were committing
/// on its own connection, the abandon would be a no-op and the merchant would
/// get a second copy of every event that ever lost that race.
///
/// # The mutations this exists for, both measured 2026-09-06
///
///   * `create_in_tx`: `.run_in_tx(&mut *tx, &ctx)` -> `.run(&ctx)`. The
///     delivery row **survives the abandoned transaction** and the first
///     assertion below fails, naming the cause.
///   * `mark_fanned_out_in_tx`: the same swap. `fanout_state` is `done` on an
///     event whose deliveries were rolled back — the worst of the three
///     states, because `Events::pending_page` will never return it again and
///     the merchant is never told, with nothing anywhere recording that.
///     The second assertion fails.
///
/// Neither mutation hangs, and that is worth recording because the currency
/// upsert's equivalent mutation *did*
/// (`a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`).
/// The reason is the lock shape: this transaction takes no `FOR UPDATE` on a
/// row it then asks a second connection to probe. `upsert(..).do_nothing()`'s
/// conflict probe finds no existing delivery, so it locks nothing; the
/// `events` row is held only by the `FOR KEY SHARE` the delivery's foreign
/// key takes, which does not conflict with the `FOR NO KEY UPDATE` a
/// non-key `UPDATE` wants. So both mutations fail loudly in about a second
/// rather than deadlocking, and this test is a red assertion either way.
#[tokio::test]
async fn an_abandoned_fan_out_leaves_no_delivery_and_the_event_still_pending() -> anyhow::Result<()>
{
    let (_container, repositories, pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_abandoned", "merchant_a"),
    )
    .await?;

    // Exactly `fan_out_one`'s shape, up to and including the `Abandon`: both
    // CrateStack writes succeed, and the transaction is thrown away anyway.
    let outcome = repositories
        .transaction(|tx| {
            Box::pin(async move {
                let created = tx
                    .create_in_tx("evt_abandoned", "ep_one", "https://example.test/one")
                    .await?;
                // Asserted inside the closure: if the write had silently done
                // nothing, the assertions after the rollback would pass for
                // the wrong reason and this test would be vacuous.
                assert!(
                    created.is_some(),
                    "the delivery must actually be created before it is rolled back, or this \
                     test proves nothing"
                );
                let flipped = tx.mark_fanned_out_in_tx("evt_abandoned").await?;
                assert!(
                    flipped,
                    "the compare-and-swap must actually match the pending event before it is \
                     rolled back"
                );
                Ok::<_, vpay_db::DbError>(TxOutcome::Abandon(created))
            })
        })
        .await?;
    assert!(
        matches!(outcome, TxOutcome::Abandon(Some(_))),
        "the value must come back out of an abandoned transaction: {outcome:?}"
    );

    let deliveries: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries WHERE event_id = $1")
            .bind(&event.id)
            .fetch_one(&pool)
            .await
            .context("counting deliveries must succeed")?;
    assert_eq!(
        deliveries, 0,
        "the CrateStack delivery insert must have been rolled back with the transaction. A 1 \
         here means it ran on its own connection — check that `create_in_tx` still says \
         `run_in_tx(&mut *tx, ...)` and not `run(...)`"
    );

    let state: String = sqlx::query_scalar("SELECT fanout_state FROM events WHERE id = $1")
        .bind(&event.id)
        .fetch_one(&pool)
        .await
        .context("reading the event back must succeed")?;
    assert_eq!(
        state, "pending",
        "the CrateStack fanout flip must have been rolled back too. A `done` here is an event \
         no drain will ever pick up again and no merchant will ever be told about — check that \
         `mark_fanned_out_in_tx` still says `run_in_tx(&mut *tx, ...)`"
    );

    // And the event is genuinely still drainable, not merely still labelled
    // `pending` — the label is only worth something if `pending_page` agrees.
    let backlog = repositories.pending_page(10).await?;
    assert!(
        backlog.iter().any(|row| row.id == event.id),
        "an abandoned fan-out leaves the event in the backlog for the next pass: {backlog:?}"
    );

    Ok(())
}

/// The second half of the same seam: a fan-out that **commits** really does
/// commit both CrateStack writes, so the test above cannot pass by the writes
/// simply never happening.
///
/// Cheap, and it is the control the rollback test needs. Without it, deleting
/// the bodies of both `create_in_tx` and `mark_fanned_out_in_tx` would leave
/// `an_abandoned_fan_out_leaves_no_delivery_and_the_event_still_pending`
/// green — the inner assertions guard against that, but only for as long as
/// somebody keeps them there.
#[tokio::test]
async fn a_committed_fan_out_keeps_both_cratestack_writes() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_committed", "merchant_a"),
    )
    .await?;

    repositories
        .transaction(|tx| {
            Box::pin(async move {
                let created = tx
                    .create_in_tx("evt_committed", "ep_one", "https://example.test/one")
                    .await?;
                let flipped = tx.mark_fanned_out_in_tx("evt_committed").await?;
                Ok::<_, vpay_db::DbError>(TxOutcome::Commit((created, flipped)))
            })
        })
        .await?;

    let rows = repositories.for_event(&event.id).await?;
    assert_eq!(rows.len(), 1, "exactly one delivery survives the commit");
    let delivery = rows
        .first()
        .context("the delivery just asserted to exist")?;
    assert_eq!(delivery.endpoint_id, "ep_one");
    // The columns `CreateWebhookDeliveryInput` does NOT carry, because
    // `@default(...)` filters them out of the generated input: they must
    // still arrive at their column defaults rather than at NULL or zero by
    // accident.
    assert_eq!(delivery.state, "pending");
    assert_eq!(delivery.attempt, 0);
    assert!(delivery.next_attempt_at.is_none());
    assert!(delivery.status_code.is_none());

    let state: String = sqlx::query_scalar("SELECT fanout_state FROM events WHERE id = $1")
        .bind(&event.id)
        .fetch_one(&pool)
        .await
        .context("reading the event back must succeed")?;
    assert_eq!(state, "done");

    // `created_at` is the other `@default(...)`-filtered column, and a NULL
    // would violate the column's own NOT NULL rather than reaching here — so
    // this asserts the value is the database's clock, not merely present.
    let created_at_is_recent: bool = sqlx::query_scalar(
        "SELECT created_at > now() - INTERVAL '1 minute' FROM webhook_deliveries WHERE id = $1",
    )
    .bind(delivery.id)
    .fetch_one(&pool)
    .await
    .context("reading created_at back must succeed")?;
    assert!(
        created_at_is_recent,
        "created_at must come from the column default the generated input omits"
    );

    Ok(())
}

/// One failed attempt records what the receiver said, moves the ladder on,
/// and — when the ladder runs out — parks the delivery as `exhausted` where
/// nothing will pick it up again.
///
/// Three separate claims, all of which a plausible-looking implementation
/// gets wrong: the excerpt is bounded to the column's CHECK rather than
/// letting the write fail (which would lose the very record it exists to
/// make), a transport failure leaves `responded_at` NULL so it cannot be
/// misread as a refusal that was actually heard, and `payload_sha256` keeps
/// the *first* attempt's digest so "we sent what we signed" stays checkable
/// across attempts.
#[tokio::test]
async fn record_attempt_bounds_the_excerpt_moves_the_ladder_and_then_exhausts() -> anyhow::Result<()>
{
    let (_container, repositories, pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_attempts", "merchant_a"),
    )
    .await?;
    let id = create_delivery(
        repositories.as_ref(),
        &event.id,
        "ep_one",
        "https://example.test/one",
    )
    .await?
    .context("the delivery must be created")?;

    // A receiver answering 500 with a long, multi-byte error page: the
    // excerpt has to survive as *something*, bounded to `excerpt_length`.
    let overlong = "é".repeat(2500);
    let due = time::OffsetDateTime::now_utc() + time::Duration::seconds(10);
    assert!(
        repositories
            .record_attempt(
                id,
                Some(500),
                Some(&overlong),
                Some("sha-first"),
                Some(due),
                false,
            )
            .await?,
        "the first attempt must match the pending row"
    );

    let row = delivery(repositories.as_ref(), id).await?;
    assert_eq!(row.attempt, 1, "attempt counts failures");
    assert_eq!(
        row.state, "pending",
        "a failure that is not exhausted is still owed an attempt"
    );
    assert_eq!(row.status_code, Some(500));
    assert_eq!(row.payload_sha256.as_deref(), Some("sha-first"));
    let excerpt = row
        .response_excerpt
        .context("the excerpt must be recorded")?;
    assert_eq!(
        excerpt.chars().count(),
        2000,
        "the excerpt is truncated to the column's CHECK, in characters, rather than refused"
    );
    assert!(
        excerpt.chars().all(|c| c == 'é'),
        "and cut on a character boundary"
    );
    let scheduled = row
        .next_attempt_at
        .context("the next attempt must be scheduled")?;
    assert!(
        (scheduled - due).abs() < time::Duration::seconds(1),
        "next_attempt_at is the caller's instant, not one this layer invented: {scheduled} vs {due}"
    );
    let (sent, responded, _) = delivery_timestamps(&pool, id).await?;
    assert!(
        sent && responded,
        "a refusal that was heard has both a send and a response"
    );

    // A transport failure: the request went out, nothing came back.
    assert!(
        repositories
            .record_attempt(id, None, None, Some("sha-second"), Some(due), false,)
            .await?
    );
    let row = delivery(repositories.as_ref(), id).await?;
    assert_eq!(row.attempt, 2);
    assert_eq!(row.status_code, None);
    assert_eq!(
        row.payload_sha256.as_deref(),
        Some("sha-first"),
        "the first attempt's digest is the one that survives — a later attempt must not be able \
         to make the row agree with bytes that were never signed"
    );
    let (sent, responded, _) = delivery_timestamps(&pool, id).await?;
    assert!(
        sent && !responded,
        "sent with no status and no responded_at is the transport-failure shape, and it must not \
         be recorded as a refusal the receiver actually made"
    );

    // The ladder runs out.
    assert!(
        repositories
            .record_attempt(id, Some(500), None, Some("sha"), None, true)
            .await?
    );
    let row = delivery(repositories.as_ref(), id).await?;
    assert_eq!(row.state, "exhausted");
    assert_eq!(row.attempt, 3);
    assert!(
        row.next_attempt_at.is_none(),
        "an exhausted delivery is owed nothing"
    );

    // And nothing can walk it further: the guard is `state = 'pending'`.
    assert!(
        !repositories
            .record_attempt(id, Some(500), None, Some("sha"), None, true)
            .await?,
        "a replayed job must not be able to keep incrementing an exhausted delivery"
    );
    assert_eq!(delivery(repositories.as_ref(), id).await?.attempt, 3);

    Ok(())
}

/// A `2xx` finishes the delivery, and only the first one does.
///
/// The guard is the point. Two workers can hold the same job — a lease
/// reaped while the first was still running is exactly the case
/// `jobs::finish` returns `false` for — and a second `record_success` that
/// wrote anyway would stamp a fresh `responded_at` on an attempt that had
/// already been settled, and could resurrect a delivery an operator had
/// deliberately parked.
#[tokio::test]
async fn record_success_settles_a_delivery_once_and_a_second_call_writes_nothing()
-> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_success", "merchant_a"),
    )
    .await?;
    let id = create_delivery(
        repositories.as_ref(),
        &event.id,
        "ep_one",
        "https://example.test/one",
    )
    .await?
    .context("the delivery must be created")?;

    // One failure first, so the success has something to clear.
    let due = time::OffsetDateTime::now_utc() + time::Duration::seconds(10);
    assert!(
        repositories
            .record_attempt(
                id,
                Some(503),
                Some("busy"),
                Some("sha-body"),
                Some(due),
                false,
            )
            .await?
    );

    assert!(
        repositories
            .record_success(id, 200, Some("ok"), "sha-body")
            .await?,
        "the first 2xx must settle the delivery"
    );
    let row = delivery(repositories.as_ref(), id).await?;
    assert_eq!(row.state, "succeeded");
    assert_eq!(row.status_code, Some(200));
    assert_eq!(row.response_excerpt.as_deref(), Some("ok"));
    assert_eq!(
        row.attempt, 1,
        "attempt counts failures, so a success does not increment it — it is the ladder's index"
    );
    assert!(
        row.next_attempt_at.is_none(),
        "a succeeded delivery must not stay due, or the backstop scan would keep finding it"
    );

    assert!(
        !repositories
            .record_success(id, 201, Some("again"), "sha-body")
            .await?,
        "a second worker running the same job must change nothing"
    );
    let row = delivery(repositories.as_ref(), id).await?;
    assert_eq!(
        row.status_code,
        Some(200),
        "the settled row keeps the answer that settled it"
    );
    assert_eq!(row.response_excerpt.as_deref(), Some("ok"));

    Ok(())
}

/// The fan-out's closing write flips `pending → done` and nothing else.
///
/// `Ok(false)` on the second call is what tells a racing drain that the
/// backlog entry was not its to claim, so it rolls back instead of
/// committing a second set of deliveries. Without the `AND fanout_state =
/// 'pending'` half, both passes would report success and only the unique
/// index would stand between the merchant and two of every webhook.
#[tokio::test]
async fn mark_fanned_out_flips_a_pending_event_once() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_fanout", "merchant_a"),
    )
    .await?;
    assert_eq!(fanout_state(&pool, &event.id).await?, "pending");

    let flipped = one_tx::mark_fanned_out_in_tx(repositories.as_ref(), &event.id).await?;
    assert!(flipped, "the first pass claims the backlog entry");
    assert_eq!(fanout_state(&pool, &event.id).await?, "done");

    let again = one_tx::mark_fanned_out_in_tx(repositories.as_ref(), &event.id).await?;
    assert!(
        !again,
        "an event already fanned out must refuse the second claim rather than silently agreeing"
    );

    // An event that does not exist is the same quiet `false`, not an error:
    // a drain reading a page whose event was removed underneath it has
    // nothing to do, and nothing to page anyone about.
    let missing = one_tx::mark_fanned_out_in_tx(repositories.as_ref(), "evt_not_here").await?;
    assert!(!missing);

    // And the backlog query agrees: nothing is pending any more.
    assert!(repositories.pending_page(10).await?.is_empty());

    Ok(())
}

/// The backstop scan returns exactly the deliveries nothing is driving — not
/// the ones scheduled for later, not the ones already settled, and not the
/// freshly created ones the queue still owns.
///
/// Every arm of the predicate is load-bearing. Without `state = 'pending'` a
/// settled delivery with an old `next_attempt_at` would be re-delivered;
/// without `next_attempt_at <= now()` the scan would drag every rung of the
/// retry ladder back to now, which is how a ladder becomes a hot loop against
/// a receiver that is already struggling; and without the `next_attempt_at IS
/// NULL AND created_at < now() - lease` arm a delivery whose job was deleted
/// before it was ever attempted is owed an attempt nothing will ever make.
///
/// The last two fixtures are the pair that pins that arm: two rows that
/// differ only in age, one older than the lease and one not. A scan that
/// simply treated NULL as "due now" would return both, and would then race
/// the queue on every delivery the fan-out had just created.
#[tokio::test]
async fn pending_due_returns_the_deliveries_nothing_is_driving() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let event = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_due", "merchant_a"),
    )
    .await?;
    let now = time::OffsetDateTime::now_utc();
    // `RecoveryPolicy::lease`'s five minutes, transcribed: this crate cannot
    // see `vpay-worker`, and the caller passes exactly that value.
    let lease = std::time::Duration::from_secs(5 * 60);

    let mut ids = Vec::new();
    for endpoint in [
        "ep_due",
        "ep_later",
        "ep_settled",
        "ep_untried_young",
        "ep_untried_stranded",
    ] {
        ids.push((
            endpoint,
            create_delivery(
                repositories.as_ref(),
                &event.id,
                endpoint,
                "https://example.test/hook",
            )
            .await?
            .context("the delivery must be created")?,
        ));
    }
    let id_of = |name: &str| -> anyhow::Result<uuid::Uuid> {
        ids.iter()
            .find(|(endpoint, _)| *endpoint == name)
            .map(|(_, id)| *id)
            .context("fixture delivery must exist")
    };

    // Due an hour ago.
    repositories
        .record_attempt(
            id_of("ep_due")?,
            Some(500),
            None,
            Some("sha"),
            Some(now - time::Duration::hours(1)),
            false,
        )
        .await?;
    // Not due for another hour.
    repositories
        .record_attempt(
            id_of("ep_later")?,
            Some(500),
            None,
            Some("sha"),
            Some(now + time::Duration::hours(1)),
            false,
        )
        .await?;
    // Overdue on paper but already settled. Written by hand rather than
    // through `record_success`, which would also clear `next_attempt_at` —
    // this is the row that isolates the `state` half of the predicate.
    repositories
        .record_attempt(
            id_of("ep_settled")?,
            Some(500),
            None,
            Some("sha"),
            Some(now - time::Duration::hours(1)),
            false,
        )
        .await?;
    sqlx::query("UPDATE webhook_deliveries SET state = 'succeeded' WHERE id = $1")
        .bind(id_of("ep_settled")?)
        .execute(&pool)
        .await
        .context("parking the settled fixture must succeed")?;
    // `ep_untried_young` keeps both `next_attempt_at IS NULL` and its default
    // `created_at`: its delivery job was written in the same transaction it
    // was, and has simply not been claimed yet. The queue owns it.
    //
    // `ep_untried_stranded` is the same row an hour older. Nothing has ever
    // attempted it, which after a whole lease means its job is not merely
    // waiting — it is gone.
    sqlx::query(
        "UPDATE webhook_deliveries SET created_at = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(id_of("ep_untried_stranded")?)
    .execute(&pool)
    .await
    .context("ageing the stranded fixture must succeed")?;

    let due = repositories.pending_due(lease, 10).await?;
    let endpoints: Vec<String> = due.iter().map(|row| row.endpoint_id.clone()).collect();
    assert_eq!(
        endpoints.len(),
        2,
        "exactly the overdue one and the stranded one: {due:?}"
    );
    assert!(
        endpoints.contains(&"ep_due".to_owned()),
        "a delivery whose next attempt has arrived must be found: {endpoints:?}"
    );
    assert!(
        endpoints.contains(&"ep_untried_stranded".to_owned()),
        "a never-attempted delivery older than the lease has lost its job and must be \
         found: {endpoints:?}"
    );
    assert!(
        !endpoints.contains(&"ep_untried_young".to_owned()),
        "a delivery the queue has not got to yet must be left alone, or the scan races the \
         fan-out on every row it creates: {endpoints:?}"
    );

    Ok(())
}

/// `GET /v1/events` paging, proven the same way the payment-intent list is:
/// forward to the end, backward to the start, an unknown cursor, and another
/// merchant's events invisible throughout.
///
/// The merchant scope is the authorisation here, so the `merchant_b` row is
/// not decoration — it is the only thing in this test that fails if the
/// `WHERE merchant_id = $1` or either cursor subquery's scope is dropped.
#[tokio::test]
async fn events_list_page_walks_forward_and_backward_over_twenty_five_events() -> anyhow::Result<()>
{
    let (_container, repositories, _pool) = migrated_postgres().await?;

    for n in 0..25 {
        let row = insert_event(
            repositories.as_ref(),
            &fixture_event(&event_page_id(n), "merchant_a"),
        )
        .await?;
        assert_eq!(row.id, event_page_id(n));
    }
    let other = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_other_merchant", "merchant_b"),
    )
    .await?;

    let ids = |rows: &[vpay_db::EventRow]| -> Vec<String> {
        rows.iter().map(|row| row.id.clone()).collect()
    };
    let newest_first =
        |from: usize, to: usize| -> Vec<String> { (from..=to).rev().map(event_page_id).collect() };

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: None,
    };
    let (first, has_more) = Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert_eq!(
        ids(&first),
        newest_first(15, 24),
        "the default page is the newest 10"
    );
    assert!(has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some(event_page_id(15)),
        ending_before: None,
    };
    let (second, has_more) = Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert_eq!(ids(&second), newest_first(5, 14));
    assert!(has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some(event_page_id(5)),
        ending_before: None,
    };
    let (third, has_more) = Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert_eq!(ids(&third), newest_first(0, 4));
    assert!(
        !has_more,
        "the last forward page must report that nothing follows it"
    );

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: Some(event_page_id(4)),
    };
    let (back, has_more) = Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert_eq!(
        ids(&back),
        ids(&second),
        "ending_before returns the page before, in the same newest-first order the envelope \
         always promises"
    );
    assert!(has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: Some(event_page_id(15)),
    };
    let (tail, has_more) = Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert_eq!(ids(&tail), newest_first(16, 24));
    assert!(
        !has_more,
        "9 rows for a limit of 10 is the end of the range in this direction"
    );

    // Another merchant's event id is not a position in this merchant's
    // range: the cursor subquery is scoped too, so it resolves to NULL and
    // the page is empty rather than a silent fallback to the newest rows.
    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some(other.id.clone()),
        ending_before: None,
    };
    let (foreign_cursor, has_more) =
        Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert!(foreign_cursor.is_empty() && !has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: Some("evt_does_not_exist".to_owned()),
        ending_before: None,
    };
    let (unknown, has_more) = Events::list_page(repositories.as_ref(), "merchant_a", &page).await?;
    assert!(unknown.is_empty() && !has_more);

    let page = vpay_db::ListPage {
        limit: 10,
        starting_after: None,
        ending_before: None,
    };
    let (others, _) = Events::list_page(repositories.as_ref(), "merchant_b", &page).await?;
    assert_eq!(
        ids(&others),
        vec!["evt_other_merchant".to_owned()],
        "every page is merchant-scoped in SQL"
    );

    Ok(())
}

/// `GET /v1/events/{id}` is merchant-scoped, and another merchant's event id
/// is indistinguishable from one that does not exist.
///
/// Both halves matter: a `404` that could be told apart from a `403` lets
/// anyone with one merchant's credentials enumerate which `evt_…` ids exist
/// across the whole deployment, and an unscoped read hands them the payload.
#[tokio::test]
async fn events_get_by_id_is_merchant_scoped() -> anyhow::Result<()> {
    let (_container, repositories, _pool) = migrated_postgres().await?;

    let mine = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_mine", "merchant_a"),
    )
    .await?;
    let theirs = insert_event(
        repositories.as_ref(),
        &fixture_event("evt_theirs", "merchant_b"),
    )
    .await?;

    let found = Events::get_by_id(repositories.as_ref(), "merchant_a", &mine.id)
        .await?
        .context("a merchant must be able to read their own event")?;
    assert_eq!(found, mine);

    assert!(
        Events::get_by_id(repositories.as_ref(), "merchant_a", &theirs.id)
            .await?
            .is_none(),
        "another merchant's event must read as absent, not as forbidden"
    );
    assert!(
        Events::get_by_id(repositories.as_ref(), "merchant_a", "evt_does_not_exist")
            .await?
            .is_none()
    );
    // And the scope is not merely reversed: merchant_b really does have
    // theirs, so the assertion above is about the filter and not about a
    // fixture that failed to insert.
    assert_eq!(
        Events::get_by_id(repositories.as_ref(), "merchant_b", &theirs.id).await?,
        Some(theirs)
    );

    Ok(())
}
