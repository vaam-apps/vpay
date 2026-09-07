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
use crate::model::{
    AccountHolder, Balance, CheckoutSession, CheckoutUiMode, Customer, DeletedCustomer, Event,
    List, PaymentIntent, PaymentMethodType, Refund,
};
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
    /// The `cus_…` this intent is for (S4a).
    ///
    /// Accepted and **dropped** by any vpay predating 2026-09-06, which is
    /// worth knowing before relying on it: an older server answers `200` with
    /// `customer: null` rather than refusing. Check the response.
    ///
    /// A `cus_…` that is not this account's is a `400` naming `customer` —
    /// never a `404`, so the field cannot be used to discover which customers
    /// exist under some other account.
    pub customer: Option<String>,
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
            (
                "customer".to_string(),
                FormValue::from(self.customer.clone()),
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

/// `POST /v1/checkout/sessions` request fields (Step 9's D1).
///
/// The URL rules are the server's and are deliberately not duplicated here:
/// `success_url`/`cancel_url` are required for `hosted` and refused for
/// `embedded`, `return_url` the other way round, all http(s) and at most
/// 2048 characters, `https` only under livemode. This SDK sends what it is
/// given and lets the server say no. A second copy of those rules here
/// would be a second thing to keep in step with them, and would refuse a
/// combination a later server version allows — the Node SDK
/// (`sdks/nodejs/src/resources/checkout-sessions.ts`) takes the same line,
/// which is what keeps the two at parity on the refusals as well as the
/// acceptances.
#[derive(Debug, Clone, Default)]
pub struct CreateCheckoutSessionParams {
    /// The `pi_…` to drive. Required; a session never creates its intent.
    pub payment_intent: String,
    /// Omitted from the body entirely when `None`, which is how the server
    /// gets to apply its own default (`hosted`) rather than this SDK
    /// guessing it.
    pub ui_mode: Option<CheckoutUiMode>,
    /// Where a paying payer is forwarded. Hosted mode. May contain the
    /// literal `{CHECKOUT_SESSION_ID}` (D5).
    pub success_url: Option<String>,
    /// Where a payer who gave up is forwarded. Hosted mode.
    pub cancel_url: Option<String>,
    /// Where vpay's framed page forwards the payer at the end. Embedded mode.
    pub return_url: Option<String>,
}

impl CreateCheckoutSessionParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "payment_intent".to_string(),
                FormValue::from(self.payment_intent.as_str()),
            ),
            (
                "ui_mode".to_string(),
                FormValue::from(self.ui_mode.map(CheckoutUiMode::as_wire_str)),
            ),
            (
                "success_url".to_string(),
                FormValue::from(self.success_url.clone()),
            ),
            (
                "cancel_url".to_string(),
                FormValue::from(self.cancel_url.clone()),
            ),
            (
                "return_url".to_string(),
                FormValue::from(self.return_url.clone()),
            ),
        ])
    }
}

/// `GET /v1/checkout/sessions` query parameters. All optional; an unset
/// field is omitted from the query string entirely.
#[derive(Debug, Clone, Default)]
pub struct ListCheckoutSessionsParams {
    /// Page size. The server's own default and ceiling apply when unset.
    pub limit: Option<u32>,
    /// Cursor: return sessions *after* this id (the next page).
    pub starting_after: Option<String>,
    /// Cursor: return sessions *before* this id (the previous page).
    pub ending_before: Option<String>,
    /// Only sessions for this `pi_…`.
    pub payment_intent: Option<String>,
}

impl ListCheckoutSessionsParams {
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
            (
                "payment_intent".to_string(),
                FormValue::from(self.payment_intent.clone()),
            ),
        ])
    }
}

/// `POST /v1/customers` request fields (S4a).
///
/// **At least one of `name`, `email` and `phone` must be present**, and this
/// SDK deliberately does not check that: which identifiers a customer needs
/// is a *product* rule vpay owns and may widen, exactly as the MSISDN rule
/// [`RetrieveAccountHolderParams::msisdn`] describes is, and a copy here
/// would refuse offline a customer a later server version accepts. The
/// server answers `400` naming the parameter, which is the same information
/// one round trip later. `sdks/nodejs` takes the identical line.
///
/// A phone number **alone** is a complete customer — the whole point of the
/// object on a mobile money rail — so an all-`None` struct is the one shape
/// this SDK will happily send and the server will refuse.
#[derive(Debug, Clone, Default)]
pub struct CreateCustomerParams {
    /// The payer's name.
    pub name: Option<String>,
    /// The payer's email.
    pub email: Option<String>,
    /// The payer's phone number, in any spelling vpay's canonicaliser
    /// accepts (`+237 6 …`, `237600000200`, `600000200`). It is stored and
    /// echoed back **canonical**, so what comes back may not be what was
    /// sent — see [`crate::Customer::phone`].
    pub phone: Option<String>,
    /// Merchant-owned key/value pairs, encoded as `metadata[key]=value`.
    pub metadata: BTreeMap<String, String>,
}

impl CreateCustomerParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("name".to_string(), FormValue::from(self.name.clone())),
            ("email".to_string(), FormValue::from(self.email.clone())),
            ("phone".to_string(), FormValue::from(self.phone.clone())),
            ("metadata".to_string(), metadata_form(&self.metadata)),
        ])
    }
}

/// `POST /v1/customers/{id}` request fields (S4a).
///
/// # Why every field is an `Option<Option<String>>`
///
/// An update has three answers per field and a merchant depends on all
/// three: *leave it alone* (`None`), *set it* (`Some(Some(v))`), and
/// **clear it** (`Some(None)`, which this SDK sends as `name=`). A plain
/// `Option<String>` collapses the first and the third, and a payer's email
/// would then be unclearable through the API that documents how to clear it.
///
/// It is the same three-state shape `vpay_db::CustomerPatch` carries on the
/// server and the same one TypeScript spells `string | null | undefined`, so
/// all three layers express the distinction rather than two of them
/// preserving it and one losing it.
///
/// `metadata` has two states rather than three, and that is the wire
/// contract: it is **merged** key-wise by the server, and a key sent empty is
/// removed. So "leave metadata alone" is an empty map, and there is no
/// separate "clear all metadata" — send each key empty.
#[derive(Debug, Clone, Default)]
pub struct UpdateCustomerParams {
    /// `None` leaves the name alone; `Some(None)` clears it.
    pub name: Option<Option<String>>,
    /// See [`Self::name`].
    pub email: Option<Option<String>>,
    /// See [`Self::name`].
    pub phone: Option<Option<String>>,
    /// Keys to merge. A key whose value is the empty string is **removed**
    /// from the stored metadata, which is Stripe's own per-key delete.
    pub metadata: BTreeMap<String, String>,
}

impl UpdateCustomerParams {
    /// `Some(None)` becomes the empty string, which is what "clear this" is
    /// on a form-encoded wire; `None` is omitted from the body entirely.
    fn patch(field: Option<&Option<String>>) -> FormValue {
        match field {
            None => FormValue::Skip,
            Some(None) => FormValue::from(""),
            Some(Some(value)) => FormValue::from(value.as_str()),
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("name".to_string(), Self::patch(self.name.as_ref())),
            ("email".to_string(), Self::patch(self.email.as_ref())),
            ("phone".to_string(), Self::patch(self.phone.as_ref())),
            ("metadata".to_string(), metadata_form(&self.metadata)),
        ])
    }
}

/// `GET /v1/customers` query parameters. All optional; an unset field is
/// omitted from the query string entirely.
///
/// There is no `email` filter, and that is the server's shape rather than an
/// omission here: a filter on a payer identifier turns the list into a
/// lookup. See `docs/flows/customers.md`.
#[derive(Debug, Clone, Default)]
pub struct ListCustomersParams {
    /// Page size. The server's own default and ceiling apply when unset.
    pub limit: Option<u32>,
    /// Cursor: return customers *after* this id (the next page).
    pub starting_after: Option<String>,
    /// Cursor: return customers *before* this id (the previous page).
    pub ending_before: Option<String>,
}

impl ListCustomersParams {
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

/// `GET /v1/account_holders` request fields (issue #47).
///
/// Both are required by the server, and both are plain owned values here
/// rather than `Option`s — unlike every list-params type in this file —
/// because there is no page to fetch without them: a lookup with no number
/// or no rail is not a narrower query, it is not a query. A caller who has
/// neither has nothing to ask.
#[derive(Debug, Clone)]
pub struct RetrieveAccountHolderParams {
    /// The mobile-money number, in any of the three shapes vpay accepts:
    /// `+2376XXXXXXXX`, `2376XXXXXXXX` or the national `6XXXXXXXX`.
    ///
    /// Not validated here, deliberately, and this is the one place this SDK
    /// departs from the shape its neighbours take with `amount`
    /// ([`crate::validate::check_amount`], which refuses locally). An
    /// `amount` rule is arithmetic and cannot change; a phone-number rule is
    /// a *market* rule that vpay owns and may widen — an SDK copy of it
    /// would refuse a number a later server version accepts, offline, with
    /// no way for the merchant to override it. The server answers `400`
    /// naming `msisdn`, which is the same information one round trip later.
    pub msisdn: String,
    /// Which rail to ask. Closed on the request side, exactly as
    /// `payment_method_types` is on a create: a rail this SDK version does
    /// not know is a rail it cannot spell.
    ///
    /// A rail with no account-holder API answers `400` naming
    /// `payment_method_type`; that is a property of the *deployment*, not of
    /// this enum, so it is not modelled here.
    pub payment_method_type: PaymentMethodType,
}

impl RetrieveAccountHolderParams {
    /// The two fields, for a caller who would otherwise write a struct
    /// literal with a `.to_owned()` in it.
    #[must_use]
    pub fn new(msisdn: impl Into<String>, payment_method_type: PaymentMethodType) -> Self {
        Self {
            msisdn: msisdn.into(),
            payment_method_type,
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("msisdn".to_string(), FormValue::from(self.msisdn.as_str())),
            (
                "payment_method_type".to_string(),
                FormValue::from(self.payment_method_type.as_wire_str()),
            ),
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

/// `client.checkout().sessions()` — see [`crate::Client::checkout`].
///
/// A namespace with no operations of its own, so that the call reads
/// `client.checkout().sessions().create(…)` — the path the wire contract
/// uses (`/v1/checkout/sessions`), and the shape the Node SDK's
/// `client.checkout.sessions` mirrors. A flat `client.checkout_sessions()`
/// would have been shorter and would have made the two SDKs read
/// differently for the same route.
#[derive(Debug, Clone, Copy)]
pub struct CheckoutResource<'a> {
    pub(crate) client: &'a Client,
}

impl<'a> CheckoutResource<'a> {
    /// `/v1/checkout/sessions`.
    #[must_use]
    pub fn sessions(&self) -> CheckoutSessionsResource<'a> {
        CheckoutSessionsResource {
            client: self.client,
        }
    }
}

/// `client.checkout().sessions()` — the four merchant operations on
/// `/v1/checkout/sessions`.
#[derive(Debug, Clone, Copy)]
pub struct CheckoutSessionsResource<'a> {
    pub(crate) client: &'a Client,
}

impl CheckoutSessionsResource<'_> {
    /// `POST /v1/checkout/sessions`. Answers the session **with**
    /// `client_secret`, and with `url` when `ui_mode` is `hosted`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn create(
        &self,
        params: CreateCheckoutSessionParams,
        opts: RequestOptions,
    ) -> Result<CheckoutSession, crate::Error> {
        post(self.client, "/checkout/sessions", params.to_form(), opts).await
    }

    /// `GET /v1/checkout/sessions/{id}`. Answers the session with
    /// `client_secret`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(&self, id: &str) -> Result<CheckoutSession, crate::Error> {
        get(
            self.client,
            &format!("/checkout/sessions/{}", path_segment(id)),
            None,
        )
        .await
    }

    /// `GET /v1/checkout/sessions`. List items never carry `client_secret`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn list(
        &self,
        params: ListCheckoutSessionsParams,
    ) -> Result<List<CheckoutSession>, crate::Error> {
        get(
            self.client,
            "/checkout/sessions",
            query_string(&params.to_form()),
        )
        .await
    }

    /// `POST /v1/checkout/sessions/{id}/expire`. No request fields.
    ///
    /// `open` → `expired`; a session whose intent already has a live charge
    /// is refused `409` by the server rather than raced here.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn expire(
        &self,
        id: &str,
        opts: RequestOptions,
    ) -> Result<CheckoutSession, crate::Error> {
        post(
            self.client,
            &format!("/checkout/sessions/{}/expire", path_segment(id)),
            FormValue::Object(Vec::new()),
            opts,
        )
        .await
    }
}

/// `client.customers()` — the five merchant operations on `/v1/customers`
/// (S4a).
///
/// `del` and not `delete`: `delete` is not a Rust keyword, but
/// `sdks/nodejs`' method is `client.customers.del(…)` — named around
/// `delete` being reserved in older JavaScript — and that is Stripe's own
/// spelling in both of its SDKs. Matching it is what keeps a merchant who
/// read one SDK's docs able to use the other (ADR-0015's parity is per
/// capability, and the *name* is what a reader looks up).
#[derive(Debug, Clone, Copy)]
pub struct CustomersResource<'a> {
    pub(crate) client: &'a Client,
}

impl CustomersResource<'_> {
    /// `POST /v1/customers`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. In particular a `400` naming `name` when
    /// none of `name`, `email` and `phone` was sent — this SDK does not
    /// check that locally; see [`CreateCustomerParams`].
    pub async fn create(
        &self,
        params: CreateCustomerParams,
        opts: RequestOptions,
    ) -> Result<Customer, crate::Error> {
        post(self.client, "/customers", params.to_form(), opts).await
    }

    /// `GET /v1/customers/{id}`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(&self, id: &str) -> Result<Customer, crate::Error> {
        get(
            self.client,
            &format!("/customers/{}", path_segment(id)),
            None,
        )
        .await
    }

    /// `POST /v1/customers/{id}` — the update.
    ///
    /// A `POST` and not a `PUT`/`PATCH`, because that is what the API is:
    /// Stripe has neither verb, and a merchant's existing client sends a
    /// `POST`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. A patch that would clear the customer's
    /// **last** identifier is a `400` naming `name`.
    pub async fn update(
        &self,
        id: &str,
        params: UpdateCustomerParams,
        opts: RequestOptions,
    ) -> Result<Customer, crate::Error> {
        post(
            self.client,
            &format!("/customers/{}", path_segment(id)),
            params.to_form(),
            opts,
        )
        .await
    }

    /// `GET /v1/customers`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn list(&self, params: ListCustomersParams) -> Result<List<Customer>, crate::Error> {
        get(self.client, "/customers", query_string(&params.to_form())).await
    }

    /// `DELETE /v1/customers/{id}` — a **hard** delete.
    ///
    /// The row is removed, not flagged: a subsequent [`Self::retrieve`]
    /// answers the same 404 as an id that never existed, and the only record
    /// of what was deleted is the `customer.deleted` event
    /// ([`crate::KnownEventType::CustomerDeleted`]).
    ///
    /// A customer any PaymentIntent or Checkout Session references **cannot**
    /// be deleted and answers `409`: vpay keeps a payment attached to the
    /// payer it was taken from. Clear `name`, `email` and `phone` with
    /// [`Self::update`] instead if the payer's details have to go.
    ///
    /// Carries an `Idempotency-Key` like every other write on this API, which
    /// is what makes a retried `DELETE` answer the original `{deleted: true}`
    /// rather than a `404`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn del(
        &self,
        id: &str,
        opts: RequestOptions,
    ) -> Result<DeletedCustomer, crate::Error> {
        self.client
            .delete(&format!("/customers/{}", path_segment(id)), opts)
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

    /// `GET /v1/refunds/{id}`.
    ///
    /// A refund is asynchronous on this rail — [`crate::RefundStatus`] has a
    /// non-terminal `Pending` — and webhook delivery is at-least-once and
    /// unordered, so this is the authoritative read a reconciliation job
    /// falls back to when a `charge.refund.updated` was missed or arrived out
    /// of order.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(&self, id: &str) -> Result<Refund, crate::Error> {
        get(self.client, &format!("/refunds/{}", path_segment(id)), None).await
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

/// `client.account_holders()` — see [`crate::Client::account_holders`].
#[derive(Debug, Clone, Copy)]
pub struct AccountHoldersResource<'a> {
    pub(crate) client: &'a Client,
}

impl AccountHoldersResource<'_> {
    /// `GET /v1/account_holders` — whose mobile-money account a number is.
    ///
    /// # Reading the answer
    ///
    /// [`AccountHolder::name`] is `None` when **the rail has no record** of
    /// the number. It is never `None` because vpay could not ask: that case
    /// is an [`enum@crate::Error`] — a `502` for a rail that could not be
    /// reached, a `400` for a rail with no such API — so a caller matching a
    /// name can tell "not registered" from "not checked". Both are refusals,
    /// and only one of them is the payer's to fix.
    ///
    /// The value is a third party's name. It is returned to the caller and
    /// deliberately never logged, stored or cached by vpay; an integrator
    /// holding it inherits the same obligation.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(
        &self,
        params: RetrieveAccountHolderParams,
    ) -> Result<AccountHolder, crate::Error> {
        get(
            self.client,
            "/account_holders",
            query_string(&params.to_form()),
        )
        .await
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
