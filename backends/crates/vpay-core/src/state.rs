//! PaymentIntent and charge state, and the flow shape that selects between
//! them. See `docs/flows/payment-lifecycle.md`.

use serde::{Deserialize, Serialize};

/// How a payer authorises on a given rail.
///
/// The core branches on this *value*. It must never branch on a provider code —
/// see `docs/adr/0002-provider-port.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFlow {
    /// A prompt reaches the payer's handset (MTN MoMo). The payer can act
    /// before we learn whether submission succeeded, so the reference must be
    /// durable *before* submitting.
    Push,
    /// The payer is redirected to the rail's hosted page (Orange Money). They
    /// cannot act until we hand them a URL, so the rail's token must be durable
    /// *before* redirecting.
    Redirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    RequiresPaymentMethod,
    /// Redirect rails only; carries `next_action.redirect_to_url`.
    RequiresAction,
    Processing,
    Succeeded,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeState {
    Submitting,
    Submitted,
    Pending,
    /// Past the 24h horizon with no terminal answer. Still polled, but a human
    /// has been alerted. Never a silent failure.
    Unresolved,
    Succeeded,
    Failed,
}

impl ChargeState {
    /// Whether the reconciler should keep polling this charge.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(
            self,
            Self::Submitting | Self::Submitted | Self::Pending | Self::Unresolved
        )
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

impl ProviderFlow {
    /// The PaymentIntent status a successful `confirm` produces on this rail.
    #[must_use]
    pub const fn status_after_confirm(self) -> IntentStatus {
        match self {
            Self::Push => IntentStatus::Processing,
            Self::Redirect => IntentStatus::RequiresAction,
        }
    }
}

impl IntentStatus {
    /// The status a PaymentIntent is created in.
    ///
    /// See [`Transition::Create`]: creation is the one edge with no source
    /// state, so it cannot be expressed as a [`next_status`] answer.
    pub const INITIAL: Self = Self::RequiresPaymentMethod;

    /// The exact label this status carries on the wire *and* in Postgres.
    ///
    /// Written out beside the `serde` rename rather than derived from it
    /// because the two paths that need it do not go through `serde`: the
    /// `intent_status` Postgres enum is read and written as a `String`
    /// (`vpay-db` binds strings, this crate parses them — Step 2's D4), and
    /// `vpay-api`'s repository calls pass the *expected* and *new* label into
    /// a compare-and-swap `UPDATE`. A hand-rolled spelling that disagreed
    /// with `serde`'s would mean a status that renders one way to a merchant
    /// and matches another way in a `WHERE` clause — so
    /// `the_wire_spelling_is_the_same_by_both_routes` below pins them
    /// together.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::RequiresPaymentMethod => "requires_payment_method",
            Self::RequiresAction => "requires_action",
            Self::Processing => "processing",
            Self::Succeeded => "succeeded",
            Self::Canceled => "canceled",
        }
    }

    /// Parses a label produced by [`Self::as_wire_str`], or `None`.
    ///
    /// `Option` rather than a `FromStr` with an error type: the only caller is
    /// the boundary reading a Postgres enum back (D4), where an unparseable
    /// label is not a caller's mistake but a schema/code mismatch that the
    /// HTTP layer answers `500` for. Returning `None` lets that layer say so
    /// in its own vocabulary instead of forcing a new public error type into
    /// this crate for a case no merchant can cause.
    #[must_use]
    pub fn from_wire(label: &str) -> Option<Self> {
        [
            Self::RequiresPaymentMethod,
            Self::RequiresAction,
            Self::Processing,
            Self::Succeeded,
            Self::Canceled,
        ]
        .into_iter()
        .find(|status| status.as_wire_str() == label)
    }
}

impl ChargeState {
    /// The state a charge row is born in, before the rail has been asked
    /// anything.
    ///
    /// Named rather than spelled at the insert site: `docs/flows/crash-safety.md`
    /// turns on the row existing in *this* state before `submit` is called, and
    /// a call site that picked `Submitted` instead would be a silent
    /// crash-safety regression rather than a compile error.
    pub const INITIAL: Self = Self::Submitting;

    /// The exact label this state carries in Postgres — see
    /// [`IntentStatus::as_wire_str`] for why it is written out.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Submitting => "submitting",
            Self::Submitted => "submitted",
            Self::Pending => "pending",
            Self::Unresolved => "unresolved",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    /// Parses a label produced by [`Self::as_wire_str`], or `None` — see
    /// [`IntentStatus::from_wire`] for why this is not a `FromStr`.
    #[must_use]
    pub fn from_wire(label: &str) -> Option<Self> {
        [
            Self::Submitting,
            Self::Submitted,
            Self::Pending,
            Self::Unresolved,
            Self::Succeeded,
            Self::Failed,
        ]
        .into_iter()
        .find(|state| state.as_wire_str() == label)
    }
}

/// One of the three verbs a *merchant* can apply to a PaymentIntent.
///
/// Deliberately not "every edge in the lifecycle": the rail-driven edges of
/// `docs/flows/payment-lifecycle.md`'s diagram (`requires_action → processing`
/// once the payer has been redirected, `processing → succeeded|failed` when a
/// status query answers) are moved by the reconciler from an authenticated
/// status query, never by a request. Modelling them here would invite a
/// handler to call [`next_status`] and move an intent on a *callback*, which
/// is exactly the thing `docs/flows/callbacks.md` forbids — a callback is a
/// hint. So [`next_status`] answers `None` for them, meaning "not something
/// this request may do", not "impossible".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// `POST /v1/payment_intents`. The `[*] -->` edge of the diagram: it has
    /// no source status at all, so [`next_status`] answers `None` for every
    /// existing one and the status a new intent is born in is
    /// [`IntentStatus::INITIAL`]. It is in the enum so the vocabulary is the
    /// API's three verbs rather than two of them.
    Create,
    /// `POST /v1/payment_intents/{id}/confirm`, on a rail with this flow
    /// shape. The flow — not the rail's name — is what selects the next
    /// status ([ADR-0002](../../../../docs/adr/0002-provider-port.md)).
    Confirm(ProviderFlow),
    /// `POST /v1/payment_intents/{id}/cancel`.
    Cancel,
}

/// The status `transition` moves an intent in `from` to, or `None` if that
/// request is not legal from that status.
///
/// This is the *whole* rule, and it is one function on purpose: a handler that
/// decided "cancel is fine here" for itself is how `canceled` becomes
/// reachable from `processing` — after the rail already has the request and
/// cannot be recalled (`docs/flows/payment-lifecycle.md`). Both legal answers
/// route through [`ProviderFlow::status_after_confirm`] rather than repeating
/// the push/redirect split, so the two cannot drift.
///
/// `None` is not the same as "the intent is stuck": see [`Transition`] for the
/// edges the reconciler owns. And a legal answer here is *not* permission to
/// write it — the write itself is a compare-and-swap on the row's current
/// status (`vpay_db::payment_intents::transition`), because between this call
/// and that `UPDATE` another request may have moved the same row.
#[must_use]
pub const fn next_status(from: IntentStatus, transition: Transition) -> Option<IntentStatus> {
    // Matched without a `_` arm in either dimension: adding a status or a verb
    // must fail to compile here, not silently fall into "illegal".
    match from {
        IntentStatus::RequiresPaymentMethod => match transition {
            Transition::Create => None,
            Transition::Confirm(flow) => Some(flow.status_after_confirm()),
            Transition::Cancel => Some(IntentStatus::Canceled),
        },
        // `requires_action` is deliberately not confirmable a second time:
        // one charge per intent, forever. `processing` and the two terminal
        // states answer nothing at all.
        IntentStatus::RequiresAction
        | IntentStatus::Processing
        | IntentStatus::Succeeded
        | IntentStatus::Canceled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_is_still_polled() {
        assert!(ChargeState::Unresolved.is_live());
        assert!(!ChargeState::Unresolved.is_terminal());
    }

    #[test]
    fn flow_selects_the_post_confirm_status() {
        assert_eq!(
            ProviderFlow::Push.status_after_confirm(),
            IntentStatus::Processing
        );
        assert_eq!(
            ProviderFlow::Redirect.status_after_confirm(),
            IntentStatus::RequiresAction
        );
    }

    /// Every `IntentStatus`, so the table below is provably total. Adding a
    /// variant without adding it here is caught by
    /// `the_transition_table_covers_every_status_and_verb`, which counts.
    const EVERY_STATUS: [IntentStatus; 5] = [
        IntentStatus::RequiresPaymentMethod,
        IntentStatus::RequiresAction,
        IntentStatus::Processing,
        IntentStatus::Succeeded,
        IntentStatus::Canceled,
    ];

    /// Every `Transition`, `Confirm` once per flow shape.
    const EVERY_TRANSITION: [Transition; 4] = [
        Transition::Create,
        Transition::Confirm(ProviderFlow::Push),
        Transition::Confirm(ProviderFlow::Redirect),
        Transition::Cancel,
    ];

    /// The lifecycle, written out from `docs/flows/payment-lifecycle.md`'s
    /// diagram rather than read back out of [`next_status`] — every
    /// `(status, verb)` pair the API can present, with the answer the
    /// document commits to.
    ///
    /// The `None`s are the point: three quarters of this table is "no", and a
    /// permissive implementation (`Some(Canceled)` from `processing`, a second
    /// `confirm` from `requires_action`) passes any test that only checks the
    /// four legal edges.
    const TABLE: [(IntentStatus, Transition, Option<IntentStatus>); 20] = [
        // create has no source state at all — the `[*] -->` edge.
        (
            IntentStatus::RequiresPaymentMethod,
            Transition::Create,
            None,
        ),
        (IntentStatus::RequiresAction, Transition::Create, None),
        (IntentStatus::Processing, Transition::Create, None),
        (IntentStatus::Succeeded, Transition::Create, None),
        (IntentStatus::Canceled, Transition::Create, None),
        // confirm on a push rail: the payer gets a handset prompt, so there is
        // nothing for a browser to do and the intent goes straight to
        // `processing`.
        (
            IntentStatus::RequiresPaymentMethod,
            Transition::Confirm(ProviderFlow::Push),
            Some(IntentStatus::Processing),
        ),
        (
            IntentStatus::RequiresAction,
            Transition::Confirm(ProviderFlow::Push),
            None,
        ),
        (
            IntentStatus::Processing,
            Transition::Confirm(ProviderFlow::Push),
            None,
        ),
        (
            IntentStatus::Succeeded,
            Transition::Confirm(ProviderFlow::Push),
            None,
        ),
        (
            IntentStatus::Canceled,
            Transition::Confirm(ProviderFlow::Push),
            None,
        ),
        // confirm on a redirect rail: the payer cannot act until we hand them
        // a URL, so the intent waits in `requires_action`.
        (
            IntentStatus::RequiresPaymentMethod,
            Transition::Confirm(ProviderFlow::Redirect),
            Some(IntentStatus::RequiresAction),
        ),
        (
            IntentStatus::RequiresAction,
            Transition::Confirm(ProviderFlow::Redirect),
            None,
        ),
        (
            IntentStatus::Processing,
            Transition::Confirm(ProviderFlow::Redirect),
            None,
        ),
        (
            IntentStatus::Succeeded,
            Transition::Confirm(ProviderFlow::Redirect),
            None,
        ),
        (
            IntentStatus::Canceled,
            Transition::Confirm(ProviderFlow::Redirect),
            None,
        ),
        // cancel: legal from `requires_payment_method` and nowhere else. Once
        // a rail has the request you cannot recall it.
        (
            IntentStatus::RequiresPaymentMethod,
            Transition::Cancel,
            Some(IntentStatus::Canceled),
        ),
        (IntentStatus::RequiresAction, Transition::Cancel, None),
        (IntentStatus::Processing, Transition::Cancel, None),
        (IntentStatus::Succeeded, Transition::Cancel, None),
        (IntentStatus::Canceled, Transition::Cancel, None),
    ];

    #[test]
    fn next_status_answers_the_lifecycle_diagram_for_every_pair() {
        for (from, transition, expected) in TABLE {
            assert_eq!(
                next_status(from, transition),
                expected,
                "{from:?} + {transition:?}"
            );
        }
    }

    /// The table above is only worth anything if it is complete: 5 statuses x
    /// 4 verbs, each pair exactly once. A pair silently dropped from `TABLE`
    /// would otherwise be a hole nobody could see.
    #[test]
    fn the_transition_table_covers_every_status_and_verb() {
        assert_eq!(TABLE.len(), EVERY_STATUS.len() * EVERY_TRANSITION.len());
        for status in EVERY_STATUS {
            for transition in EVERY_TRANSITION {
                let matches = TABLE
                    .iter()
                    .filter(|(f, t, _)| *f == status && *t == transition)
                    .count();
                assert_eq!(matches, 1, "{status:?} + {transition:?} appears {matches}x");
            }
        }
    }

    /// Spelled out separately from the table because it is the one rule
    /// `docs/flows/payment-lifecycle.md` states in its own sentence
    /// ("`canceled` is reachable only from `requires_payment_method`"), and
    /// because it is the rule a handler is most tempted to relax.
    #[test]
    fn cancel_is_legal_only_from_requires_payment_method() {
        assert_eq!(
            next_status(IntentStatus::RequiresPaymentMethod, Transition::Cancel),
            Some(IntentStatus::Canceled)
        );
        for status in EVERY_STATUS {
            if status == IntentStatus::RequiresPaymentMethod {
                continue;
            }
            assert_eq!(
                next_status(status, Transition::Cancel),
                None,
                "cancel must be refused from {status:?}"
            );
        }
    }

    /// `confirm` must not decide the next status by rail name — the split is
    /// `ProviderFlow`'s, and this pins that it is *the same* function
    /// answering, not a second copy of the push/redirect rule.
    #[test]
    fn confirm_routes_through_the_flows_own_answer() {
        for flow in [ProviderFlow::Push, ProviderFlow::Redirect] {
            assert_eq!(
                next_status(
                    IntentStatus::RequiresPaymentMethod,
                    Transition::Confirm(flow)
                ),
                Some(flow.status_after_confirm()),
                "{flow:?}"
            );
        }
        // ... and that the two flows really do disagree, so the assertion
        // above is not vacuous.
        assert_ne!(
            ProviderFlow::Push.status_after_confirm(),
            ProviderFlow::Redirect.status_after_confirm()
        );
    }

    #[test]
    fn a_new_intent_starts_where_the_diagram_says() {
        assert_eq!(IntentStatus::INITIAL, IntentStatus::RequiresPaymentMethod);
        assert_eq!(ChargeState::INITIAL, ChargeState::Submitting);
        // The initial charge state must be one the reconciler still polls, or
        // a charge inserted before `submit` would be abandoned if the process
        // died between the two (docs/flows/crash-safety.md).
        assert!(ChargeState::INITIAL.is_live());
    }

    /// The `serde` spelling and the hand-written one are two independent
    /// paths to the same label — one reaches a merchant, the other reaches a
    /// `WHERE` clause. If they drift, a status renders one way and matches
    /// another.
    #[test]
    fn the_wire_spelling_is_the_same_by_both_routes() {
        for status in EVERY_STATUS {
            let serialised =
                serde_json::to_string(&status).expect("an IntentStatus serialises as a string");
            assert_eq!(serialised, format!("\"{}\"", status.as_wire_str()));
            assert_eq!(IntentStatus::from_wire(status.as_wire_str()), Some(status));
        }
        for state in [
            ChargeState::Submitting,
            ChargeState::Submitted,
            ChargeState::Pending,
            ChargeState::Unresolved,
            ChargeState::Succeeded,
            ChargeState::Failed,
        ] {
            let serialised =
                serde_json::to_string(&state).expect("a ChargeState serialises as a string");
            assert_eq!(serialised, format!("\"{}\"", state.as_wire_str()));
            assert_eq!(ChargeState::from_wire(state.as_wire_str()), Some(state));
        }
    }

    /// A label Postgres could hold that this crate does not model must be
    /// `None` rather than a plausible default — the caller (the HTTP
    /// boundary) answers 500 for it, and a default would render a wrong
    /// status to a merchant instead.
    #[test]
    fn an_unmodelled_label_does_not_parse() {
        for label in [
            "requires_confirmation", // Stripe has it; this API deliberately does not
            "failed",                // an intent never *is* failed — see the flow doc
            "REQUIRES_PAYMENT_METHOD",
            "",
            " processing",
        ] {
            assert_eq!(IntentStatus::from_wire(label), None, "{label:?}");
        }
        assert_eq!(ChargeState::from_wire("submitting "), None);
    }

    #[test]
    fn every_state_is_live_or_terminal_exclusively() {
        for s in [
            ChargeState::Submitting,
            ChargeState::Submitted,
            ChargeState::Pending,
            ChargeState::Unresolved,
            ChargeState::Succeeded,
            ChargeState::Failed,
        ] {
            assert_ne!(s.is_live(), s.is_terminal(), "{s:?} is neither or both");
        }
    }
}
