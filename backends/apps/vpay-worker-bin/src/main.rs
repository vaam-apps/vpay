//! vpay worker: submit, poll, reconcile, deliver.
//!
//! Boots exactly as `vpay-server` does — signal handlers, crypto provider,
//! metrics recorder, tracing, YAML configuration, pool, migrations, boot
//! step 4 — and then runs [`vpay_worker::run_loop()`] instead of binding a
//! traffic listener. Everything the loop does is in that module; this file's
//! job is to assemble its inputs (the pool, the adapters, each rail's
//! configuration, the recovery policy, the merchant webhook endpoints and
//! the client that delivers to them) and to bound its drain.
//!
//! It does now bind **one** socket, which it did not before Step 6: the
//! observability listener on `--observability-bind`
//! (`vpay_api::observability`), serving `/livez` and `/metrics`. That is the
//! only HTTP surface this process has — it routes no `/v1` and answers no
//! merchant — and it exists because a Deployment needs a liveness probe and
//! because `vpay_jobs_oldest_claimable_age_seconds` is the one number that
//! says whether live charges are being driven at all.
//!
//! Webhook delivery runs here: this process owns both halves of the outbox,
//! the `fan_out_events` drain and the `deliver_webhook` sends. Nothing in
//! `vpay-server` delivers a webhook (`docs/status.md`).
//!
//! # Why this process links `vpay-api`
//!
//! For `vpay_api::v1::boot` and `vpay_api::ResourceConfig`, and nothing else
//! — no router is mounted here. Both binaries must reconcile the same
//! `providers`/`currencies` rows from the same YAML and derive each rail's
//! `ProviderConfig` the same way; a worker with its own copy of that
//! derivation could poll a rail at a host the server never charges. The
//! alternative (moving the projection into `vpay-config`) was considered and
//! declined in the Step 4 design: the edge already exists, and
//! `ResourceConfig::from_config`'s own doc comment says both binaries
//! building it identically is the point.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use mimalloc::MiMalloc;
use vpay_config::{ConfigError, LogFormat, ShutdownSignals, WorkerArgs};
use vpay_core::error::{Category, Classify as _, find_in_chain};
use vpay_db::DbError;
use vpay_provider::ProviderAdapter;
use vpay_worker::{Drain, EndpointRegistry, RecoveryPolicy};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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
        .or_else(|| find_in_chain::<StartupError>(error.chain()).map(|e| e.category()))
        .or_else(|| find_in_chain::<DbError>(error.chain()).map(|e| e.category()))
        .unwrap_or(Category::Internal);
    // Every `Category::exit_code()` is in `1..=78` (pinned by a test in
    // `vpay_core::error`), so this conversion cannot fail in practice; `1`
    // is the same honest fallback as an unclassified error, rather than a
    // truncating cast.
    u8::try_from(category.exit_code()).unwrap_or(1)
}

/// A startup input this binary was given but cannot use.
///
/// It exists so "you set the knob to a value that means nothing" reaches an
/// operator as exit `78` ("fix the deploy") rather than `1` ("this is a vpay
/// bug"). Without a typed leaf in the chain, an `anyhow!("…")` gives
/// [`exit_code_for`] nothing to classify and the honest-but-unhelpful
/// [`Category::Internal`] fallback applies.
///
/// The twin of `vpay-server`'s `StartupError`, and defined in the binary for
/// the same reason: which inputs a process requires, and which values it can
/// make sense of, are properties of *that process*. `vpay-server` takes no
/// `--worker-concurrency` at all (it claims no jobs), so a `ConfigError`
/// variant about one would be a requirement one binary has, spelled in a
/// crate both link.
#[derive(Debug, thiserror::Error)]
enum StartupError {
    /// `--worker-concurrency` / `VPAY_WORKER_CONCURRENCY` was zero. The
    /// message is `vpay_config::WorkerArgs::concurrency`'s, which names both
    /// spellings, because the message is the entire fix.
    #[error("{0}")]
    UnusableConcurrency(String),
}

impl vpay_core::error::Classify for StartupError {
    /// A deploy that must be fixed — never retried, never the caller's fault.
    /// [`Category::Configuration`] is what makes that exit `78`, the same
    /// number a malformed YAML file produces, because it is the same kind of
    /// operator problem.
    fn category(&self) -> Category {
        Category::Configuration
    }
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

    // Beside `install_crypto_provider`, and before `init_tracing`, for the
    // same reason: a process-wide default that everything after it may
    // depend on. A metric recorded before this call goes nowhere.
    let metrics = install_recorder().context("installing the Prometheus metrics recorder")?;

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

    // Validated here, beside the YAML and *before* the database, for the same
    // reason the YAML is loaded before the database: a knob set to a value
    // this process cannot use should fail in milliseconds, not after paying
    // for a Postgres connection and a migration run it is about to discard.
    let concurrency = args
        .concurrency()
        .map_err(StartupError::UnusableConcurrency)?;
    tracing::info!(concurrency, "job loop concurrency");

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

    // Each rail's `ProviderConfig`, keyed the same way the adapters are.
    //
    // Projected through `vpay_api::ResourceConfig` — the *same* projection
    // `vpay-server` hands its router — rather than read out of the `Config`
    // here, so the host, credentials, timeouts and callback URL a worker
    // polls with are byte-identical to the ones the server submitted with. A
    // second derivation would be a rail that can be charged and not queried.
    //
    // Only the rails this binary actually links are kept: a `providers:`
    // entry with no adapter is a configuration error `boot_seeds` above has
    // already refused, so this filter drops nothing in a booting deployment
    // and keeps the map's meaning exact ("what this process can talk to").
    let resource_config = vpay_api::ResourceConfig::from_config(&config)
        .context("projecting the deployment configuration onto the provider port")?;
    let rails: BTreeMap<String, vpay_provider::ProviderConfig> = adapters
        .keys()
        .filter_map(|code| {
            resource_config
                .rail(code)
                .map(|rail| (code.clone(), rail.provider_config()))
        })
        .collect();
    tracing::info!(
        rails = rails.len(),
        "rail configurations projected for the job loop"
    );

    // Every merchant's webhook endpoints, keyed on `events.merchant_id` —
    // which is the fan-out key, and deliberately *not* `client_id`: one
    // merchant may hold several credentials and all of them describe the same
    // tenant, so keying on the credential would deliver an event once per key
    // a merchant happens to have (`docs/plans/2026-09-03-step5-webhooks.md`,
    // S5).
    //
    // This is the one place `vpay_api::WebhookEndpointConfig` and
    // `vpay_worker::webhooks::Endpoint` meet, and it has to be a binary:
    // `vpay-worker` already depends on `vpay-api` to render the delivered
    // body, so the reverse edge that would let either crate do this
    // conversion itself is a cycle. See `WebhookEndpointConfig`'s own doc
    // comment.
    let endpoints = EndpointRegistry::from_pairs(resource_config.webhook_endpoints().map(
        |(merchant_id, endpoints)| {
            (
                merchant_id.to_owned(),
                endpoints
                    .iter()
                    .map(|endpoint| vpay_worker::Endpoint {
                        id: endpoint.id().to_owned(),
                        url: endpoint.url().to_owned(),
                        secrets: endpoint.secrets().to_vec(),
                    })
                    .collect(),
            )
        },
    ));
    tracing::info!(
        merchants_with_endpoints = resource_config.webhook_endpoints().count(),
        endpoints = resource_config
            .webhook_endpoints()
            .map(|(_, endpoints)| endpoints.len())
            .sum::<usize>(),
        "webhook endpoints projected for the fan-out"
    );

    // A *second* client, not the rails' one, and the difference is the
    // timeouts. A rail's budget is the port's default and rides on
    // `ProviderConfig`; a merchant's receiver is somebody else's server that
    // this process must not be held open by, so it gets a short connect and a
    // bounded read. The two budgets are
    // `vpay_worker::webhooks`' own constants and not this binary's: the
    // handler that spends them is there, and the integration suite reads the
    // same pair, so a change cannot leave the tests exercising a client that
    // no longer ships. Everything
    // else is `client_with_timeouts`'s doing and is what makes it safe to
    // POST a signed body at an operator-configured URL: vendored roots,
    // `redirect::Policy::none()` so a receiver cannot bounce the delivery at
    // a host nobody configured, and `no_proxy()`.
    //
    // Built once and cloned into every task — `reqwest::Client` is an `Arc`
    // internally, so the clones share one connection pool.
    let webhook_http = vpay_provider::http::client_with_timeouts(
        vpay_worker::WEBHOOK_CONNECT_TIMEOUT,
        vpay_worker::WEBHOOK_REQUEST_TIMEOUT,
    )
    .context("building the outbound HTTP client webhook delivery uses")?;

    // The documented numbers (`docs/flows/{crash-safety,reconciler}.md`),
    // constructed here and passed down rather than read from a
    // `#[cfg(test)]` seam — AGENTS.md rule 1. The integration suite overrides
    // this same struct, so it exercises the identical code path a deployment
    // runs.
    let policy = RecoveryPolicy::default();
    let grace = Duration::from_secs(args.common.shutdown_grace_seconds);
    let worker_id = vpay_worker::worker_id();

    // **The only socket this process opens.** Before it existed, the worker
    // had no listener at all: no liveness probe a Deployment could use, and
    // no way to export the queue-depth gauge that is the one number saying
    // whether live charges are being driven. `vpay-server`'s `main.rs` binds
    // the identical listener on the identical flag.
    //
    // Bound *last*, after the config, the database, the migrations, boot
    // step 4 and the rail projection, because that ordering is the whole
    // definition of `/livez`: a worker that is still starting, or one about
    // to exit 78 for a `--worker-concurrency` of 0, refuses the connection
    // rather than answering `ok`.
    let observability_listener = tokio::net::TcpListener::bind(args.common.observability_bind)
        .await
        .with_context(|| {
            format!(
                "binding the observability listener on {} (--observability-bind / \
                 VPAY_OBSERVABILITY_BIND)",
                args.common.observability_bind
            )
        })?;
    let observability_bound = observability_listener
        .local_addr()
        .context("reading the bound address back off the observability listener")?;
    // `--observability-bind 127.0.0.1:0` is a real configuration (the
    // subprocess tests use it) and with a `:0` port nothing else can know
    // the answer.
    tracing::info!(
        addr = %observability_bound,
        "observability listener listening (/livez, /metrics)"
    );

    // One signal, two consumers: `run_loop`'s drain and this listener. A
    // detached task with no shutdown of its own would keep answering
    // `/livez` with `ok` while the drain was already cutting jobs off.
    let (observability_shutdown_tx, observability_shutdown_rx) =
        tokio::sync::oneshot::channel::<()>();
    let observability = tokio::spawn(vpay_api::observability::serve(
        observability_listener,
        move || metrics.render(),
        async move {
            let _ = observability_shutdown_rx.await;
        },
    ));

    let report = vpay_worker::run_loop(
        &pool,
        Arc::new(adapters),
        Arc::new(rails),
        policy,
        Arc::new(endpoints),
        webhook_http,
        concurrency,
        grace,
        worker_id,
        async move {
            shutdown_signals.wait().await;
            // A closed receiver just means the listener already stopped.
            let _ = observability_shutdown_tx.send(());
        },
    )
    .await;

    match report.drain {
        Drain::Clean => {
            join_observability(observability, grace).await;
            tracing::info!("graceful shutdown complete, exiting");
            Ok(())
        }
        Drain::TimedOut => {
            // Exit non-zero, for the reason `vpay-server`'s twin of this
            // branch gives at length: an orchestrator treats the container as
            // stopped either way, but a supervisor or a `docker inspect` that
            // *does* read the code should be able to tell "in-flight work was
            // cut off" from "everything finished" without parsing logs.
            //
            // The worker's version of "cut off" is materially worse than the
            // server's, which is why it is worth the number: a job aborted
            // mid-flight has already had its `attempts` incremented and may
            // have called a rail. Its lease has been handed back
            // (`report.released`) so another worker re-runs it at once, and
            // every handler is a compare-and-swap so the re-run is a no-op if
            // the first pass committed — but repeated timeouts here mean the
            // grace period is below what a poll actually takes.
            tracing::warn!(
                shutdown_grace_seconds = args.common.shutdown_grace_seconds,
                released = report.released,
                "the shutdown grace period elapsed before in-flight jobs finished; their \
                 leases were handed back and the process is exiting anyway"
            );
            std::process::exit(1);
        }
    }
}

/// Waits for the observability listener to stop, bounded by the same grace
/// period the job drain uses.
///
/// Called only on the clean path — the timed-out path calls
/// `std::process::exit(1)` and takes this task with it, which is correct
/// there: a worker that has already cut jobs off mid-flight must not then
/// wait on a metrics socket.
///
/// Failures are logged, never propagated: the observability port is not a
/// payment path, and letting it change this binary's exit code would make
/// the forced-cutoff `1` (see [`Drain::TimedOut`]) ambiguous.
///
/// A near-copy of `vpay-server`'s function of the same name, for the same
/// reason `exit_code_for` and `init_tracing` are copies.
async fn join_observability(
    observability: tokio::task::JoinHandle<std::io::Result<()>>,
    grace: Duration,
) {
    match tokio::time::timeout(grace, observability).await {
        Ok(Ok(Ok(()))) => tracing::debug!("observability listener stopped"),
        Ok(Ok(Err(error))) => {
            tracing::warn!(%error, "the observability listener ended with an error");
        }
        Ok(Err(error)) => tracing::warn!(%error, "the observability listener task failed"),
        Err(_) => tracing::warn!(
            grace_seconds = grace.as_secs(),
            "the observability listener did not stop inside the grace period; exiting anyway"
        ),
    }
}

/// Installs this process's Prometheus recorder, describes every metric name
/// and stamps `vpay_build_info`.
///
/// A near-copy of `vpay-server`'s function of the same name — see that one
/// for the full reasoning, including where `vpay_build_info`'s `git_sha`
/// comes from and why it reads `unknown` on every build nobody passed
/// `VPAY_GIT_SHA` to.
///
/// What this binary does with the recorder that the server does not:
/// `vpay_worker::run_loop` emits `vpay_jobs_claimed_total`,
/// `vpay_jobs_completed_total` and `vpay_jobs_oldest_claimable_age_seconds`
/// into it. Those macros are no-ops until this call has run, which is why it
/// sits at the top of `run` and not beside the loop.
///
/// # Errors
///
/// Only if a recorder was already installed, which cannot happen here.
/// Reported rather than ignored because the alternative is a worker whose
/// `/metrics` renders empty forever while looking healthy — and the queue
/// gauge is the one number that says whether live charges are being driven.
fn install_recorder() -> anyhow::Result<PrometheusHandle> {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    if metrics::set_global_recorder(recorder).is_err() {
        anyhow::bail!(
            "a metrics recorder was already installed in this process; vpay-worker-bin installs \
             exactly one, so this is a bug rather than a deployment problem"
        );
    }
    vpay_core::metrics::describe_all();
    vpay_core::metrics::record_build_info(env!("CARGO_PKG_VERSION"));
    Ok(handle)
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
/// `reqwest::Client` is built", and this process builds one on every start:
/// [`adapters`] hands `vpay_provider::http`'s client to both rail adapters,
/// and the job loop then calls those rails over HTTPS on each poll that
/// reaches the rail and on every `resubmit_charge`. ("Each poll that reaches
/// the rail" and not "every `poll_charge`": a poll of an already-terminal
/// charge, one the recovery table answers `FailDeadOrder` to, and one it
/// answers `Resubmit` to all return before the status query.) Without this
/// line that construction is the
/// panic described above, on the startup path of a shipping binary. Right
/// after the signal handlers and above `init_tracing` puts it ahead of
/// [`adapters`] with room to spare, so no future edit can slip a client
/// construction in front of it either.
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
