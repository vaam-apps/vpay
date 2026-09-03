//! The JSON bodies Orange's Web Payment API exchanges, and nothing else.
//!
//! Kept apart from [`crate::mapping`] (which decides what a body *means*) and
//! from `lib.rs` (which decides how to send one) so that the shape of the wire
//! is reviewable against `docs/flows/adapter-orange-money.md` in one screen —
//! that doc is a reconstruction from community SDKs, not a vendor spec, so the
//! transcription is the part most likely to be wrong at onboarding.
//!
//! # Why none of these derive `Debug`
//!
//! Every one of them carries either a credential (`merchant_key`) or rail key
//! material (`pay_token`, `notif_token`, `access_token`). A derived `Debug` is
//! how those reach a log line: one `tracing::debug!(?body)` added later, and a
//! token that gates a payer's redirect is in the log stream. `pub(crate)`
//! types are exempt from `missing_debug_implementations`, so the lint does not
//! push back — the omission is deliberate, not an oversight.

use serde::{Deserialize, Serialize};

/// `POST {base}/v1/webpayment`.
///
/// `amount` is an `i64` and serialises as a JSON *number*, per the flow doc's
/// `"amount": 5000`. MTN's equivalent field is a string. Both come from the
/// same [`vpay_core::Money`] via `to_provider_minor` / `to_provider_string` —
/// one exponent lookup, two renderings (`docs/flows/money.md`).
#[derive(Serialize)]
pub(crate) struct WebPaymentRequest<'a> {
    pub(crate) merchant_key: &'a str,
    pub(crate) currency: &'a str,
    pub(crate) order_id: String,
    pub(crate) amount: i64,
    pub(crate) return_url: &'a str,
    pub(crate) cancel_url: &'a str,
    pub(crate) notif_url: &'a str,
    pub(crate) lang: &'a str,
}

/// The 201 body of `webpayment`.
///
/// Every field is `Option` even though the flow doc shows all three present:
/// a missing `pay_token` must produce a precise
/// [`vpay_provider::ProviderError::Malformed`] naming the field, not a serde
/// error naming a line and column of a body we must never log.
#[derive(Deserialize)]
pub(crate) struct WebPaymentResponse {
    pub(crate) pay_token: Option<String>,
    pub(crate) payment_url: Option<String>,
    pub(crate) notif_token: Option<String>,
}

/// `POST {base}/v1/transactionstatus`.
///
/// All three fields are required by the rail: the `order_id` alone does not
/// authorise the read, which is why `pay_token` must have been persisted
/// before the payer could act (`docs/flows/crash-safety.md`).
#[derive(Serialize)]
pub(crate) struct TransactionStatusRequest<'a> {
    pub(crate) order_id: String,
    pub(crate) amount: i64,
    pub(crate) pay_token: &'a str,
}

/// The 200 body of `transactionstatus`.
///
/// `message` is not in the flow doc's example. It is accepted anyway because
/// an unmapped `FAILED` must carry the rail's own words to an operator
/// (`docs/flows/failures.md`: "`provider_error` … carries the raw reason"),
/// and a field serde ignores costs nothing if the rail never sends it.
#[derive(Deserialize)]
pub(crate) struct TransactionStatusResponse {
    pub(crate) status: Option<String>,
    pub(crate) txnid: Option<String>,
    pub(crate) message: Option<String>,
}

/// The 200 body of `POST /oauth/v2/token`.
///
/// `expires_in` is `Option` and its absence is *not* an error: see
/// [`crate::token`] for why a missing lifetime means "use it once, do not
/// cache it" rather than an invented default.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: String,
    pub(crate) expires_in: Option<u64>,
}

/// The body Orange POSTs to `notif_url`.
///
/// It also carries a `status`, and this struct deliberately has no field for
/// it. A callback is a hint; only the authenticated status query moves money
/// (`docs/flows/reconciler.md`), and the cheapest way to guarantee an adapter
/// cannot leak a status out of an unauthenticated request is to give it
/// nowhere to put one.
#[derive(Deserialize)]
pub(crate) struct CallbackBody {
    pub(crate) order_id: Option<String>,
    pub(crate) notif_token: Option<String>,
    pub(crate) pay_token: Option<String>,
}
