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

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use mimalloc::MiMalloc;
use vpay_config::{ConfigError, LogFormat, ShutdownSignals, WorkerArgs};
use vpay_core::error::{Category, Classify as _, find_in_chain};
use vpay_db::DbError;
use vpay_provider::ProviderAdapter;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// How often the "not implemented" banner repeats while the process is up.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Runs [`run`] and turns its failure into a *classified* exit code.
///
/// Synchronous, returning [`ExitCode`] rather than `anyhow::Result<()>`,
/// for the same reason as `vpay-server`'s `main` (see that binary): the
/// `Termination` impl for `Result` always exits `1`, which is precisely the
/// "a supervisor cannot tell 'fix the YAML' from 'Postgres is down'" problem
/// ADR-0011 exists to fix. Both binaries must answer identically, since an
/// operator debugging a compose stack reads both exit codes the same way.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `eprintln!`, not `tracing::error!`: a missing or invalid
            // `--config` fails before `init_tracing` has installed a
            // subscriber, so a `tracing` event would be dropped and the
            // process would exit with a bare number and no explanation.
            // `{error:#}` renders the full `anyhow` context chain inline so
            // the `.context(..)` calls in `run` actually reach an operator.
            eprintln!("vpay-worker-bin: {error:#}");
            ExitCode::from(exit_code_for(&error))
        }
    }
}

/// The exit code for a failed startup, per ADR-0011's Tier 3 and the table in
/// `docs/flows/errors.md`. Deliberately a near-copy of `vpay-server`'s
/// function of the same name rather than a shared helper. A shared version
/// has to name both leaf types it looks for, so it needs a home that depends
/// on `vpay-config` **and** `vpay-db` — and that home would exist only to
/// hold this function. "The set of leaf errors a binary knows how to
/// classify" is not a library boundary this workspace wants: it is a
/// property of the binary, and the two binaries are free to diverge as they
/// grow (this one will learn about job-queue failures the server never
/// meets). Two copies of eight lines, each pinned by its own CLI tests, cost
/// less than a crate that exists to avoid them. ADR-0011 also keeps `anyhow`
/// out of every library crate's `[dependencies]`, so the shared helper could
/// not take an `&anyhow::Error` anyway.
///
/// The order is load-bearing, not alphabetical: `ConfigError` is looked for
/// **before** `DbError`, so a chain carrying both (a config that names an
/// unreachable database) reports the operator's real problem — `78` ("fix the
/// deploy") rather than `69` ("wait for Postgres"). `find_in_chain` is typed,
/// so this is also the exhaustive list of leaf errors this binary knows how
/// to classify; anything else falls through to [`Category::Internal`], exit
/// `1`. That fallback is the pessimistic one on purpose: an unclassified
/// startup failure in a payment binary should look like a bug.
fn exit_code_for(error: &anyhow::Error) -> u8 {
    let category = find_in_chain::<ConfigError>(error.chain())
        .map(|e| e.category())
        .or_else(|| find_in_chain::<DbError>(error.chain()).map(|e| e.category()))
        .unwrap_or(Category::Internal);
    // Every `Category::exit_code()` is in `1..=78` (pinned by a test in
    // `vpay_core::error`), so this conversion cannot fail in practice; `1`
    // is the same honest fallback as an unclassified error, rather than a
    // truncating cast.
    u8::try_from(category.exit_code()).unwrap_or(1)
}

/// Every adapter linked into this binary.
///
/// **A deliberate duplicate of `vpay_server::adapters`** (Step 2's D6), not
/// an import of it: `vpay-worker-bin` depending on `vpay-server` to learn
/// which rails exist would make the worker's capabilities a function of the
/// API server's crate, and the two processes deploy independently. The list
/// is four lines and each binary's own; the thing that must not diverge is
/// the *port*, and that is `vpay-provider`, which both link.
///
/// This is now the *only* thing duplicated between the two binaries' boot
/// paths. Everything downstream of the list — keying it by code, joining it
/// against the YAML, deriving each rail's row — is
/// `vpay_api::v1::boot`, one implementation both call, because both write
/// the same `providers` table and a divergence there would be silent.
///
/// Note what is absent here too: no stub, no fake. A stub rail is a WireMock
/// host in configuration (`docs/adr/0006-no-mocks-in-main-processes.md`).
fn adapters(http: reqwest::Client) -> Vec<Box<dyn ProviderAdapter>> {
    vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new(http.clone())),
        Box::new(vpay_adapter_orange_money::Adapter::new(http)),
    ]
}

/// The rail codes this binary links, for the boot log line — the twin of
/// `vpay_server::adapter_codes`, duplicated for the same reason
/// [`adapters`] is.
///
/// A `const` list rather than "construct every adapter and ask it": a log
/// line must not need an HTTP client. `the_codes_match_the_adapters_that_are_linked`
/// below keeps the two from drifting.
const fn adapter_codes() -> [&'static str; 2] {
    ["mtn_momo", "orange_money"]
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
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

    install_crypto_provider();

    init_tracing(&args.common.log_filter, args.common.log_format);

    // `profile` names a YAML config file, never a code path — see
    // vpay_config::cli and docs/adr/0003-yaml-configuration.md.
    tracing::info!(profile = %args.common.profile, "deployment profile (selects a config file only)");

    // Load and validate the YAML deployment configuration (ADR-0003) before
    // touching the database — same ordering and reasoning as
    // `vpay-server`'s `main.rs`: boot-sequence steps 1-3 in
    // docs/flows/configuration.md (load, resolve `${ENV}`, validate) come
    // before step 4 (DB reconciliation, not yet implemented), and a broken
    // local YAML file should fail in milliseconds rather than after this
    // process has already paid for a Postgres connection and migration run
    // it is about to discard. `--config` / `VPAY_CONFIG` stays
    // `Option<PathBuf>` at the clap level for the same reason
    // `--database-url` does below, but is required here.
    let config = vpay_config::Config::load(args.common.config.as_deref(), &args.common.profile)
        .context("loading and validating configuration (--config / VPAY_CONFIG, ADR-0003)")?;
    // Redaction-safe by construction — see `vpay-server`'s identical log
    // line for why this logs discrete fields rather than the `Config`
    // struct itself.
    tracing::info!(
        deployment = %config.deployment.name,
        livemode = config.deployment.livemode,
        providers = config.providers.len(),
        merchant_clients = config.merchant_clients.len(),
        dashboard_client_configured = config.dashboard_client.is_some(),
        "configuration loaded and validated"
    );

    // Boot step 4's inputs, before the database is touched — see
    // `vpay_api::v1::boot::boot_seeds`, which is the *same function*
    // `vpay-server` calls, over this binary's own `adapters()`. Both
    // binaries reconcile, so both have to agree about which rails exist and
    // what each one's row should say; a worker that skipped this could be
    // started against a config the server would have refused, and a worker
    // with its own copy of the derivation could write a row the server would
    // immediately overwrite.
    tracing::info!(rails = ?adapter_codes(), "provider adapters linked");
    // One client per process, shared by every adapter — see `vpay-server`'s
    // `main.rs` for the full reasoning (a second client is a second
    // connection pool, and construction is fallible so it belongs in `main`).
    // The durations are the port's defaults; a rail's own budget rides on
    // `ProviderConfig` and an adapter applies it per request.
    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .context("building the outbound HTTP client every rail adapter shares")?;
    let adapters = vpay_api::v1::boot::adapters_by_code(adapters(http));
    let (currency_seeds, provider_seeds) = vpay_api::v1::boot::boot_seeds(&config, &adapters)?;

    // `--database-url` / `DATABASE_URL` stays `Option<String>` at the clap
    // level (`vpay_config::CommonArgs`) but is treated as required *here*,
    // matching `vpay-server`'s decision (see that binary's `main.rs` for the
    // fuller reasoning): a process that boots with no database at all is not
    // doing the thing this pass exists to do, and silently skipping the
    // connection when the flag is absent would leave it dormant behind an
    // optional flag rather than the live path this repo's delivery style
    // requires.
    let database_url = args.common.database_url.as_deref().context(
        "--database-url / DATABASE_URL is required: vpay-worker-bin cannot start without \
         a database to open a pool against and migrate (see docs/status.md)",
    )?;

    // Connect and migrate at startup, same as `vpay-server` — "database
    // connectivity and migrations at boot" applies to both binaries, not
    // just the one with an HTTP listener. The pool's only consumer is boot
    // step 4 below (the job loop is not implemented, docs/status.md), so it
    // is dropped once that has run; the goal here is the loud, fail-fast
    // proof that the database is reachable, migrated and agreeing with this
    // deployment's configuration before this process claims to be running.
    let pool = vpay_db::connect(database_url)
        .await
        .context("connecting to Postgres")?;
    vpay_db::run_migrations(&pool)
        .await
        .context("running database migrations")?;
    tracing::info!("database connected and migrations applied");

    // Boot step 4 (`docs/flows/configuration.md`), the same call
    // `vpay-server` makes and in the same position: after the migrations, in
    // one transaction.
    //
    // Two things make it safe to run this from both binaries rather than
    // nominating one of them as the writer, and neither is "idempotence".
    // Idempotence covers *repeating* a reconcile, which is not what happens
    // during a rollout — there, two of them *overlap*:
    //
    // * they cannot interleave, because `reconcile`'s transaction opens by
    //   taking `vpay_db::lock_keys::CONFIG_RECONCILE` (proven taken by
    //   `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`);
    // * they cannot disagree about *what* to write, because the seeds come
    //   from one shared derivation — `vpay_api::v1::boot::boot_seeds`, the
    //   same function `vpay-server` calls, over this binary's own linked
    //   rails.
    //
    // What is still true and worth stating plainly: two processes configured
    // with *different* YAML will each write their own view, last commit
    // winning. Nothing here detects that, and nothing should — the lock
    // makes the outcome one of the two inputs rather than a mixture of both.
    vpay_db::config_reconcile::reconcile(&pool, &currency_seeds, &provider_seeds)
        .await
        .context("reconciling currencies and providers from configuration (boot step 4)")?;
    tracing::info!(
        currencies = currency_seeds.len(),
        providers = provider_seeds.len(),
        enabled_providers = provider_seeds.iter().filter(|seed| seed.enabled).count(),
        "reference tables reconciled from configuration"
    );

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

/// Installs the process-wide rustls [`CryptoProvider`] this binary's HTTP
/// clients need, before anything can build one.
///
/// [`CryptoProvider`]: rustls::crypto::CryptoProvider
///
/// **Why it exists at all.** The root `Cargo.toml` pins `reqwest` with
/// `rustls-no-provider` (see the long comment on that pin, and the one on
/// the `authkestra-*` pins below it): the alternative selects `aws-lc-rs`,
/// which `deny.toml` bans outright because two providers in one process are
/// exactly what makes `install_default()` panic. The cost of picking nothing
/// is that reqwest 0.13's `ClientBuilder::build()` calls
/// `CryptoProvider::get_default()` and **panics** — "No rustls crypto
/// provider is configured" — when there is no process default. That is a
/// panic in a shipping payment binary, i.e. a defect under ADR-0007, on a
/// path no unit test reaches. `docs/status.md`'s "rustls `CryptoProvider`
/// process default" row tracks it as a documented landmine, and until this
/// call landed the workspace's only `install_default()` was a `#[cfg(test)]`
/// helper in `vpay_api::resource_auth`.
///
/// **Why here.** The one ordering constraint is "before the first
/// `reqwest::Client` is built". This process builds none today — the job
/// loop that will call rails over HTTPS is not implemented
/// (`docs/status.md`) — so this is a prerequisite put in place ahead of the
/// first caller, not a fix for a panic anyone has seen here. It is
/// nevertheless not speculative wiring: it is the same one-line startup
/// invariant `vpay-server` needs, and having the two binaries diverge on it
/// is how the worker would acquire the panic silently the day the job loop
/// lands. Right after the signal handlers and above `init_tracing` means no
/// future edit can slip a client construction in ahead of it.
///
/// **Why the result is dropped.** `install_default()` returns
/// `Err(Arc<CryptoProvider>)` for exactly one reason: a default was already
/// installed. In a binary that means some other code got there first, which
/// is the state this call wanted anyway — so `.ok()`, which is what the root
/// `Cargo.toml`'s own note on the `authkestra-*` pins recommends verbatim.
/// `unwrap`/`expect` are denied here (ADR-0007) and would turn a harmless
/// double install into a startup crash.
///
/// A byte-identical copy of `vpay-server`'s function of the same name, for
/// the same reason `exit_code_for` and `init_tracing` are copies.
fn install_crypto_provider() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
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

#[cfg(test)]
mod tests {
    use super::{adapter_codes, adapters, install_crypto_provider};

    /// The log line and the wiring must not drift — `adapter_codes` exists
    /// only so the boot log needs no HTTP client, and a hand-written list is
    /// one someone can forget to update. The twin of `vpay-server`'s test of
    /// the same name.
    #[test]
    fn the_codes_match_the_adapters_that_are_linked() {
        let http = vpay_provider::http::client().expect("the vendored-roots client builds");
        let linked: Vec<&str> = adapters(http).iter().map(|a| a.code()).collect();
        assert_eq!(linked, adapter_codes().to_vec());
    }

    /// The provider install is idempotent and leaves a process default
    /// behind.
    ///
    /// What this proves: after `install_crypto_provider()` returns,
    /// `CryptoProvider::get_default()` is `Some`, and calling it a second
    /// time does not panic — which is the whole contract the `.ok()` on the
    /// `Err(Arc<CryptoProvider>)` relies on.
    ///
    /// What it deliberately does **not** prove: that the `reqwest::Client`
    /// this binary builds at boot succeeds *because of* this call. It does
    /// not — `vpay_provider::http` hands reqwest a finished
    /// `rustls::ClientConfig`, which takes the branch that consults neither
    /// the process default nor the OS trust store (that module's own docs
    /// explain why, and its tests prove it). This call remains the guard for
    /// any *other* client construction in the graph, `authkestra-engine`'s
    /// bare `reqwest::Client::new()` foremost.
    #[test]
    fn installing_the_crypto_provider_leaves_a_process_default_and_is_idempotent() {
        install_crypto_provider();
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "no process-wide CryptoProvider after install_crypto_provider(); \
             reqwest would panic on its first client"
        );

        // The second call takes the `Err(_)` branch by construction. It must
        // stay silent rather than panic: in a real process this is what
        // happens when some other component installed one first.
        install_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
