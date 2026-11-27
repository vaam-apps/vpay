//! A collector in the process, for tests: an OTLP/HTTP endpoint that keeps every request it receives. Behind the
//! feature `testkit`; in no image.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use serde_json::Value;
use tokio::task::JoinHandle;

/// Every request the collector got.
#[derive(Default)]
struct Received {
    traces: Vec<Value>,
    metrics: Vec<Value>,
    logs: Vec<Value>,
    bodies: Vec<String>,
    headers: Vec<HeaderMap>,
}

/// The collector. Dropping it stops it.
pub struct TestCollector {
    addr: SocketAddr,
    received: Arc<Mutex<Received>>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for TestCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestCollector")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl Drop for TestCollector {
    fn drop(&mut self) {
        self.task.abort();
    }
}

type Shared = Arc<Mutex<Received>>;

async fn take(
    shared: &Shared,
    headers: HeaderMap,
    body: &Bytes,
    store: impl FnOnce(&mut Received, Value),
) -> StatusCode {
    let Ok(document) = serde_json::from_slice::<Value>(body) else {
        return StatusCode::BAD_REQUEST;
    };
    let mut received = shared.lock().unwrap_or_else(PoisonError::into_inner);
    received
        .bodies
        .push(String::from_utf8_lossy(body).into_owned());
    received.headers.push(headers);
    store(&mut received, document);
    StatusCode::OK
}

impl TestCollector {
    /// Starts on a free loopback port.
    ///
    /// # Panics
    ///
    /// When no loopback port can be bound: a test cannot go on.
    #[allow(clippy::expect_used)]
    pub async fn start() -> Self {
        let received = Shared::default();
        let app = Router::new()
            .route(
                "/v1/traces",
                post(
                    |State(s): State<Shared>, h: HeaderMap, b: Bytes| async move {
                        take(&s, h, &b, |r, d| r.traces.push(d)).await
                    },
                ),
            )
            .route(
                "/v1/metrics",
                post(
                    |State(s): State<Shared>, h: HeaderMap, b: Bytes| async move {
                        take(&s, h, &b, |r, d| r.metrics.push(d)).await
                    },
                ),
            )
            .route(
                "/v1/logs",
                post(
                    |State(s): State<Shared>, h: HeaderMap, b: Bytes| async move {
                        take(&s, h, &b, |r, d| r.logs.push(d)).await
                    },
                ),
            )
            .with_state(Arc::clone(&received));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let addr = listener.local_addr().expect("its address");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            addr,
            received,
            task,
        }
    }

    /// The endpoint to give `OTEL_EXPORTER_OTLP_ENDPOINT`.
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn with<T>(&self, f: impl FnOnce(&Received) -> T) -> T {
        f(&self.received.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Every span received, flattened out of the request documents.
    #[must_use]
    pub fn spans(&self) -> Vec<Value> {
        self.with(|r| {
            r.traces
                .iter()
                .flat_map(|d| items(d, "resourceSpans", "scopeSpans", "spans"))
                .collect()
        })
    }

    /// Every metric received (the latest request's copy of each name last).
    #[must_use]
    pub fn metrics(&self) -> Vec<Value> {
        self.with(|r| {
            r.metrics
                .iter()
                .flat_map(|d| items(d, "resourceMetrics", "scopeMetrics", "metrics"))
                .collect()
        })
    }

    /// Every log record received.
    #[must_use]
    pub fn logs(&self) -> Vec<Value> {
        self.with(|r| {
            r.logs
                .iter()
                .flat_map(|d| items(d, "resourceLogs", "scopeLogs", "logRecords"))
                .collect()
        })
    }

    /// The resource attributes of the first request of each signal.
    #[must_use]
    pub fn resources(&self) -> Vec<Value> {
        self.with(|r| {
            let mut out = Vec::new();
            for (docs, key) in [
                (&r.traces, "resourceSpans"),
                (&r.metrics, "resourceMetrics"),
                (&r.logs, "resourceLogs"),
            ] {
                if let Some(resource) = docs
                    .first()
                    .and_then(|doc| doc.get(key))
                    .and_then(|list| list.get(0))
                    .and_then(|first| first.get("resource"))
                {
                    out.push(resource.clone());
                }
            }
            out
        })
    }

    /// Every request body, as text: for a test that scans for what must never be there.
    #[must_use]
    pub fn bodies(&self) -> Vec<String> {
        self.with(|r| r.bodies.clone())
    }

    /// The headers of every request.
    #[must_use]
    pub fn headers(&self) -> Vec<HeaderMap> {
        self.with(|r| r.headers.clone())
    }

    /// Waits until `ready` holds of the collector, at most `within`.
    pub async fn wait_for(&self, within: Duration, ready: impl Fn(&Self) -> bool) -> bool {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if ready(self) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ready(self)
    }
}

fn items(doc: &Value, outer: &str, inner: &str, leaf: &str) -> Vec<Value> {
    doc[outer]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|r| r[inner].as_array().into_iter().flatten())
        .flat_map(|s| s[leaf].as_array().into_iter().flatten().cloned())
        .collect()
}
