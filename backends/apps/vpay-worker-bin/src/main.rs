//! vpay worker: submit, poll, reconcile, deliver.
//!
//! The job loop itself is not implemented (`docs/STATUS.md`) — this process
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

use clap::Parser as _;
use mimalloc::MiMalloc;
use vpay_config::{LogFormat, WorkerArgs};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// How often the "not implemented" banner repeats while the process is up.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = WorkerArgs::parse();
    init_tracing(&args.common.log_filter, args.common.log_format);

    // `profile` names a YAML config file, never a code path — see
    // vpay_config::cli and docs/adr/0003-yaml-configuration.md.
    tracing::info!(profile = %args.common.profile, "deployment profile (selects a config file only)");

    tracing::warn!(
        "vpay-worker-bin is a scaffold: the job loop is NOT implemented. No jobs \
         are being dequeued, polled, or delivered. This process stays up only to \
         answer shutdown signals correctly. See docs/STATUS.md."
    );

    // `--shutdown-grace-seconds` / `VPAY_SHUTDOWN_GRACE_SECONDS` is parsed
    // and validated here (it is shared with `vpay-server` via
    // `CommonArgs`), but intentionally not consumed below: there is no
    // in-flight work for it to bound yet, because the job loop it is meant
    // to bound the drain of does not exist (docs/STATUS.md). Once that loop
    // is implemented, this value should bound how long it waits for an
    // in-flight job to finish after a shutdown signal, the same way
    // `vpay-server`'s `main.rs` bounds its HTTP request drain. Wiring it in
    // now, ahead of the loop it is meant to bound, would be exactly the
    // "dormant behind a flag" anti-pattern this repo forbids — so it stays
    // unused here until there is real work to bound.
    tracing::debug!(
        shutdown_grace_seconds = args.common.shutdown_grace_seconds,
        "shutdown grace period accepted for CLI parity with vpay-server; has no effect yet, \
         there is no job loop for it to bound (see docs/STATUS.md)"
    );

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    // The first tick fires immediately; skip it, the startup banner above
    // already said this once.
    heartbeat.tick().await;

    let shutdown = shutdown_signal();
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
                     are being processed. See docs/STATUS.md."
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

/// Waits for SIGINT (Ctrl+C) or, on Unix, SIGTERM — whichever arrives first —
/// then logs and returns. Mirrors `vpay-server`'s shutdown handling so both
/// binaries behave identically under `docker compose down`.
async fn shutdown_signal() {
    let ctrl_c = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {}
            Err(err) => {
                // No panic in a shutdown path: log and fall back to waiting
                // on the other signal source instead.
                tracing::error!(%err, "failed to install Ctrl+C handler; this shutdown path is now inert");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!(%err, "failed to install SIGTERM handler; this shutdown path is now inert");
                std::future::pending::<()>().await;
            }
        }
    };

    // SIGTERM handling is Unix-only; other platforms shut down on Ctrl+C
    // alone rather than failing to compile.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received SIGINT, starting graceful shutdown"),
        () = terminate => tracing::info!("received SIGTERM, starting graceful shutdown"),
    }
}
