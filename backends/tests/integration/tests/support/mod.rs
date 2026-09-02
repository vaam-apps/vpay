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
use std::sync::{Arc, Once};

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
use vpay_api::resource_auth::MerchantJwtValidator;
use vpay_api::{ResourceConfig, RouterDeps};
use vpay_config::oauth::{GrantType, MerchantClient};
use vpay_config::{Config, MERCHANT_AUDIENCE};
use vpay_core::ProviderFlow;
use vpay_db::{CurrencySeed, ProviderSeed};
use vpay_provider::ProviderAdapter;

/// A migrated Postgres in a fresh container.
///
/// The container itself comes from
/// `vpay_testkit::containers::start_postgres_with_retry` (why the tag is
/// pinned, and which start errors are retried, are documented there); what
/// lives here is the pool and the migration run.
///
/// The `ContainerAsync` is returned rather than dropped: dropping it stops
/// the container, and a pool talking to a stopped container fails in a way
/// that looks like a vpay bug.
pub(crate) async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
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

    Ok((container, pool))
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
    MerchantClient {
        client_id: client_id.to_owned(),
        merchant_id: merchant_id.to_owned(),
        jwks: Some(jwks),
        grant_types: vec![GrantType::ClientCredentials],
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
        client_secret: None,
    }
}

/// Every shipping adapter, keyed by `providers.code` — the same map
/// `vpay-server`'s `main` hands `RouterDeps`.
///
/// The real `vpay-adapter-*` crates, not stand-ins: a suite that proved
/// `confirm` reaches "an adapter" without reaching *these* adapters would not
/// be proving that a confirm answers `501 not_implemented` for the reason it
/// actually does.
pub(crate) fn adapters_by_code() -> BTreeMap<String, Box<dyn ProviderAdapter>> {
    let adapters: Vec<Box<dyn ProviderAdapter>> = vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new()),
        Box::new(vpay_adapter_orange_money::Adapter::new()),
    ];
    adapters
        .into_iter()
        .map(|adapter| (adapter.code().to_owned(), adapter))
        .collect()
}

/// [`RouterDeps`] assembled the way `vpay-server`'s `main` assembles it.
///
/// Takes the `Config` rather than a pre-built [`ResourceConfig`] so a suite
/// cannot hand the router a projection that disagrees with the configuration
/// it also seeded the database from.
pub(crate) fn router_deps(
    pool: PgPool,
    merchant_op: Arc<MerchantOp>,
    merchant_validator: MerchantJwtValidator,
    config: &Config,
) -> RouterDeps {
    RouterDeps {
        pool,
        merchant_op,
        merchant_validator,
        adapters: Arc::new(adapters_by_code()),
        resource_config: Arc::new(ResourceConfig::from_config(config)),
    }
}

/// Boot step 4, run against a test's own configuration.
///
/// Calls the same `vpay_db::config_reconcile::reconcile` both binaries call,
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
pub(crate) async fn reconcile_from_config(pool: &PgPool, config: &Config) -> anyhow::Result<()> {
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

    vpay_db::config_reconcile::reconcile(pool, &currencies, &providers)
        .await
        .context("boot step 4: reconciling currencies and providers")
}
