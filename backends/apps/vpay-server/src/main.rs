//! vpay API server.
//!
//! Writes rows and returns. It never calls a payment rail — that is the
//! worker's job, and it is what makes the system crash-safe.

use std::future::IntoFuture as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
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
/// It exists so that "you forgot a flag" reaches an operator as exit `78`
/// ("fix the deploy") rather than `1` ("this is a vpay bug"). Without a
/// typed leaf in the chain, `.context("...is required")` on an `Option`
/// produces a bare `anyhow` error, [`exit_code_for`] finds nothing to
/// classify, and the honest-but-unhelpful [`Category::Internal`] fallback
/// applies. `--config`'s equivalent already gets `78` because
/// `vpay_config::Config::load` returns a typed `ConfigError::MissingPath`;
/// this is the same idea for the inputs that are checked here rather than
/// inside a library.
///
/// **Defined in the binary, not in `vpay-config`, deliberately.** Which
/// inputs a process requires is a property of *that process*, not of the
/// crate that declares the flags: `vpay-worker-bin` takes no
/// `--oauth-signing-key-file` at all (it issues no tokens, so mounting the
/// signing key into it would widen the Secret's blast radius for no
/// capability), so a `ConfigError` variant about a missing signing key would
/// be a requirement one binary has, spelled in a crate both link. ADR-0011
/// allows this: a closed `thiserror` enum with one `Classify` impl,
/// classified once and never re-decided at a call site. It is the same
/// reasoning [`exit_code_for`]'s own doc comment gives for why that function
/// is a deliberate near-copy rather than a shared helper.
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
/// `vpay-worker-bin` has a near-copy of this function, deliberately. A shared
/// helper has to name both leaf types it looks for, so it would need a home
/// that depends on `vpay-config` **and** `vpay-db` and exists only to hold
/// it. "Which leaf errors this binary knows how to classify" is a property of
/// the binary rather than a library boundary — the two are free to diverge as
/// they grow — so this is two copies of eight lines, each pinned by its own
/// CLI tests, rather than a crate created to avoid them.
///
/// The order is load-bearing, not alphabetical: `ConfigError` and
/// `SigningKeyError` are looked for **before** `DbError` because a chain can
/// plausibly contain both (a config that names an unreachable database), and
/// in that case the operator's actual problem is the configuration — `78`
/// ("fix the deploy") is more useful than `69` ("wait for Postgres").
///
/// Note that `DbError` being last does **not** mean it always means `69`:
/// the arm asks the leaf for its own category, and
/// `DbError::SigningKeyRetired` (a deployed Secret naming a key this
/// database has already retired — see `vpay_db::ensure_active_signing_key`)
/// classifies as `Category::Configuration`, so it exits `78` from this same
/// arm. That is the point of deriving the code from `Classify` rather than
/// from which `find_in_chain` matched: a leaf that knows it is a deploy
/// problem says so, and no ordering change was needed to let it.
/// `find_in_chain` is typed, so this function is also the exhaustive list of
/// leaf errors this binary knows how to classify; anything else — a `clap`
/// failure that got this far, a bind error, a panic-free `anyhow!` from
/// somewhere new — falls through to [`Category::Internal`], i.e. exit `1`.
/// That fallback is deliberately the pessimistic one: an unclassified
/// startup failure in a payment binary should look like a bug, not like a
/// known condition.
///
/// `SigningKeyError` joined the list when this binary started loading the
/// OAuth signing key: every one of its variants classifies as
/// `Category::Configuration`, so a missing Secret mount, a key that is not
/// RSA, and a key below the 2048-bit floor all exit `78` — the same number
/// as a broken YAML file, because they are the same kind of operator
/// problem. Without an arm here it would have fallen through to `1` and
/// looked like a vpay bug. Its position relative to `ConfigError` is
/// arbitrary in outcome (both are `78`) and deliberate in intent: config is
/// loaded first, so it is listed first.
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

#[tokio::main]
async fn run() -> anyhow::Result<()> {
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

    install_crypto_provider();

    init_tracing(&args.common.log_filter, args.common.log_format);

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
    let adapters = vpay_api::v1::boot::adapters_by_code(vpay_server::adapters(http));
    // `profile` names a YAML config file, never a code path — see
    // vpay_config::cli and docs/adr/0003-yaml-configuration.md.
    tracing::info!(profile = %args.common.profile, "deployment profile (selects a config file only)");

    // Load and validate the YAML deployment configuration (ADR-0003) before
    // touching the database at all. Boot-sequence steps 1-3 in
    // docs/flows/configuration.md — load, resolve `${ENV}`, validate — are
    // deliberately ordered ahead of step 4 (DB reconciliation, not yet
    // implemented) and step 5 (bind the port) in that doc, and this mirrors
    // it: validating a local YAML file needs no network round trip, so a
    // broken config fails in milliseconds instead of after paying for a
    // Postgres connection attempt and a migration run this process is about
    // to throw away anyway. `--config` / `VPAY_CONFIG` stays `Option<PathBuf>`
    // at the clap level (`vpay_config::CommonArgs::config`) for the same
    // reason `--database-url` does — see that field's doc comment — but is
    // required here: a payment gateway that boots with no validated
    // deployment configuration is exactly the half-configured process
    // ADR-0003 says must never serve traffic, `/healthz` included.
    let config = vpay_config::Config::load(args.common.config.as_deref(), &args.common.profile)
        .context("loading and validating configuration (--config / VPAY_CONFIG, ADR-0003)")?;
    // Redaction-safe by construction: discrete, non-secret fields only,
    // never the `Config` struct itself (its `Debug` is safe too, per its
    // own doc comment, but logging it wholesale would make that safety a
    // load-bearing assumption of this log line instead of a defence in
    // depth behind it).
    tracing::info!(
        deployment = %config.deployment.name,
        livemode = config.deployment.livemode,
        providers = config.providers.len(),
        merchant_clients = config.merchant_clients.len(),
        dashboard_client_configured = config.dashboard_client.is_some(),
        "configuration loaded and validated"
    );

    // Boot step 4's *inputs*, resolved here rather than beside the
    // `reconcile` call below: joining the YAML's rails against this binary's
    // adapters is pure CPU, so a `providers[]` entry naming a rail nothing
    // implements fails in milliseconds and — as
    // `a_provider_code_with_no_linked_adapter_is_exit_78` in `tests/cli.rs`
    // relies on — without a database. The write itself still happens at step
    // 4's documented position, after the migrations.
    let (currency_seeds, provider_seeds) = vpay_api::v1::boot::boot_seeds(&config, &adapters)?;

    // The RS256 signing key, loaded *before* the database and before the
    // listener — the same "cheapest hard failure first" ordering the config
    // load above follows, and for a sharper reason: reading a file needs no
    // network round trip, so a Secret that was not mounted fails in
    // milliseconds instead of after a Postgres connection and a migration
    // run this process is about to throw away. It also means the negative
    // path is testable without Docker (`tests/cli.rs`).
    //
    // `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE` stays
    // `Option<PathBuf>` at the clap level for the same reason `--config`
    // does (see `vpay_config::cli::ServerArgs`), and is required here: this
    // binary serves `/v1/oauth/token`, and a server that cannot sign cannot
    // mint a single merchant token — it would bind a port, answer
    // `/healthz` with a cheerful 200, and refuse every real request. That is
    // precisely the half-configured process ADR-0003 says must never serve
    // traffic.
    let signing_key_file = args
        .oauth_signing_key_file
        .as_deref()
        .ok_or(StartupError::MissingSigningKeyFile)?;

    // The issuer string every token carries and every validator pins. Asked
    // for rather than formatted here: `vpay_api::op::issuer_for` is the one
    // derivation in the workspace, and `MerchantOp::new` calls the same
    // function, so the `iss` this key stamps and the `iss` the OP advertises
    // cannot drift. See that function for what a mismatch would look like
    // (a bare 401 on every /v1 call, with no diagnostic anywhere).
    let issuer = vpay_api::op::issuer_for(&config);

    let signing_key =
        LoadedSigningKey::from_file(signing_key_file, &issuer).with_context(|| {
            format!(
                "loading the OAuth signing key from {} (--oauth-signing-key-file / \
             VPAY_OAUTH_SIGNING_KEY_FILE)",
                signing_key_file.display()
            )
        })?;
    // The `kid` is public (it is in every token header and in
    // `/jwks.json`); the key itself never reaches a log line, a database
    // column or an error message — see `vpay_api::op::keys`.
    tracing::info!(
        kid = signing_key.kid(),
        %issuer,
        "OAuth signing key loaded"
    );

    // `--database-url` / `DATABASE_URL` stays `Option<String>` at the clap
    // level (`vpay_config::CommonArgs`) for the same reason `--config` does
    // above, but is treated as required *here*: a payment server that binds
    // a listener and answers `/healthz` with no database behind it would be
    // lying about its own readiness (`/healthz` now runs a real `SELECT 1`
    // — see `vpay_api::router`), and this repo's own rule is never to look
    // more finished than it is. A missing value is a hard, loud startup
    // failure, not a silently DB-less scaffold mode.
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

    // Boot step 4 (`docs/flows/configuration.md`): make `currencies` and
    // `providers` match this deployment's configuration, in one transaction.
    // After the migrations because the tables have to exist, and before the
    // signing key is announced because everything past this point assumes a
    // database that agrees with the config file this process just validated.
    // Fatal on failure: a `providers` table that still enables a rail an
    // operator removed is a deployment that would keep taking charges on it.
    vpay_db::config_reconcile::reconcile(&pool, &currency_seeds, &provider_seeds)
        .await
        .context("reconciling currencies and providers from configuration (boot step 4)")?;
    tracing::info!(
        currencies = currency_seeds.len(),
        providers = provider_seeds.len(),
        enabled_providers = provider_seeds.iter().filter(|seed| seed.enabled).count(),
        "reference tables reconciled from configuration"
    );

    // Announce this key as the active one, so `/jwks.json` publishes it and
    // every replica agrees on which `kid` is current. Fatal on failure: a
    // process whose key is not in `oauth_signing_keys` mints tokens that no
    // verifier — including its own `/v1` — can check, so serving traffic
    // would be worse than not starting. Runs inside one locked transaction
    // in `vpay_db`, so N replicas booting at once produce one rotation
    // between them.
    let activation = signing_key
        .ensure_active_in_database(&pool)
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

    // A boot-time stopgap, not the cleanup job this table needs: vpay has no
    // worker job loop yet (`docs/status.md`), so nothing runs scheduled work.
    // Sweeping once per process start bounds `client_assertion_jtis` at
    // roughly "assertions since the last restart" instead of "assertions
    // forever". When the job loop lands, it should call this on a timer —
    // this call is meant to be *scheduled* properly then, not replaced.
    //
    // Non-fatal, deliberately: failing to prune is not a reason to refuse to
    // serve payments, and the rows it would have deleted are already expired
    // and therefore already unusable for a replay (see
    // `vpay_db::delete_expired_client_assertion_jtis`).
    match vpay_db::delete_expired_client_assertion_jtis(&pool).await {
        Ok(deleted) => tracing::info!(
            deleted,
            "swept expired client-assertion jtis (boot-time stopgap; there is no cleanup job yet)"
        ),
        Err(error) => tracing::warn!(
            %error,
            "could not sweep expired client-assertion jtis; continuing — replay protection is \
             unaffected, only table growth"
        ),
    }

    // The same stopgap, for the same reason, on `idempotency_keys`: no job
    // loop exists to schedule it, so once per process start is what there
    // is. When the loop lands, this call is the one to move onto a timer.
    //
    // Purely about table size, and deliberately so — no idempotency
    // guarantee depends on it running. A key past its 24-hour window is
    // reclaimed by `vpay_db::idempotency::claim` itself if a merchant
    // presents it again, so a deployment that never swept would still hand
    // every expired key back; it would simply keep the rows. Non-fatal for
    // that reason.
    match vpay_db::idempotency::sweep_expired(&pool).await {
        Ok(deleted) => tracing::info!(
            deleted,
            "swept expired idempotency keys (boot-time stopgap; there is no cleanup job yet)"
        ),
        Err(error) => tracing::warn!(
            %error,
            "could not sweep expired idempotency keys; continuing — an expired key is still \
             reclaimable on its next use, only the rows remain"
        ),
    }

    let merchant_op = Arc::new(MerchantOp::new(&config, signing_key, pool.clone()));

    // Bind *before* building the validator, because the validator needs the
    // port this listener actually got — `--bind 127.0.0.1:0` is a real
    // configuration (every test in tests/cli.rs uses an ephemeral port) and
    // `args.bind` would still say `:0`.
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let bound = listener
        .local_addr()
        .context("reading the bound address back off the listener")?;

    let jwks_url = loopback_jwks_url(bound);
    tracing::info!(
        %jwks_url,
        public_jwks_url = %merchant_op.jwks_url(),
        "validating /v1 tokens against this process's own JWKS over loopback"
    );
    let merchant_validator = MerchantJwtValidator(
        JwtValidator::new(
            jwks_url,
            JWKS_REFRESH_INTERVAL,
            merchant_op.issuer(),
            Surface::Merchant,
        )
        .context("building the JWKS client the /v1 token validator fetches with")?,
    );

    tracing::warn!(
        "vpay-server implements /healthz, /v1/oauth (token, discovery, jwks), the /v1 \
         authentication boundary and /v1/payment_intents (create, retrieve, list, confirm, \
         cancel). No rail adapter implements `submit`, so a confirm reaches the rail and \
         answers 501 not_implemented; every other /v1 resource answers the honest 404. See \
         docs/status.md"
    );
    tracing::info!(addr = %bound, "listening");

    let deps = RouterDeps {
        pool,
        merchant_op,
        merchant_validator,
        adapters: Arc::new(adapters),
        // The projection, not the whole `Config` — see `ResourceConfig`, and
        // note this is the only way `deployment.livemode` reaches a handler.
        resource_config: Arc::new(
            ResourceConfig::from_config(&config)
                .context("projecting the validated configuration onto the /v1 request path")?,
        ),
    };

    let shutdown_grace = Duration::from_secs(args.common.shutdown_grace_seconds);
    match serve_with_bounded_drain(listener, deps, shutdown_grace, shutdown_signals).await? {
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
/// **Deliberately not `public_base_url`.** The public URL is what a
/// *merchant* uses and what the discovery document advertises
/// (`MerchantOp::jwks_url`), but a pod is not guaranteed to be able to reach
/// its own public hostname: split-horizon DNS may not resolve it inside the
/// cluster, an ingress may terminate somewhere this process cannot route
/// back through, an egress `NetworkPolicy` may forbid the hairpin, and a
/// deployment behind a not-yet-warm DNS record would fail its first
/// validation. All of those turn "verify a token" into a network dependency
/// on infrastructure that exists to serve *inbound* traffic. Loopback has
/// none of those failure modes and reaches the same handler, backed by the
/// same database rows, that a merchant's fetch would.
///
/// The port comes from `TcpListener::local_addr`, not from `--bind`,
/// because `:0` is a real configuration.
///
/// An unspecified bind address (`0.0.0.0`, `[::]`) is mapped to the
/// corresponding loopback address rather than used as-is: `0.0.0.0` means
/// "listen on every interface" and is not a *destination* — connecting to it
/// is platform-dependent (Linux happens to route it to loopback; it is not
/// something to rely on in a payment binary). The address family is
/// preserved, so an IPv6-only deployment dials `[::1]` and not `127.0.0.1`.
/// A specific bind address is used verbatim: an operator who bound one
/// interface on purpose gets a URL on that interface.
///
/// This whole round trip is an HTTP call to ourselves and could later be
/// replaced by an in-process key source, which would remove a socket from
/// the path entirely. It is not done here because the alternative —
/// publishing the one key *this* process holds — is exactly the mistake
/// `vpay_api::op::jwks`'s module docs reject: during a rotation the JWKS
/// must carry every key still inside its overlap window, which is a property
/// of the database and not of this process's memory. An in-process source
/// would have to read the same rows and cache them, which is a real design
/// with its own invalidation question, not a simplification.
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
/// `axum::serve(..).with_graceful_shutdown(..)` waits indefinitely for
/// in-flight connections once its signal future resolves — there is no
/// built-in bound on that wait. To add one, the shutdown signal is observed
/// twice via a oneshot: axum's graceful-shutdown future uses it to start
/// draining, and a second consumer starts a `shutdown_grace`-long clock at
/// that same moment. Whichever finishes first — the drain, or the clock —
/// decides the [`DrainOutcome`].
async fn serve_with_bounded_drain(
    listener: tokio::net::TcpListener,
    deps: RouterDeps,
    shutdown_grace: Duration,
    mut shutdown_signals: ShutdownSignals,
) -> anyhow::Result<DrainOutcome> {
    let (drain_started_tx, drain_started_rx) = tokio::sync::oneshot::channel::<()>();

    // `axum::serve(..).with_graceful_shutdown(..)` returns a builder that
    // implements `IntoFuture`, not `Future` directly — `tokio::select!`
    // needs the latter, hence the explicit `.into_future()`.
    let serve_fut = axum::serve(listener, vpay_api::router(deps))
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
/// `reqwest::Client` is built", so this sits at the top of `run`, right
/// after the signal handlers and above `init_tracing`, where no future edit
/// can slip a client construction in ahead of it. It is deliberately *not*
/// done in a library: installing a process-wide default from a library takes
/// a decision out of the application's hands (the reasoning `sdks/rust`
/// records for why it hands reqwest a pre-built `ClientConfig` instead).
///
/// **What it does *not* cover, and why it stays anyway.** This used to be
/// the thing standing between `run` and a panic: the [`MerchantJwtValidator`]
/// that guards `/v1` was built from `authkestra_resource::jwt::JwksCache`,
/// whose `new` calls `reqwest::Client::new()` eagerly. That is no longer
/// true — `vpay_api::http_client` hands reqwest a finished
/// `rustls::ClientConfig`, which takes a branch that consults neither the
/// process default nor the OS trust store, and `vpay_api::jwks_cache` is
/// what lets the validator be given that client at all. `sqlx` never needed
/// it either (`vpay_db`'s module doc reads `sqlx-core`'s TLS setup and shows
/// it passes its own provider explicitly). So no path this binary reaches
/// today depends on this call.
///
/// It stays because the hazard it guards is one `use` away, not gone:
/// `authkestra-engine` still writes `reqwest::Client::new()` in its captcha
/// and device/client-credentials flows (`authkestra-engine-0.7.1/src/flow/`),
/// and tomorrow's first HTTPS-speaking rail adapter is another candidate.
/// Note that this call is **not** sufficient protection for either: inside
/// the `FROM scratch` runtime image a bare `reqwest::Client::new()` panics
/// on the *trust store* (`"No CA certificates were loaded from the system"`)
/// whether or not a provider is installed. `vpay_api::http_client::client`
/// is the only client constructor that works there, and any new outbound
/// HTTP in this binary should use it.
///
/// **Why the result is dropped.** `install_default()` returns
/// `Err(Arc<CryptoProvider>)` for exactly one reason: a default was already
/// installed. In a binary that means some other code got there first, which
/// is the state this call wanted anyway — so `.ok()`, which is what the root
/// `Cargo.toml`'s own note on the `authkestra-*` pins recommends verbatim.
/// `unwrap`/`expect` are denied here (ADR-0007) and would turn a harmless
/// double install into a startup crash.
///
/// `vpay-worker-bin` has a byte-identical copy, for the same reason its
/// `exit_code_for` and `init_tracing` are copies: this is a property of the
/// binary, not a library boundary.
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
