//! A typed client for the Notch Pay REST API, written from its own reference.
//!
//! Notch Pay publishes SDKs for JavaScript, PHP, Python, Laravel and mobile,
//! and none for Rust. This crate is vpay's own, transcribed from the
//! OpenAPI specification Notch Pay publishes (**2.1.0**,
//! `https://developer.notchpay.co/openapi.yaml`, read 2026-10-20) and from
//! its API reference pages. It is internal to the workspace (`publish =
//! false`) and exists to be called by a `notchpay` adapter.
//!
//! # What is built, and what is not — read this first
//!
//! **No adapter uses this crate yet, and nothing in any binary links it.**
//! [RFC-0012](../../../../docs/rfc/0012-notchpay-adapter.md) designs the
//! adapter and records why it is not written: Notch Pay's status read is
//! documented as authenticated by the key Notch Pay calls public, and
//! [ADR-0029](../../../../docs/adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
//! D1's third hard rule refuses an adapter to such a provider. Resolving that
//! is the maintainer's decision (RFC-0012, delivery step 0). This crate does
//! not resolve it, and it does not move money on its own: it sends what a
//! caller tells it to and reports what Notch Pay answered.
//!
//! Nothing here has called Notch Pay. Every shape is the specification's, and
//! the specification and Notch Pay's guides disagree in places that RFC-0012
//! § 10 lists. The crate follows the specification and says so at each one.
//!
//! # The operations, and why only these
//!
//! | Operation | Notch Pay endpoint | Method here |
//! | --- | --- | --- |
//! | Initialize a payment | `POST /payments` | [`Client::initialize_payment`] |
//! | Process a payment (the USSD prompt) | `POST /payments/{reference}` | [`Client::process_payment`] |
//! | Retrieve a payment (the authoritative read) | `GET /payments/{reference}` | [`Client::retrieve_payment`] |
//! | List webhook endpoints | `GET /webhooks` | [`Client::list_webhooks`] |
//! | Create a webhook endpoint | `POST /webhooks` | [`Client::create_webhook`] |
//! | A webhook event (inbound) | — | [`parse_event`] |
//!
//! The two webhook calls are here because the maintainer asked on 2026-10-20
//! that vpay register its own endpoints rather than depend on a merchant
//! doing it by hand ([RFC-0013](../../../../docs/rfc/0013-vpay-registers-its-own-callbacks.md)).
//! The client only performs them; when, how often and how a duplicate is
//! avoided is that RFC's design, not this crate's.
//!
//! Deliberately absent:
//!
//! - **Refunds.** `POST /refunds` appears in a Notch Pay guide and not in the
//!   specification or the API reference, and the guide's request carries no
//!   reference that would make a replay safe. RFC-0012 § 7 is the whole
//!   argument; the adapter will declare a `NotImplemented` token instead.
//! - **Cancel** (`DELETE /payments/{reference}`). The port has no
//!   cancel-at-rail operation, and a courtesy call in the money path is a
//!   call with no test behind it.
//! - **Transfers, beneficiaries, customers, balance, channels**, and webhook
//!   **update and delete**. vpay calls none of them. Never deleting an
//!   endpoint is deliberate: an account's other endpoints are not vpay's.
//! - **Webhook signature verification.** Notch Pay signs with HMAC-SHA256 in
//!   `x-notch-signature`, but the port's `parse_callback` receives no headers
//!   (RFC-0010 open question 4), so a verifier would be code nothing can
//!   call.
//!
//! # Every call carries the private key
//!
//! The specification requires only `Authorization` (the public key) on the
//! payment endpoints, and `X-Grant` (the private key) on transfers, balance,
//! beneficiaries and webhooks. [`Client`] sends **both on every request**.
//! That is what RFC-0012's way A would require of the read, and a server
//! that ignores the header loses nothing by receiving it.
//!
//! # No error type of its own
//!
//! A call returns `Result<Answer<T>, ProviderError>`. The `Err` side is the
//! port's [`ProviderError`] and only ever one of two variants, both decided
//! without knowing what a payment means: `Transport` (the request did not
//! complete, or Notch Pay answered `429` or `5xx`, so the payment's fate is
//! unknown) and `Malformed` (an answer that is not the documented shape). A
//! `4xx` is **not** an error here: it is [`Answer::Refused`], because what a
//! `409` or a `422` means for a charge is the adapter's mapping, and a
//! client that turned it into a decline would be deciding that in a library.
//! A leaf error enum would be a second place to classify the same failures,
//! which is why the two existing adapters do not define one either.

pub mod status;
pub mod wire;

use std::collections::BTreeMap;
use std::fmt;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use vpay_core::privacy::Secret;
use vpay_provider::ProviderError;
use vpay_provider::http::{path_segment, read_rail_body};

use crate::wire::{
    CreateWebhook, ErrorEnvelope, Event, InitializePayment, PaymentEnvelope, ProcessPayment,
    WebhookEnvelope, WebhookPage,
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
    /// header value. The message names which key and never contains it:
    /// a key with a stray newline is a configuration mistake, and reporting it
    /// must not put the key in a log.
    pub fn new(public_key: &str, private_key: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            public_key: sensitive_header("public_key", public_key)?,
            private_key: sensitive_header("private_key", private_key)?,
            sync_account: None,
        })
    }

    /// Adds the Sync connected account, sent as `X-Sync` on every call.
    ///
    /// RFC-0012 § 2 does **not** offer Sync until Notch Pay confirms where a
    /// connected account's money lands. The method exists because the
    /// header is part of the API reference and costs nothing to carry; no
    /// caller passes one.
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

/// Every key hidden; whether a Sync account is set is a fact about the
/// configuration and prints as `Some`/`None`.
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
    /// A `4xx` other than `429`, with Notch Pay's error envelope.
    Refused(Refusal),
}

/// A `4xx` answer: the status, and Notch Pay's error envelope when the body
/// was one.
///
/// `message` and `errors` are Notch Pay's own prose — "A payment with this
/// reference already exists", "Phone number is required for mobile money
/// payments" — and a validation message can quote the value it refused.
/// They are for an operator's eyes through the adapter's mapping, never for
/// a log line, which is why `Debug` prints the field **names** of `errors`
/// and nothing of what they say.
pub struct Refusal {
    /// The HTTP status: `400`, `401`, `403`, `404`, `409` and `422` are the
    /// ones Notch Pay documents.
    pub http_status: u16,
    /// The envelope's `message`, if the body was an envelope.
    pub message: Option<String>,
    /// The envelope's `errors`, from a field name to its messages.
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

/// A Notch Pay account, reached through the `reqwest::Client` the caller
/// built.
///
/// The client is the caller's because its timeouts are the caller's: the
/// binary builds one at boot with `vpay_provider::http::client_with_timeouts`
/// and every adapter shares that discipline.
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    keys: Keys,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for the account `keys` opens, at `base_url` — Notch Pay's
    /// `https://api.notchpay.co` for both its sandbox and its production, or
    /// a WireMock host in a test deployment. A trailing `/` is dropped.
    #[must_use]
    pub fn new(http: reqwest::Client, base_url: impl Into<String>, keys: Keys) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            http,
            base_url,
            keys,
        }
    }

    /// `POST /payments`. The specification documents `201` on success.
    ///
    /// # Errors
    ///
    /// `Transport` or `Malformed`, as the crate docs describe. A duplicate
    /// reference is `Ok(Answer::Refused)` with `http_status` 409, per Notch
    /// Pay's errors page (the specification itself omits the `409`).
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
    /// `trx.…` — is the specification and the direct-charge guide
    /// disagreeing; the caller decides, and RFC-0012 § 10 item 2 settles it.
    /// The reference is percent-encoded as one path segment, so no value can
    /// move the request to another endpoint.
    ///
    /// # Errors
    ///
    /// As for [`Client::initialize_payment`].
    pub async fn process_payment(
        &self,
        reference: &str,
        body: &ProcessPayment<'_>,
    ) -> Result<Answer<PaymentEnvelope>, ProviderError> {
        let path = format!("/payments/{}", path_segment(reference));
        let request = self.request(Method::POST, &path).json(body);
        self.send(request, "notchpay: processing a payment").await
    }

    /// `GET /payments/{reference}`, the read. The specification documents
    /// `202` on success and `404` for an unknown reference; the `404` is
    /// `Ok(Answer::Refused)` here, and the adapter turns it into
    /// `ChargeStatus::NotFound`.
    ///
    /// # Errors
    ///
    /// As for [`Client::initialize_payment`].
    pub async fn retrieve_payment(
        &self,
        reference: &str,
    ) -> Result<Answer<PaymentEnvelope>, ProviderError> {
        let path = format!("/payments/{}", path_segment(reference));
        let request = self.request(Method::GET, &path);
        self.send(request, "notchpay: reading a payment").await
    }

    /// `GET /webhooks?page={page}`, one page of the account's endpoints. The
    /// specification documents `200` and requires the private key, which
    /// every call here sends anyway.
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

    /// `POST /webhooks`. The specification documents `201` and a `422` for
    /// a body it refuses.
    ///
    /// Not idempotent as documented: the specification names no idempotency
    /// header and no refusal of a duplicate URL, so two calls make two
    /// endpoints. A caller lists first.
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
        request
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        context: &str,
    ) -> Result<Answer<T>, ProviderError> {
        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::transport_from(context, error))?;
        let (status, body) = read_rail_body(response, context).await?;
        interpret(status, &body, context)
    }
}

/// Decodes a webhook body Notch Pay POSTed.
///
/// # Errors
///
/// [`ProviderError::Malformed`] if the body is not an event. The message is
/// fixed and never quotes the body or serde's account of it, which can quote
/// a value.
pub fn parse_event(body: &[u8]) -> Result<Event, ProviderError> {
    serde_json::from_slice(body)
        .map_err(|_| ProviderError::malformed("notchpay: the webhook body is not an event"))
}

/// What a completed exchange means at the HTTP level, and nothing more.
///
/// Pure, so the whole table is a unit test:
///
/// | Status | Result |
/// | --- | --- |
/// | `2xx` with the documented shape | `Ok(Success)` |
/// | `2xx` with anything else | `Err(Malformed)` |
/// | `429`, `5xx` | `Err(Transport)` — the payment's fate is unknown |
/// | any other `4xx` | `Ok(Refused)`, with the envelope if the body is one |
/// | `1xx`, `3xx` | `Err(Malformed)` — redirects are not followed into a payment API |
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
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(ProviderError::transport(format!(
            "{context}: Notch Pay answered {status}"
        )));
    }
    if status.is_client_error() {
        let envelope = serde_json::from_slice::<ErrorEnvelope>(body).ok();
        let (message, errors) = envelope.map_or((None, BTreeMap::new()), |envelope| {
            (envelope.message, envelope.errors.unwrap_or_default())
        });
        return Ok(Answer::Refused(Refusal {
            http_status: status.as_u16(),
            message,
            errors,
        }));
    }
    Err(ProviderError::malformed(format!(
        "{context}: an unexpected {status}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::ProcessData;

    const PUBLIC: &str = "pk_test_public0001";
    const PRIVATE: &str = "sk_test_private0002";

    fn http() -> reqwest::Client {
        vpay_provider::http::client().expect("the vendored-roots client builds")
    }

    fn client(base_url: &str) -> Client {
        Client::new(
            http(),
            base_url,
            Keys::new(PUBLIC, PRIVATE).expect("valid keys"),
        )
    }

    fn header<'a>(request: &'a reqwest::Request, name: &str) -> Option<&'a str> {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
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
        let keys = Keys::new(PUBLIC, PRIVATE)
            .and_then(|keys| keys.with_sync_account("sync_123456789"))
            .expect("valid keys");
        let request = Client::new(http(), "https://api.notchpay.co", keys)
            .request(Method::GET, "/payments/abc")
            .build()
            .expect("builds");
        assert_eq!(header(&request, "x-sync"), Some("sync_123456789"));
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_the_separator() {
        let request = client("http://wiremock-notchpay:8080//")
            .request(Method::POST, "/payments")
            .build()
            .expect("builds");
        assert_eq!(
            request.url().as_str(),
            "http://wiremock-notchpay:8080/payments"
        );
    }

    #[test]
    fn a_reference_cannot_move_the_request_to_another_endpoint() {
        let client = client("https://api.notchpay.co");
        let path = format!("/payments/{}", path_segment("../balance?x=1"));
        let request = client.request(Method::GET, &path).build().expect("builds");
        assert_eq!(
            request.url().path(),
            "/payments/..%2Fbalance%3Fx%3D1",
            "the reference stayed one segment"
        );
        assert_eq!(request.url().query(), None);
    }

    #[test]
    fn a_key_that_is_empty_or_not_a_header_value_is_a_config_error_that_does_not_echo_it() {
        for (public, private, named) in [
            ("", PRIVATE, "public_key"),
            ("   ", PRIVATE, "public_key"),
            (PUBLIC, "sk_test_with\nnewline", "private_key"),
        ] {
            let error = Keys::new(public, private).expect_err("refused");
            assert!(matches!(error, ProviderError::Config(_)), "{error:?}");
            let printed = format!("{error} {error:?}");
            assert!(printed.contains(named), "{printed}");
            assert!(!printed.contains("sk_test_with"), "{printed}");
        }
    }

    #[test]
    fn no_debug_output_prints_a_key() {
        let keys = Keys::new(PUBLIC, PRIVATE)
            .and_then(|keys| keys.with_sync_account("sync_secret03"))
            .expect("valid keys");
        let client = Client::new(http(), "https://api.notchpay.co", keys);
        let request = client
            .request(Method::GET, "/payments/abc")
            .build()
            .expect("builds");
        let printed = format!("{client:?} {request:?}");
        for leaked in [PUBLIC, PRIVATE, "sync_secret03"] {
            assert!(!printed.contains(leaked), "{leaked} printed: {printed}");
        }
        assert!(printed.contains("https://api.notchpay.co"), "{printed}");
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
        let payment = envelope.transaction.expect("has a transaction");
        assert_eq!(payment.status.as_deref(), Some("pending"));
    }

    #[test]
    fn a_success_with_an_undocumented_body_is_malformed_and_never_quotes_it() {
        let result: Result<Answer<PaymentEnvelope>, _> =
            interpret(StatusCode::ACCEPTED, b"+237670000000 not json", "test");
        let error = result.expect_err("refused");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
        let printed = format!("{error} {error:?}");
        assert!(!printed.contains("+237670000000"), "{printed}");
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
    fn a_rate_limit_or_a_server_error_leaves_the_fate_unknown() {
        for status in [
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
    fn a_redirect_is_not_followed_into_a_payment_api() {
        let result: Result<Answer<PaymentEnvelope>, _> = interpret(StatusCode::FOUND, b"", "test");
        assert!(
            matches!(result, Err(ProviderError::Malformed { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn a_webhook_body_that_is_not_an_event_is_malformed_and_never_quoted() {
        let error = parse_event(b"payer +237670000000").expect_err("refused");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
        assert!(!format!("{error} {error:?}").contains("+237670000000"));
    }

    #[tokio::test]
    async fn a_request_that_never_completes_is_transport_and_never_a_refusal() {
        // Nothing listens on loopback port 1.
        let result = client("http://127.0.0.1:1")
            .initialize_payment(&InitializePayment {
                amount: 5000,
                currency: "XAF",
                phone: "+237670000000",
                reference: "6d7c0f1e-0000-4000-8000-000000000001",
                description: None,
            })
            .await;
        assert!(
            matches!(result, Err(ProviderError::Transport { .. })),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn the_two_payment_paths_are_built_from_one_escaped_segment() {
        // Also a closed port: the assertion is on the error's context, which
        // names the operation, not on any answer.
        let client = client("http://127.0.0.1:1");
        let process = client
            .process_payment(
                "trx.123",
                &ProcessPayment {
                    channel: "cm.mobile",
                    data: ProcessData {
                        phone: "+237670000000",
                    },
                },
            )
            .await;
        let read = client.retrieve_payment("trx.123").await;
        for (result, operation) in [(process, "processing"), (read, "reading")] {
            let error = result.expect_err("nothing listens");
            assert!(
                matches!(error, ProviderError::Transport { .. }),
                "{error:?}"
            );
            assert!(error.to_string().contains(operation), "{error}");
        }
    }
}
