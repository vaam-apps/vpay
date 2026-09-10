//! `vpay-server worker`: submit, poll, reconcile, deliver.
//!
//! **This file is `backends/apps/vpay-worker-bin/src/main.rs`, moved.** Issue
//! #77 (2026-09-07) folded that package into this one: two binaries built
//! from one `cargo` invocation, shipped as two `scratch` images and signed
//! twice, became one binary with a subcommand. Nothing the job loop *does*
//! changed in that move, and nothing in `vpay-worker` — the crate holding
//! every handler — was touched.
//!
//! What did change, and each of these was a duplicate whose stated
//! justification the merge removed rather than weakened:
//!
//! * `adapters()` / `adapter_codes()` are gone from here. They were a
//!   deliberate copy of `vpay_server::adapters` (Step 2's D6) because "the
//!   worker depending on the server to learn which rails exist would make the
//!   worker's capabilities a function of the API server's crate, and the two
//!   processes deploy independently". They no longer deploy independently:
//!   they are one image, and a divergence is now impossible rather than
//!   merely discouraged. This module calls `vpay_server::adapters`.
//! * `exit_code_for`, `install_recorder`, `install_crypto_provider`,
//!   `init_tracing` and `env_filter` are gone from here for the same reason —
//!   `main.rs` has one of each, and "the two binaries are free to diverge as
//!   they grow" (the old `exit_code_for`'s doc) is not a property one binary
//!   can have. `StartupError` merged into `main.rs`'s enum of the same name,
//!   gaining that binary's `UnusableConcurrency` variant.
//! * `start_observability` / `join_observability` are `main.rs`'s, taking a
//!   `SocketAddr` rather than a `&WorkerArgs`/`&ServerArgs`. The listener,
//!   the flag and the shutdown coupling are unchanged.
//!
//! What is deliberately unchanged: the boot order, every log line, every
//! `.context(..)` string that an operator greps for, the drain, the
//! `Drain::TimedOut` exit of 1, and the exit codes 69 and 78. `tests/cli.rs`'s
//! `worker` module is the same suite that used to live in
//! `vpay-worker-bin/tests/cli.rs`, case for case and name for name.
//!
//! It binds exactly one socket: the observability listener on
//! `--observability-bind` (`vpay_api::observability`), serving `/livez` and
//! `/metrics`. That is the only HTTP surface this mode has — it routes no
//! `/v1` and answers no merchant — and it exists because a Deployment needs a
//! liveness probe and because `vpay_jobs_oldest_claimable_age_seconds` is the
//! one number that says whether live charges are being driven at all.
//!
//! Webhook delivery runs here: this mode owns both halves of the outbox, the
//! `fan_out_events` drain and the `deliver_webhook` sends. Nothing served by
//! `vpay-server` with no subcommand delivers a webhook (`docs/status.md`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use metrics_exporter_prometheus::PrometheusHandle;
use vpay_config::{CommonArgs, ConfigError, ShutdownSignals, WorkerArgs};
use vpay_db::MAX_CONNECTIONS;
use vpay_provider::ProviderAdapter;
use vpay_worker::{Drain, EndpointRegistry, RecoveryPolicy};

use crate::StartupError;

/// Everything [`boot`] assembles for the job loop.
///
/// One struct rather than a tuple of six, because the loop's argument list is
/// already long enough that a positional mistake would compile.
struct Booted {
    repositories: Arc<dyn vpay_db::Repositories>,
    adapters: BTreeMap<String, Box<dyn ProviderAdapter>>,
    rails: BTreeMap<String, vpay_provider::ProviderConfig>,
    endpoints: EndpointRegistry,
    egress: vpay_worker::EgressPolicy,
    concurrency: usize,
}

/// The whole of `vpay-server worker`, as the ordered list of steps it is.
///
/// Every step is a named function and the order between them is the contract —
/// see
/// [docs/reference/vpay-config.md § the boot sequence](../../../../docs/reference/vpay-config.md#the-boot-sequence)
/// for why each one is where it is, and what a probe against a half-started
/// process gets.
///
/// `shutdown_signals` is taken by value rather than installed here because
/// `main`'s `run` installs it before anything else — including tracing init,
/// including the parse of this subcommand's own flags — so the SIGTERM
/// handler is live for the process's entire lifetime rather than from the
/// first poll of a future constructed at this depth. See `vpay_config::signal`
/// for the race that closes, and `sigterm_immediately_after_startup_still_triggers_graceful_shutdown`
/// in `tests/cli.rs` for what reintroducing it looks like.
///
/// # Errors
///
/// Everything [`boot`] fails with, plus a bind failure on
/// `--observability-bind`. Each carries a typed leaf so `main`'s
/// `exit_code_for` can tell `78` from `69`.
pub(crate) async fn run(
    common: &CommonArgs,
    args: &WorkerArgs,
    mut shutdown_signals: ShutdownSignals,
    metrics: PrometheusHandle,
) -> anyhow::Result<()> {
    let booted = boot(common, args).await?;

    // The documented numbers (`docs/flows/{crash-safety,reconciler}.md`),
    // constructed here and passed down rather than read from a
    // `#[cfg(test)]` seam — AGENTS.md rule 1. The integration suite overrides
    // this same struct, so it exercises the identical code path a deployment
    // runs.
    let policy = RecoveryPolicy::default();
    let grace = Duration::from_secs(common.shutdown_grace_seconds);
    let worker_id = vpay_worker::worker_id();

    let (observability, observability_shutdown_tx) =
        crate::start_observability(common.observability_bind, metrics).await?;

    let report = vpay_worker::run_loop(
        booted.repositories,
        Arc::new(booted.adapters),
        Arc::new(booted.rails),
        policy,
        Arc::new(booted.endpoints),
        booted.egress,
        booted.concurrency,
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
            crate::join_observability(observability, grace).await;
            tracing::info!("graceful shutdown complete, exiting");
            Ok(())
        }
        Drain::TimedOut => {
            // Exit non-zero, for the reason the serve path's twin of this
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
                shutdown_grace_seconds = common.shutdown_grace_seconds,
                released = report.released,
                "the shutdown grace period elapsed before in-flight jobs finished; their \
                 leases were handed back and the process is exiting anyway"
            );
            std::process::exit(1);
        }
    }
}

/// Boot steps 1-4 plus everything the job loop needs to be handed.
///
/// Everything here that the serve path also does is [`vpay_api::boot`], one
/// implementation, because both write the same two tables in the same database
/// and a divergence there would be silent. What is left is what only *this*
/// mode does: the rail projection the loop polls with, and the webhook
/// endpoints and egress policy it delivers with.
///
/// Nothing here binds a socket, which is the entire definition of `/livez`: a
/// probe against a process still inside this function, or one about to exit
/// `78` for a `--worker-concurrency` of 0, gets a connection refusal rather
/// than a cheerful 200.
///
/// The YAML and `--worker-concurrency` are validated before the database is
/// touched, deliberately: a knob set to a value this process cannot use should
/// fail in milliseconds, not after paying for a Postgres connection and a
/// migration run it is about to discard.
///
/// # Errors
///
/// A missing flag, an invalid YAML file, a rail with no linked adapter, a
/// concurrency of zero, an unreachable database, a migration that will not
/// apply, or a reconcile that cannot take its lock. Each carries a typed leaf
/// so `exit_code_for` can tell `78` from `69`.
async fn boot(common: &CommonArgs, args: &WorkerArgs) -> anyhow::Result<Booted> {
    let config = vpay_api::boot::load_config(common.config.as_deref(), &common.profile)
        .context("loading and validating configuration (--config / VPAY_CONFIG, ADR-0003)")?;

    let concurrency = args
        .concurrency()
        .map_err(StartupError::UnusableConcurrency)?;
    tracing::info!(concurrency, "job loop concurrency");

    // Issue #63: a fan-out on the Existing branch holds two connections, so
    // the maximum safe concurrency is MAX_CONNECTIONS / 2. At the default
    // concurrency of 4 with MAX_CONNECTIONS=10 it fits; at 10 it would not,
    // and crash recovery would queue on ACQUIRE_TIMEOUT. Refuse it here, not
    // at the first crash.
    let pool_max = u32::from(MAX_CONNECTIONS) as usize;
    let max_safe = pool_max / 2;
    if concurrency > max_safe {
        return Err(ConfigError::WorkerConcurrencyExceedsPoolSize {
            concurrency,
            pool_max,
            max_safe,
        }
        .into());
    }

    // Boot step 4's inputs, before the database is touched — over the same
    // `vpay_server::adapters` the serve path joins against. Both modes
    // reconcile, so both have to agree about which rails exist and what each
    // one's row should say; before issue #77 this list was a second copy in a
    // second package, and a worker with its own copy of the derivation could
    // write a row the server would immediately overwrite.
    tracing::info!(rails = ?vpay_server::adapter_codes(), "provider adapters linked");
    // One client per process, shared by every adapter — see `main.rs`'s
    // `boot` for the full reasoning (a second client is a second connection
    // pool, and construction is fallible so it belongs here). The durations
    // are the port's defaults; a rail's own budget rides on `ProviderConfig`
    // and an adapter applies it per request.
    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .context("building the outbound HTTP client every rail adapter shares")?;
    let adapters = vpay_api::boot::adapters_by_code(vpay_server::adapters(http));
    let (currency_seeds, provider_seeds) = vpay_api::boot::boot_seeds(&config, &adapters)?;

    // `--database-url` / `DATABASE_URL` stays `Option<String>` at the clap
    // level and is required here — see docs/reference/vpay-config.md
    // § optional flags that are required in practice.
    let database_url = common
        .database_url
        .as_deref()
        .ok_or(StartupError::MissingDatabaseUrl)?;
    let repositories = vpay_api::boot::open_migrated_database(database_url).await?;

    vpay_api::boot::reconcile_reference_tables(
        repositories.as_ref(),
        &currency_seeds,
        &provider_seeds,
    )
    .await
    .context("reconciling currencies and providers from configuration (boot step 4)")?;

    let resource_config = vpay_api::ResourceConfig::from_config(&config)
        .context("projecting the deployment configuration onto the provider port")?;
    let rails = project_rails(&resource_config, &adapters);
    let endpoints = project_endpoints(&resource_config);

    // **No second client is built here, and that is the Step 8 change.**
    // Webhook delivery used to share one `reqwest::Client` built at this
    // point; it cannot, because each delivery's client is pinned to the
    // addresses its own endpoint resolved to (`vpay_worker::ssrf`,
    // `vpay_provider::http::client_pinned_to`) and a pin is a property of the
    // builder. The two budgets are unchanged and still
    // `vpay_worker::webhooks`' own constants; what is passed down instead is
    // the *policy*.
    //
    // Projected out of YAML exactly as the endpoint table above is: a handler
    // must never read a config document's shape (ADR-0003 —
    // `vpay_worker::WebhookContext`). The refusal of `livemode: true` with
    // `allow_private_targets: true` has already happened, in
    // `Config::validate_all`, before this line runs.
    let egress = vpay_worker::EgressPolicy {
        allow_private_targets: config.webhooks.allow_private_targets,
    };
    tracing::info!(
        allow_private_targets = egress.allow_private_targets,
        livemode = config.deployment.livemode,
        "webhook egress policy loaded"
    );

    Ok(Booted {
        repositories,
        adapters,
        rails,
        endpoints,
        egress,
        concurrency,
    })
}

/// Each rail's `ProviderConfig`, keyed the same way the adapters are.
///
/// Projected through `vpay_api::ResourceConfig` — the *same* projection the
/// serve path hands its router — rather than read out of the `Config`, so the
/// host, credentials, timeouts and callback URL a worker polls with are
/// byte-identical to the ones the server submitted with. A second derivation
/// would be a rail that can be charged and not queried.
///
/// Only the rails this binary actually links are kept: a `providers:` entry
/// with no adapter is a configuration error `boot_seeds` has already refused,
/// so this filter drops nothing in a booting deployment and keeps the map's
/// meaning exact ("what this process can talk to").
fn project_rails(
    resource_config: &vpay_api::ResourceConfig,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) -> BTreeMap<String, vpay_provider::ProviderConfig> {
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
    rails
}

/// Every merchant's webhook endpoints, keyed on `events.merchant_id` — which
/// is the fan-out key, and deliberately *not* `client_id`.
///
/// This is the one place `vpay_api::WebhookEndpointConfig` and
/// `vpay_worker::Endpoint` meet, and it has to be in a binary: `vpay-worker`
/// already depends on `vpay-api` to render the delivered body, so the reverse
/// edge that would let either crate do this conversion itself is a cycle. See
/// `WebhookEndpointConfig`'s own doc comment.
fn project_endpoints(resource_config: &vpay_api::ResourceConfig) -> EndpointRegistry {
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
    endpoints
}
