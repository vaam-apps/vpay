//! vpay API server.
//!
//! Writes rows and returns. It never calls a payment rail — that is the
//! worker's job, and it is what makes the system crash-safe.

use std::future::IntoFuture as _;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use mimalloc::MiMalloc;
use vpay_config::{LogFormat, ServerArgs, ShutdownSignals};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Outcome of [`serve_with_bounded_drain`]: did in-flight work finish inside
/// the grace period, or did the process stop waiting for it?
enum DrainOutcome {
    /// The shutdown signal fired and every in-flight request finished
    /// before the grace period elapsed (or no signal fired and the served
    /// future ended some other way — not reachable in normal operation).
    Clean,
    /// The shutdown signal fired but `shutdown_grace_seconds` elapsed
    /// before draining finished; the process stopped waiting.
    TimedOut,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = ServerArgs::parse();

    // Installed before anything else — including tracing init — so the
    // SIGTERM/SIGINT handlers are live for this process's entire lifetime,
    // not just once axum::serve's graceful-shutdown future is first polled.
    // See vpay_config::signal for the race this closes. A failure here is a
    // hard startup failure (see ShutdownSignals::install's docs for why),
    // deliberately not a logged warning that lets the process continue with
    // no graceful shutdown path at all.
    let shutdown_signals =
        ShutdownSignals::install().context("installing SIGINT/SIGTERM handlers")?;

    init_tracing(&args.common.log_filter, args.common.log_format);

    let registry = vpay_server::adapter_registry();
    tracing::info!(rails = ?registry, "provider adapters linked");
    // `profile` names a YAML config file, never a code path — see
    // vpay_config::cli and docs/adr/0003-yaml-configuration.md.
    tracing::info!(profile = %args.common.profile, "deployment profile (selects a config file only)");

    // `--database-url` / `DATABASE_URL` stays `Option<String>` at the clap
    // level (`vpay_config::CommonArgs`, out of scope for this change — see
    // docs/status.md) but is treated as required *here*: a payment server
    // that binds a listener and answers `/healthz` with no database behind
    // it would be lying about its own readiness (`/healthz` now runs a real
    // `SELECT 1` — see `vpay_api::router`), and this repo's own rule is
    // never to look more finished than it is. A missing value is a hard,
    // loud startup failure, not a silently DB-less scaffold mode.
    let database_url = args.common.database_url.as_deref().context(
        "--database-url / DATABASE_URL is required: vpay-server cannot serve traffic without \
         a database to open a pool against and migrate (see docs/status.md)",
    )?;

    // Connect and migrate *before* binding the listener: a server that binds
    // its port before proving the database is reachable and up to date would
    // start accepting connections it cannot actually serve correctly.
    let pool = vpay_db::connect(database_url)
        .await
        .context("connecting to Postgres")?;
    vpay_db::run_migrations(&pool)
        .await
        .context("running database migrations")?;
    tracing::info!("database connected and migrations applied");

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;

    tracing::warn!(
        "vpay-server is a scaffold: only /healthz and the database connection are implemented. \
         See docs/status.md"
    );
    tracing::info!(addr = %args.bind, "listening");

    let shutdown_grace = Duration::from_secs(args.common.shutdown_grace_seconds);
    match serve_with_bounded_drain(listener, pool, shutdown_grace, shutdown_signals).await? {
        DrainOutcome::Clean => {
            tracing::info!("graceful shutdown complete, exiting");
            Ok(())
        }
        DrainOutcome::TimedOut => {
            tracing::warn!(
                shutdown_grace_seconds = args.common.shutdown_grace_seconds,
                "shutdown grace period elapsed before in-flight requests finished draining; \
                 stopped waiting for them and exiting anyway"
            );
            // Exit non-zero rather than the clean path's implicit 0.
            //
            // Reasoning: a container orchestrator (docker compose, k8s)
            // already treats the container as "stopped" the moment this
            // process exits at all, whatever the code — it does not retry
            // or block shutdown on a non-zero exit here, so this changes
            // nothing about the orchestration outcome. But unlike the clean
            // path, this exit means real in-flight work was cut off rather
            // than finished, which is not "successful" from this process's
            // own point of view. A non-zero exit lets anything that *does*
            // watch this process's exit code — a supervisor, `docker inspect
            // --format '{{.State.ExitCode}}'`, a monitoring rule on
            // container restarts/exit codes — tell a forced cutoff apart
            // from a clean drain without having to parse logs. `1` rather
            // than a SIGKILL-style `128+n` encoding, since nothing signalled
            // this process; it chose to stop waiting on its own.
            std::process::exit(1);
        }
    }
}

/// Serves `listener` until the shutdown signal fires, then waits at most
/// `shutdown_grace` for in-flight connections to drain.
///
/// `axum::serve(..).with_graceful_shutdown(..)` waits indefinitely for
/// in-flight connections once its signal future resolves — there is no
/// built-in bound on that wait. To add one, the shutdown signal is observed
/// twice via a oneshot: axum's graceful-shutdown future uses it to start
/// draining, and a second consumer starts a `shutdown_grace`-long clock at
/// that same moment. Whichever finishes first — the drain, or the clock —
/// decides the [`DrainOutcome`].
async fn serve_with_bounded_drain(
    listener: tokio::net::TcpListener,
    pool: vpay_db::PgPool,
    shutdown_grace: Duration,
    mut shutdown_signals: ShutdownSignals,
) -> anyhow::Result<DrainOutcome> {
    let (drain_started_tx, drain_started_rx) = tokio::sync::oneshot::channel::<()>();

    // `axum::serve(..).with_graceful_shutdown(..)` returns a builder that
    // implements `IntoFuture`, not `Future` directly — `tokio::select!`
    // needs the latter, hence the explicit `.into_future()`.
    let serve_fut = axum::serve(listener, vpay_api::router(pool))
        .with_graceful_shutdown(async move {
            shutdown_signals.wait().await;
            // A closed receiver just means the grace-period clock below
            // already lost interest — the drain itself already won the race.
            let _ = drain_started_tx.send(());
        })
        .into_future();
    tokio::pin!(serve_fut);

    tokio::select! {
        result = &mut serve_fut => {
            result.context("server error")?;
            Ok(DrainOutcome::Clean)
        }
        () = grace_clock(drain_started_rx, shutdown_grace) => Ok(DrainOutcome::TimedOut),
    }
}

/// Waits for the shutdown signal to actually reach `with_graceful_shutdown`
/// (i.e. for draining to have started), then sleeps for `grace`.
///
/// If the sender is dropped without ever sending — which only happens if
/// the served future resolved some other way first, e.g. an accept error —
/// this never resolves. That is intentional: `tokio::select!` in
/// [`serve_with_bounded_drain`] drops this future the moment the served
/// future wins, so a server error can never be mistaken for a timed-out
/// drain.
async fn grace_clock(drain_started_rx: tokio::sync::oneshot::Receiver<()>, grace: Duration) {
    if drain_started_rx.await.is_ok() {
        tokio::time::sleep(grace).await;
    } else {
        std::future::pending::<()>().await;
    }
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
// installed once this future is first polled, which is exactly the startup
// race that type exists to close.

// `grace_clock` is the entire bounded-wait mechanism behind
// `serve_with_bounded_drain`, deliberately factored out as a pure function
// of a oneshot receiver and a `Duration` so it can be tested without a real
// HTTP server, socket, or in-flight request. That matters here specifically
// because there is no honest way to produce a genuinely slow in-flight
// request against the real router (see `backends/apps/vpay-server/tests/
// cli.rs` for the process-level integration test and why it stops at the
// clean-drain path): `/healthz` answers instantly, and adding a slow
// test-only route to `vpay-api` would put a test double in the shipping
// router, which `cargo xtask verify-no-mocks` forbids and AGENTS.md rules
// out outright. These tests instead prove the timing/race logic itself is
// correct in isolation.
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::grace_clock;

    #[tokio::test]
    async fn grace_clock_waits_at_least_the_full_grace_period_once_the_signal_fires() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        // Small enough to keep the test fast, large enough that a `sleep`
        // which was accidentally skipped (e.g. a bug that resolves
        // `grace_clock` as soon as the signal fires, ignoring `grace`
        // entirely) would fail the lower-bound assertion below rather than
        // race it.
        let grace = Duration::from_millis(200);

        let start = Instant::now();
        let clock = tokio::spawn(grace_clock(rx, grace));
        tx.send(()).expect("receiver still alive");
        clock.await.expect("grace_clock task should not panic");
        let elapsed = start.elapsed();

        assert!(
            elapsed >= grace,
            "grace_clock returned before the full grace period elapsed: {elapsed:?} < {grace:?}"
        );
    }

    #[tokio::test]
    async fn grace_clock_never_resolves_if_the_signal_never_fires() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        drop(tx); // Simulates the served future winning some other way.

        let result = tokio::time::timeout(
            Duration::from_millis(200),
            grace_clock(rx, Duration::from_secs(5)),
        )
        .await;

        assert!(
            result.is_err(),
            "grace_clock should never resolve when its sender is dropped without sending"
        );
    }
}
