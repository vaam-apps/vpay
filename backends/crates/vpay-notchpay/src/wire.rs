//! The JSON bodies of the Notch Pay operations vpay uses — five calls and
//! one inbound event — and nothing else.
//!
//! Transcribed from Notch Pay's OpenAPI specification **2.1.0**
//! (`https://developer.notchpay.co/openapi.yaml`, read 2026-10-20). Where the
//! specification and Notch Pay's own guides disagree, the field follows the
//! specification and the disagreement is a "To confirm" item of
//! [RFC-0012](../../../../docs/rfc/0012-notchpay-adapter.md) § 10.
//!
//! # Only the fields vpay reads or sends
//!
//! Notch Pay's payment object carries a customer id, a payment-method id,
//! timestamps, metadata, items, a shipping address. None of it is modelled:
//! serde ignores a field this file does not name, so the payer's data that
//! vpay has no use for is never held in a typed value at all. A field is
//! added here on the day a caller needs it, with the reason.
//!
//! # Every field is renamed explicitly
//!
//! The workspace convention (ADR-0016, standard 3) is
//! `#[serde(rename_all = "snake_case")]` on every type modelling **vpay's**
//! wire. These model **Notch Pay's**, which happens to be snake_case today,
//! and `rename_all` would read as a promise that the spelling is ours to
//! normalise. Each field instead carries `#[serde(rename = "…")]` with Notch
//! Pay's own name, which satisfies the standard without claiming anything:
//! the attribute says exactly what the rail sends, and a reviewer can check
//! it against the specification one line at a time.
//!
//! # Which fields are required, and which are `Option`
//!
//! Each envelope's **payload** — `transaction`, `endpoint`, `endpoints`, an
//! event's `data` — is required. Without it a body is not an answer at all,
//! and `{}` must be `Malformed`, not an empty success: a webhook list that
//! decoded as "no endpoints" would make a caller register a duplicate on
//! every run. The fields **inside** a payload are `Option`, so that a missing
//! one reaches the adapter as `None` and becomes a `Malformed` that names the
//! field. Neither path ever shows serde's own message, which can quote the
//! value it choked on; `interpret` and `parse_event` discard it.
//!
//! # Where Notch Pay's pages disagree with the specification
//!
//! The API reference pages show some answers under a `data` key — the
//! webhook list as `{ "code": 200, "data": [ … ] }`, with an `active` field
//! on each endpoint — where the specification says `endpoints`,
//! `transaction` and `endpoint`, and has no `active`. The API reference also
//! lists a `payment.processing` event that the specification's enum does
//! not. This file follows the specification, so a `data`-shaped answer is
//! `Malformed` rather than silently read; RFC-0012 § 10 lists all three, and
//! the first sandbox session settles them.

use std::fmt;

use serde::{Deserialize, Serialize};
use vpay_core::privacy::Secret;

/// `POST /payments` — "Initialize a payment".
///
/// The specification requires `amount`, `currency`, and one of `email`,
/// `phone` or `customer`. vpay sends `phone`, which is the one payer datum it
/// already holds, and never invents an email or a name. `reference` is the
/// charge's own reference, so the payment can be found again after a crash
/// (`docs/flows/crash-safety.md`). `callback`, the browser redirect, is not
/// sent: a push charge has no browser.
///
/// `amount` is in "the smallest currency unit", and XAF has none below the
/// franc, so it is `Money::to_provider_minor()` unchanged.
#[derive(Serialize)]
pub struct InitializePayment<'a> {
    #[serde(rename = "amount")]
    pub amount: i64,
    #[serde(rename = "currency")]
    pub currency: &'a str,
    #[serde(rename = "phone")]
    pub phone: &'a str,
    #[serde(rename = "reference")]
    pub reference: &'a str,
    #[serde(rename = "description", skip_serializing_if = "Option::is_none")]
    pub description: Option<&'a str>,
}

/// Hides the payer's number; everything else is vpay's own and prints.
impl fmt::Debug for InitializePayment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitializePayment")
            .field("amount", &self.amount)
            .field("currency", &self.currency)
            .field("phone", &Secret::new(self.phone))
            .field("reference", &self.reference)
            .field("description", &self.description)
            .finish()
    }
}

/// `POST /payments/{reference}` — "Process a payment".
///
/// The specification puts the number under `data.phone`, with
/// `data.account_number` as an alternative; one of Notch Pay's guides uses
/// the second. This sends the first. `client_ip` is optional and vpay does
/// not have the payer's.
#[derive(Serialize)]
pub struct ProcessPayment<'a> {
    #[serde(rename = "channel")]
    pub channel: &'a str,
    #[serde(rename = "data")]
    pub data: ProcessData<'a>,
}

/// The channel-specific half of [`ProcessPayment`].
#[derive(Serialize)]
pub struct ProcessData<'a> {
    #[serde(rename = "phone")]
    pub phone: &'a str,
}

impl fmt::Debug for ProcessPayment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessPayment")
            .field("channel", &self.channel)
            .field("data", &self.data)
            .finish()
    }
}

impl fmt::Debug for ProcessData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessData")
            .field("phone", &Secret::new(self.phone))
            .finish()
    }
}

/// The body of a successful initialize, process or retrieve.
///
/// Notch Pay wraps every payment answer in the same envelope: its own
/// `code`, a `status` word, a `message`, and the `transaction`. The
/// envelope's `status` is the HTTP reason phrase ("Accepted"), **not** the
/// payment's status, which is [`Payment::status`]. Mixing the two up is the
/// mistake this comment exists to prevent.
#[derive(Deserialize)]
pub struct PaymentEnvelope {
    #[serde(rename = "code")]
    pub code: Option<u16>,
    #[serde(rename = "message")]
    pub message: Option<String>,
    #[serde(rename = "transaction")]
    pub transaction: Payment,
}

/// Hides Notch Pay's `message`, which is the rail's own prose.
impl fmt::Debug for PaymentEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentEnvelope")
            .field("code", &self.code)
            .field("message", &self.message.as_ref().map(Secret::new))
            .field("transaction", &self.transaction)
            .finish()
    }
}

/// A Notch Pay payment, reduced to what vpay reads.
///
/// `reference` and `trxref` are the field the specification and the
/// direct-charge guide disagree about: the specification's example puts the
/// merchant's reference in `reference`, and the guide's example puts Notch
/// Pay's own (`trx.…`) there and the merchant's in `trxref`. **Notch Pay's
/// sandbox answered with the guide's shape on 2026-10-21**: `reference` was
/// `trx.test_…`, and `trxref` (and an unmodelled `merchant_reference`) the
/// merchant's. It also sent no `id` (RFC-0012 § 10 item 2;
/// `docs/status/verification/2026-10-21-notchpay-sandbox-run.md`).
///
/// `amount` is a JSON number (`type: number`). It is held as
/// [`serde_json::Number`] rather than `i64` so that a body which renders it
/// as `5000.0` still decodes, and [`Payment::amount_minor`] decides whether
/// that is an amount vpay can compare.
#[derive(Deserialize)]
pub struct Payment {
    #[serde(rename = "id")]
    pub id: Option<String>,
    #[serde(rename = "reference")]
    pub reference: Option<String>,
    #[serde(rename = "trxref")]
    pub trxref: Option<String>,
    #[serde(rename = "amount")]
    pub amount: Option<serde_json::Number>,
    #[serde(rename = "currency")]
    pub currency: Option<String>,
    #[serde(rename = "status")]
    pub status: Option<String>,
    /// `amounts`, absent from `openapi.yaml` 2.1.0 and present on every
    /// payment answer Notch Pay's sandbox gave on 2026-10-21
    /// (`docs/status/verification/2026-10-21-notchpay-sandbox-run.md`).
    #[serde(rename = "amounts")]
    pub amounts: Option<Amounts>,
}

/// A payment's `amounts` object: `{ "converted", "currency", "rate",
/// "total" }`. Only `total` is read.
///
/// On 2026-10-21 `total` equalled `amount` on every sandbox answer but one:
/// the read of Notch Pay's "insufficient funds" test number, which said
/// `complete` with `amount` 107 and `total` 105. So `total` is what was
/// collected, and a success whose `total` is not the charge's amount is not
/// a success (`vpay_adapter_notchpay`'s mapping).
#[derive(Debug, Deserialize)]
pub struct Amounts {
    #[serde(rename = "total")]
    pub total: Option<serde_json::Number>,
}

impl Amounts {
    /// `total` as whole minor units, on [`Payment::amount_minor`]'s terms: a
    /// float or a missing value is `None`.
    #[must_use]
    pub fn total_minor(&self) -> Option<i64> {
        self.total.as_ref().and_then(serde_json::Number::as_i64)
    }
}

impl Payment {
    /// The amount as whole minor units, or `None` if Notch Pay sent no
    /// amount or one that is not an integer in JSON.
    ///
    /// `5000.0` is `None`, not `5000`. Deciding that a float is "really" an
    /// integer is float arithmetic in the money path, which the workspace
    /// denies (`docs/flows/money.md`), and a caller that cannot compare the
    /// amount must not report the payment as matching.
    #[must_use]
    pub fn amount_minor(&self) -> Option<i64> {
        self.amount.as_ref().and_then(serde_json::Number::as_i64)
    }
}

/// Hides the three references, as the port's `ChargeRef` hides its rail
/// key material: a reference is what an operator would paste into Notch
/// Pay's dashboard, and a log line is not where it belongs.
impl fmt::Debug for Payment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Payment")
            .field("id", &self.id.as_ref().map(Secret::new))
            .field("reference", &self.reference.as_ref().map(Secret::new))
            .field("trxref", &self.trxref.as_ref().map(Secret::new))
            .field("amount", &self.amount)
            .field("currency", &self.currency)
            .field("status", &self.status)
            .field("amounts_total", &self.amounts.as_ref().map(|a| &a.total))
            .finish()
    }
}

/// A webhook event, as Notch Pay POSTs it to a registered endpoint.
///
/// `{ "id": "evt_…", "type": "payment.complete", "created_at": "…",
/// "data": { … the payment … } }` (Webhooks API, "Webhook Event Object").
/// The event's own `data.status` is deliberately not modelled: a callback is
/// a hint, and only the authenticated read moves money
/// (`docs/flows/reconciler.md`). A type with nowhere to put a status cannot
/// leak one.
#[derive(Deserialize)]
pub struct Event {
    #[serde(rename = "id")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    #[serde(rename = "data")]
    pub data: EventData,
}

/// The part of an event's `data` vpay reads: which payment it is about.
#[derive(Deserialize)]
pub struct EventData {
    #[serde(rename = "id")]
    pub id: Option<String>,
    #[serde(rename = "reference")]
    pub reference: Option<String>,
    #[serde(rename = "trxref")]
    pub trxref: Option<String>,
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Event")
            .field("id", &self.id)
            .field("event_type", &self.event_type)
            .field("data", &self.data)
            .finish()
    }
}

impl fmt::Debug for EventData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventData")
            .field("id", &self.id.as_ref().map(Secret::new))
            .field("reference", &self.reference.as_ref().map(Secret::new))
            .field("trxref", &self.trxref.as_ref().map(Secret::new))
            .finish()
    }
}

/// The five payment events in the specification's `createWebhook` enum, in
/// its order: `payment.created`, `payment.complete`, `payment.failed`,
/// `payment.canceled`, `payment.expired`.
///
/// These are the events a vpay endpoint subscribes to. Transfer and customer
/// events are in the same enum and are not charges, so a vpay endpoint never
/// asks for them. `payment.processing`, which the API reference lists and the
/// specification's enum does not, is not here until the sandbox shows it is
/// accepted.
pub const PAYMENT_EVENTS: [&str; 5] = [
    "payment.created",
    "payment.complete",
    "payment.failed",
    "payment.canceled",
    "payment.expired",
];

/// `POST /webhooks` — "Create a webhook". `url` and `events` are required;
/// `description` is what lets vpay recognise an endpoint it created itself
/// among an account's others.
#[derive(Serialize)]
pub struct CreateWebhook<'a> {
    #[serde(rename = "url")]
    pub url: &'a str,
    #[serde(rename = "description", skip_serializing_if = "Option::is_none")]
    pub description: Option<&'a str>,
    #[serde(rename = "events")]
    pub events: &'a [&'a str],
}

/// Hides the URL, as the port's `ChargeRef` hides its callback URL.
impl fmt::Debug for CreateWebhook<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateWebhook")
            .field("url", &Secret::new(self.url))
            .field("description", &self.description)
            .field("events", &self.events)
            .finish()
    }
}

/// The body of a successful create or retrieve: `{ code, status, message,
/// endpoint }`.
#[derive(Deserialize)]
pub struct WebhookEnvelope {
    #[serde(rename = "endpoint")]
    pub endpoint: Webhook,
}

/// The body of `GET /webhooks`: one page of endpoints.
///
/// The specification paginates with `page` and `limit` (default 30) and
/// reports `current_page` and `last_page`. A caller looking for its own
/// endpoint reads every page; stopping at the first would miss one and
/// register a duplicate.
#[derive(Deserialize)]
pub struct WebhookPage {
    #[serde(rename = "endpoints")]
    pub endpoints: Vec<Webhook>,
    #[serde(rename = "current_page")]
    pub current_page: Option<u32>,
    #[serde(rename = "last_page")]
    pub last_page: Option<u32>,
}

/// A registered webhook endpoint (`Webhooks.Webhook` in the specification).
///
/// **No signing secret is modelled.** The specification's schema has no
/// field for one. The API reference says the secret can be found "in your
/// dashboard or when creating a webhook via the API", which contradicts it,
/// and RFC-0012 § 10 lists the question. A field that the schema does not
/// have is not added on the strength of a sentence.
#[derive(Deserialize)]
pub struct Webhook {
    #[serde(rename = "id")]
    pub id: Option<String>,
    #[serde(rename = "url")]
    pub url: Option<String>,
    #[serde(rename = "description")]
    pub description: Option<String>,
    #[serde(rename = "events")]
    pub events: Option<Vec<String>>,
}

impl fmt::Debug for WebhookEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookEnvelope")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl fmt::Debug for WebhookPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookPage")
            .field("endpoints", &self.endpoints)
            .field("current_page", &self.current_page)
            .field("last_page", &self.last_page)
            .finish()
    }
}

impl fmt::Debug for Webhook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Webhook")
            .field("id", &self.id)
            .field("url", &self.url.as_ref().map(Secret::new))
            .field("description", &self.description)
            .field("events", &self.events)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYER: &str = "+237670000000";

    #[test]
    fn an_initialize_body_is_the_five_fields_and_nothing_else() {
        let body = InitializePayment {
            amount: 5000,
            currency: "XAF",
            phone: PAYER,
            reference: "6d7c0f1e-0000-4000-8000-000000000001",
            description: Some("Payment"),
        };
        assert_eq!(
            serde_json::to_value(&body).expect("serialises"),
            serde_json::json!({
                "amount": 5000,
                "currency": "XAF",
                "phone": PAYER,
                "reference": "6d7c0f1e-0000-4000-8000-000000000001",
                "description": "Payment",
            })
        );
    }

    #[test]
    fn an_absent_description_is_omitted_rather_than_sent_null() {
        let body = InitializePayment {
            amount: 100,
            currency: "XAF",
            phone: PAYER,
            reference: "r",
            description: None,
        };
        let value = serde_json::to_value(&body).expect("serialises");
        assert!(value.get("description").is_none(), "{value}");
    }

    #[test]
    fn a_process_body_nests_the_number_under_data_as_the_specification_does() {
        let body = ProcessPayment {
            channel: "cm.mobile",
            data: ProcessData { phone: PAYER },
        };
        assert_eq!(
            serde_json::to_value(&body).expect("serialises"),
            serde_json::json!({ "channel": "cm.mobile", "data": { "phone": PAYER } })
        );
    }

    #[test]
    fn the_specifications_retrieve_example_decodes() {
        // openapi.yaml 2.1.0, `GET /payments/{reference}`, 202, with the
        // Payment schema's example values.
        let body = br#"{
            "status": "Accepted",
            "message": "Transaction retrieved",
            "code": 202,
            "transaction": {
                "id": "pay_123456789",
                "reference": "ref_123456789",
                "amount": 5000,
                "currency": "XAF",
                "status": "complete",
                "customer": "cus_123456789",
                "created_at": "2023-01-01T12:00:00Z",
                "completed_at": "2023-01-01T12:05:00Z",
                "payment_method": "pm.ndzAfIh555VCPML1"
            }
        }"#;
        let envelope: PaymentEnvelope = serde_json::from_slice(body).expect("decodes");
        assert_eq!(envelope.code, Some(202));
        let payment = envelope.transaction;
        assert_eq!(payment.id.as_deref(), Some("pay_123456789"));
        assert_eq!(payment.status.as_deref(), Some("complete"));
        assert_eq!(payment.amount_minor(), Some(5000));
        assert_eq!(payment.currency.as_deref(), Some("XAF"));
        assert_eq!(payment.trxref, None);
    }

    #[test]
    fn the_guides_two_reference_shape_decodes_both_references() {
        // The direct-charge guide's example: Notch Pay's reference in
        // `reference`, the merchant's in `trxref`.
        let body = br#"{"transaction":{"reference":"trx.123456789abcdefghj","trxref":"order_123","status":"processing"}}"#;
        let envelope: PaymentEnvelope = serde_json::from_slice(body).expect("decodes");
        let payment = envelope.transaction;
        assert_eq!(payment.reference.as_deref(), Some("trx.123456789abcdefghj"));
        assert_eq!(payment.trxref.as_deref(), Some("order_123"));
        assert_eq!(payment.amount_minor(), None, "no amount was sent");
    }

    #[test]
    fn an_amount_rendered_as_a_float_is_not_an_amount_vpay_can_compare() {
        let payment: Payment = serde_json::from_slice(br#"{"amount": 5000.0}"#).expect("decodes");
        assert_eq!(payment.amount_minor(), None);
    }

    #[test]
    fn the_webhook_event_example_decodes_and_carries_no_status() {
        // Webhooks API, "Webhook Event Object".
        let body = br#"{
            "id": "evt_123456789",
            "type": "payment.complete",
            "created_at": "2023-01-01T12:00:00Z",
            "data": {
                "id": "pay_123456789",
                "reference": "order_123",
                "amount": 5000,
                "currency": "XAF",
                "status": "complete"
            }
        }"#;
        let event: Event = serde_json::from_slice(body).expect("decodes");
        assert_eq!(event.event_type.as_deref(), Some("payment.complete"));
        let data = event.data;
        assert_eq!(data.reference.as_deref(), Some("order_123"));
        assert_eq!(data.id.as_deref(), Some("pay_123456789"));
    }

    #[test]
    fn no_debug_output_prints_a_number_a_reference_or_the_rails_words() {
        let initialize = InitializePayment {
            amount: 5000,
            currency: "XAF",
            phone: PAYER,
            reference: "r",
            description: None,
        };
        let process = ProcessPayment {
            channel: "cm.mobile",
            data: ProcessData { phone: PAYER },
        };
        let envelope: PaymentEnvelope = serde_json::from_slice(
            br#"{"message":"rail prose naming +237670000000","transaction":{"id":"pay_secret01","reference":"trx.secret02","trxref":"secret03","status":"failed"}}"#,
        )
        .expect("decodes");
        let event: Event = serde_json::from_slice(
            br#"{"id":"evt_1","type":"payment.failed","data":{"id":"pay_secret01","reference":"trx.secret02","trxref":"secret03"}}"#,
        )
        .expect("decodes");

        let printed = format!("{initialize:?} {process:?} {envelope:?} {event:?}");
        for leaked in [
            PAYER,
            "rail prose",
            "pay_secret01",
            "trx.secret02",
            "secret03",
        ] {
            assert!(!printed.contains(leaked), "{leaked} printed: {printed}");
        }
        // What stays visible is what an operator needs and is not the payer's.
        for visible in ["5000", "cm.mobile", "failed", "payment.failed"] {
            assert!(printed.contains(visible), "{visible} hidden: {printed}");
        }
    }

    #[test]
    fn a_create_webhook_body_is_the_specifications_three_fields() {
        let body = CreateWebhook {
            url: "https://pay.example.cm/provider/notchpay/callback",
            description: Some("vpay"),
            events: &PAYMENT_EVENTS,
        };
        assert_eq!(
            serde_json::to_value(&body).expect("serialises"),
            serde_json::json!({
                "url": "https://pay.example.cm/provider/notchpay/callback",
                "description": "vpay",
                "events": ["payment.created", "payment.complete", "payment.failed",
                           "payment.canceled", "payment.expired"],
            })
        );
    }

    #[test]
    fn the_specifications_webhook_list_decodes_with_its_pagination() {
        // openapi.yaml 2.1.0, `GET /webhooks`, 200, with the Webhook
        // schema's example values.
        let body = br#"{"code":200,"status":"OK","message":"Webhook endpoints retrieved",
            "totals":1,"last_page":1,"current_page":1,"selected":1,
            "endpoints":[{"id":"wh_123456789","url":"https://example.com/webhooks",
            "description":"Webhook principal","events":["payment.complete"],
            "created_at":"2023-01-01T12:00:00Z","updated_at":"2023-01-02T12:00:00Z"}]}"#;
        let page: WebhookPage = serde_json::from_slice(body).expect("decodes");
        assert_eq!(page.current_page, Some(1));
        assert_eq!(page.last_page, Some(1));
        let endpoints = page.endpoints;
        let [endpoint] = endpoints.as_slice() else {
            panic!("one endpoint: {endpoints:?}");
        };
        assert_eq!(endpoint.id.as_deref(), Some("wh_123456789"));
        assert_eq!(endpoint.description.as_deref(), Some("Webhook principal"));
    }

    #[test]
    fn no_webhook_debug_output_prints_the_url() {
        let url = "https://pay.example.cm/provider/notchpay/callback";
        let create = CreateWebhook {
            url,
            description: Some("vpay"),
            events: &PAYMENT_EVENTS,
        };
        let page: WebhookPage = serde_json::from_slice(
            br#"{"endpoints":[{"id":"wh_1","url":"https://pay.example.cm/provider/notchpay/callback"}]}"#,
        )
        .expect("decodes");
        let printed = format!("{create:?} {page:?}");
        assert!(!printed.contains(url), "{printed}");
        assert!(
            printed.contains("wh_1") && printed.contains("payment.complete"),
            "{printed}"
        );
    }
}
