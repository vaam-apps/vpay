//! [`Client`] — the signing HTTP client for `/v1`, built via [`ClientBuilder`].
//!
//! Every call is sealed as a COSE_Sign1 message under the merchant's Ed25519
//! key and bound to the operation it calls, the `Idempotency-Key` it carries and
//! the audience it is addressed to (`crate::contracts`, cratestack's
//! `ClientEnvelope::seal_call`); every answer is opened against the server keys
//! the client pinned (`PendingResponse::open`). The body inside the seal is the
//! same `application/x-www-form-urlencoded` the bearer-token SDK sent, and the
//! payload that comes out is the same JSON (owner's ruling D6: the Stripe shape
//! is kept, carried inside the envelope by payload media types).
//!
//! There is **no token and no retry**: a sealed request is single-use (its
//! `cti` is spent the moment the server opens it), so the old "re-authenticate
//! and send once more on a 401" has no meaning here — the 401 is the same bytes
//! for an unknown key, a bad signature, a stale clock and a replay, and sending
//! again would only spend another `cti`. A caller that gets
//! [`Error::Unsigned`] checks its registered key and its clock.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use cratestack_client_rust::cose::{CoseEnvelope, CoseMode, StaticVerifierResolver};
use cratestack_client_rust::{ClientEnvelope, ClientError, EnvelopeError, RouteRef, SealCall};
use reqwest::Method;
use reqwest::header::USER_AGENT;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::contracts;
use crate::credentials::{Credentials, ServerKeys};
use crate::error::{ConfigError, Error};
use crate::resources::{
    AccountHoldersResource, BalanceResource, CheckoutResource, CustomersResource, EventsResource,
    InvoiceItemsResource, InvoicesResource, PaymentIntentsResource, RefundsResource,
    RequestOptions,
};

/// The payload type of every request body: the Stripe form, byte for byte what
/// the bearer-token SDK put on the wire. Declared by every `/v1` operation on
/// the server (`vpay_api::envelope::binding::REQUEST_TYPES`).
const REQUEST_PAYLOAD_TYPE: &str = "application/x-www-form-urlencoded";

/// The one payload type every answer is read in.
const RESPONSE_PAYLOAD_TYPE: &str = "application/json";

/// The maximum bytes of a non-JSON-envelope error body kept in
/// [`Error::UnexpectedResponse`]. An unbounded prefix would let a
/// misbehaving upstream hand this crate an unbounded amount of memory to
/// hold in an error value.
const BODY_PREFIX_LIMIT: usize = 500;

/// Mozilla's CA bundle as a rustls root store — see
/// [`rustls_client_config`] for why the roots are vendored rather than read
/// from the platform.
fn vendored_root_store() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// The TLS configuration every request in this crate goes out over: rustls
/// with the `ring` provider and Mozilla's vendored root bundle.
///
/// Built here rather than left to `reqwest::ClientBuilder`'s own TLS setup,
/// for two reasons that both bite a *library*:
///
/// 1. **No panic.** reqwest 0.13 is pinned with `rustls-no-provider` (its
///    `rustls` feature would select the `aws-lc-rs` provider, which
///    `deny.toml` bans — two providers in one process make
///    `rustls::crypto::CryptoProvider::install_default()` panic). Under that
///    feature reqwest's own builder panics if no process-wide default
///    provider was installed. This crate runs inside a merchant's process; it
///    may not panic there, and it may not install a process-wide default on
///    that process's behalf either — that is the application's decision, and
///    an SDK quietly making it would break a merchant who wanted a different
///    provider. Handing reqwest a finished `ClientConfig` takes its
///    `BuiltRustls` path, which never consults the process default at all.
/// 2. **Deterministic roots.** reqwest 0.13 dropped its vendored-roots
///    feature for `rustls-platform-verifier` (the OS trust store). vpay's
///    runtime image is `FROM scratch` (`docs/adr/0004`) and has no trust
///    store, and Node's SDK ships its own CA bundle rather than reading the
///    OS one — so vendored roots keep both the deployment target and the two
///    SDKs consistent.
///
/// ALPN is set explicitly because reqwest only sets it on the path *not*
/// taken here: without these two protocol names a TLS connection would
/// silently never negotiate HTTP/2.
fn rustls_client_config() -> Result<rustls::ClientConfig, ConfigError> {
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| ConfigError::Http(e.to_string()))?
    .with_root_certificates(vendored_root_store())
    .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

fn user_agent() -> String {
    format!("vpay-sdk-rust/{}", env!("CARGO_PKG_VERSION"))
}

/// How many bytes of a UTF-8 sequence `lead` introduces. `1` for ASCII and
/// for any byte that cannot start a sequence at all — such a byte is not a
/// truncated character, it is simply invalid, and the lossy decode below
/// renders it as `U+FFFD` whether or not the cut fell here.
fn utf8_sequence_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Backs the cut off the tail of a *truncated* multi-byte character, so the
/// prefix ends on a character boundary.
///
/// Parity with `sdks/nodejs/src/errors.ts`'s `boundedBodyPrefix`, which
/// decodes with a streaming `TextDecoder` it never flushes: a character
/// straddling the cut is **dropped**, not emitted as `U+FFFD`. Without this,
/// a merchant reading `err.body_prefix` from the two SDKs would see the same
/// upstream body end differently, and a `U+FFFD` that is an artefact of the
/// truncation is indistinguishable from one that was really in the body.
fn trim_truncated_tail(bytes: &[u8]) -> &[u8] {
    // At most 3 continuation bytes can precede the lead byte of the longest
    // (4-byte) UTF-8 sequence.
    for back in 0..4usize {
        let Some(index) = bytes.len().checked_sub(back + 1) else {
            return bytes;
        };
        let Some(&byte) = bytes.get(index) else {
            return bytes;
        };
        if byte & 0b1100_0000 == 0b1000_0000 {
            continue; // a continuation byte; keep walking back to the lead
        }
        // `byte` leads the final sequence and `back + 1` of its bytes are
        // present. Complete: keep everything. Short: drop the whole partial
        // character.
        return if utf8_sequence_len(byte) > back + 1 {
            bytes.get(..index).unwrap_or(bytes)
        } else {
            bytes
        };
    }
    bytes
}

/// The first [`BODY_PREFIX_LIMIT`] **bytes** of `bytes`, cut on a character
/// boundary and lossily decoded.
///
/// The bound is on bytes, not `char`s: the point is to keep an unbounded
/// response body out of an error value and a log line, and a body of
/// multi-byte text is up to four times its character count on the wire.
///
/// Written with `.get(..)` rather than `&bytes[..limit]` even though `limit`
/// is provably in range: `clippy::indexing_slicing` is on workspace-wide and
/// this repository's house style answers it by not indexing at all rather
/// than by allowing the lint locally (see `vpay_api::resource_auth`'s test
/// helper, which says so).
fn bounded_prefix(bytes: &[u8]) -> String {
    let limit = bytes.len().min(BODY_PREFIX_LIMIT);
    let cut = bytes.get(..limit).unwrap_or(bytes);
    String::from_utf8_lossy(trim_truncated_tail(cut)).into_owned()
}

#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    code: Option<String>,
    message: String,
    #[serde(default)]
    param: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

struct Inner {
    http: reqwest::Client,
    credentials: Credentials,
    envelope: ClientEnvelope,
    resource_base: String,
    audience: String,
}

/// A configured `/v1` API client. Cheap to clone — internally an `Arc`, so
/// every clone shares the same connection pool and key.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl fmt::Debug for Client {
    /// Hand-written, like [`Credentials`]'s own `Debug`: this must never
    /// print the private key — see `tests/debug_redaction.rs`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("credentials", &self.inner.credentials)
            .field("resource_base", &self.inner.resource_base)
            .field("audience", &self.inner.audience)
            .finish()
    }
}

impl Client {
    /// Starts building a client against `base_url` (e.g.
    /// `"https://api.vpay.example"`). See [`ClientBuilder`] for the defaults
    /// this derives.
    #[must_use]
    pub fn builder(base_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(base_url)
    }

    /// `/v1/payment_intents`.
    #[must_use]
    pub fn payment_intents(&self) -> PaymentIntentsResource<'_> {
        PaymentIntentsResource { client: self }
    }

    /// `/v1/checkout` — today only `checkout().sessions()`.
    #[must_use]
    pub fn checkout(&self) -> CheckoutResource<'_> {
        CheckoutResource { client: self }
    }

    /// `/v1/customers` — the merchant-owned record of a payer (S4a).
    #[must_use]
    pub fn customers(&self) -> CustomersResource<'_> {
        CustomersResource { client: self }
    }

    /// `/v1/invoices` — a merchant's bill to one customer (S4b).
    #[must_use]
    pub fn invoices(&self) -> InvoicesResource<'_> {
        InvoicesResource { client: self }
    }

    /// `/v1/invoice_items` — the lines of a draft invoice (S4b).
    #[must_use]
    pub fn invoice_items(&self) -> InvoiceItemsResource<'_> {
        InvoiceItemsResource { client: self }
    }

    /// `/v1/refunds`.
    #[must_use]
    pub fn refunds(&self) -> RefundsResource<'_> {
        RefundsResource { client: self }
    }

    /// `/v1/events`.
    #[must_use]
    pub fn events(&self) -> EventsResource<'_> {
        EventsResource { client: self }
    }

    /// `/v1/balance`.
    #[must_use]
    pub fn balance(&self) -> BalanceResource<'_> {
        BalanceResource { client: self }
    }

    /// `/v1/account_holders` — whose mobile-money account a number is
    /// (issue #47).
    #[must_use]
    pub fn account_holders(&self) -> AccountHoldersResource<'_> {
        AccountHoldersResource { client: self }
    }

    fn resource_url(&self, path: &str) -> String {
        format!("{}{path}", self.inner.resource_base)
    }

    /// Decodes an opened answer exactly as the bearer-token SDK decoded a
    /// plain one: the typed body on a 2xx, the Stripe-shaped envelope on a
    /// non-2xx, and a bounded prefix of anything else.
    fn decode<T: DeserializeOwned>(status: u16, bytes: &[u8]) -> Result<T, Error> {
        if (200..300).contains(&status) {
            serde_json::from_slice(bytes).map_err(|_| Error::UnexpectedResponse {
                status,
                body_prefix: bounded_prefix(bytes),
            })
        } else if let Ok(envelope) = serde_json::from_slice::<ApiErrorEnvelope>(bytes) {
            Err(Error::Api {
                status,
                kind: envelope.error.kind,
                code: envelope.error.code,
                message: envelope.error.message,
                param: envelope.error.param,
            })
        } else {
            Err(Error::UnexpectedResponse {
                status,
                body_prefix: bounded_prefix(bytes),
            })
        }
    }

    /// Seals and sends one `/v1` request, opens the answer and decodes it.
    ///
    /// Exactly one attempt: see the module docs for why a signed request is
    /// never retried here.
    async fn send_signed<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        query: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<T, Error> {
        let bound =
            contracts::operation_for(method.as_str(), path).map_err(|e| Error::InvalidParams {
                param: "path".to_owned(),
                message: e.to_string(),
            })?;
        let params: Vec<&str> = bound.params.iter().map(String::as_str).collect();
        let digest = contracts::contract_digest(method.as_str(), bound.template);
        let query = query.filter(|q| !q.is_empty());

        let payload = body.as_deref().unwrap_or("").as_bytes();
        let mut call = SealCall::new(
            method.as_str(),
            RouteRef::new(bound.template, &params),
            digest,
        )
        .query(query.as_deref())
        .payload(payload, REQUEST_PAYLOAD_TYPE)
        .accept(RESPONSE_PAYLOAD_TYPE);
        if let Some(key) = idempotency_key.as_deref() {
            call = call.idempotency_key(key);
        }
        let sealed = self
            .inner
            .envelope
            .seal_call(call)
            .await
            .map_err(|error| Error::Seal(error.to_string()))?;

        let mut url = self.resource_url(path);
        if let Some(q) = &query {
            url.push('?');
            url.push_str(q);
        }
        let mut request = self
            .inner
            .http
            .request(method, &url)
            .header(USER_AGENT, user_agent())
            .body(sealed.body);
        for (name, value) in &sealed.headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        match sealed.pending.open(status, &headers, bytes).await {
            Ok(opened) => Self::decode(status, &opened.body),
            Err(ClientError::Envelope(EnvelopeError::Unsigned { status })) => {
                Err(Error::Unsigned { status })
            }
            Err(ClientError::Envelope(EnvelopeError::Unverified)) => Err(Error::Unverified),
            Err(other) => Err(Error::Seal(other.to_string())),
        }
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Option<String>,
    ) -> Result<T, Error> {
        self.send_signed(Method::GET, path, None, query, None).await
    }

    pub(crate) async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: String,
        opts: RequestOptions,
    ) -> Result<T, Error> {
        let idempotency_key = opts
            .idempotency_key
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        self.send_signed(Method::POST, path, Some(body), None, Some(idempotency_key))
            .await
    }

    /// `DELETE`, which this API has on `/v1/customers/{id}`,
    /// `/v1/invoices/{id}` and `/v1/invoice_items/{id}`.
    ///
    /// It carries an `Idempotency-Key` like a `POST`, and **no body**. Both
    /// halves are the server's contract: `/v1` requires the header on every
    /// write regardless of verb, and a `DELETE` is the write where a replay is
    /// most confusing without one — the second call would otherwise answer
    /// `404` for a deletion that succeeded. The key is bound by the signature,
    /// as sent.
    pub(crate) async fn delete<T: DeserializeOwned>(
        &self,
        path: &str,
        opts: RequestOptions,
    ) -> Result<T, Error> {
        let idempotency_key = opts
            .idempotency_key
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        self.send_signed(Method::DELETE, path, None, None, Some(idempotency_key))
            .await
    }
}

/// Builds a [`Client`].
pub struct ClientBuilder {
    base_url: String,
    credentials: Option<Credentials>,
    server_keys: Option<ServerKeys>,
    audience: Option<String>,
    timeout: Duration,
}

impl fmt::Debug for ClientBuilder {
    /// Hand-written for the same reason as [`Client`]'s: `credentials`, once
    /// set, must never reach a `{:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("base_url", &self.base_url)
            .field("credentials", &self.credentials)
            .field("server_keys", &self.server_keys)
            .field("audience", &self.audience)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl ClientBuilder {
    fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            credentials: None,
            server_keys: None,
            audience: None,
            timeout: Duration::from_secs(30),
        }
    }

    /// The merchant's signing key. Required — [`ClientBuilder::build`]
    /// returns [`ConfigError::MissingCredentials`] without it.
    #[must_use]
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// The response keys to trust, `{current, next}`. Required —
    /// [`ClientBuilder::build`] returns [`ConfigError::MissingServerKeys`]
    /// without it.
    #[must_use]
    pub fn server_keys(mut self, server_keys: ServerKeys) -> Self {
        self.server_keys = Some(server_keys);
        self
    }

    /// The audience every request binds: vpay's configured public base URL
    /// (`deployment.public_base_url`, without a trailing slash).
    ///
    /// Defaults to `base_url`, which is correct only when this process reaches
    /// vpay at the URL vpay publishes as its own. When it does not — a merchant
    /// server calling `http://vpay-server:8080` inside a compose network, a
    /// private DNS name in production — set it to the public one
    /// (`vpay_api::envelope::audience_for`): the audience is a *name*, never the
    /// `Host`, so that a request signed for one deployment cannot be replayed at
    /// another, and a mismatch has no symptom but the coarse `401`.
    ///
    /// ```
    /// # use vpay_sdk::{Client, Credentials, ServerKeys};
    /// # fn demo(credentials: Credentials, keys: ServerKeys) -> Result<Client, vpay_sdk::ConfigError> {
    /// Client::builder("http://vpay-server:8080")
    ///     .credentials(credentials)
    ///     .server_keys(keys)
    ///     .audience("https://api.vpay.example")
    ///     .build()
    /// # }
    /// ```
    #[must_use]
    pub fn audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// The HTTP client's request timeout.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Validates configuration and constructs the [`Client`].
    ///
    /// # Errors
    /// [`ConfigError::MissingCredentials`] / [`ConfigError::MissingServerKeys`]
    /// if either was never set; [`ConfigError::InvalidPublicKey`] for a pinned
    /// key that is not a curve point; [`ConfigError::Envelope`] if the
    /// transport cannot be assembled; [`ConfigError::Http`] if the underlying
    /// `reqwest::Client` fails to build.
    pub fn build(self) -> Result<Client, ConfigError> {
        let credentials = self.credentials.ok_or(ConfigError::MissingCredentials)?;
        let server_keys = self.server_keys.ok_or(ConfigError::MissingServerKeys)?;

        let base_url = self.base_url.trim_end_matches('/').to_string();
        let audience = self
            .audience
            .map(|audience| audience.trim_end_matches('/').to_owned())
            .unwrap_or_else(|| base_url.clone());
        let resource_base = format!("{base_url}/v1");

        let resolver = server_keys
            .verify_keys()?
            .into_iter()
            .fold(StaticVerifierResolver::new(), |resolver, key| {
                resolver.with_key(key)
            });
        let cose = CoseEnvelope::client(CoseMode::Sign1, credentials.signer(), Arc::new(resolver))
            .build()
            .map_err(|e| ConfigError::Envelope(e.to_string()))?;
        let envelope = ClientEnvelope::new(cose, audience.clone())
            .map_err(|e| ConfigError::Envelope(e.to_string()))?;

        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            // The bare config, not `Some(config)`: `tls_backend_preconfigured`
            // wraps its argument in an `Option` itself before downcasting, so
            // an already-`Option` argument silently becomes
            // `UnknownPreconfigured` and `build()` returns "builder error".
            .tls_backend_preconfigured(rustls_client_config()?)
            .build()
            .map_err(|e| ConfigError::Http(e.to_string()))?;

        Ok(Client {
            inner: Arc::new(Inner {
                http,
                credentials,
                envelope,
                resource_base,
                audience,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials::ed25519_seed("svc-1", &[7; 32])
    }

    /// A real curve point: pinned keys are checked when the client is built.
    fn public_of(seed: u8) -> [u8; 32] {
        cratestack_client_rust::cose::Ed25519Signer::from_seed(&[seed; 32])
            .verify_key()
            .ed25519_bytes()
            .expect("an Ed25519 key")
    }

    fn keys() -> ServerKeys {
        ServerKeys::new(public_of(1), public_of(2))
    }

    fn builder(base: &str) -> ClientBuilder {
        Client::builder(base)
            .credentials(creds())
            .server_keys(keys())
    }

    #[test]
    fn derives_the_documented_defaults() {
        let client = builder("https://api.vpay.example").build().unwrap();
        assert_eq!(client.inner.resource_base, "https://api.vpay.example/v1");
        assert_eq!(client.inner.audience, "https://api.vpay.example");
    }

    #[test]
    fn strips_a_trailing_slash_from_base_url_and_audience() {
        let client = builder("https://api.vpay.example/")
            .audience("https://public.vpay.example/")
            .build()
            .unwrap();
        assert_eq!(client.inner.resource_base, "https://api.vpay.example/v1");
        assert_eq!(client.inner.audience, "https://public.vpay.example");
    }

    #[test]
    fn an_explicit_audience_does_not_move_where_requests_go() {
        let client = builder("http://vpay-server:8080")
            .audience("https://api.vpay.example")
            .build()
            .unwrap();
        assert_eq!(client.inner.resource_base, "http://vpay-server:8080/v1");
        assert_eq!(client.inner.audience, "https://api.vpay.example");
    }

    #[test]
    fn build_fails_without_credentials() {
        let result = Client::builder("https://api.vpay.example")
            .server_keys(keys())
            .build();
        assert!(matches!(result, Err(ConfigError::MissingCredentials)));
    }

    #[test]
    fn build_fails_without_pinned_server_keys() {
        let result = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .build();
        assert!(matches!(result, Err(ConfigError::MissingServerKeys)));
    }

    #[test]
    fn build_refuses_a_pinned_key_that_is_not_a_point() {
        // About half of all y coordinates have no x on the curve: take the
        // first one the library itself refuses.
        let not_a_point = (2_u8..=255)
            .map(|first| {
                let mut bytes = [0_u8; 32];
                bytes[0] = first;
                bytes
            })
            .find(|bytes| cratestack_client_rust::cose::CoseVerifyKey::ed25519(bytes).is_err())
            .expect("some y is not a curve point");
        let result = Client::builder("https://api.vpay.example")
            .credentials(creds())
            .server_keys(ServerKeys::new(not_a_point, public_of(2)))
            .build();
        assert!(
            matches!(result, Err(ConfigError::InvalidPublicKey)),
            "{result:?}"
        );
    }

    #[test]
    fn the_tls_config_carries_vendored_roots_and_advertises_http2() {
        // Both properties are invisible from the outside: an empty root store
        // fails only against a real server (there is no live-TLS test — see
        // `tests/tls.rs`), and missing ALPN silently downgrades every HTTPS
        // connection to HTTP/1.1, since reqwest only sets ALPN on the code
        // path this crate deliberately does not take.
        let roots = vendored_root_store();
        assert!(
            roots.roots.len() > 100,
            "the vendored Mozilla bundle should hold hundreds of anchors, got {}",
            roots.roots.len()
        );

        let config = rustls_client_config().expect("the rustls config builds");
        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }

    #[test]
    fn bounded_prefix_cuts_on_a_character_boundary_and_keeps_real_replacements() {
        // Under the byte limit: returned whole.
        assert_eq!(bounded_prefix("héllo".as_bytes()), "héllo");

        // Exactly the limit, all ASCII: 500 bytes, nothing dropped.
        let ascii = "x".repeat(10_000);
        assert_eq!(bounded_prefix(ascii.as_bytes()).len(), BODY_PREFIX_LIMIT);

        // 3-byte characters: 166 fit in 498 bytes and the 167th straddles the
        // cut, so it is dropped rather than replaced.
        let three_byte = "€".repeat(1_000);
        let cut = bounded_prefix(three_byte.as_bytes());
        assert_eq!(cut.len(), 498);
        assert!(!cut.contains('\u{FFFD}'));

        // 4-byte characters: 125 fit exactly in 500 bytes, so nothing is
        // dropped — the back-off must not fire when the cut already lands on
        // a boundary.
        let four_byte = "😀".repeat(1_000);
        let cut = bounded_prefix(four_byte.as_bytes());
        assert_eq!(cut.len(), BODY_PREFIX_LIMIT);
        assert_eq!(cut.chars().count(), 125);

        // A byte that is invalid UTF-8 wherever it sits is still rendered as
        // U+FFFD: only a character *truncated by the cut* is dropped.
        assert!(bounded_prefix(b"ok\xffthen").contains('\u{FFFD}'));
    }

    #[test]
    fn client_debug_output_never_contains_the_seed() {
        let client = builder("https://api.vpay.example").build().unwrap();
        let debug = format!("{client:?}");
        assert!(debug.contains("[redacted]"));
        // The seed is [7; 32]; neither its base64url nor its raw bytes appear.
        assert!(!debug.contains("BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc"));
    }

    #[test]
    fn builder_debug_output_never_contains_the_seed() {
        let debug = format!("{:?}", builder("https://api.vpay.example"));
        assert!(!debug.contains("BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc"));
    }
}
