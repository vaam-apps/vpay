//! Integration tests for `vpay-db` against a real Postgres via testcontainers.
//!
//! Mirrors the pattern in `backends/tests/integration/tests/postgres_smoke.rs`
//! (that crate is owned by another agent and out of scope here — this file
//! is `vpay-db`'s own copy of the same, deliberately duplicated rather than
//! shared, technique): `testcontainers-modules` 0.15 defaults to
//! `postgres:11-alpine`, which is not cached on this machine and this
//! machine cannot reach Docker Hub to pull it. `16-alpine` IS cached
//! locally and matches `compose.yml`, hence the explicit `.with_tag(...)`
//! below.
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
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Starts a fresh, unmigrated Postgres 16 container and returns its
/// connection URL. The returned container guard must be kept alive for as
/// long as the URL is used — dropping it stops and removes the container.
async fn start_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, String)> {
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
