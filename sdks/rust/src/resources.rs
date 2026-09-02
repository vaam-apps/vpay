//! The `/v1` resources — `docs/flows/merchant-auth.md`'s "Resources" table —
//! and the request-parameter types each method encodes onto the wire.
//!
//! Every params type's `to_form` builds a [`crate::form::FormValue`] with
//! its fields in the exact order the docs' wire examples pin (e.g.
//! `amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&
//! metadata[order_id]=1234`); `tests/resources.rs` asserts the encoded byte
//! string directly, so a reordering here is a test failure, not a silent
//! drift.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;

use crate::client::Client;
use crate::form::FormValue;
use crate::model::{Balance, Event, List, PaymentIntent, PaymentMethodType, Refund};
use crate::validate::check_amount;

/// Per-call options every write (`POST`) method accepts.
///
/// Carries only the idempotency key today — see `docs/flows/merchant-auth.md`
/// §"Headers": every `POST` sends one, caller-supplied if given here,
/// otherwise a fresh UUIDv4 generated per call "so a network retry can never
/// double-create".
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    /// The `Idempotency-Key` header to send.
    ///
    /// Supply one derived from the merchant's own order id to make a retry of
    /// the *same* logical operation safe across process restarts; leave it
    /// `None` and each call gets a fresh UUIDv4, which protects against a
    /// network-level retry of one call but not against the caller running the
    /// operation twice.
    pub idempotency_key: Option<String>,
}

impl RequestOptions {
    /// Default options: the SDK generates a UUIDv4 idempotency key per call.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins the `Idempotency-Key` for this call — see
    /// [`RequestOptions::idempotency_key`] for when that matters.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

/// Percent-encodes one path segment before it is interpolated into a URL.
///
/// An id is merchant-controlled input: an unescaped `/` would move the
/// request to a different route, and a `?` or `#` would truncate the path and
/// turn the rest into a query or fragment. `sdks/nodejs/src/resources/
/// payment-intents.ts` wraps every id in `encodeURIComponent` for the same
/// reason, and `crate::form::percent_encode` is that function's exact rule
/// (see `crate::form`'s module doc), so the two SDKs request the same URL for
/// the same id.
fn path_segment(id: &str) -> String {
    crate::form::percent_encode(id)
}

fn metadata_form(metadata: &BTreeMap<String, String>) -> FormValue {
    if metadata.is_empty() {
        return FormValue::Skip;
    }
    FormValue::Object(
        metadata
            .iter()
            .map(|(k, v)| (k.clone(), FormValue::from(v.as_str())))
            .collect(),
    )
}

/// `POST /v1/payment_intents` request fields.
#[derive(Debug, Clone, Default)]
pub struct CreatePaymentIntentParams {
    /// Minor units — `5000` on a `xaf` intent is 5,000 FCFA, because XAF is
    /// zero-decimal (`docs/flows/money.md`).
    ///
    /// The type being `i64` already rules out a fractional amount. It does
    /// not rule out a negative one, or one past `2^53-1` — which the Node
    /// SDK refuses outright (`sdks/nodejs/src/validate.ts`), so this one does
    /// too: [`PaymentIntentsResource::create`] returns
    /// [`crate::Error::InvalidParams`] before building a request. Two SDKs
    /// disagreeing about which amounts are sendable is a parity defect in the
    /// money path.
    pub amount: i64,
    /// Lower-cased at encode time regardless of how it was supplied — the
    /// wire contract requires lowercase (`docs/flows/merchant-auth.md`'s
    /// encoding table), and silently normalizing it here means a currency
    /// constant that happens to be upper-cased elsewhere in a caller's code
    /// does not turn into a server-side rejection.
    pub currency: String,
    /// The rails this intent may be confirmed against, in the order they are
    /// sent (`payment_method_types[0]`, `[1]`, …).
    ///
    /// A closed [`PaymentMethodType`], matching the Node SDK's request type;
    /// the *response* field of the same name stays a `Vec<String>` so an
    /// unknown rail still decodes. See [`PaymentMethodType`].
    pub payment_method_types: Vec<PaymentMethodType>,
    /// Merchant-owned key/value pairs, echoed back on the object and on every
    /// event about it. Encoded as `metadata[key]=value`; a key containing a
    /// bracket is escaped, never treated as nesting (see `crate::form`).
    pub metadata: BTreeMap<String, String>,
    /// Free text shown to the merchant, never to the payer. Omitted from the
    /// body entirely when `None` — `description=` and no `description` are
    /// different requests.
    pub description: Option<String>,
}

impl CreatePaymentIntentParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("amount".to_string(), FormValue::from(self.amount)),
            (
                "currency".to_string(),
                FormValue::from(self.currency.to_lowercase()),
            ),
            (
                "payment_method_types".to_string(),
                FormValue::Array(
                    self.payment_method_types
                        .iter()
                        .map(|t| FormValue::from(t.as_wire_str()))
                        .collect(),
                ),
            ),
            ("metadata".to_string(), metadata_form(&self.metadata)),
            (
                "description".to_string(),
                FormValue::from(self.description.clone()),
            ),
        ])
    }
}

/// `POST /v1/payment_intents/{id}/confirm` request fields — one variant per
/// rail, because the rails do not take the same fields.
///
/// An enum, not a struct of options, and deliberately the same shape as the
/// Node SDK's discriminated union (`ConfirmPaymentIntentParams` in
/// `sdks/nodejs/src/types.ts`): a push rail needs an `msisdn` and has no
/// `return_url`, a redirect rail needs a `return_url` and has no instrument.
/// Expressed as one struct with two `Option`s, three of the four combinations
/// are wrong and only the server can say so — which costs a round trip and,
/// for `MtnMomo` with no `msisdn`, produces a confirm that can never prompt
/// anyone.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfirmPaymentIntentParams {
    /// Push rail: the payer approves on their handset, so the number to
    /// prompt is the whole instrument and there is nowhere to redirect to.
    MtnMomo {
        /// The payer's MSISDN, in the rail's own format (e.g.
        /// `237670000000`). Sent as
        /// `payment_method_data[mtn_momo][msisdn]`.
        msisdn: String,
    },
    /// Redirect rail: there is no instrument to submit, only where to send
    /// the payer back to afterwards.
    OrangeMoney {
        /// Where the rail returns the payer after they approve or abandon.
        /// Sent as a top-level `return_url`, beside
        /// `payment_method_data[type]`.
        return_url: String,
    },
}

impl ConfirmPaymentIntentParams {
    /// Confirms against MTN Mobile Money, prompting `msisdn`.
    #[must_use]
    pub fn mtn_momo(msisdn: impl Into<String>) -> Self {
        ConfirmPaymentIntentParams::MtnMomo {
            msisdn: msisdn.into(),
        }
    }

    /// Confirms against Orange Money, returning the payer to `return_url`.
    #[must_use]
    pub fn orange_money(return_url: impl Into<String>) -> Self {
        ConfirmPaymentIntentParams::OrangeMoney {
            return_url: return_url.into(),
        }
    }

    /// The rail this confirm is for.
    #[must_use]
    pub fn payment_method_type(&self) -> PaymentMethodType {
        match self {
            ConfirmPaymentIntentParams::MtnMomo { .. } => PaymentMethodType::MtnMomo,
            ConfirmPaymentIntentParams::OrangeMoney { .. } => PaymentMethodType::OrangeMoney,
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        let type_field = (
            "type".to_string(),
            FormValue::from(self.payment_method_type().as_wire_str()),
        );
        match self {
            ConfirmPaymentIntentParams::MtnMomo { msisdn } => FormValue::Object(vec![(
                "payment_method_data".to_string(),
                FormValue::Object(vec![
                    type_field,
                    (
                        "mtn_momo".to_string(),
                        FormValue::Object(vec![(
                            "msisdn".to_string(),
                            FormValue::from(msisdn.as_str()),
                        )]),
                    ),
                ]),
            )]),
            ConfirmPaymentIntentParams::OrangeMoney { return_url } => FormValue::Object(vec![
                (
                    "payment_method_data".to_string(),
                    FormValue::Object(vec![type_field]),
                ),
                (
                    "return_url".to_string(),
                    FormValue::from(return_url.as_str()),
                ),
            ]),
        }
    }
}

/// `GET /v1/payment_intents` query parameters. All optional; an unset field
/// is omitted from the query string entirely.
#[derive(Debug, Clone, Default)]
pub struct ListPaymentIntentsParams {
    /// Page size. The server's own default and ceiling apply when unset.
    pub limit: Option<u32>,
    /// Cursor: return objects *after* this id (the next page).
    pub starting_after: Option<String>,
    /// Cursor: return objects *before* this id (the previous page).
    pub ending_before: Option<String>,
}

impl ListPaymentIntentsParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("limit".to_string(), FormValue::from(self.limit)),
            (
                "starting_after".to_string(),
                FormValue::from(self.starting_after.clone()),
            ),
            (
                "ending_before".to_string(),
                FormValue::from(self.ending_before.clone()),
            ),
        ])
    }
}

/// `POST /v1/refunds` request fields.
#[derive(Debug, Clone, Default)]
pub struct CreateRefundParams {
    /// The `pi_…` to refund.
    pub payment_intent: String,
    /// Minor units. **Omit for a full refund** — `amount=` and no `amount`
    /// are different requests, and only the second means "all of it".
    ///
    /// When present, held to the same bound as
    /// [`CreatePaymentIntentParams::amount`]: non-negative and at most
    /// `2^53-1`, or [`RefundsResource::create`] returns
    /// [`crate::Error::InvalidParams`] without sending anything.
    pub amount: Option<i64>,
    /// Merchant-supplied reason, echoed back on the refund object.
    pub reason: Option<String>,
    /// Merchant-owned key/value pairs, encoded as `metadata[key]=value`.
    pub metadata: BTreeMap<String, String>,
}

impl CreateRefundParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "payment_intent".to_string(),
                FormValue::from(self.payment_intent.as_str()),
            ),
            ("amount".to_string(), FormValue::from(self.amount)),
            ("reason".to_string(), FormValue::from(self.reason.clone())),
            ("metadata".to_string(), metadata_form(&self.metadata)),
        ])
    }
}

/// `GET /v1/events` query parameters. All optional; an unset field is
/// omitted from the query string entirely.
#[derive(Debug, Clone, Default)]
pub struct ListEventsParams {
    /// Page size. The server's own default and ceiling apply when unset.
    pub limit: Option<u32>,
    /// Cursor: return events *after* this id (the next page).
    pub starting_after: Option<String>,
    /// Cursor: return events *before* this id (the previous page).
    pub ending_before: Option<String>,
    /// Filters by event type, e.g. `payment_intent.succeeded`.
    ///
    /// **Sent as `type=…`**, not `event_type=…`: `type` is a Rust keyword, so
    /// the field is named `event_type` here and
    /// [`ListEventsParams::to_form`](ListEventsParams) writes the wire name.
    /// A `String` rather than a closed enum, for the same reason
    /// [`crate::Event::kind`] is one.
    pub event_type: Option<String>,
}

impl ListEventsParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("limit".to_string(), FormValue::from(self.limit)),
            (
                "starting_after".to_string(),
                FormValue::from(self.starting_after.clone()),
            ),
            (
                "ending_before".to_string(),
                FormValue::from(self.ending_before.clone()),
            ),
            ("type".to_string(), FormValue::from(self.event_type.clone())),
        ])
    }
}

fn query_string(form: &FormValue) -> Option<String> {
    let encoded = crate::form::encode_form(form);
    if encoded.is_empty() {
        None
    } else {
        Some(encoded)
    }
}

async fn get<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    query: Option<String>,
) -> Result<T, crate::Error> {
    client.get(path, query).await
}

async fn post<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    body: FormValue,
    opts: RequestOptions,
) -> Result<T, crate::Error> {
    client
        .post(path, crate::form::encode_form(&body), opts)
        .await
}

/// `client.payment_intents()` — see [`crate::Client::payment_intents`].
#[derive(Debug, Clone, Copy)]
pub struct PaymentIntentsResource<'a> {
    pub(crate) client: &'a Client,
}

impl PaymentIntentsResource<'_> {
    /// `POST /v1/payment_intents`.
    ///
    /// # Errors
    /// [`crate::Error::InvalidParams`] if `amount` is negative or beyond
    /// `2^53-1`, before any request is sent; otherwise see
    /// [`enum@crate::Error`].
    pub async fn create(
        &self,
        params: CreatePaymentIntentParams,
        opts: RequestOptions,
    ) -> Result<PaymentIntent, crate::Error> {
        // Before anything is built, so a refused amount spends neither an
        // assertion `jti` nor an idempotency key.
        check_amount(params.amount, "amount")?;
        post(self.client, "/payment_intents", params.to_form(), opts).await
    }

    /// `GET /v1/payment_intents/{id}`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(&self, id: &str) -> Result<PaymentIntent, crate::Error> {
        get(
            self.client,
            &format!("/payment_intents/{}", path_segment(id)),
            None,
        )
        .await
    }

    /// `POST /v1/payment_intents/{id}/confirm`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn confirm(
        &self,
        id: &str,
        params: ConfirmPaymentIntentParams,
        opts: RequestOptions,
    ) -> Result<PaymentIntent, crate::Error> {
        post(
            self.client,
            &format!("/payment_intents/{}/confirm", path_segment(id)),
            params.to_form(),
            opts,
        )
        .await
    }

    /// `POST /v1/payment_intents/{id}/cancel`. No request fields.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn cancel(
        &self,
        id: &str,
        opts: RequestOptions,
    ) -> Result<PaymentIntent, crate::Error> {
        post(
            self.client,
            &format!("/payment_intents/{}/cancel", path_segment(id)),
            FormValue::Object(Vec::new()),
            opts,
        )
        .await
    }

    /// `GET /v1/payment_intents`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn list(
        &self,
        params: ListPaymentIntentsParams,
    ) -> Result<List<PaymentIntent>, crate::Error> {
        get(
            self.client,
            "/payment_intents",
            query_string(&params.to_form()),
        )
        .await
    }
}

/// `client.refunds()` — see [`crate::Client::refunds`].
#[derive(Debug, Clone, Copy)]
pub struct RefundsResource<'a> {
    pub(crate) client: &'a Client,
}

impl RefundsResource<'_> {
    /// `POST /v1/refunds`.
    ///
    /// # Errors
    /// [`crate::Error::InvalidParams`] if `amount` is present and negative or
    /// beyond `2^53-1`, before any request is sent; otherwise see
    /// [`enum@crate::Error`].
    pub async fn create(
        &self,
        params: CreateRefundParams,
        opts: RequestOptions,
    ) -> Result<Refund, crate::Error> {
        if let Some(amount) = params.amount {
            check_amount(amount, "amount")?;
        }
        post(self.client, "/refunds", params.to_form(), opts).await
    }
}

/// `client.events()` — see [`crate::Client::events`].
#[derive(Debug, Clone, Copy)]
pub struct EventsResource<'a> {
    pub(crate) client: &'a Client,
}

impl EventsResource<'_> {
    /// `GET /v1/events`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn list(&self, params: ListEventsParams) -> Result<List<Event>, crate::Error> {
        get(self.client, "/events", query_string(&params.to_form())).await
    }
}

/// `client.balance()` — see [`crate::Client::balance`].
#[derive(Debug, Clone, Copy)]
pub struct BalanceResource<'a> {
    pub(crate) client: &'a Client,
}

impl BalanceResource<'_> {
    /// `GET /v1/balance`. No request fields.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(&self) -> Result<Balance, crate::Error> {
        get(self.client, "/balance", None).await
    }
}
