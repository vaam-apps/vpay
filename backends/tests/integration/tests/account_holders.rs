//! `GET /v1/account_holders` end to end: the real `vpay_api::router` on a
//! real socket, with the shipping `vpay-adapter-mtn-momo` talking HTTP to a
//! real WireMock container (issue #47).
//!
//! ```text
//!   raw reqwest + a merchant bearer token   (the merchant's own server)
//!     -> GET /v1/account_holders?msisdn=…&payment_method_type=mtn_momo
//!            -> MTN adapter -> HTTP -> WireMock
//!            -> { object, payment_method_type, name, verified }
//! ```
//!
//! # What this file claims, and what it deliberately does not
//!
//! It claims the four things only a real request can show:
//!
//! 1. the route is **mounted and authenticated** — no token is a `401`,
//!    exactly as every other `/v1` path is;
//! 2. a number the rail knows answers `200` with the projected name, and a
//!    number it does not know answers `200` with `name: null` — over a
//!    socket, through the real adapter, from a real HTTP response;
//! 3. a rail that cannot be reached is a `502` envelope and **never** a
//!    `200` with nulls, which is the distinction the whole feature is for;
//! 4. a `payment_method_type` whose rail has no such API is a `400` naming
//!    the parameter, decided on the capability value.
//!
//! It does **not** claim anything about MTN. Every answer here comes from
//! `backends/tests/conformance/wiremock/mtn/mappings/basicuserinfo.json` —
//! the same directory `compose.yml` bind-mounts and the conformance suite
//! starts — and vpay has never called MTN's real sandbox for this or any
//! other operation. `docs/status.md` says so.
//!
//! # Where the other half of the proof is
//!
//! `backends/tests/conformance` holds the port-level cases (the projection,
//! the oversized body, the source chain, the PII that must not reach a log),
//! parameterised over **both** rails. `vpay_api::v1::account_holders`' own
//! unit tests hold the validation table, the metric and the log line. This
//! file is the seam those two cannot cover: that the handler is reachable,
//! authenticated, and wired to the adapter map a binary builds.
//!
//! # No test doubles
//!
//! Real Postgres, a real WireMock rail, the shipping adapter, the shipping
//! router (ADR-0006).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use serde_json::Value;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The rail that declares `supports_account_holder_lookup` and implements it.
const PUSH_RAIL: &str = "mtn_momo";
/// The rail that declares it `false` and inherits the port's `Unsupported`.
/// Configured here **on purpose**, so the refusal is proven against a rail
/// this deployment really offers rather than against a typo.
const REDIRECT_RAIL: &str = "orange_money";

/// The MSISDNs `wiremock/mtn/mappings/basicuserinfo.json` stubs. Digits
/// only, because this route validates Cameroon E.164 and would refuse a hex
/// steering number before the rail was called.
const MSISDN_REGISTERED: &str = "237600000200";
const MSISDN_UNREGISTERED: &str = "237600000404";

/// The name that mapping registers the number to.
const REGISTERED_HOLDER_NAME: &str = "David Mbarga";

/// A port nothing listens on: connection refused, which the adapter maps to
/// `ProviderError::Transport` and ADR-0011 classifies to `Category::Rail`
/// (502). The adapter's own 503 and timeout mappings are proven against a
/// stub in the conformance suite; what this file proves is what the
/// **boundary** does with that category.
const UNREACHABLE_RAIL: &str = "http://127.0.0.1:1";

fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

// ------------------------------------------------------------------ harness

struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    server: tokio::task::JoinHandle<()>,
    base_url: String,
    signing_key: LoadedSigningKey,
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// A `/v1` bearer token for `CLIENT_A`, minted with the server's own
    /// signer.
    ///
    /// Raw HTTP rather than `vpay-sdk` here, unlike `confirm_rails.rs`: what
    /// this file proves is the *server's* route, and the SDK's own coverage
    /// of it is `sdks/rust/tests/resources.rs` against an HTTP stub of the
    /// contract. Driving the SDK here would make one failure report two
    /// things.
    ///
    /// **`payments:read`, not `payments:write`.** This is a `GET`, and
    /// `required_scopes` says a read scope is enough for one — asserting it
    /// with the write scope would leave the route's actual authorisation
    /// untested, and a read-only credential is exactly what a merchant would
    /// issue to the thing that checks nominated destinations.
    fn bearer(&self) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token_with_extra(
                CLIENT_A,
                900,
                Some(vpay_api::SCOPE_PAYMENTS_READ.to_owned()),
                Some(MERCHANT_AUDIENCE.to_owned()),
                std::collections::HashMap::new(),
            )
            .expect("the server's own signer mints a merchant token")
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// A plain HTTP client, the way a merchant's own backend would hold one.
fn merchant_http() -> reqwest::Client {
    ensure_crypto_provider_installed();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("a plain reqwest client builds")
}

/// Two rails, one merchant, one currency — shaped exactly like
/// `config/application.yml`'s, including the settings and credentials keys
/// `vpay_config`'s `REQUIRED_RAIL_KEYS` insists on.
fn config_with(base_url: &str, mtn_url: &str, jwks_a: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "account-holders".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![
            ProviderHost {
                code: PUSH_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: mtn_url.to_owned(),
                    label: "mtn-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("target_environment".to_owned(), "sandbox".to_owned()),
                    (
                        "api_user".to_owned(),
                        "11111111-2222-3333-4444-555555555555".to_owned(),
                    ),
                ]),
                callback_url: None,
                currency: "XAF".to_owned(),
                credentials: BTreeMap::from([
                    (
                        "subscription_key".to_owned(),
                        "stub-subscription-key".to_owned(),
                    ),
                    ("api_key".to_owned(), "stub-api-key".to_owned()),
                ]),
            },
            // Pointed at nothing, deliberately: no case here reaches it over
            // the network, because the capability check refuses the request
            // before a rail call — which is what `ADR-0002` asks the core to
            // do, and what the refusal case below proves by the *absence* of
            // a connection failure.
            ProviderHost {
                code: REDIRECT_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: format!("{UNREACHABLE_RAIL}/orange-money-webpay/dev"),
                    label: "orange-nowhere".to_owned(),
                },
                settings: BTreeMap::from([
                    ("env".to_owned(), "dev".to_owned()),
                    ("lang".to_owned(), "en".to_owned()),
                ]),
                callback_url: None,
                currency: "XAF".to_owned(),
                credentials: BTreeMap::from([
                    ("merchant_key".to_owned(), "stub-merchant-key".to_owned()),
                    ("client_id".to_owned(), "stub-client-id".to_owned()),
                    ("client_secret".to_owned(), "stub-client-secret".to_owned()),
                ]),
            },
        ],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![merchant_client(CLIENT_A, MERCHANT_A, jwks_a)],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
    }
}

/// Boots Postgres, the MTN stub, and a server wired to both rails.
///
/// `mtn_url` is a parameter so the one case that needs an **unreachable**
/// MTN can boot the same deployment pointed at a dead port — same rail, same
/// route, one wrong value, which is how it happens in production on the day
/// a host moves.
async fn harness_with_mtn_at(mtn_url: Option<&str>) -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (postgres, repositories, _pool) = migrated_postgres().await?;

    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let stub_url = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );
    let configured = mtn_url.unwrap_or(&stub_url).to_owned();

    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, &configured, jwks_a)
    })
    .await?;

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        server: served.server,
        base_url: served.base_url,
        signing_key: served.signing_key,
    })
}

async fn harness() -> anyhow::Result<Harness> {
    harness_with_mtn_at(None).await
}

/// One lookup, as a merchant's backend makes it.
async fn lookup(h: &Harness, msisdn: &str, rail: &str) -> anyhow::Result<(u16, Value)> {
    let response = merchant_http()
        .get(h.url("/v1/account_holders"))
        .bearer_auth(h.bearer())
        .query(&[("msisdn", msisdn), ("payment_method_type", rail)])
        .send()
        .await
        .context("GET /v1/account_holders")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    Ok((status, body))
}

// -------------------------------------------------------------------- cases

/// A number the rail knows answers with the name, over a socket, from a real
/// HTTP response — and with **four keys and no more**.
///
/// The key count is the assertion that would fail if the rail's other five
/// documented fields ever reached the wire: the stub sends a birthdate, a
/// locale, a gender and a status, and none of them has anywhere to go.
#[tokio::test]
async fn a_registered_number_answers_with_the_holders_name() -> anyhow::Result<()> {
    let h = harness().await?;

    let (status, body) = lookup(&h, MSISDN_REGISTERED, PUSH_RAIL).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body,
        serde_json::json!({
            "object": "account_holder",
            "payment_method_type": PUSH_RAIL,
            "name": REGISTERED_HOLDER_NAME,
            "verified": true,
        }),
        "the four documented keys, and nothing the rail also sent"
    );

    h.shutdown().await;
    Ok(())
}

/// A number the rail has no record of answers `200` with `name: null` — the
/// answer a caller may act on as a fact about the number.
///
/// `null` and **present**, not omitted: both SDKs model `name` as a required
/// nullable field, so a dropped key is a decode failure in a merchant's own
/// client.
#[tokio::test]
async fn an_unregistered_number_answers_with_a_null_name_and_not_an_error() -> anyhow::Result<()> {
    let h = harness().await?;

    let (status, body) = lookup(&h, MSISDN_UNREGISTERED, PUSH_RAIL).await?;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body,
        serde_json::json!({
            "object": "account_holder",
            "payment_method_type": PUSH_RAIL,
            "name": null,
            "verified": false,
        })
    );
    assert!(
        body.as_object().is_some_and(|map| map.contains_key("name")),
        "`name` must be present and null: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// **The one that matters.** A rail that cannot be reached is a `502`
/// envelope, not a `200` whose `name` is null.
///
/// Reported as `Ok(None)` all the way to the merchant, this route would tell
/// an integrator that a real buyer's account is unregistered — and issue
/// #47's caller refuses a nominated refund destination on exactly that
/// signal. The status is derived from the error's `Category` (ADR-0011) and
/// never chosen at the handler, so this asserts the derivation survived the
/// whole stack.
#[tokio::test]
async fn a_rail_that_cannot_be_reached_is_a_502_and_never_a_null_name() -> anyhow::Result<()> {
    let h = harness_with_mtn_at(Some(UNREACHABLE_RAIL)).await?;

    let (status, body) = lookup(&h, MSISDN_REGISTERED, PUSH_RAIL).await?;
    assert_eq!(
        status, 502,
        "a rail we could not ask must not answer 200: {body:#}"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("api_error"),
        "{body:#}"
    );
    assert!(
        body.get("name").is_none() && body.get("verified").is_none(),
        "a failure must not be rendered as an account_holder at all: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// A rail whose capability is `false` is refused with a `400` naming the
/// parameter — **without a rail call**, which is why its host in this
/// deployment points at a dead port.
///
/// A `502` here would mean the core called a rail it was supposed to have
/// branched away from (ADR-0002); a `400` means the capability check ran
/// first. The two are told apart by the status alone, which is what makes
/// the dead port load-bearing rather than incidental.
#[tokio::test]
async fn a_rail_without_the_capability_is_refused_by_the_capability_not_by_the_rail()
-> anyhow::Result<()> {
    let h = harness().await?;

    let (status, body) = lookup(&h, MSISDN_REGISTERED, REDIRECT_RAIL).await?;
    assert_eq!(
        status, 400,
        "the capability check must refuse before any rail call: {body:#}"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "{body:#}"
    );
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("payment_method_type"),
        "an SDK reads `param` to point at a form field: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}

/// A number that is not a Cameroon mobile number is a `400` naming `msisdn`,
/// and the rail is never asked.
///
/// The hex steering numbers the WireMock mappings key on are in this list on
/// purpose: they are what a caller would try if they had read the stub
/// directory, and the validator is what keeps stub-specific behaviour
/// unreachable through a production-shaped route.
#[tokio::test]
async fn a_number_that_is_not_e164_is_refused_before_the_rail_is_asked() -> anyhow::Result<()> {
    let h = harness().await?;

    for msisdn in [
        "not-a-number",
        "237600000f01",
        "234600000200",
        "237700000200",
        "23760000020",
        "",
    ] {
        let (status, body) = lookup(&h, msisdn, PUSH_RAIL).await?;
        assert_eq!(status, 400, "{msisdn:?}: {body:#}");
        assert_eq!(
            body.pointer("/error/param").and_then(Value::as_str),
            Some("msisdn"),
            "{msisdn:?}: {body:#}"
        );
    }

    h.shutdown().await;
    Ok(())
}

/// The route is behind the same bearer boundary as every other `/v1` path.
///
/// `payment_intents.rs`'s `every_registered_v1_path_answers_401_without_a_token`
/// walks `V1_ROUTES` and therefore already covers this one the moment it was
/// added to the table — which is the point of that table being the router's
/// source. This case is here anyway because *this* route returns a third
/// party's name: "is it authenticated" is not a question to answer by
/// inference from a loop somewhere else, and a reader of this file should be
/// able to see it.
#[tokio::test]
async fn the_route_is_not_reachable_without_a_token() -> anyhow::Result<()> {
    let h = harness().await?;

    let response = merchant_http()
        .get(h.url("/v1/account_holders"))
        .query(&[
            ("msisdn", MSISDN_REGISTERED),
            ("payment_method_type", PUSH_RAIL),
        ])
        .send()
        .await
        .context("GET /v1/account_holders with no credential")?;
    assert_eq!(response.status().as_u16(), 401);
    let body: Value = response.json().await.context("the body is JSON")?;
    assert!(
        !body.to_string().contains(REGISTERED_HOLDER_NAME),
        "an unauthenticated caller must learn nothing about the holder: {body:#}"
    );

    h.shutdown().await;
    Ok(())
}
