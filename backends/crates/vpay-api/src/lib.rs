//! The Stripe-shaped HTTP surface.
//!
//! STATUS: `/healthz`, the `/v1/oauth` merchant OP ([`op`]), the
//! authentication boundary in front of `/v1`, `/v1/payment_intents`
//! ([`v1::payment_intents`]), `/v1/events` ([`v1::events`], read-only, since
//! Step 5) and the Stripe-shaped 404 envelope are implemented. The two
//! `/v1` resources an SDK can name and vpay does not serve — `/v1/refunds`
//! and `/v1/balance` — are routed nowhere and answer the honest 404 from the
//! nest's fallback. `GET /v1/events`'s documented `?type=` filter is
//! deliberately not implemented and is ignored rather than refused; see that
//! module. This file must never grow a route that returns fabricated data;
//! a real database check (below) and a real 404 are the opposite of
//! fabricated data, so they stay. See `docs/status.md`.
//!
//! [`observability`] is this crate's *other* router: `/livez` and
//! `/metrics`, mounted by both binaries on `--observability-bind` and by
//! neither on the traffic port. It is here rather than in `main.rs` because
//! it is mechanism both binaries share, and because the test that keeps
//! those two paths off [`router`] belongs beside the module that owns them.
//!
//! [`resource_auth`] supplies the bearer-token validation now mounted in
//! front of `/v1` — see [`router`]'s "Route tree" section for exactly which
//! paths sit inside and outside it. `/dash/v1` is still mounted nowhere:
//! that surface needs the dashboard OIDC login flow, which is later work.
//!
//! Since Step 5c there is a **third** surface, [`browser`]: two routes under
//! `/v1/browser` that a payer's own page calls with a publishable key and the
//! payment intent's `client_secret` instead of a bearer token. It is
//! deliberately not part of [`V1_ROUTES`] (its table is [`BROWSER_ROUTES`]),
//! because the boundary test that walks that constant asserts every entry
//! answers `401` without a token — which these two must not.
//!
//! [`ApiError`] is this layer's Tier-2 composite error
//! ([ADR-0011](../../../docs/adr/0011-error-modelling.md)). Every failure
//! response in this crate is rendered by its `IntoResponse` — including the
//! 404 fallback below — so a handler returns `Result<_, ApiError>` and never
//! picks a status or writes a merchant-facing sentence. The one deliberate
//! exception is `/healthz`, which answers plain text for the reasons given
//! in [`error`]'s module docs.
//!
//! [`router`] mounts a five-layer middleware stack — a caller-supplied id
//! vetted, request id in, the same id mirrored onto the response under
//! Stripe's `request-id` spelling, span around the handler, request id back
//! out — described on that function. It is the mechanism `error`'s "No
//! `request_id` field here, deliberately" section defers to, and it is what
//! makes `Category::Internal`'s "Contact support with the request id"
//! something a merchant can actually do — through either header name.

use std::borrow::Cow;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{FromRef, MatchedPath, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderName, Method, Request, Uri};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::{Router, http::StatusCode, routing::get, routing::post};
use serde_json::{Map, Value, json};
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::Span;
use vpay_core::metrics::{HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL};
use vpay_db::Repositories;
use vpay_provider::ProviderAdapter;

pub mod browser;
pub mod error;
pub mod form;
pub mod idempotency;
mod jwks_cache;
pub mod model;
// The second listener: `/livez` and `/metrics`, on `--observability-bind`,
// mounted by BOTH binaries and by neither's traffic router. It lives in this
// crate rather than being written twice in `main.rs` because it is
// mechanism, not policy — and because the test that keeps those two paths
// *off* `router()` below belongs next to the module that owns them.
pub mod observability;
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

pub use browser::{BROWSER_ROUTES, PayerScope};
pub use error::ApiError;
pub use resource_auth::MerchantJwtValidator;
/// The boot steps both binaries call, under the name to call them by.
///
/// The module itself is [`v1::boot`] for historical reasons; nothing about
/// keying adapters, loading YAML, migrating or reconciling belongs to the `/v1`
/// wire surface, and a boot path spelled `vpay_api::v1::boot` reads as if it
/// did.
pub use v1::boot;
pub use v1::{
    MerchantScope, ResourceConfig, SCOPE_PAYMENTS_READ, SCOPE_PAYMENTS_WRITE, V1_ROUTES, V1Route,
    WebhookEndpointConfig, required_scopes,
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

/// The *second* name the same request id goes out under, for the benefit of
/// clients that only look for Stripe's spelling.
///
/// stripe-node reads `response.headers['request-id']` and nothing else when
/// it populates `err.requestId` and `obj.lastResponse.requestId`
/// (`RequestSender.ts`). A merchant driving vpay with the official Stripe
/// SDK — the point of `docs/plans/2026-09-03-step5b-stripe-sdk.md` — would
/// otherwise hold an `undefined` request id while
/// [`Category::Internal`](vpay_core::Category::Internal)'s public sentence
/// tells them to contact support with it.
///
/// **Response-only, and deliberately not a second id.** The request is
/// correlated by `x-request-id` alone: that is the name a reverse proxy
/// sets, the name [`make_request_span`] records, and the name
/// [`discard_unusable_request_id`] vets. This header is a copy of that one
/// value on the way out, so the two can never name different requests. It is
/// not accepted as *input* for the same reason — a caller who set both would
/// otherwise be asking which one wins.
///
/// A `HeaderName` rather than a `&str` like its sibling above because it is
/// only ever *written*: `HeaderMap::insert` wants the name, and
/// `HeaderName::from_static` in a `const` makes an invalid spelling a
/// compile error instead of a runtime panic (ADR-0007).
const STRIPE_REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("request-id");

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
/// `JwtValidator` only its validation policy, and `vpay_db`'s repository
/// handle names its own type and nothing else.
#[derive(Debug)]
pub struct RouterDeps {
    /// Everything this router may ask Postgres to do (`vpay_db`). `/healthz`
    /// probes it, `/v1/oauth/jwks.json` reads the published key set from it,
    /// and the OP's client store consults `disabled_clients` through it.
    ///
    /// A trait object, so this crate names no `sqlx` type on the request
    /// path and a handler can be read without knowing which table family a
    /// query belongs to — the method says.
    pub repositories: Arc<dyn Repositories>,
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
    repositories: Arc<dyn Repositories>,
    merchant_op: std::sync::Arc<op::MerchantOp>,
    merchant_validator: MerchantJwtValidator,
    adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>,
    resource_config: Arc<ResourceConfig>,
}

/// So [`op::jwks::jwks_handler`] can take `State<Arc<dyn Repositories>>` and
/// stay independent of how this router is assembled — its own doc comment
/// names this impl as the assembler's side of that contract.
impl FromRef<AppState> for Arc<dyn Repositories> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.repositories)
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
/// **This is now the readiness half of a real split** (Step 6). The
/// paragraph that used to stand here said the endpoint was deliberately
/// *not* split, because nothing in the repository defined a Kubernetes
/// liveness vs. readiness probe and a `/readyz` nobody polls proves nothing.
/// `deploy/helm/vpay` is that consumer: `deployment-server.yaml` wires
/// `readinessProbe` → `GET /healthz` on the traffic port and
/// `livenessProbe` → `GET /livez` on the observability port. So the split
/// happened, exactly as that paragraph specified it should — and it happened
/// on **two ports**, not two paths.
///
/// `/healthz` keeps the database check and keeps its meaning: a pod whose
/// Postgres is unreachable should stop receiving traffic. It is deliberately
/// *not* the liveness probe, because a liveness probe that fails on a
/// database outage restarts every pod in the deployment, repeatedly, and a
/// restart cannot fix a database.
///
/// Liveness is [`crate::observability`]'s `/livez`, a static `"ok"` on
/// `--observability-bind`. It is not mounted here and must not be: see that
/// module's header for why `/metrics` cannot share a port with the surface
/// an Ingress fronts, and
/// `neither_livez_nor_metrics_is_reachable_on_the_traffic_router` below for
/// the test that keeps it off.
///
/// Nothing about `/healthz` itself changed. CI's readiness gate
/// (`.github/workflows/ci.yml`) and `compose.e2e.yml` both depend on its
/// current meaning, and moving the DB check off it would break them
/// silently.
async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match state.repositories.check_connection().await {
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

/// The label [`HTTP_REQUESTS_TOTAL`] carries for a request that matched no
/// route.
///
/// A bounded stand-in for an unbounded set. The alternative — falling back
/// to `uri().path()` — hands anyone who can reach the port the ability to
/// create a new time series per request, which is the classic way to fill a
/// metrics store from the outside.
///
/// `pub` for the reason [`OTHER_METHOD`] is: these two are the label values
/// an operator reads on a dashboard, and a caller outside this crate that
/// wants to assert on one should name the constant rather than re-spell the
/// string.
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// The label [`HTTP_REQUESTS_TOTAL`] carries for a request whose method is
/// not one of the ten [`Method`] constants.
///
/// [`UNMATCHED_ROUTE`]'s reasoning, on the other label of the same series.
/// RFC 9110 §9.1 makes a method any token, `http::Method` parses one into
/// its extension form rather than rejecting it, and axum routes the request
/// — so `M12345 /healthz`, unauthenticated, on a loop, mints one time
/// series per request. Bounding `route` and leaving `method` free closed
/// one half of the same hole.
///
/// `pub` so the label an operator sees in a dashboard has a name in code,
/// and so a `/metrics` assertion outside this crate can be written against
/// the constant rather than against a copy of the string.
pub const OTHER_METHOD: &str = "other";

/// The ten methods this router labels verbatim, and the constants those
/// ten labels are checked against.
///
/// A `static` rather than a `const` so [`bounded_method`] can hand out
/// `&'static str` from it: the point of returning `http`'s own spelling is
/// that a dashboard's `method="GET"` cannot drift from what `Method::GET`
/// renders, and a copy of the literal here would be exactly that drift.
static STANDARD_METHODS: [Method; 10] = [
    Method::GET,
    Method::HEAD,
    Method::POST,
    Method::PUT,
    Method::DELETE,
    Method::CONNECT,
    Method::OPTIONS,
    Method::TRACE,
    Method::PATCH,
    Method::QUERY,
];

/// A request method as a bounded label value: one of [`STANDARD_METHODS`]
/// verbatim, or [`OTHER_METHOD`].
///
/// A linear scan over ten `&'static` constants, on a path that already
/// awaited a whole HTTP response; the alternative — a `match` on
/// `method.as_str()` with ten string literals — would re-spell the labels
/// in this file, which is the drift the `static` above exists to prevent.
fn bounded_method(method: &Method) -> &'static str {
    STANDARD_METHODS
        .iter()
        .find(|standard| *standard == method)
        .map_or(OTHER_METHOD, Method::as_str)
}

/// Counts and times one HTTP response: the single seam for
/// [`HTTP_REQUESTS_TOTAL`] and [`HTTP_REQUEST_DURATION_SECONDS`].
///
/// # Why a `from_fn` middleware and not `TraceLayer`'s `on_response`
///
/// `on_response` is handed the response, the elapsed time and the span — but
/// not the request, and a `route` label has to come from the request's
/// [`MatchedPath`] extension. Reading it back off the span is not possible
/// (`tracing` fields are write-only), and recording the route into a span
/// field so a callback could read it would be a second mechanism to keep
/// correct. A middleware that holds the request, awaits the inner service
/// and then records is one function that can see both halves.
///
/// # Why the route pattern and never the path
///
/// `/v1/payment_intents/pi_3Nk…` as a label value would mint a time series
/// per payment intent, and a metrics store's cardinality is the one resource
/// a payment gateway's own success exhausts. The pattern
/// (`/v1/payment_intents/{id}`) is a fixed, small set — the route table —
/// and it is what a dashboard actually groups by.
///
/// `method` is bounded for the same reason and was not always: see
/// [`bounded_method`] and [`OTHER_METHOD`]. Every label on this series now
/// comes from a closed set — the route table, ten methods, and a status
/// code.
///
/// # Where this is mounted, and why in three places rather than one
///
/// axum inserts `MatchedPath` **after** routing, and a layer added with
/// `Router::layer` runs after that router's own routing but *before* a
/// nested router's. For a request into a nest, the outer router therefore
/// has no `MatchedPath` to offer at all: `axum::extract::matched_path`
/// stores a private `MatchedNestedPath` instead whenever the matched pattern
/// ends in the nest's tail-capture parameter, precisely so that a
/// half-resolved pattern cannot be mistaken for a real one. Mounted only on
/// the outermost router, this middleware would therefore label every `/v1`
/// request `unmatched`.
///
/// So it is mounted three times inside [`router`] — once on the outer
/// router's own routes, once inside the `/v1/oauth` nest and once inside the
/// `/v1` nest — and each copy sees the pattern its own router matched. The
/// outer mount is applied *before* the two nests are added, which is what
/// keeps a `/v1` request from being counted twice: `Router::layer` wraps
/// only the routes that already exist ("Additional routes added after
/// `layer` is called will not have the middleware added"). That ordering is
/// load-bearing and `every_mounted_group_is_counted_exactly_once` fails if
/// someone reorders it.
///
/// Inside `/v1` it is applied *outside* the authentication layer, so a `401`
/// — the response a confused integrator is most likely to be holding — is
/// counted against the route it was aimed at rather than disappearing into
/// `unmatched`.
///
/// # What is not counted here
///
/// `/livez` and `/metrics`: they are [`observability`]'s router on a
/// different port. A scraper polling every 15 seconds would otherwise be the
/// largest traffic source on the series, and it is not traffic anyone wants
/// to see on a dashboard of merchant requests.
async fn track_http_metrics(request: Request<Body>, next: Next) -> Response {
    // Owned before the request is consumed by the inner service; both are
    // small and bounded (a route pattern from the table, a method).
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| UNMATCHED_ROUTE.to_owned(), |m| m.as_str().to_owned());
    let method = bounded_method(request.method());

    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed = started.elapsed();
    let status = response.status().as_u16().to_string();

    metrics::counter!(
        HTTP_REQUESTS_TOTAL,
        "route" => route.clone(),
        "method" => method,
        "status" => status.clone(),
    )
    .increment(1);
    metrics::histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        "route" => route,
        "method" => method,
        "status" => status,
    )
    .record(elapsed.as_secs_f64());

    response
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

/// Copies the request id onto the response a second time, under
/// [`STRIPE_REQUEST_ID_HEADER`].
///
/// **Why not a second [`PropagateRequestIdLayer`].** That layer reads the
/// header it is named with *off the request* and writes it back under the
/// same name (tower-http `request_id.rs`); no request carries `request-id`,
/// so a `PropagateRequestIdLayer::new("request-id")` would find nothing and
/// propagate nothing. The only way to get tower-http to emit it would be a
/// second [`SetRequestIdLayer`], which mints its own UUID — two headers
/// naming two different requests, which is worse than one header.
///
/// So the value is read from the request's `x-request-id` rather than from
/// the response's: mounted below [`SetRequestIdLayer`] the request always
/// carries one, and taking it from the same place [`make_request_span`] does
/// is what makes the log line, `x-request-id` and `request-id` provably the
/// same string rather than three things that happen to agree.
///
/// `insert`, not `append`: a handler that had already set `request-id` would
/// be answering with an id this router did not mint, and exactly one value
/// is what a client reading `headers['request-id']` can act on.
async fn mirror_request_id_header(request: Request<Body>, next: Next) -> Response {
    let request_id = request.headers().get(REQUEST_ID_HEADER).cloned();
    let mut response = next.run(request).await;
    if let Some(request_id) = request_id {
        response
            .headers_mut()
            .insert(STRIPE_REQUEST_ID_HEADER, request_id);
    }
    response
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
/// Three route groups, and which group a path falls into is the whole security
/// boundary of this process: the unauthenticated ones (`/healthz`, the whole
/// `/v1/oauth` OP subtree, the two `/v1/browser` payer routes), everything else
/// under `/v1` behind [`require_merchant_token`], and the honest 404 for
/// anything else. `/livez` and `/metrics` are **not** served here at all — they
/// belong to [`observability`], on a different port, because `/metrics` names
/// every rail, route pattern and error code this deployment has.
///
/// Both nested routers carry their own `.fallback(not_found)`. That is
/// load-bearing rather than decorative: without it axum flattens the nest into
/// the outer path table and an unmatched `/v1/oauth/...` path matches
/// `/v1/{*rest}` — the *authenticated* nest — and answers 401 to a caller whose
/// entire reason for being there is that it has no token yet.
/// `the_oauth_nest_answers_its_own_404` fails with `left: 401, right: 404` if
/// the fallback is removed.
///
/// Five middleware layers wrap everything, and their order is load-bearing:
/// vet the caller's own `x-request-id`, mint one if none survived, mirror it
/// onto the response under Stripe's `request-id` spelling, open the span, then
/// propagate. That is what makes [`vpay_core::Category::Internal`]'s "Contact
/// support with the request id" a promise a merchant can act on.
///
/// The full route table, the three answers that make the auth boundary
/// observable, why `Router::layer` and not `route_layer`, and what each
/// middleware layer must sit above:
/// [docs/reference/vpay-api.md § the router](../../../../docs/reference/vpay-api.md#the-router).
pub fn router(deps: RouterDeps) -> Router {
    let state = AppState {
        repositories: deps.repositories,
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
        .fallback(not_found)
        // Inside the nest, because that is the only place the OP's own route
        // patterns exist — see `track_http_metrics`.
        .layer(from_fn(track_http_metrics));

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
        .layer(RequestBodyLimitLayer::new(V1_BODY_LIMIT_BYTES))
        // Outermost of the three, and inside this nest rather than on the
        // outer router: a `/v1` request's route pattern only exists once
        // this router has matched, and a 401 decided by the layer above must
        // still be counted against the route it was aimed at. See
        // `track_http_metrics`.
        .layer(from_fn(track_http_metrics));

    // Unauthenticated by necessity too, and for a different reason from the
    // OP's: the caller is a payer's browser, which has no merchant
    // credential and must never be given one. What authorises a request here
    // is the payment intent's own `client_secret` — see `browser`.
    //
    // Mounted *before* `/v1`, though axum's router is order-independent for
    // distinct prefixes: `Router::nest` registers `/v1/browser` and
    // `/v1/browser/{*rest}` as their own entries, which match more
    // specifically than `/v1/{*rest}`, so a browser path never reaches the
    // authenticated nest. The nest's own `.fallback` is what makes that true
    // for an *unmatched* browser path as well — see `browser::routes`.
    let browser = browser::routes()
        // On this nest only. A payer's page is served from the merchant's own
        // origin, so every call it makes here is cross-origin and would
        // otherwise be blocked by the browser before it left. The merchant
        // `/v1` nest gets no `CorsLayer` at all: nothing legitimate calls it
        // from a browser, and a permissive header there would invite a
        // merchant to put a bearer token in a page.
        .layer(
            CorsLayer::new()
                // `Any`, with credentials OFF (the default, and asserted by
                // `a_browser_preflight_allows_any_origin_without_credentials`).
                // A merchant's checkout can be on any domain they own, vpay
                // has no list of them, and the browser's own rules forbid
                // `Access-Control-Allow-Origin: *` together with credentials
                // — which is exactly the combination that would make a
                // wildcard dangerous. Nothing here reads a cookie, so there
                // is no ambient authority for an origin to borrow.
                .allow_origin(Any)
                // Exactly what `BROWSER_ROUTES` answers, plus the preflight
                // itself.
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                // The only header `@vpay/stripe-js` sets. Notably **not**
                // `Idempotency-Key` or `Authorization`: allowing either would
                // invite a browser to send one, and sending one is what turns
                // a simple request into a preflighted one (§0 S4).
                .allow_headers([CONTENT_TYPE])
                .max_age(Duration::from_secs(600)),
        )
        // Inside the nest, for the same reason as the other two: a browser
        // path's route pattern (`/v1/browser/payment_intents/{id}`, not the
        // raw id) only exists once this nest has matched, and mounting the
        // layer here rather than on the outer router is what keeps a browser
        // request from being double-counted or labelled `unmatched`. See
        // `track_http_metrics` and the ordering guard below.
        .layer(from_fn(track_http_metrics));

    Router::new()
        .route("/healthz", get(healthz))
        .fallback(not_found)
        // **Before the two nests, and that ordering is the whole reason a
        // `/v1` request is not counted twice**: `Router::layer` wraps the
        // routes that exist when it is called and nothing added afterwards,
        // so this copy covers `/healthz` and the outer 404 only, while each
        // nest carries its own. Moving this line below the nests would
        // double every `/v1` count and label half of them `unmatched`.
        .layer(from_fn(track_http_metrics))
        .nest("/v1/oauth", oauth)
        .nest("/v1/browser", browser)
        .nest("/v1", v1)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(from_fn(discard_unusable_request_id))
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(from_fn(mirror_request_id_header))
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

    /// Renders a real Prometheus scrape of whatever `body` recorded.
    ///
    /// The **shipping** exporter over a *local* recorder — see
    /// `vpay_core::metrics`' own tests for why both halves of that matter.
    /// Asserting on rendered text means a label spelled wrongly fails here
    /// for the same reason a dashboard would be empty.
    fn scrape_of(body: impl FnOnce()) -> String {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, body);
        handle.render()
    }

    /// Drives one request through the real router under a local recorder and
    /// returns `(status, scrape)`.
    ///
    /// The runtime is built inside the recorder's scope and on one thread:
    /// `with_local_recorder` installs a *thread-local*, so a multi-threaded
    /// runtime could poll the middleware on a thread that cannot see it and
    /// the test would silently assert on an empty document.
    fn request_and_scrape(method: &str, uri: &str) -> (StatusCode, String) {
        let mut status = StatusCode::IM_A_TEAPOT;
        let scrape = scrape_of(|| {
            let response = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime builds")
                .block_on(async {
                    router(deps())
                        .oneshot(
                            Request::builder()
                                .method(method)
                                .uri(uri)
                                .body(Body::empty())
                                .expect("valid request"),
                        )
                        .await
                        .expect("router does not fail to serve")
                });
            status = response.status();
        });
        (status, scrape)
    }

    /// The label `vpay_http_requests_total` carries for a healthz probe —
    /// the series `deploy/helm/vpay`'s dashboards and the server's own
    /// `tests/cli.rs` both name.
    #[test]
    fn a_healthz_probe_is_counted_under_its_own_route_pattern() {
        let (status, scrape) = request_and_scrape("GET", "/healthz");
        assert!(
            scrape.contains(&format!(
                r#"vpay_http_requests_total{{route="/healthz",method="GET",status="{}"}} 1"#,
                status.as_u16()
            )),
            "{scrape}"
        );
        assert!(
            scrape.contains(&format!(
                r#"vpay_http_request_duration_seconds_count{{route="/healthz",method="GET",status="{}"}} 1"#,
                status.as_u16()
            )),
            "the histogram must observe the same request: {scrape}"
        );
    }

    /// **The reason `track_http_metrics` is mounted inside the nests**: a
    /// `/v1` request is labelled with the route *pattern*, not the concrete
    /// path and not `unmatched`.
    ///
    /// The id in the URI is a literal that no route table entry contains, so
    /// a scrape naming it would be a per-object time series — the
    /// cardinality failure the pattern exists to prevent. It is a `401`
    /// (no bearer token), which is the other half: the count happens outside
    /// the authentication layer, so the response a confused integrator is
    /// most likely holding is counted against the route they aimed at.
    #[test]
    fn a_v1_request_is_counted_under_the_route_pattern_and_not_the_path() {
        let (status, scrape) = request_and_scrape("GET", "/v1/payment_intents/pi_3NkExAmPlE");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            scrape.contains(
                r#"vpay_http_requests_total{route="/v1/payment_intents/{id}",method="GET",status="401"} 1"#
            ),
            "{scrape}"
        );
        assert!(
            !scrape.contains("pi_3NkExAmPlE"),
            "a concrete object id must never become a label value: {scrape}"
        );
    }

    /// The OP subtree is a second nest and carries its own copy of the
    /// middleware, so its patterns are real rather than `unmatched`.
    #[test]
    fn an_oauth_request_is_counted_under_the_ops_own_route_pattern() {
        let (_, scrape) = request_and_scrape("GET", "/v1/oauth/.well-known/openid-configuration");
        assert!(
            scrape.contains(
                r#"vpay_http_requests_total{route="/v1/oauth/.well-known/openid-configuration",method="GET",status="200"} 1"#
            ),
            "{scrape}"
        );
    }

    /// A path nothing routes is counted once, under the bounded label.
    ///
    /// Falling back to `uri().path()` here is the classic way to let anyone
    /// who can reach the port mint a time series per request; this asserts
    /// the path does not appear at all.
    #[test]
    fn an_unrouted_path_is_counted_under_a_bounded_label() {
        let (status, scrape) = request_and_scrape("GET", UNROUTED_PATH);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            scrape.contains(
                r#"vpay_http_requests_total{route="unmatched",method="GET",status="404"} 1"#
            ),
            "{scrape}"
        );
        assert!(
            !scrape.contains(UNROUTED_PATH),
            "a caller-supplied path must never become a label value: {scrape}"
        );
    }

    /// **The cardinality guard on the `method` label.** An extension method
    /// — RFC 9110 §9.1 lets a method be any token, and `http::Method` parses
    /// one rather than rejecting it — is counted under [`OTHER_METHOD`], and
    /// the caller's own text never reaches the scrape.
    ///
    /// Driven through the *real* router rather than through
    /// [`bounded_method`] directly, because the defect this pins was not in a
    /// mapping function (there was none): it was `request.method().to_string()`
    /// at the seam. A revert to that line puts `M12345` back in the render and
    /// fails the second assertion; a revert that keeps a mapping function but
    /// stops calling it fails the first.
    ///
    /// The method is unauthenticated and unroutable, so this is also the shape
    /// of the attack: anyone who can open a socket to the traffic port can
    /// repeat it with a fresh token every time.
    #[test]
    fn an_extension_method_is_counted_under_a_bounded_label() {
        const EXTENSION_METHOD: &str = "M12345";

        let (status, scrape) = request_and_scrape(EXTENSION_METHOD, "/healthz");
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "the router must still answer this request: {scrape}"
        );
        assert!(
            scrape.contains(&format!(
                r#"vpay_http_requests_total{{route="/healthz",method="{OTHER_METHOD}",status="405"}} 1"#
            )),
            "{scrape}"
        );
        assert!(
            !scrape.contains(EXTENSION_METHOD),
            "a caller-supplied method must never become a label value — one new time series \
             per request, from an unauthenticated caller: {scrape}"
        );
    }

    /// The ten labels [`bounded_method`] hands out are `http`'s own
    /// spellings, and the tenth case is [`OTHER_METHOD`].
    ///
    /// The drift this pins is a re-spelled literal (`"Get"`, `"get"`) in
    /// [`STANDARD_METHODS`]' place: it would not fail any request, it would
    /// silently split one dashboard series in two.
    #[test]
    fn every_standard_method_is_labelled_with_its_own_spelling() {
        for method in [
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::CONNECT,
            Method::OPTIONS,
            Method::TRACE,
            Method::PATCH,
            Method::QUERY,
        ] {
            assert_eq!(
                bounded_method(&method),
                method.as_str(),
                "{method} must be labelled verbatim"
            );
        }
        assert_eq!(
            STANDARD_METHODS.len(),
            10,
            "all ten of http::Method's constants, or the set is not the standard one"
        );

        let extension = Method::from_bytes(b"M12345").expect("an extension method parses");
        assert_eq!(bounded_method(&extension), OTHER_METHOD);
    }

    /// **The ordering guard.** `track_http_metrics` is mounted four times —
    /// once on the outer router, and once inside each of the three nests
    /// (`oauth`, `browser`, `v1`) — and the outer mount is applied before any
    /// `.nest(...)` call precisely so a request handled by a nest is not
    /// wrapped twice. Move that `.layer(...)` line below the `.nest(...)`
    /// calls and every count here becomes `2`.
    ///
    /// One request per group, all in one recorder, so this also proves the
    /// four groups produce four distinct series rather than one shared one.
    #[test]
    fn every_mounted_group_is_counted_exactly_once() {
        let scrape = scrape_of(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime builds");
            for uri in [
                "/healthz",
                UNROUTED_PATH,
                "/v1/payment_intents",
                "/v1/oauth/jwks.json",
                // The browser nest's own mount of `track_http_metrics`: no
                // `key`/`client_secret` query is supplied, so this answers a
                // 4xx from `authenticate` — but it is still counted, and
                // under the *pattern*, exactly like the `/v1` case above.
                "/v1/browser/payment_intents/pi_3NkExAmPlE",
            ] {
                runtime.block_on(async {
                    router(deps())
                        .oneshot(
                            Request::builder()
                                .uri(uri)
                                .body(Body::empty())
                                .expect("valid request"),
                        )
                        .await
                        .expect("router does not fail to serve")
                });
            }
        });

        let counted: Vec<&str> = scrape
            .lines()
            .filter(|line| line.starts_with("vpay_http_requests_total{"))
            .collect();
        assert_eq!(
            counted.len(),
            5,
            "five requests must produce five distinct series, once each:\n{scrape}"
        );
        for line in counted {
            assert!(
                line.ends_with(" 1"),
                "a request counted more than once — is the outer .layer() still applied \
                 *before* the two .nest() calls? line: {line}"
            );
        }
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

    /// The browser surface is outside the merchant boundary, and its
    /// unmatched paths are answered by its *own* fallback.
    ///
    /// Decisive twice over. Delete `.fallback(not_found)` from
    /// `browser::routes` and the first path answers **401** — axum flattens
    /// the nest and `/v1/{*rest}` catches it — which would tell a payer's
    /// browser to present a bearer token it can never hold. Move
    /// `.nest("/v1/browser", …)` after `.nest("/v1", …)` and nothing changes,
    /// which is why the ordering comment in [`router`] says the specificity
    /// is what does the work rather than the order.
    ///
    /// The two *real* browser paths are asserted as "not 401" rather than as
    /// a status: against the fixture's lazy pool a `GET` with no credential
    /// is the uniform 404 decided before any read, and the property under
    /// test is that the request reached the handler at all.
    #[tokio::test]
    async fn the_browser_nest_is_outside_the_merchant_boundary_and_answers_its_own_404() {
        for path in [
            "/v1/browser/not_a_route",
            // The two writes this surface must never offer, spelled the way
            // a Stripe integration would reach for them.
            "/v1/browser/payment_intents",
            "/v1/browser/payment_intents/pi_x/cancel",
        ] {
            let response = get(path).await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{path} must answer the browser nest's own honest 404"
            );
        }

        for route in BROWSER_ROUTES {
            let path = format!("/v1/browser{}", route.path.replace("{id}", "pi_x"));
            let response = get(&path).await;
            assert_ne!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must not require a merchant token a payer cannot have"
            );
        }
    }

    /// CORS is on the browser nest and **only** there.
    ///
    /// A preflight for the merchant `/v1` answering with an
    /// `access-control-allow-origin` would invite a merchant to call it from
    /// a page — which means putting a bearer token in one. The absence is the
    /// assertion that matters here; the browser half's real proof (a live
    /// `OPTIONS` reaching a mounted server) is in
    /// `backends/tests/integration/tests/browser_checkout.rs`.
    #[tokio::test]
    async fn cors_is_mounted_on_the_browser_nest_and_on_no_other() {
        async fn preflight(uri: &str) -> Response {
            let request = Request::builder()
                .method("OPTIONS")
                .uri(uri)
                .header("origin", "https://shop.example")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .expect("valid request");
            router(deps())
                .oneshot(request)
                .await
                .expect("router does not fail to serve")
        }

        let browser = preflight("/v1/browser/payment_intents/pi_x/confirm").await;
        assert_eq!(
            browser
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some("*"),
            "a payer's page is on the merchant's origin; without this the request never leaves"
        );
        assert!(
            browser
                .headers()
                .get("access-control-allow-credentials")
                .is_none(),
            "`allow_origin(Any)` is only safe with credentials off"
        );

        for uri in [
            "/v1/payment_intents",
            "/v1/payment_intents/pi_x/confirm",
            "/v1/oauth/token",
            "/healthz",
        ] {
            let response = preflight(uri).await;
            assert!(
                response
                    .headers()
                    .get("access-control-allow-origin")
                    .is_none(),
                "{uri} must carry no CORS header: nothing legitimate calls it from a browser"
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

    /// The decisive test for step-6 decision (9): `/livez` and `/metrics`
    /// live on [`crate::observability`]'s own listener
    /// (`--observability-bind`, default `0.0.0.0:9090`) and are **not**
    /// reachable on the port an Ingress fronts.
    ///
    /// `/metrics` is the one that matters. It names every rail this
    /// deployment talks to, every route pattern it serves and every error
    /// code it has produced; on the public port that is an operational map
    /// handed to anyone who can reach `/healthz`. The chart's
    /// NetworkPolicy admits the observability port from the monitoring
    /// namespace only, and that policy is only expressible because the two
    /// ports are different — so mounting either path here would silently
    /// undo it.
    ///
    /// A 404, not a 401: neither path is under `/v1`, so the honest answer
    /// is "this router does not serve that", and asserting the *code* rather
    /// than merely "not 200" is what would fail if someone put them behind
    /// the merchant boundary instead of leaving them off.
    #[tokio::test]
    async fn neither_livez_nor_metrics_is_reachable_on_the_traffic_router() {
        for path in ["/livez", "/metrics", "/v1/livez", "/v1/metrics"] {
            let status = get(path).await.status();
            let expected = if path.starts_with("/v1/") {
                // Under the merchant boundary, an unauthenticated request is
                // refused before routing — which is also "not served here".
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::NOT_FOUND
            };
            assert_eq!(
                status, expected,
                "{path} must not be served by the traffic router; it belongs to \
                 vpay_api::observability on --observability-bind"
            );
        }
    }

    /// …and neither is in [`V1_ROUTES`], which is the list `v1::routes`
    /// folds into the authenticated nest. The check above would already
    /// fail if one were mounted, but this one names the cause: a route added
    /// to that table is mounted by construction, so the table is where the
    /// mistake would be made.
    #[test]
    fn the_v1_route_table_carries_no_observability_path() {
        for route in V1_ROUTES {
            assert!(
                !route.path.contains("livez") && !route.path.contains("metrics"),
                "{} is an observability path and must not be under /v1",
                route.path
            );
        }
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

    /// The value under `request-id`, the header stripe-node reads.
    fn stripe_request_id_of(response: &Response) -> &str {
        response
            .headers()
            .get(STRIPE_REQUEST_ID_HEADER)
            .expect("every response carries a request-id header")
            .to_str()
            .expect("the request id is ascii")
    }

    /// Every response carries the request id under **both** names, with one
    /// value.
    ///
    /// The equality is the assertion, not the presence: a `request-id`
    /// header alone would also be produced by a second `SetRequestIdLayer`,
    /// which mints its own UUID — two headers naming two different requests,
    /// and a merchant quoting the one support cannot find. It is what fails
    /// if `mirror_request_id_header` is replaced by anything that does not
    /// read the id `x-request-id` settled on.
    ///
    /// Both the minted and the caller-supplied cases, because the layer that
    /// settles the id differs between them (`SetRequestIdLayer` mints only
    /// when the header is absent) and the mirror has to sit below both.
    #[tokio::test]
    async fn a_response_carries_the_request_id_under_both_names() {
        for supplied in [None, Some("abc-123")] {
            let response = router(deps())
                .oneshot(a_request(supplied))
                .await
                .expect("router does not fail to serve");

            let x_request_id = request_id_of(&response).to_owned();
            assert_eq!(
                stripe_request_id_of(&response),
                x_request_id,
                "the two headers must carry one value (supplied: {supplied:?})"
            );
            if let Some(supplied) = supplied {
                assert_eq!(x_request_id, supplied);
            } else {
                uuid::Uuid::parse_str(&x_request_id)
                    .unwrap_or_else(|e| panic!("{x_request_id:?} is not a uuid: {e}"));
            }
        }
    }

    /// A caller cannot choose the id by sending `request-id`: the mirror is
    /// an output, and `x-request-id` remains the one input.
    ///
    /// Without this, "two spellings of one header" would quietly become "two
    /// ways in", and a caller sending both would be asking which wins.
    #[tokio::test]
    async fn a_caller_supplied_stripe_request_id_header_is_not_honoured() {
        let request = Request::builder()
            .uri(UNROUTED_PATH)
            .header(STRIPE_REQUEST_ID_HEADER, "chosen-by-the-caller")
            .body(Body::empty())
            .expect("valid request");

        let response = router(deps())
            .oneshot(request)
            .await
            .expect("router does not fail to serve");

        let id = request_id_of(&response).to_owned();
        assert_ne!(id, "chosen-by-the-caller");
        assert_eq!(stripe_request_id_of(&response), id);
        uuid::Uuid::parse_str(&id)
            .unwrap_or_else(|e| panic!("a minted id was expected, got {id:?}: {e}"));
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
