//! `auth.methods` on the router (ADR-0037): what each combination mounts on
//! `/v1`, which credential it accepts, and what it answers.
//!
//! Every case drives the real `router()` (or, where a signature has to be
//! opened, the real `EnvelopeLayer` and the real `merchant_scope` step) with the
//! real validator over a real JWKS served from a local HTTP server. What is
//! replaced is only the database: the replay cache and the kill switch are
//! in-memory here because a unit test has no Postgres, and the sealed
//! round trip against the real tables is
//! `backends/tests/integration/tests/merchant_token_flow.rs` and
//! `signed_transport.rs`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::Response;
use axum::routing::get;
use axum::{Router, middleware::Next};
use chrono::{DateTime, Utc};
use cratestack_core::{CratestackError, NonceStore, VerifiedSigner};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use vpay_config::{AuthMethod, Config, MERCHANT_AUDIENCE};
use vpay_db::{DbError, DisabledClients};

use crate::envelope::scope::{ScopeState, merchant_scope};
use crate::envelope::{SurfaceMode, V1Envelope};
use crate::merchant_keys::{MerchantKeyRegistry, MerchantKeyResolver};
use crate::router;
use crate::test_fixtures::tokens::{generate_keypair, jwks_server, mint_token, v1_claims};
use crate::test_fixtures::{
    PUBLIC_BASE_URL, config_with_auth, deps_over, deps_with_auth, fixture_keys, lazy_repositories,
    token_validator, v1_envelope_with_tokens,
};
use crate::test_log::{CapturedLog, captured_log_subscriber};

/// The Ed25519 seed the fixture merchant signs with.
const MERCHANT_SEED: [u8; 32] = [0x33; 32];
/// The fixture merchant's `client_id` (see `test_fixtures::config`).
const CLIENT_ID: &str = "acme-cameroon";

// ------------------------------------------------------------------ helpers --

async fn send(app: Router, method: &str, uri: &str, headers: &[(&str, &str)]) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.oneshot(builder.body(Body::empty()).expect("valid request"))
        .await
        .expect("router does not fail to serve")
}

async fn body_of(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the answer is small");
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn json_of(response: Response) -> Value {
    serde_json::from_str(&body_of(response).await).expect("the answer is JSON")
}

fn error_code(answer: &Value) -> Option<&str> {
    answer
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
}

fn content_type(response: &Response) -> String {
    response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn app_with_auth(methods: &[&str]) -> Router {
    router(deps_with_auth(methods))
}

/// A router whose `/v1` accepts tokens signed by the key a local JWKS server
/// publishes, over `methods`.
fn app_with_tokens(methods: &[&str], jwks_url: &str) -> Router {
    let config = config_with_auth(Some(methods));
    router(deps_over(
        &config,
        Some(v1_envelope_with_tokens(&config, jwks_url)),
    ))
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

const BOTH: &[&str] = &["keypair", "machine_tokens"];

// ------------------------------------------------------ keypair alone: today --

/// `[keypair]` is exactly what `/v1` was before this change: `Required`, so a
/// bearer token is not a credential and the answer is the layer's coarse `401`,
/// and the token endpoint is not mounted at all.
#[tokio::test]
async fn keypair_alone_refuses_a_bearer_with_the_layers_401_and_mounts_no_token_endpoint() {
    let response = send(
        app_with_auth(&["keypair"]),
        "GET",
        "/v1/payment_intents",
        &[("authorization", "Bearer abc.def.ghi")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_of(response).await;
    assert!(body.contains("UNAUTHORIZED"), "{body}");
    assert!(
        !body.contains("\"error\"") && !body.contains("bearer"),
        "the layer's coarse body, not the token path's: {body}"
    );

    for (method, path) in [
        ("POST", "/v1/oauth/token"),
        ("GET", "/v1/oauth/jwks.json"),
        ("GET", "/v1/oauth/.well-known/openid-configuration"),
    ] {
        assert_eq!(
            send(app_with_auth(&["keypair"]), method, path, &[])
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{method} {path} must be the honest 404 when machine_tokens is not permitted"
        );
    }
}

#[test]
fn the_envelope_mode_follows_auth_methods() {
    let mode = |methods: &[&str]| {
        SurfaceMode::from_config(&config_with_auth(Some(methods))).expect("a legal list")
    };
    assert_eq!(mode(&["keypair"]), SurfaceMode::Required);
    assert_eq!(mode(&["keypair", "hmac_webhooks"]), SurfaceMode::Required);
    assert_eq!(mode(BOTH), SurfaceMode::Optional);
    assert_eq!(mode(&["machine_tokens"]), SurfaceMode::Off);
    assert_eq!(
        SurfaceMode::from_config(&config_with_auth(None)).expect("absent"),
        SurfaceMode::Optional,
        "the absent key permits both ways in"
    );
    assert!(
        SurfaceMode::from_config(&config_with_auth(Some(&["hmac_webhooks"]))).is_err(),
        "no way in is not a mode"
    );
}

// ------------------------------------------------- keypair + machine_tokens --

/// The valid token reaches the handler (`503`: the fixture's database is a pool
/// that never connects, which is what proves the handler ran) and is answered
/// in plain JSON, not as a COSE message.
#[tokio::test]
async fn both_ways_a_valid_token_reaches_the_handler_and_is_answered_in_plain_json() {
    let log = CapturedLog::default();
    let _subscriber = tracing::subscriber::set_default(captured_log_subscriber(log.clone()));
    let (encoding_key, jwk) = generate_keypair("tok-1");
    let jwks = jwks_server(&jwk).await;
    let token = mint_token(
        &encoding_key,
        "tok-1",
        &v1_claims(CLIENT_ID, "payments:write"),
    );
    let response = send(
        app_with_tokens(BOTH, &format!("{}/jwks.json", jwks.uri())),
        "GET",
        "/v1/payment_intents/pi_x",
        &[("authorization", &bearer(&token))],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a valid token must get past the scope step to the (database-less) handler"
    );
    assert!(
        content_type(&response).starts_with("application/json"),
        "a token caller's answer is plain JSON: {}",
        content_type(&response)
    );
    let answer = json_of(response).await;
    assert!(
        answer.get("error").is_some(),
        "the Stripe envelope: {answer}"
    );
    assert!(
        log.contents().contains(r#"auth.method="machine_token""#),
        "the request span names how the caller authenticated: {}",
        log.contents()
    );
}

/// Every way a bearer token can fail on a deployment that accepts them, and the
/// answer each gets. None reaches the handler.
#[tokio::test]
async fn both_ways_a_token_that_does_not_earn_the_request_is_refused_plainly() {
    let (encoding_key, jwk) = generate_keypair("tok-1");
    let jwks = jwks_server(&jwk).await;
    let url = format!("{}/jwks.json", jwks.uri());
    let sign = |claims: Value| mint_token(&encoding_key, "tok-1", &claims);

    let wrong_audience = json!({
        "iss": crate::test_fixtures::TOKEN_ISSUER,
        "aud": crate::mcp::merchant_resource(PUBLIC_BASE_URL),
        "sub": CLIENT_ID,
        "scope": "payments:write",
        "exp": crate::test_fixtures::tokens::now_secs() + 300,
    });

    for (what, token, method, path, expected_status, expected_code) in [
        (
            "a token for a client this deployment never registered",
            Some(sign(v1_claims("someone-else", "payments:write"))),
            "GET",
            "/v1/payment_intents/pi_x",
            StatusCode::FORBIDDEN,
            "forbidden",
        ),
        (
            "a read-only token on a write",
            Some(sign(v1_claims(CLIENT_ID, "payments:read"))),
            "POST",
            "/v1/payment_intents",
            StatusCode::FORBIDDEN,
            "forbidden",
        ),
        (
            "a token for another audience",
            Some(sign(wrong_audience)),
            "GET",
            "/v1/payment_intents/pi_x",
            StatusCode::UNAUTHORIZED,
            "invalid_token",
        ),
        (
            "garbage",
            Some("not.a.jwt".to_owned()),
            "GET",
            "/v1/payment_intents/pi_x",
            StatusCode::UNAUTHORIZED,
            "invalid_token",
        ),
        (
            "no credential at all",
            None,
            "GET",
            "/v1/payment_intents/pi_x",
            StatusCode::UNAUTHORIZED,
            "missing_bearer_token",
        ),
    ] {
        let value = token.as_deref().map(bearer);
        let headers: Vec<(&str, &str)> = value
            .as_deref()
            .map(|header| ("authorization", header))
            .into_iter()
            .collect();
        let response = send(app_with_tokens(BOTH, &url), method, path, &headers).await;
        assert_eq!(response.status(), expected_status, "{what}");
        let answer = json_of(response).await;
        assert_eq!(error_code(&answer), Some(expected_code), "{what}: {answer}");
    }
}

/// A read token may read, and may not write: `required_scopes` is the same
/// table the signed path uses.
#[tokio::test]
async fn both_ways_a_read_token_may_read() {
    let (encoding_key, jwk) = generate_keypair("tok-1");
    let jwks = jwks_server(&jwk).await;
    let token = mint_token(
        &encoding_key,
        "tok-1",
        &v1_claims(CLIENT_ID, "payments:read"),
    );
    let response = send(
        app_with_tokens(BOTH, &format!("{}/jwks.json", jwks.uri())),
        "GET",
        "/v1/payment_intents/pi_x",
        &[("authorization", &bearer(&token))],
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// `Optional` must not downgrade a signed request that fails to verify: a
/// message that claims to be COSE is opened or refused exactly as `Required`
/// refuses it, never handed to the bearer path (where a valid bearer token on
/// the same request would otherwise rescue it).
#[tokio::test]
async fn both_ways_a_forged_signed_request_is_refused_as_required_refuses_it() {
    let (encoding_key, jwk) = generate_keypair("tok-1");
    let jwks = jwks_server(&jwk).await;
    let token = mint_token(
        &encoding_key,
        "tok-1",
        &v1_claims(CLIENT_ID, "payments:write"),
    );
    let forged = |app: Router| async {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/payment_intents/pi_x")
            .header("content-type", "application/cose; cose-type=\"cose-sign1\"")
            .header("authorization", bearer(&token))
            .body(Body::from(vec![0xd2, 0x84, 0x40, 0xa0, 0x40, 0x40]))
            .expect("valid request");
        let response = app.oneshot(request).await.expect("served");
        (response.status(), body_of(response).await)
    };
    let both = forged(app_with_tokens(BOTH, &format!("{}/jwks.json", jwks.uri()))).await;
    let required = forged(app_with_auth(&["keypair"])).await;
    assert_eq!(both, required, "Optional must refuse what Required refuses");
    assert_ne!(
        both.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "a forged signature must not fall through to a bearer-authenticated handler"
    );
    assert!(
        both.0.is_client_error(),
        "refused as the client's problem: {both:?}"
    );
}

/// A method the table does not serve on a `/v1` path is the layer's own `405`
/// with `Allow`, under `Optional` as under `Required`: the policy is a closure
/// with `unresolved_mode` pinned back to `Required`. With a bare
/// `EnvelopeMode::Optional` the request would pass through to the scope step
/// and be answered `401` for want of a bearer token, and a wrong method on a
/// route the resolver could not bind would run unsigned.
#[tokio::test]
async fn optional_keeps_unserved_methods_a_405_from_the_layer() {
    for methods in [&["keypair"][..], BOTH] {
        let response = send(
            app_with_tokens(methods, "http://127.0.0.1:1/jwks.json"),
            "DELETE",
            "/v1/payment_intents",
            &[],
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{methods:?}"
        );
        assert!(
            response.headers().contains_key("allow"),
            "{methods:?}: the layer's 405 names what the path serves"
        );
    }
}

// ------------------------------------------------------ machine_tokens alone --

/// A legacy-only deployment: the envelope is `Off`, so a COSE body is refused
/// (`415`) rather than forwarded unverified, and a bearer token is the way in.
#[tokio::test]
async fn tokens_alone_run_the_envelope_off() {
    let (encoding_key, jwk) = generate_keypair("tok-1");
    let jwks = jwks_server(&jwk).await;
    let url = format!("{}/jwks.json", jwks.uri());
    let methods = ["machine_tokens"];

    let cose = send(
        app_with_tokens(&methods, &url),
        "POST",
        "/v1/payment_intents",
        &[("content-type", "application/cose; cose-type=\"cose-sign1\"")],
    )
    .await;
    assert_eq!(cose.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let token = mint_token(
        &encoding_key,
        "tok-1",
        &v1_claims(CLIENT_ID, "payments:write"),
    );
    let ok = send(
        app_with_tokens(&methods, &url),
        "GET",
        "/v1/payment_intents/pi_x",
        &[("authorization", &bearer(&token))],
    )
    .await;
    assert_eq!(ok.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(content_type(&ok).starts_with("application/json"));

    let none = send(
        app_with_tokens(&methods, &url),
        "GET",
        "/v1/payment_intents/pi_x",
        &[],
    )
    .await;
    assert_eq!(none.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        error_code(&json_of(none).await).map(str::to_owned),
        Some("missing_bearer_token".to_owned())
    );
}

/// A router assembled with `machine_tokens` permitted and no validator fails
/// every unsigned request with a `500` instead of letting one through.
#[tokio::test]
async fn tokens_permitted_but_no_validator_wired_refuses_everything() {
    let config = config_with_auth(Some(BOTH));
    let bare = V1Envelope::new(&config, &fixture_keys(), lazy_repositories()).expect("builds");
    let app = router(deps_over(&config, Some(bare)));
    let response = send(
        app,
        "GET",
        "/v1/payment_intents/pi_x",
        &[("authorization", "Bearer x.y.z")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ------------------------------------------------------------- the scope step --

struct NobodyDisabled;

#[async_trait]
impl DisabledClients for NobodyDisabled {
    async fn is_client_disabled(&self, _: &str) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn disable_client(&self, _: &str, _: Option<&str>) -> Result<(), DbError> {
        Ok(())
    }
    async fn enable_client(&self, _: &str) -> Result<(), DbError> {
        Ok(())
    }
}

#[derive(Default)]
struct MemoryNonces(Mutex<HashSet<String>>);

#[async_trait]
impl NonceStore for MemoryNonces {
    async fn record_if_unseen(
        &self,
        nonce: &str,
        _expires_at: DateTime<Utc>,
    ) -> Result<bool, CratestackError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| CratestackError::Internal("poisoned".to_owned()))?
            .insert(nonce.to_owned()))
    }
}

fn merchant_public_key() -> [u8; 32] {
    cratestack_cose::Ed25519Signer::from_seed(&MERCHANT_SEED)
        .verify_key()
        .ed25519_bytes()
        .expect("an Ed25519 key")
}

/// The fixture config with the merchant's signing key registered.
fn config_with_merchant_key(methods: &[&str]) -> Config {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let mut config = config_with_auth(Some(methods));
    for merchant in &mut config.merchant_clients {
        merchant.cose_keys = vec![vpay_config::CoseKey {
            public_key: URL_SAFE_NO_PAD.encode(merchant_public_key()),
            label: None,
        }];
    }
    config
}

fn merchants_for(config: &Config) -> Arc<MerchantKeyResolver> {
    Arc::new(MerchantKeyResolver::new(
        Arc::new(MerchantKeyRegistry::from_config(config).expect("the keys parse")),
        Arc::new(NobodyDisabled),
    ))
}

async fn ok_handler() -> &'static str {
    "ok"
}

/// The scope step alone, with the layer's one effect (recording the verified
/// signer) simulated by a middleware: the point is what the step does with a
/// signer, a bearer, or both.
fn scope_app(config: &Config, with_signer: bool) -> Router {
    let state = ScopeState {
        merchants: merchants_for(config),
        machine_tokens: true,
        validator: Some(token_validator("http://127.0.0.1:1/jwks.json")),
        resource_config: Arc::new(crate::ResourceConfig::from_config(config).expect("projects")),
    };
    let router = Router::new()
        .route("/probe", get(ok_handler))
        .route_layer(from_fn_with_state(state, merchant_scope));
    match with_signer {
        true => {
            let thumbprint = cratestack_cose::CoseVerifyKey::ed25519(&merchant_public_key())
                .expect("a point")
                .thumbprint();
            router.layer(from_fn(
                move |mut request: Request<Body>, next: Next| async move {
                    request.extensions_mut().insert(VerifiedSigner::new(
                        vec![0u8; 8],
                        thumbprint,
                        -19,
                    ));
                    next.run(request).await
                },
            ))
        }
        false => router,
    }
}

/// One caller, one credential: a request that proved a key and also presents a
/// bearer token is `400`, and the same signed request without the header runs.
#[tokio::test]
async fn a_signature_and_a_bearer_together_are_a_400() {
    let config = config_with_merchant_key(BOTH);

    let both = send(
        scope_app(&config, true),
        "GET",
        "/probe",
        &[("authorization", "Bearer x.y.z")],
    )
    .await;
    assert_eq!(both.status(), StatusCode::BAD_REQUEST);
    let answer = json_of(both).await;
    assert_eq!(
        answer
            .get("error")
            .and_then(|error| error.get("param"))
            .and_then(Value::as_str),
        Some("authorization"),
        "{answer}"
    );

    let signed_only = send(scope_app(&config, true), "GET", "/probe", &[]).await;
    assert_eq!(
        signed_only.status(),
        StatusCode::OK,
        "the control: the same signer, no header, resolves through the key registry"
    );
}

/// A signer whose key no merchant holds is `403`, as before.
#[tokio::test]
async fn a_signer_no_merchant_holds_is_forbidden() {
    let config = config_with_auth(Some(BOTH));
    let response = send(scope_app(&config, true), "GET", "/probe", &[]).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ------------------------------------------------------------- the real thing --

/// The same router answers a sealed request sealed and a token request plain.
///
/// A real listener, the real SDK sealing the request, the real `EnvelopeLayer`
/// opening it (against an in-memory replay cache), and the real validator over
/// a real JWKS. The database is the one thing missing, so both calls end in the
/// handler's `503`: what differs is how that answer arrives. The SDK refuses an
/// answer that is not a message sealed under the pinned server key
/// (`Error::Unsigned`), so an `Error::Api` is the proof it was sealed.
#[tokio::test]
async fn the_same_router_seals_for_a_signer_and_answers_a_token_in_plain_json() {
    use vpay_sdk::{Client, Credentials, Error, ServerKeys};

    let log = CapturedLog::default();
    let _subscriber = tracing::subscriber::set_default(captured_log_subscriber(log.clone()));
    let (encoding_key, jwk) = generate_keypair("tok-1");
    let jwks = jwks_server(&jwk).await;
    let config = config_with_merchant_key(BOTH);
    let envelope = V1Envelope::with_replay_cache(
        &config,
        &fixture_keys(),
        merchants_for(&config),
        Arc::new(MemoryNonces::default()),
    )
    .expect("builds")
    .with_machine_tokens(token_validator(&format!("{}/jwks.json", jwks.uri())));
    let app = router(deps_over(&config, Some(envelope)));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds");
    let address = listener.local_addr().expect("bound");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let public_of = |seed: u8| {
        cratestack_cose::Ed25519Signer::from_seed(&[seed; 32])
            .verify_key()
            .ed25519_bytes()
            .expect("an Ed25519 key")
    };
    let client = Client::builder(format!("http://{address}"))
        .audience(PUBLIC_BASE_URL)
        .credentials(Credentials::ed25519_seed(CLIENT_ID, &MERCHANT_SEED).expect("a seed"))
        .server_keys(ServerKeys::new(public_of(0x11), public_of(0x22)))
        .build()
        .expect("the SDK client builds");

    match client.payment_intents().retrieve("pi_x").await {
        Err(Error::Api { status, .. }) => assert_eq!(status, 503, "the handler's own answer"),
        other => panic!("a sealed request must get a sealed answer, got {other:?}"),
    }
    assert!(
        log.contents().contains(r#"auth.method="keypair""#),
        "the request span names how the caller authenticated: {}",
        log.contents()
    );

    let token = mint_token(
        &encoding_key,
        "tok-1",
        &v1_claims(CLIENT_ID, "payments:write"),
    );
    let plain = crate::http_client::client()
        .expect("the vendored-roots client builds")
        .get(format!("http://{address}/v1/payment_intents/pi_x"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("answers");
    assert_eq!(plain.status().as_u16(), 503);
    let content_type = plain
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("application/json"),
        "a token caller gets JSON, not COSE: {content_type}"
    );

    server.abort();
}

// ------------------------------------------------------------------ discovery --

#[tokio::test]
async fn discovery_lists_the_methods_a_deployment_serves() {
    let methods = |config: Config| {
        let app = router(deps_over(&config, None));
        async move {
            let response = send(app, "GET", "/.well-known/vpay-auth", &[]).await;
            assert_eq!(response.status(), StatusCode::OK);
            json_of(response).await
        }
    };
    assert_eq!(
        methods(config_with_auth(Some(&["keypair"]))).await,
        json!({ "methods": ["keypair"] })
    );
    assert_eq!(
        methods(config_with_auth(Some(&[
            "keypair",
            "machine_tokens",
            "hmac_webhooks"
        ])))
        .await,
        json!({ "methods": ["keypair", "machine_tokens", "hmac_webhooks"] })
    );
    assert_eq!(
        methods(config_with_auth(None)).await,
        json!({ "methods": ["keypair", "machine_tokens", "hmac_webhooks"] }),
        "the default permits human_login and no release serves it yet"
    );
}

/// Outside every envelope and every credential, and secret-free by shape.
#[tokio::test]
async fn discovery_needs_no_credential() {
    let response = send(
        app_with_auth(&["keypair"]),
        "GET",
        "/.well-known/vpay-auth",
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(content_type(&response).starts_with("application/json"));
}

/// The discovery vocabulary is `AuthMethod`'s own.
#[test]
fn the_discovery_names_are_the_yaml_spellings() {
    let names: Vec<&str> = AuthMethod::ALL
        .into_iter()
        .map(AuthMethod::as_str)
        .collect();
    assert_eq!(
        names,
        ["keypair", "machine_tokens", "hmac_webhooks", "human_login"]
    );
    assert_eq!(MERCHANT_AUDIENCE, "vpay:v1");
}
