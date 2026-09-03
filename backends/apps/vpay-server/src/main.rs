//! vpay API server.
//!
//! Writes rows and returns. It never calls a payment rail — that is the
//! worker's job, and it is what makes the system crash-safe.

use std::collections::BTreeMap;
use std::future::{Future, IntoFuture as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use mimalloc::MiMalloc;
use vpay_api::op::MerchantOp;
use vpay_api::op::keys::{LoadedSigningKey, SigningKeyError};
use vpay_api::resource_auth::{JwtValidator, MerchantJwtValidator, Surface};
use vpay_api::{ResourceConfig, RouterDeps};
use vpay_config::{ConfigError, LogFormat, ServerArgs, ShutdownSignals};
use vpay_core::error::{Category, Classify as _, find_in_chain};
use vpay_db::DbError;

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

/// Runs [`run`] and turns its failure into a *classified* exit code.
///
/// `main` is deliberately synchronous and returns [`ExitCode`] rather than
/// `anyhow::Result<()>`: the `Termination` impl for `Result` prints the error
/// with `Debug` and always exits `1`, which is exactly the "a supervisor
/// cannot tell 'fix the YAML' from 'Postgres is down'" problem ADR-0011 was
/// written to fix.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `eprintln!`, not `tracing::error!`: the earliest failures this
            // handles (a missing `--config`, a YAML file that does not
            // validate) happen *before* `init_tracing` has installed a
            // subscriber, so a `tracing` event would be dropped on the floor
            // and the process would exit silently with only a number. stderr
            // is the one sink that is guaranteed to exist at every point in
            // startup. `{error:#}` renders the whole `anyhow` context chain
            // on one line — "connecting to Postgres: failed to connect to
            // Postgres: ..." — so the `.context(..)` calls below actually
            // reach an operator.
            eprintln!("vpay-server: {error:#}");
            ExitCode::from(exit_code_for(&error))
        }
    }
}

/// A required startup input this binary was not given.
///
/// It exists so that "you forgot a flag" reaches an operator as exit `78` ("fix
/// the deploy") rather than `1` ("this is a vpay bug"): without a typed leaf in
/// the chain, [`exit_code_for`] has nothing to classify and the
/// honest-but-unhelpful [`Category::Internal`] fallback applies.
///
/// **Defined in the binary, not in `vpay-config`, deliberately** — which inputs
/// a process requires is a property of *that process*. See
/// [docs/reference/vpay-config.md § optional flags that are required in practice](../../../../docs/reference/vpay-config.md#optional-flags-that-are-required-in-practice).
#[derive(Debug, thiserror::Error)]
enum StartupError {
    /// `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE` was not
    /// supplied. Named in full, both spellings, because the message is the
    /// entire fix.
    #[error(
        "--oauth-signing-key-file / VPAY_OAUTH_SIGNING_KEY_FILE is required: vpay-server signs \
         every /v1 access token with it and cannot serve the merchant API without one (ADR-0010)"
    )]
    MissingSigningKeyFile,
}

impl vpay_core::error::Classify for StartupError {
    /// A deploy that must be fixed — never retried, never the caller's
    /// fault. [`Category::Configuration`] is what makes that exit `78`,
    /// the same number a malformed YAML file produces, because it is the
    /// same kind of operator problem.
    fn category(&self) -> Category {
        Category::Configuration
    }
}

/// The exit code for a failed startup, per ADR-0011's Tier 3 and the table in
/// `docs/flows/errors.md`.
///
/// The lookup order is load-bearing rather than alphabetical, `DbError` last
/// does not mean it always means `69`, and `vpay-worker-bin`'s near-copy is
/// deliberate rather than an oversight — all three are explained in
/// [docs/reference/vpay-config.md § exit codes](../../../../docs/reference/vpay-config.md#exit-codes).
///
/// `find_in_chain` is typed, so this function is also the exhaustive list of
/// leaf errors this binary knows how to classify; anything else falls through to
/// [`Category::Internal`], i.e. exit `1`.
fn exit_code_for(error: &anyhow::Error) -> u8 {
    let category = find_in_chain::<StartupError>(error.chain())
        .map(|e| e.category())
        .or_else(|| find_in_chain::<ConfigError>(error.chain()).map(|e| e.category()))
        .or_else(|| find_in_chain::<SigningKeyError>(error.chain()).map(|e| e.category()))
        .or_else(|| find_in_chain::<DbError>(error.chain()).map(|e| e.category()))
        .unwrap_or(Category::Internal);
    // Every `Category::exit_code()` is in `1..=78` (pinned by a test in
    // `vpay_core::error`), so this conversion cannot actually fail; `1` is
    // the same honest fallback as an unclassified error rather than a
    // truncating cast that could turn 78 into something meaningless.
    u8::try_from(category.exit_code()).unwrap_or(1)
}

/// Everything the boot sequence produces that [`run`] still needs once it is
/// over: the validated deployment, the rails this binary can reach, the pool
/// behind them, and the OP that will sign every `/v1` token.
///
/// A struct rather than a five-tuple because two of the fields are only
/// distinguishable by type (`Arc<dyn Repositories>` and `Arc<MerchantOp>` both
/// read as "some shared thing") and one of them, `adapters`, is the value the
/// router and the reconcile both consume.
struct Booted {
    config: vpay_config::Config,
    adapters: BTreeMap<String, Box<dyn vpay_provider::ProviderAdapter>>,
    repositories: Arc<dyn vpay_db::Repositories>,
    merchant_op: Arc<MerchantOp>,
}

/// The whole startup, as the ordered list of steps it is.
///
/// Every step is a named function and the order between them is the contract —
/// see
/// [docs/reference/vpay-config.md § the boot sequence](../../../../docs/reference/vpay-config.md#the-boot-sequence)
/// for why each one is where it is, and what a probe against a half-started
/// process gets.
#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let args = ServerArgs::parse();

    // Installed before anything else — including tracing init — so the
    // SIGTERM/SIGINT handlers are live for this process's entire lifetime, not
    // just once axum::serve's graceful-shutdown future is first polled. See
    // vpay_config::signal for the race this closes. A failure here is a hard
    // startup failure, deliberately not a logged warning that lets the process
    // continue with no graceful shutdown path at all.
    let mut shutdown_signals =
        ShutdownSignals::install().context("installing SIGINT/SIGTERM handlers")?;

    let metrics = install_process_defaults(&args)?;
    let booted = boot(&args).await?;

    // Bound *before* the validator is built, because the validator needs the
    // port this listener actually got — `--bind 127.0.0.1:0` is a real
    // configuration (every test in tests/cli.rs uses an ephemeral port) and
    // `args.bind` would still say `:0`.
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let bound = listener
        .local_addr()
        .context("reading the bound address back off the listener")?;
    let merchant_validator = loopback_validator(bound, &booted.merchant_op)?;

    tracing::warn!(
        "vpay-server implements /healthz, /v1/oauth (token, discovery, jwks), the /v1 \
         authentication boundary and /v1/payment_intents (create, retrieve, list, confirm, \
         cancel). No rail adapter implements `submit`, so a confirm reaches the rail and \
         answers 501 not_implemented; every other /v1 resource answers the honest 404. See \
         docs/status.md"
    );
    tracing::info!(addr = %bound, "listening");

    let deps = RouterDeps {
        repositories: booted.repositories,
        merchant_op: booted.merchant_op,
        merchant_validator,
        adapters: Arc::new(booted.adapters),
        // The projection, not the whole `Config` — see `ResourceConfig`, and
        // note this is the only way `deployment.livemode` reaches a handler.
        resource_config: Arc::new(
            ResourceConfig::from_config(&booted.config)
                .context("projecting the validated configuration onto the /v1 request path")?,
        ),
    };

    let (observability, observability_shutdown_tx) = start_observability(&args, metrics).await?;

    let shutdown_grace = Duration::from_secs(args.common.shutdown_grace_seconds);
    let shutdown = async move {
        shutdown_signals.wait().await;
        // A closed receiver just means the observability listener already
        // stopped — nothing to tell.
        let _ = observability_shutdown_tx.send(());
    };
    match serve_with_bounded_drain(listener, deps, shutdown_grace, shutdown).await? {
        DrainOutcome::Clean => {
            join_observability(observability, shutdown_grace).await;
            tracing::info!("graceful shutdown complete, exiting");
            Ok(())
        }
        DrainOutcome::TimedOut => {
            tracing::warn!(
                shutdown_grace_seconds = args.common.shutdown_grace_seconds,
                "shutdown grace period elapsed before in-flight requests finished draining; \
                 stopped waiting for them and exiting anyway"
            );
            // Exit non-zero rather than the clean path's implicit 0: unlike a
            // clean drain, this exit means real in-flight work was cut off. See
            // docs/reference/vpay-config.md § shutdown and drain.
            std::process::exit(1);
        }
    }
}

/// The three process-wide defaults, in the one order that is safe.
///
/// All three are global state that anything after them may depend on, so
/// nothing can be allowed to run ahead of them: a `reqwest::Client` built
/// before the crypto provider panics, and a metric recorded before the
/// recorder is installed goes nowhere and is never recovered. Tracing is last
/// of the three so the two that can fail do so before a subscriber exists to
/// swallow the message — `main`'s `eprintln!` is what reports them.
///
/// # Errors
///
/// Only [`install_recorder`]'s, which cannot happen in this binary.
fn install_process_defaults(args: &ServerArgs) -> anyhow::Result<PrometheusHandle> {
    install_crypto_provider();
    let metrics = install_recorder().context("installing the Prometheus metrics recorder")?;
    init_tracing(&args.common.log_filter, args.common.log_format);
    Ok(metrics)
}

/// Boot steps 1-4 plus this binary's own signing key, in the order
/// `docs/flows/configuration.md` fixes: rails, YAML, the join between them, the
/// key, the database, the reconcile, the key's activation.
///
/// Everything here that both binaries share is [`vpay_api::boot`], one
/// implementation, because both write the same two tables in the same database
/// and a divergence there would be silent. What is left in this function is
/// what only *this* binary does: it serves `/v1/oauth/token`, so it loads and
/// activates an RS256 signing key.
///
/// Nothing here binds a socket. That is the entire definition of `/livez`: a
/// probe against a process that is still inside this function, or one about to
/// exit `78`, gets a connection refusal rather than a cheerful 200.
///
/// # Errors
///
/// A missing flag, an invalid YAML file, a rail with no linked adapter, an
/// unmounted or unusable signing key, an unreachable database, a migration that
/// will not apply, or a reconcile that cannot take its lock. Each carries a
/// typed leaf so [`exit_code_for`] can tell `78` from `69`.
async fn boot(args: &ServerArgs) -> anyhow::Result<Booted> {
    // The codes, before anything is constructed: this log line is the first
    // thing an operator reads on a boot that later fails, and
    // `vpay_server::adapter_codes` is a `const` list precisely so printing it
    // cannot depend on a TLS stack having assembled.
    tracing::info!(rails = ?vpay_server::adapter_codes(), "provider adapters linked");

    // *One* outbound HTTP client for this process, handed to every adapter
    // (clones share its connection pool). Built here rather than inside
    // `adapters()` because construction is fallible and this is where a
    // failure can still exit cleanly with a classified code — and because a
    // second client would be a second pool, with no guarantee it was the
    // vendored-roots one the `FROM scratch` image needs (ADR-0004).
    //
    // The durations are the port's defaults. A rail's own budget travels on
    // `ProviderConfig::{connect_timeout,request_timeout}` and an adapter
    // applies it per request; this bounds anything that does not.
    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .context("building the outbound HTTP client every rail adapter shares")?;

    // Built once and used twice: the boot-step-4 join below, and
    // `RouterDeps::adapters`, which is how a `/v1/payment_intents/{id}/confirm`
    // reaches a rail at all.
    let adapters = vpay_api::boot::adapters_by_code(vpay_server::adapters(http));

    let config =
        vpay_api::boot::load_config(args.common.config.as_deref(), &args.common.profile)
            .context("loading and validating configuration (--config / VPAY_CONFIG, ADR-0003)")?;

    // Boot step 4's *inputs*, resolved before the database is touched: joining
    // the YAML's rails against this binary's adapters is pure CPU, so a
    // `providers[]` entry naming a rail nothing implements fails in
    // milliseconds and — as `a_provider_code_with_no_linked_adapter_is_exit_78`
    // in `tests/cli.rs` relies on — without a database.
    let (currency_seeds, provider_seeds) = vpay_api::boot::boot_seeds(&config, &adapters)?;

    let signing_key = load_signing_key(args, &config)?;

    // `--database-url` / `DATABASE_URL` stays `Option<String>` at the clap
    // level and is required here — see docs/reference/vpay-config.md
    // § optional flags that are required in practice.
    let database_url = args.common.database_url.as_deref().context(
        "--database-url / DATABASE_URL is required: vpay-server cannot serve traffic without \
         a database to open a pool against and migrate (see docs/status.md)",
    )?;
    let repositories = vpay_api::boot::open_migrated_database(database_url).await?;

    vpay_api::boot::reconcile_reference_tables(
        repositories.as_ref(),
        &currency_seeds,
        &provider_seeds,
    )
    .await
    .context("reconciling currencies and providers from configuration (boot step 4)")?;

    announce_signing_key(&signing_key, repositories.as_ref()).await?;

    let merchant_op = Arc::new(MerchantOp::new(
        &config,
        signing_key,
        Arc::clone(&repositories),
    ));

    Ok(Booted {
        config,
        adapters,
        repositories,
        merchant_op,
    })
}

/// The RS256 key every `/v1` access token is signed with, read from the Secret
/// mount named by `--oauth-signing-key-file`.
///
/// Loaded *before* the database and before the listener — reading a file needs
/// no network round trip, so a Secret that was not mounted fails in
/// milliseconds instead of after a Postgres connection and a migration run this
/// process is about to throw away. It also means the negative path is testable
/// without Docker (`tests/cli.rs`).
///
/// The `issuer` is asked for rather than formatted here: `vpay_api::op::issuer_for`
/// is the one derivation in the workspace and `MerchantOp::new` calls the same
/// function, so the `iss` this key stamps and the `iss` the OP advertises cannot
/// drift.
///
/// # Errors
///
/// [`StartupError::MissingSigningKeyFile`] when the flag is absent — this
/// binary serves `/v1/oauth/token`, and a server that cannot sign would bind a
/// port, answer `/healthz` with a cheerful 200 and refuse every real request.
/// [`SigningKeyError`] for a file that is not there, is not RSA, or is below
/// the 2048-bit floor. All of them are `78`.
fn load_signing_key(
    args: &ServerArgs,
    config: &vpay_config::Config,
) -> anyhow::Result<LoadedSigningKey> {
    let signing_key_file = args
        .oauth_signing_key_file
        .as_deref()
        .ok_or(StartupError::MissingSigningKeyFile)?;
    let issuer = vpay_api::op::issuer_for(config);
    let signing_key =
        LoadedSigningKey::from_file(signing_key_file, &issuer).with_context(|| {
            format!(
                "loading the OAuth signing key from {} (--oauth-signing-key-file / \
             VPAY_OAUTH_SIGNING_KEY_FILE)",
                signing_key_file.display()
            )
        })?;
    // The `kid` is public (it is in every token header and in `/jwks.json`);
    // the key itself never reaches a log line, a database column or an error
    // message — see `vpay_api::op::keys`.
    tracing::info!(kid = signing_key.kid(), %issuer, "OAuth signing key loaded");
    Ok(signing_key)
}

/// Records this key as the active one, so `/jwks.json` publishes it and every
/// replica agrees on which `kid` is current.
///
/// Fatal on failure: a process whose key is not in `oauth_signing_keys` mints
/// tokens that no verifier — including its own `/v1` — can check, so serving
/// traffic would be worse than not starting. Runs inside one locked transaction
/// in `vpay_db`, so N replicas booting at once produce one rotation between
/// them.
///
/// # Errors
///
/// [`vpay_db::DbError`], including `SigningKeyRetired` for a rollback to a key
/// this database has already retired — which classifies as a deploy to fix
/// (`78`), not a database to wait for (`69`).
async fn announce_signing_key(
    signing_key: &LoadedSigningKey,
    repositories: &dyn vpay_db::Repositories,
) -> anyhow::Result<()> {
    let activation = signing_key
        .ensure_active_in_database(repositories)
        .await
        .context("recording the OAuth signing key as active in oauth_signing_keys")?;
    match &activation {
        vpay_db::ActivationOutcome::AlreadyActive => {
            tracing::info!(
                kid = signing_key.kid(),
                "OAuth signing key was already the active one; no rotation"
            );
        }
        vpay_db::ActivationOutcome::Rotated { previous } => {
            tracing::warn!(
                kid = signing_key.kid(),
                previous_kid = previous.as_deref().unwrap_or("<none>"),
                "OAuth signing key rotated; the previous key stays publishable in /jwks.json for \
                 its overlap window"
            );
        }
    }
    Ok(())
}

/// The `/v1` token validator, pointed at this process's own JWKS over loopback.
///
/// Why loopback rather than the public URL the discovery document advertises is
/// [`loopback_jwks_url`]'s doc comment.
///
/// # Errors
///
/// Whatever building the JWKS client fails with — in practice only a TLS or
/// client-construction failure, since no request is made here.
fn loopback_validator(
    bound: SocketAddr,
    merchant_op: &MerchantOp,
) -> anyhow::Result<MerchantJwtValidator> {
    let jwks_url = loopback_jwks_url(bound);
    tracing::info!(
        %jwks_url,
        public_jwks_url = %merchant_op.jwks_url(),
        "validating /v1 tokens against this process's own JWKS over loopback"
    );
    Ok(MerchantJwtValidator(
        JwtValidator::new(
            jwks_url,
            JWKS_REFRESH_INTERVAL,
            merchant_op.issuer(),
            Surface::Merchant,
        )
        .context("building the JWKS client the /v1 token validator fetches with")?,
    ))
}

/// Binds `--observability-bind` and starts serving `/livez` and `/metrics` on
/// it, returning the task and the switch that stops it.
///
/// Bound **last**, after the config, the signing key, the database, the
/// migrations, boot step 4 and the validator: that ordering is the entire
/// definition of `/livez`, and nothing in the handler checks anything — the
/// bind *is* the check.
///
/// Never `args.bind`: `/metrics` names every rail, route and error code this
/// deployment has, and `args.bind` is the port an Ingress fronts. See
/// `vpay_config::CommonArgs::observability_bind`.
///
/// The returned sender is observed by the served future, so this listener stops
/// accepting at the same moment the traffic drain starts. A detached task with
/// no shutdown of its own would keep the port open past the drain and answer
/// `/livez` with `ok` while the process was on its way out.
///
/// # Errors
///
/// A bind failure on `--observability-bind`, or a listener whose bound address
/// cannot be read back.
async fn start_observability(
    args: &ServerArgs,
    metrics: PrometheusHandle,
) -> anyhow::Result<(
    tokio::task::JoinHandle<std::io::Result<()>>,
    tokio::sync::oneshot::Sender<()>,
)> {
    let listener = tokio::net::TcpListener::bind(args.common.observability_bind)
        .await
        .with_context(|| {
            format!(
                "binding the observability listener on {} (--observability-bind / \
                 VPAY_OBSERVABILITY_BIND)",
                args.common.observability_bind
            )
        })?;
    let bound = listener
        .local_addr()
        .context("reading the bound address back off the observability listener")?;
    // Logged because `--observability-bind 127.0.0.1:0` is a real configuration
    // — the subprocess tests in `tests/cli.rs` use it — and with a `:0` port
    // nothing else in the system can know the answer.
    tracing::info!(addr = %bound, "observability listener listening (/livez, /metrics)");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(vpay_api::observability::serve(
        listener,
        move || metrics.render(),
        async move {
            // A dropped sender means the traffic future ended some other way
            // and the process is exiting regardless; either way this listener
            // should stop.
            let _ = shutdown_rx.await;
        },
    ));
    Ok((task, shutdown_tx))
}

/// How often the `/v1` validator re-fetches the JWKS.
///
/// 300 s, matching [`vpay_api::op::jwks::JWKS_CACHE_MAX_AGE`] — the
/// `Cache-Control: max-age` the document is served with. Two different
/// numbers here would mean this process either re-fetched a document it was
/// told was still fresh, or held a copy past the freshness it advertised to
/// everyone else. It is not a correctness bound in either direction: the
/// cache also refetches immediately on an unrecognised `kid`, so a rotation
/// reaches this validator without waiting for the interval to lapse
/// (`vpay_api::resource_auth`), and the 24 h publication overlap
/// (`vpay_api::op::keys::ROTATION_OVERLAP`) means a stale copy is still a
/// usable one.
const JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// The URL this process's own `/v1` validator fetches the JWKS from: always
/// loopback, on the port actually bound.
///
/// **Deliberately not `public_base_url`**, and deliberately not `--bind` either
/// — an unspecified bind address is mapped to the matching loopback address of
/// the same family rather than used as a destination. Why, and what the
/// in-process alternative would cost:
/// [docs/reference/vpay-config.md § why the /v1 validator fetches its JWKS over loopback](../../../../docs/reference/vpay-config.md#why-the-v1-validator-fetches-its-jwks-over-loopback).
fn loopback_jwks_url(bound: SocketAddr) -> String {
    let host = match bound.ip() {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        specific => specific,
    };
    // `SocketAddr`'s own `Display` puts the brackets around an IPv6 literal,
    // which a URL authority requires and a bare `Ipv6Addr` does not produce.
    let authority = SocketAddr::new(host, bound.port());
    format!("http://{authority}/v1/oauth/jwks.json")
}

/// Serves `listener` until the shutdown signal fires, then waits at most
/// `shutdown_grace` for in-flight connections to drain.
///
/// axum has no built-in bound on that wait; the bound here comes from observing
/// the signal twice through a oneshot — see
/// [docs/reference/vpay-config.md § shutdown and drain](../../../../docs/reference/vpay-config.md#shutdown-and-drain).
///
/// `shutdown` is a future rather than the [`ShutdownSignals`] itself because the
/// signal has a *third* observer, the observability listener. Taking a future
/// keeps that composition in [`run`], where the listener it belongs to is
/// constructed, and matches `vpay_worker::run_loop`'s signature so the two
/// binaries' shutdown paths read the same way.
///
/// # Errors
///
/// Whatever `axum::serve` ends with, if it ends with one.
async fn serve_with_bounded_drain(
    listener: tokio::net::TcpListener,
    deps: RouterDeps,
    shutdown_grace: Duration,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<DrainOutcome> {
    let (drain_started_tx, drain_started_rx) = tokio::sync::oneshot::channel::<()>();

    // `axum::serve(..).with_graceful_shutdown(..)` returns a builder that
    // implements `IntoFuture`, not `Future` directly — `tokio::select!`
    // needs the latter, hence the explicit `.into_future()`.
    let serve_fut = axum::serve(listener, vpay_api::router(deps))
        .with_graceful_shutdown(async move {
            shutdown.await;
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

/// Waits for the observability listener to stop, bounded by the same grace
/// period the traffic drain uses.
///
/// Called only on the clean path, and failures here are logged rather than
/// propagated — see
/// [docs/reference/vpay-config.md § shutdown and drain](../../../../docs/reference/vpay-config.md#shutdown-and-drain).
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

/// Installs this process's Prometheus recorder, describes every metric name and
/// stamps `vpay_build_info`.
///
/// The twin of [`install_crypto_provider`] and placed beside it for the same
/// reason: it is a process-wide default, and anything recorded before it is
/// silently lost. Why a library does not do this, why `vpay-worker-bin` keeps a
/// near-copy, and when `git_sha` reads `unknown`:
/// [docs/reference/vpay-config.md § vpay_build_info's git_sha](../../../../docs/reference/vpay-config.md#vpay_build_infos-git_sha-and-when-it-is-unknown).
///
/// # Errors
///
/// Only if a recorder was already installed, which cannot happen in this binary
/// — nothing else calls `metrics::set_global_recorder`. It is reported rather
/// than ignored because the alternative is a process whose `/metrics` renders
/// empty forever while looking healthy.
fn install_recorder() -> anyhow::Result<PrometheusHandle> {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    if metrics::set_global_recorder(recorder).is_err() {
        anyhow::bail!(
            "a metrics recorder was already installed in this process; vpay-server installs \
             exactly one, so this is a bug rather than a deployment problem"
        );
    }
    vpay_core::metrics::describe_all();
    // `env!` here and `option_env!` inside `record_build_info`: the version
    // is this binary's own (a workspace where the two ever differ should
    // report the binary's), and the sha is the one thing a build is told
    // from outside.
    vpay_core::metrics::record_build_info(env!("CARGO_PKG_VERSION"));
    Ok(handle)
}

/// Waits for the shutdown signal to actually reach `with_graceful_shutdown`
/// (i.e. for draining to have started), then sleeps for `grace`.
///
/// If the sender is dropped without ever sending — which only happens if the
/// served future resolved some other way first, e.g. an accept error — this
/// never resolves. That is intentional: `tokio::select!` in
/// [`serve_with_bounded_drain`] drops this future the moment the served future
/// wins, so a server error can never be mistaken for a timed-out drain.
async fn grace_clock(drain_started_rx: tokio::sync::oneshot::Receiver<()>, grace: Duration) {
    if drain_started_rx.await.is_ok() {
        tokio::time::sleep(grace).await;
    } else {
        std::future::pending::<()>().await;
    }
}

/// Installs the process-wide rustls [`CryptoProvider`] this binary's HTTP
/// clients need, before anything can build one.
///
/// [`CryptoProvider`]: rustls::crypto::CryptoProvider
///
/// Without it, reqwest 0.13's `ClientBuilder::build()` panics — a panic in a
/// shipping payment binary, i.e. a defect under ADR-0007. No path this binary
/// reaches today depends on it, and it stays because the hazard it guards is one
/// `use` away rather than gone. Why the result is dropped, and what it is *not*
/// sufficient protection for:
/// [docs/reference/vpay-config.md § the rustls CryptoProvider process default](../../../../docs/reference/vpay-config.md#the-rustls-cryptoprovider-process-default).
///
/// `vpay-worker-bin` has a byte-identical copy, for the reason
/// [`exit_code_for`]'s doc gives.
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

    use std::net::SocketAddr;

    use super::{grace_clock, install_crypto_provider, loopback_jwks_url};

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

    /// The decisive property of `loopback_jwks_url`: the host is always a
    /// loopback address and the port is always the one that was *bound*.
    ///
    /// Each case is a bind address an operator actually writes.
    /// `--bind 0.0.0.0:8080` is `vpay_config::cli::ServerArgs`'s own default
    /// and is the case that matters most: a URL of
    /// `http://0.0.0.0:8080/...` is not a destination, and this process
    /// would be dialling it on every JWKS refresh. The `:0` cases are what
    /// every subprocess test in `tests/cli.rs` binds, which is why the port
    /// has to come from `TcpListener::local_addr` rather than from `--bind`.
    #[test]
    fn the_validators_jwks_url_is_always_loopback_on_the_bound_port() {
        for (bound, expected) in [
            ("0.0.0.0:8080", "http://127.0.0.1:8080/v1/oauth/jwks.json"),
            (
                "127.0.0.1:34567",
                "http://127.0.0.1:34567/v1/oauth/jwks.json",
            ),
            // A specific, non-loopback interface an operator bound on
            // purpose is used verbatim — this function does not second-guess
            // an explicit choice, it only refuses to treat "every interface"
            // as an address.
            ("10.1.2.3:8080", "http://10.1.2.3:8080/v1/oauth/jwks.json"),
            // IPv6: the family is preserved (an IPv6-only pod cannot dial
            // 127.0.0.1) and the literal is bracketed, which a URL authority
            // requires.
            ("[::]:8080", "http://[::1]:8080/v1/oauth/jwks.json"),
            ("[::1]:9090", "http://[::1]:9090/v1/oauth/jwks.json"),
        ] {
            let bound: SocketAddr = bound.parse().expect("a valid socket address");
            assert_eq!(loopback_jwks_url(bound), expected);
        }
    }

    /// The path half, pinned separately against the route `vpay_api::router`
    /// actually mounts. If the OP were ever remounted somewhere else, this
    /// URL would 404 and every `/v1` request would fail authentication with
    /// no clue why — so the two are asserted to agree rather than left to a
    /// reader to notice.
    #[test]
    fn the_validators_jwks_url_ends_at_the_route_the_router_mounts() {
        let url = loopback_jwks_url("127.0.0.1:8080".parse().expect("a valid socket address"));
        assert!(
            url.ends_with("/v1/oauth/jwks.json"),
            "the loopback URL must point at the route vpay_api::router mounts; got {url}"
        );
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
    /// this binary's real consumer builds succeeds. That consumer is the
    /// merchant validator's `JwksCache`, which builds its client lazily
    /// inside `Jwks::fetch` on the first `/v1` token validation — i.e. only
    /// once `run` has bound a port, mounted the router and served a request,
    /// none of which this unit test does. Reaching it here would mean
    /// standing up a JWKS server and a full `run`, which is an integration
    /// test (`backends/tests/integration`), not this one. The reqwest-side
    /// behaviour is documented on the root `Cargo.toml`'s pins; what is
    /// asserted here is only the precondition that call site relies on.
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

    /// A rollback to a retired signing key exits `78`, not `69`.
    ///
    /// This is the whole reason `vpay_db::DbError::SigningKeyRetired` is its
    /// own variant. The failure is a deployed Secret naming a key this
    /// database has already retired: restarting cannot fix it, so a
    /// supervisor that reads `69` ("wait for Postgres") sits in a crash loop
    /// forever, while `78` ("fix the deploy") is actionable. The error is
    /// wrapped in the same `.context(..)` `run` applies, so this exercises
    /// the chain walk and not just the leaf.
    ///
    /// The `Query` case below is the control: without it, an
    /// `exit_code_for` that returned 78 for *every* `DbError` — or one that
    /// stopped looking at categories at all — would still pass.
    #[test]
    fn a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69() {
        let retired = anyhow::Error::new(vpay_db::DbError::SigningKeyRetired {
            kid: "kid_old".to_owned(),
            retired_at: time::OffsetDateTime::UNIX_EPOCH,
        })
        .context("recording the OAuth signing key as active in oauth_signing_keys");
        assert_eq!(
            super::exit_code_for(&retired),
            78,
            "a retired-key rollback is a deploy to fix, not a database to wait for"
        );

        let unreachable = anyhow::Error::new(vpay_db::DbError::Connect(sqlx::Error::PoolTimedOut))
            .context("connecting to Postgres");
        assert_eq!(
            super::exit_code_for(&unreachable),
            69,
            "an unreachable database is still the transient case"
        );
    }
}
