//! `human_login` on the router (ADR-0037, VPA-3): what a deployment that serves
//! the staff sign-in mounts on `/dash/v1`, which credential it accepts there
//! beside a console key, and what a deployment that does not serve it keeps shut.
//!
//! Every case drives the real `router()` (or, where a signature has to be
//! simulated, the real `dash_scope` step) with the real validator over a real
//! JWKS served from a local HTTP server. What is replaced is only the database,
//! which a unit test has none of: the cases here are the ones a request meets
//! *before* any query, and the one that gets past every gate is shown to end at
//! the database's own error. The whole flow against Postgres (password, TOTP,
//! PKCE, token, read) is `backends/tests/integration/tests/staff_sign_in.rs`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use axum::routing::get;
use cratestack_core::VerifiedSigner;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use vpay_config::Config;

use crate::envelope::SurfaceMode;
use crate::envelope::console::{DashScopeState, DashSurface, dash_scope};
use crate::router;
use crate::staff::STAFF_ROUTES;
use crate::test_fixtures::tokens::{generate_keypair, jwks_server, mint_token, now_secs};
use crate::test_fixtures::{
    DASHBOARD_CLIENT_ID, DASHBOARD_SCOPE, TOKEN_ISSUER, config_with_auth, config_with_human_login,
    dashboard_validator, deps_for_human_login, deps_with_surfaces, lazy_repositories,
};

// ------------------------------------------------------------------ helpers --

async fn send(
    app: Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.oneshot(
        builder
            .body(Body::from(body.to_owned()))
            .expect("a request"),
    )
    .await
    .expect("router does not fail to serve")
}

async fn json_of(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the answer is small");
    serde_json::from_slice(&bytes).expect("the answer is JSON")
}

fn error_code(answer: &Value) -> Option<&str> {
    answer
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
}

/// Claims shaped as the staff authorization-code grant mints them: the staff
/// member as `sub`, the dashboard client as `aud`, the tenant claim, one scope.
fn staff_claims(sub: &str, scope: &str, merchant: Option<&str>) -> Value {
    let mut claims = json!({
        "iss": TOKEN_ISSUER,
        "aud": DASHBOARD_CLIENT_ID,
        "sub": sub,
        "scope": scope,
        "exp": now_secs() + 300,
    });
    if let (Some(merchant), Some(object)) = (merchant, claims.as_object_mut()) {
        object.insert(
            vpay_config::DASHBOARD_MERCHANT_CLAIM.to_owned(),
            json!(merchant),
        );
    }
    claims
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// The tenant the fixture dashboard client is bound to (the fixture merchant's).
const TENANT: &str = "acme-cameroon-tenant";

/// A router that serves `human_login`, whose validator fetches the JWKS the
/// returned server publishes, plus a token minter for that key.
async fn served_app() -> (Router, impl Fn(&Value) -> String, wiremock::MockServer) {
    let (encoding_key, jwk) = generate_keypair("staff-1");
    let server = jwks_server(&jwk).await;
    let config = config_with_human_login(None);
    let app = router(deps_for_human_login(
        &config,
        &format!("{}/jwks.json", server.uri()),
    ));
    let mint = move |claims: &Value| mint_token(&encoding_key, "staff-1", claims);
    (app, mint, server)
}

// ------------------------------------------------------------------- modes --

/// `/dash/v1` runs `Optional` exactly when `human_login` is **served**, not when
/// it is merely permitted: the absent-key default permits it and a deployment
/// that never wrote its blocks must stay signed-only.
#[test]
fn the_dash_mode_is_optional_iff_human_login_is_served() {
    let mode = |config: &Config| SurfaceMode::for_dashboard(config).expect("methods resolve");

    // Permitted by the default, blocks absent: not served.
    assert_eq!(mode(&config_with_auth(None)), SurfaceMode::Required);
    // Served by the default.
    assert_eq!(mode(&config_with_human_login(None)), SurfaceMode::Optional);
    assert_eq!(
        mode(&config_with_human_login(Some(&["keypair", "human_login"]))),
        SurfaceMode::Optional
    );
    // Blocks written but the method not named (refused at boot; here, resolved):
    // not served, so signed-only.
    assert_eq!(
        mode(&config_with_human_login(Some(&["keypair"]))),
        SurfaceMode::Required
    );
    // Keypair off and the sign-in served: a key is no credential, so the layer
    // is off, as `/v1`'s is for tokens alone.
    assert_eq!(
        mode(&config_with_human_login(Some(&[
            "machine_tokens",
            "human_login"
        ]))),
        SurfaceMode::Off
    );
    // Nothing on the list authenticates /dash/v1: the nests are not mounted.
    assert_eq!(
        mode(&config_with_auth(Some(&["keypair", "machine_tokens"]))),
        SurfaceMode::Required
    );
}

// --------------------------------------------------------- method off: 404s --

/// With the method off the staff routes are **not mounted**: an honest `404`
/// for every one of them (not a refusal, which would invite a caller to look for
/// a credential this deployment cannot issue), and `/dash/v1` itself is signed
/// only, so an unsigned read is the layer's coarse `401` whatever it carries.
///
/// Decisive: mount `staff::routes()` unconditionally in `management_nests` and
/// the first loop answers something other than `404`; give the dash layer
/// `Optional` unconditionally and the last request reaches the scope step, which
/// answers its own `500`.
#[tokio::test]
async fn with_the_method_off_the_staff_routes_404_and_dash_refuses_unsigned_requests() {
    // `deps_with_surfaces` serves the console (`dashboard:`) and not the
    // sign-in: both surfaces, default `auth.methods`.
    let app = router(deps_with_surfaces(None));
    for (path, methods) in STAFF_ROUTES {
        for method in *methods {
            let uri = format!("/dash/v1{path}");
            let response = send(app.clone(), method, &uri, &[], "").await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {uri} must 404 when human_login is not served"
            );
        }
    }

    let app = router(deps_with_surfaces(None));
    for header in [None, Some("Bearer abc.def.ghi"), Some("Basic dXNlcjpwdw==")] {
        let headers: Vec<(&str, &str)> = header.map(|h| ("authorization", h)).into_iter().collect();
        let response = send(app.clone(), "GET", "/dash/v1/payment_intents", &headers, "").await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{header:?}: a console that does not serve human_login takes a signature or nothing"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        assert!(
            !String::from_utf8_lossy(&bytes).contains("bearer"),
            "the coarse unsigned 401 says nothing of bearers: {header:?}"
        );
    }
}

/// And the same for the procedure transport under `/dash/v1`.
#[tokio::test]
async fn with_the_method_off_the_procedure_transport_is_signed_only() {
    let app = router(deps_with_surfaces(None));
    let response = send(
        app,
        "POST",
        "/dash/v1/$procs/searchPaymentIntents",
        &[
            ("authorization", "Bearer abc.def.ghi"),
            ("content-type", "application/json"),
        ],
        "{}",
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ------------------------------------------------------- method on: mounted --

/// With the method served every staff route is mounted: none answers the honest
/// `404`. (They answer what they answer without a database: a refused form, a
/// refused session. What each does is the integration suite's.)
#[tokio::test]
async fn with_the_method_served_the_staff_routes_are_mounted() {
    let (app, _mint, _server) = served_app().await;
    for (path, methods) in STAFF_ROUTES {
        for method in *methods {
            let uri = format!("/dash/v1{path}");
            let response = send(app.clone(), method, &uri, &[], "").await;
            assert_ne!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {uri} must be mounted when human_login is served"
            );
        }
    }
}

/// A route nobody mounted is still the honest `404` on a deployment that serves
/// the sign-in: merging the staff routes beside the console's must not change
/// what an unmatched `/dash/v1/...` path answers.
#[tokio::test]
async fn an_unknown_path_is_still_the_honest_404_when_served() {
    let (app, mint, _server) = served_app().await;
    let token = mint(&staff_claims("stf_1", DASHBOARD_SCOPE, Some(TENANT)));
    let response = send(
        app,
        "GET",
        "/dash/v1/not_a_dashboard_route",
        &[("authorization", &bearer(&token))],
        "",
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ------------------------------------------------- the session path's gates --

/// No token, or a token that does not validate, is the plain `401` of a bearer
/// caller: `Optional` lets the request through, and the scope step is the one
/// that asks who it is.
#[tokio::test]
async fn a_session_request_with_no_valid_token_is_a_plain_401() {
    let (app, mint, _server) = served_app().await;

    let none = send(app.clone(), "GET", "/dash/v1/payment_intents", &[], "").await;
    assert_eq!(none.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        error_code(&json_of(none).await),
        Some("missing_bearer_token"),
        "a missing token says so: this is not the layer's coarse unsigned 401"
    );

    // Signed by the published key but addressed to another surface: the merchant
    // audience is refused here as a staff token is refused on `/v1`.
    let mut claims = staff_claims("stf_1", DASHBOARD_SCOPE, Some(TENANT));
    if let Some(object) = claims.as_object_mut() {
        object.insert("aud".to_owned(), json!(vpay_config::MERCHANT_AUDIENCE));
    }
    let merchant_audience = mint(&claims);
    for token in ["not.a.jwt", merchant_audience.as_str()] {
        let response = send(
            app.clone(),
            "GET",
            "/dash/v1/payment_intents",
            &[("authorization", &bearer(token))],
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token}");
        assert_eq!(error_code(&json_of(response).await), Some("invalid_token"));
    }
}

/// A validly signed token for the dashboard audience that is not a staff token
/// is `403`: a `client_credentials` token carries no tenant claim, so no machine
/// client reads this surface; another merchant's claim reads nothing here; a
/// token without the registration's scope reads nothing.
#[tokio::test]
async fn a_token_that_is_not_this_tenants_staff_token_is_forbidden() {
    let (app, mint, _server) = served_app().await;
    for (label, claims) in [
        (
            "no tenant claim",
            staff_claims("some-client", DASHBOARD_SCOPE, None),
        ),
        (
            "another tenant",
            staff_claims("stf_1", DASHBOARD_SCOPE, Some("beta-tenant")),
        ),
        (
            "no scope",
            staff_claims("stf_1", "payments:write", Some(TENANT)),
        ),
    ] {
        let response = send(
            app.clone(),
            "GET",
            "/dash/v1/payment_intents",
            &[("authorization", &bearer(&mint(&claims)))],
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{label}");
    }
}

/// A method the surface does not serve never reaches a token check. There are
/// two walls, and each is shown on its own.
///
/// The envelope layer is the first: its binding lists only the methods of
/// [`crate::DASH_ROUTES`] (and `POST` for the procedure transport), so even an
/// unsigned request under `Optional` is the layer's own `405` for a write, with
/// a perfectly good token and with none. The scope step is the second, for a
/// route table a later edit widens: it refuses a method its router does not
/// serve with a `403` *before* it asks for a token, which is ADR-0008's "no
/// write reaches a handler without an audit log" held by the boundary and not
/// by a route table.
#[tokio::test]
async fn a_session_may_not_use_a_method_the_surface_does_not_serve() {
    let (app, mint, _server) = served_app().await;
    let good = mint(&staff_claims("stf_1", DASHBOARD_SCOPE, Some(TENANT)));
    let auth = bearer(&good);

    for (method, uri) in [
        ("POST", "/dash/v1/payment_intents"),
        ("DELETE", "/dash/v1/payment_intents/pi_x"),
        ("GET", "/dash/v1/$procs/searchPaymentIntents"),
    ] {
        // With a perfectly good token, and with none: the answer is the same.
        for headers in [vec![("authorization", auth.as_str())], vec![]] {
            let response = send(app.clone(), method, uri, &headers, "").await;
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {uri} (token presented: {})",
                !headers.is_empty()
            );
        }
    }

    // The second wall, with the first removed: a probe route that answers every
    // method, behind the real scope step, and no layer in front of it.
    let config = config_with_human_login(None);
    let probe = |surface: DashSurface| {
        let state = DashScopeState {
            repositories: lazy_repositories(),
            resource_config: Arc::new(
                crate::ResourceConfig::from_config(&config).expect("projects"),
            ),
            surface,
            sessions: Some(dashboard_validator("http://127.0.0.1:1/jwks.json")),
        };
        Router::new()
            .route(
                "/probe",
                get(|| async { "ok" })
                    .post(|| async { "ok" })
                    .delete(|| async { "ok" }),
            )
            .route_layer(from_fn_with_state(state, dash_scope))
    };
    for (surface, method, expected) in [
        // Not served here: refused before the token is asked for (no token was
        // sent, and the answer is not the `401` a missing token earns).
        (DashSurface::Reads, "POST", StatusCode::FORBIDDEN),
        (DashSurface::Reads, "DELETE", StatusCode::FORBIDDEN),
        (DashSurface::Procedures, "GET", StatusCode::FORBIDDEN),
        (DashSurface::Procedures, "DELETE", StatusCode::FORBIDDEN),
        // Served here: on to the token, which is missing.
        (DashSurface::Reads, "GET", StatusCode::UNAUTHORIZED),
        (DashSurface::Procedures, "POST", StatusCode::UNAUTHORIZED),
    ] {
        let response = send(probe(surface), method, "/probe", &[], "").await;
        assert_eq!(response.status(), expected, "{surface:?} {method}");
    }
}

/// A token that clears every gate reaches the staff row, and here ends at the
/// database's own error (the pool is lazy and nothing listens): the proof the
/// order is audience, tenant, scope, then the member — and that nothing before
/// the member admits the request.
#[tokio::test]
async fn a_token_that_clears_every_gate_ends_at_the_staff_lookup() {
    let (app, mint, _server) = served_app().await;
    let token = mint(&staff_claims("stf_1", DASHBOARD_SCOPE, Some(TENANT)));
    let response = send(
        app,
        "GET",
        "/dash/v1/payment_intents",
        &[("authorization", &bearer(&token))],
        "",
    )
    .await;
    assert!(
        response.status().is_server_error(),
        "an unreachable database must not read as 'the member is active': {}",
        response.status()
    );
}

// ------------------------------------------- one caller, one credential: 400 --

/// A scope step over a simulated verified signer: the layer's one effect.
fn dash_scope_app(config: &Config, with_signer: bool) -> Router {
    let state = DashScopeState {
        repositories: lazy_repositories(),
        resource_config: Arc::new(crate::ResourceConfig::from_config(config).expect("projects")),
        surface: DashSurface::Reads,
        sessions: Some(dashboard_validator("http://127.0.0.1:1/jwks.json")),
    };
    let router = Router::new()
        .route("/probe", get(|| async { "ok" }))
        .route_layer(from_fn_with_state(state, dash_scope));
    match with_signer {
        true => router.layer(from_fn(
            |mut request: Request<Body>, next: Next| async move {
                request
                    .extensions_mut()
                    .insert(VerifiedSigner::new(vec![0u8; 8], [7u8; 32], -7));
                next.run(request).await
            },
        )),
        false => router,
    }
}

/// A request that proved a key and also presents a bearer token is `400` naming
/// `authorization`, whether or not the deployment serves the sign-in. Without
/// the header the same signer proceeds to the key lookup, which here ends at the
/// database's own error: the 400 is the header's, not the signer's.
#[tokio::test]
async fn a_signature_and_a_bearer_together_are_a_400() {
    for config in [config_with_human_login(None), {
        let mut config = config_with_auth(None);
        config.dashboard = Some(crate::test_fixtures::dashboard("acme-cameroon-tenant"));
        config
    }] {
        let both = send(
            dash_scope_app(&config, true),
            "GET",
            "/probe",
            &[("authorization", "Bearer x.y.z")],
            "",
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

        let signed_only = send(dash_scope_app(&config, true), "GET", "/probe", &[], "").await;
        assert!(
            signed_only.status().is_server_error(),
            "the control: the same signer with no header goes on to the key lookup, got {}",
            signed_only.status()
        );
    }
}

/// An unsigned request on a deployment that does **not** serve `human_login` is
/// the misconfiguration `500` (the layer is not in front of this route), never a
/// pass and never a bearer check: the scope step has no session to validate.
#[tokio::test]
async fn an_unsigned_request_where_the_method_is_not_served_is_never_admitted() {
    let mut config = config_with_auth(None);
    config.dashboard = Some(crate::test_fixtures::dashboard("acme-cameroon-tenant"));
    let response = send(
        dash_scope_app(&config, false),
        "GET",
        "/probe",
        &[("authorization", "Bearer x.y.z")],
        "",
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ------------------------------------------------------------ refresh grant --

/// The dashboard client has no `refresh_token` grant, on any deployment (Q8).
///
/// At `f885170^` the staff grant never issued one (ADR-0017: "no refresh token
/// on this surface and there is not going to be one"), so there is nothing to
/// make conditional but the surface itself: the token endpoint exists only while
/// `human_login` is served, and while it does it answers a `refresh_token`
/// request with a `400` naming `grant_type`, before any code, any session or any
/// database. A keypair caller never reaches it either way.
#[tokio::test]
async fn the_refresh_token_grant_is_not_honoured_while_the_method_is_served() {
    let (app, _mint, _server) = served_app().await;
    let form = format!("grant_type=refresh_token&client_id={DASHBOARD_CLIENT_ID}&refresh_token=x");
    let response = send(
        app,
        "POST",
        "/dash/v1/oauth/token",
        &[("content-type", "application/x-www-form-urlencoded")],
        &form,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let answer = json_of(response).await;
    assert_eq!(
        answer
            .get("error")
            .and_then(|error| error.get("param"))
            .and_then(Value::as_str),
        Some("grant_type"),
        "{answer}"
    );

    // And off, the endpoint is not there at all.
    let off = send(
        router(deps_with_surfaces(None)),
        "POST",
        "/dash/v1/oauth/token",
        &[("content-type", "application/x-www-form-urlencoded")],
        &form,
    )
    .await;
    assert_eq!(off.status(), StatusCode::NOT_FOUND);
}

// -------------------------------------------------------------- discovery ----

/// The discovery document lists `human_login` exactly when it is served.
#[tokio::test]
async fn discovery_lists_human_login_only_when_both_blocks_are_present() {
    let methods = |config: Config| async move {
        let response = send(
            router(crate::test_fixtures::deps_over(&config, None)),
            "GET",
            "/.well-known/vpay-auth",
            &[],
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_of(response).await
    };
    assert_eq!(
        methods(config_with_human_login(None)).await,
        json!({ "methods": ["keypair", "machine_tokens", "hmac_webhooks", "human_login"] })
    );
    assert_eq!(
        methods(config_with_human_login(Some(&["keypair", "human_login"]))).await,
        json!({ "methods": ["keypair", "human_login"] })
    );
    assert_eq!(
        methods(config_with_auth(None)).await,
        json!({ "methods": ["keypair", "machine_tokens", "hmac_webhooks"] }),
        "permitted by the default, and not served without its blocks"
    );
}
