//! The signed `/v1` transport end to end: the real `vpay_api::router` with its
//! real `EnvelopeLayer` on a real socket, over a real Postgres replay cache and
//! the real merchant key registry (VP-3, `docs/signed-transport.md`).
//!
//! Every other suite in this crate *uses* the transport — each `/v1` call it
//! makes is sealed by `support::sealed` and opened by the layer. This one is
//! about the transport itself, and about the four ways a call can be refused
//! before a handler sees it. Each is shown failing, not only passing: the
//! assertion is on the refusal, and the control beside it is the same call
//! without the one defect.
//!
//! - **(a)** a sealed form `POST` round-trips as sealed JSON: the request body
//!   is opened to `application/x-www-form-urlencoded`, the handler's answer is
//!   sealed under the server's key, and nothing on the wire is plain.
//! - **(b)** a **replayed** envelope — the identical bytes, twice — is
//!   refused the second time, and the handler ran once. The nonce row lives in
//!   Postgres (`envelope_nonces`), so the proof does not depend on one
//!   process's memory.
//! - **(c)** a **disabled** client is refused on its next call, with no
//!   restart, and the refusal is the same coarse `401` an unknown key gets.
//! - **(d)** a **foreign tenant** gets the uniform sealed `404`: another
//!   merchant's object and an id that never existed are indistinguishable.
//! - **(e)** an **unsigned** request is refused, on every route the router
//!   registers and for every cause — no signature at all, a bearer token, a
//!   key nobody registered, a request addressed to a different audience. The answer is the layer's coarse
//!   `401`, identical for each.
//! - **(f)** a client that pins the **wrong server key** refuses the answer
//!   rather than trusting it.
//!
//! No test doubles (ADR-0006): the signing side is `support::sealed`, which
//! seals with cratestack's own client and binds routes from `vpay-wire`
//! independently of the SDK.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use anyhow::Context as _;
use serde_json::Value;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_config::{Config, Deployment};

mod support;

use support::sealed::SealedHttp;
use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: std::sync::Arc<dyn vpay_db::Repositories>,
    base_url: String,
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "signed-transport".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: Vec::new(),
        currencies: Vec::new(),
        merchant_clients: vec![
            merchant_client(CLIENT_A, MERCHANT_A, jwks_a),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard: None,
        service_clients: Vec::new(),
        routing: Default::default(),
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();
    let (container, repositories, _pool) = migrated_postgres().await?;
    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();
    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b)
    })
    .await?;
    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        base_url: served.base_url,
    })
}

fn plain() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
}

/// A fresh `Idempotency-Key`: every write names one, and a reused key would
/// replay a stored answer instead of reaching the handler.
fn key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The coarse answer, asserted once: the layer's own `401`, unsigned, carrying
/// `UNAUTHORIZED` and nothing about *why*. A caller that asked for JSON gets
/// `{"code": "UNAUTHORIZED", …}`; a sealed caller asked for the envelope's type
/// and gets the same document as CBOR, so the code is looked for as the text it
/// is in either encoding, and parsed when it is JSON.
fn assert_coarse_401(status: u16, signed: bool, body: &[u8], what: &str) {
    assert_eq!(status, 401, "{what}");
    assert!(!signed, "{what}: a refusal is never sealed");
    assert!(
        body.windows(b"UNAUTHORIZED".len())
            .any(|window| window == b"UNAUTHORIZED"),
        "{what}: the refusal carries UNAUTHORIZED: {body:?}"
    );
    if body.first() == Some(&b'{') {
        let body: Value = serde_json::from_slice(body)
            .unwrap_or_else(|error| panic!("{what}: the 401 body is JSON ({error})"));
        assert_eq!(
            body.get("code").and_then(Value::as_str),
            Some("UNAUTHORIZED"),
            "{what}: got {body:#}"
        );
    }
}

async fn create_customer(h: &Harness, client_id: &str, name: &str) -> anyhow::Result<Value> {
    let response = SealedHttp::new()
        .post(h.url("/v1/customers"))
        .signed_by(client_id)
        .header("idempotency-key", key())
        .form(&[("name", name)])
        .send()
        .await
        .context("creating a customer")?;
    anyhow::ensure!(
        response.is_sealed(),
        "the answer to a sealed call is sealed"
    );
    anyhow::ensure!(
        response.status().as_u16() == 201,
        "{:?}",
        response.text().await
    );
    Ok(response.json().await?)
}

// --------------------------------------------------------------- case (a)

#[tokio::test]
async fn a_sealed_form_post_round_trips_as_sealed_json() -> anyhow::Result<()> {
    let h = harness().await?;

    let response = SealedHttp::new()
        .post(h.url("/v1/customers"))
        .signed_by(CLIENT_A)
        .header("idempotency-key", key())
        .form(&[("name", "Ada")])
        .send()
        .await?;

    assert!(
        response.is_sealed(),
        "the answer is sealed under the server's key"
    );
    assert_eq!(response.status().as_u16(), 201);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/cose")),
        "on the wire the answer is a COSE message, not JSON: {:?}",
        response.headers()
    );
    let body: Value = response.json().await?;
    assert_eq!(body.get("object").and_then(Value::as_str), Some("customer"));
    assert_eq!(body.get("name").and_then(Value::as_str), Some("Ada"));

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (b)

/// The identical envelope, twice. The handler ran once.
///
/// The control is the first answer: the same bytes are accepted when they are
/// new. The failure is the second, and it is the layer's coarse `401` — not a
/// `409` from the idempotency table, which would mean the replay reached the
/// handler and was only caught there.
#[tokio::test]
async fn a_replayed_envelope_is_refused_the_second_time_and_the_handler_ran_once()
-> anyhow::Result<()> {
    let h = harness().await?;

    let answers = SealedHttp::new()
        .post(h.url("/v1/customers"))
        .signed_by(CLIENT_A)
        .header("idempotency-key", key())
        .form(&[("name", "Replayed")])
        .send_replayed(2)
        .await?;

    let [first, second] = answers.as_slice() else {
        anyhow::bail!("send_replayed(2) answered {} times", answers.len());
    };
    assert!(first.is_sealed());
    assert_eq!(first.status().as_u16(), 201, "the first use is accepted");
    assert_coarse_401(
        second.status().as_u16(),
        second.is_sealed(),
        &second.bytes().await?,
        "the same envelope a second time",
    );

    let listed = SealedHttp::new()
        .get(h.url("/v1/customers"))
        .signed_by(CLIENT_A)
        .send()
        .await?;
    let listed: Value = listed.json().await?;
    assert_eq!(
        listed.get("data").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "the replay did not create a second customer: {listed:#}"
    );

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (c)

#[tokio::test]
async fn a_disabled_client_is_refused_on_its_next_call_like_an_unknown_one() -> anyhow::Result<()> {
    let h = harness().await?;
    let url = h.url("/v1/customers");

    let before = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .send()
        .await?;
    assert!(before.is_sealed());
    assert_eq!(
        before.status().as_u16(),
        200,
        "the control: enabled, it works"
    );

    h.repositories
        .disable_client(CLIENT_A, Some("signed_transport test"))
        .await
        .context("flipping the kill switch")?;

    let after = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .send()
        .await?;
    assert_coarse_401(
        after.status().as_u16(),
        after.is_sealed(),
        &after.bytes().await?,
        "a disabled client",
    );

    // Indistinguishable from a key nobody registered.
    let unknown = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .signed_with_seed([7; 32])
        .send()
        .await?;
    assert_eq!(unknown.status(), after.status());
    assert_eq!(unknown.bytes().await?, after.bytes().await?);

    // And the other merchant is unaffected.
    let other = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_B)
        .send()
        .await?;
    assert_eq!(other.status().as_u16(), 200);

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (d)

/// Another merchant's object and an id that never existed answer the same
/// sealed `404`, byte for byte once each request's own id is substituted out.
#[tokio::test]
async fn a_foreign_tenant_gets_the_uniform_sealed_404() -> anyhow::Result<()> {
    let h = harness().await?;
    let theirs = create_customer(&h, CLIENT_A, "PrivateToA").await?;
    let theirs_id = theirs.get("id").and_then(Value::as_str).unwrap().to_owned();
    let missing_id = "cus_00000000000000000000000x";

    let foreign = SealedHttp::new()
        .get(h.url(&format!("/v1/customers/{theirs_id}")))
        .signed_by(CLIENT_B)
        .send()
        .await?;
    let missing = SealedHttp::new()
        .get(h.url(&format!("/v1/customers/{missing_id}")))
        .signed_by(CLIENT_B)
        .send()
        .await?;

    assert!(foreign.is_sealed() && missing.is_sealed());
    assert_eq!(foreign.status().as_u16(), 404);
    assert_eq!(missing.status().as_u16(), 404);
    let foreign = String::from_utf8_lossy(&foreign.bytes().await?).replace(&theirs_id, "<id>");
    let missing = String::from_utf8_lossy(&missing.bytes().await?).replace(missing_id, "<id>");
    assert_eq!(foreign, missing, "a foreign id must not be an oracle");

    // The control: the owner reads it.
    let own = SealedHttp::new()
        .get(h.url(&format!("/v1/customers/{theirs_id}")))
        .signed_by(CLIENT_A)
        .send()
        .await?;
    assert_eq!(own.status().as_u16(), 200);

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (e)

/// Every `/v1` route the router registers, on every method, refuses a request
/// that carries no signature — with the coarse `401` and nothing else.
#[tokio::test]
async fn every_route_refuses_an_unsigned_request_with_the_coarse_401() -> anyhow::Result<()> {
    let h = harness().await?;
    let http = plain();

    assert!(
        !vpay_api::V1_ROUTES.is_empty(),
        "an empty table proves nothing"
    );
    let mut checked = 0_usize;
    for route in vpay_api::V1_ROUTES {
        let path = route.path.replace("{id}", "pi_00000000000000000000000x");
        for method in route.methods {
            let url = h.url(&format!("/v1{path}"));
            let response = http
                .request(method.parse()?, &url)
                .header("accept", "application/json")
                .header("idempotency-key", "unsigned")
                .send()
                .await
                .with_context(|| format!("{method} {url}"))?;
            let status = response.status().as_u16();
            let body = response.bytes().await?;
            assert_coarse_401(status, false, &body, &format!("{method} /v1{path}"));
            checked += 1;
        }
    }
    assert!(
        checked >= vpay_api::V1_ROUTES.len(),
        "every method was tried"
    );

    h.shutdown().await;
    Ok(())
}

/// Every cause of an unsigned refusal is the same bytes.
///
/// Two families, because the layer answers in the type the caller asked for
/// and a sealed caller asks for the envelope's: a plain request that asks for
/// JSON gets the JSON refusal, and a signed one that is refused gets the CBOR
/// one. Within a family the answer is identical for every cause — which is the
/// property, since a refusal that varied with the cause would be an oracle.
#[tokio::test]
async fn every_cause_of_a_refusal_is_the_same_unsigned_answer() -> anyhow::Result<()> {
    let h = harness().await?;
    let url = h.url("/v1/customers");

    // Plain callers, asking for JSON: no signature at all, and a bearer token.
    let no_signature = plain()
        .get(&url)
        .header("accept", "application/json")
        .send()
        .await?;
    let bearer = plain()
        .get(&url)
        .header("accept", "application/json")
        .bearer_auth("an.access.token")
        .send()
        .await?;
    let reference_status = no_signature.status().as_u16();
    let reference_body = no_signature.bytes().await?;
    assert_coarse_401(reference_status, false, &reference_body, "no signature");
    assert_eq!(bearer.status().as_u16(), reference_status, "a bearer token");
    assert_eq!(bearer.bytes().await?, reference_body, "a bearer token");

    // Signed callers that are refused: a key nobody registered, and a request
    // addressed to another audience.
    let unregistered = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .signed_with_seed([9; 32])
        .send()
        .await?;
    let wrong_audience = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .addressed_to("https://some-other-vpay.example")
        .send()
        .await?;
    h.repositories
        .disable_client(CLIENT_B, Some("signed_transport test"))
        .await
        .context("disabling the second merchant")?;
    let disabled = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_B)
        .send()
        .await?;

    let signed_reference = unregistered.bytes().await?;
    for (what, answer) in [
        ("an unregistered key", &unregistered),
        ("the wrong audience", &wrong_audience),
        ("a disabled client", &disabled),
    ] {
        assert!(!answer.is_sealed(), "{what}");
        assert_eq!(answer.status().as_u16(), 401, "{what}");
        assert_eq!(answer.bytes().await?, signed_reference, "{what}");
    }

    h.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- case (f)

/// A client that trusts the wrong server key does not accept the answer: the
/// call reaches the server (and its effect is real), but the answer does not
/// verify under the pin and is refused.
#[tokio::test]
async fn an_answer_is_refused_by_a_client_pinning_the_wrong_server_key() -> anyhow::Result<()> {
    let h = harness().await?;

    let refused = SealedHttp::new()
        .get(h.url("/v1/customers"))
        .signed_by(CLIENT_A)
        .pinning_server_seed([3; 32])
        .send()
        .await;
    assert!(
        refused.is_err(),
        "an answer sealed under a key the client did not pin must not be returned"
    );

    // The control: the same call with the right pin is answered.
    let current = SealedHttp::new()
        .get(h.url("/v1/customers"))
        .signed_by(CLIENT_A)
        .send()
        .await?;
    assert!(current.is_sealed());

    h.shutdown().await;
    Ok(())
}
