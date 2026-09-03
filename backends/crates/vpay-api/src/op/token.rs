//! The two HTTP handlers vpay writes itself for `/v1/oauth`: the RFC 6749
//! token endpoint and the metadata document that tells a merchant where it is.
//!
//! **This endpoint renders RFC 6749 §5.2's body, not [`crate::ApiError`]'s
//! Stripe envelope** — every OAuth client in existence, including vpay's own
//! SDKs, parses `{"error":…,"error_description":…}`. [`token_handler`] is a
//! port of `authkestra-axum-0.7.1/src/op.rs::axum_token_handler` with three
//! deliberate omissions (no DPoP, no mTLS binding, no device/authorize/userinfo
//! route).
//!
//! Why the port exists, the inlined reference copy to re-diff a bump against,
//! and each omission's reasoning:
//! [docs/reference/vpay-api.md § the merchant OP](../../../../../docs/reference/vpay-api.md#the-merchant-op-op).

use std::sync::Arc;

use authkestra_op::handlers::token::{TokenRequest, handle_token};
use axum::Json;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use crate::op::{
    MerchantOp, OP_ASSERTION_SIGNING_ALGS, OP_GRANT_TYPES, OP_TOKEN_ENDPOINT_AUTH_METHOD,
};

/// `POST {issuer}/token` — the one place a merchant turns a
/// `client_assertion` into a `/v1` access token.
///
/// Unauthenticated in the bearer-token sense: the credential *is* the
/// request body (RFC 7523 §2.2), so a router that required a bearer token
/// here would be circular. The credential is checked by
/// `authkestra_op::handlers::token::authenticate_client`, which resolves the
/// client through [`crate::op::clients::YamlClientStore`] (so the
/// `disabled_clients` kill switch applies) and spends the assertion's `jti`
/// through [`vpay_db::SqlClientAssertionStore`] (so it is single-use).
///
/// `Form<TokenRequest>` because RFC 6749 §4.4.2 fixes the request encoding
/// as `application/x-www-form-urlencoded`; a JSON body is not an alternative
/// spelling of it, and axum answers a mismatched content type with its own
/// 415 before this handler runs. `TokenRequest` is `#[non_exhaustive]` but
/// `Deserialize`, so this is the only way to build one from a request.
///
/// The `Authorization` header is forwarded because RFC 6749 §2.3.1 also
/// permits `client_secret_basic` there. vpay accepts no secret-based method
/// (see [`OP_TOKEN_ENDPOINT_AUTH_METHOD`]), so a `Basic` credential is
/// *refused* rather than ignored — `authenticate_client` treats a
/// registration's `token_endpoint_auth_method` as an exclusive binding, and
/// a credential of the wrong kind is `invalid_client`, never a fallback.
/// Forwarding it is what makes that refusal happen instead of the request
/// looking like it presented no credential at all.
///
/// A non-ASCII `Authorization` header is passed as `None` (`to_str().ok()`),
/// exactly as `axum_token_handler` does: such a value cannot be any
/// credential this endpoint accepts, and the request then fails as
/// "no credential presented", which is the same `invalid_client` answer.
pub async fn token_handler(
    State(op): State<Arc<MerchantOp>>,
    headers: HeaderMap,
    Form(mut request): Form<TokenRequest>,
) -> Response {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    // RFC 6749 §3.3: a request that names no scope is granted this
    // deployment's "locally defined default", which is the client's own
    // registered `scopes:` — see `MerchantOp::default_scope_for`. Applied
    // here, before `handle_token`, because `handle_client_credentials`
    // treats an absent `scope` as "grant none" and there is no seam inside
    // it to hook.
    //
    // This only ever *fills in* an omitted value. A request that names a
    // scope keeps it verbatim, including a deliberately narrower one, and
    // `handle_client_credentials` still refuses anything outside the
    // registration with `invalid_scope`.
    if request.scope.is_none()
        && let Some(client_id) = default_scope_client_id(&request)
        && let Some(default) = op.default_scope_for(&client_id)
    {
        request.scope = Some(default.to_owned());
    }

    match handle_token(request, auth_header, op.config(), op.store(), op.tokens()).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => (token_error_status(&error.error), Json(error)).into_response(),
    }
}

/// Which client's registered scopes to use as this request's default scope.
///
/// The `client_id` form field when there is one. When there is not, the
/// `sub` of the `client_assertion`, **read without verifying anything** —
/// and that needs saying, because it looks exactly like the mistake it is
/// not.
///
/// # Why the unverified read is safe here, and nowhere else
///
/// RFC 7523 §3 lets a `private_key_jwt` client authenticate with the
/// assertion alone: `client_id` is redundant with the assertion's `sub`, and
/// a conformant client may omit it. `authkestra_op::handlers::token`
/// resolves such a request from the assertion perfectly well — but
/// `token_handler` ran first and, keyed on the form field alone, handed it
/// no default scope. The result was a token with no `scope` claim, an
/// authenticated client, and a `403` on every `/v1` call, for a client whose
/// registration lists exactly the scope it was refused for. That is a real
/// integration this deployment accepts, so the default has to be found for
/// it too.
///
/// What comes back is used for **one** thing: choosing which registration's
/// `scopes:` to copy into an omitted `scope` parameter. It authenticates
/// nothing and authorises nothing. `handle_token` then verifies the
/// assertion's signature, issuer, audience, expiry and `jti` against the
/// registration it resolves for itself, and `handle_client_credentials`
/// checks the resulting scope against *that* client's registration — so a
/// forged `sub` naming a better-privileged client selects a default for a
/// request that is then refused `invalid_client`, and cannot select a scope
/// the authenticated client is not registered for. Never widen this: the
/// moment an unverified claim decides anything else, it is a credential.
fn default_scope_client_id(request: &TokenRequest) -> Option<String> {
    if let Some(client_id) = request.client_id.as_deref() {
        return Some(client_id.to_owned());
    }

    // Deliberately not `jsonwebtoken::decode` with
    // `insecure_disable_signature_validation()`: a decode call configured to
    // skip its checks reads, at a glance, like a decode call that performed
    // them. Splitting the payload segment out by hand cannot be mistaken for
    // verification by anybody.
    let payload = request.client_assertion.as_deref()?.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("sub").and_then(Value::as_str).map(str::to_owned)
}

/// RFC 6749 §5.2's status mapping, as `authkestra-axum` implements it:
/// `invalid_client` is the only code that answers 401, every other code
/// answers 400.
///
/// Split out as a named function of the code alone so the mapping is
/// testable without a store, a signer or a request — and so that the one
/// case that matters to `sdks/rust` (a disabled or unknown client must be a
/// 401, because that is what its `401` re-authentication path keys on) is
/// pinned by a test rather than by reading the match arm.
///
/// The RFC actually says a 401 "MAY" be used for `invalid_client` and
/// requires `WWW-Authenticate` only when the request used an HTTP
/// authentication scheme. vpay's clients authenticate with a body parameter
/// (`client_assertion`), never with `WWW-Authenticate`-negotiated HTTP auth,
/// so no challenge header is emitted — matching `axum_token_handler`, and
/// matching what both SDKs expect.
fn token_error_status(code: &str) -> StatusCode {
    match code {
        "invalid_client" => StatusCode::UNAUTHORIZED,
        _ => StatusCode::BAD_REQUEST,
    }
}

/// `GET {issuer}/.well-known/openid-configuration` — the metadata document.
///
/// Served at the OpenID Connect Discovery path rather than RFC 8414's
/// `/.well-known/oauth-authorization-server` because that is the path
/// `authkestra_op::config::OpConfig::discovery_url` names, the path the
/// authkestra ecosystem's clients look at, and the path
/// `docs/flows/merchant-auth.md` documents. The *content* is OAuth 2.0
/// authorization-server metadata (RFC 8414 §2): `/v1` is not an OpenID
/// Provider — it has no end user, issues no `id_token`, and supports no
/// `openid` scope.
///
/// Unauthenticated, like the token endpoint and `/jwks.json`: this is how a
/// client that has never spoken to vpay learns where to go.
pub async fn discovery_handler(State(op): State<Arc<MerchantOp>>) -> impl IntoResponse {
    Json(discovery_document(&op))
}

/// Builds the metadata document by hand rather than through
/// `authkestra_op::handlers::discovery::OidcDiscovery::from_config`.
///
/// `from_config` was read (`authkestra-op-0.7.1/src/handlers/discovery.rs`)
/// and rejected: for a provider whose only grant is `client_credentials` it
/// still emits four things vpay does not serve, and a metadata document that
/// promises a route which 404s is worse than one that stays quiet — the same
/// principle authkestra applies to its own `with_private_key_jwt` and
/// `with_dpop_support` opt-ins.
///
/// - `userinfo_endpoint: Some({issuer}/userinfo)` — emitted
///   *unconditionally*, and vpay mounts no userinfo route. This is the
///   decisive one: it is a URL a conformant client will call and get vpay's
///   404 envelope from.
/// - `token_endpoint_auth_methods_supported: ["client_secret_basic",
///   "client_secret_post", "none"]` — vpay accepts none of the three
///   (`private_key_jwt` only, and it must be opted into with
///   `.with_private_key_jwt()`). Advertising `none` in particular would say
///   this token endpoint serves unauthenticated public clients.
/// - `claims_supported: [… "name", "email"]` — vpay mints no claim about a
///   person; there is no person in `client_credentials`.
/// - `response_modes_supported: ["query"]` — a property of an authorization
///   endpoint that does not exist.
///
/// Cheaper alternatives were considered and are worse: post-processing
/// `from_config`'s output would mean deleting fields by name from a
/// `#[non_exhaustive]` struct (so a future authkestra field arrives
/// advertised by default, silently), and `OidcDiscovery`'s fields are typed
/// as required `String`/`Vec` rather than `Option`, so several cannot be
/// removed at all. Building the object here means every member is one this
/// deployment can answer for. The cost is that an authkestra bump does not
/// bring new metadata with it, which for a document describing *vpay's*
/// surface is the right default.
///
/// Every value comes from [`MerchantOp`] or from the constants in
/// [`crate::op`], never from a literal duplicated here, so the document and
/// the running configuration cannot drift.
fn discovery_document(op: &MerchantOp) -> Value {
    json!({
        "issuer": op.issuer(),
        "token_endpoint": op.token_endpoint(),
        "jwks_uri": op.jwks_url(),
        "grant_types_supported": OP_GRANT_TYPES,
        // RFC 8414 §2 lists this as REQUIRED. Empty is the truthful value:
        // there is no authorization endpoint, so there is no response type.
        "response_types_supported": op.config().response_types_supported,
        "scopes_supported": op.config().scopes_supported,
        "token_endpoint_auth_methods_supported": [OP_TOKEN_ENDPOINT_AUTH_METHOD],
        // REQUIRED by RFC 8414 §2 once `private_key_jwt` is advertised. See
        // `OP_ASSERTION_SIGNING_ALGS` for why this is a union across key
        // types rather than a single algorithm.
        "token_endpoint_auth_signing_alg_values_supported": OP_ASSERTION_SIGNING_ALGS,
    })
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use tower::ServiceExt as _;

    use super::*;
    use crate::test_fixtures::merchant_op;

    /// Serves the discovery route on a router built exactly as
    /// `crate::router` builds it — a real axum request in, a real response
    /// out — against a pool that has never connected, because discovery
    /// reads no table.
    ///
    /// The token endpoint is deliberately **not** exercised here: every path
    /// through it resolves a client, which reads `disabled_clients`, and one
    /// spends a `jti`, which writes `client_assertion_jtis`. A unit test
    /// against a lazy pool could only ever observe the `server_error` a
    /// failed database lookup produces, which proves nothing about the
    /// endpoint's contract. The real cases — a token minted, a disabled
    /// client refused as `invalid_client`/401, a replayed assertion refused
    /// — are
    /// `backends/tests/integration/tests/merchant_token_flow.rs`, against a
    /// real Postgres.
    async fn get_discovery() -> (StatusCode, Value) {
        let app = Router::new()
            .route(
                "/v1/oauth/.well-known/openid-configuration",
                get(discovery_handler),
            )
            .with_state(merchant_op());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/oauth/.well-known/openid-configuration")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("the discovery document is small");
        (
            status,
            serde_json::from_slice(&bytes).expect("the discovery document is JSON"),
        )
    }

    /// The three URLs a merchant needs, and the fact that they agree with
    /// what `sdks/rust` derives from a base URL on its own.
    #[tokio::test]
    async fn discovery_publishes_the_endpoints_the_sdk_would_have_guessed() {
        let (status, document) = get_discovery().await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            document.get("issuer").and_then(Value::as_str),
            Some("https://api.vpay.test/v1/oauth")
        );
        assert_eq!(
            document.get("token_endpoint").and_then(Value::as_str),
            Some("https://api.vpay.test/v1/oauth/token")
        );
        assert_eq!(
            document.get("jwks_uri").and_then(Value::as_str),
            Some("https://api.vpay.test/v1/oauth/jwks.json")
        );
    }

    /// The document must not name a route vpay does not mount. This is the
    /// decisive test for building the document by hand: every one of these
    /// keys is emitted by `OidcDiscovery::from_config`, and
    /// `userinfo_endpoint` in particular is emitted with a URL that would
    /// 404.
    #[tokio::test]
    async fn discovery_advertises_no_endpoint_this_deployment_does_not_serve() {
        let (_status, document) = get_discovery().await;

        for absent in [
            "userinfo_endpoint",
            "authorization_endpoint",
            "device_authorization_endpoint",
            "revocation_endpoint",
            "registration_endpoint",
            "introspection_endpoint",
        ] {
            assert!(
                document.get(absent).is_none(),
                "the discovery document must not advertise {absent}, which vpay does not mount; \
                 got {document:#}"
            );
        }
    }

    /// `private_key_jwt` and nothing else. A document listing
    /// `client_secret_basic` or `none` would tell a merchant they may
    /// authenticate with a secret vpay refuses to store, or with nothing at
    /// all.
    #[tokio::test]
    async fn discovery_advertises_only_private_key_jwt() {
        let (_status, document) = get_discovery().await;

        assert_eq!(
            document.get("token_endpoint_auth_methods_supported"),
            Some(&json!(["private_key_jwt"]))
        );
        assert_eq!(
            document.get("grant_types_supported"),
            Some(&json!(["client_credentials"]))
        );
        assert_eq!(
            document.get("response_types_supported"),
            Some(&json!([])),
            "an empty array, not a missing key: RFC 8414 lists it as REQUIRED"
        );
        assert_eq!(
            document.get("scopes_supported"),
            Some(&json!(["payments:write"]))
        );
    }

    /// The status mapping `sdks/rust` keys its re-authentication on. A
    /// `401` from the token endpoint means "your credential was refused";
    /// anything else about the *request* is a `400`. Written as explicit
    /// pairs rather than as a loop over the match, so collapsing the match
    /// into one arm fails here.
    #[test]
    fn only_invalid_client_answers_401() {
        assert_eq!(
            token_error_status("invalid_client"),
            StatusCode::UNAUTHORIZED
        );
        for code in [
            "invalid_request",
            "invalid_grant",
            "invalid_scope",
            "invalid_target",
            "unauthorized_client",
            "unsupported_grant_type",
            "server_error",
        ] {
            assert_eq!(
                token_error_status(code),
                StatusCode::BAD_REQUEST,
                "{code} must not be answered 401: an SDK treats a 401 from the token endpoint as \
                 a credential problem and will not retry it as a request problem"
            );
        }
    }
}
