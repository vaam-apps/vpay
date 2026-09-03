//! Integration tests for `vpay-db` against a real Postgres via testcontainers.
//!
//! The container itself is started by
//! `vpay_testkit::containers::start_postgres_with_retry`, which every
//! Postgres-backed suite in this workspace now shares (this file's own copy
//! of the bootstrap was one of eight identical ones). Why the image tag is
//! pinned to `16-alpine`, and why a start is retried on a host-port
//! collision, are documented there rather than repeated here.
//!
//! Deliberately few, chunky tests rather than one per assertion, to keep
//! this crate's own container count down: `.config/nextest.toml` bounds
//! `package(vpay-tests-integration)`'s container tests to one at a time
//! because 13+ concurrent container starts caused real flakes on this
//! machine's 4-vCPU Docker Desktop VM (see that file's own comment) — this
//! crate is *not* covered by that filter (it does not own `.config/
//! nextest.toml` and is told not to add itself to it), so nextest's default
//! per-CPU parallelism applies to these tests too. Fewer, chunkier tests
//! here reduces how many containers this crate can have starting at the
//! same moment regardless.
//!
//! Helper functions return `anyhow::Result` and propagate with `?` rather
//! than `.expect`/`.unwrap` — the workspace lint policy's test exemption
//! (`clippy.toml`) only covers code inside a `#[test]`-attributed function
//! body, not a plain helper one happens to call.

use anyhow::Context;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_db::{TxOutcome, UnitOfWork as _};

/// Starts a fresh, unmigrated Postgres 16 container and returns its
/// connection URL. The returned container guard must be kept alive for as
/// long as the URL is used — dropping it stops and removes the container.
///
/// The image request and its host-port-collision retry live in
/// `vpay_testkit::containers` — see that module for why the tag is pinned and
/// which errors are retried.
async fn start_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, String)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
        .await
        .context("postgres:16-alpine container starts (it is cached locally on this machine)")?;

    let host = container.get_host().await.context("container host")?;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .context("container port")?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    Ok((container, url))
}

/// Covers two of the four required cases at once: migrations apply cleanly
/// to a fresh database, and running them a second time is a no-op rather
/// than an error.
#[tokio::test]
async fn run_migrations_applies_cleanly_and_is_idempotent() -> anyhow::Result<()> {
    let (_container, url) = start_postgres().await?;
    let repositories = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;

    repositories
        .run_migrations()
        .await
        .context("first migration run against an empty database")?;

    // Prove the schema is actually usable, not just that `migrate!` returned
    // `Ok` — query a table created by a migration. `currencies`
    // (0001_create-currencies.sql) rather than a later one: this crate does
    // not own `backends/migrations` (another agent actively evolves it —
    // e.g. 0008's `merchant_api_keys` was created then dropped again by a
    // later migration in the same pass this test was written), so pin this
    // assertion to the one table least likely to be restructured out from
    // under it.
    let pool = sqlx::PgPool::connect(&url)
        .await
        .context("a raw-SQL pool against the same container")?;
    sqlx::query("SELECT 1 FROM currencies LIMIT 1")
        .fetch_optional(&pool)
        .await
        .context("querying a table created by migration 0001_create-currencies.sql")?;

    // Idempotent: sqlx tracks applied migrations by checksum in its own
    // `_sqlx_migrations` table and skips anything already recorded, so a
    // second run against the same database must succeed, not error.
    repositories
        .run_migrations()
        .await
        .context("second migration run against an already-migrated database should be a no-op")?;

    Ok(())
}

/// The `/healthz`-style connectivity check, against a database that is
/// actually up.
#[tokio::test]
async fn check_connection_succeeds_against_a_live_database() -> anyhow::Result<()> {
    let (_container, url) = start_postgres().await?;
    let repositories = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;

    repositories
        .check_connection()
        .await
        .context("healthcheck against a live database should succeed")?;

    Ok(())
}

/// The same check, against a database that has since gone away — the case
/// `/healthz` exists to catch. Connects while the container is alive (and
/// proves that connection actually works, so a later failure is provably
/// caused by stopping the container and nothing else), then stops the
/// container out from under the already-built pool.
#[tokio::test]
async fn check_connection_fails_against_a_dead_database() -> anyhow::Result<()> {
    let (container, url) = start_postgres().await?;
    let repositories = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;

    repositories
        .check_connection()
        .await
        .context("must be reachable before it is stopped, or a later failure proves nothing")?;

    container
        .stop_with_timeout(None)
        .await
        .context("stopping the container to simulate a dead database")?;

    let result = repositories.check_connection().await;
    assert!(
        result.is_err(),
        "check_connection should fail once the database has been stopped, not report ok"
    );

    Ok(())
}

/// Terminates the backend holding an open transaction, and returns how many
/// it terminated.
///
/// A single connection is killed rather than the whole container (which the
/// dead-database test above does) because the caller needs the *rest* of the
/// database to keep working: it runs the same staging twice against one
/// container and compares the two endings. `idle in transaction` is exactly
/// the state a connection is in between a statement inside a `BEGIN` and its
/// `COMMIT`/`ROLLBACK`; the killer's own connection is `active` and excluded
/// twice over.
async fn terminate_backends_holding_a_transaction(pool: &sqlx::PgPool) -> anyhow::Result<usize> {
    let terminated: Vec<bool> = sqlx::query_scalar(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() \
           AND pid <> pg_backend_pid() \
           AND state = 'idle in transaction'",
    )
    .fetch_all(pool)
    .await
    .context("terminating the backend that holds the open transaction")?;

    Ok(terminated.into_iter().filter(|killed| *killed).count())
}

/// A `TxOutcome::Abandon` whose connection died before `ROLLBACK` could be
/// sent still answers `Ok(Abandon)` — and the identical staging on the
/// commit path still answers `Err`.
///
/// The pair is the whole test. The second half is the control: it proves the
/// connection really was unusable by the time the transaction was finished,
/// so the first half's `Ok` is a deliberate swallow and not a rollback that
/// quietly succeeded. Restoring `pending.rollback().await?` in
/// `UnitOfWork::transaction` fails the first half and leaves the second
/// passing.
///
/// Why the swallow is right: `ROLLBACK` is best-effort by construction — a
/// transaction whose connection died is aborted by the server either way —
/// so the failure changes nothing about the database and only about what the
/// caller may say. Both abandoning call sites have an answer that must
/// survive: the confirm path's duplicate-charge recovery owes the merchant
/// its `409`, and `persist_submitted` owes an operator the `Internal` alert
/// saying a rail may hold a live payment. A `503` in place of either is the
/// loss of the only report of it.
#[tokio::test]
async fn an_abandoned_transaction_survives_a_rollback_it_cannot_send() -> anyhow::Result<()> {
    let (_container, url) = start_postgres().await?;
    let repositories = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;
    repositories
        .run_migrations()
        .await
        .context("the transaction below writes through payment_intents")?;

    // The pool `authkestra`'s store is handed (`op_store_pool`), used here
    // as the *second* connection the kill has to arrive on: the first is
    // busy holding the transaction being killed.
    let pool = repositories.op_store_pool();

    let abandoned = repositories
        .transaction(|tx| {
            Box::pin(async {
                // One statement inside the `BEGIN`, so the backend holding
                // this transaction is findable. A compare-and-swap against
                // an intent that does not exist matches no row and answers
                // `Ok(None)` — it writes nothing, which keeps this test
                // about the transaction's ending and not about its content.
                let matched = tx
                    .transition_in_tx(
                        "acct_nobody",
                        "pi_nobody",
                        "requires_payment_method",
                        "processing",
                    )
                    .await?;
                assert!(matched.is_none(), "no such intent, so no row moves");

                assert_eq!(
                    terminate_backends_holding_a_transaction(&pool).await?,
                    1,
                    "exactly the connection this closure is writing through must be killed, \
                     or the ROLLBACK below would succeed and prove nothing"
                );

                Ok::<_, anyhow::Error>(TxOutcome::Abandon("the answer the caller owes"))
            })
        })
        .await
        .context("an abandoned transaction must not surface its own rollback failure")?;

    assert!(
        matches!(abandoned, TxOutcome::Abandon("the answer the caller owes")),
        "the closure's value must come back unchanged, not be replaced by a storage error"
    );

    // The control, on the same container: the same broken connection still
    // fails a *commit*, because a commit that did not happen is a fact the
    // caller cannot be allowed to miss.
    let committed = repositories
        .transaction(|tx| {
            Box::pin(async {
                let matched = tx
                    .transition_in_tx(
                        "acct_nobody",
                        "pi_nobody",
                        "requires_payment_method",
                        "processing",
                    )
                    .await?;
                assert!(matched.is_none(), "no such intent, so no row moves");

                assert_eq!(
                    terminate_backends_holding_a_transaction(&pool).await?,
                    1,
                    "the control has to break the connection the same way"
                );

                Ok::<_, anyhow::Error>(TxOutcome::Commit(()))
            })
        })
        .await;

    assert!(
        committed.is_err(),
        "a COMMIT that could not be sent must be an error — if this passes, the abandon \
         half above is not proving that the connection was broken"
    );

    Ok(())
}
