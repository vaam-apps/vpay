//! The `/livez` + `/metrics` listener, on its own port, on **both**
//! binaries.
//!
//! # Why this is a second listener and not two more routes
//!
//! `/metrics` is an operational map of the deployment: every rail it talks
//! to, every route pattern it serves, every error code it has produced, and
//! every webhook delivery outcome. `--bind`'s port is the one an
//! Ingress fronts and the internet reaches. Those two facts cannot both be
//! true of one port.
//!
//! Keeping them apart is also what makes the policy *expressible*:
//! `deploy/helm/vpay/templates/networkpolicy.yaml` admits the observability
//! port from the monitoring namespace only, and the chart's
//! `observability-port` guard refuses a values file that sets it equal to
//! `server.port`. A path-based exclusion on a shared port would depend on
//! the ingress controller's rule ordering staying correct forever; a
//! separate port does not.
//!
//! [`crate::router`] therefore mounts neither path, and
//! `neither_livez_nor_metrics_is_reachable_on_the_traffic_router` in that
//! module fails if either ever appears there.
//!
//! # Why `/livez` is static and `/healthz` is not
//!
//! `/healthz` runs a real `SELECT 1` (see [`crate::router`]'s own notes) and
//! is the **readiness** probe: a pod whose database is unreachable should
//! stop receiving traffic. `/livez` is the **liveness** probe and answers a
//! constant, because a liveness probe that fails on a database outage
//! restarts every pod in the deployment — repeatedly — and a restart cannot
//! fix a database. The `vpay-api` doc comment on `healthz` pre-authorised
//! exactly this split; the chart is the consumer that made it real
//! (`deployment-server.yaml`: readiness `/healthz` on 8080, liveness
//! `/livez` on 9090).
//!
//! "The process finished booting" is carried by *when this listener is
//! bound*, not by anything the handler checks. Both binaries bind it after
//! every fallible startup step — config, signing key, database, migrations,
//! boot step 4 — so a probe against a process still starting up, or one that
//! is about to exit 78, gets a connection refusal rather than a cheerful
//! `200`. That is the honest answer, and it is the reason the handler needs
//! no state at all.
//!
//! # Why the renderer is a closure and not a `PrometheusHandle`
//!
//! Installing a recorder is an application decision — the same one
//! `install_crypto_provider()` is (see `vpay_core::metrics`' header for the
//! full argument). Taking `impl Fn() -> String` keeps
//! `metrics-exporter-prometheus` out of this crate's dependency graph, so
//! the choice of exporter stays in the two `main.rs` files that make it, and
//! it lets this module's own tests exercise the routes without installing a
//! process-global recorder that would then leak into every other test in the
//! binary.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

/// The `Content-Type` a Prometheus scrape expects for the text exposition
/// format.
///
/// Spelled in full — including `version=0.0.4` and `charset=utf-8` — because
/// a scraper that receives a bare `text/plain` will still parse it, and one
/// that receives `application/json` will not; being exact here is free and
/// the failure it prevents is silent.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// What the `/metrics` handler calls to produce the exposition text.
///
/// `Arc<dyn Fn>` rather than a generic parameter on the router: axum's state
/// must be `Clone`, and a boxed closure keeps [`router`] a plain function
/// instead of a generic one every caller has to name a type argument for.
#[derive(Clone)]
struct Renderer(Arc<dyn Fn() -> String + Send + Sync>);

// `Renderer` holds a closure, which has no useful `Debug`. Hand-written
// rather than derived so the crate-wide `missing_debug_implementations` warn
// is satisfied by something true.
impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Renderer(<closure>)")
    }
}

/// The observability router: `GET /livez` and `GET /metrics`, and nothing
/// else.
///
/// `render` is called once per scrape and must produce the Prometheus text
/// exposition format — in both binaries that is
/// `PrometheusHandle::render()`.
///
/// There is deliberately **no fallback and no authentication**. No
/// fallback, because an unmatched path here should be axum's bare 404 and
/// not [`crate::ApiError`]'s merchant-facing envelope: this surface has no
/// merchants, and rendering the API's error shape on it would invite
/// someone to treat it as part of that API. No authentication, because the
/// port is reachable only from inside the cluster (the NetworkPolicy above)
/// and a credential this listener could check would have to be mounted into
/// every pod for a scraper to use — a Secret that buys nothing the network
/// boundary does not already give.
pub fn router(render: impl Fn() -> String + Send + Sync + 'static) -> Router {
    Router::new()
        .route("/livez", get(livez))
        .route("/metrics", get(metrics))
        .with_state(Renderer(Arc::new(render)))
}

/// Serves [`router`] on an already-bound listener until `shutdown`
/// resolves.
///
/// The listener is bound by the caller, not here, for two reasons that are
/// both about honesty: a `:0` bind has to have its real address read back
/// and logged before anything can reach it, and — more importantly — *when*
/// the bind happens is the entire meaning of `/livez`. Both binaries bind
/// after their last fallible startup step, so a probe against a process that
/// is still starting, or one that is about to exit `78`, is refused rather
/// than answered `ok`.
///
/// `shutdown` is the same signal the process's own drain observes, so this
/// listener stops accepting when the rest of the process does instead of
/// outliving it as a detached task. There is deliberately no grace bound
/// *inside* this function: a scrape is a few milliseconds of work with no
/// payment consequence, and the caller — which owns
/// `--shutdown-grace-seconds` — is where a bound belongs.
///
/// # Errors
///
/// Whatever `axum::serve` returns, i.e. an accept-loop I/O failure. A
/// caller that is already shutting down should log it rather than change
/// its exit code: the observability port failing is not a payment failure.
pub async fn serve(
    listener: tokio::net::TcpListener,
    render: impl Fn() -> String + Send + Sync + 'static,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(render))
        .with_graceful_shutdown(shutdown)
        .await
}

/// The liveness probe: a constant.
///
/// Takes no `State`, touches no database, and that is the property worth
/// keeping — see this module's header. If this ever grows a dependency, a
/// failure of that dependency becomes a rolling restart of the whole
/// deployment.
async fn livez() -> &'static str {
    "ok"
}

/// The Prometheus scrape endpoint.
async fn metrics(State(renderer): State<Renderer>) -> Response {
    let body = (renderer.0)();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    use super::{PROMETHEUS_CONTENT_TYPE, router};

    async fn get(path: &str, render: impl Fn() -> String + Send + Sync + 'static) -> (u16, String) {
        let response = router(render)
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("the observability router does not fail to serve");
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("the body is small");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn livez_is_a_static_ok_and_needs_no_state() {
        let (status, body) = get("/livez", || {
            panic!("/livez must never call the renderer — it takes no state at all")
        })
        .await;
        assert_eq!(status, 200);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn metrics_returns_what_the_renderer_produced() {
        let (status, body) = get("/metrics", || "# HELP vpay_build_info x\n".to_owned()).await;
        assert_eq!(status, 200);
        assert_eq!(body, "# HELP vpay_build_info x\n");
    }

    /// A scraper decides how to parse a response from its `Content-Type`.
    /// Pinned as a literal because getting it wrong produces an empty target
    /// in Prometheus rather than an error anywhere.
    #[tokio::test]
    async fn metrics_is_served_as_the_prometheus_text_exposition_format() {
        let response = router(String::new)
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("the observability router does not fail to serve");
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some(PROMETHEUS_CONTENT_TYPE)
        );
    }

    /// This surface is exactly two routes. A third one appearing here would
    /// be a surface nothing documents, on a port with no authentication.
    #[tokio::test]
    async fn nothing_else_is_mounted_on_this_listener() {
        for path in ["/", "/healthz", "/v1/payment_intents", "/metrics/extra"] {
            let (status, _) = get(path, String::new).await;
            assert_eq!(status, 404, "{path} must not be routed on this listener");
        }
    }
}
