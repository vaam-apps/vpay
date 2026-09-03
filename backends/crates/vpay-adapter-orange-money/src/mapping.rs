//! Turning Orange's two success bodies into the core's vocabulary.
//!
//! Everything here is pure: bytes in, a port type out. That is what makes it
//! unit-testable without a rail, and it is where ADR-0006 lands for an adapter
//! — this crate has no in-process HTTP double, so the parts worth asserting
//! case by case are the parts that never touch a socket. The transport half is
//! covered by the shared conformance suite against a real WireMock host.

use vpay_core::FailureCode;
use vpay_provider::{ChargeStatus, ProviderError, RefExtra, Submitted};

use crate::wire::{TransactionStatusResponse, WebPaymentResponse};

/// What one of Orange's documented `status` strings means to the core.
///
/// An enum rather than a `ChargeStatus` directly because `Succeeded` and
/// `Failed` both need a value from elsewhere in the body (`txnid`, the raw
/// reason), and a table that carried those would stop being a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Meaning {
    Pending,
    Succeeded,
    Failed(FailureCode),
}

/// `docs/flows/adapter-orange-money.md`'s "Status mapping" table, transcribed.
///
/// `INITIATED` is `Pending`, not a failure, and that is the row most likely to
/// be got wrong: it is the state a charge sits in when the merchant obtained a
/// `pay_token` and never redirected the payer. Failing it here would close a
/// charge the payer can still complete.
///
/// `FAILED` maps to [`FailureCode::ProviderError`] because Orange documents no
/// sub-reason vocabulary for it. That is a mapping gap to alert on, not a
/// resting place (`docs/flows/failures.md`); when the sub-reasons are learned
/// at onboarding they become rows here and nothing else changes.
pub(crate) const STATUS_TABLE: [(&str, Meaning); 5] = [
    ("INITIATED", Meaning::Pending),
    ("PENDING", Meaning::Pending),
    ("SUCCESS", Meaning::Succeeded),
    ("EXPIRED", Meaning::Failed(FailureCode::PayerTimeout)),
    ("FAILED", Meaning::Failed(FailureCode::ProviderError)),
];

/// Case-insensitive because a rail that starts answering `success` instead of
/// `SUCCESS` has not changed what it means, and losing a settled payment to a
/// capitalisation change is not a trade this system should take.
fn meaning(status: &str) -> Option<Meaning> {
    STATUS_TABLE
        .iter()
        .find(|(documented, _)| documented.eq_ignore_ascii_case(status))
        .map(|(_, meaning)| *meaning)
}

/// A `webpayment` 201 body as the value the core commits.
///
/// # Why `ref_extra` and `redirect_url` are built together
///
/// [`Submitted`] carries both or the call fails. A caller physically cannot
/// hold the redirect URL without the `pay_token` beside it, which is the
/// adapter's whole share of the commit-gates-redirect rule
/// (`docs/flows/crash-safety.md`); the rest — committing before answering — is
/// the `confirm` handler's transaction boundary and cannot be enforced here.
///
/// `notif_token` is carried when present and its absence is not an error: it
/// is used only to verify a later callback, and a callback is a hint. A submit
/// that succeeded is not going to be thrown away because the rail omitted the
/// weaker of two identifiers.
///
/// # Why the rail's URL is validated and not just carried
///
/// `payment_url` is the one value in this system that travels from a rail
/// straight into a payer's browser: it is persisted on `charges`, rendered
/// as `next_action.redirect_to_url.url`, and followed. Checking only that
/// it was non-empty — which is all this did until the Step 3 security
/// review — meant anything the rail (or anything answering as the rail, on
/// a plain-`http` sandbox host) put in that field became a URL a merchant's
/// checkout would send a payer to, `javascript:` and `data:` included. So
/// the scheme is required to be `http`/`https` and the length is bounded at
/// the same 2048 the `charges.redirect_url` CHECK enforces (migration
/// `0019`), which keeps the refusal a `Malformed` here rather than a `503`
/// from Postgres three statements later.
///
/// `http` is allowed alongside `https` because the conformance stub and
/// `compose.yml` serve the hosted page over plain HTTP; livemode's
/// https-only rule is `vpay_config`'s `validate_host`, which is where a
/// deployment-wide rule belongs.
///
/// # Errors
///
/// [`ProviderError::Malformed`] if the body is not JSON, lacks `pay_token`
/// or `payment_url`, or carries a `payment_url` that is not a bounded
/// `http(s)` URL. The body itself never reaches the message: it contains
/// the token that gates a payer's redirect.
pub(crate) fn submitted(body: &[u8]) -> Result<Submitted, ProviderError> {
    let parsed: WebPaymentResponse = serde_json::from_slice(body).map_err(|error| {
        ProviderError::malformed(format!(
            "orange_money: the webpayment response is not the documented JSON: {error}"
        ))
    })?;

    let pay_token = non_empty(parsed.pay_token).ok_or_else(|| {
        ProviderError::malformed(
            "orange_money: the webpayment response carries no pay_token".to_owned(),
        )
    })?;
    let payment_url = non_empty(parsed.payment_url).ok_or_else(|| {
        ProviderError::malformed(
            "orange_money: the webpayment response carries no payment_url".to_owned(),
        )
    })?;
    let payment_url = checked_redirect_url(payment_url)?;

    let mut ref_extra = RefExtra::new();
    ref_extra.insert("pay_token".to_owned(), pay_token);
    if let Some(notif_token) = non_empty(parsed.notif_token) {
        ref_extra.insert("notif_token".to_owned(), notif_token);
    }

    Ok(Submitted {
        ref_extra,
        redirect_url: Some(payment_url),
    })
}

/// A `transactionstatus` 200 body as a [`ChargeStatus`].
///
/// # Why an unknown status is an error rather than a failure
///
/// Mapping an unrecognised string to `Failed` would close a charge that may
/// still be in flight, on the strength of a word this adapter does not
/// understand — irreversible, and wrong in the payer's favour exactly never.
/// [`ProviderError::Malformed`] classifies as [`vpay_core::Category::Rail`],
/// so the poll ladder keeps asking and the mapping gap shows up as a rail
/// error rate rather than as lost payments.
///
/// # Errors
///
/// [`ProviderError::Malformed`] if the body is not JSON, carries no `status`,
/// or carries one that is not in [`STATUS_TABLE`].
pub(crate) fn charge_status(body: &[u8]) -> Result<ChargeStatus, ProviderError> {
    let parsed: TransactionStatusResponse = serde_json::from_slice(body).map_err(|error| {
        ProviderError::malformed(format!(
            "orange_money: the transactionstatus response is not the documented JSON: {error}"
        ))
    })?;

    let status = non_empty(parsed.status).ok_or_else(|| {
        ProviderError::malformed(
            "orange_money: the transactionstatus response carries no status".to_owned(),
        )
    })?;

    match meaning(&status) {
        Some(Meaning::Pending) => Ok(ChargeStatus::Pending),
        Some(Meaning::Succeeded) => Ok(ChargeStatus::Succeeded {
            provider_txn_id: non_empty(parsed.txnid),
        }),
        Some(Meaning::Failed(code)) => Ok(ChargeStatus::Failed {
            code,
            raw: raw_reason(&status, parsed.message.as_deref()),
        }),
        None => Err(ProviderError::malformed(format!(
            "orange_money: unrecognised transaction status {status:?}; refusing to guess whether \
             the charge is still in flight"
        ))),
    }
}

/// The rail's own words, for an operator reading a charge's `failure_reason`.
///
/// Never empty — the conformance suite asserts that, and it is the only thing
/// that tells someone *why* a `provider_error` happened once the taxonomy has
/// flattened it.
fn raw_reason(status: &str, message: Option<&str>) -> String {
    match message.map(str::trim).filter(|m| !m.is_empty()) {
        Some(message) => format!("{status}: {message}"),
        None => status.to_owned(),
    }
}

/// The ceiling `charges.redirect_url` is constrained to (migration `0019`).
///
/// 2048 characters is the practical URL limit every browser and proxy
/// agrees on, and matching the column exactly is the point: a URL this
/// function accepts must be one the database accepts, or the refusal
/// arrives as a `503` from a CHECK instead of as a rail error the recovery
/// path understands.
const MAX_REDIRECT_URL_CHARS: usize = 2048;

/// The schemes a payer's browser may be sent to, lowercased for comparison.
///
/// A closed list rather than "not `javascript:`": a denylist of dangerous
/// schemes is a list somebody has to keep up to date, and the set of things
/// a rail could legitimately answer with here is exactly two.
const ALLOWED_REDIRECT_SCHEMES: [&str; 2] = ["http://", "https://"];

/// The rail's `payment_url`, if it is something a payer may be sent to.
///
/// Compared case-insensitively because URL schemes are (RFC 3986 §3.1), and
/// `charges.redirect_url`'s CHECK lowercases for the same reason — a rail
/// that answers `HTTPS://` must be accepted by both layers or by neither.
///
/// # Errors
///
/// [`ProviderError::Malformed`] for a non-`http(s)` scheme or a URL over
/// [`MAX_REDIRECT_URL_CHARS`]. The offending value is *not* quoted: the URL
/// carries the `pay_token`, and an error message is logged.
fn checked_redirect_url(url: String) -> Result<String, ProviderError> {
    let lowercase = url.to_lowercase();
    if !ALLOWED_REDIRECT_SCHEMES
        .iter()
        .any(|scheme| lowercase.starts_with(scheme))
    {
        return Err(ProviderError::malformed(
            "orange_money: the webpayment response's payment_url is not an http(s) URL; \
             refusing to hand a payer a redirect this rail invented"
                .to_owned(),
        ));
    }
    if url.chars().count() > MAX_REDIRECT_URL_CHARS {
        return Err(ProviderError::malformed(format!(
            "orange_money: the webpayment response's payment_url is longer than \
             {MAX_REDIRECT_URL_CHARS} characters"
        )));
    }
    Ok(url)
}

/// An absent field and a present-but-empty one mean the same thing to us, and
/// treating them differently is how "the rail sent `\"\"`" becomes a token of
/// zero length written to the database.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_status_maps_and_nothing_else_does() {
        // The doc's table, spelled out again rather than looped over
        // STATUS_TABLE: a test that reads the table it is testing asserts the
        // table equals itself.
        assert_eq!(meaning("INITIATED"), Some(Meaning::Pending));
        assert_eq!(meaning("PENDING"), Some(Meaning::Pending));
        assert_eq!(meaning("SUCCESS"), Some(Meaning::Succeeded));
        assert_eq!(
            meaning("EXPIRED"),
            Some(Meaning::Failed(FailureCode::PayerTimeout))
        );
        assert_eq!(
            meaning("FAILED"),
            Some(Meaning::Failed(FailureCode::ProviderError))
        );
        assert_eq!(meaning("SETTLED"), None);
        assert_eq!(meaning(""), None);
    }

    #[test]
    fn status_matching_ignores_case() {
        assert_eq!(meaning("success"), Some(Meaning::Succeeded));
        assert_eq!(meaning("Pending"), Some(Meaning::Pending));
    }

    /// `INITIATED` is the row a careless transcription turns into a failure:
    /// the token exists, the payer has not started, and the charge is alive.
    #[test]
    fn initiated_is_pending_not_a_failure() {
        let status = charge_status(br#"{"status":"INITIATED","order_id":"x"}"#)
            .expect("a documented status parses");
        assert_eq!(status, ChargeStatus::Pending);
    }

    #[test]
    fn success_carries_the_rails_transaction_id() {
        let status = charge_status(br#"{"status":"SUCCESS","txnid":"OM123"}"#)
            .expect("a documented status parses");
        assert_eq!(
            status,
            ChargeStatus::Succeeded {
                provider_txn_id: Some("OM123".to_owned())
            }
        );
    }

    /// A settled charge with no `txnid` is still settled. Erroring here would
    /// keep polling a payment the payer has already made.
    #[test]
    fn success_without_a_txnid_is_still_a_success() {
        let status = charge_status(br#"{"status":"SUCCESS"}"#).expect("a documented status parses");
        assert_eq!(
            status,
            ChargeStatus::Succeeded {
                provider_txn_id: None
            }
        );
    }

    #[test]
    fn expired_is_the_payers_timeout_and_carries_a_raw_reason() {
        let status = charge_status(br#"{"status":"EXPIRED"}"#).expect("a documented status parses");
        match status {
            ChargeStatus::Failed { code, raw } => {
                assert_eq!(code, FailureCode::PayerTimeout);
                assert!(!raw.is_empty(), "an operator needs the rail's own words");
            }
            other => panic!("expected a decline, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_status_carries_the_rails_message_when_there_is_one() {
        let status = charge_status(br#"{"status":"FAILED","message":"debit refused"}"#)
            .expect("a documented status parses");
        match status {
            ChargeStatus::Failed { code, raw } => {
                assert_eq!(code, FailureCode::ProviderError);
                assert_eq!(raw, "FAILED: debit refused");
            }
            other => panic!("expected a decline, got {other:?}"),
        }
    }

    /// The safety property: an unknown word must not close a charge.
    #[test]
    fn an_unrecognised_status_is_an_error_never_a_failure() {
        let error = charge_status(br#"{"status":"REVERSED"}"#)
            .expect_err("an unmapped status must not be guessed at");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_status_body_without_a_status_is_malformed() {
        let error = charge_status(br#"{"order_id":"x"}"#).expect_err("no status, no answer");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
    }

    /// The shape the whole redirect flow rests on: the URL and the key
    /// material arrive in one value or not at all.
    #[test]
    fn a_submit_response_yields_the_url_and_the_key_material_together() {
        let submitted = submitted(
            br#"{"pay_token":"pt","payment_url":"https://webpayment.example/pay/pt",
                 "notif_token":"nt","status":201}"#,
        )
        .expect("the documented body parses");

        assert_eq!(
            submitted.redirect_url.as_deref(),
            Some("https://webpayment.example/pay/pt")
        );
        assert_eq!(
            submitted.ref_extra.get("pay_token").map(String::as_str),
            Some("pt")
        );
        assert_eq!(
            submitted.ref_extra.get("notif_token").map(String::as_str),
            Some("nt")
        );
        assert_eq!(
            submitted.ref_extra.len(),
            2,
            "ref_extra is persisted verbatim; an extra key is a schema change nobody reviewed"
        );
    }

    #[test]
    fn a_submit_response_without_a_notif_token_still_submits() {
        let submitted =
            submitted(br#"{"pay_token":"pt","payment_url":"https://p/x"}"#).expect("still valid");
        assert!(submitted.ref_extra.contains_key("pay_token"));
        assert!(!submitted.ref_extra.contains_key("notif_token"));
    }

    #[test]
    fn a_submit_response_without_a_pay_token_is_malformed() {
        let error = submitted(br#"{"payment_url":"https://p/x"}"#)
            .expect_err("a URL with no token is exactly what must not reach a caller");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_submit_response_without_a_payment_url_is_malformed() {
        let error = submitted(br#"{"pay_token":"pt"}"#)
            .expect_err("a redirect rail owes us somewhere to send the payer");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
    }

    /// The rail's own words must never carry the token that gates the payer's
    /// redirect: this error is logged, and the body it came from is not.
    #[test]
    fn a_malformed_submit_error_does_not_quote_the_body() {
        let error = submitted(br#"{"pay_token":"","payment_url":"https://p/x"}"#)
            .expect_err("an empty token is no token");
        assert!(!error.to_string().contains("https://p/x"), "{error}");
    }

    /// The finding this check exists for: a `payment_url` a payer's browser
    /// would *execute* rather than navigate to. The submit must fail rather
    /// than persist it and render it as `next_action.redirect_to_url.url`.
    #[test]
    fn a_payment_url_that_is_not_an_http_url_is_refused() {
        for hostile in [
            r#"{"pay_token":"pt","payment_url":"javascript:alert(document.cookie)"}"#,
            r#"{"pay_token":"pt","payment_url":"data:text/html;base64,PHNjcmlwdD4="}"#,
            r#"{"pay_token":"pt","payment_url":"file:///etc/passwd"}"#,
            r#"{"pay_token":"pt","payment_url":"//evil.example/pay"}"#,
        ] {
            let error = submitted(hostile.as_bytes())
                .expect_err("only an http(s) URL may be handed to a payer");
            assert!(
                matches!(error, ProviderError::Malformed { .. }),
                "{hostile}: {error:?}"
            );
        }
    }

    /// Schemes are case-insensitive, and the database CHECK lowercases too:
    /// a rail shouting its scheme must be accepted by both layers, not
    /// refused here and accepted there (or the reverse).
    #[test]
    fn an_uppercase_scheme_is_still_an_http_url() {
        let submitted = submitted(br#"{"pay_token":"pt","payment_url":"HTTPS://p.example/x"}"#)
            .expect("a scheme is case-insensitive");
        assert_eq!(
            submitted.redirect_url.as_deref(),
            Some("HTTPS://p.example/x"),
            "accepted, and carried through byte for byte"
        );
    }

    /// The same 2048 the `charges.redirect_url` CHECK enforces, so an
    /// over-long URL is a rail error here and never a `503` from Postgres.
    #[test]
    fn a_payment_url_over_the_column_limit_is_refused() {
        let long = format!(
            r#"{{"pay_token":"pt","payment_url":"https://p.example/{}"}}"#,
            "x".repeat(MAX_REDIRECT_URL_CHARS)
        );
        let error = submitted(long.as_bytes()).expect_err("the column would refuse it too");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );

        let at_the_limit = "x".repeat(MAX_REDIRECT_URL_CHARS - "https://p.example/".len());
        let ok =
            format!(r#"{{"pay_token":"pt","payment_url":"https://p.example/{at_the_limit}"}}"#);
        assert!(
            submitted(ok.as_bytes()).is_ok(),
            "exactly at the limit must be accepted, or the two layers disagree by one"
        );
    }

    /// The URL carries the `pay_token`. A refusal is logged; the value that
    /// caused it must not be.
    #[test]
    fn refusing_a_payment_url_does_not_quote_it() {
        let error =
            submitted(br#"{"pay_token":"pt","payment_url":"javascript:steal(pay-secret)"}"#)
                .expect_err("not an http(s) URL");
        assert!(!error.to_string().contains("pay-secret"), "{error}");
    }

    #[test]
    fn an_empty_string_field_is_treated_as_absent() {
        let status = charge_status(br#"{"status":"SUCCESS","txnid":"  "}"#)
            .expect("a documented status parses");
        assert_eq!(
            status,
            ChargeStatus::Succeeded {
                provider_txn_id: None
            }
        );
    }
}
