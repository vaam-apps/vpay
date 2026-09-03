//! MTN's error vocabulary, translated into the core's closed taxonomy.
//!
//! Kept in one module, away from the transport, because this is the part that
//! *drifts*: MTN grows a `reason` string, and the only visible symptom is a
//! rising `provider_error` rate ("an alert, not a resting place" —
//! `docs/flows/failures.md`). A table in one file is a table a reviewer can
//! diff against the rail's documentation; the same rows spread across `match`
//! arms in the request code are not.
//!
//! Three separate vocabularies live here because MTN uses three, and
//! collapsing them would lose information:
//!
//! * [`FAILURE_REASONS`] — the `reason` on a `FAILED` status, and the `code`
//!   on a 400. These are *outcomes* and map onto [`FailureCode`].
//! * [`CONFIGURATION_CODES`] — codes MTN returns with HTTP 500 that are in
//!   fact **our** misconfiguration. Retrying these forever is the failure
//!   mode this list exists to prevent.
//! * everything else on a 500 — the rail, not us; a transport failure the
//!   poll ladder retries.

use vpay_core::FailureCode;
use vpay_provider::ProviderError;

/// `docs/flows/adapter-mtn-momo.md`'s mapping table, transcribed row for row
/// and in the same order, so the two can be diffed by eye.
///
/// The table's last row — "anything else → `provider_error` + raw reason" —
/// is [`failure_code`]'s fallback rather than an entry here, because it is a
/// default, not a mapping.
pub(crate) const FAILURE_REASONS: [(&str, FailureCode); 9] = [
    ("NOT_ENOUGH_FUNDS", FailureCode::InsufficientFunds),
    // The payer never entered their PIN; MTN gives them about five minutes.
    ("COULD_NOT_PERFORM_TRANSACTION", FailureCode::PayerTimeout),
    ("PAYER_NOT_FOUND", FailureCode::InvalidPayer),
    ("PAYER_LIMIT_REACHED", FailureCode::PayerLimitReached),
    (
        "SENDER_ACCOUNT_NOT_ACTIVE",
        FailureCode::PayerAccountBlocked,
    ),
    ("PAYEE_NOT_FOUND", FailureCode::InvalidPayee),
    (
        "PAYEE_NOT_ALLOWED_TO_RECEIVE",
        FailureCode::PayeeAccountBlocked,
    ),
    // *Our* partner account, not the payer's: this one pages
    // (`ProviderError`'s severity table).
    ("NOT_ALLOWED", FailureCode::ProviderAccountBlocked),
    ("SERVICE_UNAVAILABLE", FailureCode::ProviderUnavailable),
];

/// The codes MTN returns *with HTTP 500* that mean the request we sent was
/// wrong — a currency the environment does not accept, a target environment
/// that is not ours, a callback host that is not the registered one.
///
/// This is the rail's biggest wart and the reason a 500 is never blindly
/// retried here: all three are permanent until a human edits configuration,
/// and a retry loop against them is an outage that looks like a flake.
pub(crate) const CONFIGURATION_CODES: [&str; 3] = [
    "INVALID_CURRENCY",
    "NOT_ALLOWED_TARGET_ENVIRONMENT",
    "INVALID_CALLBACK_URL_HOST",
];

/// Maps one of MTN's `reason`/`code` strings onto the core taxonomy.
///
/// Case-insensitive: MTN documents these uppercase and sends them uppercase,
/// but a mapping table that silently stops matching because a rail changed
/// the case of a string is a table that fails open into `provider_error`.
pub(crate) fn failure_code(reason: &str) -> FailureCode {
    for (name, code) in FAILURE_REASONS {
        if reason.eq_ignore_ascii_case(name) {
            return code;
        }
    }
    // Deliberately not an error: an unmapped reason is still a decline, and
    // the raw string travels with it so an operator can add the row.
    FailureCode::ProviderError
}

/// Decides what an HTTP 500 from MTN actually was, from the `code` in its
/// body.
///
/// `None` — a 500 with no JSON body, or JSON without a `code` — is a
/// [`ProviderError::Transport`]: the rail spoke, we could not tell why, and
/// the poll ladder is what resolves it. Never a decline: a payer must not be
/// told they were refused because someone's gateway fell over.
pub(crate) fn internal_error(code: Option<&str>, raw: &str) -> ProviderError {
    match code {
        Some(code)
            if CONFIGURATION_CODES
                .iter()
                .any(|c| code.eq_ignore_ascii_case(c)) =>
        {
            ProviderError::Config(format!("mtn_momo: the rail refused our request: {code}"))
        }
        // `INTERNAL_PROCESSING_ERROR` can mean the wallet platform is down
        // *or* that the payer had no funds (`docs/flows/adapter-mtn-momo.md`).
        // Ambiguity resolves to the rail's side, because reporting it as a
        // decline would close a charge that may still be alive, whereas
        // reporting it as transport leaves the status query to settle it.
        Some(code) => ProviderError::Transport(format!("mtn_momo: HTTP 500 {code}")),
        None => ProviderError::Transport(format!("mtn_momo: HTTP 500 {raw}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The doc's table has nine mapped rows plus a catch-all; a row silently
    /// dropped here is a decline that starts arriving as `provider_error`.
    #[test]
    fn every_documented_reason_maps_to_its_documented_code() {
        assert_eq!(FAILURE_REASONS.len(), 9, "docs/flows/adapter-mtn-momo.md");
        for (reason, expected) in FAILURE_REASONS {
            assert_eq!(failure_code(reason), expected, "{reason}");
            assert_eq!(
                failure_code(&reason.to_lowercase()),
                expected,
                "{reason} (lowercased)"
            );
        }
    }

    /// A table with two rows for one reason would make the mapping depend on
    /// iteration order, which is exactly the kind of thing that is true until
    /// someone appends a row.
    #[test]
    fn no_reason_appears_twice() {
        let mut names: Vec<&str> = FAILURE_REASONS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate reason in the table");
    }

    #[test]
    fn an_unknown_reason_is_provider_error_and_never_a_guess() {
        assert_eq!(
            failure_code("SOMETHING_MTN_INVENTED_LAST_TUESDAY"),
            FailureCode::ProviderError
        );
        assert_eq!(failure_code(""), FailureCode::ProviderError);
    }

    #[test]
    fn a_500_that_is_our_configuration_is_never_a_transport_error() {
        for code in CONFIGURATION_CODES {
            assert!(
                matches!(internal_error(Some(code), "{}"), ProviderError::Config(_)),
                "{code} must not be retried"
            );
        }
    }

    #[test]
    fn a_500_the_rail_cannot_explain_is_transport_not_a_decline() {
        for code in [Some("INTERNAL_PROCESSING_ERROR"), Some("WHO_KNOWS"), None] {
            let error = internal_error(code, "Internal Server Error");
            assert!(
                matches!(error, ProviderError::Transport(_)),
                "{code:?} produced {error:?}"
            );
        }
    }
}
