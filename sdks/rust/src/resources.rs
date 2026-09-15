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
    AccountHolder, Balance, CheckoutSession, CheckoutUiMode, Customer, DeletedCustomer,
    DeletedInvoice, DeletedInvoiceItem, Event, Invoice, InvoiceLine, InvoiceStatus, List,
    PaymentIntent, PaymentMethodType, Refund,
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
    /// The `cus_…` this session is for (issue #70), or `None` to inherit the
    /// intent's customer — see [`crate::model::CheckoutSession::customer`]
    /// for the inheritance rule this SDK does not duplicate.
    ///
    /// Refused `409` naming `customer` when the session's `PaymentIntent`
    /// already names a *different* customer; the server never lets the two
    /// disagree.
    pub customer: Option<String>,
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
            (
                "customer".to_string(),
                FormValue::from(self.customer.clone()),
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
    /// The payer's postal address, encoded as `address[line1]=…`.
    ///
    /// An address alone does not name anybody, so it does not satisfy the
    /// one-of rule this struct's doc describes: a create carrying only an
    /// address is refused exactly as one carrying nothing is.
    pub address: Option<AddressParams>,
    /// Merchant-owned key/value pairs, encoded as `metadata[key]=value`.
    pub metadata: BTreeMap<String, String>,
}

/// The address components, in the shape the wire spells them — Stripe's six
/// **and** the GPS point.
///
/// A type of its own rather than [`crate::Address`] — which the *response*
/// carries — for the reason every params struct on this surface is its own
/// type: what a merchant may send and what the server returns are two
/// contracts, and the second one grows keys the first must not accept. They
/// happen to have the same eight fields today.
///
/// The coordinate is vpay's own and has no counterpart on Stripe's address;
/// see [`crate::Address`] for why it exists and why it is an integer count of
/// microdegrees rather than degrees.
///
/// `country` is sent as typed and stored **upper case**, so what comes back
/// may not be what was sent. This SDK deliberately does not check the shape
/// locally, exactly as it does not check an MSISDN: which codes vpay accepts
/// is a rule vpay owns and may widen, and a copy here would refuse offline an
/// address a later server accepts. The same line is taken on the coordinate:
/// the range and the pair rule are the server's, and it answers a `400`
/// naming `address` one round trip later.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddressParams {
    /// Street address, line 1.
    pub line1: Option<String>,
    /// Street address, line 2 — apartment, suite, PO box.
    pub line2: Option<String>,
    /// City, district, suburb, town or village.
    pub city: Option<String>,
    /// State, county, province or region.
    pub state: Option<String>,
    /// ZIP or postal code.
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2 — `CM`, `FR`, `NG`.
    pub country: Option<String>,
    /// Latitude in **microdegrees** — `4_061_000` for 4.061°N. An `i64` and
    /// never an `f64`: vpay stores no floating-point coordinate, and a type
    /// that could hold one here would make `4.061` expressible in Rust and
    /// refused on the wire. Sent with [`Self::longitude_microdeg`] or not at
    /// all.
    pub latitude_microdeg: Option<i64>,
    /// Longitude in **microdegrees**. See [`Self::latitude_microdeg`].
    pub longitude_microdeg: Option<i64>,
}

impl AddressParams {
    /// The eight components as `address[line1]=…` pairs, with every unset one
    /// omitted from the body entirely.
    ///
    /// Omitted and not sent empty, because on this API those are different
    /// requests everywhere else — and because the server replaces the address
    /// whole, so an omitted component is cleared either way and sending
    /// `address[city]=` would only add a pair that says the same thing.
    ///
    /// The coordinate goes out as a decimal integer through
    /// `FormValue::from(i64)`, which is the same encoder every amount on this
    /// surface uses — there is no path from this struct to a body containing
    /// a decimal point.
    fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("line1".to_string(), FormValue::from(self.line1.clone())),
            ("line2".to_string(), FormValue::from(self.line2.clone())),
            ("city".to_string(), FormValue::from(self.city.clone())),
            ("state".to_string(), FormValue::from(self.state.clone())),
            (
                "postal_code".to_string(),
                FormValue::from(self.postal_code.clone()),
            ),
            ("country".to_string(), FormValue::from(self.country.clone())),
            (
                "latitude_microdeg".to_string(),
                FormValue::from(self.latitude_microdeg),
            ),
            (
                "longitude_microdeg".to_string(),
                FormValue::from(self.longitude_microdeg),
            ),
        ])
    }
}

impl CreateCustomerParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("name".to_string(), FormValue::from(self.name.clone())),
            ("email".to_string(), FormValue::from(self.email.clone())),
            ("phone".to_string(), FormValue::from(self.phone.clone())),
            (
                "address".to_string(),
                self.address
                    .as_ref()
                    .map_or(FormValue::Skip, AddressParams::to_form),
            ),
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
    /// The address, in the same three states — with one difference worth
    /// stating, because it is the one a merchant can get wrong silently.
    ///
    /// `None` leaves the stored address alone. `Some(None)` removes it
    /// (sent as `address=`). `Some(Some(address))` **replaces** it whole:
    /// every component the request does not name is cleared, so correcting a
    /// street means sending the city again. vpay does not merge components,
    /// deliberately — an address assembled out of two requests is an address
    /// that was never anybody's, and its failure mode is a plausible wrong
    /// address rather than a visible one.
    pub address: Option<Option<AddressParams>>,
    /// Keys to merge. A key whose value is the empty string is **removed**
    /// from the stored metadata, which is Stripe's own per-key delete.
    pub metadata: BTreeMap<String, String>,
}

/// One three-state patch field, in the wire's own encoding.
///
/// `None` is omitted from the body entirely ("leave it alone"), `Some(None)`
/// becomes the empty string — which is what "clear this" is on a
/// form-encoded wire — and `Some(Some(v))` sets it.
///
/// Written once and used by every patch type on this surface
/// ([`UpdateCustomerParams`], [`UpdateInvoiceParams`]) rather than per
/// resource: the three states are one wire rule, and a second copy is a
/// second thing that can lose the distinction between "not mentioned" and
/// "cleared".
fn patch_form<T: Into<FormValue> + Clone>(field: Option<&Option<T>>) -> FormValue {
    match field {
        None => FormValue::Skip,
        Some(None) => FormValue::from(""),
        Some(Some(value)) => value.clone().into(),
    }
}

impl UpdateCustomerParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            ("name".to_string(), patch_form(self.name.as_ref())),
            ("email".to_string(), patch_form(self.email.as_ref())),
            ("phone".to_string(), patch_form(self.phone.as_ref())),
            (
                "address".to_string(),
                match self.address.as_ref() {
                    None => FormValue::Skip,
                    // `address=` — the wire's own "remove this", the same
                    // spelling `name=` uses. NOT `address[line1]=` and seven
                    // more: the server reads a *scalar* `address` as the
                    // clear, and an object of eight empty components as an
                    // address whose components are all absent, which it
                    // stores as no address by a different route. One
                    // spelling, so a merchant reading the body sees what
                    // they asked for.
                    Some(None) => FormValue::from(""),
                    Some(Some(address)) => address.to_form(),
                },
            ),
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

/// `POST /v1/invoices` request fields (S4b).
///
/// `customer` and `currency` are both required, and both are plain `String`s
/// rather than `Option<String>`s so that the type says so.
///
/// `customer` is required for a reason worth stating: an invoice is a bill to
/// somebody, and one that names no payer is one nobody can be asked to pay.
/// The server answers a `400` naming `customer` for an absent, unknown or
/// *other merchant's* `cus_…`, always the same sentence, so the parameter
/// cannot be used to discover which customers exist under some other account.
///
/// `currency` is required because **the server requires it**, exactly as it
/// does on an intent — `parse_currency(None, …)` is
/// `400 A three-letter \`currency\` code is required.`, and there is no
/// deployment default it falls back to. This field was an `Option<String>`
/// until 2026-09-08, documented as "omitted from the body entirely when
/// `None`, which is how the server gets to apply this deployment's own
/// default": measured against a running vpay, that request is a `400`. A
/// type that can express only sendable requests is the same reason
/// [`CreatePaymentIntentParams::currency`] is a `String`.
///
/// A created invoice is always a [`crate::InvoiceStatus::Draft`] with no
/// number, no lines and zero amounts. Add lines with
/// [`InvoiceItemsResource::create`], then issue it with
/// [`InvoicesResource::finalize`].
#[derive(Debug, Clone, Default)]
pub struct CreateInvoiceParams {
    /// The `cus_…` this invoice bills. Required.
    pub customer: String,
    /// Lower-cased at encode time regardless of how it was supplied, for
    /// [`CreatePaymentIntentParams::currency`]'s reason. Required — see the
    /// type's own documentation.
    pub currency: String,
    /// The merchant's note on the document. At most 1000 characters, which
    /// the server checks.
    pub description: Option<String>,
    /// Unix **seconds**, as Stripe spells it — not milliseconds.
    ///
    /// **Advisory**: nothing in vpay reads it. There is no dunning and no
    /// automatic transition, so setting this changes nothing about what vpay
    /// does; it is a field the merchant's own systems may read back off
    /// [`crate::Invoice::due_date`].
    pub due_date: Option<i64>,
    /// Merchant-owned key/value pairs, encoded as `metadata[key]=value`.
    pub metadata: BTreeMap<String, String>,
}

impl CreateInvoiceParams {
    /// The two required fields, for a caller who would otherwise write a
    /// struct literal with a `..Default::default()` in it.
    #[must_use]
    pub fn new(customer: impl Into<String>, currency: impl Into<String>) -> Self {
        Self {
            customer: customer.into(),
            currency: currency.into(),
            ..Self::default()
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "customer".to_string(),
                FormValue::from(self.customer.as_str()),
            ),
            (
                "currency".to_string(),
                FormValue::from(self.currency.to_lowercase()),
            ),
            (
                "description".to_string(),
                FormValue::from(self.description.clone()),
            ),
            ("due_date".to_string(), FormValue::from(self.due_date)),
            ("metadata".to_string(), metadata_form(&self.metadata)),
        ])
    }
}

/// `POST /v1/invoices/{id}` request fields (S4b) — **draft only**.
///
/// # Three states per field, exactly as [`UpdateCustomerParams`] has
///
/// `None` leaves the field alone, `Some(Some(v))` sets it and `Some(None)`
/// **clears** it, which this SDK sends as `description=`. A plain
/// `Option<T>` collapses the first and the third and a merchant could not
/// remove a due date they set by mistake.
///
/// `customer` and `currency` are **not** patchable and deliberately absent
/// from this type: an invoice that changed who it bills, or what it is
/// denominated in, is a different document.
///
/// A finalized invoice is a `409` naming its status — an issued document's
/// terms are what was sent to the payer. Void it and issue a new one.
#[derive(Debug, Clone, Default)]
pub struct UpdateInvoiceParams {
    /// `None` leaves the description alone; `Some(None)` clears it.
    pub description: Option<Option<String>>,
    /// See [`Self::description`]. Unix **seconds**.
    pub due_date: Option<Option<i64>>,
    /// Keys to merge. A key whose value is the empty string is **removed**
    /// from the stored metadata, which is Stripe's own per-key delete, so an
    /// empty map means "leave metadata alone" and there is no separate
    /// "clear all of it".
    pub metadata: BTreeMap<String, String>,
}

impl UpdateInvoiceParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "description".to_string(),
                patch_form(self.description.as_ref()),
            ),
            ("due_date".to_string(), patch_form(self.due_date.as_ref())),
            ("metadata".to_string(), metadata_form(&self.metadata)),
        ])
    }
}

/// `GET /v1/invoices` query parameters (S4b). All optional; an unset field is
/// omitted from the query string entirely.
#[derive(Debug, Clone, Default)]
pub struct ListInvoicesParams {
    /// Page size. The server's own default and ceiling apply when unset.
    pub limit: Option<u32>,
    /// Cursor: return invoices *after* this `in_…` (the next page).
    pub starting_after: Option<String>,
    /// Cursor: return invoices *before* this `in_…` (the previous page).
    pub ending_before: Option<String>,
    /// Only this customer's invoices.
    pub customer: Option<String>,
    /// Only invoices in this state.
    ///
    /// A typed [`crate::InvoiceStatus`] rather than a `String`, and that is
    /// the one filter where the type earns its place: the server answers a
    /// `400` naming `status` for a label it does not have rather than
    /// silently returning the whole first page, so a typo here would cost a
    /// round trip and read like an empty result. It is also how a merchant
    /// finds their write-offs, which emit no event.
    pub status: Option<InvoiceStatus>,
}

impl ListInvoicesParams {
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
                "customer".to_string(),
                FormValue::from(self.customer.clone()),
            ),
            (
                "status".to_string(),
                FormValue::from(self.status.map(InvoiceStatus::as_wire_str)),
            ),
        ])
    }
}

/// `POST /v1/invoices/{id}/pay` request fields (S4b).
///
/// **Both URLs are required**, unlike on a checkout session where the pair is
/// required only in hosted mode: `pay` mints a *hosted* session and there is
/// no other mode to be in. They take the same rules the session's do —
/// http(s), at most 2048 characters, `https` only under livemode, and
/// `success_url` may carry the literal `{CHECKOUT_SESSION_ID}` — and this SDK
/// deliberately does not duplicate them, for
/// [`CreateCheckoutSessionParams`]' reason.
///
/// This does **not** charge anything. Stripe's `pay` charges a stored payment
/// method; vpay has none, so this mints a payment intent for
/// [`crate::Invoice::amount_remaining`] and answers the invoice with
/// [`crate::Invoice::hosted_invoice_url`] set — a page to send the payer to.
#[derive(Debug, Clone, Default)]
pub struct PayInvoiceParams {
    /// Where a paying payer is forwarded.
    pub success_url: String,
    /// Where a payer who gave up is forwarded.
    pub cancel_url: String,
}

impl PayInvoiceParams {
    /// The two required URLs.
    #[must_use]
    pub fn new(success_url: impl Into<String>, cancel_url: impl Into<String>) -> Self {
        Self {
            success_url: success_url.into(),
            cancel_url: cancel_url.into(),
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "success_url".to_string(),
                FormValue::from(self.success_url.as_str()),
            ),
            (
                "cancel_url".to_string(),
                FormValue::from(self.cancel_url.as_str()),
            ),
        ])
    }
}

/// `POST /v1/invoice_items` request fields (S4b).
///
/// There is no `currency` parameter and no `amount` one, and both absences
/// are the wire contract rather than an omission here: a line is always in
/// its invoice's currency (copied off the parent in the statement that writes
/// the row), and `amount` is `quantity * unit_amount` computed by the
/// database. A merchant who could send `amount` could send one that did not
/// match its own factors.
#[derive(Debug, Clone, Default)]
pub struct CreateInvoiceItemParams {
    /// The `in_…` to add this line to. Required, and it must be one of your
    /// **drafts** — a `400` naming `invoice` otherwise, including for a
    /// finalized invoice and for another merchant's.
    pub invoice: String,
    /// The text on the document. Required, at most 1000 characters.
    pub description: String,
    /// How many. At least 1; omitted from the body when `None`, so the
    /// server applies its own default of 1 rather than this SDK sending one.
    pub quantity: Option<i64>,
    /// The price of one, in integer minor units.
    ///
    /// Held to the same `0..=2^53-1` bound as every other amount on this
    /// surface, refused before a request is built — see
    /// [`crate::validate`]'s reasoning: an amount this SDK sent and the Node
    /// SDK refused would be a divergence in the money path.
    pub unit_amount: i64,
}

impl CreateInvoiceItemParams {
    /// The three required fields.
    #[must_use]
    pub fn new(
        invoice: impl Into<String>,
        description: impl Into<String>,
        unit_amount: i64,
    ) -> Self {
        Self {
            invoice: invoice.into(),
            description: description.into(),
            quantity: None,
            unit_amount,
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "invoice".to_string(),
                FormValue::from(self.invoice.as_str()),
            ),
            (
                "description".to_string(),
                FormValue::from(self.description.as_str()),
            ),
            ("quantity".to_string(), FormValue::from(self.quantity)),
            ("unit_amount".to_string(), FormValue::from(self.unit_amount)),
        ])
    }
}

/// `POST /v1/invoice_items/{id}` request fields (S4b) — **draft parent
/// only**.
///
/// # Why these are single `Option`s and [`UpdateInvoiceParams`]' are double
///
/// All three columns are `NOT NULL`, so there is no "clear it" state to
/// carry: `None` leaves the field alone and `Some(v)` sets it. Sending
/// `description=` is a `400` naming the parameter rather than a clear, which
/// is why this type cannot express it.
///
/// `amount` is not here, ever — see [`CreateInvoiceItemParams`].
#[derive(Debug, Clone, Default)]
pub struct UpdateInvoiceItemParams {
    /// The text on the document.
    pub description: Option<String>,
    /// How many. At least 1.
    pub quantity: Option<i64>,
    /// The price of one, in integer minor units. Bounded like
    /// [`CreateInvoiceItemParams::unit_amount`].
    pub unit_amount: Option<i64>,
}

impl UpdateInvoiceItemParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![
            (
                "description".to_string(),
                FormValue::from(self.description.clone()),
            ),
            ("quantity".to_string(), FormValue::from(self.quantity)),
            ("unit_amount".to_string(), FormValue::from(self.unit_amount)),
        ])
    }
}

/// Where a refund's money goes, on a rail that cannot send it back the way it
/// came.
///
/// # The rail's code is the outer key, and that is the whole point
///
/// On the wire this is `destination[<payment_method_type>][…]` — a
/// rail-agnostic container with a rail-specific interior, exactly as
/// `payment_method_data[<payment_method_type>][…]` is on the confirm path.
/// The server strips the outer key and hands the interior to that rail's own
/// adapter, which is the only thing that knows what a payee looks like there.
/// An SDK that flattened this, or that wrote one rail's code into the body
/// whatever rail the charge was on, would be deciding a rail's payee shape
/// from the client.
///
/// # Which rail, and why you do not choose it
///
/// A refund goes back on the rail the charge was made on. The server reads
/// that off the charge and refuses a `destination` naming any other rail with
/// a `400` naming `destination`, so the value here must be the
/// [`PaymentMethodType`] the intent was confirmed on — not a preference.
///
/// # Only rails that need one take one
///
/// A rail that returns money to the instrument that paid takes **no**
/// `destination` and answers `400` if sent one. Both rails vpay carries today
/// need one, because a mobile-money refund is an outbound transfer to a
/// number. See [`RefundsResource::create`].
///
/// # The `+` is required, and this SDK does not add it for you
///
/// The server canonicalises the number and **refuses one that does not start
/// with `+`**: `+237600000200` is a payee and the bare national `600000200`
/// is a `400` naming `destination`. That is not the rule
/// `GET /v1/account_holders` applies — that route knows it is in Cameroon and
/// the refund path deliberately does not, because read as an international
/// number `600000200` names a different country. It is asymmetric in the safe
/// direction: a lookup that guesses wrong returns the wrong name, a transfer
/// that guesses wrong sends the money.
///
/// This SDK sends the string it is given, unchanged: it does not add a `+`,
/// strip one, or rewrite the number into any other form, for the reason
/// [`RetrieveAccountHolderParams`] gives about validating numbers locally —
/// the rule is the server's and it may widen it. Separators a caller types
/// (`+237 600 000 200`, `+237-600-000-200`) are accepted by the server and
/// are not this type's business either.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefundDestination {
    /// A mobile-money payee: the rail to pay on, and the number to pay.
    ///
    /// Named for the *shape* rather than for either rail, so that a
    /// destination which is not a phone number is a sibling variant rather
    /// than a silent change of meaning to this one.
    MobileMoney {
        /// The rail the charge was made on — its code is the outer key.
        payment_method_type: PaymentMethodType,
        /// The payee's number, **in international form, starting with `+`**.
        /// Sent as `destination[<payment_method_type>][msisdn]`.
        msisdn: String,
    },
}

impl RefundDestination {
    /// A mobile-money payee on `payment_method_type`.
    #[must_use]
    pub fn mobile_money(payment_method_type: PaymentMethodType, msisdn: impl Into<String>) -> Self {
        RefundDestination::MobileMoney {
            payment_method_type,
            msisdn: msisdn.into(),
        }
    }

    /// The rail this destination is on.
    #[must_use]
    pub fn payment_method_type(&self) -> PaymentMethodType {
        match self {
            RefundDestination::MobileMoney {
                payment_method_type,
                ..
            } => *payment_method_type,
        }
    }

    pub(crate) fn to_form(&self) -> FormValue {
        match self {
            RefundDestination::MobileMoney {
                payment_method_type,
                msisdn,
            } => FormValue::Object(vec![(
                payment_method_type.as_wire_str().to_string(),
                FormValue::Object(vec![(
                    "msisdn".to_string(),
                    FormValue::from(msisdn.as_str()),
                )]),
            )]),
        }
    }
}

/// Renders the rail and **not** the number.
///
/// Hand-written for [`crate::PaymentIntent`]'s `client_secret` reason: the
/// payee is a third party, their number is personal data this merchant holds
/// on their behalf, and a structured logger prints `{:?}` of whatever it is
/// handed. vpay's own handler logs this value masked and never raw; an SDK
/// that printed it in full would put it in the merchant's logs instead, which
/// is the same leak one hop earlier.
///
/// "The payee is on `mtn_momo`" is what a merchant debugging a `400` needs
/// from a log line. The number they already have.
impl std::fmt::Debug for RefundDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefundDestination::MobileMoney {
                payment_method_type,
                ..
            } => f
                .debug_struct("RefundDestination::MobileMoney")
                .field("payment_method_type", payment_method_type)
                .field("msisdn", &"[redacted]")
                .finish(),
        }
    }
}

/// `POST /v1/refunds` request fields.
///
/// # No refund this creates has ever moved money
///
/// The route is real and so is every refusal it makes, but nothing on the
/// other side of it pays anybody: `orange_money`'s refund is a declared
/// `NotImplemented` token, `mtn_momo`'s is MTN's Disbursements `transfer`
/// which is WireMock-proven and which no deployment holds a credential for,
/// and **nothing settles a `pending` refund** — there is no refund poll
/// ladder, so a refund this creates stays
/// [`Pending`](crate::RefundStatus::Pending) until an operator moves it.
/// `docs/status.md` carries all three. Do not read a `200` here as money on
/// its way back.
#[derive(Clone, Default)]
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
    ///
    /// A rail that refunds in full only refuses a partial amount with a
    /// `400` naming `amount`, and it measures "in full" against the intent's
    /// own amount rather than against what is left.
    pub amount: Option<i64>,
    /// Merchant-supplied reason, echoed back on the refund object.
    pub reason: Option<String>,
    /// The payee, on a rail that cannot return money to the instrument that
    /// paid. See [`RefundDestination`], and [`RefundsResource::create`] for
    /// which rails need one.
    pub destination: Option<RefundDestination>,
    /// Merchant-owned key/value pairs, encoded as `metadata[key]=value`.
    pub metadata: BTreeMap<String, String>,
}

/// Redacts [`destination`](CreateRefundParams::destination) by delegating to
/// its own `Debug`, and is hand-written so that a future `#[derive(Debug)]`
/// here cannot quietly re-derive the field.
///
/// The derive would in fact still be safe today, because
/// [`RefundDestination`]'s own `Debug` is the redacting one. It is written
/// out anyway for the reason `vpay_api::v1::refunds::CreateParams` gives
/// about the same field: the guarantee is worth an impl a test can name, and
/// this is what `a_create_refund_params_debug_output_never_contains_the_payees_number`
/// pins.
impl std::fmt::Debug for CreateRefundParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateRefundParams")
            .field("payment_intent", &self.payment_intent)
            .field("amount", &self.amount)
            .field("reason", &self.reason)
            .field("destination", &self.destination)
            .field("metadata", &self.metadata)
            .finish()
    }
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
            (
                "destination".to_string(),
                self.destination
                    .as_ref()
                    .map_or(FormValue::Skip, RefundDestination::to_form),
            ),
            ("metadata".to_string(), metadata_form(&self.metadata)),
        ])
    }
}

/// `POST /v1/refunds/{id}` request fields — **metadata, and nothing else**.
///
/// There is no `amount`, no `reason`, no `payment_intent` and no
/// `destination` here, and their absence is this type saying what the server
/// says: all four are refused with a `400` naming the parameter once a refund
/// exists. A refund whose amount or payee is wrong is
/// [cancelled](RefundsResource::cancel) while it is still
/// [`Pending`](crate::RefundStatus::Pending) and created again.
///
/// # The merge is key-wise, and an empty value deletes a key
///
/// Stripe's rule, which vpay follows: the map sent here is merged into the
/// stored one key by key — keys not mentioned are left alone — and a key sent
/// with an **empty string** value is removed. Sending an empty map is a
/// no-op read: the refund comes back unchanged and **no event is written**.
#[derive(Debug, Clone, Default)]
pub struct UpdateRefundParams {
    /// The keys to set, and the keys to delete (an empty value deletes).
    pub metadata: BTreeMap<String, String>,
}

impl UpdateRefundParams {
    pub(crate) fn to_form(&self) -> FormValue {
        FormValue::Object(vec![("metadata".to_string(), metadata_form(&self.metadata))])
    }
}

/// `GET /v1/refunds` query parameters. All optional; an unset field is
/// omitted from the query string entirely.
#[derive(Debug, Clone, Default)]
pub struct ListRefundsParams {
    /// Page size. The server's own default and ceiling apply when unset.
    pub limit: Option<u32>,
    /// Cursor: return refunds *after* this `re_…` (the next page).
    pub starting_after: Option<String>,
    /// Cursor: return refunds *before* this `re_…` (the previous page).
    pub ending_before: Option<String>,
    /// Only refunds against this `pi_…`.
    ///
    /// Held to the PaymentIntent id shape by the server, which answers a
    /// `400` naming `payment_intent` for anything else — a `re_…` sent here
    /// is the easy mistake, since both ids are on the refund object. An id
    /// of the right shape that names nothing, or another merchant's intent,
    /// is an **empty page** and not a `404`.
    pub payment_intent: Option<String>,
}

impl ListRefundsParams {
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

/// `client.invoices()` — the nine merchant operations on `/v1/invoices`
/// (S4b): five CRUD and the four transitions.
///
/// # The one thing to know before calling any of them
///
/// **The state machine is the server's `WHERE` clause, not a check this SDK
/// makes.** A transition that does not apply comes back a `409` naming the
/// invoice's current status, and this SDK does not try to predict which —
/// `draft → open → paid | void | uncollectible`, and every method below says
/// which statuses it accepts. Reading the invoice first to decide whether to
/// call is a race; call, and read the refusal.
///
/// `del` and not `delete`, for [`CustomersResource`]'s reason.
#[derive(Debug, Clone, Copy)]
pub struct InvoicesResource<'a> {
    pub(crate) client: &'a Client,
}

impl InvoicesResource<'_> {
    /// `POST /v1/invoices` — creates a **draft**, with no number and zero
    /// amounts.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. In particular a `400` naming `customer`
    /// when it is absent, unknown, or another merchant's — one sentence for
    /// all three, so the parameter is not an oracle; and a `400` naming
    /// `currency` for one this deployment does not settle.
    pub async fn create(
        &self,
        params: CreateInvoiceParams,
        opts: RequestOptions,
    ) -> Result<Invoice, crate::Error> {
        post(self.client, "/invoices", params.to_form(), opts).await
    }

    /// `GET /v1/invoices/{id}` — the invoice **with its lines**.
    ///
    /// This is the read a webhook handler falls back to: an `invoice.*` event
    /// body carries an empty `lines.data` (see [`crate::Event::invoice`]),
    /// and this response always carries the lines.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. Another merchant's `in_…` is the same
    /// `404`, byte for byte, as one that never existed.
    pub async fn retrieve(&self, id: &str) -> Result<Invoice, crate::Error> {
        get(
            self.client,
            &format!("/invoices/{}", path_segment(id)),
            None,
        )
        .await
    }

    /// `POST /v1/invoices/{id}` — patches a **draft**.
    ///
    /// A `POST` and not a `PATCH`, for [`CustomersResource::update`]'s
    /// reason, even though the server mounts both verbs on the one handler:
    /// `POST` is what a merchant's existing Stripe client sends, so it is the
    /// one that has to work.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. A `409` naming the status once the invoice
    /// is no longer a draft — an issued document's terms are what was sent to
    /// the payer.
    pub async fn update(
        &self,
        id: &str,
        params: UpdateInvoiceParams,
        opts: RequestOptions,
    ) -> Result<Invoice, crate::Error> {
        post(
            self.client,
            &format!("/invoices/{}", path_segment(id)),
            params.to_form(),
            opts,
        )
        .await
    }

    /// `GET /v1/invoices` — newest first.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn list(&self, params: ListInvoicesParams) -> Result<List<Invoice>, crate::Error> {
        get(self.client, "/invoices", query_string(&params.to_form())).await
    }

    /// `DELETE /v1/invoices/{id}` — **draft only**, and its lines go with it.
    ///
    /// An issued invoice is [`Self::void`]ed, never deleted: a voided draft
    /// would be a non-draft row with no number, which migration `0036`
    /// refuses outright, and a document that vanished is a hole an accountant
    /// reads as a destroyed one.
    ///
    /// Carries an `Idempotency-Key` like every other write, which is what
    /// makes a retried delete answer the original `{deleted: true}` rather
    /// than a `404`.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. A `409` for anything but a draft.
    pub async fn del(
        &self,
        id: &str,
        opts: RequestOptions,
    ) -> Result<DeletedInvoice, crate::Error> {
        self.client
            .delete(&format!("/invoices/{}", path_segment(id)), opts)
            .await
    }

    /// `POST /v1/invoices/{id}/finalize` — issues the document. **Draft
    /// only.**
    ///
    /// The money transition: the invoice takes the next number out of this
    /// merchant's own sequence under a row lock, its lines freeze, its
    /// amounts are computed once, and it moves to
    /// [`crate::InvoiceStatus::Open`] — all in one transaction, with
    /// `invoice.finalized` inside it. Numbers are consecutive and have no
    /// holes, unlike Stripe's.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. A `400` naming `invoice` when it has no
    /// lines, or when its total is past `2^53-1` minor units; a `409` when it
    /// is not a draft.
    pub async fn finalize(&self, id: &str, opts: RequestOptions) -> Result<Invoice, crate::Error> {
        post(
            self.client,
            &format!("/invoices/{}/finalize", path_segment(id)),
            FormValue::Object(Vec::new()),
            opts,
        )
        .await
    }

    /// `POST /v1/invoices/{id}/void` — cancels an issued document. **Open
    /// only.**
    ///
    /// The invoice **keeps its number**. Terminal.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. A `409` when it is not open, and a `409`
    /// while a payment intent is attached and not yet canceled — cancel that
    /// intent first.
    pub async fn void(&self, id: &str, opts: RequestOptions) -> Result<Invoice, crate::Error> {
        post(
            self.client,
            &format!("/invoices/{}/void", path_segment(id)),
            FormValue::Object(Vec::new()),
            opts,
        )
        .await
    }

    /// `POST /v1/invoices/{id}/mark_uncollectible` — writes an issued
    /// document off. **Open only.**
    ///
    /// Still owed, never expected. Terminal, and it **emits no event**: vpay
    /// does not write `invoice.marked_uncollectible`, so nothing arrives on a
    /// webhook and a merchant reconciling write-offs reads them with
    /// [`Self::list`] filtered on
    /// [`crate::InvoiceStatus::Uncollectible`].
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. As [`Self::void`].
    pub async fn mark_uncollectible(
        &self,
        id: &str,
        opts: RequestOptions,
    ) -> Result<Invoice, crate::Error> {
        post(
            self.client,
            &format!("/invoices/{}/mark_uncollectible", path_segment(id)),
            FormValue::Object(Vec::new()),
            opts,
        )
        .await
    }

    /// `POST /v1/invoices/{id}/pay` — mints a payment intent and a hosted
    /// checkout for what is left. **Open only.**
    ///
    /// **This does not charge anybody.** Stripe's `pay` charges a stored
    /// payment method; vpay has none, so this answers the invoice with
    /// [`crate::Invoice::payment_intent`] and
    /// [`crate::Invoice::hosted_invoice_url`] set — a page to send the payer
    /// to. The invoice stays [`crate::InvoiceStatus::Open`] until the
    /// settlement moves it, which is when `invoice.paid` is emitted.
    ///
    /// One payment at a time: calling this again while the attached intent is
    /// live is a `409`, and cancelling that intent is the way back. Two
    /// concurrent calls attach exactly one intent.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. A `409` when it is not open or an intent is
    /// already live, a `400` for a URL the server refuses, and a `500`
    /// `checkout_not_configured` when this deployment serves no checkout
    /// page.
    pub async fn pay(
        &self,
        id: &str,
        params: PayInvoiceParams,
        opts: RequestOptions,
    ) -> Result<Invoice, crate::Error> {
        post(
            self.client,
            &format!("/invoices/{}/pay", path_segment(id)),
            params.to_form(),
            opts,
        )
        .await
    }
}

/// `client.invoice_items()` — the four operations on `/v1/invoice_items`
/// (S4b): the lines of a **draft** invoice.
///
/// There is deliberately no `list`: an invoice's lines are read off the
/// invoice ([`crate::Invoice::lines`], expanded on every render), and a
/// merchant-wide list of lines across invoices answers no question anybody
/// asked. The server mounts no collection `GET` either, so this is not an
/// omission here.
///
/// Every write requires the parent to still be a
/// [`crate::InvoiceStatus::Draft`]; [`Self::retrieve`] does not.
#[derive(Debug, Clone, Copy)]
pub struct InvoiceItemsResource<'a> {
    pub(crate) client: &'a Client,
}

impl InvoiceItemsResource<'_> {
    /// `POST /v1/invoice_items` — adds a line to a draft, and the invoice's
    /// `amount_due` moves with it.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. [`crate::Error::InvalidParams`] before any
    /// request when `unit_amount` is outside `0..=2^53-1`; a `400` naming
    /// `invoice` when it is not one of your drafts.
    pub async fn create(
        &self,
        params: CreateInvoiceItemParams,
        opts: RequestOptions,
    ) -> Result<InvoiceLine, crate::Error> {
        check_amount(params.unit_amount, "unit_amount")?;
        post(self.client, "/invoice_items", params.to_form(), opts).await
    }

    /// `GET /v1/invoice_items/{id}` — readable whatever the parent's status.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn retrieve(&self, id: &str) -> Result<InvoiceLine, crate::Error> {
        get(
            self.client,
            &format!("/invoice_items/{}", path_segment(id)),
            None,
        )
        .await
    }

    /// `POST /v1/invoice_items/{id}` — **draft parent only**, with `amount`
    /// recomputed.
    ///
    /// # Errors
    /// See [`enum@crate::Error`]. [`crate::Error::InvalidParams`] for a
    /// `unit_amount` outside the bound; a `409` once the parent is no longer
    /// a draft.
    pub async fn update(
        &self,
        id: &str,
        params: UpdateInvoiceItemParams,
        opts: RequestOptions,
    ) -> Result<InvoiceLine, crate::Error> {
        if let Some(unit_amount) = params.unit_amount {
            check_amount(unit_amount, "unit_amount")?;
        }
        post(
            self.client,
            &format!("/invoice_items/{}", path_segment(id)),
            params.to_form(),
            opts,
        )
        .await
    }

    /// `DELETE /v1/invoice_items/{id}` — **draft parent only**, and the
    /// invoice re-totals.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn del(
        &self,
        id: &str,
        opts: RequestOptions,
    ) -> Result<DeletedInvoiceItem, crate::Error> {
        self.client
            .delete(&format!("/invoice_items/{}", path_segment(id)), opts)
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
    /// # The `destination` is not optional in practice
    ///
    /// It is an `Option` because the port allows a rail that refunds to the
    /// instrument that paid, and such a rail refuses a `destination` with a
    /// `400`. **Neither rail vpay carries today is one**: `mtn_momo` and
    /// `orange_money` both declare that a refund needs a payee, because on
    /// both of them it is an outbound transfer. A
    /// [`CreateRefundParams`] with no [`destination`](CreateRefundParams::destination)
    /// is therefore a `400` naming `destination` against every deployment
    /// this repository can build.
    ///
    /// # What a `200` here does and does not mean
    ///
    /// It means a refund row exists, its amount is reserved against the
    /// intent, and `charge.refunded` has been written. It does **not** mean
    /// the payee has been paid: see [`CreateRefundParams`].
    ///
    /// # Errors
    /// [`crate::Error::InvalidParams`] if `amount` is present and negative or
    /// beyond `2^53-1`, before any request is sent; otherwise see
    /// [`enum@crate::Error`] — in particular a `400` naming `destination` for
    /// a payee this rail will not take, a `400` naming `amount` for a partial
    /// refund on a rail that has no partials, and a `409` for an intent with
    /// nothing left to refund.
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

    /// `POST /v1/refunds/{id}` — the **metadata** update, and nothing else.
    ///
    /// See [`UpdateRefundParams`]: the other four fields of a refund are
    /// fixed once it exists, the merge is key-wise, and an empty value
    /// deletes a key. A refund whose metadata actually changes emits
    /// `charge.refund.updated`; one sent an empty map is read back unchanged
    /// and emits nothing.
    ///
    /// # Errors
    /// See [`enum@crate::Error`] — a `404` for a refund this merchant has
    /// none of, and a `400` naming the parameter for anything but
    /// `metadata`, which this params type has no way to send.
    pub async fn update(
        &self,
        id: &str,
        params: UpdateRefundParams,
        opts: RequestOptions,
    ) -> Result<Refund, crate::Error> {
        post(
            self.client,
            &format!("/refunds/{}", path_segment(id)),
            params.to_form(),
            opts,
        )
        .await
    }

    /// `GET /v1/refunds` — this merchant's refunds, newest first.
    ///
    /// # Errors
    /// See [`enum@crate::Error`].
    pub async fn list(&self, params: ListRefundsParams) -> Result<List<Refund>, crate::Error> {
        get(self.client, "/refunds", query_string(&params.to_form())).await
    }

    /// `POST /v1/refunds/{id}/cancel` — **while it is still pending**. No
    /// request fields.
    ///
    /// The reservation this refund holds against its intent is released and
    /// `charge.refund.updated` is written. The state machine is the server's
    /// `WHERE` clause, not a check this SDK makes: a refund that settles
    /// between your read and this call is refused with a `409` naming the
    /// status that refused it. Reading the refund first to decide whether to
    /// call is a race; call, and read the refusal.
    ///
    /// # Nothing here reaches a rail
    ///
    /// A cancel is a vpay-side release. No rail is told to stop anything,
    /// because no rail was ever told to start: see [`CreateRefundParams`].
    ///
    /// # Errors
    /// See [`enum@crate::Error`] — a `404` for a refund this merchant has
    /// none of, and a `409` for one that is no longer
    /// [`Pending`](crate::RefundStatus::Pending).
    pub async fn cancel(&self, id: &str, opts: RequestOptions) -> Result<Refund, crate::Error> {
        post(
            self.client,
            &format!("/refunds/{}/cancel", path_segment(id)),
            FormValue::Object(vec![]),
            opts,
        )
        .await
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
