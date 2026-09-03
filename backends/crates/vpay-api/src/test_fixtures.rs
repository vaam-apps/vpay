//! Shared `#[cfg(test)]` fixtures for the four test modules in this crate
//! that need a fully-wired router or a real signing key.
//!
//! **Not a test double.** Everything here builds the *real* types
//! (`vpay_config::Config`, `op::MerchantOp`, `resource_auth::JwtValidator`,
//! `RouterDeps`) from real material — including a real RSA keypair — so a
//! test that passes here passes against the objects that ship. What it saves
//! is repetition: `lib.rs`, `error.rs` and the two `op` modules all need the
//! same three-field `RouterDeps`, and four private copies of it would drift.
//!
//! It lives in its own file rather than in `lib.rs`'s `mod tests` because
//! `error.rs` and `op/*.rs` cannot reach a sibling module's `#[cfg(test)]`
//! items without one — the same reason [`crate::test_log`] is a file.
//!
//! # Nothing here performs I/O
//!
//! The pool is `connect_lazy` (parses a URL, opens nothing) pointed at a
//! port nothing listens on, and the JWKS URL is likewise a dead loopback
//! port. That is deliberate rather than incidental: a test in this crate
//! that started reaching a database or fetching a key set would fail on a
//! refused connection rather than quietly passing against whatever was
//! listening. The cases that genuinely need Postgres live in
//! `backends/tests/integration`.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};
use vpay_config::oauth::{GrantType, MerchantClient};
use vpay_config::{Config, Deployment, MERCHANT_AUDIENCE};
use vpay_db::Repositories;

use crate::RouterDeps;
use crate::op::MerchantOp;
use crate::op::keys::LoadedSigningKey;
use crate::resource_auth::{JwtValidator, MerchantJwtValidator, Surface};

/// The base URL every fixture below derives its issuer and endpoints from.
/// A real `https` URL, not `localhost`: `Config::validate_all` treats the
/// scheme as meaningful under `livemode`, and a fixture that could not pass
/// validation would prove nothing about the real path.
pub(crate) const PUBLIC_BASE_URL: &str = "https://api.vpay.test";

/// What `MerchantOp::new` derives from [`PUBLIC_BASE_URL`], spelled out so a
/// test can assert against it without recomputing the formatting rule under
/// test.
pub(crate) const ISSUER: &str = "https://api.vpay.test/v1/oauth";

/// Repositories over a pool that has never opened a connection — see the
/// module docs and [`vpay_db::connect_lazy`].
///
/// Port 1 rather than 5432: a developer running `compose.yml` locally has a
/// real Postgres on 5432, and a fixture that could silently connect to it
/// would make these tests pass or fail depending on what is running on the
/// machine.
/// `acquire_timeout` is cut to 500 ms from sqlx's 30 s default purely for
/// suite runtime: sqlx keeps *retrying* a refused connection until the
/// timeout elapses, so the two tests that do reach the pool (`/healthz` and
/// `/v1/oauth/jwks.json`, both of which assert on "not a 401" rather than on
/// a successful query) would otherwise cost 30 s each. The property under
/// test is which answer an unreachable database produces, not how long the
/// failure took to arrive — the same reasoning `op::clients`'s own fixture
/// records.
pub(crate) fn lazy_repositories() -> Arc<dyn Repositories> {
    vpay_db::connect_lazy(
        "postgres://vpay:vpay@127.0.0.1:1/does-not-exist",
        Duration::from_millis(500),
    )
    .expect("connect_lazy performs no I/O and only fails on a malformed URL")
}

/// One real 2048-bit RSA key for the whole test binary.
///
/// Generated, never hard-coded: a fixture keypair checked into the
/// repository would be a private key in version control, and a shared one
/// would let a broken key-derivation path still "verify" against itself.
/// 2048 is the floor `LoadedSigningKey` enforces, chosen here because
/// generation is the slowest thing in this crate's test suite and no test
/// depends on the modulus size.
/// The key is *loaded* once too, not merely generated once:
/// `LoadedSigningKey::from_pem` parses the PEM twice by design (see its own
/// docs — the `kid` cannot be known before the first parse), and RSA private
/// key parsing precomputes CRT parameters, so re-loading it per test
/// dominated this crate's suite runtime. `LoadedSigningKey` is `Clone` over
/// an `Arc`'d `TokenManager`, so sharing it shares the same signer every
/// test would have built anyway.
static SIGNING_KEY: LazyLock<LoadedSigningKey> = LazyLock::new(|| {
    let mut rng = rand::rngs::OsRng;
    let pem = rsa::RsaPrivateKey::new(&mut rng, 2048)
        .expect("rsa key generation succeeds")
        .to_pkcs1_pem(LineEnding::LF)
        .expect("pkcs1 pem encoding succeeds")
        .to_string();
    LoadedSigningKey::from_pem(&pem, ISSUER).expect("a freshly generated 2048-bit RSA key loads")
});

/// The signing key a fixture `MerchantOp` mints with, loaded through the
/// real [`LoadedSigningKey::from_pem`] so the `kid` and public JWK are the
/// ones production would derive.
pub(crate) fn signing_key() -> LoadedSigningKey {
    SIGNING_KEY.clone()
}

/// A merchant registration shaped exactly as `config/application.yml`'s is,
/// including the `vpay:v1` audience `Config::validate_all` requires.
///
/// The JWK set is empty: no test in this crate verifies a `client_assertion`
/// through a fixture-built registration (`op::clients`'s own tests do that,
/// with real key material), and an empty set makes it impossible for one to
/// pass by accident.
pub(crate) fn merchant(client_id: &str, scopes: &[&str]) -> MerchantClient {
    MerchantClient {
        client_id: client_id.to_owned(),
        // Deliberately *not* the `client_id`: the tenant and the credential
        // are different values (`MerchantClient::merchant_id`), and a
        // fixture that made them equal would let a handler that queried by
        // the wrong one pass every test in this crate.
        merchant_id: format!("{client_id}-tenant"),
        jwks: Some(serde_json::json!({ "keys": [] })),
        grant_types: vec![GrantType::ClientCredentials],
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
        client_secret: None,
        // Empty for the same reason `jwks` is: nothing in this crate
        // delivers a webhook, and an endpoint here would be an endpoint no
        // test in this binary could ever exercise. The suites that do —
        // `backends/tests/integration/tests/webhooks.rs` — build their own
        // registration with real secrets.
        webhooks: Vec::new(),
        // The fail-closed default. Nothing in this crate's unit tests reaches
        // `/v1/browser` with a real key — that needs a database, so it is
        // `backends/tests/integration/tests/browser_checkout.rs` — and an
        // unused key here would be a key no test in this binary could
        // exercise.
        publishable_keys: Vec::new(),
    }
}

/// A `Config` with the given base URL and merchants and nothing else — no
/// providers, no currencies, no dashboard client. Built by struct literal
/// rather than loaded from YAML because the fields under test here are the
/// two `MerchantOp` reads, and a YAML fixture would add a file to keep in
/// sync for no extra coverage.
pub(crate) fn config_with(public_base_url: &str, merchants: Vec<MerchantClient>) -> Config {
    Config {
        deployment: Deployment {
            name: "test".to_owned(),
            livemode: false,
            public_base_url: public_base_url.to_owned(),
        },
        providers: Vec::new(),
        currencies: Vec::new(),
        merchant_clients: merchants,
        dashboard_client: None,
    }
}

/// The default fixture config: [`PUBLIC_BASE_URL`] and one merchant.
pub(crate) fn config() -> Config {
    config_with(
        PUBLIC_BASE_URL,
        vec![merchant("acme-cameroon", &["payments:write"])],
    )
}

/// A fully assembled OP over [`config`], the generated key and a lazy pool.
pub(crate) fn merchant_op() -> Arc<MerchantOp> {
    Arc::new(MerchantOp::new(
        &config(),
        signing_key(),
        lazy_repositories(),
    ))
}

/// The same [`RouterDeps`] `vpay-server`'s `main` builds, with a
/// generated key instead of a Secret mount and a deliberately unreachable
/// JWKS URL.
///
/// The URL never resolves and no test in this crate makes it try: every
/// `/v1` assertion is about a request with a missing or malformed
/// `Authorization` header, which `resource_auth::extract_bearer_token`
/// refuses before the JWKS cache is consulted. A test that *did* present a
/// syntactically valid token would fail here on a refused connection — the
/// loud failure, rather than a quiet pass. Proving the whole token round
/// trip needs a real server on a real port, which is
/// `backends/tests/integration/tests/merchant_token_flow.rs`.
pub(crate) fn deps() -> RouterDeps {
    RouterDeps {
        repositories: lazy_repositories(),
        merchant_op: merchant_op(),
        // Empty, and correct: `vpay-api` links no adapter crate at all
        // (ADR-0002) — the map is built by each binary from its own
        // `adapters()`. A test in this crate that needed a rail would be a
        // test that belongs in `backends/tests/integration`.
        adapters: Arc::new(std::collections::BTreeMap::new()),
        resource_config: Arc::new(
            crate::ResourceConfig::from_config(&config())
                .expect("the fixture's rails project onto the port"),
        ),
        merchant_validator: MerchantJwtValidator(
            JwtValidator::new(
                "http://127.0.0.1:1/v1/oauth/jwks.json",
                Duration::from_secs(300),
                ISSUER,
                Surface::Merchant,
            )
            .expect("the vendored-roots JWKS client builds"),
        ),
    }
}
