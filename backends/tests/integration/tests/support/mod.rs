//! Shared harness for the integration suites in this directory.
//!
//! Each `tests/*.rs` compiles to its own binary, so a helper used by two of
//! them has to live in a subdirectory module like this one — a `tests/*.rs`
//! file would become a third test binary. Three of the four suites here used
//! to carry private copies of the pool-and-migrate helper, with the comment
//! "there is no `pub` item to import without introducing a shared module for
//! a handful of lines". Step 2 made it more than a handful: `RouterDeps` now
//! needs the adapter map and the configuration projection, and boot step 4
//! has to have run before any charge can reference a rail. Three copies of
//! *that* would drift, and a suite whose harness drifts from the binary's
//! wiring stops proving anything about the binary.
//!
//! **Nothing here is a test double.** The pool is a real Postgres in a real
//! container, the adapters are the shipping `vpay-adapter-*` crates, and the
//! router is `vpay_api::router`. What this module saves is repetition, not
//! reality (`AGENTS.md` rule 1).

// Each test binary uses a different subset of this module, so anything a
// given binary does not call is dead code *in that binary* — the standard
// consequence of the `tests/support` pattern, and not a signal about the
// item itself.
#![allow(dead_code)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Once};
use std::time::Duration;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};
use rsa::traits::PublicKeyParts as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::MerchantOp;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_api::resource_auth::{JwtValidator, MerchantJwtValidator, Surface};
use vpay_api::{ResourceConfig, RouterDeps};
use vpay_config::oauth::{GrantType, MerchantClient, WebhookEndpoint};
use vpay_config::{Config, MERCHANT_AUDIENCE};
use vpay_core::ProviderFlow;
use vpay_db::{CurrencySeed, ProviderSeed, Repositories, TxOutcome, UnitOfWork as _};
use vpay_provider::ProviderAdapter;

/// A migrated Postgres in a fresh container.
///
/// The container itself comes from
/// `vpay_testkit::containers::start_postgres_with_retry` (why the tag is
/// pinned, and which start errors are retried, are documented there); what
/// lives here are the repositories, a plain `sqlx` pool for the assertions
/// that read the schema itself, and the migration run.
///
/// The `ContainerAsync` is returned rather than dropped: dropping it stops
/// the container, and a pool talking to a stopped container fails in a way
/// that looks like a vpay bug.
pub(crate) async fn migrated_postgres()
-> anyhow::Result<(ContainerAsync<PostgresImage>, Arc<dyn Repositories>, PgPool)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
        .await
        .context("postgres:16-alpine container starts (it is cached locally on this machine)")?;

    let host = container.get_host().await.context("container host")?;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .context("container port")?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPool::connect(&url)
        .await
        .context("connects to the freshly started container")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("every migration under backends/migrations applies cleanly")?;

    let repositories = vpay_db::connect(&url)
        .await
        .context("the repositories connect to the same container")?;

    Ok((container, repositories, pool))
}

/// Installs the process-wide rustls `CryptoProvider`.
///
/// `authkestra_resource::jwt::JwksCache::new` builds a `reqwest::Client`
/// eagerly and the workspace pins reqwest with `rustls-no-provider`, which
/// panics without a process-wide default. `vpay-server`'s `main` installs one
/// at the top of startup for exactly this reason; each test binary is its own
/// process, so it has to do the same.
pub(crate) fn ensure_crypto_provider_installed() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
}

/// An RSA keypair in the two shapes these suites need: the private half as a
/// PKCS#1 PEM (what a merchant hands `vpay_sdk::Credentials::rsa_pem`, and
/// what a Secret mount holds for the server's own key) and the public half as
/// a JWK Set (what vpay holds in YAML for a merchant).
///
/// Generated per call, never hard-coded. 2048 bits is the floor
/// `vpay_api::op::keys` enforces and is what keeps these tests to about a
/// second of key generation each.
pub(crate) fn generate_key() -> (String, Value) {
    let mut rng = rand::rngs::OsRng;
    let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
    let public_key = private_key.to_public_key();
    let pem = private_key
        .to_pkcs1_pem(LineEnding::LF)
        .expect("pkcs1 pem encoding succeeds")
        .to_string();

    let jwks = json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be()),
        }]
    });
    (pem, jwks)
}

/// A merchant registration shaped exactly as `config/application.yml`'s is,
/// including the `vpay:v1` audience `Config::validate_all` requires — a
/// fixture that could not load from YAML would prove nothing about the real
/// path.
///
/// `merchant_id` is a separate argument rather than defaulted from
/// `client_id` on purpose: the tenant and the credential are different values
/// (`MerchantClient::merchant_id`), and a helper that made them equal would
/// let a handler querying by the wrong one pass every test here.
pub(crate) fn merchant_client(client_id: &str, merchant_id: &str, jwks: Value) -> MerchantClient {
    merchant_client_with_scopes(
        client_id,
        merchant_id,
        jwks,
        &[vpay_api::SCOPE_PAYMENTS_WRITE],
    )
}

/// The same, with the registration's `scopes:` spelled out.
///
/// The scope list is not decoration: it is what a token for this client
/// carries when the client requests none (RFC 6749 §3.3's default scope,
/// applied in `vpay_api::op::token::token_handler`), and therefore what
/// `/v1` authorises it for. A suite that wants to see a `403` about
/// authorisation registers a client with `&[]` here — which is a different
/// thing from an unregistered client, and answers 403 for a different
/// reason.
pub(crate) fn merchant_client_with_scopes(
    client_id: &str,
    merchant_id: &str,
    jwks: Value,
    scopes: &[&str],
) -> MerchantClient {
    merchant_client_with(client_id, merchant_id, jwks, scopes, Vec::new(), Vec::new())
}

/// The same, with this merchant's `/v1/browser` publishable keys spelled out.
///
/// Its own helper rather than a sixth argument everywhere for
/// [`merchant_client_with`]'s reason: exactly one suite
/// (`tests/browser_checkout.rs`) registers a key, and the fail-closed empty
/// list is what every other registration should carry — a default that a
/// call site has to opt out of is the wrong way round for a credential
/// namespace.
pub(crate) fn merchant_client_with_publishable_keys(
    client_id: &str,
    merchant_id: &str,
    jwks: Value,
    publishable_keys: &[&str],
) -> MerchantClient {
    merchant_client_with_checkout(client_id, merchant_id, jwks, publishable_keys, &[])
}

/// The same again, with this merchant's `checkout_origins` spelled out too
/// (Step 9, D4).
///
/// Its own helper rather than a seventh argument everywhere, for the reason
/// above: `tests/checkout_sessions.rs` is the only suite that registers an
/// origin, and the empty list — no embedding at all — is the fail-closed
/// shape every other registration should carry.
pub(crate) fn merchant_client_with_checkout(
    client_id: &str,
    merchant_id: &str,
    jwks: Value,
    publishable_keys: &[&str],
    checkout_origins: &[&str],
) -> MerchantClient {
    MerchantClient {
        checkout_origins: checkout_origins
            .iter()
            .map(|origin| (*origin).to_owned())
            .collect(),
        ..merchant_client_with(
            client_id,
            merchant_id,
            jwks,
            &[vpay_api::SCOPE_PAYMENTS_WRITE],
            Vec::new(),
            publishable_keys
                .iter()
                .map(|key| (*key).to_owned())
                .collect(),
        )
    }
}

/// The same, with this merchant's webhook endpoints spelled out.
///
/// Separate rather than a fifth argument on every call site: only
/// `tests/webhooks.rs` configures an endpoint, and the endpoints have to be
/// built *after* the receiver container has a mapped port, so they cannot be
/// a constant either.
pub(crate) fn merchant_client_with(
    client_id: &str,
    merchant_id: &str,
    jwks: Value,
    scopes: &[&str],
    webhooks: Vec<WebhookEndpoint>,
    publishable_keys: Vec<String>,
) -> MerchantClient {
    MerchantClient {
        client_id: client_id.to_owned(),
        merchant_id: merchant_id.to_owned(),
        jwks: Some(jwks),
        grant_types: vec![GrantType::ClientCredentials],
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
        client_secret: None,
        webhooks,
        publishable_keys,
        // The fail-closed default: no site may frame vpay's embedded
        // checkout for this merchant. `merchant_client_with_checkout` is how
        // a suite opts in, for the reason the publishable-key helper gives.
        checkout_origins: Vec::new(),
        // Absent, which is what most registrations carry — and the case the
        // browser reads fall back to the tenant id for.
        // `merchant_client_with_display_name` is how the one suite that
        // tests the configured half opts in.
        display_name: None,
    }
}

/// The same again, with a `display_name` — what a payer is told they are
/// paying on vpay's own checkout page (Step 9).
///
/// Its own helper for [`merchant_client_with_checkout`]'s reason, and one
/// more: absent is the shape that exercises the fallback, so a suite has to
/// say the word to get the configured branch, and both branches end up
/// covered rather than only whichever the default happens to be.
pub(crate) fn merchant_client_with_display_name(
    client_id: &str,
    merchant_id: &str,
    jwks: Value,
    publishable_keys: &[&str],
    checkout_origins: &[&str],
    display_name: &str,
) -> MerchantClient {
    MerchantClient {
        display_name: Some(display_name.to_owned()),
        ..merchant_client_with_checkout(
            client_id,
            merchant_id,
            jwks,
            publishable_keys,
            checkout_origins,
        )
    }
}

/// Every shipping adapter, keyed by `providers.code` — the same map
/// `vpay-server`'s `main` hands `RouterDeps`.
///
/// The real `vpay-adapter-*` crates, not stand-ins: a suite that proved
/// `confirm` reaches "an adapter" without reaching *these* adapters would not
/// be proving that a confirm answers `501 not_implemented` for the reason it
/// actually does.
///
/// Keyed through `vpay_api::v1::boot::adapters_by_code` rather than by a
/// `.collect()` of its own, which it used to be. That function is where a
/// rail is wrapped in `vpay_provider::Measured`, so a suite that built its
/// own map would run against *unmeasured* adapters and `worker_e2e`'s
/// `vpay_provider_requests_total` assertion would be asserting the absence
/// of a metric the shipping binaries do emit. Two spellings of "the adapter
/// map" is exactly the drift that function's own header argues against.
pub(crate) fn adapters_by_code() -> BTreeMap<String, Box<dyn ProviderAdapter>> {
    // The same vendored-roots client both binaries build at boot and hand to
    // every adapter (Step 3) — not a test-only substitute, for the same
    // reason the adapters themselves are the real ones.
    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .expect("the vendored-roots client builds");
    vpay_api::v1::boot::adapters_by_code(vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new(http.clone())),
        Box::new(vpay_adapter_orange_money::Adapter::new(http)),
    ])
}

/// [`RouterDeps`] assembled the way `vpay-server`'s `main` assembles it.
///
/// Takes the `Config` rather than a pre-built [`ResourceConfig`] so a suite
/// cannot hand the router a projection that disagrees with the configuration
/// it also seeded the database from.
pub(crate) fn router_deps(
    repositories: Arc<dyn Repositories>,
    merchant_op: Arc<MerchantOp>,
    merchant_validator: MerchantJwtValidator,
    config: &Config,
) -> RouterDeps {
    RouterDeps {
        repositories,
        merchant_op,
        merchant_validator,
        adapters: Arc::new(adapters_by_code()),
        resource_config: Arc::new(
            ResourceConfig::from_config(config)
                .expect("the suite's configuration projects onto the port"),
        ),
    }
}

/// Boot step 4, run against a test's own configuration.
///
/// Calls the same `vpay_db::ConfigReconcile::reconcile` both binaries call,
/// with seeds joined from the same real adapters, because `charges` has
/// foreign keys onto `providers(code)` and `currencies(code)`: without this,
/// every confirm in these suites would fail on a foreign key rather than on
/// whatever it is actually testing.
///
/// # Errors
///
/// Fails if the configuration names a rail no adapter implements — the same
/// `ConfigError::ProviderWithoutAdapter` a binary exits 78 on, so a suite
/// cannot configure a rail that could never be charged.
pub(crate) async fn reconcile_from_config(
    repositories: &dyn Repositories,
    config: &Config,
) -> anyhow::Result<()> {
    let adapters = adapters_by_code();

    let currencies = config
        .currencies
        .iter()
        .map(|entry| {
            Ok(CurrencySeed {
                code: entry.code.to_ascii_uppercase(),
                exponent: i32::try_from(entry.exponent)
                    .context("a currency exponent that fits the column")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let providers = config
        .providers
        .iter()
        .map(|provider| {
            let adapter = adapters.get(&provider.code).with_context(|| {
                format!(
                    "provider {} is configured but no adapter is linked (linked: {})",
                    provider.code,
                    adapters.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })?;
            let capabilities = adapter.capabilities();
            Ok(ProviderSeed {
                code: provider.code.clone(),
                display_name: provider.code.clone(),
                flow: match capabilities.flow {
                    ProviderFlow::Push => "push",
                    ProviderFlow::Redirect => "redirect",
                }
                .to_owned(),
                supports_refunds: capabilities.supports_refunds,
                supports_partial_refunds: capabilities.supports_partial_refunds,
                delivers_callbacks: capabilities.delivers_callbacks,
                requires_ip_allowlist: capabilities.requires_ip_allowlist,
                enabled: provider.enabled,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    repositories
        .reconcile(&currencies, &providers)
        .await
        .context("boot step 4: reconciling currencies and providers")
}

// ---------------------------------------------------------------- worker --
//
// Fixtures for `worker_recovery.rs` and `worker_e2e.rs`. Everything below
// writes rows the way a *crashed process* would leave them — a committed
// charge with no attempt row, an attempt row with no response — rather than
// simulating a crash, which is the honest framing `docs/flows/crash-safety.md`
// itself uses ("we cannot SIGKILL a handler mid-transaction from a test; what
// we can do is write the state each kill point leaves").

/// Each linked rail's `ProviderConfig`, keyed by `providers.code` — the
/// `vpay_worker::RailConfigs` map `vpay-worker-bin`'s `main` builds.
///
/// Built through `ResourceConfig` and not directly from the `Config`, exactly
/// as the binary does, so a suite cannot poll a rail at a host the same
/// configuration would have submitted to.
pub(crate) fn rail_configs(config: &Config) -> BTreeMap<String, vpay_provider::ProviderConfig> {
    let resource_config = ResourceConfig::from_config(config)
        .expect("the suite's configuration projects onto the port");
    config
        .providers
        .iter()
        .filter_map(|provider| {
            resource_config
                .rail(&provider.code)
                .map(|rail| (provider.code.clone(), rail.provider_config()))
        })
        .collect()
}

/// A merchant and an intent whose `confirm` has opened a charge but has not
/// come back — i.e. still `requires_payment_method`.
///
/// # Why not `processing`
///
/// Because `processing` is not what a crashed confirm leaves, and every
/// caller of this fixture pairs it with [`crashed_charge`]. `confirm` commits
/// the charge and its poll job in one transaction *before* the rail is
/// called, and moves the intent only afterwards, in `persist_submitted`
/// (`vpay_api::v1::payment_intents`). So all three of
/// `docs/flows/crash-safety.md`'s kill points leave a live charge against an
/// intent that still reads `requires_payment_method`, and a fixture that
/// moved it would be staging a state a crash cannot produce — and would hide
/// exactly the settlement path the recovery pass has to take.
///
/// This fixture used to write `processing` on the theory that the settlement
/// writers' guard "is the invariant, not a detail to work around". The guard
/// was the thing that was wrong: it refused the crash state outright, which
/// dead-lettered the poll job of a charge the rail may have collected. It now
/// names `requires_payment_method` too
/// (`vpay_db::payment_intents::SETTLEABLE_STATUSES`), because the *charge* —
/// not the intent's status — is the record of whether a confirm happened.
///
/// A suite that wants the post-confirm intent as well drives it there itself;
/// nothing here needs one, because a settled crash-recovery charge moves the
/// intent from wherever it is.
pub(crate) async fn confirmed_intent(
    repositories: &dyn Repositories,
    merchant_id: &str,
    rail: &str,
    amount: i64,
    currency: &str,
) -> anyhow::Result<String> {
    let id = vpay_core::ids::payment_intent_id();
    repositories
        .insert(&vpay_db::NewPaymentIntent {
            id: id.clone(),
            merchant_id: merchant_id.to_owned(),
            livemode: false,
            amount,
            currency_code: currency.to_owned(),
            status: vpay_core::IntentStatus::RequiresPaymentMethod
                .as_wire_str()
                .to_owned(),
            last_payment_error_code: None,
            last_payment_error_message: None,
            payment_method_types: json!([rail]),
            metadata: json!({}),
            description: None,
            // The same generator `vpay_api`'s `create` uses, not a literal: a
            // fixture with a hand-written suffix would be a fixture whose
            // intents are addressable by a value no real intent ever carries.
            client_secret_suffix: vpay_core::ids::client_secret_suffix(),
            // Supplied by the caller, as the real create path supplies it.
            // It is also the instant `keep_polling` measures the 24-hour
            // horizon from — the age of the payer's exposure, not of our
            // bookkeeping — so a fixture that back-dated it would be staging
            // an escalation rather than testing one.
            created_at: time::OffsetDateTime::now_utc(),
        })
        .await
        .context("inserting the payment intent")?;

    // Deliberately no `transition` call: `insert` already leaves the intent
    // in `requires_payment_method`, which is where a confirm that crashed
    // before `persist_submitted` leaves it. See this function's own comment.
    Ok(id)
}

/// The state a crash between the charge insert and the rail call leaves: a
/// committed charge in `submitting`, its poll job, and nothing else.
///
/// This is kill point 1 of `docs/flows/crash-safety.md`, and it is also the
/// starting point for the other two — they add an attempt row on top.
///
/// `reference` is a parameter because the rail stubs select their answer by
/// it: `…0404` is the reference MTN answers `RESOURCE_NOT_FOUND` to, `…0f01`
/// the one it declines, `…0560` the one it answers slowly. A confirm cannot
/// choose its reference (the handler mints it), so a suite that needs a
/// specific rail answer has to write the charge itself.
pub(crate) async fn crashed_charge(
    repositories: &dyn Repositories,
    payment_intent_id: &str,
    rail: &str,
    reference: uuid::Uuid,
    amount: i64,
    currency: &str,
    payer_ref: Option<&str>,
) -> anyhow::Result<String> {
    let id = vpay_core::ids::charge_id();
    repositories
        .transaction(|tx| {
            let id = &id;
            Box::pin(async move {
                tx.insert_for_intent(&vpay_db::NewCharge {
                    id: id.clone(),
                    payment_intent_id: payment_intent_id.to_owned(),
                    provider_code: rail.to_owned(),
                    provider_reference_id: reference,
                    provider_ref_extra: None,
                    redirect_url: None,
                    return_url: None,
                    state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
                    amount,
                    currency_code: currency.to_owned(),
                    payer_ref: payer_ref.map(str::to_owned),
                    payer_ref_masked: None,
                })
                .await
                .context("inserting the charge")?;
                // In the same transaction, exactly as `vpay_api`'s
                // `insert_charge` does it — that atomicity is the property
                // these suites exist to exercise.
                tx.enqueue_in_tx(
                    vpay_worker::JobKind::PollCharge.as_wire_str(),
                    &vpay_worker::jobs::poll_dedupe_key(id),
                    &json!({ "charge_id": id }),
                    time::OffsetDateTime::now_utc(),
                )
                .await
                .context("enqueueing the poll job")?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(()))
            })
        })
        .await
        .context("committing the charge")?;
    Ok(id)
}

/// How old a `submitting` charge must be before `vpay_worker::recovery_step`
/// will recover it at all, at the default policy, with margin.
///
/// `RecoveryPolicy::not_found_window` is sixty seconds; ninety is comfortably
/// past it and still nowhere near the twenty-four-hour horizon, so a fixture
/// that uses this crosses exactly one boundary.
pub(crate) const RECOVERABLE_CRASH_AGE: Duration = Duration::from_secs(90);

/// Back-dates `charges.created_at`, so a crash the fixture staged a moment ago
/// is as old as the crash it stands for.
///
/// **Why a fixture needs this at all.** `recovery_step` refuses to recover a
/// `submitting` charge younger than `RecoveryPolicy::not_found_window`,
/// because that state is also the ordinary state of a confirm that is still
/// running — `vpay-api` commits the charge and its poll job in one transaction
/// and only then compare-and-swaps `submitting → submitted`. A charge inserted
/// milliseconds ago is therefore *indistinguishable from a live confirm*, and a
/// suite whose crash fixtures were that young would be asserting the recovery
/// table against the one input the worker must not act on. Ageing the charge is
/// how a crash gets to be a minute old without the suite sleeping for a minute;
/// it is the same lever `age_past_the_horizon` pulls for the 24-hour
/// escalation, on the same column, and it leaves `RecoveryPolicy` exactly as a
/// deployment has it.
pub(crate) async fn age_the_crash(
    pool: &PgPool,
    charge_id: &str,
    age: Duration,
) -> anyhow::Result<()> {
    let aged = sqlx::query(
        "UPDATE charges SET created_at = now() - ($2::BIGINT * INTERVAL '1 second') \
         WHERE id = $1",
    )
    .bind(charge_id)
    .bind(i64::try_from(age.as_secs()).context("a fixture's crash age fits in an i64")?)
    .execute(pool)
    .await
    .context("ageing the crashed charge")?
    .rows_affected();
    anyhow::ensure!(aged == 1, "the charge was not there to age");
    Ok(())
}

/// Brings every future-dated job back to now.
///
/// The poll ladder's first rung is ten seconds (`vpay_worker::poll_delay`), so
/// a suite that wanted to watch three rungs would otherwise spend thirty
/// seconds asleep. Moving `run_at` is the test controlling *the queue's*
/// clock, not the code under test: the reschedule that put the job in the
/// future is a real write made by the real loop, and this only decides when
/// the next claim is allowed to see it.
pub(crate) async fn make_every_job_runnable(pool: &PgPool) -> anyhow::Result<u64> {
    let moved = sqlx::query("UPDATE jobs SET run_at = now() WHERE run_at > now()")
        .execute(pool)
        .await
        .context("moving future-dated jobs to now")?
        .rows_affected();
    Ok(moved)
}

/// Every `provider_requests.provider_reference_id` recorded for one charge.
///
/// The assertion `docs/flows/crash-safety.md` cares about most: "a fresh
/// reference on retry is how you double-charge a customer". Whatever the
/// recovery table decides, the set this returns must have exactly one member.
pub(crate) async fn attempted_references(
    pool: &PgPool,
    charge_id: &str,
) -> anyhow::Result<Vec<uuid::Uuid>> {
    let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT DISTINCT provider_reference_id FROM provider_requests WHERE charge_id = $1",
    )
    .bind(charge_id)
    .fetch_all(pool)
    .await
    .context("reading the attempted references")?;
    Ok(rows.into_iter().map(|(reference,)| reference).collect())
}

/// One merchant webhook endpoint, shaped exactly as
/// `merchant_clients[].webhooks[]` is in YAML.
///
/// A helper rather than a struct literal per test for the reason
/// [`merchant_client`] is one: a registration a suite built by hand that
/// `Config::validate_all` would refuse proves nothing about the real path,
/// and the rules here (a non-empty id, one or two non-empty secrets) are
/// exactly the ones an operator gets wrong.
pub(crate) fn webhook_endpoint(id: &str, url: &str, secrets: &[&str]) -> WebhookEndpoint {
    WebhookEndpoint {
        id: id.to_owned(),
        url: url.to_owned(),
        secrets: secrets.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// One running server: the task serving it, where it is, and the key it
/// signs tokens with.
pub(crate) struct Served {
    pub(crate) server: tokio::task::JoinHandle<()>,
    pub(crate) base_url: String,
    pub(crate) signing_key: LoadedSigningKey,
}

/// Stands a vpay server up on an ephemeral port over `repositories`, in
/// `vpay-server`'s own boot order: announce the signing key, run boot step 4,
/// bind, serve.
///
/// `make_config` takes the base URL because the configuration cannot be
/// built until the port is known — `public_base_url` is what the issuer, the
/// assertion audience and every callback URL are derived from, and a
/// placeholder would make the OP mint tokens no validator here would accept.
///
/// A function rather than inlined in each suite's harness because several
/// tests boot a *second* server over the same database with a different
/// configuration, which is exactly what an operator editing
/// `application.yml` and redeploying does. It lives in this module rather
/// than in a suite because three test binaries need it and three
/// hand-rolled copies would be three chances to boot it in an order the
/// binary does not — the same argument this module's header makes about the
/// pool-and-migrate helper.
///
/// # Errors
///
/// Fails if the port cannot be bound, the signing key cannot be loaded or
/// announced, or boot step 4 refuses the configuration.
pub(crate) async fn serve(
    repositories: &Arc<dyn Repositories>,
    server_pem: &str,
    make_config: impl FnOnce(&str) -> Config,
) -> anyhow::Result<Served> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("binding an ephemeral loopback port")?;
    let bound = listener.local_addr().context("reading the bound port")?;
    let base_url = format!("http://{bound}");
    let issuer = format!("{base_url}/v1/oauth");

    let signing_key =
        LoadedSigningKey::from_pem(server_pem, &issuer).context("loading the signing key")?;
    signing_key
        .ensure_active_in_database(repositories.as_ref())
        .await
        .context("announcing the signing key in oauth_signing_keys")?;

    let config = make_config(&base_url);
    reconcile_from_config(repositories.as_ref(), &config).await?;

    let merchant_op = Arc::new(MerchantOp::new(
        &config,
        signing_key.clone(),
        Arc::clone(repositories),
    ));
    let merchant_validator = MerchantJwtValidator(
        JwtValidator::new(
            format!("{base_url}/v1/oauth/jwks.json"),
            Duration::from_secs(300),
            merchant_op.issuer(),
            Surface::Merchant,
        )
        .expect("the vendored-roots JWKS client builds"),
    );

    let deps = router_deps(
        Arc::clone(repositories),
        merchant_op,
        merchant_validator,
        &config,
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, vpay_api::router(deps)).await;
    });

    Ok(Served {
        server,
        base_url,
        signing_key,
    })
}

// -------------------------------------------------------------- webhooks --

/// The endpoint table `vpay_worker::{run_loop, run_once}` take, for a suite
/// whose subject is not webhook delivery.
///
/// **Empty is a real configuration, not a stand-in.** A deployment whose
/// merchants have registered no endpoints is exactly this, and the fan-out
/// drain still runs against it: it marks such an event `fanout_state = 'done'`
/// with zero deliveries, because the alternative is a backlog index that grows
/// forever (`vpay_worker::webhooks::handle_fan_out`). So a suite that uses
/// this is asserting the loop's real behaviour for that deployment rather
/// than switching a feature off — which a `#[cfg(test)]` seam would be, and
/// AGENTS.md rule 1 forbids.
pub(crate) fn no_webhook_endpoints() -> Arc<vpay_worker::EndpointRegistry> {
    Arc::new(vpay_worker::EndpointRegistry::from_pairs(
        std::iter::empty::<(String, Vec<vpay_worker::Endpoint>)>(),
    ))
}

/// The egress policy `vpay_worker::{run_loop, run_once}` take, for a suite
/// whose subject is not webhook delivery.
///
/// The **shipping default** — a private address is refused — because that is
/// what a deployment gets, and a suite that quietly allowed private targets
/// would be exercising a worker no deployment runs. It costs these suites
/// nothing: they register no endpoints (see [`no_webhook_endpoints`]), so no
/// delivery is ever attempted and the guard never runs. The suite whose
/// subject *is* delivery — `webhooks.rs` — passes
/// `EgressPolicy::ALLOW_PRIVATE`, because its receiver container answers on
/// loopback, and proves both verdicts.
///
/// There is no `webhook_client()` helper any more: since Step 8 the delivery
/// client is built per delivery and pinned to the addresses that delivery's
/// host resolved to (`vpay_worker::ssrf`), so there is no client for a
/// harness to build. The two budgets it used to single-source are still
/// `vpay_worker::WEBHOOK_{CONNECT,REQUEST}_TIMEOUT`, read now by
/// `vpay_worker::ssrf::pinned_client`.
pub(crate) fn default_egress_policy() -> vpay_worker::EgressPolicy {
    ensure_crypto_provider_installed();
    vpay_worker::EgressPolicy::DENY_PRIVATE
}
