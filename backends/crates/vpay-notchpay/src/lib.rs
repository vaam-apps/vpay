//! A typed client for the Notch Pay REST API, transcribed from Notch Pay's
//! OpenAPI specification 2.1.0, for vpay's own use (`publish = false`).
//!
//! **`vpay-adapter-notchpay` uses it since 2026-10-20**, admitted under
//! ADR-0031 (ADR-0029 D1's third rule amended), and `vpay-server` links that
//! adapter. It sends what a caller asks and reports what Notch Pay answered;
//! deciding what an answer means for a charge is the adapter's
//! ([RFC-0012](../../../../docs/rfc/0012-notchpay-adapter.md)). **Nothing here
//! has called Notch Pay.** Why it covers what it covers, why every
//! call carries the private key, why a `4xx` is an answer and not an error,
//! and every place the specification and Notch Pay's guides disagree are in
//! [docs/reference/vpay-notchpay.md](../../../../docs/reference/vpay-notchpay.md).
//!
//! | Operation | Notch Pay endpoint | Here |
//! | --- | --- | --- |
//! | Initialize a payment | `POST /payments` | [`Client::initialize_payment`] |
//! | Process a payment (the USSD prompt) | `POST /payments/{reference}` | [`Client::process_payment`] |
//! | Retrieve a payment (the read) | `GET /payments/{reference}` | [`Client::retrieve_payment`] |
//! | List webhook endpoints | `GET /webhooks` | [`Client::list_webhooks`] |
//! | Create a webhook endpoint | `POST /webhooks` | [`Client::create_webhook`] |
//! | A webhook event (inbound) | — | [`parse_event`] |

pub mod status;
pub mod wire;

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use vpay_core::privacy::{SafeUrl, Secret};
use vpay_provider::ProviderError;
use vpay_provider::http::{path_segment, read_rail_body};

use crate::wire::{
    CreateWebhook, Event, InitializePayment, PaymentEnvelope, ProcessPayment, WebhookEnvelope,
    WebhookPage,
};

/// The private-key header, as the specification names it (`GrantAuth`).
const X_GRANT: &str = "X-Grant";
/// The connected-account header (`SyncAuth`), sent only on Sync.
const X_SYNC: &str = "X-Sync";

/// One Notch Pay account's keys, already validated as header values.
///
/// The keys are held as [`HeaderValue`]s marked sensitive, so `reqwest`'s own
/// `Debug` of a request does not print them either.
pub struct Keys {
    public_key: HeaderValue,
    private_key: HeaderValue,
    sync_account: Option<HeaderValue>,
}

impl Keys {
    /// The account's public key (`Authorization`) and private key
    /// (`X-Grant`).
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] if either is empty or is not a valid HTTP
    /// header value. The message names which key and never contains it.
    pub fn new(public_key: &str, private_key: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            public_key: sensitive_header("public_key", public_key)?,
            private_key: sensitive_header("private_key", private_key)?,
            sync_account: None,
        })
    }

    /// Adds the Sync connected account, sent as `X-Sync` on every call.
    /// RFC-0012 § 2 does not offer Sync until Notch Pay confirms where a
    /// connected account's money lands, so no caller passes one yet.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] as for [`Keys::new`].
    pub fn with_sync_account(mut self, sync_account: &str) -> Result<Self, ProviderError> {
        self.sync_account = Some(sensitive_header("sync_account", sync_account)?);
        Ok(self)
    }
}

fn sensitive_header(name: &str, value: &str) -> Result<HeaderValue, ProviderError> {
    if value.trim().is_empty() {
        return Err(ProviderError::Config(format!("notchpay: {name} is empty")));
    }
    let mut header = HeaderValue::from_str(value).map_err(|_| {
        ProviderError::Config(format!("notchpay: {name} is not a valid HTTP header value"))
    })?;
    header.set_sensitive(true);
    Ok(header)
}

/// Every key hidden; whether a Sync account is set prints as `Some`/`None`.
impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keys")
            .field("public_key", &Secret::new(&self.public_key))
            .field("private_key", &Secret::new(&self.private_key))
            .field("sync_account", &self.sync_account.as_ref().map(Secret::new))
            .finish()
    }
}

/// What Notch Pay answered to a call that completed.
#[derive(Debug)]
pub enum Answer<T> {
    /// A `2xx`, decoded into the documented shape.
    Success(T),
    /// A `4xx` other than `408` and `429`, with Notch Pay's error envelope.
    Refused(Refusal),
}

/// A `4xx` answer: the status, and Notch Pay's error envelope as far as the
/// body was one.
///
/// `message` and `errors` are Notch Pay's own prose, and a validation message
/// can quote the value it refused, so `Debug` prints the field **names** of
/// `errors` and nothing of what they say.
pub struct Refusal {
    /// The HTTP status: `400`, `401`, `403`, `404`, `409` and `422` are the
    /// ones Notch Pay documents.
    pub http_status: u16,
    /// The envelope's `message`, when the body carried one as a string.
    pub message: Option<String>,
    /// The envelope's `errors`, from a field name to its messages, keeping
    /// only the entries that are an array of strings.
    pub errors: BTreeMap<String, Vec<String>>,
}

impl fmt::Debug for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Refusal")
            .field("http_status", &self.http_status)
            .field("message", &self.message.as_ref().map(Secret::new))
            .field("errors", &self.errors.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A Notch Pay account, reached through a `reqwest::Client` the caller built.
///
/// **The client must not follow redirects.** A followed redirect replays
/// every header `reqwest` does not consider sensitive to the new host, and
/// `X-Grant` is one of them. `vpay_provider::http::client_with_timeouts` and
/// `vpay_provider::http::client` both refuse redirects; build it with one of
/// them, as the binary does at boot.
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    keys: Keys,
    /// The whole-request deadline every call carries, when the caller set
    /// one with [`Client::with_request_timeout`].
    request_timeout: Option<Duration>,
}

/// The base URL prints as `scheme://host[:port]` only, as every URL in this
/// workspace's `Debug` output does.
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &SafeUrl::new(&self.base_url))
            .field("keys", &self.keys)
            .field("request_timeout", &self.request_timeout)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for the account `keys` opens, at `base_url`: Notch Pay's
    /// `https://api.notchpay.co` for both its sandbox and its production, or
    /// a WireMock host in a test deployment.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] unless `base_url` is an absolute `http` or
    /// `https` URL with a host and no user information, query or fragment. A
    /// URL that every call would fail to use is refused here, at boot, rather
    /// than turning into a `Transport` error on every charge — a `Transport`
    /// says the payment's fate is unknown, and a request that was never sent
    /// has no fate. The message never quotes the URL, which could carry a
    /// password in its user information.
    pub fn new(http: reqwest::Client, base_url: &str, keys: Keys) -> Result<Self, ProviderError> {
        let refused = |why: &str| ProviderError::Config(format!("notchpay: the base URL {why}"));
        let url = Url::parse(base_url).map_err(|_| refused("is not an absolute URL"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(refused("is not http or https"));
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(refused("has no host"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(refused("carries user information"));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(refused("carries a query or a fragment"));
        }
        Ok(Self {
            http,
            base_url: url.as_str().trim_end_matches('/').to_owned(),
            keys,
            request_timeout: None,
        })
    }

    /// Every call made through this client carries `timeout` as its
    /// whole-request deadline.
    ///
    /// The adapter sets it from `ProviderConfig::request_timeout`, because the
    /// process's one `reqwest::Client` is shared by every rail and carries the
    /// default deadline only (`vpay_provider::ProviderConfig`'s
    /// `request_timeout` field says why the deadline lives on the
    /// configuration). Without it a configured deadline would be ignored, and
    /// the conformance case that proves a slow rail is a `Transport` error
    /// would pass on the client's default instead of on the adapter's
    /// configuration — exactly what it caught on `orange_money`.
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    /// `POST /payments`. The specification documents `201` on success.
    ///
    /// # Errors
    ///
    /// `Transport` (the fate is unknown) or `Malformed` (an answer that is
    /// not the documented shape). A duplicate reference is
    /// `Ok(Answer::Refused)` with `http_status` 409, per Notch Pay's errors
    /// page; the specification itself omits the `409`.
    pub async fn initialize_payment(
        &self,
        body: &InitializePayment<'_>,
    ) -> Result<Answer<PaymentEnvelope>, ProviderError> {
        let request = self.request(Method::POST, "/payments").json(body);
        self.send(request, "notchpay: initializing a payment").await
    }

    /// `POST /payments/{reference}`, which sends the payer the prompt. The
    /// specification documents `202` on success.
    ///
    /// Which reference the path takes — the caller's or Notch Pay's own
    /// `trx.…` — is a disagreement between the specification and a guide;
    /// RFC-0012 § 10 item 2 settles it in the sandbox.
    ///
    /// # Errors
    ///
    /// As for [`Client::initialize_payment`], and `Malformed` without any
    /// request if `reference` cannot be one path segment (see
    /// [`Client::retrieve_payment`]).
    pub async fn process_payment(
        &self,
        reference: &str,
        body: &ProcessPayment<'_>,
    ) -> Result<Answer<PaymentEnvelope>, ProviderError> {
        let request = self.payment_request(Method::POST, reference)?.json(body);
        self.send(request, "notchpay: processing a payment").await
    }

    /// `GET /payments/{reference}`, the read. The specification documents
    /// `202` on success and `404` for an unknown reference.
    ///
    /// A `404` is `Ok(Answer::Refused)` and **not** necessarily "no such
    /// payment": an unknown `X-Sync` account is a `404` too ("Sync account
    /// not found"), and so is every call against a base URL with a wrong
    /// path. Which `404` this is, is the adapter's to tell from Notch Pay's
    /// `message` (RFC-0012 § 3).
    ///
    /// # Errors
    ///
    /// As for [`Client::initialize_payment`], and `Malformed` without any
    /// request if `reference` is empty or made only of dots. Those would
    /// resolve to another endpoint — `/payments/` is the list, and
    /// `/payments/..` is the API root — and an escape does not help, because
    /// URL parsing treats `%2E%2E` as `..` as well. The reference can be
    /// Notch Pay's own, from a stored answer, so the rail is one of the
    /// callers this guards against.
    pub async fn retrieve_payment(
        &self,
        reference: &str,
    ) -> Result<Answer<PaymentEnvelope>, ProviderError> {
        let request = self.payment_request(Method::GET, reference)?;
        self.send(request, "notchpay: reading a payment").await
    }

    /// `GET /webhooks?page={page}`, one page of the account's endpoints. The
    /// specification documents `200`.
    ///
    /// # Errors
    ///
    /// As for [`Client::initialize_payment`].
    pub async fn list_webhooks(&self, page: u32) -> Result<Answer<WebhookPage>, ProviderError> {
        let path = format!("/webhooks?page={page}");
        let request = self.request(Method::GET, &path);
        self.send(request, "notchpay: listing webhook endpoints")
            .await
    }

    /// `POST /webhooks`. The specification documents `201` and a `422` for a
    /// body it refuses. **Not idempotent as documented**: no idempotency
    /// header and no refusal of a duplicate URL, so a caller lists first.
    ///
    /// # Errors
    ///
    /// As for [`Client::initialize_payment`].
    pub async fn create_webhook(
        &self,
        body: &CreateWebhook<'_>,
    ) -> Result<Answer<WebhookEnvelope>, ProviderError> {
        let request = self.request(Method::POST, "/webhooks").json(body);
        self.send(request, "notchpay: creating a webhook endpoint")
            .await
    }

    fn payment_request(
        &self,
        method: Method,
        reference: &str,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let path = format!("/payments/{}", reference_segment(reference)?);
        Ok(self.request(method, &path))
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.base_url);
        let mut request = self
            .http
            .request(method, url)
            .header(AUTHORIZATION, self.keys.public_key.clone())
            .header(X_GRANT, self.keys.private_key.clone())
            .header(ACCEPT, "application/json");
        if let Some(sync_account) = &self.keys.sync_account {
            request = request.header(X_SYNC, sync_account.clone());
        }
        if let Some(timeout) = self.request_timeout {
            request = request.timeout(timeout);
        }
        request
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        context: &str,
    ) -> Result<Answer<T>, ProviderError> {
        // `without_url`: a payment path carries a reference, and the error's
        // text would otherwise end in the full URL.
        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::transport_from(context, error.without_url()))?;
        let (status, body) = read_rail_body(response, context).await?;
        interpret(status, &body, context)
    }
}

/// One path segment for a payment reference, or a refusal.
fn reference_segment(reference: &str) -> Result<String, ProviderError> {
    if reference.chars().all(|c| c == '.') {
        return Err(ProviderError::malformed(
            "notchpay: a payment reference that is empty or only dots cannot be a path segment",
        ));
    }
    Ok(path_segment(reference))
}

/// Decodes a webhook body Notch Pay POSTed.
///
/// # Errors
///
/// [`ProviderError::Malformed`] if the body is not an event with a `data`
/// object. The message is fixed and never quotes the body or serde's account
/// of it, which can quote a value.
pub fn parse_event(body: &[u8]) -> Result<Event, ProviderError> {
    serde_json::from_slice(body)
        .map_err(|_| ProviderError::malformed("notchpay: the webhook body is not an event"))
}

/// What a completed exchange means at the HTTP level, and nothing more.
///
/// | Status | Result |
/// | --- | --- |
/// | `2xx` with the documented shape | `Ok(Success)` |
/// | `2xx` with anything else | `Err(Malformed)` |
/// | `408`, `429`, `5xx` | `Err(Transport)` — the payment's fate is unknown |
/// | any other `4xx` | `Ok(Refused)`, with as much of the envelope as parses |
/// | `1xx`, `3xx` | `Err(Malformed)` — the client follows no redirect |
fn interpret<T: DeserializeOwned>(
    status: StatusCode,
    body: &[u8],
    context: &str,
) -> Result<Answer<T>, ProviderError> {
    if status.is_success() {
        return serde_json::from_slice(body)
            .map(Answer::Success)
            .map_err(|_| {
                ProviderError::malformed(format!(
                    "{context}: a {status} answer whose body is not the documented shape"
                ))
            });
    }
    if matches!(
        status,
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
    {
        return Err(ProviderError::transport(format!(
            "{context}: Notch Pay answered {status}"
        )));
    }
    if status.is_client_error() {
        return Ok(Answer::Refused(refusal(status, body)));
    }
    Err(ProviderError::malformed(format!(
        "{context}: an unexpected {status}"
    )))
}

/// The error envelope, read field by field.
///
/// Not one serde struct: a Laravel API answers an empty error list as `[]`
/// rather than `{}`, and an all-or-nothing decode would then lose `message`
/// too — the one field that says "A payment with this reference already
/// exists".
fn refusal(status: StatusCode, body: &[u8]) -> Refusal {
    let envelope: Option<serde_json::Value> = serde_json::from_slice(body).ok();
    let field = |name: &str| envelope.as_ref().and_then(|value| value.get(name));
    let message = field("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let errors = field("errors")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(name, messages)| {
                    let messages = messages.as_array()?;
                    let strings: Option<Vec<String>> = messages
                        .iter()
                        .map(|m| m.as_str().map(str::to_owned))
                        .collect();
                    Some((name.clone(), strings?))
                })
                .collect()
        })
        .unwrap_or_default();
    Refusal {
        http_status: status.as_u16(),
        message,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::ProcessData;

    const PUBLIC: &str = "public-key-0001";
    const PRIVATE: &str = "private-key-0002";
    const PAYER: &str = "+237670000000";

    fn http() -> reqwest::Client {
        vpay_provider::http::client().expect("the vendored-roots client builds")
    }

    fn keys() -> Keys {
        Keys::new(PUBLIC, PRIVATE).expect("valid keys")
    }

    fn client(base_url: &str) -> Client {
        Client::new(http(), base_url, keys()).expect("a valid base URL")
    }

    fn header<'a>(request: &'a reqwest::Request, name: &str) -> Option<&'a str> {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    fn process_body() -> ProcessPayment<'static> {
        ProcessPayment {
            channel: "cm.mobile",
            data: ProcessData { phone: PAYER },
        }
    }

    /// The deadline the adapter configures is on the request itself, so it
    /// applies whatever the shared `reqwest::Client` was built with — and a
    /// client nobody configured carries none of its own.
    #[test]
    fn a_configured_deadline_rides_on_every_request_and_none_is_invented() {
        let deadline = Duration::from_millis(150);
        let timed = client("https://api.notchpay.co").with_request_timeout(deadline);
        for request in [
            timed.request(Method::GET, "/payments/abc"),
            timed.request(Method::POST, "/payments"),
            timed.request(Method::GET, "/webhooks?page=1"),
        ] {
            let request = request.build().expect("builds");
            assert_eq!(request.timeout(), Some(&deadline));
        }
        let untimed = client("https://api.notchpay.co")
            .request(Method::GET, "/payments/abc")
            .build()
            .expect("builds");
        assert_eq!(untimed.timeout(), None);
    }

    #[test]
    fn every_request_carries_both_keys_and_asks_for_json() {
        let request = client("https://api.notchpay.co")
            .request(Method::GET, "/payments/abc")
            .build()
            .expect("builds");
        assert_eq!(
            request.url().as_str(),
            "https://api.notchpay.co/payments/abc"
        );
        assert_eq!(header(&request, "authorization"), Some(PUBLIC));
        assert_eq!(header(&request, "x-grant"), Some(PRIVATE));
        assert_eq!(header(&request, "accept"), Some("application/json"));
        assert_eq!(header(&request, "x-sync"), None, "no Sync account was set");
    }

    #[test]
    fn a_sync_account_is_sent_as_x_sync() {
        let keys = keys()
            .with_sync_account("sync_123456789")
            .expect("valid keys");
        let request = Client::new(http(), "https://api.notchpay.co", keys)
            .expect("a valid base URL")
            .request(Method::GET, "/payments/abc")
            .build()
            .expect("builds");
        assert_eq!(header(&request, "x-sync"), Some("sync_123456789"));
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_the_separator() {
        for base in [
            "http://wiremock-notchpay:8080",
            "http://wiremock-notchpay:8080/",
        ] {
            let request = client(base)
                .request(Method::POST, "/payments")
                .build()
                .expect("builds");
            assert_eq!(
                request.url().as_str(),
                "http://wiremock-notchpay:8080/payments"
            );
        }
    }

    #[test]
    fn a_base_url_no_call_could_use_is_refused_at_construction_without_echoing_it() {
        for base in [
            "",
            "api.notchpay.co",
            "ftp://api.notchpay.co",
            "https://user:pw-secret@api.notchpay.co",
            "https://api.notchpay.co?key=pw-secret",
            "https://api.notchpay.co#pw-secret",
        ] {
            let error = Client::new(http(), base, keys()).expect_err("refused");
            assert!(
                matches!(error, ProviderError::Config(_)),
                "{base}: {error:?}"
            );
            assert!(
                !format!("{error} {error:?}").contains("pw-secret"),
                "{base}"
            );
        }
    }

    #[test]
    fn both_payment_paths_escape_the_reference_as_one_segment() {
        let client = client("https://api.notchpay.co");
        for method in [Method::GET, Method::POST] {
            for (reference, path) in [
                ("trx.123456789abcdefghj", "/payments/trx.123456789abcdefghj"),
                ("../balance?x=1", "/payments/..%2Fbalance%3Fx%3D1"),
                ("a/b#c", "/payments/a%2Fb%23c"),
            ] {
                let request = client
                    .payment_request(method.clone(), reference)
                    .expect("a usable reference")
                    .build()
                    .expect("builds");
                assert_eq!(request.url().path(), path, "{method} {reference}");
                assert_eq!(request.url().query(), None, "{method} {reference}");
            }
        }
    }

    #[test]
    fn an_empty_or_dot_only_reference_is_refused_before_any_request() {
        let client = client("https://api.notchpay.co");
        for reference in ["", ".", "..", "..."] {
            let error = client
                .payment_request(Method::GET, reference)
                .expect_err("refused");
            assert!(
                matches!(error, ProviderError::Malformed { .. }),
                "{reference:?}: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_public_payment_methods_refuse_a_dot_reference_without_sending() {
        // A closed port: had either method sent a request, the answer would
        // be `Transport`, not `Malformed`.
        let client = client("http://127.0.0.1:1");
        let read = client.retrieve_payment("..").await;
        let process = client.process_payment(".", &process_body()).await;
        for result in [read, process] {
            assert!(
                matches!(result, Err(ProviderError::Malformed { .. })),
                "{result:?}"
            );
        }
    }

    #[test]
    fn a_key_that_is_empty_or_not_a_header_value_is_a_config_error_that_does_not_echo_it() {
        for (public, private, named) in [
            ("", PRIVATE, "public_key"),
            ("   ", PRIVATE, "public_key"),
            (PUBLIC, "refused-private-key\nnewline", "private_key"),
        ] {
            let error = Keys::new(public, private).expect_err("refused");
            assert!(matches!(error, ProviderError::Config(_)), "{error:?}");
            let printed = format!("{error} {error:?}");
            assert!(printed.contains(named), "{printed}");
            for leaked in ["refused-private-key", PUBLIC, PRIVATE] {
                assert!(!printed.contains(leaked), "{leaked} printed: {printed}");
            }
        }
    }

    #[test]
    fn no_debug_output_prints_a_key_or_more_of_the_url_than_its_host() {
        let keys = keys()
            .with_sync_account("sync_secret03")
            .expect("valid keys");
        let client = Client::new(http(), "https://api.notchpay.co/v1-secret-path", keys)
            .expect("a valid base URL");
        let request = client
            .request(Method::GET, "/payments/abc")
            .build()
            .expect("builds");
        let printed = format!("{client:?} {request:?}");
        for leaked in [PUBLIC, PRIVATE, "sync_secret03"] {
            assert!(!printed.contains(leaked), "{leaked} printed: {printed}");
        }
        let client_only = format!("{client:?}");
        assert!(client_only.contains("api.notchpay.co"), "{client_only}");
        assert!(!client_only.contains("v1-secret-path"), "{client_only}");
    }

    #[test]
    fn a_documented_success_decodes() {
        let body = br#"{"status":"Accepted","message":"Payment initialized","code":201,
            "transaction":{"id":"pay_123456789","reference":"order_123","amount":5000,
            "currency":"XAF","status":"pending"}}"#;
        let answer: Answer<PaymentEnvelope> =
            interpret(StatusCode::CREATED, body, "test").expect("decodes");
        let Answer::Success(envelope) = answer else {
            panic!("a 201 is a success: {answer:?}");
        };
        assert_eq!(envelope.transaction.status.as_deref(), Some("pending"));
    }

    #[test]
    fn a_success_missing_its_payload_is_malformed_not_an_empty_success() {
        // `{}`, and the `data` envelope Notch Pay's API reference page shows
        // where the specification says `transaction` / `endpoints`.
        for body in [&b"{}"[..], br#"{"code":200,"data":{"id":"pay_1"}}"#] {
            let payment: Result<Answer<PaymentEnvelope>, _> =
                interpret(StatusCode::ACCEPTED, body, "test");
            assert!(
                matches!(payment, Err(ProviderError::Malformed { .. })),
                "{payment:?}"
            );
        }
        for body in [
            &b"{}"[..],
            br#"{"code":200,"data":[{"id":"wh_1","url":"https://example.com","active":true}]}"#,
        ] {
            let page: Result<Answer<WebhookPage>, _> = interpret(StatusCode::OK, body, "test");
            assert!(
                matches!(page, Err(ProviderError::Malformed { .. })),
                "{page:?}"
            );
            let created: Result<Answer<WebhookEnvelope>, _> =
                interpret(StatusCode::CREATED, body, "test");
            assert!(
                matches!(created, Err(ProviderError::Malformed { .. })),
                "{created:?}"
            );
        }
    }

    #[test]
    fn a_success_whose_body_makes_serde_quote_a_value_is_malformed_and_never_quotes_it() {
        // serde's own message for this body is `invalid type: string
        // "+237670000000", expected …` — it quotes the number, so the
        // assertion below would fail if that message were ever forwarded.
        let body = br#"{"transaction":{"amount":"+237670000000"}}"#;
        let serde_says = serde_json::from_slice::<PaymentEnvelope>(body)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(
            serde_says.contains(PAYER),
            "the canary must make serde quote the value: {serde_says}"
        );
        let result: Result<Answer<PaymentEnvelope>, _> =
            interpret(StatusCode::ACCEPTED, body, "test");
        let error = result.expect_err("refused");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
        assert!(!format!("{error} {error:?}").contains(PAYER));
    }

    #[test]
    fn a_duplicate_reference_is_a_refusal_carrying_notch_pays_words() {
        // The errors page's 409 example, verbatim.
        let body = br#"{"code":409,"status":"Conflict","message":"A payment with this reference already exists"}"#;
        let answer: Answer<PaymentEnvelope> =
            interpret(StatusCode::CONFLICT, body, "test").expect("a refusal is an answer");
        let Answer::Refused(refusal) = answer else {
            panic!("a 409 is a refusal: {answer:?}");
        };
        assert_eq!(refusal.http_status, 409);
        assert_eq!(
            refusal.message.as_deref(),
            Some("A payment with this reference already exists")
        );
        assert!(refusal.errors.is_empty());
    }

    #[test]
    fn an_irregular_errors_field_does_not_cost_the_message() {
        for body in [
            &br#"{"message":"A payment with this reference already exists","errors":[]}"#[..],
            br#"{"message":"A payment with this reference already exists","errors":{"f":"one string"}}"#,
            br#"{"message":"A payment with this reference already exists","errors":{"f":[1,2]}}"#,
        ] {
            let answer: Answer<PaymentEnvelope> =
                interpret(StatusCode::CONFLICT, body, "test").expect("an answer");
            let Answer::Refused(refusal) = answer else {
                panic!("a 409 is a refusal: {answer:?}");
            };
            assert_eq!(
                refusal.message.as_deref(),
                Some("A payment with this reference already exists")
            );
            assert!(refusal.errors.is_empty(), "{refusal:?}");
        }
    }

    #[test]
    fn a_validation_refusal_keeps_its_field_names_and_hides_what_they_say() {
        // The errors page's 422 example for a missing number.
        let body = br#"{"code":422,"status":"Unprocessable Entity","message":"Validation failed",
            "errors":{"data.phone":["Phone number is required for mobile money payments"]}}"#;
        let answer: Answer<PaymentEnvelope> =
            interpret(StatusCode::UNPROCESSABLE_ENTITY, body, "test").expect("an answer");
        let Answer::Refused(refusal) = answer else {
            panic!("a 422 is a refusal: {answer:?}");
        };
        assert_eq!(refusal.http_status, 422);
        assert_eq!(
            refusal.errors.get("data.phone").map(Vec::len),
            Some(1),
            "{refusal:?}"
        );
        let printed = format!("{refusal:?}");
        assert!(printed.contains("data.phone"), "{printed}");
        assert!(!printed.contains("Phone number is required"), "{printed}");
        assert!(!printed.contains("Validation failed"), "{printed}");
    }

    #[test]
    fn a_refusal_whose_body_is_not_an_envelope_is_still_a_refusal() {
        let answer: Answer<PaymentEnvelope> =
            interpret(StatusCode::NOT_FOUND, b"<html>not found</html>", "test").expect("an answer");
        let Answer::Refused(refusal) = answer else {
            panic!("a 404 is a refusal: {answer:?}");
        };
        assert_eq!(refusal.http_status, 404);
        assert_eq!(refusal.message, None);
    }

    #[test]
    fn a_timeout_a_rate_limit_or_a_server_error_leaves_the_fate_unknown() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let result: Result<Answer<PaymentEnvelope>, _> = interpret(status, b"{}", "test");
            assert!(
                matches!(result, Err(ProviderError::Transport { .. })),
                "{status}: {result:?}"
            );
        }
    }

    #[test]
    fn a_3xx_answer_is_malformed_because_the_client_follows_no_redirect() {
        let result: Result<Answer<PaymentEnvelope>, _> = interpret(StatusCode::FOUND, b"", "test");
        assert!(
            matches!(result, Err(ProviderError::Malformed { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn a_webhook_body_that_is_not_an_event_is_malformed_and_never_quoted() {
        // `data` as a string makes serde say `invalid type: string
        // "+237670000000", expected struct EventData`.
        for body in [&b"payer +237670000000"[..], br#"{"data":"+237670000000"}"#] {
            let error = parse_event(body).expect_err("refused");
            assert!(
                matches!(error, ProviderError::Malformed { .. }),
                "{error:?}"
            );
            assert!(!format!("{error} {error:?}").contains(PAYER));
        }
    }

    #[tokio::test]
    async fn a_request_that_never_completes_is_transport_and_prints_no_url() {
        // Nothing listens on loopback port 1.
        let client = client("http://127.0.0.1:1");
        let initialize = client
            .initialize_payment(&InitializePayment {
                amount: 5000,
                currency: "XAF",
                phone: PAYER,
                reference: "6d7c0f1e-0000-4000-8000-000000000001",
                description: None,
            })
            .await;
        let process = client
            .process_payment("trx.secret-ref", &process_body())
            .await;
        let read = client.retrieve_payment("trx.secret-ref").await;
        for (result, operation) in [
            (initialize, "initializing"),
            (process, "processing"),
            (read, "reading"),
        ] {
            let error = result.expect_err("nothing listens");
            assert!(
                matches!(error, ProviderError::Transport { .. }),
                "{error:?}"
            );
            let printed = format!("{error} {error:?}");
            assert!(printed.contains(operation), "{printed}");
            assert!(!printed.contains("trx.secret-ref"), "{printed}");
        }
    }
}
