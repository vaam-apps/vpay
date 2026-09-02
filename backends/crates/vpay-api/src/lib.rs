//! The Stripe-shaped HTTP surface.
//!
//! STATUS: `/healthz` and the Stripe-shaped 404 envelope are implemented.
//! No `/v1/*` route exists yet. See `docs/status.md` — this file must never
//! grow a route that returns fabricated data. A real database check (below)
//! is the opposite of fabricated data, so it stays.
//!
//! [`resource_auth`] adds bearer-token validation for `/v1` and `/dash/v1`
//! (OP-3) — a real `JwtValidator`/extractor pair, unit-tested against a real
//! JWKS server, but **not mounted onto any route here**. Nothing in this
//! file's `router()` changes as part of that: mounting `/v1`/`/dash/v1` is
//! later work that needs the OP assembled first.
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

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, Uri};
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use axum::{Router, http::StatusCode, routing::get};
use serde_json::{Map, Value, json};
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::Span;
use vpay_db::PgPool;

pub mod error;
pub mod resource_auth;
#[cfg(test)]
mod test_log;

pub use error::ApiError;

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

/// Shared state for every route in this router. Just the pool today; grows
/// as real routes land.
#[derive(Clone)]
struct AppState {
    pool: PgPool,
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

/// Builds the router. `pool` is required, not optional: every route this
/// binary serves — starting with `/healthz` — now depends on the database
/// being real, so a router without one would be a router that cannot tell
/// the truth about its own health.
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
/// Mounted with `Router::layer` after `fallback`, so it wraps the 404 path
/// too — the response most likely to be the one a confused integrator is
/// holding. Neither `/healthz`'s plain-text body nor the 404 envelope's
/// bytes change: these layers only add a response header and a span.
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(not_found)
        .with_state(AppState { pool })
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

    /// `connect_lazy` parses the URL and builds a pool without performing any
    /// I/O — no real Postgres, no testcontainer, no fake data returned by
    /// the route under test. This proves the router wires state through and
    /// the fallback still 404s; it does *not* claim to prove `/healthz`
    /// against a live database — that is `vpay-db`'s own
    /// `tests/postgres.rs`, against a real `postgres:16-alpine` container,
    /// and this crate does not duplicate that infrastructure.
    fn lazy_pool() -> PgPool {
        PgPool::connect_lazy("postgres://vpay:vpay@localhost:5432/vpay")
            .expect("connect_lazy performs no I/O and only fails on a malformed URL")
    }

    #[tokio::test]
    async fn unknown_routes_still_get_the_honest_404() {
        let app = router(lazy_pool());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/payment_intents")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router does not fail to serve");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A `GET` of a route that does not exist, which is the cheapest request
    /// that traverses the whole middleware stack *and* makes `ApiError` log
    /// on the way out — so one request can pin both the header behaviour and
    /// the span behaviour.
    fn a_request(request_id: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/v1/payment_intents");
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
        let response = router(lazy_pool())
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
        let response = router(lazy_pool())
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
            .uri("/v1/payment_intents")
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
        router(lazy_pool())
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
            .uri("/v1/payment_intents")
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
    async fn serve_capturing_log(request: Request<Body>) -> (Response, String) {
        let sink = test_log::CapturedLog::default();
        let guard =
            tracing::subscriber::set_default(test_log::captured_log_subscriber(sink.clone()));
        let response = router(lazy_pool())
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
