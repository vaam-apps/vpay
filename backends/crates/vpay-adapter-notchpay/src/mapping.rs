//! What a Notch Pay answer means for a charge: RFC-0012 §§ 3 and 4, as code.
//!
//! **Written 2026-10-20 from Notch Pay's documentation and `openapi.yaml`
//! 2.1.0; partly confirmed against Notch Pay's sandbox on 2026-10-21**
//! (`docs/status/verification/2026-10-21-notchpay-sandbox-run.md`, delivery
//! step 1, through `vpay-notchpay`'s probe and not through this adapter).
//! Confirmed: `complete`, `failed`, `expired` and `canceled` are what the
//! sandbox's outcome numbers read, with no failure reason (item 5), and a
//! repeated reference is a `409` (item 4). Changed on that evidence: a
//! success needs `amounts.total` to be the charge's, `canceled` is
//! `payer_declined`, and a success with no `id` records `reference`. The
//! path takes Notch Pay's `trx.…` reference, not ours (item 2), so the
//! adapter addresses process and read by it (`lib.rs`). `processing`, `incomplete`, `refunded`,
//! `partialy-refunded`, `rejected` and `abandoned` were not observed, and
//! `docs/flows/adapter-notchpay.md` § "To confirm" lists what is open.
//!
//! Confirmed through this adapter on 2026-10-21 (delivery step 5,
//! `docs/status/verification/2026-10-21-notchpay-step5.md`): `complete` with
//! `amounts.total` equal to the amount is `Succeeded`; `failed` is
//! `Failed { provider_error }`; and the "insufficient funds" number answered
//! `pending` on its first read, then a `complete` whose `amounts.total` was
//! short, which was `Malformed` on each of the next eleven reads. Since
//! 2026-10-23 that answer is `NeedsReconciliation` and the worker escalates
//! the charge on the first one (vaam-apps/vpay#313).
//!
//! The split with `vpay-notchpay` is the one the other two adapters draw
//! between `wire.rs` and `mapping.rs`, at a crate boundary: the client reports
//! what Notch Pay said, as `Answer::Success` or `Answer::Refused`, and this
//! module is the only place that decides what a status, a `409` or a `404`
//! means for money.

use vpay_core::{FailureCode, Money};
use vpay_notchpay::Refusal;
use vpay_notchpay::status::PaymentStatus;
use vpay_notchpay::wire::Payment;
use vpay_provider::{ChargeStatus, ProviderError};

/// Every [`FailureCode`] this adapter can produce, and nothing else.
///
/// `payer_timeout` (`expired`, `abandoned`), `payer_declined` (`canceled`),
/// `provider_error` (`failed`, `rejected`, a refused initialize or process),
/// `provider_account_blocked` (a `401`/`403` on our own keys) and
/// `provider_unavailable` (`prepare`'s `409` for a reference no submit was
/// ever made for: an initialize that lost its answer and prompted nobody,
/// ADR-0032). The last is produced in `lib.rs`, not by this module's
/// mapping, so the test below adds it by hand, as it adds
/// `provider_account_blocked`. The conformance suite's `the_declines_prove_every_code_each_rail_can_produce`
/// compares this list against the declines it drives, in both directions.
///
/// **Short on purpose.** Notch Pay's read names no failure reason: on
/// 2026-10-21 the sandbox's "failure (other reason)" number read `failed`
/// and nothing else, and its "insufficient funds" number did not fail at
/// all (it read `complete` for less than was asked; see
/// `require_the_charges_amount`). So nothing maps to `insufficient_funds`.
/// Its "canceled by user" number read `canceled`, which is the evidence
/// RFC-0012 § 4 named for `payer_declined`; its "timeout" number read
/// `expired`, which was already `payer_timeout`.
pub const PRODUCED_FAILURE_CODES: [FailureCode; 5] = [
    FailureCode::PayerTimeout,
    FailureCode::PayerDeclined,
    FailureCode::ProviderError,
    FailureCode::ProviderAccountBlocked,
    FailureCode::ProviderUnavailable,
];

/// The message Notch Pay's errors page gives a read of a payment it has no
/// record of. Compared without case and without a trailing full stop.
const PAYMENT_NOT_FOUND: &str = "Payment not found";

/// The message Notch Pay's errors page gives a process call on a payment that
/// already left `pending`. Compared as a case-insensitive substring, because
/// the page shows it inside longer prose in one place.
const ALREADY_PROCESSED: &str = "already been processed";

/// The longest piece of Notch Pay's own prose that is carried into a message.
const PROSE_LIMIT: usize = 200;

/// What the read says about a charge — RFC-0012 § 4's table, row by row.
///
/// | Notch Pay `status` | `ChargeStatus` |
/// | --- | --- |
/// | `pending`, `processing`, `incomplete` | `Pending` — `incomplete` is never a success |
/// | `complete`, `refunded`, `partialy-refunded` | `Succeeded`, **only if** the amount, the collected `amounts.total` and the currency are the charge's; otherwise `NeedsReconciliation` |
/// | `canceled` | `Failed { payer_declined }`, with the status as `raw` |
/// | `failed`, `rejected` | `Failed { provider_error }`, with the status as `raw` |
/// | `expired`, `abandoned` | `Failed { payer_timeout }` |
/// | anything else, or none | `Malformed`, naming it |
///
/// `canceled` is `payer_declined`, since Notch Pay's sandbox read its
/// "canceled by user" test number (`+237670000004`) as `canceled` on
/// 2026-10-21: the evidence RFC-0012 § 4 named for this row. It was
/// `provider_error` before. The caveat stays: Notch Pay says a payment is
/// canceled "by the merchant or customer", and the public key can cancel one
/// (ADR-0031), so a `canceled` read is evidence of a decline, not proof of
/// whose. Either way the charge fails and no money moves.
///
/// # Errors
///
/// [`ProviderError::Malformed`] for a status Notch Pay does not document, a
/// missing one, or a success whose amount or currency is not the charge's.
pub(crate) fn charge_status(
    payment: &Payment,
    expected: Money,
) -> Result<ChargeStatus, ProviderError> {
    let Some(raw) = payment.status.as_deref() else {
        return Err(ProviderError::malformed(
            "notchpay: the payment read carries no status",
        ));
    };
    let Some(status) = PaymentStatus::parse(raw) else {
        return Err(ProviderError::malformed(format!(
            "notchpay: the payment read reports a status Notch Pay does not document: {}",
            truncated(raw)
        )));
    };
    Ok(match status {
        PaymentStatus::Pending | PaymentStatus::Processing | PaymentStatus::Incomplete => {
            ChargeStatus::Pending
        }
        PaymentStatus::Complete | PaymentStatus::Refunded | PaymentStatus::PartiallyRefunded => {
            require_the_charges_amount(payment, expected, status)?;
            ChargeStatus::Succeeded {
                provider_txn_id: provider_txn_id(payment),
            }
        }
        PaymentStatus::Canceled => ChargeStatus::Failed {
            code: FailureCode::PayerDeclined,
            raw: status.wire().to_owned(),
        },
        PaymentStatus::Failed | PaymentStatus::Rejected => ChargeStatus::Failed {
            code: FailureCode::ProviderError,
            raw: status.wire().to_owned(),
        },
        PaymentStatus::Expired | PaymentStatus::Abandoned => ChargeStatus::Failed {
            code: FailureCode::PayerTimeout,
            raw: status.wire().to_owned(),
        },
    })
}

/// The payment's identifier on Notch Pay, for `provider_txn_id`.
///
/// `id` where the answer has one, as `openapi.yaml` 2.1.0 shapes it. Notch
/// Pay's sandbox sent no `id` on any payment answer on 2026-10-21; its
/// identifier was `reference` (`trx.…`), the value its own paths take
/// (RFC-0012 § 10 item 2). So `reference` is the fallback.
fn provider_txn_id(payment: &Payment) -> Option<String> {
    payment.id.clone().or_else(|| payment.reference.clone())
}

/// Refuses a success for any money but the charge's.
///
/// The read's integrity rests on TLS, and on this check (ADR-0031 condition
/// 5): a third party who holds the public key cannot forge what Notch Pay
/// answers, but a mapping that took any `complete` as this charge's would
/// settle a charge on another payment's word. A float amount is refused
/// rather than rounded (`vpay_notchpay::wire::Payment::amount_minor`).
///
/// **`amounts.total` must be the charge's amount too.** On 2026-10-21
/// Notch Pay's sandbox read its "insufficient funds" test number as
/// `complete`, with `amount` 107 (what was asked) and `amounts.total` 105.
/// On every other answer that day `total` equalled `amount`. A `complete`
/// that collected less than the charge is not this charge's success, so it
/// is refused, as a wrong `amount` is. A success with no `amounts`, or
/// no integer `total`, is refused on the same terms: every sandbox answer
/// carried one (`docs/status/verification/2026-10-21-notchpay-sandbox-run.md`).
///
/// **All three refusals are [`ProviderError::NeedsReconciliation`]**, not
/// `Malformed` (vaam-apps/vpay#313, 2026-10-23). Each is an answer the
/// adapter read perfectly, terminal on Notch Pay's side: a payment's amount,
/// currency and collected total do not change on a second read. As
/// `Malformed` each rode the poll ladder at `Warn` for 24 hours before
/// anyone was told; now the worker makes the charge `unresolved`, alerting,
/// on the first read that says so. Including the two besides the short
/// total was the implementer's call, recorded in the pull request.
fn require_the_charges_amount(
    payment: &Payment,
    expected: Money,
    status: PaymentStatus,
) -> Result<(), ProviderError> {
    let amount_matches = payment.amount_minor() == Some(expected.minor());
    let currency_matches = payment.currency.as_deref() == Some(expected.currency().code());
    if !(amount_matches && currency_matches) {
        return Err(ProviderError::needs_reconciliation(format!(
            "notchpay: the read reports {} for an amount or currency that is not this charge's \
             (amount {}, currency {}); never a success",
            status.wire(),
            if amount_matches { "matches" } else { "differs" },
            if currency_matches {
                "matches"
            } else {
                "differs"
            },
        )));
    }
    // Added to the check above, not folded into it: the two failures mean
    // different things to an operator.
    match payment
        .amounts
        .as_ref()
        .and_then(vpay_notchpay::wire::Amounts::total_minor)
    {
        Some(total) if total == expected.minor() => Ok(()),
        Some(_) => Err(ProviderError::needs_reconciliation(format!(
            "notchpay: the read reports {} for this charge's amount, but amounts.total is not \
             that amount: Notch Pay moved money, and not this charge's amount. Never a success; \
             the charge needs an operator to reconcile it with Notch Pay",
            status.wire(),
        ))),
        None => Err(ProviderError::needs_reconciliation(format!(
            "notchpay: the read reports {} with no integer amounts.total, so what was collected \
             is unknown; never a success",
            status.wire(),
        ))),
    }
}

/// What a refused read means.
///
/// | Notch Pay answered | Result |
/// | --- | --- |
/// | `404` "Payment not found" | `Ok(())`: the caller's `ChargeStatus::NotFound`, never on its own a failure (`docs/flows/crash-safety.md`) |
/// | any other `404` | `Config` — an unknown Sync account, or a base URL with a wrong path |
/// | `401`, `403` | `Rejected { provider_account_blocked }` — our own keys; it pages |
/// | anything else | `Malformed` |
///
/// # Errors
///
/// As in the table.
pub(crate) fn read_refusal(refusal: &Refusal) -> Result<(), ProviderError> {
    match refusal.http_status {
        404 if is_payment_not_found(refusal) => Ok(()),
        404 => Err(not_found_elsewhere("reading a payment")),
        401 | 403 => Err(keys_refused(refusal, "reading a payment")),
        other => Err(ProviderError::malformed(format!(
            "notchpay: reading a payment answered an undocumented {other}"
        ))),
    }
}

/// What a refused initialize means.
#[derive(Debug)]
pub(crate) enum Initialized {
    /// `409`: a payment with this reference already exists. The crash-retry
    /// case — RFC-0012 § 3 reads it and decides from there.
    AlreadyExists,
    /// Anything else, already mapped.
    Refused(ProviderError),
}

/// The initialize call's refusals, RFC-0012 § 3's first table.
///
/// | Notch Pay answered | Result |
/// | --- | --- |
/// | `409` | [`Initialized::AlreadyExists`] — documented on the errors page, absent from the specification, and confirmed by the sandbox on 2026-10-21 ("Reference already existing", item 4) |
/// | `400`, `422` | `Rejected { provider_error }` with Notch Pay's message |
/// | `401`, `403` | `Rejected { provider_account_blocked }` |
/// | `404` | `Config` |
/// | anything else | `Malformed` |
pub(crate) fn initialize_refusal(refusal: &Refusal) -> Initialized {
    Initialized::Refused(match refusal.http_status {
        409 => return Initialized::AlreadyExists,
        400 | 422 => declined(refusal, "initializing a payment"),
        401 | 403 => keys_refused(refusal, "initializing a payment"),
        404 => not_found_elsewhere("initializing a payment"),
        other => ProviderError::malformed(format!(
            "notchpay: initializing a payment answered an undocumented {other}"
        )),
    })
}

/// What a refused process means.
#[derive(Debug)]
pub(crate) enum Processed {
    /// `422` "Payment has already been processed": not a decline. The
    /// adapter reads the payment and decides from there (RFC-0012 § 3).
    AlreadyProcessed,
    /// Anything else, already mapped.
    Refused(ProviderError),
}

/// The process call's refusals.
///
/// | Notch Pay answered | Result |
/// | --- | --- |
/// | `422` "…already been processed" | [`Processed::AlreadyProcessed`] |
/// | other `400`, `422` | `Rejected { provider_error }` with Notch Pay's message |
/// | `401`, `403` | `Rejected { provider_account_blocked }` |
/// | `404` "Payment not found" | `Malformed`: the initialize just answered, so this contradicts it |
/// | any other `404` | `Config` |
/// | anything else | `Malformed` |
pub(crate) fn process_refusal(refusal: &Refusal) -> Processed {
    Processed::Refused(match refusal.http_status {
        422 if mentions(refusal, ALREADY_PROCESSED) => return Processed::AlreadyProcessed,
        400 | 422 => declined(refusal, "processing a payment"),
        401 | 403 => keys_refused(refusal, "processing a payment"),
        404 if is_payment_not_found(refusal) => ProviderError::malformed(
            "notchpay: processing a payment it had just initialized answered \"Payment not \
             found\"",
        ),
        404 => not_found_elsewhere("processing a payment"),
        other => ProviderError::malformed(format!(
            "notchpay: processing a payment answered an undocumented {other}"
        )),
    })
}

/// Notch Pay's own "Payment not found", and nothing that merely shares the
/// status code.
fn is_payment_not_found(refusal: &Refusal) -> bool {
    refusal.message.as_deref().is_some_and(|message| {
        message
            .trim()
            .trim_end_matches('.')
            .eq_ignore_ascii_case(PAYMENT_NOT_FOUND)
    })
}

fn mentions(refusal: &Refusal, needle: &str) -> bool {
    refusal
        .message
        .as_deref()
        .is_some_and(|message| message.to_ascii_lowercase().contains(needle))
}

/// A `404` that is not "Payment not found": the deployment's own mistake.
///
/// Both other adapters read a `404` on submit this way. Here it matters more,
/// because the read's `404` is also how a payment that does not exist is
/// reported, and reading an unknown Sync account or a wrong base path as "no
/// such payment" would resubmit a payment that may be alive.
fn not_found_elsewhere(doing: &str) -> ProviderError {
    ProviderError::Config(format!(
        "notchpay: {doing} answered 404 without Notch Pay's \"Payment not found\"; the base URL \
         or the account is wrong"
    ))
}

/// Our own keys, refused. It pages (`docs/flows/failures.md`), because every
/// charge on the rail is failing and no payer can fix it.
pub(crate) fn keys_refused(refusal: &Refusal, doing: &str) -> ProviderError {
    ProviderError::Rejected {
        code: FailureCode::ProviderAccountBlocked,
        message: format!(
            "notchpay: {doing} answered {}: the account's keys were refused",
            refusal.http_status
        ),
    }
}

/// A validation refusal, carried as `provider_error` with Notch Pay's own
/// words for an operator, bounded.
///
/// Not mapped any finer. A `422` on the phone could be called `invalid_payer`,
/// but the errors page names fields only by example, and a wrong guess would
/// tell a merchant the payer mistyped when the deployment sent a bad amount.
/// The names of the fields Notch Pay complained about are carried, never what
/// it said about them, because a validation message can quote the value.
fn declined(refusal: &Refusal, doing: &str) -> ProviderError {
    let fields = refusal
        .errors
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    ProviderError::Rejected {
        code: FailureCode::ProviderError,
        message: format!(
            "notchpay: {doing} answered {}: {}{}",
            refusal.http_status,
            refusal
                .message
                .as_deref()
                .map_or_else(|| "no message".to_owned(), truncated),
            if fields.is_empty() {
                String::new()
            } else {
                format!(" (fields: {})", truncated(&fields))
            },
        ),
    }
}

/// Bounds a rail-supplied string before it reaches a message.
pub(crate) fn truncated(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= PROSE_LIMIT {
        return trimmed.to_owned();
    }
    trimmed
        .chars()
        .take(PROSE_LIMIT)
        .chain("…".chars())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vpay_core::Currency;

    use super::*;

    fn xaf(amount: i64) -> Money {
        Money::new(amount, Currency::Xaf).expect("non-negative")
    }

    /// A read as `openapi.yaml` 2.1.0's `Payment` schema shapes it, decoded
    /// through the client's own type, so the test cannot drift from the wire.
    fn payment(status: &str, amount: serde_json::Value, currency: &str) -> Payment {
        serde_json::from_value(serde_json::json!({
            "id": "pay.test_0001",
            "reference": "00000000-0000-0000-0000-000000000202",
            "amount": amount,
            "amounts": { "converted": amount, "currency": currency, "rate": 1, "total": amount },
            "currency": currency,
            "status": status,
        }))
        .expect("the client's Payment decodes the documented shape")
    }

    fn refusal(http_status: u16, message: Option<&str>) -> Refusal {
        Refusal {
            http_status,
            message: message.map(str::to_owned),
            errors: BTreeMap::new(),
        }
    }

    /// RFC-0012 § 4, every row, including the five the Payments API page does
    /// not list and the Collect page does. The table is the RFC's; the
    /// statuses come from the client's own enumeration, so a status added
    /// there without a row here fails `every_status_the_client_names_has_a_row`.
    #[test]
    fn every_documented_status_maps_as_rfc_0012_says() {
        let expected = xaf(5_000);
        let succeeded = ChargeStatus::Succeeded {
            provider_txn_id: Some("pay.test_0001".to_owned()),
        };
        let failed = |code, raw: &str| ChargeStatus::Failed {
            code,
            raw: raw.to_owned(),
        };
        let table = [
            ("pending", ChargeStatus::Pending),
            ("processing", ChargeStatus::Pending),
            ("incomplete", ChargeStatus::Pending),
            ("complete", succeeded.clone()),
            ("refunded", succeeded.clone()),
            ("partialy-refunded", succeeded),
            ("failed", failed(FailureCode::ProviderError, "failed")),
            ("canceled", failed(FailureCode::PayerDeclined, "canceled")),
            ("rejected", failed(FailureCode::ProviderError, "rejected")),
            ("expired", failed(FailureCode::PayerTimeout, "expired")),
            ("abandoned", failed(FailureCode::PayerTimeout, "abandoned")),
        ];
        for (status, want) in table {
            let got = charge_status(&payment(status, 5_000.into(), "XAF"), expected)
                .unwrap_or_else(|error| panic!("{status}: {error}"));
            assert_eq!(got, want, "{status}");
        }
    }

    #[test]
    fn every_status_the_client_names_has_a_row() {
        for status in PaymentStatus::ALL {
            assert!(
                charge_status(&payment(status.wire(), 5_000.into(), "XAF"), xaf(5_000)).is_ok(),
                "{} has no row in the mapping",
                status.wire()
            );
        }
    }

    /// `incomplete` is "Customer pay less than payment amount". It is the
    /// status most tempting to read as paid, and it must never be.
    #[test]
    fn an_incomplete_payment_is_never_a_success() {
        let status = charge_status(&payment("incomplete", 5_000.into(), "XAF"), xaf(5_000));
        assert_eq!(status.expect("documented"), ChargeStatus::Pending);
    }

    /// ADR-0031 condition 5. Each of the three ways the money can disagree,
    /// on each of the three success statuses.
    #[test]
    fn a_success_for_another_amount_or_currency_is_never_a_success() {
        for status in ["complete", "refunded", "partialy-refunded"] {
            for (amount, currency, what) in [
                (serde_json::json!(4_999), "XAF", "a different amount"),
                (serde_json::json!(5_000), "XOF", "a different currency"),
                (
                    serde_json::json!(5_000.0),
                    "XAF",
                    "an amount written as a float",
                ),
                (serde_json::Value::Null, "XAF", "no amount"),
            ] {
                let outcome = charge_status(&payment(status, amount, currency), xaf(5_000));
                assert!(
                    matches!(outcome, Err(ProviderError::NeedsReconciliation { .. })),
                    "{status} with {what} must be NeedsReconciliation: a second read cannot \
                     change a payment's amount, got {outcome:?}"
                );
            }
        }
    }

    /// Notch Pay's sandbox, 2026-10-21: the "insufficient funds" test number
    /// (`+237670000001`) read `complete` with `amount` 107 and
    /// `amounts.total` 105. Transcribed from that read, minus identifiers.
    /// Before this check it mapped to `Succeeded`.
    #[test]
    fn a_complete_that_collected_less_than_the_charge_is_never_a_success() {
        let short: Payment = serde_json::from_value(serde_json::json!({
            "amount": 107,
            "amounts": { "converted": 107, "currency": "XAF", "rate": 1, "total": 105 },
            "charge": "business",
            "currency": "XAF",
            "fees": [],
            "reference": "trx.test_0001",
            "sandbox": 1,
            "status": "complete",
            "trxref": "00000000-0000-0000-0000-000000000202",
        }))
        .expect("the sandbox's shape decodes");
        let outcome = charge_status(&short, xaf(107));
        assert!(
            matches!(outcome, Err(ProviderError::NeedsReconciliation { .. })),
            "a complete for 105 of 107 must be NeedsReconciliation, which the worker escalates \
             at once; as Malformed it rode the ladder for 24 h (#313). Got {outcome:?}"
        );
        let message = outcome.expect_err("refused").to_string();
        assert!(
            message.contains("moved money") && message.contains("operator"),
            "the error must say money moved and an operator is needed: {message}"
        );
    }

    #[test]
    fn a_success_with_no_collected_total_is_never_a_success() {
        for amounts in [
            serde_json::Value::Null,
            serde_json::json!({ "total": null }),
            serde_json::json!({ "total": 5_000.0 }),
        ] {
            let payment: Payment = serde_json::from_value(serde_json::json!({
                "reference": "trx.test_0001",
                "amount": 5_000,
                "amounts": amounts,
                "currency": "XAF",
                "status": "complete",
            }))
            .expect("decodes");
            let outcome = charge_status(&payment, xaf(5_000));
            assert!(
                matches!(outcome, Err(ProviderError::NeedsReconciliation { .. })),
                "amounts {amounts} must be NeedsReconciliation, got {outcome:?}"
            );
        }
    }

    /// The sandbox's payment answers carry no `id`: `reference` (`trx.…`) is
    /// the payment's identifier, and is what a success records.
    #[test]
    fn a_success_with_no_id_records_notch_pays_reference() {
        let payment: Payment = serde_json::from_value(serde_json::json!({
            "reference": "trx.test_0001",
            "trxref": "00000000-0000-0000-0000-000000000202",
            "amount": 5_000,
            "amounts": { "total": 5_000 },
            "currency": "XAF",
            "status": "complete",
        }))
        .expect("decodes");
        assert_eq!(
            charge_status(&payment, xaf(5_000)).expect("a success"),
            ChargeStatus::Succeeded {
                provider_txn_id: Some("trx.test_0001".to_owned())
            }
        );
    }

    #[test]
    fn a_status_notch_pay_never_documented_is_not_guessed_at() {
        for status in ["succeeded", "Complete", "partially-refunded", "", "paid"] {
            let outcome = charge_status(&payment(status, 5_000.into(), "XAF"), xaf(5_000));
            assert!(
                matches!(outcome, Err(ProviderError::Malformed { .. })),
                "{status:?} must be Malformed, got {outcome:?}"
            );
        }
        let none: Payment =
            serde_json::from_value(serde_json::json!({ "id": "pay.test_0001" })).expect("decodes");
        assert!(matches!(
            charge_status(&none, xaf(5_000)),
            Err(ProviderError::Malformed { .. })
        ));
    }

    /// RFC-0012 § 3: only Notch Pay's own words are `NotFound`. A `404` from a
    /// wrong base path, or "Sync account not found", is the deployment's
    /// mistake — reading it as "no such payment" would resubmit a live one.
    #[test]
    fn only_notch_pays_own_not_found_is_not_found() {
        for message in [
            "Payment not found",
            "payment not found.",
            " Payment Not Found ",
        ] {
            assert!(
                read_refusal(&refusal(404, Some(message))).is_ok(),
                "{message:?} is Notch Pay's own \"Payment not found\""
            );
        }
        for message in [Some("Sync account not found"), Some("Not Found"), None] {
            let outcome = read_refusal(&refusal(404, message));
            assert!(
                matches!(outcome, Err(ProviderError::Config(_))),
                "{message:?} must be Config, got {outcome:?}"
            );
        }
    }

    #[test]
    fn refused_keys_page_on_every_call_and_are_never_a_payers_problem() {
        use vpay_core::{Classify as _, Severity};
        for status in [401, 403] {
            let errors = [
                read_refusal(&refusal(status, Some("Unauthenticated"))).expect_err("refused"),
                match initialize_refusal(&refusal(status, None)) {
                    Initialized::Refused(error) => error,
                    Initialized::AlreadyExists => panic!("not a duplicate"),
                },
                match process_refusal(&refusal(status, None)) {
                    Processed::Refused(error) => error,
                    Processed::AlreadyProcessed => panic!("not already processed"),
                },
            ];
            for error in errors {
                assert!(
                    matches!(
                        error,
                        ProviderError::Rejected {
                            code: FailureCode::ProviderAccountBlocked,
                            ..
                        }
                    ),
                    "{status}: {error:?}"
                );
                assert_eq!(error.severity(), Severity::Page);
            }
        }
    }

    #[test]
    fn a_duplicate_reference_on_initialize_is_the_crash_retry_case_not_an_error() {
        let refused = refusal(409, Some("A payment with this reference already exists"));
        assert!(matches!(
            initialize_refusal(&refused),
            Initialized::AlreadyExists
        ));
    }

    #[test]
    fn an_already_processed_payment_is_not_a_decline() {
        let refused = refusal(422, Some("Payment has already been processed"));
        assert!(matches!(
            process_refusal(&refused),
            Processed::AlreadyProcessed
        ));
        let invalid = refusal(422, Some("The given data was invalid."));
        assert!(matches!(
            process_refusal(&invalid),
            Processed::Refused(ProviderError::Rejected {
                code: FailureCode::ProviderError,
                ..
            })
        ));
    }

    /// A validation message can quote the value it refused. The fields are
    /// named; what Notch Pay said about each one is not carried.
    #[test]
    fn a_validation_refusal_names_fields_and_never_their_messages() {
        let refused = Refusal {
            http_status: 422,
            message: Some("The given data was invalid.".to_owned()),
            errors: BTreeMap::from([(
                "phone".to_owned(),
                vec!["The phone +237670000000 is not valid".to_owned()],
            )]),
        };
        let Initialized::Refused(error) = initialize_refusal(&refused) else {
            panic!("a 422 is not a duplicate");
        };
        let rendered = error.to_string();
        assert!(rendered.contains("phone"), "{rendered}");
        assert!(!rendered.contains("237670000000"), "{rendered}");
    }

    #[test]
    fn a_rails_prose_is_bounded_before_it_reaches_a_message() {
        let long = "x".repeat(10_000);
        let Initialized::Refused(error) = initialize_refusal(&refusal(422, Some(&long))) else {
            panic!("a 422 is not a duplicate");
        };
        assert!(error.to_string().chars().count() < 400);
    }

    #[test]
    fn the_declared_codes_are_exactly_what_the_mapping_can_produce() {
        // Neither comes from this module: `provider_account_blocked` is the
        // client's `401`/`403`, and `provider_unavailable` is `prepare`'s
        // `409` with no prior submit, which
        // `tests::a_duplicate_no_submit_ever_prompted_for_fails_the_charge_cleanly`
        // drives.
        let mut produced = vec![
            FailureCode::ProviderAccountBlocked,
            FailureCode::ProviderUnavailable,
        ];
        for status in PaymentStatus::ALL {
            if let Ok(ChargeStatus::Failed { code, .. }) =
                charge_status(&payment(status.wire(), 5_000.into(), "XAF"), xaf(5_000))
            {
                produced.push(code);
            }
        }
        if let Initialized::Refused(ProviderError::Rejected { code, .. }) =
            initialize_refusal(&refusal(422, None))
        {
            produced.push(code);
        }
        produced.sort_unstable_by_key(|code| code.as_str());
        produced.dedup();
        let mut declared = PRODUCED_FAILURE_CODES.to_vec();
        declared.sort_unstable_by_key(|code| code.as_str());
        assert_eq!(produced, declared);
    }
}
