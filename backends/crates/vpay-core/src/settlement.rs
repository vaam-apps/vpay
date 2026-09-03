//! What a rail's status answer means for a charge that is still live.
//!
//! The reconciler's half of the state machine, deliberately kept out of
//! [`crate::state::Transition`] so a rail-driven edge can never be reached
//! from an HTTP handler. The table is `docs/flows/reconciler.md` plus the
//! recovery table of `docs/flows/crash-safety.md`, transcribed; why it is a
//! sibling module rather than three more `Transition` variants is in
//! [docs/reference/vpay-core.md § settlement](../../../../docs/reference/vpay-core.md#settlement).

use crate::failure::FailureCode;
use crate::state::ChargeState;

/// A rail's status answer, stripped of everything the decision does not
/// depend on.
///
/// A near-copy of `vpay_provider::ChargeStatus`, deliberately: this crate
/// knows nothing about any payment rail, so the port's type cannot appear in
/// this signature. The caller maps one onto the other and drops the two
/// payloads no state decision may read — the rail's transaction id and, for a
/// decline, its raw reason string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    /// The rail has the charge and has not finished with it.
    Pending,
    /// The rail says the money moved.
    Succeeded,
    /// The rail refused, with a code from the closed taxonomy
    /// (`docs/flows/failures.md`). The code travels through [`settle`]
    /// because it is written to the charge in the same statement that fails
    /// it.
    Failed(FailureCode),
    /// The rail has no record of the reference. **Never on its own grounds
    /// to fail a charge** — see [`Settlement::Recover`].
    NotFound,
}

/// What the reconciler does to a charge, given what the rail just said.
///
/// The variants are the four physically different outcomes, not four
/// spellings of "update the row": three of them write, one does not, and one
/// is not a state at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// Nothing changes. The charge is already in the right live state and the
    /// poll ladder simply runs again — distinct from [`settle`]'s `None`,
    /// which means "terminal, stop".
    Stay,
    /// Move the charge to another **live** state, and leave the intent alone.
    ///
    /// Carries a [`ChargeState`] so the caller's write is one
    /// compare-and-swap parameterised by this value. No arm of [`settle`]
    /// ever names a terminal state here; the terminal edges have their own
    /// variants precisely so they cannot be reached by accident.
    Live(ChargeState),
    /// The money moved: charge terminal, intent `succeeded`,
    /// `payment_intent.succeeded` emitted — one transaction.
    Succeeded,
    /// The rail refused: charge terminal with this taxonomy code, intent back
    /// to `requires_payment_method` carrying `last_payment_error`,
    /// `payment_intent.payment_failed` emitted — one transaction.
    Failed(FailureCode),
    /// Not a state at all: the rail's "I have no record" on a charge that may
    /// never have reached it. Hand this to the recovery table
    /// (`docs/flows/crash-safety.md`, `vpay_worker::recovery::recovery_step`),
    /// which decides between polling again and resubmitting the *same*
    /// reference.
    ///
    /// It is a variant of its own so that "the rail has never heard of this"
    /// cannot be spelled like any state: a `NotFound` folded into
    /// [`Self::Failed`] would fail a charge a payer may already have paid.
    Recover,
}

/// What `status` means for a charge currently in `charge`, or `None` if the
/// charge is past caring.
///
/// `None` is "this charge is terminal, so no rail answer moves it" — the same
/// shape of answer, and the same reason, as [`crate::state::next_status`]
/// returning `None`. A poll that lands on a terminal charge is not an error;
/// it is simply finished.
///
/// A `const fn`, total, with no wildcard in either dimension, so the table
/// *is* the specification: adding a [`ChargeState`] or a [`StatusKind`] is a
/// compile error here rather than a silent default.
///
/// ```
/// use vpay_core::{ChargeState, FailureCode, Settlement, StatusKind, settle};
///
/// // The rail acknowledged a charge we had only sent: it advances.
/// assert_eq!(
///     settle(StatusKind::Pending, ChargeState::Submitted),
///     Some(Settlement::Live(ChargeState::Pending))
/// );
/// // …and once it is `pending`, the same answer changes nothing.
/// assert_eq!(
///     settle(StatusKind::Pending, ChargeState::Pending),
///     Some(Settlement::Stay)
/// );
///
/// // "No record" on an unanswered submission is the crash-safety case, and
/// // is never a failure.
/// assert_eq!(
///     settle(StatusKind::NotFound, ChargeState::Submitting),
///     Some(Settlement::Recover)
/// );
/// // On a charge the rail already acknowledged it is the rail losing track:
/// // keep polling, do not resubmit.
/// assert_eq!(
///     settle(StatusKind::NotFound, ChargeState::Pending),
///     Some(Settlement::Stay)
/// );
///
/// // The taxonomy code is carried through, never re-derived.
/// assert_eq!(
///     settle(
///         StatusKind::Failed(FailureCode::InsufficientFunds),
///         ChargeState::Pending
///     ),
///     Some(Settlement::Failed(FailureCode::InsufficientFunds))
/// );
///
/// // A settled charge is never moved by the rail.
/// assert_eq!(settle(StatusKind::Succeeded, ChargeState::Failed), None);
/// ```
#[must_use]
pub const fn settle(status: StatusKind, charge: ChargeState) -> Option<Settlement> {
    match charge {
        // Terminal. Nothing the rail says moves these, in either direction:
        // a `succeeded` charge that a later poll calls `failed` is a rail
        // contradiction for a human to reconcile against a settlement
        // statement, never something this function silently applies.
        ChargeState::Succeeded | ChargeState::Failed => None,
        ChargeState::Submitting | ChargeState::Submitted => match status {
            // The rail has it and is working on it: the ambiguity of
            // "submitted" is over, so the charge advances to the state that
            // says the *rail* accepted it, not just that we sent it.
            StatusKind::Pending => Some(Settlement::Live(ChargeState::Pending)),
            StatusKind::Succeeded => Some(Settlement::Succeeded),
            StatusKind::Failed(code) => Some(Settlement::Failed(code)),
            // The crash-safety case: we may have sent nothing at all.
            StatusKind::NotFound => Some(Settlement::Recover),
        },
        ChargeState::Pending | ChargeState::Unresolved => match status {
            // Already there. `Unresolved` deliberately does not fall back to
            // `Pending`: the escalation is a fact about how long this charge
            // has been outstanding, and un-escalating it would drop the alert
            // a human is working from (`docs/flows/reconciler.md`).
            StatusKind::Pending => Some(Settlement::Stay),
            StatusKind::Succeeded => Some(Settlement::Succeeded),
            StatusKind::Failed(code) => Some(Settlement::Failed(code)),
            // The rail *did* accept this charge — that is what `pending`
            // records — so "no record" here is the rail losing track, not
            // evidence we never sent it.
            StatusKind::NotFound => Some(Settlement::Stay),
        },
    }
}

/// Does the rail's answer *disagree* with a charge that is already terminal?
///
/// [`settle`] answers `None` for every terminal charge, which folds two very
/// different situations into one word: a poll that is simply late, and a rail
/// telling us the money went the *other* way from what we recorded and told
/// the merchant. vpay must not act on the second — a charge is settled once —
/// but discarding it silently is how a real double-charge goes unnoticed
/// until the rail's monthly statement, so the caller finishes the job and
/// raises an alert (`docs/runbooks/unresolved-charges.md`).
///
/// Only the two money-bearing disagreements count; see
/// [docs/reference/vpay-core.md § contradictions](../../../../docs/reference/vpay-core.md#contradictions)
/// for why `Pending` and `NotFound` deliberately do not.
///
/// ```
/// use vpay_core::{ChargeState, FailureCode, StatusKind, contradiction};
///
/// // The two that reach a human.
/// assert!(contradiction(
///     StatusKind::Failed(FailureCode::PayerDeclined),
///     ChargeState::Succeeded
/// ));
/// assert!(contradiction(StatusKind::Succeeded, ChargeState::Failed));
///
/// // A rail that has not caught up with itself is not a contradiction, and
/// // neither is "no record" — alerting on either would bury the two above.
/// assert!(!contradiction(StatusKind::Pending, ChargeState::Succeeded));
/// assert!(!contradiction(StatusKind::NotFound, ChargeState::Failed));
/// // Nothing is recorded yet for a live charge to disagree with.
/// assert!(!contradiction(StatusKind::Succeeded, ChargeState::Pending));
/// ```
#[must_use]
pub const fn contradiction(status: StatusKind, charge: ChargeState) -> bool {
    match charge {
        // Not terminal: nothing has been recorded for an answer to disagree
        // with, and `settle` has a real answer for every one of these.
        ChargeState::Submitting
        | ChargeState::Submitted
        | ChargeState::Pending
        | ChargeState::Unresolved => false,
        ChargeState::Succeeded => match status {
            StatusKind::Failed(_) => true,
            StatusKind::Pending | StatusKind::Succeeded | StatusKind::NotFound => false,
        },
        ChargeState::Failed => match status {
            StatusKind::Succeeded => true,
            StatusKind::Pending | StatusKind::Failed(_) | StatusKind::NotFound => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every [`ChargeState`], written out rather than derived, so adding one
    /// makes the exhaustiveness assertion below fail to compile.
    const STATES: [ChargeState; 6] = [
        ChargeState::Submitting,
        ChargeState::Submitted,
        ChargeState::Pending,
        ChargeState::Unresolved,
        ChargeState::Succeeded,
        ChargeState::Failed,
    ];

    /// One representative decline code. The table does not branch on which
    /// code it is — `carries_the_taxonomy_code_through_unchanged` proves
    /// that separately, over the whole vocabulary.
    const DECLINE: FailureCode = FailureCode::InsufficientFunds;

    fn kinds() -> [StatusKind; 4] {
        [
            StatusKind::Pending,
            StatusKind::Succeeded,
            StatusKind::Failed(DECLINE),
            StatusKind::NotFound,
        ]
    }

    /// The whole of §3 of `docs/plans/2026-09-03-step4-worker.md`, as data.
    /// Every `(StatusKind × ChargeState)` pair appears exactly once; the
    /// count assertion below is what keeps that true.
    const TABLE: [(StatusKind, ChargeState, Option<Settlement>); 24] = [
        // --- the rail is still working on it
        (
            StatusKind::Pending,
            ChargeState::Submitting,
            Some(Settlement::Live(ChargeState::Pending)),
        ),
        (
            StatusKind::Pending,
            ChargeState::Submitted,
            Some(Settlement::Live(ChargeState::Pending)),
        ),
        (
            StatusKind::Pending,
            ChargeState::Pending,
            Some(Settlement::Stay),
        ),
        (
            StatusKind::Pending,
            ChargeState::Unresolved,
            Some(Settlement::Stay),
        ),
        (StatusKind::Pending, ChargeState::Succeeded, None),
        (StatusKind::Pending, ChargeState::Failed, None),
        // --- the money moved
        (
            StatusKind::Succeeded,
            ChargeState::Submitting,
            Some(Settlement::Succeeded),
        ),
        (
            StatusKind::Succeeded,
            ChargeState::Submitted,
            Some(Settlement::Succeeded),
        ),
        (
            StatusKind::Succeeded,
            ChargeState::Pending,
            Some(Settlement::Succeeded),
        ),
        (
            StatusKind::Succeeded,
            ChargeState::Unresolved,
            Some(Settlement::Succeeded),
        ),
        (StatusKind::Succeeded, ChargeState::Succeeded, None),
        (StatusKind::Succeeded, ChargeState::Failed, None),
        // --- the rail refused
        (
            StatusKind::Failed(DECLINE),
            ChargeState::Submitting,
            Some(Settlement::Failed(DECLINE)),
        ),
        (
            StatusKind::Failed(DECLINE),
            ChargeState::Submitted,
            Some(Settlement::Failed(DECLINE)),
        ),
        (
            StatusKind::Failed(DECLINE),
            ChargeState::Pending,
            Some(Settlement::Failed(DECLINE)),
        ),
        (
            StatusKind::Failed(DECLINE),
            ChargeState::Unresolved,
            Some(Settlement::Failed(DECLINE)),
        ),
        (StatusKind::Failed(DECLINE), ChargeState::Succeeded, None),
        (StatusKind::Failed(DECLINE), ChargeState::Failed, None),
        // --- the rail has no record
        (
            StatusKind::NotFound,
            ChargeState::Submitting,
            Some(Settlement::Recover),
        ),
        (
            StatusKind::NotFound,
            ChargeState::Submitted,
            Some(Settlement::Recover),
        ),
        (
            StatusKind::NotFound,
            ChargeState::Pending,
            Some(Settlement::Stay),
        ),
        (
            StatusKind::NotFound,
            ChargeState::Unresolved,
            Some(Settlement::Stay),
        ),
        (StatusKind::NotFound, ChargeState::Succeeded, None),
        (StatusKind::NotFound, ChargeState::Failed, None),
    ];

    /// The two money-bearing disagreements, and nothing else. Written as the
    /// full cartesian product rather than as two positive cases, because the
    /// bug this guards against is an over-eager alert: a rule that fires on
    /// `Pending` or `NotFound` against a settled charge pages on every late
    /// poll of every settled payment, and an alert that fires constantly is
    /// an alert nobody reads.
    #[test]
    fn only_a_rail_that_reverses_a_settled_charge_is_a_contradiction() {
        for state in STATES {
            for kind in kinds() {
                let expected = matches!(
                    (kind, state),
                    (StatusKind::Failed(_), ChargeState::Succeeded)
                        | (StatusKind::Succeeded, ChargeState::Failed)
                );
                assert_eq!(
                    contradiction(kind, state),
                    expected,
                    "contradiction({kind:?}, {state:?})"
                );
            }
        }
    }

    /// A contradiction is only ever reported where [`settle`] has no answer.
    /// If these two ever disagreed, the caller would be logging "the rail
    /// contradicts us" on a path that is also about to write the rail's
    /// answer to the charge.
    #[test]
    fn a_contradiction_is_only_possible_where_settle_declines_to_act() {
        for state in STATES {
            for kind in kinds() {
                if contradiction(kind, state) {
                    assert_eq!(
                        settle(kind, state),
                        None,
                        "settle({kind:?}, {state:?}) would act on a contradiction"
                    );
                }
            }
        }
    }

    /// Which code the rail declined with is not part of the disagreement:
    /// any decline against a `succeeded` charge is the same alert.
    #[test]
    fn every_decline_code_contradicts_a_succeeded_charge() {
        for code in [
            FailureCode::InsufficientFunds,
            FailureCode::PayerDeclined,
            FailureCode::ProviderUnavailable,
        ] {
            assert!(contradiction(
                StatusKind::Failed(code),
                ChargeState::Succeeded
            ));
            assert!(!contradiction(
                StatusKind::Failed(code),
                ChargeState::Failed
            ));
        }
    }

    #[test]
    fn the_documented_table_is_the_implementation() {
        for (status, charge, expected) in TABLE {
            assert_eq!(
                settle(status, charge),
                expected,
                "settle({status:?}, {charge:?}) disagrees with docs/flows/reconciler.md"
            );
        }
    }

    /// The table above is only a specification if it covers everything. This
    /// walks the cartesian product independently and asserts each pair is
    /// present exactly once — so a row deleted from `TABLE` (or a state added
    /// to the enum without a row) fails here rather than silently shrinking
    /// what `the_documented_table_is_the_implementation` checks.
    #[test]
    fn the_table_covers_every_pair_exactly_once() {
        assert_eq!(TABLE.len(), kinds().len() * STATES.len());
        for kind in kinds() {
            for state in STATES {
                let matches = TABLE
                    .iter()
                    .filter(|(k, s, _)| *k == kind && *s == state)
                    .count();
                assert_eq!(matches, 1, "({kind:?}, {state:?}) appears {matches} times");
            }
        }
    }

    /// A `NotFound` on an unanswered submission must never reach the charge
    /// as a failure. This is the assertion `docs/flows/crash-safety.md`'s
    /// "a bare `NotFound` is **never** on its own grounds to fail a charge"
    /// turns into code.
    #[test]
    fn not_found_never_fails_a_charge() {
        for state in STATES {
            match settle(StatusKind::NotFound, state) {
                Some(Settlement::Failed(code)) => {
                    panic!("NotFound failed a {state:?} charge with {code}")
                }
                Some(Settlement::Succeeded) => panic!("NotFound succeeded a {state:?} charge"),
                Some(Settlement::Stay | Settlement::Recover) | None => {}
                Some(Settlement::Live(next)) => {
                    panic!("NotFound moved a {state:?} charge to {next:?}")
                }
            }
        }
    }

    /// The recovery marker is reserved for the two states in which we do not
    /// know whether the rail ever received the charge. Anywhere else it would
    /// send a resubmit at a charge the rail has already acknowledged.
    #[test]
    fn only_an_unanswered_submission_asks_for_recovery() {
        for state in STATES {
            for kind in kinds() {
                let recovering = settle(kind, state) == Some(Settlement::Recover);
                let expected = kind == StatusKind::NotFound
                    && matches!(state, ChargeState::Submitting | ChargeState::Submitted);
                assert_eq!(recovering, expected, "settle({kind:?}, {state:?})");
            }
        }
    }

    /// The taxonomy code is carried, never re-derived: whatever the adapter
    /// mapped is exactly what lands on the charge.
    #[test]
    fn carries_the_taxonomy_code_through_unchanged() {
        const EVERY_CODE: [FailureCode; 11] = [
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
        for code in EVERY_CODE {
            for state in [
                ChargeState::Submitting,
                ChargeState::Submitted,
                ChargeState::Pending,
                ChargeState::Unresolved,
            ] {
                assert_eq!(
                    settle(StatusKind::Failed(code), state),
                    Some(Settlement::Failed(code)),
                    "the code changed on the way through, for a {state:?} charge"
                );
            }
        }
    }

    /// A terminal charge answers `None` for every rail answer — including one
    /// that contradicts it. That contradiction is a reconciliation problem
    /// for a human, and quietly rewriting a settled charge is the wrong
    /// answer to it.
    #[test]
    fn a_terminal_charge_is_never_moved_by_the_rail() {
        for state in [ChargeState::Succeeded, ChargeState::Failed] {
            for kind in kinds() {
                assert_eq!(settle(kind, state), None, "settle({kind:?}, {state:?})");
            }
        }
    }

    /// Every live answer keeps the charge live: `settle` never returns a
    /// [`Settlement::Live`] naming a terminal state, so the caller's
    /// compare-and-swap out of a live state cannot be used to settle money.
    #[test]
    fn the_live_variant_never_names_a_terminal_state() {
        for state in STATES {
            for kind in kinds() {
                if let Some(Settlement::Live(next)) = settle(kind, state) {
                    assert!(
                        next.is_live(),
                        "settle({kind:?}, {state:?}) moved a charge to terminal {next:?} \
                         through the live edge"
                    );
                }
            }
        }
    }
}
