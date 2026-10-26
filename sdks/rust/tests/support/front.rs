//! The server half of the signed transport, in front of a `wiremock` upstream.
//!
//! Every resource test in this directory asserts at the byte level what the SDK
//! sent (path, method, encoded body, `Idempotency-Key`) and what it decodes. The
//! bearer-token SDK sent those bytes to a `wiremock` server directly. The signed
//! one cannot: its request is a COSE message that only an envelope layer can
//! open, and its answer must be sealed by a key it pinned. So a [`Front`] stands
//! between: **cratestack's own `EnvelopeLayer`** (the very layer `vpay-api`
//! mounts on `/v1`, in `Required` mode) opens each request and verifies it
//! against the merchant's public key, the opened request is forwarded to the
//! `wiremock` upstream as the plain form request it wraps, and whatever the
//! upstream answers is sealed on the way back.
//!
//! The upstream therefore records exactly what a `vpay-api` handler would see
//! (`Content-Type: application/x-www-form-urlencoded`, the `Idempotency-Key`,
//! the path and query, no `Authorization`), which is what keeps the 4000 lines of
//! byte-level assertions meaningful without rewriting any of them. Nothing here
//! is a copy of cratestack's sealing code: a bug in the SDK's sealing is
//! refused by the real verifier, and a bug in the SDK's binding is refused by the
//! real contract check.
//!
//! What this front does **not** prove is that the *operation table* the SDK
//! vendors agrees with `vpay-api`'s router. Its resolver reads that same table
//! (`vpay_sdk::contracts`), so the check is circular here. It is closed from the
//! other side, in the workspace: `vpay-api`'s `sdk_contracts` test compares the
//! SDK's table to `vpay-wire`, and `backends/tests/integration/tests/
//! signed_transport.rs` drives this crate against the real router.

use std::sync::{Arc, LazyLock};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use cratestack_axum::envelope_layer::{
    BindingResolver, EnvelopeLayer, EnvelopeMode, Resolution, ResolvedRoute, RouteRequest,
};
use cratestack_client_rust::cose::{
    CoseEnvelope, CoseMode, CoseVerifyKey, Ed25519Signer, StaticVerifierResolver,
};
use cratestack_core::{AcceptedContracts, InMemoryNonceStore};
use vpay_sdk::contracts::{OPERATIONS, contract_digest, operation_for};
use vpay_sdk::{Credentials, ServerKeys};

/// Audience the front answers to: the front's own base URL.
fn audience(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_owned()
}

/// Two distinct server seeds and a merchant seed, drawn once per test process.
struct Seeds {
    merchant: [u8; 32],
    server_current: [u8; 32],
    server_next: [u8; 32],
}

static SEEDS: LazyLock<Seeds> = LazyLock::new(|| Seeds {
    merchant: rand::random(),
    server_current: rand::random(),
    server_next: rand::random(),
});

fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    Ed25519Signer::from_seed(seed)
        .verify_key()
        .ed25519_bytes()
        .expect("an Ed25519 key")
}

/// The merchant's credentials for every test in this process.
pub(crate) fn merchant_credentials() -> Credentials {
    Credentials::ed25519_seed("merchant_acme", &SEEDS.merchant)
}

/// The merchant's seed, for a test that signs through an external callback.
pub(crate) fn merchant_seed() -> [u8; 32] {
    SEEDS.merchant
}

/// The response keys the SDK pins, `{current, next}` — the real public halves
/// of the keys the front seals with.
pub(crate) fn server_keys() -> ServerKeys {
    ServerKeys::new(
        public_of(&SEEDS.server_current),
        public_of(&SEEDS.server_next),
    )
}

/// The first pinned key, alone.
pub(crate) fn server_keys_current() -> [u8; 32] {
    public_of(&SEEDS.server_current)
}

/// A key the front never seals with and the client never pinned.
pub(crate) fn unpinned_server_seed() -> [u8; 32] {
    rand::random()
}

/// The contract table the front accepts: one digest per operation.
static CONTRACTS: LazyLock<AcceptedContracts> = LazyLock::new(|| {
    let rows: Vec<(&'static str, &'static [[u8; 32]])> = OPERATIONS
        .iter()
        .map(|(method, template)| {
            let key: &'static str = Box::leak(format!("{method} {template}").into_boxed_str());
            let digests: &'static [[u8; 32]] =
                Box::leak(Box::new([contract_digest(method, template)]));
            (key, digests)
        })
        .collect();
    Box::leak(rows.into_boxed_slice())
});

/// Binds a request to the operation the SDK's table names for its raw path.
#[derive(Debug, Clone, Copy)]
struct SdkTableBinding;

impl BindingResolver for SdkTableBinding {
    fn resolve(&self, request: &RouteRequest<'_>) -> Resolution {
        let Some(path) = request.path().strip_prefix("/v1") else {
            return Resolution::NotAnOp;
        };
        let method = match request.method().as_str() {
            "HEAD" => "GET",
            other => other,
        };
        match operation_for(method, path) {
            Ok(bound) => Resolution::Op(
                ResolvedRoute::new(bound.template, bound.params)
                    .with_contract_key(format!("{method} {}", bound.template))
                    .with_payload_types(
                        &["application/x-www-form-urlencoded"],
                        &["application/json"],
                    ),
            ),
            Err(_) => Resolution::NotAnOp,
        }
    }
}

/// How the front answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seal {
    /// Seal with the first pinned key (`current`).
    Current,
    /// Seal with the second pinned key (`next`).
    Next,
    /// Seal with a key the client did not pin.
    Unpinned,
}

/// A running front. Dropping it stops the server.
pub(crate) struct Front {
    pub(crate) base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Front {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Front {
    /// A client for this front: the process's merchant credentials and pinned
    /// server keys.
    pub(crate) fn client(&self) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(merchant_credentials())
            .server_keys(server_keys())
            .build()
            .expect("the front's client builds")
    }
}

/// Starts a front forwarding to `upstream` (a `wiremock` URI), sealing as `seal`.
pub(crate) async fn start(upstream: &str, seal: Seal) -> Front {
    // The front's own forwarding client is a plain `reqwest::Client`, which under
    // the workspace's `rustls-no-provider` needs a process-wide provider. The SDK
    // under test does not (it hands reqwest a finished `ClientConfig`, which
    // `tests/tls.rs` proves in a process that never touches this module).
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding a loopback port");
    let base_url = format!("http://{}", listener.local_addr().expect("a bound address"));

    let signer_seed = match seal {
        Seal::Current => SEEDS.server_current,
        Seal::Next => SEEDS.server_next,
        Seal::Unpinned => unpinned_server_seed(),
    };
    let merchant_key = CoseVerifyKey::ed25519(&public_of(&SEEDS.merchant)).expect("a point");
    let envelope = CoseEnvelope::server(
        CoseMode::Sign1,
        Arc::new(Ed25519Signer::from_seed(&signer_seed)),
        Arc::new(StaticVerifierResolver::new().with_key(merchant_key)),
        Arc::new(InMemoryNonceStore::new()),
    )
    .build()
    .expect("a server envelope");
    let layer: EnvelopeLayer = EnvelopeLayer::builder(envelope, audience(&base_url), *CONTRACTS)
        .binding_resolver(SdkTableBinding)
        .policy(EnvelopeMode::Required)
        .payload_media_types(["application/x-www-form-urlencoded"], ["application/json"])
        .build()
        .expect("the front's envelope layer builds");

    let app = Router::new()
        .fallback(forward)
        .with_state((
            reqwest::Client::new(),
            upstream.trim_end_matches('/').to_owned(),
        ))
        .layer(layer);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Front { base_url, task }
}

/// Forwards the opened (plain) request to the upstream and returns its answer,
/// which the layer then seals.
async fn forward(
    State((http, upstream)): State<(reqwest::Client, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .expect("a bounded body");
    let url = format!(
        "{upstream}{}",
        parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or_default()
    );
    let mut builder = http.request(parts.method, url).body(bytes);
    for (name, value) in &parts.headers {
        // `host` and `content-length` belong to the hop; `accept` was rewritten
        // by the layer to the negotiated response type, which is what a handler
        // sees and so what the upstream should too.
        if name != "host" && name != "content-length" {
            builder = builder.header(name, value);
        }
    }
    let answer = builder.send().await.expect("the upstream answers");
    let mut response = Response::builder().status(answer.status());
    for (name, value) in answer.headers() {
        if name != "content-length" && name != "transfer-encoding" {
            response = response.header(name, value);
        }
    }
    response
        .body(Body::from(answer.bytes().await.expect("the upstream body")))
        .expect("a valid response")
}
