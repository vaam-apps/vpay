//! The merchant side of the signed `/v1` transport, for the integration suites.
//!
//! Every `/v1` call these suites make is a COSE_Sign1 message under the calling
//! merchant's Ed25519 key, opened by the real `EnvelopeLayer` in the real
//! router against a real Postgres replay cache, and answered by a message
//! sealed under the server's key. [`SealedHttp`] is the stand-in for the
//! `reqwest::Client` the suites used to call with `.bearer_auth(..)`: the same
//! builder shape (`get`/`post`/`delete`, `.header`, `.body`, `.query`, `.send`),
//! with `.signed_by("<client_id>")` where the token was.
//!
//! **It is not `vpay-sdk`.** The SDK has its own tests (`sdks/rust/tests`) and
//! is driven against this server by `customers.rs` and the live suites; this
//! module seals with cratestack's `ClientEnvelope` directly and binds routes from
//! `vpay-wire`, so a bug in the SDK's own sealing or vendored table cannot hide
//! behind a harness that shares it.
//!
//! # Keys
//!
//! Test keys are derived, not stored: a merchant's seed is
//! `SHA-256("vpay-it-merchant:" || client_id)`, so the harness that registers a
//! merchant (`support::merchant_client_with`, which puts the public half in
//! `cose_keys`) and the code that signs for it agree without a registry to
//! thread through 14 suites. They are test-only values; no deployment holds
//! them. The server's two response seeds are the two constants below.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use cratestack_client_rust::cose::{
    CoseEnvelope, CoseMode, CoseSigner, CoseVerifyKey, Ed25519Signer, P256Signer,
    StaticVerifierResolver,
};
use cratestack_client_rust::{ClientEnvelope, ClientError, EnvelopeError, RouteRef, SealCall};
use reqwest::header::HeaderMap;
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};

fn seed_of(label: &str) -> [u8; 32] {
    Sha256::digest(label.as_bytes()).into()
}

/// The Ed25519 seed `client_id` signs with.
pub(crate) fn merchant_seed(client_id: &str) -> [u8; 32] {
    seed_of(&format!("vpay-it-merchant:{client_id}"))
}

/// The public half of [`merchant_seed`], 32 bytes.
pub(crate) fn merchant_public(client_id: &str) -> [u8; 32] {
    public_of(&merchant_seed(client_id))
}

/// [`merchant_public`] as the unpadded base64url `cose_keys` holds.
pub(crate) fn merchant_public_b64(client_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(merchant_public(client_id))
}

/// The seed the test server seals with now, and the one it will after rotation.
pub(crate) fn server_seed_current() -> [u8; 32] {
    seed_of("vpay-it-server-current")
}

/// See [`server_seed_current`].
pub(crate) fn server_seed_next() -> [u8; 32] {
    seed_of("vpay-it-server-next")
}

fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    Ed25519Signer::from_seed(seed)
        .verify_key()
        .ed25519_bytes()
        .expect("an Ed25519 key")
}

/// The two response keys a client pins, as base64url: `(current, next)`.
pub(crate) fn server_keys_b64() -> (String, String) {
    (
        URL_SAFE_NO_PAD.encode(public_of(&server_seed_current())),
        URL_SAFE_NO_PAD.encode(public_of(&server_seed_next())),
    )
}

// ------------------------------------------------------------- the console --

/// The P-256 scalar a console named `label` signs with.
///
/// Derived, like a merchant's seed, so the harness that pairs a key and the code
/// that signs with it agree without a registry. `SHA-256` of a label is a valid
/// scalar with overwhelming probability (it fails only for 0 or >= the group
/// order); [`console_signer`] panics if one ever is not, and the label is
/// changed.
pub(crate) fn console_scalar(label: &str) -> [u8; 32] {
    seed_of(&format!("vpay-it-console:{label}"))
}

fn console_signer(scalar: &[u8; 32]) -> P256Signer {
    P256Signer::from_scalar(scalar).expect("a derived scalar is a valid P-256 scalar")
}

/// The SEC1 uncompressed public key (65 bytes, `0x04 || X || Y`) of [`console_scalar`].
pub(crate) fn console_public(label: &str) -> [u8; 65] {
    console_signer(&console_scalar(label))
        .verify_key()
        .p256_sec1_uncompressed()
        .expect("a P-256 key")
}

/// [`console_public`] as the unpadded base64url a pairing carries.
pub(crate) fn console_public_b64(label: &str) -> String {
    URL_SAFE_NO_PAD.encode(console_public(label))
}

/// The thumbprint (RFC 9679, lower-case hex) of the console key `label`: the
/// `console_keys.thumbprint` a pairing of it stores.
pub(crate) fn console_thumbprint(label: &str) -> String {
    let key = CoseVerifyKey::p256_sec1(&console_public(label)).expect("a point");
    key.thumbprint()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The Ed25519 seed the service `client_id` signs with.
pub(crate) fn service_seed(client_id: &str) -> [u8; 32] {
    seed_of(&format!("vpay-it-service:{client_id}"))
}

/// [`service_seed`]'s public half as the unpadded base64url `cose_keys` holds.
pub(crate) fn service_public_b64(client_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(public_of(&service_seed(client_id)))
}

/// The ECDSA-SHA256 possession proof over `"vpay.console-pair.v1\n" +
/// publicKey + "\n" + issuedAt` that a pairing carries, signed by console `label`'s
/// private half, as the unpadded base64url of the 64-byte `r || s`.
pub(crate) fn possession_proof(label: &str, public_key_b64: &str, issued_at: i64) -> String {
    use p256::ecdsa::signature::Signer as _;
    let key = p256::ecdsa::SigningKey::from_bytes((&console_scalar(label)).into())
        .expect("a valid scalar");
    let signature: p256::ecdsa::Signature =
        key.sign(format!("vpay.console-pair.v1\n{public_key_b64}\n{issued_at}").as_bytes());
    URL_SAFE_NO_PAD.encode(signature.to_bytes())
}

/// Every op digest of the generated procedure transport, from the schema file
/// itself: the independent oracle for what `POST /dash/v1/$procs/<name>` binds.
fn procedure_digest(name: &str) -> Option<[u8; 32]> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<(String, [u8; 32])>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let schema =
            cratestack_parser::parse_schema(include_str!("../../../../../schemas/vpay.cstack"))
                .expect("schemas/vpay.cstack parses");
        cratestack_core::bound_contracts(&schema)
    });
    let key = format!("POST /$procs/{name}");
    table
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, digest)| *digest)
}

/// Which surface a URL addresses, and the operation it binds there.
struct Bound {
    template: String,
    params: Vec<String>,
    digest: [u8; 32],
    /// Whether the surface speaks JSON in both directions (everything but `/v1`).
    json: bool,
}

fn bind_operations(
    operations: &'static [vpay_wire::Operation],
    method: &str,
    path: &str,
    digest: fn(&str, &str) -> [u8; 32],
    json: bool,
) -> Option<Bound> {
    let (template, params) = match_operation(operations, method, path)?;
    let method = if method == "HEAD" { "GET" } else { method };
    Some(Bound {
        template: template.to_owned(),
        params,
        digest: digest(method, template),
        json,
    })
}

/// Resolves a full URL path to the operation it addresses.
fn bind_url_path(method: &str, full_path: &str) -> Option<Bound> {
    if let Some(name) = full_path.strip_prefix("/dash/v1/$procs/") {
        return Some(Bound {
            template: format!("/$procs/{name}"),
            params: Vec::new(),
            digest: procedure_digest(name)?,
            json: true,
        });
    }
    if let Some(path) = full_path.strip_prefix("/dash/v1") {
        return bind_operations(
            vpay_wire::DASH_OPERATIONS,
            method,
            path,
            vpay_wire::dash_contract_digest,
            true,
        );
    }
    if let Some(path) = full_path.strip_prefix("/svc/v1") {
        return bind_operations(
            vpay_wire::SVC_OPERATIONS,
            method,
            path,
            vpay_wire::svc_contract_digest,
            true,
        );
    }
    let path = full_path.strip_prefix("/v1")?;
    bind_operations(
        vpay_wire::V1_OPERATIONS,
        method,
        path,
        vpay_wire::contract_digest,
        false,
    )
}

/// The operation of `operations` a concrete path and method address, with the
/// decoded parameter values: the independent oracle for what the server binds.
fn match_operation(
    operations: &'static [vpay_wire::Operation],
    method: &str,
    path: &str,
) -> Option<(&'static str, Vec<String>)> {
    let method = if method == "HEAD" { "GET" } else { method };
    let segments: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    'rows: for operation in operations {
        if operation.method != method {
            continue;
        }
        let pattern: Vec<&str> = operation.path.trim_start_matches('/').split('/').collect();
        if pattern.len() != segments.len() {
            continue;
        }
        let mut params = Vec::new();
        for (wanted, got) in pattern.iter().zip(&segments) {
            if wanted.starts_with('{') {
                if got.is_empty() {
                    continue 'rows;
                }
                params.push(
                    percent_encoding::percent_decode_str(got)
                        .decode_utf8()
                        .ok()?
                        .into_owned(),
                );
            } else if wanted != got {
                continue 'rows;
            }
        }
        return Some((operation.path, params));
    }
    None
}

/// A client for the test server's signed `/v1`.
#[derive(Clone, Debug)]
pub(crate) struct SealedHttp {
    http: reqwest::Client,
}

impl Default for SealedHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl SealedHttp {
    /// A plain HTTP client underneath, with redirects off.
    pub(crate) fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("a client builds"),
        }
    }

    pub(crate) fn request(&self, method: &str, url: impl Into<String>) -> SealedRequest {
        SealedRequest {
            http: self.http.clone(),
            method: method.to_owned(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            query: Vec::new(),
            client_id: None,
            pinned: server_seed_current(),
            seal_with: None,
            console_scalar: None,
            forced_binding: None,
            audience: None,
        }
    }

    pub(crate) fn get(&self, url: impl Into<String>) -> SealedRequest {
        self.request("GET", url)
    }

    pub(crate) fn post(&self, url: impl Into<String>) -> SealedRequest {
        self.request("POST", url)
    }

    pub(crate) fn delete(&self, url: impl Into<String>) -> SealedRequest {
        self.request("DELETE", url)
    }

    pub(crate) fn patch(&self, url: impl Into<String>) -> SealedRequest {
        self.request("PATCH", url)
    }
}

/// One call, being built.
pub(crate) struct SealedRequest {
    http: reqwest::Client,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    query: Vec<(String, String)>,
    client_id: Option<String>,
    pinned: [u8; 32],
    seal_with: Option<[u8; 32]>,
    console_scalar: Option<[u8; 32]>,
    forced_binding: Option<(String, [u8; 32])>,
    audience: Option<String>,
}

impl SealedRequest {
    /// Sign as `client_id` — the merchant whose derived key this is.
    pub(crate) fn signed_by(mut self, client_id: &str) -> Self {
        self.client_id = Some(client_id.to_owned());
        self
    }

    /// Bind the call to this route template and contract digest instead of the
    /// ones the URL addresses: a method or path the contract does not list, for
    /// the cases that prove the layer refuses it.
    pub(crate) fn bound_to(mut self, template: &str, digest: [u8; 32]) -> Self {
        self.forced_binding = Some((template.to_owned(), digest));
        self
    }

    /// Sign as the console key `label` (a P-256 key, [`console_scalar`]).
    pub(crate) fn signed_by_console(mut self, label: &str) -> Self {
        self.client_id = Some(format!("console:{label}"));
        self.console_scalar = Some(console_scalar(label));
        self
    }

    /// Sign as the registered service `client_id` ([`service_seed`]).
    pub(crate) fn signed_by_service(mut self, client_id: &str) -> Self {
        self.client_id = Some(client_id.to_owned());
        self.seal_with = Some(service_seed(client_id));
        self
    }

    /// Sign with this P-256 scalar: a console key nobody paired, for the guard
    /// cases.
    pub(crate) fn signed_with_console_scalar(mut self, scalar: [u8; 32]) -> Self {
        self.client_id = Some("console:adhoc".to_owned());
        self.console_scalar = Some(scalar);
        self
    }

    /// Sign with this seed instead of the one derived from the `client_id`: a
    /// key nobody registered, for the guard cases.
    pub(crate) fn signed_with_seed(mut self, seed: [u8; 32]) -> Self {
        self.seal_with = Some(seed);
        self
    }

    /// Address the request to this audience instead of the server's origin.
    pub(crate) fn addressed_to(mut self, audience: &str) -> Self {
        self.audience = Some(audience.to_owned());
        self
    }

    /// Trust this server seed's public key for the answer, instead of the
    /// current one — the pin the case wants to see refused or accepted.
    pub(crate) fn pinning_server_seed(mut self, seed: [u8; 32]) -> Self {
        self.pinned = seed;
        self
    }

    pub(crate) fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_lowercase(), value.into()));
        self
    }

    pub(crate) fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// An `application/x-www-form-urlencoded` body from `pairs`, the way
    /// `reqwest::RequestBuilder::form` writes one.
    pub(crate) fn form(mut self, pairs: &[(&str, &str)]) -> Self {
        self.body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
            .into_bytes();
        self
    }

    /// A JSON body, declared as `application/json`: a type no `/v1` operation
    /// takes, for the cases that prove the layer refuses it.
    pub(crate) fn json(mut self, value: &serde_json::Value) -> Self {
        self.body = value.to_string().into_bytes();
        self.headers
            .push(("content-type".to_owned(), "application/json".to_owned()));
        self
    }

    pub(crate) fn query(mut self, pairs: &[(&str, &str)]) -> Self {
        self.query.extend(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        self
    }

    fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }

    /// Seal and send.
    ///
    /// # Errors
    ///
    /// Anything that stops a request being sealed or sent, and a response that
    /// claims to be sealed and does not verify under the pinned key
    /// (`EnvelopeError::Unverified`) — an unsigned answer is **not** an error
    /// here, it is a [`SealedResponse`] with `is_sealed() == false`, because
    /// asserting the layer's bare `401` is half the point of several cases.
    pub(crate) async fn send(self) -> anyhow::Result<SealedResponse> {
        let mut answers = self.transmit(1).await?;
        Ok(answers.remove(0))
    }

    /// Seal **once** and transmit the identical bytes `times` times — a
    /// replay, which is the thing a retrying proxy or an attacker with a
    /// packet capture does. The first answer is opened like [`send`](Self::send)'s;
    /// the later ones are returned as they arrived (`is_sealed()` reads the
    /// content type), because a replay refusal is the layer's unsigned `401`.
    pub(crate) async fn send_replayed(self, times: usize) -> anyhow::Result<Vec<SealedResponse>> {
        self.transmit(times).await
    }

    async fn transmit(self, times: usize) -> anyhow::Result<Vec<SealedResponse>> {
        let client_id = self
            .client_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("a sealed request needs .signed_by(..)"))?;
        let url = reqwest::Url::parse(&self.url)?;
        let origin = url.origin().ascii_serialization();
        let bound = match &self.forced_binding {
            Some((template, digest)) => Bound {
                template: template.clone(),
                params: Vec::new(),
                digest: *digest,
                json: !url.path().starts_with("/v1"),
            },
            None => bind_url_path(&self.method, url.path()).ok_or_else(|| {
                anyhow::anyhow!("no operation matches {} {}", self.method, url.path())
            })?,
        };
        let params: Vec<&str> = bound.params.iter().map(String::as_str).collect();

        let mut query_pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        query_pairs.extend(self.query.iter().cloned());
        let query_string = (!query_pairs.is_empty()).then(|| {
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&query_pairs)
                .finish()
        });

        let signer: std::sync::Arc<dyn CoseSigner> = match self.console_scalar {
            Some(scalar) => std::sync::Arc::new(console_signer(&scalar)),
            None => std::sync::Arc::new(Ed25519Signer::from_seed(
                &self.seal_with.unwrap_or_else(|| merchant_seed(&client_id)),
            )),
        };
        let server_key = CoseVerifyKey::ed25519(&public_of(&self.pinned)).expect("a point");
        let cose = CoseEnvelope::client(
            CoseMode::Sign1,
            signer,
            std::sync::Arc::new(StaticVerifierResolver::new().with_key(server_key)),
        )
        .build()?;
        let envelope = ClientEnvelope::new(cose, self.audience.clone().unwrap_or(origin))?;

        let payload_type = self
            .header_value("content-type")
            .unwrap_or(if bound.json {
                "application/json"
            } else {
                "application/x-www-form-urlencoded"
            })
            .to_owned();
        let idempotency_key = self.header_value("idempotency-key").map(str::to_owned);
        let mut call = SealCall::new(
            &self.method,
            RouteRef::new(&bound.template, &params),
            bound.digest,
        )
        .query(query_string.as_deref())
        .payload(&self.body, &payload_type)
        .accept("application/json");
        if let Some(key) = idempotency_key.as_deref() {
            call = call.idempotency_key(key);
        }
        let sealed = envelope.seal_call(call).await?;

        let mut target = url.clone();
        target.set_query(query_string.as_deref());
        let mut pending = Some(sealed.pending);
        let mut answers = Vec::with_capacity(times);
        for _ in 0..times {
            let mut request = self
                .http
                .request(self.method.parse()?, target.clone())
                .body(sealed.body.clone());
            for (name, value) in &sealed.headers {
                request = request.header(name, value);
            }
            // Not part of the seal: the one header a case sets to prove the
            // surface refuses a signature that arrives beside a bearer.
            if let Some(value) = self.header_value("authorization") {
                request = request.header("authorization", value);
            }
            let response = request.send().await?;
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = response.bytes().await?;

            let Some(pending) = pending.take() else {
                let sealed = headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.starts_with("application/cose"));
                answers.push(SealedResponse {
                    status,
                    headers,
                    body: bytes,
                    sealed,
                });
                continue;
            };
            answers.push(
                match pending.open(status.as_u16(), &headers, bytes.clone()).await {
                    Ok(opened) => SealedResponse {
                        status,
                        headers,
                        body: opened.body,
                        sealed: true,
                    },
                    Err(ClientError::Envelope(EnvelopeError::Unsigned { .. })) => SealedResponse {
                        status,
                        headers,
                        body: bytes,
                        sealed: false,
                    },
                    Err(other) => return Err(other.into()),
                },
            );
        }
        Ok(answers)
    }
}

/// An answer: the opened payload if it was sealed, the raw body if it was not.
#[derive(Debug)]
pub(crate) struct SealedResponse {
    status: reqwest::StatusCode,
    headers: HeaderMap,
    body: Bytes,
    sealed: bool,
}

impl SealedResponse {
    /// A plain `reqwest` answer wrapped as an unsealed one: for the requests a
    /// case sends with no signature at all, so they are asserted through the
    /// same accessors as the sealed ones.
    pub(crate) async fn from_plain(response: reqwest::Response) -> Result<Self, reqwest::Error> {
        let status = response.status();
        let headers = response.headers().clone();
        let sealed = headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/cose"));
        let body = response.bytes().await?;
        Ok(Self {
            status,
            headers,
            body,
            sealed,
        })
    }

    pub(crate) fn status(&self) -> reqwest::StatusCode {
        self.status
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Whether the answer was a COSE message that verified under the pinned key.
    pub(crate) fn is_sealed(&self) -> bool {
        self.sealed
    }

    pub(crate) async fn json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    pub(crate) async fn text(&self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.body.to_vec())
    }

    pub(crate) async fn bytes(&self) -> Result<Bytes, std::convert::Infallible> {
        Ok(self.body.clone())
    }
}

/// Every `client_id` → seed pairing the harness registered, for a failure
/// message that wants to say which key signed.
pub(crate) fn describe(client_ids: &[&str]) -> BTreeMap<String, String> {
    client_ids
        .iter()
        .map(|id| ((*id).to_owned(), merchant_public_b64(id)))
        .collect()
}

/// `client_id`'s credentials as `vpay-sdk` takes them — for the suites that
/// drive the shipping SDK against the real server (`customers.rs`, the
/// payment-intent happy paths). The same derived seed [`SealedHttp`] signs with.
pub(crate) fn sdk_credentials(client_id: &str) -> vpay_sdk::Credentials {
    vpay_sdk::Credentials::ed25519_seed(client_id, &merchant_seed(client_id))
        .expect("a derived test seed signs")
}

/// The test server's two response keys, pinned the way a merchant pins them.
pub(crate) fn sdk_server_keys() -> vpay_sdk::ServerKeys {
    let (current, next) = server_keys_b64();
    vpay_sdk::ServerKeys::from_base64url(&current, &next).expect("two public keys")
}
