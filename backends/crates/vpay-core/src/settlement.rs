//! What a rail's status answer means for a charge that is still live.
//!
//! This is the reconciler's half of the state machine, and it is deliberately
//! *not* part of [`crate::state::Transition`]. That enum is "one of the three
//! verbs a **merchant** can apply", and [`crate::state::next_status`] answers
//! `None` for every rail-driven edge on purpose: adding a variant for
//! `processing → succeeded` would make that edge reachable from an HTTP
//! handler, which is the single thing that enum exists to prevent. So the
//! rail-driven edges live here, in a sibling function with its own vocabulary,
//! and the two cannot be confused at a call site.
//!
//! The table is `docs/flows/reconciler.md` plus the recovery table of
//! `docs/flows/crash-safety.md`, transcribed. It is pure and total: a new
//! [`ChargeState`] or a new [`StatusKind`] fails to compile here rather than
//! falling into a wildcard.

use crate::failure::FailureCode;
use crate::state::ChargeState;

/// A rail's status answer, stripped of everything the decision does not
/// depend on.
///
/// A near-copy of `vpay_provider::ChargeStatus`, and that duplication is
/// deliberate: this crate knows nothing about any payment rail
/// (`vpay-provider` depends on *it*, never the other way round), so the port's
/// type cannot appear in this signature. The caller maps one onto the other —
/// a four-arm `match` in `vpay_worker::handlers` — and drops the two payloads
/// no state decision may read: the rail's transaction id (a reconciliation
/// field, not an input to a state machine) and, for a decline, the rail's raw
/// reason string (which belongs in `charges.failure_raw`, never in a branch).
///
/// [`Self::Failed`] keeps its [`FailureCode`] because the decision *result*
/// carries it: the taxonomy code is written to the charge in the same
/// statement that fails it, so it has to travel through this function rather
/// than be re-derived beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    /// The rail has the charge and has not finished with it.
    Pending,
    /// The rail says the money moved.
    Succeeded,
    /// The rail refused, with a code from the closed taxonomy
    /// (`docs/flows/failures.md`).
    Failed(FailureCode),
    /// The rail has no record of the reference. **Never on its own grounds
    /// to fail a charge** — see [`Settlement::Recover`].
    NotFound,
}

/// What the reconciler does to a charge, given what the rail just said.
///
/// The variants are the four physically different outcomes, not four spellings
/// of "update the row": three of them write, one of them does not, and one of
/// them is not a state at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// Nothing changes. The charge is already in the right live state and
    /// the poll ladder simply runs again.
    ///
    /// Distinct from `None` (see [`settle`]) because `Stay` means "still
    /// live, keep polling" while `None` means "terminal, stop".
    Stay,
    /// Move the charge to another **live** state, and leave the intent alone.
    ///
    /// Carries a [`ChargeState`] rather than being one variant per state so
    /// the caller's write is one compare-and-swap parameterised by this
    /// value; the type does not stop a terminal state being named here, but
    /// no arm of [`settle`] produces one and the terminal edges have their
    /// own variants precisely so they cannot be reached by accident.
    Live(ChargeState),
    /// The money moved: charge terminal, intent `succeeded`,
    /// `payment_intent.succeeded` emitted — one transaction.
    Succeeded,
    /// The rail refused: charge terminal with this taxonomy code, intent back
    /// to `requires_payment_method` carrying `last_payment_error`,
    /// `payment_intent.payment_failed` emitted — one transaction.
    ///
    /// `docs/flows/payment-lifecycle.md`: "a rail-reported failure is the
    /// only thing that fails a payment".
    Failed(FailureCode),
    /// Not a state at all: the rail's "I have no record" on a charge that may
    /// never have reached it. Hand this to the recovery table
    /// (`docs/flows/crash-safety.md`, `vpay_worker::recovery::recovery_step`),
    /// which decides between polling again and resubmitting the *same*
    /// reference.
    ///
    /// This variant exists so that "the rail has never heard of this" cannot
    /// be spelled the same way as any state: a `NotFound` that fell into
    /// [`Self::Failed`] would fail a charge a payer may already have paid,
    /// which is the one conclusion the whole recovery design refuses to draw.
    Recover,
}

/// What `status` means for a charge currently in `charge`, or `None` if the
/// charge is past caring.
///
/// `None` is "this charge is terminal, so no rail answer moves it" — the same
/// shape of answer, and the same reason, as [`crate::state::next_status`]
/// returning `None`: the caller learns that the edge is not available rather
/// than being handed a plausible one. A poll that lands on a terminal charge
/// is not an error (a duplicate job, a callback arriving after the answer);
/// it is simply finished.
///
/// A `const fn` and total, with no wildcard in either dimension, so the table
/// below *is* the specification — adding a [`ChargeState`] or a
/// [`StatusKind`] is a compile error here, not a silent default.
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
            // evidence we never sent it. Treated exactly as `Pending`: keep
            // polling. The caller still counts the streak, because a rising
            // one is worth an operator's attention, but no resubmit follows
            // from a charge the rail has already acknowledged.
            StatusKind::NotFound => Some(Settlement::Stay),
        },
    }
}

/// Does the rail's answer *disagree* with a charge that is already terminal?
///
/// [`settle`] answers `None` for every terminal charge, which is right — no
/// rail answer moves one — but `None` folds two very different situations
/// into one word. Usually it means the charge settled a moment ago and this
/// poll is simply late: the rail says `SUCCESSFUL`, the charge already says
/// `succeeded`, and there is nothing to do. Sometimes it means the rail is
/// telling us the money went the *other* way from what we recorded and told
/// the merchant.
///
/// That second case is the one that has to reach a human. vpay must not act
/// on it — a charge is settled once, and flipping `failed` to `succeeded`
/// from a poll would make the settlement transaction's compare-and-swap
/// meaningless — but discarding it silently is how a real double-charge, or a
/// payment a merchant was told had failed, goes unnoticed until the rail's
/// monthly statement. So the caller keeps the job finished and raises an
/// alert (`docs/runbooks/unresolved-charges.md` is the reconciliation this
/// starts).
///
/// Only the two money-bearing disagreements count. `Pending` against a
/// terminal charge is a rail that has not caught up with itself, and
/// `NotFound` is never on its own grounds for any conclusion
/// (`docs/flows/crash-safety.md`) — neither says the money moved differently
/// than recorded, and alerting on them would bury the two that do.
///
/// Total and wildcard-free in both dimensions, like [`settle`]: a new
/// [`ChargeState`] or [`StatusKind`] is a compile error here rather than a
/// silent `false`.
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
