//! Integration tests for `vpay-db` against a real Postgres via testcontainers.
//!
//! The container itself is started by
//! `vpay_testkit::containers::start_postgres_with_retry`, which every
//! Postgres-backed suite in this workspace now shares (this file's own copy
//! of the bootstrap was one of eight identical ones). Why the image tag is
//! pinned to `16-alpine`, and why a start is retried on a host-port
//! collision, are documented there rather than repeated here.
//!
//! Deliberately three tests, not four separate ones, to keep this crate's
//! own container count down: `.config/nextest.toml` bounds
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
    let pool = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;

    vpay_db::run_migrations(&pool)
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
    sqlx::query("SELECT 1 FROM currencies LIMIT 1")
        .fetch_optional(&pool)
        .await
        .context("querying a table created by migration 0001_create-currencies.sql")?;

    // Idempotent: sqlx tracks applied migrations by checksum in its own
    // `_sqlx_migrations` table and skips anything already recorded, so a
    // second run against the same database must succeed, not error.
    vpay_db::run_migrations(&pool)
        .await
        .context("second migration run against an already-migrated database should be a no-op")?;

    Ok(())
}

/// The `/healthz`-style connectivity check, against a database that is
/// actually up.
#[tokio::test]
async fn check_connection_succeeds_against_a_live_database() -> anyhow::Result<()> {
    let (_container, url) = start_postgres().await?;
    let pool = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;

    vpay_db::check_connection(&pool)
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
    let pool = vpay_db::connect(&url)
        .await
        .context("connecting to the freshly started container")?;

    vpay_db::check_connection(&pool)
        .await
        .context("must be reachable before it is stopped, or a later failure proves nothing")?;

    container
        .stop_with_timeout(None)
        .await
        .context("stopping the container to simulate a dead database")?;

    let result = vpay_db::check_connection(&pool).await;
    assert!(
        result.is_err(),
        "check_connection should fail once the database has been stopped, not report ok"
    );

    Ok(())
}
