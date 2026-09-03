//! The Stripe-shaped HTTP surface.
//!
//! STATUS: `/healthz`, the `/v1/oauth` merchant OP ([`op`]), the
//! authentication boundary in front of `/v1`, `/v1/payment_intents`
//! ([`v1::payment_intents`]) and the Stripe-shaped 404 envelope are
//! implemented. What is *not*: no rail adapter implements `submit`, so a
//! `confirm` reaches the rail, records the attempt, and answers the
//! documented `501 not_implemented` — a real answer, not a fabricated
//! success. Every other `/v1` resource an SDK can name (`/v1/refunds`,
//! `/v1/balance`, `/v1/events`) is routed nowhere and answers the honest
//! 404 from the nest's fallback. See `docs/status.md`. This file must never
//! grow a route that returns fabricated data; a real database check (below),
//! a real 404 and a real 501 are the opposite of fabricated data, so they
//! stay.
//!
//! [`resource_auth`] supplies the bearer-token validation now mounted in
//! front of `/v1` — see [`router`]'s "Route tree" section for exactly which
//! paths sit inside and outside it. `/dash/v1` is still mounted nowhere:
//! that surface needs the dashboard OIDC login flow, which is later work.
//!
//! [`ApiError`] is this layer's Tier-2 composite error
//! ([ADR-0011](../../../docs/adr/0011-error-modelling.md)). Every failure
//! response in this crate is rendered by its `IntoResponse` — including the
//! 404 fallback below — so a handler returns `Result<_, ApiError>` and never
//! picks a status or writes a merchant-facing sentence. The one deliberate
//! exception is `/healthz`, which answers plain text for the reasons given
//! in [`error`]'s module docs.
//!
//! [`router`] mounts a four-layer middleware stack — a caller-supplied id
//! vetted, request id in, span around the handler, request id back out —
//! described on that function. It is the mechanism `error`'s "No
//! `request_id` field here, deliberately" section defers to, and it is what
//! makes `Category::Internal`'s "Contact support with the request id"
//! something a merchant can actually do.

use std::borrow::Cow;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRef, State};
use axum::http::{Method, Request, Uri};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::{Router, http::StatusCode, routing::get, routing::post};
use serde_json::{Map, Value, json};
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::Span;
use vpay_db::PgPool;
use vpay_provider::ProviderAdapter;

pub mod error;
pub mod form;
pub mod idempotency;
mod jwks_cache;
pub mod model;
pub mod op;
pub mod resource_auth;
#[cfg(test)]
mod test_fixtures;
#[cfg(test)]
mod test_log;
pub mod v1;

/// The outbound HTTP client, which now lives in the port crate.
///
/// It was `vpay_api::http_client` (its own module here) until Step 3, and
/// moved to [`vpay_provider::http`] when the rail adapters needed it: an
/// adapter depends on the port and must never depend on this crate, so the
/// only home both can reach is `vpay-provider`. Re-exported under the old
/// name rather than renamed at every call site — `resource_auth` and
/// `jwks_cache` still spell `crate::http_client::client`, and a mechanical
/// rename across them would have buried the one-line move in noise.
pub use vpay_provider::http as http_client;

pub use error::ApiError;
pub use resource_auth::MerchantJwtValidator;
pub use v1::{
    MerchantScope, ResourceConfig, SCOPE_PAYMENTS_READ, SCOPE_PAYMENTS_WRITE, V1_ROUTES, V1Route,
    required_scopes,
};

use resource_auth::extract_bearer_token;

/// The header carrying the request id, in and out.
///
/// `x-request-id` rather than a vpay-specific name: it is what tower-http's
/// `x_request_id` constructors use, what a reverse proxy in front of this
/// process is most likely to set already (and which the layer below then
/// honours rather than overwriting — within the bounds
/// [`discard_unusable_request_id`] enforces), and what a merchant's own
/// tooling is most likely to already log.
///
/// The single source of the name: the vetting step, the two tower-http
/// layers and [`make_request_span`] all spell it through this constant, so
/// they cannot drift onto different headers.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Stripe's error envelope, so SDK clients surface `.message` correctly.
///
/// Kept as the three-argument form because that is what an envelope without
/// a `param` is; see [`error_envelope_with_param`] for the fourth field and
/// why it is `Option` rather than always present. Nothing in production
/// calls this one — [`ApiError`]'s `IntoResponse` calls
/// [`error_envelope_with_param`] directly — and it stays because
/// `the_error_envelope_matches_stripes_shape` below pins the envelope's
/// shape through it, which is worth keeping independent of the classification
/// machinery that now decides what goes *into* it.
///
/// `pub(crate)`, like its four-argument sibling: ADR-0011 wants one renderer,
/// and visibility is what makes that structural instead of a convention a
/// handler can quietly break. `#[cfg(test)]` on top of that because the test
/// is now its only caller in any build — with no production caller left,
/// compiling it into the binary would be dead code the workspace's
/// `-D warnings` gate rightly refuses, and silencing that with an `allow`
/// would hide the very fact this comment is recording.
#[cfg(test)]
#[must_use = "the envelope is the response body"]
pub(crate) fn error_envelope(kind: &str, code: &str, message: &str) -> Value {
    error_envelope_with_param(kind, code, message, None)
}

/// [`error_envelope`] plus Stripe's optional `param`, naming the request
/// field a caller has to fix.
///
/// `param` is **omitted** rather than serialised as `null` when there is
/// none: Stripe omits it, an SDK testing `"param" in error` would be misled
/// by a null, and every response this crate has emitted so far had no such
/// key. Building the object by hand rather than mutating a `json!` literal
/// keeps that decision in one visible place.
///
/// **The one production envelope renderer**, called from [`ApiError`]'s
/// `IntoResponse` and nowhere else. `pub(crate)` on purpose: a handler in
/// another crate cannot reach it at all, so "handlers do not build envelopes"
/// is enforced by the module system rather than by review. The SDKs model
/// this shape from the wire (`sdks/rust`, `sdks/nodejs`), not from this
/// signature.
#[must_use = "the envelope is the response body"]
pub(crate) fn error_envelope_with_param(
    kind: &str,
    code: &str,
    message: &str,
    param: Option<&str>,
) -> Value {
    let mut error = Map::new();
    error.insert("type".to_owned(), Value::String(kind.to_owned()));
    error.insert("code".to_owned(), Value::String(code.to_owned()));
    error.insert("message".to_owned(), Value::String(message.to_owned()));
    if let Some(param) = param {
        error.insert("param".to_owned(), Value::String(param.to_owned()));
    }
    json!({ "error": Value::Object(error) })
}

/// Everything [`router`] needs to build a real, fully-wired application.
///
/// A struct rather than three positional arguments: two of the three fields
/// are cheap `Arc`-backed handles that look alike at a call site, and a
/// caller who swapped them would get a router that compiles and fails only
/// at runtime. It is also the list a future step extends — adding a
/// `/dash/v1` validator is a new field here, not a fourth argument every
/// existing call site has to be edited for.
///
/// Owned by value: `router` consumes it. Building two routers from one set
/// of dependencies is not a thing this crate supports, because the
/// [`MerchantOp`](op::MerchantOp) inside is deliberately not `Clone`.
///
/// `Debug` is derived and safe: every field formats through its own impl,
/// and all three redact — `MerchantOp` shows only its public metadata,
/// `JwtValidator` only its validation policy, and sqlx's `PgPool` prints no
/// connection string.
#[derive(Debug)]
pub struct RouterDeps {
    /// The database pool. `/healthz` probes it, `/v1/oauth/jwks.json` reads
    /// the published key set from it, and the OP's client store consults
    /// `disabled_clients` through it.
    pub pool: PgPool,
    /// The assembled merchant OP — see [`op::MerchantOp::new`]. `Arc`
    /// because axum clones router state per request and the OP holds a
    /// signing key and a trait-object store that must not be duplicated.
    pub merchant_op: std::sync::Arc<op::MerchantOp>,
    /// Validates the bearer tokens the OP above minted, in front of every
    /// `/v1` route that is not part of the OP itself.
    ///
    /// Deliberately a separate value rather than something `router` derives
    /// from `merchant_op`: the validator needs a *JWKS URL it can actually
    /// reach*, which is a deployment fact (`vpay-server` points it at
    /// loopback on the port it bound) and not something derivable from the
    /// OP's public issuer. See [`op::MerchantOp::jwks_url`].
    pub merchant_validator: MerchantJwtValidator,
    /// Every payment rail this process can reach, by `providers.code`.
    ///
    /// **Built by the binary, not by this crate.** `vpay-api` links no
    /// adapter crate and must not: `if provider == "mtn_momo"` outside an
    /// adapter is a defect (ADR-0002), and the way to make that structural
    /// rather than a review rule is for the HTTP layer to hold nothing but
    /// trait objects it cannot name the concrete types of. Each binary owns
    /// its own `adapters()` and hands the map in here.
    ///
    /// A `BTreeMap` rather than a `Vec` scanned per request: a confirm
    /// resolves a rail by the `payment_method_data[type]` a caller sent, and
    /// that is a lookup. Ordered rather than hashed so a log line or a test
    /// that iterates it is deterministic; the map holds two entries, so the
    /// lookup cost is irrelevant either way.
    ///
    /// `Arc` because axum clones router state per request and an adapter may
    /// hold a connection pool of its own.
    pub adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>,
    /// The slice of the YAML deployment configuration a request path needs —
    /// see [`ResourceConfig`], which is also where `livemode` reaches a
    /// handler from.
    ///
    /// Passed in rather than loaded here so that both binaries project the
    /// same `Config` the same way, and so this crate never learns how to
    /// find a config file.
    pub resource_config: Arc<ResourceConfig>,
}

/// Shared state for every route in this router.
///
/// `FromRef` impls below rather than one god-state extractor: each handler
/// names only the piece it needs, so [`op::jwks::jwks_handler`] stays a
/// function of a `PgPool` and could be mounted by a different assembler
/// entirely.
#[derive(Clone)]
pub(crate) struct AppState {
    pool: PgPool,
    merchant_op: std::sync::Arc<op::MerchantOp>,
    merchant_validator: MerchantJwtValidator,
    adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>,
    resource_config: Arc<ResourceConfig>,
}

/// So [`op::jwks::jwks_handler`] can take `State<PgPool>` and stay
/// independent of how this router is assembled — its own doc comment names
/// this impl as the assembler's side of that contract.
impl FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}

/// So the OP handlers can take `State<Arc<MerchantOp>>`.
impl FromRef<AppState> for std::sync::Arc<op::MerchantOp> {
    fn from_ref(state: &AppState) -> Self {
        std::sync::Arc::clone(&state.merchant_op)
    }
}

/// So [`require_merchant_token`] resolves its validator out of router state
/// — the bound that middleware requires.
impl FromRef<AppState> for MerchantJwtValidator {
    fn from_ref(state: &AppState) -> Self {
        state.merchant_validator.clone()
    }
}

/// So [`require_merchant_token`] can map a token's `client_id` to a tenant,
/// and so a `/v1` handler can take `State<Arc<ResourceConfig>>` and stay
/// independent of how this router is assembled.
impl FromRef<AppState> for Arc<ResourceConfig> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.resource_config)
    }
}

/// So a `/v1` handler can resolve a rail by code without this crate knowing
/// which adapters exist — see [`RouterDeps::adapters`].
impl FromRef<AppState> for Arc<BTreeMap<String, Box<dyn ProviderAdapter>>> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.adapters)
    }
}

/// `/healthz`: reflects whether the database is actually reachable right
/// now, rather than a static `"ok"`.
///
/// Single combined liveness+readiness endpoint, deliberately not split into
/// `/healthz` (process alive) and `/readyz` (DB reachable): nothing in this
/// repository defines a Kubernetes liveness vs. readiness probe today — no
/// manifest, no `Dockerfile HEALTHCHECK` (the runtime image is `FROM
/// scratch` and has no shell to run one — see `backends/Dockerfile`), no
/// `compose.yml` healthcheck on `vpay-server` itself. Inventing a real
/// liveness/readiness split ahead of an actual orchestration consumer that
/// would treat them differently would be exactly the kind of feature that
/// only *looks* more finished than it is (`CLAUDE.md`: "never make the repo
/// look more finished than it is") — a `/readyz` nobody polls proves
/// nothing. When real k8s manifests land, split this: a liveness probe
/// should stay a static `"ok"` (so a transient database blip does not cause
/// a pod restart that cannot fix a database outage), and a new readiness
/// probe should carry this DB check instead.
async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match vpay_db::check_connection(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(error) => {
            tracing::error!(%error, "healthz: database unreachable");
            (StatusCode::SERVICE_UNAVAILABLE, "database unreachable")
        }
    }
}

/// Honest 404 for every unimplemented route, naming vpay rather than pretending
/// to be Stripe.
///
/// Returns an [`ApiError`] rather than building an envelope here: ADR-0011
/// wants one renderer, and this handler used to be a second caller of
/// `error_envelope`. It is not one now, and since both envelope functions
/// became `pub(crate)` it could not be — `ApiError`'s `IntoResponse` is the
/// only production path to `error_envelope_with_param`. The response bytes
/// are unchanged — pinned verbatim by
/// `the_404_fallback_is_byte_for_byte_what_it_was_before_api_error`.
///
/// The method and path are captured for the log line only; the body
/// deliberately does not echo them back (see
/// [`ApiError::public_message`](vpay_core::Classify::public_message)).
async fn not_found(method: Method, uri: Uri) -> ApiError {
    ApiError::UnknownRoute {
        method: method.to_string(),
        path: uri.path().to_owned(),
    }
}

/// The span every handler runs inside, carrying the request id that
/// [`router`]'s middleware stack put on the request.
///
/// Recording `request_id` *here*, on the span, rather than as a field on
/// each event, is what makes it free for code that knows nothing about it:
/// `ApiError`'s `IntoResponse` logs a failure without naming a request id
/// anywhere, and the id still appears on that line because the span encloses
/// it (see [`error`]'s module docs, and
/// `an_error_logged_while_serving_a_request_carries_the_request_id`).
///
/// `path`, not the whole URI: a query string is caller-supplied and can
/// carry anything the caller put there, and this crate's rule is that caller
/// data does not get echoed into places it was not asked for — the same
/// reason [`not_found`]'s body does not repeat the path back.
///
/// The header is read rather than `Extensions::get::<RequestId>()` because
/// this function must also be correct if the span layer is ever mounted
/// without the id layer above it: a missing header records an empty
/// `request_id` — visibly wrong in a log line — instead of inventing a
/// second id that no response header would carry. `from_utf8_lossy` rather
/// than `to_str().ok()` for the same reason: under [`router`] the value is
/// always ASCII by the time it arrives here — [`discard_unusable_request_id`]
/// ran first and either vetted the caller's id or had it replaced by a
/// minted UUID — but this function does not get to assume its own stack, and
/// a caller's non-ASCII bytes should be *visible* in the log rather than
/// silently blanking the field.
fn make_request_span(request: &Request<Body>) -> Span {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .map_or(Cow::Borrowed(""), |value| {
            String::from_utf8_lossy(value.as_bytes())
        });
    tracing::info_span!(
        "request",
        method = %request.method(),
        path = request.uri().path(),
        request_id = %request_id,
    )
}

/// The longest caller-supplied request id this router will honour, in bytes.
///
/// 64 comfortably admits every correlation-id shape that actually turns up
/// in this header — a hyphenated v4 UUID (36), a hyphenless one (32), a W3C
/// `traceparent` trace-id (32 hex), a ULID (26) — while bounding what an
/// unauthenticated caller can force into every log event of their request
/// and into the response header. There is no standard maximum to defer to;
/// this is a budget, not a spec, which is why it lives here as one named
/// number rather than inline at the comparison.
const MAX_REQUEST_ID_LEN: usize = 64;

/// Whether a caller-supplied `x-request-id` value is one this router will
/// carry, rather than mint over.
///
/// 1..=[`MAX_REQUEST_ID_LEN`] bytes of ASCII `[A-Za-z0-9._-]`: the
/// intersection of the shapes named on that constant, and a charset with no
/// quote, brace, backslash, comma or `=` in it — the bytes that turn a
/// request id embedded in a structured log line into a second field, or a
/// broken one. Empty is rejected too: an empty id correlates nothing, and
/// letting it through would make `make_request_span` record a blank
/// `request_id` for a request that a fresh UUID would have made traceable.
fn is_usable_request_id(value: &[u8]) -> bool {
    (1..=MAX_REQUEST_ID_LEN).contains(&value.len())
        && value
            .iter()
            .all(|&byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Strips a caller-supplied `x-request-id` this router will not honour, so
/// that [`SetRequestIdLayer`] below mints a fresh UUID in its place.
///
/// [`SetRequestIdLayer`] only mints an id when the header is *absent*, so
/// without this step whatever a caller puts in the header becomes the id:
/// recorded by [`make_request_span`] onto the span enclosing the whole
/// request — and therefore onto every log event that request emits — and
/// echoed back by [`PropagateRequestIdLayer`]. `HeaderValue` already refuses
/// control bytes, but nothing bounded the length or the charset, which left
/// an unauthenticated caller choosing a 4 KB, quote-laden string repeated on
/// every line an operator later greps.
///
/// **It removes the header instead of rejecting the request.** A 400 here
/// would let a malformed *diagnostic* header stop a payment, trading a
/// cosmetic problem for a real one; the request id exists to help someone
/// correlate a log line, and no merchant should lose a charge over one. A
/// hostile caller loses only the ability to *choose* the id — they still get
/// one, it is still on their response, and support can still act on it.
///
/// Every value is checked, not just the first, and one bad value drops the
/// whole header. Today the three consumers below all read `.get()` (the
/// first value), so a second garbage value would be inert — but "inert
/// because of what the current readers happen to do" is not a property worth
/// depending on. An absent header vacuously passes, which is the intended
/// no-op: there is nothing to strip and the layer below mints.
async fn discard_unusable_request_id(mut request: Request<Body>, next: Next) -> Response {
    let usable = request
        .headers()
        .get_all(REQUEST_ID_HEADER)
        .iter()
        .all(|value| is_usable_request_id(value.as_bytes()));
    if !usable {
        request.headers_mut().remove(REQUEST_ID_HEADER);
    }
    next.run(request).await
}

/// The `/v1` authentication **and authorisation** boundary: validates the
/// bearer token once, checks it carries a scope for what is being asked, and
/// puts what it learned on the request.
///
/// Two things go into the request's extensions, and both are read back by
/// extractors that fail closed if they are missing
/// ([`resource_auth::AuthenticatedMerchant`], [`MerchantScope`]):
///
/// - the validated [`ResourceClaims`](resource_auth::ResourceClaims) —
///   `client_id` and `scope`;
/// - the [`MerchantScope`] those claims resolve to, which is the tenant
///   every `/v1` query is filtered by.
///
/// # Why the scope check is here and not in a handler
///
/// [`v1::required_scopes`] decides what the request's method needs, and a
/// token carrying none of those scopes is [`ApiError::Forbidden`] — 403,
/// not 404: this is a statement about the *credential*, which the caller can
/// inspect for itself, so there is nothing to leak by saying so plainly.
/// (Object-level tenancy is the opposite case and answers
/// [`ApiError::NotFound`]; see that variant.)
///
/// It runs before the router matches a route, which is what makes it
/// impossible for a new `/v1` handler to be added without it — the failure
/// mode of a per-handler check is a handler that simply does not have one,
/// and nothing in the type system notices. The cost is that the rule can
/// only see the method, which is why it is expressed per-method.
///
/// # Why a middleware and not `from_extractor_with_state`
///
/// axum 0.8's `from_extractor_with_state::<AuthenticatedMerchant, _>`
/// *discards* the value it extracted. It is a fine gate and a poor
/// hand-off: every handler that then asked for the claims validated the
/// same token a second time — a second JWKS cache consultation, which can
/// legitimately return a different key set than the first one did if a
/// refresh landed in between. One validation per request is both cheaper
/// and the only way the boundary and the handler cannot disagree.
///
/// # Why an unknown `client_id` is 403 and not 401
///
/// The token is genuine: this process signed it, and it validates. What is
/// missing is a *registration* — the deployment's YAML no longer maps this
/// client to a tenant, which is what happens for the remaining TTL of a
/// token minted before a config change that removed the client. 401 would
/// tell the caller to present a credential, and it already has a valid one;
/// 403 says the credential is not permitted to act here, which is the truth.
/// (It is also what would happen if a token minted by a *different* vpay
/// deployment sharing this issuer arrived, which is worth failing closed on
/// rather than falling back to any tenant.)
///
/// Generic over the state so that `resource_auth`'s own tests mount this
/// exact function rather than a copy of it — a middleware reimplemented in a
/// test harness proves nothing about the one that ships.
pub async fn require_merchant_token<S>(
    State(state): State<S>,
    request: Request<Body>,
    next: Next,
) -> Response
where
    S: Send + Sync + Clone + 'static,
    MerchantJwtValidator: FromRef<S>,
    Arc<ResourceConfig>: FromRef<S>,
{
    let (mut parts, body) = request.into_parts();

    let claims = {
        let token = match extract_bearer_token(&parts) {
            Ok(token) => token,
            Err(rejection) => return ApiError::from(rejection).into_response(),
        };
        match MerchantJwtValidator::from_ref(&state)
            .0
            .validate(token)
            .await
        {
            Ok(claims) => claims,
            Err(rejection) => return ApiError::from(rejection).into_response(),
        }
    };

    let resource_config = Arc::<ResourceConfig>::from_ref(&state);
    let Some(merchant_id) = resource_config.merchant_id_for(&claims.client_id) else {
        tracing::warn!(
            client_id = %claims.client_id,
            "a validly signed /v1 token names a client this deployment has no registration for; \
             refusing rather than guessing a tenant"
        );
        return ApiError::Forbidden.into_response();
    };
    let scope = MerchantScope {
        merchant_id: merchant_id.to_owned(),
    };

    // The token says which client, and it also says what that client was
    // authorised to do. Checked here rather than in each handler for the
    // same reason the tenant is resolved here: a handler that forgot would
    // be a silent hole, and there is nothing in a `pub(crate) async fn`'s
    // signature to notice it is missing.
    let required = v1::required_scopes(&parts.method);
    if !required.iter().any(|scope| claims.has_scope(scope)) {
        tracing::warn!(
            client_id = %claims.client_id,
            method = %parts.method,
            granted = ?claims.scope,
            required = ?required,
            "a /v1 token carries none of the scopes this request needs; refusing"
        );
        return ApiError::Forbidden.into_response();
    }

    parts.extensions.insert(claims);
    parts.extensions.insert(scope);

    next.run(Request::from_parts(parts, body)).await
}

/// The largest `/v1` request body this router will read, in bytes.
///
/// 64 KiB. Every documented `/v1` body is a handful of form fields
/// (`examples/merchant-curl/README.md`); the biggest legitimate one is a
/// create with 50 metadata entries, which the validation bounds cap at
/// roughly 27 KB of keys and values. 64 KiB leaves room for percent-encoding
/// and still refuses a body an unauthenticated caller could use to make this
/// process buffer megabytes — the limit layer is mounted *outside* the token
/// check for exactly that reason.
const V1_BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Builds the application router from [`RouterDeps`].
///
/// # Route tree
///
/// Three groups, and which group a path falls into is the whole security
/// boundary of this process:
///
/// | Path | Auth | Why |
/// |---|---|---|
/// | `GET /healthz` | none | A probe must answer before anything is configured, and it reveals only whether Postgres is reachable. |
/// | `POST /v1/oauth/token` | none | The credential *is* the request body (RFC 7523 `client_assertion`). Requiring a bearer token to get a bearer token is circular. |
/// | `GET /v1/oauth/.well-known/openid-configuration` | none | How a client that has never spoken to vpay finds the token endpoint. |
/// | `GET /v1/oauth/jwks.json` | none | How a verifier that has never spoken to vpay learns the public keys. Same circularity. |
/// | **anything else under `/v1/oauth`** | none | The OP subtree is public by design; its own `.fallback(not_found)` answers the honest 404 rather than letting the path escape to the outer router. |
/// | **everything else under `/v1`** | `AuthenticatedMerchant` | The merchant API. |
/// | anything else | none | The honest 404. |
///
/// The `/v1` nest mounts [`v1::V1_ROUTES`] and a 404 fallback for
/// everything else, which is the production behaviour and not a
/// placeholder: `/v1/payment_intents` is real, and `/v1/refunds`,
/// `/v1/balance` and `/v1/events` are not implemented and are therefore not
/// routed (`docs/status.md`). The boundary is observable in three answers —
///
/// - `GET /v1/payment_intents/pi_x` with no bearer token → **401**, the
///   [`ApiError::Auth`] envelope;
/// - the same request with a valid merchant token, for an id this merchant
///   has no intent under → **404**, the `resource_missing` envelope;
/// - `GET /v1/balance` with a valid token → **404**, the `unknown_route`
///   envelope.
///
/// — which is exactly what a merchant integrating against this deployment
/// should get. Inventing a `/v1/balance` so the third answer could be a
/// `200` is the failure mode `CLAUDE.md` names first.
///
/// The authentication layer is [`require_merchant_token`] via
/// [`from_fn_with_state`] (Step 2's D3 — that function's docs say why it is
/// not `from_extractor_with_state`), mounted with `Router::layer` on the
/// nested router so that it wraps that router's fallback too. `route_layer`
/// is the wrong tool and axum says so: it does not apply to a fallback by
/// design, so an unmatched `/v1/...` path would answer an *unauthenticated*
/// 404 and tell an anonymous caller which `/v1` resources exist. When this
/// nest had no routes at all, axum refused that spelling outright ("Adding a
/// route_layer before any routes is a no-op"); now that it has routes, the
/// swap would compile and be silently wrong, which is why the choice is
/// written down here rather than left to the compiler —
/// `an_unauthenticated_v1_request_is_401_not_404` is what actually catches
/// it.
///
/// A path under `/v1/oauth` that matches no OP route (say
/// `/v1/oauth/authorize`, which vpay does not serve) answers an
/// unauthenticated 404 — and does so from the OP router's **own**
/// `.fallback(not_found)`, mounted below. That outcome is intended: the
/// whole `/v1/oauth` subtree is public by design, so a 404 there leaks
/// nothing a merchant could not learn from the discovery document.
///
/// **The fallback is load-bearing, not decoration, and this paragraph used
/// to say the opposite.** It previously claimed that an unmatched
/// `/v1/oauth/...` path "falls through to the outer router's fallback and
/// answers an unauthenticated 404". Measured, it did not: with no fallback
/// on the OP router, axum flattens that nest's three routes into the outer
/// path table and registers no `/v1/oauth/{*rest}` entry at all, so
/// `GET /v1/oauth/not_a_route` matched `/v1/{*rest}` — the *authenticated*
/// nest — and answered **401**. Removing the `.fallback(not_found)` below
/// reproduces it, and `the_oauth_nest_answers_its_own_404` fails with
/// `left: 401, right: 404`.
///
/// A 401 there is the wrong answer twice over: it tells an integrator who
/// mistyped an OP path to present a bearer token, on the one subtree whose
/// entire purpose is handing out bearer tokens to callers that do not have
/// one yet — and it made "which router serves this path" depend on an axum
/// flattening detail rather than on anything written here. With the
/// fallback the OP router is closed over its own prefix: every
/// `/v1/oauth/...` path is served by the OP router, unmatched ones included,
/// and that is a property a test checks rather than an accident of
/// registration order.
///
/// A *known* OP path with the wrong method — `GET /v1/oauth/token` — gets
/// axum's own bare `405`, not this crate's envelope. Left as-is
/// deliberately: 405 is the correct status, and turning it into the 404
/// envelope would tell an integrator the path does not exist when it does.
/// The gap is that its body is empty rather than the Stripe envelope; that
/// is worth fixing when a `method_not_allowed` renderer exists for the
/// whole surface, not one route at a time.
///
/// # Middleware order
///
/// `ServiceBuilder` applies layers outside-in, so the list below is the
/// order a *request* traverses them, and the reverse of the order a response
/// does. All four are load-bearing in that order:
///
/// 1. `discard_unusable_request_id` (private, just above this function in
///    the source) — removes a caller's `x-request-id` unless it is short and
///    plain enough to carry (`is_usable_request_id`, above it again). It
///    must be first, and above the minting layer specifically: step 2 only
///    mints when the header is *absent*, so this step's removal is exactly
///    what causes a fresh id to be minted for a caller whose own id was not
///    usable.
/// 2. [`SetRequestIdLayer`] — mints an `x-request-id` (a v4 UUID, via
///    [`MakeRequestUuid`]) on the request, **unless the caller already sent
///    one that step 1 kept**, in which case theirs is kept. Everything below
///    reads the header it sets.
/// 3. [`TraceLayer`] — opens `make_request_span` (private, just above this
///    function in the source) around the handler, so the id is on the span
///    before any handler, extractor or error renderer runs, and every event
///    they emit inherits it.
/// 4. [`PropagateRequestIdLayer`] — innermost, so it sees the id step 2 set and
///    is the first layer to touch the response on the way out; it copies the request's id onto the
///    response, which is what makes `Category::Internal`'s "Contact support
///    with the request id" a promise a merchant can actually act on.
///
/// Step 1 is [`axum::middleware::from_fn`] rather than
/// `tower::util::MapRequestLayer`: `MapRequestLayer` sits behind tower's
/// `util` feature, which the workspace pin (`tower = "0.5"`, no feature
/// list) does not enable — it is on today only through feature unification
/// from an unrelated transitive dependency, so using it would make this
/// stack compile by accident. `from_fn` needs no feature axum does not
/// already have.
///
/// Mounted on the outermost router, so it wraps every group above —
/// including the 401 an unauthenticated `/v1` request gets, which is the
/// response most likely to be the one a confused integrator is holding, and
/// which therefore needs a request id on it more than any other.
pub fn router(deps: RouterDeps) -> Router {
    let state = AppState {
        pool: deps.pool,
        merchant_op: deps.merchant_op,
        merchant_validator: deps.merchant_validator,
        adapters: deps.adapters,
        resource_config: deps.resource_config,
    };

    // Unauthenticated by necessity, not by omission — see the table above.
    let oauth = Router::new()
        .route("/token", post(op::token::token_handler))
        .route(
            "/.well-known/openid-configuration",
            get(op::token::discovery_handler),
        )
        .route("/jwks.json", get(op::jwks::jwks_handler))
        // Explicit, not inherited — see this function's route table and the
        // paragraph under it.
        .fallback(not_found);

    // Every route is mounted from `v1::V1_ROUTES` — see that constant, and
    // `v1::routes`, which also carries the fallback. `Router::layer` (not
    // `route_layer`) is what puts the two layers below in front of that
    // fallback as well as in front of the routes.
    let v1 = v1::routes()
        .layer(from_fn_with_state(
            state.clone(),
            require_merchant_token::<AppState>,
        ))
        // Outside the token check, deliberately: a body limit that only
        // applied to authenticated callers would let an anonymous one make
        // this process buffer a body before the 401. `RequestBodyLimitLayer`
        // rather than axum's `DefaultBodyLimit` because the latter is
        // applied per-handler through extractors and would not cover a body
        // read by middleware.
        .layer(RequestBodyLimitLayer::new(V1_BODY_LIMIT_BYTES));

    Router::new()
        .route("/healthz", get(healthz))
        .nest("/v1/oauth", oauth)
        .nest("/v1", v1)
        .fallback(not_found)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(from_fn(discard_unusable_request_id))
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http().make_span_with(make_request_span))
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{HeaderValue, Request};
    use axum::response::Response;
    use tower::ServiceExt as _;

    use super::*;
    use crate::test_fixtures::deps;
    use crate::test_log;

    #[test]
    fn the_error_envelope_matches_stripes_shape() {
        let v = error_envelope("invalid_request_error", "x", "y");
        let err = v.get("error").expect("error key");
        assert_eq!(
            err.get("type").and_then(Value::as_str),
            Some("invalid_request_error")
        );
        assert_eq!(err.get("code").and_then(Value::as_str), Some("x"));
        assert_eq!(err.get("message").and_then(Value::as_str), Some("y"));
    }

    /// A path that this router deliberately does not route, used by every
    /// middleware test below: it is the cheapest request that traverses the
    /// whole stack *and* makes `ApiError` render and log on the way out.
    ///
    /// Deliberately **outside** `/v1`. Once the authentication layer went in
    /// front of that nest, a `/v1/...` path stopped being an "unrouted" path
    /// at all — it is a 401, decided before routing — so a middleware test
    /// aimed there would be asserting on the auth boundary instead of on the
    /// request-id stack. The boundary has its own tests, further down.
    const UNROUTED_PATH: &str = "/not_a_vpay_route";

    async fn get(uri: &str) -> Response {
        router(deps())
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve")
    }

    #[tokio::test]
    async fn unknown_routes_still_get_the_honest_404() {
        assert_eq!(get(UNROUTED_PATH).await.status(), StatusCode::NOT_FOUND);
    }

    /// The security boundary, asserted from the outside: no bearer token, no
    /// `/v1`.
    ///
    /// This is the decisive test for mounting the auth layer with
    /// `Router::layer` rather than `route_layer` — `route_layer` does not
    /// wrap a fallback, and since the `/v1` nest's only route *is* its
    /// fallback, that spelling would answer 404 here and let every future
    /// `/v1` resource inherit the hole. Removing the layer entirely fails
    /// this test the same way.
    #[tokio::test]
    async fn an_unauthenticated_v1_request_is_401_not_404() {
        for path in [
            "/v1/payment_intents",
            "/v1/payment_intents/pi_x",
            "/v1/refunds",
            // `/v1` itself — `Router::nest` registers the bare prefix as
            // well as `/v1/{*rest}`, so the boundary covers it.
            "/v1",
            // Two slashes: `{*rest}` captures `/payment_intents`, so a
            // caller cannot slip past the boundary by doubling a separator.
            "/v1//payment_intents",
        ] {
            let response = get(path).await;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must be behind the merchant authentication boundary"
            );
        }
    }

    /// The 401 a merchant gets is this crate's own envelope, not axum's bare
    /// rejection body — so an SDK decoding `error.code` sees something it can
    /// branch on.
    #[tokio::test]
    async fn the_unauthenticated_v1_401_is_the_stripe_shaped_envelope() {
        let response = get("/v1/payment_intents").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("the envelope is small");
        let envelope: Value = serde_json::from_slice(&body).expect("the 401 body is JSON");
        let error = envelope.get("error").expect("error key");
        assert_eq!(
            error.get("type").and_then(Value::as_str),
            Some("authentication_error")
        );
        assert_eq!(
            error.get("code").and_then(Value::as_str),
            Some("missing_bearer_token")
        );
    }

    /// The one path under `/v1` that is **not** behind the boundary, stated
    /// as a fact rather than left to be discovered.
    ///
    /// `Router::nest("/v1", ..)` registers `/v1` and `/v1/{*rest}`, and
    /// matchit's catch-all requires at least one character — so the bare
    /// trailing-slash form `/v1/` matches neither and falls through to the
    /// outer 404. That is not a hole: there is no resource at `/v1/`, every
    /// real resource path has a non-empty segment after it (covered above,
    /// including the doubled-slash form), and an unauthenticated 404 for a
    /// path that does not exist discloses nothing. It is asserted so the
    /// behaviour is recorded rather than assumed, and so that a future axum
    /// or matchit change here is a visible test failure.
    #[tokio::test]
    async fn the_bare_trailing_slash_form_of_v1_falls_through_to_the_outer_404() {
        assert_eq!(get("/v1/").await.status(), StatusCode::NOT_FOUND);
    }

    /// The OP's own three routes are *outside* the boundary. A 401 on any of
    /// them would be circular — a merchant cannot present a token it has no
    /// way to obtain, and a verifier cannot fetch keys it needs a token for.
    ///
    /// `jwks.json` is asserted as "not 401" rather than as `200`: it reads
    /// `oauth_signing_keys` through a pool that has never connected, so its
    /// honest answer here is the 503 [`op::jwks::jwks_handler`] documents.
    /// The property under test is that the request reached the handler at
    /// all.
    #[tokio::test]
    async fn the_oauth_routes_are_reachable_without_a_token() {
        assert_eq!(
            get("/v1/oauth/.well-known/openid-configuration")
                .await
                .status(),
            StatusCode::OK
        );

        let jwks = get("/v1/oauth/jwks.json").await;
        assert_ne!(
            jwks.status(),
            StatusCode::UNAUTHORIZED,
            "/v1/oauth/jwks.json must not require the token it exists to let a verifier check"
        );

        let token = router(deps())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("grant_type=client_credentials"))
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");
        // No `client_id` and no credential, so this is `invalid_client` —
        // a 401 from the *token endpoint's own* RFC 6749 body, not from the
        // resource-server boundary. Distinguished by the body shape below.
        assert_eq!(token.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(token.into_body(), 64 * 1024)
            .await
            .expect("the token error body is small");
        let error: Value = serde_json::from_slice(&body).expect("the token error body is JSON");
        assert_eq!(
            error.get("error").and_then(Value::as_str),
            Some("invalid_client"),
            "the token endpoint speaks RFC 6749, not the Stripe envelope"
        );
    }

    /// An unmatched path under `/v1/oauth` is answered by the OP router's
    /// own fallback: a 404 in this crate's envelope, unauthenticated, which
    /// is the intended answer because that whole subtree is the public OP
    /// surface (see [`router`]'s route table).
    ///
    /// Decisive: deleting `.fallback(not_found)` from the `oauth` router in
    /// [`router`] makes the first path answer **401** instead — the
    /// unmatched path is then served by the authenticated `/v1` nest, for
    /// the reason written on that function. Nothing under `/v1/oauth` may
    /// require the token it exists to hand out, so 401 there is a bug even
    /// though it is the *more* restrictive answer.
    ///
    /// The body is asserted too, so a future change that answers 404 with an
    /// empty body — axum's own default — is a failure here rather than a
    /// silently worse answer for an SDK that mistyped a path.
    #[tokio::test]
    async fn the_oauth_nest_answers_its_own_404() {
        for path in [
            "/v1/oauth/not_a_route",
            // The two shapes a real integrator mistypes: an OP endpoint vpay
            // does not serve at all, and a real one with a trailing segment.
            "/v1/oauth/authorize",
            "/v1/oauth/jwks.json/extra",
        ] {
            let response = get(path).await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{path} must answer the honest 404"
            );

            let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("the envelope is small");
            let envelope: Value = serde_json::from_slice(&body)
                .unwrap_or_else(|e| panic!("{path}: the 404 body must be this crate's JSON: {e}"));
            let error = envelope.get("error").expect("error key");
            assert_eq!(
                error.get("code").and_then(Value::as_str),
                Some("unknown_route"),
                "{path}: got {envelope:#}"
            );
            assert_eq!(
                error.get("type").and_then(Value::as_str),
                Some("invalid_request_error"),
                "{path}: got {envelope:#}"
            );
        }
    }

    /// `/healthz` stayed outside `/v1` and outside the boundary. Asserted as
    /// "not 401" for the same reason as `jwks.json` above: against a lazy
    /// pool its honest answer is 503.
    #[tokio::test]
    async fn healthz_is_still_unauthenticated() {
        assert_ne!(get("/healthz").await.status(), StatusCode::UNAUTHORIZED);
    }

    /// A `GET` of a route that does not exist, which is the cheapest request
    /// that traverses the whole middleware stack *and* makes `ApiError` log
    /// on the way out — so one request can pin both the header behaviour and
    /// the span behaviour.
    fn a_request(request_id: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(UNROUTED_PATH);
        if let Some(id) = request_id {
            builder = builder.header(REQUEST_ID_HEADER, id);
        }
        builder.body(Body::empty()).expect("valid request")
    }

    fn request_id_of(response: &Response) -> &str {
        response
            .headers()
            .get(REQUEST_ID_HEADER)
            .expect("every response carries an x-request-id header")
            .to_str()
            .expect("the request id is ascii")
    }

    #[tokio::test]
    async fn a_request_without_an_id_gets_a_generated_uuid_on_the_response() {
        let response = router(deps())
            .oneshot(a_request(None))
            .await
            .expect("router does not fail to serve");

        let id = request_id_of(&response);
        assert!(!id.is_empty(), "the generated request id must not be empty");
        // Parsed, not merely non-empty: a `MakeRequestId` that handed back a
        // constant, or the header name being right but the value being some
        // other string, would pass an emptiness check and fail this one.
        uuid::Uuid::parse_str(id)
            .unwrap_or_else(|e| panic!("generated request id {id:?} is not a uuid: {e}"));
    }

    #[tokio::test]
    async fn a_caller_supplied_request_id_is_returned_unchanged() {
        // A caller's id is *not* required to be a UUID — this one deliberately
        // is not. A reverse proxy or a merchant's own tracing system chose
        // it, and overwriting it would break the correlation it exists for.
        let response = router(deps())
            .oneshot(a_request(Some("abc-123")))
            .await
            .expect("router does not fail to serve");

        assert_eq!(request_id_of(&response), "abc-123");
    }

    /// [`a_request`] for an id that is not a valid `&str` header value.
    ///
    /// `Request::builder().header(_, "caf\u{e9}")` would fail to build, so the
    /// non-ASCII case is unreachable through [`a_request`] — and asserting on
    /// a request the router never saw would prove nothing. `HeaderValue`
    /// accepts these bytes (obs-text is legal in a header value), which is
    /// precisely why the router has to.
    fn a_request_with_raw_id(id: &[u8]) -> Request<Body> {
        Request::builder()
            .uri(UNROUTED_PATH)
            .header(
                REQUEST_ID_HEADER,
                HeaderValue::from_bytes(id).expect("a header value a caller could really send"),
            )
            .body(Body::empty())
            .expect("valid request")
    }

    /// Asserts the response carries a freshly minted UUID rather than
    /// whatever the caller asked for.
    ///
    /// Parsing as a UUID, not merely differing from the input, is what makes
    /// these tests decisive: a step that mangled the caller's id (truncated
    /// it, say) instead of dropping the header would satisfy "not what they
    /// sent" while still letting the caller choose most of the id.
    fn assert_minted_uuid(response: &Response, supplied: &[u8]) {
        let id = request_id_of(response);
        assert_ne!(
            id.as_bytes(),
            supplied,
            "a request id this router will not honour must not be echoed back"
        );
        uuid::Uuid::parse_str(id)
            .unwrap_or_else(|e| panic!("replacement request id {id:?} is not a uuid: {e}"));
    }

    async fn serve(request: Request<Body>) -> Response {
        router(deps())
            .oneshot(request)
            .await
            .expect("router does not fail to serve")
    }

    #[tokio::test]
    async fn an_oversized_caller_supplied_request_id_is_replaced_by_a_uuid() {
        // 4 KB: comfortably inside what a server will accept as a header and
        // far outside anything an operator wants on every log line of a
        // request an unauthenticated caller can make for free.
        let supplied = "a".repeat(4096);
        let response = serve(a_request(Some(&supplied))).await;
        assert_minted_uuid(&response, supplied.as_bytes());
    }

    #[tokio::test]
    async fn a_caller_supplied_request_id_with_a_disallowed_byte_is_replaced_by_a_uuid() {
        for supplied in [
            b"abc 123".as_slice(),  // space: splits a value in a log line
            b"abc/123".as_slice(),  // path separator
            b"abc\"123".as_slice(), // quote: breaks a JSON-encoded field
            b"caf\xe9".as_slice(),  // non-ASCII, reachable only as raw bytes
        ] {
            let response = serve(a_request_with_raw_id(supplied)).await;
            assert_minted_uuid(&response, supplied);
        }
    }

    /// The length bound at its two decisive inputs. Written as literals
    /// rather than derived from `MAX_REQUEST_ID_LEN`, so that widening the
    /// constant is a deliberate change to this test too and not something
    /// that slips through green.
    #[tokio::test]
    async fn the_request_id_length_bound_admits_64_bytes_and_rejects_65() {
        let at_bound = "a".repeat(64);
        let response = serve(a_request(Some(&at_bound))).await;
        assert_eq!(
            request_id_of(&response),
            at_bound,
            "64 bytes is inside the bound and must be carried unchanged"
        );

        let over_bound = "a".repeat(65);
        let response = serve(a_request(Some(&over_bound))).await;
        assert_minted_uuid(&response, over_bound.as_bytes());
    }

    /// A usable first value does not license an unusable second one.
    ///
    /// Pins the choice of `get_all` over `get` in
    /// [`discard_unusable_request_id`]: an implementation that inspected only
    /// the first value would return `abc-123` here and fail this test.
    #[tokio::test]
    async fn one_unusable_value_drops_the_whole_request_id_header() {
        let request = Request::builder()
            .uri(UNROUTED_PATH)
            .header(REQUEST_ID_HEADER, "abc-123")
            .header(REQUEST_ID_HEADER, "a".repeat(4096))
            .body(Body::empty())
            .expect("valid request");

        let response = serve(request).await;
        assert_ne!(
            request_id_of(&response),
            "abc-123",
            "a second, unusable value must not be masked by a usable first one"
        );
        uuid::Uuid::parse_str(request_id_of(&response))
            .unwrap_or_else(|e| panic!("replacement request id is not a uuid: {e}"));
    }

    /// Serves one request against the real router with `tracing` captured.
    ///
    /// Holds the [`tracing::subscriber::set_default`] guard across the
    /// `.await` rather than using `test_log::with_captured_log`: a closure
    /// returning a future would install the subscriber only while the future
    /// was *built*, not while it ran, and every event of interest is emitted
    /// during the latter. `#[tokio::test]`'s default runtime is
    /// current-thread, so the whole future is polled on this thread and the
    /// thread-local default applies for all of it.
    ///
    /// # Why it serves the request twice
    ///
    /// `tracing` caches each callsite's `Interest` **globally**, computed
    /// the first time that callsite is reached, from whatever dispatcher is
    /// default *on the thread that got there first*
    /// (`tracing-core-0.1.33/src/callsite.rs`,
    /// `rebuild_callsite_interest`). `subscriber::set_default` installs a
    /// thread-local dispatcher and does not touch that cache. So under
    /// `cargo test`, where the whole suite shares one process, a sibling
    /// test that drove the router on another thread with no subscriber
    /// installed can cache `make_request_span`'s callsite — and
    /// `TraceLayer`'s own — as "never interested", after which the span
    /// below is a no-op and this test fails on a log line missing its
    /// `request_id`. Reproduced: fails at default parallelism, passes at
    /// `--test-threads=2`, passes under `cargo nextest` (a process per
    /// test).
    ///
    /// The fix is ordering, not luck. The throwaway request forces every
    /// callsite this test depends on to be *registered*; the
    /// `rebuild_interest_cache()` then recomputes all of them against this
    /// thread's subscriber, which is the capturing one. A callsite already
    /// registered cannot be reset to "never" except by another rebuild, and
    /// nothing else in this workspace calls that function — so from here the
    /// measured request is deterministic.
    ///
    /// The warm-up writes into its own discarded sink, so the returned
    /// capture contains exactly one request's events.
    async fn serve_capturing_log(request: Request<Body>) -> (Response, String) {
        {
            let warmup = tracing::subscriber::set_default(test_log::captured_log_subscriber(
                test_log::CapturedLog::default(),
            ));
            let _ = router(deps())
                .oneshot(a_request(None))
                .await
                .expect("router does not fail to serve");
            drop(warmup);
        }

        let sink = test_log::CapturedLog::default();
        let guard =
            tracing::subscriber::set_default(test_log::captured_log_subscriber(sink.clone()));
        tracing::callsite::rebuild_interest_cache();
        let response = router(deps())
            .oneshot(request)
            .await
            .expect("router does not fail to serve");
        drop(guard);
        (response, sink.contents())
    }

    #[tokio::test]
    async fn an_error_logged_while_serving_a_request_carries_the_request_id() {
        const SUPPLIED: &str = "abc-123";

        let (response, log) = serve_capturing_log(a_request(Some(SUPPLIED))).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // The assertion is deliberately scoped to `ApiError`'s own line
        // ("api error", from `ApiError::log`) rather than to the whole
        // capture. `TraceLayer` emits its own request/response events inside
        // the same span, so `log.contains(SUPPLIED)` would pass even if
        // nothing this crate writes inherited the span — which is precisely
        // the property under test: `error.rs` names no request id anywhere,
        // and its line carries one only because `make_request_span` put it
        // on the enclosing span.
        let api_error_line = log
            .lines()
            .find(|line| line.contains("api error"))
            .unwrap_or_else(|| panic!("the 404 fallback must log through ApiError; got:\n{log}"));

        assert!(
            api_error_line.contains(SUPPLIED),
            "the request id must reach the log line an operator correlates on.\n\
             line: {api_error_line}\nfull capture:\n{log}"
        );
        assert!(
            api_error_line.contains("request_id"),
            "the id must be a named span field, not an accident of some other value.\n\
             line: {api_error_line}"
        );
    }
}
