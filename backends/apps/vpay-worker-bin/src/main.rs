//! vpay worker: submit, poll, reconcile, deliver.
//!
//! The job loop itself is not implemented (`docs/status.md`) — this process
//! supervises nothing yet. It used to exit immediately on start, on the
//! theory that idling in a loop that did nothing would look like a running
//! worker in `docker compose ps`. In practice that made the compose stack
//! unstable: an orchestrator (compose, k8s) that expects a long-running
//! process treats an immediate clean exit as a crash loop. So instead it
//! stays up and answers the same shutdown signals `vpay-server` does, while
//! shouting — loudly and repeatedly — that it is doing no real work. That
//! makes shutdown/orchestration behaviour real and testable today, without
//! pretending the job loop exists.

use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use mimalloc::MiMalloc;
use vpay_config::{LogFormat, ShutdownSignals, WorkerArgs};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// How often the "not implemented" banner repeats while the process is up.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = WorkerArgs::parse();

    // Installed before anything else — including tracing init — so the
    // SIGTERM/SIGINT handlers are live for this process's entire lifetime,
    // not just once the shutdown future below is first polled. See
    // vpay_config::signal for the race this closes. A failure here is a
    // hard startup failure (see ShutdownSignals::install's docs for why),
    // deliberately not a logged warning that lets the process continue with
    // no graceful shutdown path at all.
    let mut shutdown_signals =
        ShutdownSignals::install().context("installing SIGINT/SIGTERM handlers")?;

    init_tracing(&args.common.log_filter, args.common.log_format);

    // `profile` names a YAML config file, never a code path — see
    // vpay_config::cli and docs/adr/0003-yaml-configuration.md.
    tracing::info!(profile = %args.common.profile, "deployment profile (selects a config file only)");

    // `--database-url` / `DATABASE_URL` stays `Option<String>` at the clap
    // level (`vpay_config::CommonArgs`, out of scope for this change — see
    // docs/status.md) but is treated as required *here*, matching
    // `vpay-server`'s decision (see that binary's `main.rs` for the fuller
    // reasoning): a process that boots with no database at all is not doing
    // the thing this pass exists to do, and silently skipping the connection
    // when the flag is absent would leave it dormant behind an optional
    // flag rather than the live path this repo's delivery style requires.
    let database_url = args.common.database_url.as_deref().context(
        "--database-url / DATABASE_URL is required: vpay-worker-bin cannot start without \
         a database to open a pool against and migrate (see docs/status.md)",
    )?;

    // Connect and migrate at startup, same as `vpay-server` — "database
    // connectivity and migrations at boot" applies to both binaries, not
    // just the one with an HTTP listener. The pool itself has no consumer
    // yet (the job loop is not implemented, docs/status.md) so it is not
    // held past this point; the goal here is only the loud, fail-fast proof
    // that the database is reachable and up to date before this process
    // claims to be running.
    let pool = vpay_db::connect(database_url)
        .await
        .context("connecting to Postgres")?;
    vpay_db::run_migrations(&pool)
        .await
        .context("running database migrations")?;
    tracing::info!("database connected and migrations applied");
    drop(pool);

    tracing::warn!(
        "vpay-worker-bin is a scaffold: the job loop is NOT implemented. No jobs \
         are being dequeued, polled, or delivered. This process stays up only to \
         answer shutdown signals correctly. See docs/status.md."
    );

    // `--shutdown-grace-seconds` / `VPAY_SHUTDOWN_GRACE_SECONDS` is parsed
    // and validated here (it is shared with `vpay-server` via
    // `CommonArgs`), but intentionally not consumed below: there is no
    // in-flight work for it to bound yet, because the job loop it is meant
    // to bound the drain of does not exist (docs/status.md). Once that loop
    // is implemented, this value should bound how long it waits for an
    // in-flight job to finish after a shutdown signal, the same way
    // `vpay-server`'s `main.rs` bounds its HTTP request drain. Wiring it in
    // now, ahead of the loop it is meant to bound, would be exactly the
    // "dormant behind a flag" anti-pattern this repo forbids — so it stays
    // unused here until there is real work to bound.
    tracing::debug!(
        shutdown_grace_seconds = args.common.shutdown_grace_seconds,
        "shutdown grace period accepted for CLI parity with vpay-server; has no effect yet, \
         there is no job loop for it to bound (see docs/status.md)"
    );

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    // The first tick fires immediately; skip it, the startup banner above
    // already said this once.
    heartbeat.tick().await;

    let shutdown = shutdown_signals.wait();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            () = &mut shutdown => {
                tracing::info!("graceful shutdown complete, exiting");
                break;
            }
            _ = heartbeat.tick() => {
                tracing::warn!(
                    "vpay-worker-bin heartbeat: job loop still not implemented, no jobs \
                     are being processed. See docs/status.md."
                );
            }
        }
    }

    Ok(())
}

/// Initialises the global tracing subscriber per `--log-format`.
///
/// The two formats produce differently-typed subscriber builders, so this is
/// a match over independent `.init()` calls rather than one shared pipeline.
fn init_tracing(log_filter: &str, log_format: LogFormat) {
    match log_format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter(log_filter))
                .init();
        }
        LogFormat::Text => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter(log_filter))
                .init();
        }
    }
}

fn env_filter(directive: &str) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_new(directive)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
}

// SIGINT/SIGTERM handling itself now lives in `vpay_config::ShutdownSignals`,
// installed eagerly at the top of `main` (see the comment there) rather than
// constructed here — a signal handler constructed this late would only be
// installed once its future is first polled, which is exactly the startup
// race that type exists to close. Mirrors `vpay-server`'s shutdown handling
// so both binaries behave identically under `docker compose down`.
