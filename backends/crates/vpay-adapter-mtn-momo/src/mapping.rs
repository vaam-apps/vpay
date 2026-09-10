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
//!
//! # Where the reason strings come from (issue #59)
//!
//! Until 2026-09-10 this table's nine rows were transcribed from
//! `docs/flows/adapter-mtn-momo.md`, which is itself a reconstruction. The
//! rows are now checked against MTN's **published** vocabulary: the
//! `ErrorReason.code` enum in the Collection API's OpenAPI components
//! document, retrieved 2026-09-10 from
//!
//! ```text
//! GET https://momodeveloper.mtn.com/developer/apis/Collection/schemas/\
//!     668d4753d54e6119240c675d?api-version=2022-04-01-preview
//! ```
//!
//! — the same unauthenticated portal read `docs/plans/issue-47-notes/impl.md`
//! documents for `GetBasicUserinfo`. It enumerates **seventeen** codes; the
//! full comparison against this table, row by row, is in
//! `docs/plans/exp48-failure-codes-notes/opus.md`.
//!
//! Two consequences, and both are recorded rather than smoothed over:
//!
//! * **MTN binds this enum to the response this adapter actually polls.**
//!   `RequestToPayResult.reason` is `$ref: "#/components/schemas/ErrorReason"`
//!   in the same document, and `RequesttoPayTransactionStatus`
//!   (`GET /v1_0/requesttopay/{referenceId}`) answers `RequestToPayResult` on
//!   its `200` — whose description is "the 'reason' field can be used to
//!   retrieve a cause in case of failure", and two of whose worked examples
//!   put `PAYER_NOT_FOUND` and `PAYEE_NOT_FOUND` in `reason.code`. So these
//!   rows are a *citation* about `requesttopay`, not an inference from a
//!   neighbouring operation.
//!
//!   This bullet said the opposite until 2026-09-11 — that `ErrorReason` was
//!   "the whole Collection API's error schema, not `requesttopay`'s", that
//!   MTN "publishes no per-operation subset", and that every new row was
//!   therefore a deliberate assumption. The document says otherwise, and the
//!   correction matters in both directions: it is the difference between
//!   `PAYMENT_NOT_APPROVED` being evidence and being a guess a maintainer
//!   would be right to revert, and it sharpens the next bullet from "absent
//!   from a big shared schema" to "absent from the enum MTN types this exact
//!   field with".
//!
//!   What a schema cannot say is that a value ever *arrives*. Nothing here
//!   has called MTN.
//! * **Two rows below are not in that enum at all** —
//!   `COULD_NOT_PERFORM_TRANSACTION` and `SENDER_ACCOUNT_NOT_ACTIVE`. They
//!   are kept, because the flow doc and this repository's stubs have carried
//!   them since Step 3 and removing them would turn two mapped declines back
//!   into `provider_error`, but nothing in MTN's published documentation
//!   confirms either. See [`UNPUBLISHED_REASONS`].

use vpay_core::FailureCode;
use vpay_provider::ProviderError;

/// `docs/flows/adapter-mtn-momo.md`'s mapping table, transcribed row for row
/// and in the same order, so the two can be diffed by eye.
///
/// The table's last row — "anything else → `provider_error` + raw reason" —
/// is [`failure_code`]'s fallback rather than an entry here, because it is a
/// default, not a mapping. [`UNMAPPED_REASONS`] says which of MTN's published
/// codes land there *on purpose*, so that list is a decision and not an
/// oversight.
pub(crate) const FAILURE_REASONS: [(&str, FailureCode); 12] = [
    ("NOT_ENOUGH_FUNDS", FailureCode::InsufficientFunds),
    // The payer never entered their PIN; MTN gives them about five minutes.
    // Not in MTN's published `ErrorReason` enum — see [`UNPUBLISHED_REASONS`].
    ("COULD_NOT_PERFORM_TRANSACTION", FailureCode::PayerTimeout),
    // The prompt's own window closed. Published, and the same outcome as the
    // row above from a payer's point of view; Orange spells its equivalent
    // `EXPIRED` too, which is why both rails' timeouts reach a merchant as
    // one code.
    ("EXPIRED", FailureCode::PayerTimeout),
    // The payer was asked and did not approve. This is the row issue #59
    // exists for: `FailureCode::PayerDeclined` was defined by the core,
    // promised by the shop's buyer copy and produced by nothing.
    ("PAYMENT_NOT_APPROVED", FailureCode::PayerDeclined),
    // An approval the payer explicitly rejected. Same outcome, same sentence
    // to a buyer; kept as its own row because it is its own published code
    // and flattening the two would lose the rail's own words.
    ("APPROVAL_REJECTED", FailureCode::PayerDeclined),
    ("PAYER_NOT_FOUND", FailureCode::InvalidPayer),
    ("PAYER_LIMIT_REACHED", FailureCode::PayerLimitReached),
    // Not in MTN's published `ErrorReason` enum — see [`UNPUBLISHED_REASONS`].
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

/// The rows above that MTN's published `ErrorReason` enum does **not**
/// contain, and which therefore rest on `docs/flows/adapter-mtn-momo.md`
/// alone.
///
/// Kept as data rather than as prose because the useful question — "is this
/// mapping backed by the rail's documentation or by ours?" — is one an
/// operator asks about a *specific* string while reading a `failure_raw`, and
/// a list they can grep answers it. Removing a row from here is a claim that
/// MTN has published it; removing it from [`FAILURE_REASONS`] turns a mapped
/// decline back into `provider_error`.
///
/// `pub` for the same reason [`PRODUCED_FAILURE_CODES`] is: it is a claim
/// this crate makes about how far its own table is evidenced, and a caveat
/// only the adapter can state is one every reader of the adapter should be
/// able to reach.
pub const UNPUBLISHED_REASONS: [&str; 2] =
    ["COULD_NOT_PERFORM_TRANSACTION", "SENDER_ACCOUNT_NOT_ACTIVE"];

/// The published codes that stay [`FailureCode::ProviderError`], and why.
///
/// Every one of these is in MTN's `ErrorReason` enum, so none of them is an
/// unknown string the fallback happens to catch — each is a decision that the
/// core's taxonomy has no honest home for it, taken once and written down so
/// that a reviewer diffing the enum against [`FAILURE_REASONS`] can see the
/// difference is accounted for rather than missed.
///
/// * `INTERNAL_PROCESSING_ERROR` — the rail could not say what happened. On a
///   500 it is [`ProviderError::Transport`] and the poll ladder resolves it
///   (see [`internal_error`]); arriving as the `reason` on a terminal
///   `FAILED`, there is nothing left to retry and nothing to tell a payer,
///   which is exactly what `provider_error` means.
/// * `RESOURCE_NOT_FOUND` — about the *reference*, not the payment. The
///   status query answers this as HTTP 404, which is
///   [`vpay_provider::ChargeStatus::NotFound`] and the whole recovery story;
///   as a `FAILED` reason it describes no payer outcome.
/// * `RESOURCE_ALREADY_EXIST` — the duplicate-reference answer, handled as
///   HTTP 409 → `Submitted`, which is what makes a same-reference retry safe.
///   As a `FAILED` reason it would be MTN contradicting itself.
/// * `TRANSACTION_CANCELED` — **the one genuinely open row.** A cancellation
///   is a nameable event and `payer_declined` would read well, but MTN
///   publishes no description saying *who* cancels, and the Collection API's
///   only cancel operations are `CancelInvoice` and `CancelPreApproval` —
///   neither of which vpay calls. Guessing "the payer" would put a sentence
///   in front of a buyer on the strength of a verb. It stays here, carrying
///   the rail's own word, until someone asks MTN.
/// * `INVALID_CURRENCY`, `NOT_ALLOWED_TARGET_ENVIRONMENT`,
///   `INVALID_CALLBACK_URL_HOST` — [`CONFIGURATION_CODES`], ours to fix. On a
///   500 they are [`ProviderError::Config`]; as a `FAILED` reason they are a
///   shape nobody has seen, and inventing a payer-facing code for our own
///   misconfiguration would blame the wrong party.
///
/// `pub` alongside [`UNPUBLISHED_REASONS`]: "we chose not to map this" and
/// "we have never seen this" are different answers to an operator staring at
/// a `provider_error`, and neither is discoverable from a table of the rows
/// that *were* mapped.
pub const UNMAPPED_REASONS: [&str; 7] = [
    "INTERNAL_PROCESSING_ERROR",
    "RESOURCE_NOT_FOUND",
    "RESOURCE_ALREADY_EXIST",
    "TRANSACTION_CANCELED",
    "INVALID_CURRENCY",
    "NOT_ALLOWED_TARGET_ENVIRONMENT",
    "INVALID_CALLBACK_URL_HOST",
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

/// Every [`FailureCode`] this adapter can put on a charge, and nothing else.
///
/// The rail's *declared vocabulary*. It exists because the interesting
/// question about the core's eleven codes is not what they mean — that is
/// `docs/flows/failures.md` — but which of them a given rail can actually
/// reach: the shop's buyer copy, the demo's test-number panel and the SDK
/// docs each promise outcomes per rail, and until issue #59 they promised
/// `payer_declined`, which no adapter produced. A promise checked against a
/// list is a promise that fails when the list changes.
///
/// MTN reaches all eleven. Ten come from [`FAILURE_REASONS`]; the other two
/// paths are not table rows and are the reason this cannot simply be derived
/// from one:
///
/// * [`FailureCode::ProviderAccountBlocked`] also arrives from HTTP `401`
///   and `403` (`crate::rail_credentials_refused`, `crate::token`), without
///   a `reason` string anywhere.
/// * [`FailureCode::ProviderError`] is [`failure_code`]'s fallback for a
///   string this table does not know, so it is reachable by construction and
///   can never be absent.
///
/// Ordered as [`vpay_core::FailureCode::ALL`] is, so the two can be diffed by
/// eye.
pub const PRODUCED_FAILURE_CODES: [FailureCode; 11] = [
    FailureCode::InsufficientFunds,
    FailureCode::PayerTimeout,
    FailureCode::PayerDeclined,
    FailureCode::InvalidPayer,
    FailureCode::PayerLimitReached,
    FailureCode::PayerAccountBlocked,
    FailureCode::InvalidPayee,
    FailureCode::PayeeAccountBlocked,
    FailureCode::ProviderAccountBlocked,
    FailureCode::ProviderUnavailable,
    FailureCode::ProviderError,
];

/// The codes in [`PRODUCED_FAILURE_CODES`] that no row of
/// [`FAILURE_REASONS`] produces, so the test below can hold the two lists to
/// each other instead of to itself.
///
/// Naming the path in a comment would not fail when the path is deleted;
/// naming it here means the sum has to keep adding up.
#[cfg(test)]
const NON_TABLE_FAILURE_CODES: [FailureCode; 2] = [
    FailureCode::ProviderAccountBlocked,
    FailureCode::ProviderError,
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
        Some(code) => ProviderError::transport(format!("mtn_momo: HTTP 500 {code}")),
        None => ProviderError::transport(format!("mtn_momo: HTTP 500 {raw}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MTN's published `ErrorReason.code` enum, verbatim, retrieved
    /// 2026-09-10 (see this module's header for the exact URL).
    ///
    /// Spelled out here rather than fetched: a test that reached the network
    /// would fail on a train, and the point of this list is to be a
    /// *snapshot* someone re-retrieves deliberately. Seventeen strings, in
    /// the order the document lists them.
    const PUBLISHED_ERROR_REASONS: [&str; 17] = [
        "PAYEE_NOT_FOUND",
        "PAYER_NOT_FOUND",
        "NOT_ALLOWED",
        "NOT_ALLOWED_TARGET_ENVIRONMENT",
        "INVALID_CALLBACK_URL_HOST",
        "INVALID_CURRENCY",
        "SERVICE_UNAVAILABLE",
        "INTERNAL_PROCESSING_ERROR",
        "NOT_ENOUGH_FUNDS",
        "PAYER_LIMIT_REACHED",
        "PAYEE_NOT_ALLOWED_TO_RECEIVE",
        "PAYMENT_NOT_APPROVED",
        "RESOURCE_NOT_FOUND",
        "APPROVAL_REJECTED",
        "EXPIRED",
        "TRANSACTION_CANCELED",
        "RESOURCE_ALREADY_EXIST",
    ];

    /// The doc's table has twelve mapped rows plus a catch-all; a row
    /// silently dropped here is a decline that starts arriving as
    /// `provider_error`.
    #[test]
    fn every_documented_reason_maps_to_its_documented_code() {
        assert_eq!(FAILURE_REASONS.len(), 12, "docs/flows/adapter-mtn-momo.md");
        for (reason, expected) in FAILURE_REASONS {
            assert_eq!(failure_code(reason), expected, "{reason}");
            assert_eq!(
                failure_code(&reason.to_lowercase()),
                expected,
                "{reason} (lowercased)"
            );
        }
    }

    /// **Every published code is accounted for, one way or the other.**
    ///
    /// This is the assertion issue #59 turns on. MTN publishes seventeen
    /// reasons; before 2026-09-10 this adapter mapped nine of them and had
    /// never been compared against the list, so seven arrived as
    /// `provider_error` and nobody could tell which of those were decisions.
    /// A code that is neither mapped nor deliberately unmapped now fails
    /// here, naming itself.
    #[test]
    fn every_published_reason_is_mapped_or_deliberately_not() {
        for published in PUBLISHED_ERROR_REASONS {
            let mapped = FAILURE_REASONS
                .iter()
                .any(|(reason, _)| reason.eq_ignore_ascii_case(published));
            let deliberate = UNMAPPED_REASONS
                .iter()
                .any(|reason| reason.eq_ignore_ascii_case(published));
            assert!(
                mapped != deliberate,
                "{published} is {}: MTN publishes it, so it belongs in exactly one of \
                 FAILURE_REASONS or UNMAPPED_REASONS",
                if mapped {
                    "in both lists"
                } else {
                    "in neither list"
                }
            );
        }
    }

    /// The converse, and the one that keeps this repository honest about what
    /// it has actually read: a row here that MTN does not publish must say so
    /// in [`UNPUBLISHED_REASONS`] rather than pass for documented.
    #[test]
    fn a_reason_mtn_does_not_publish_is_declared_as_such() {
        for (reason, _) in FAILURE_REASONS {
            let published = PUBLISHED_ERROR_REASONS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(reason));
            let declared = UNPUBLISHED_REASONS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(reason));
            assert!(
                published != declared,
                "{reason}: a mapped reason is either in MTN's published enum or listed in \
                 UNPUBLISHED_REASONS, never both and never neither"
            );
        }
        // The list is not vacuous, and shrinking it is a claim about MTN's
        // documentation that has to be made on purpose.
        assert_eq!(UNPUBLISHED_REASONS.len(), 2);
    }

    /// A deliberately unmapped reason still reaches `provider_error` — the
    /// list is documentation of a decision, never a second dispatch table
    /// that could disagree with [`failure_code`].
    #[test]
    fn a_deliberately_unmapped_reason_is_still_provider_error() {
        for reason in UNMAPPED_REASONS {
            assert_eq!(failure_code(reason), FailureCode::ProviderError, "{reason}");
        }
    }

    /// The declared vocabulary is exactly the table's codes plus the two
    /// paths that have no table row. Held to the table rather than to a
    /// hand-written list, so adding a row without widening the promise, or
    /// widening the promise without a row, both fail.
    #[test]
    fn the_declared_vocabulary_is_the_table_plus_the_paths_that_have_no_row() {
        let mut from_table: Vec<FailureCode> =
            FAILURE_REASONS.iter().map(|(_, code)| *code).collect();
        from_table.extend(NON_TABLE_FAILURE_CODES);
        from_table.sort_unstable_by_key(|code| code.as_str());
        from_table.dedup();

        let mut declared = PRODUCED_FAILURE_CODES.to_vec();
        declared.sort_unstable_by_key(|code| code.as_str());
        let before = declared.len();
        declared.dedup();
        assert_eq!(before, declared.len(), "a code is declared twice");

        assert_eq!(declared, from_table);
    }

    /// MTN reaches the whole taxonomy, and that is worth pinning: it is what
    /// makes it *this* rail the per-rail tables in `docs/flows/failures.md`
    /// measure the other against.
    #[test]
    fn mtn_reaches_every_code_the_core_defines() {
        for code in FailureCode::ALL {
            assert!(
                PRODUCED_FAILURE_CODES.contains(&code),
                "{code} is unreachable on MTN"
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
                matches!(error, ProviderError::Transport { .. }),
                "{code:?} produced {error:?}"
            );
        }
    }
}
