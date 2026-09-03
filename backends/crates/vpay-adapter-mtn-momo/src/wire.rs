//! The JSON MTN's Collections API sends and expects, and nothing else.
//!
//! Separated from the request logic so the wire shapes can be read against
//! `docs/flows/adapter-mtn-momo.md` without the transport in the way, and so
//! a change to a field name is a change to one small file rather than to a
//! request builder.
//!
//! Everything here is deliberately *liberal* on the way in and *exact* on the
//! way out. A rail that starts quoting a number must not turn a settled
//! payment into a parse error the poll ladder retries forever; a request body
//! that drifts from the documented shape is a charge the rail refuses.
//!
//! # No `#[serde(rename_all = "snake_case")]` in this file, ever
//!
//! The workspace convention is that every type modelling *vpay's own* wire or
//! config carries it, so a field added as `payTo` fails review instead of
//! shipping. These types model **MTN's** wire, which is camelCase
//! (`externalId`, `partyIdType`, `partyId`, `financialTransactionId`), and
//! the rename attributes below are what makes each one exact. A blanket
//! `rename_all` here would be a no-op masked by those per-field renames on
//! the fields that have one, and a silent wire break on the fields that do
//! not. Same rule applies to `token.rs`'s `TokenResponse`, which models
//! OAuth's.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vpay_provider::{ChargeRef, ProviderError};

/// The `requesttopay` body, per `docs/flows/adapter-mtn-momo.md`.
///
/// `payerMessage` and `payeeNote` are documented by MTN and deliberately not
/// sent: both are shown to the payer on their handset, the port carries no
/// merchant-supplied text to put in them, and inventing a constant string
/// would put words nobody chose in front of a payer. They go in when the
/// core has a field for them.
#[derive(Debug, Serialize)]
pub(crate) struct RequestToPay {
    /// `Money::to_provider_string` — the single conversion point
    /// (`docs/flows/money.md`). A decimal *string*, which is MTN's shape;
    /// Orange takes the same amount as a JSON number.
    amount: String,
    currency: &'static str,
    /// Our own reference, rendered.
    ///
    /// The flow doc calls this "the charge id", and the port hands an adapter
    /// no charge id: [`ChargeRef`] carries `reference_id` and nothing else
    /// that identifies the charge. `reference_id` is the durable identifier
    /// written before any network call (`docs/flows/crash-safety.md`), so it
    /// is what goes here — and it is what makes `parse_callback` able to
    /// recover a reference from a body MTN echoes back.
    #[serde(rename = "externalId")]
    external_id: String,
    payer: Payer,
}

#[derive(Debug, Serialize)]
struct Payer {
    /// Always `MSISDN` on Cameroon collections: the payer is identified by
    /// their phone number.
    #[serde(rename = "partyIdType")]
    party_id_type: &'static str,
    #[serde(rename = "partyId")]
    party_id: String,
}

impl RequestToPay {
    /// # Errors
    ///
    /// [`ProviderError::Config`] when the charge has no `payer_ref`. A push
    /// rail prompts a payer's own handset, so there is nothing to prompt
    /// without one — and answering anything else would let a charge be
    /// "submitted" to nobody.
    pub(crate) fn new(charge: &ChargeRef) -> Result<Self, ProviderError> {
        let party_id = charge.payer_ref.clone().ok_or_else(|| {
            ProviderError::Config("mtn_momo: payer_ref required on a push rail".to_owned())
        })?;
        Ok(Self {
            amount: charge.amount.to_provider_string(),
            // The amount's own currency, not `ProviderConfig::currency`: the
            // amount is what is being charged, and a disagreement between the
            // two is a core-level bug that must reach the rail as the
            // `INVALID_CURRENCY` it is rather than being silently rewritten
            // here.
            currency: charge.amount.currency().code(),
            external_id: charge.reference_id.to_string(),
            payer: Payer {
                party_id_type: "MSISDN",
                party_id,
            },
        })
    }
}

/// `GET /collection/v1_0/requesttopay/{ref}`.
#[derive(Debug, Deserialize)]
pub(crate) struct StatusResponse {
    /// `PENDING` | `SUCCESSFUL` | `FAILED`.
    pub(crate) status: String,
    /// Present on `FAILED`. MTN sends it either as a bare string or as an
    /// object with a `code`; both appear in its documentation.
    #[serde(default)]
    pub(crate) reason: Option<Reason>,
    /// MTN's own transaction id, present once the charge settles.
    #[serde(default, rename = "financialTransactionId")]
    pub(crate) financial_transaction_id: Option<Scalar>,
}

/// The two shapes of `reason`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Reason {
    Text(String),
    Structured {
        code: String,
        #[serde(default)]
        message: Option<String>,
    },
}

impl Reason {
    /// The string to look up in [`crate::mapping::FAILURE_REASONS`].
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Structured { code, .. } => code,
        }
    }

    /// The rail's own words, for an operator. Carried into
    /// [`vpay_provider::ChargeStatus::Failed`]'s `raw`, never shown to a
    /// merchant (`docs/flows/failures.md`).
    pub(crate) fn raw(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Structured { code, message } => match message {
                Some(message) => format!("{code}: {message}"),
                None => code.clone(),
            },
        }
    }
}

/// A JSON value that ought to be a string but might arrive as a number.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Scalar {
    Text(String),
    Number(i64),
}

impl Scalar {
    pub(crate) fn into_string(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Number(number) => number.to_string(),
        }
    }
}

/// MTN's error envelope, shared by its 400s and by the 500s that are really
/// logical errors.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ApiError {
    #[serde(default)]
    pub(crate) code: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

impl ApiError {
    /// Parses an error body, tolerating one that is not JSON at all — MTN's
    /// gateway answers some 500s with plain text, and that difference is
    /// load-bearing (`docs/flows/adapter-mtn-momo.md`).
    pub(crate) fn parse(body: &str) -> Self {
        serde_json::from_str(body).unwrap_or_default()
    }
}

/// What MTN POSTs to `X-Callback-Url`: the status body again.
///
/// Only the identifiers are read. The `status` field is present in the body
/// and deliberately absent from this struct — "callbacks are hints", and an
/// adapter that could read a status off an unauthenticated request is an
/// adapter that could be talked into settling a charge by anyone who can
/// reach the callback URL.
#[derive(Debug, Deserialize)]
pub(crate) struct CallbackBody {
    #[serde(default, rename = "referenceId")]
    reference_id: Option<String>,
    #[serde(default, rename = "externalId")]
    external_id: Option<String>,
}

impl CallbackBody {
    /// Recovers the reference this notification is about.
    ///
    /// MTN's notification does not reliably carry the `X-Reference-Id` we
    /// submitted under: some environments echo it as `referenceId`, and the
    /// only field guaranteed to be there is the `externalId` *we* set. Since
    /// [`RequestToPay`] sets `externalId` to `reference_id` (the port hands
    /// an adapter no other identifier — see that field's comment), both paths
    /// arrive at the same UUID, and `referenceId` is preferred because it is
    /// the rail's own copy of the reference rather than ours.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`] when neither field is a UUID. Failing
    /// closed is the only safe answer: a callback whose charge cannot be
    /// named is a callback that must be dropped, not guessed at.
    pub(crate) fn reference(&self) -> Result<Uuid, ProviderError> {
        self.reference_id
            .as_deref()
            .and_then(|v| Uuid::parse_str(v.trim()).ok())
            .or_else(|| {
                self.external_id
                    .as_deref()
                    .and_then(|v| Uuid::parse_str(v.trim()).ok())
            })
            .ok_or_else(|| {
                ProviderError::malformed(
                    "mtn_momo: callback carries neither a referenceId nor an externalId that is \
                     one of our references"
                        .to_owned(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vpay_core::{Currency, Money};

    use super::*;

    fn charge(payer: Option<&str>) -> ChargeRef {
        ChargeRef {
            reference_id: Uuid::from_u128(0x0202),
            amount: Money::new(5_000, Currency::Eur).expect("non-negative"),
            payer_ref: payer.map(ToOwned::to_owned),
            ref_extra: BTreeMap::new(),
        }
    }

    /// The body in `docs/flows/adapter-mtn-momo.md`, field for field. A
    /// renamed field here is a charge the rail refuses, and nothing else in
    /// the workspace would notice.
    #[test]
    fn the_request_body_has_the_documented_shape() {
        let body = RequestToPay::new(&charge(Some("237600000000"))).expect("a payer is present");
        let json: serde_json::Value = serde_json::to_value(&body).expect("serialises");

        assert_eq!(
            json,
            serde_json::json!({
                // EUR is two-decimal: 5000 minor units is "50.00", and this
                // is the assertion that would fail if someone sent minor
                // units to a rail that wants a decimal string.
                "amount": "50.00",
                "currency": "EUR",
                "externalId": "00000000-0000-0000-0000-000000000202",
                "payer": { "partyIdType": "MSISDN", "partyId": "237600000000" },
            })
        );
    }

    #[test]
    fn a_push_charge_without_a_payer_is_a_configuration_error() {
        assert!(matches!(
            RequestToPay::new(&charge(None)),
            Err(ProviderError::Config(_))
        ));
    }

    #[test]
    fn a_failure_reason_parses_in_both_of_mtns_shapes() {
        let text: StatusResponse =
            serde_json::from_str(r#"{"status":"FAILED","reason":"NOT_ENOUGH_FUNDS"}"#)
                .expect("bare-string reason");
        let structured: StatusResponse = serde_json::from_str(
            r#"{"status":"FAILED","reason":{"code":"NOT_ENOUGH_FUNDS","message":"no funds"}}"#,
        )
        .expect("object reason");

        assert_eq!(
            text.reason.as_ref().map(Reason::code),
            Some("NOT_ENOUGH_FUNDS")
        );
        assert_eq!(
            structured.reason.as_ref().map(Reason::code),
            Some("NOT_ENOUGH_FUNDS")
        );
        assert_eq!(
            structured.reason.as_ref().map(Reason::raw).as_deref(),
            Some("NOT_ENOUGH_FUNDS: no funds")
        );
    }

    #[test]
    fn a_transaction_id_parses_whether_or_not_the_rail_quotes_it() {
        for body in [
            r#"{"status":"SUCCESSFUL","financialTransactionId":"1234567890"}"#,
            r#"{"status":"SUCCESSFUL","financialTransactionId":1234567890}"#,
        ] {
            let parsed: StatusResponse = serde_json::from_str(body).expect("parses");
            assert_eq!(
                parsed.financial_transaction_id.map(Scalar::into_string),
                Some("1234567890".to_owned()),
                "{body}"
            );
        }
    }

    #[test]
    fn an_error_body_that_is_not_json_is_still_an_error_body() {
        assert_eq!(ApiError::parse("<html>502 Bad Gateway</html>").code, None);
        assert_eq!(
            ApiError::parse(r#"{"code":"PAYER_NOT_FOUND","message":"nope"}"#).code,
            Some("PAYER_NOT_FOUND".to_owned())
        );
    }

    #[test]
    fn a_callback_reference_comes_from_either_field_and_prefers_the_rails() {
        let ours = Uuid::from_u128(0x0202);
        let theirs = Uuid::from_u128(0x0303);

        let external_only: CallbackBody =
            serde_json::from_str(&format!(r#"{{"externalId":"{ours}"}}"#)).expect("parses");
        assert_eq!(external_only.reference().expect("a reference"), ours);

        let both: CallbackBody = serde_json::from_str(&format!(
            r#"{{"referenceId":"{theirs}","externalId":"{ours}"}}"#
        ))
        .expect("parses");
        assert_eq!(both.reference().expect("a reference"), theirs);
    }

    #[test]
    fn a_callback_we_cannot_name_a_charge_from_is_refused() {
        for body in [
            r#"{}"#,
            r#"{"externalId":"not-a-uuid"}"#,
            r#"{"referenceId":"","externalId":""}"#,
        ] {
            let parsed: CallbackBody = serde_json::from_str(body).expect("parses");
            assert!(
                matches!(parsed.reference(), Err(ProviderError::Malformed { .. })),
                "{body}"
            );
        }
    }
}
