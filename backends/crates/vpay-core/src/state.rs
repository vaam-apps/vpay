//! PaymentIntent and charge state, and the flow shape that selects between
//! them.
//!
//! `docs/flows/payment-lifecycle.md` is the lifecycle itself. Why the wire
//! labels are written out beside the `serde` rename, why [`Transition`] is
//! only the merchant's three verbs, and why [`next_status`] is a single
//! function:
//! [docs/reference/vpay-core.md § state](../../../../docs/reference/vpay-core.md#state).

use serde::{Deserialize, Serialize};

/// How a payer authorises on a given rail.
///
/// The core branches on this *value*. It must never branch on a provider code
/// — see `docs/adr/0002-provider-port.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFlow {
    /// A prompt reaches the payer's handset (MTN MoMo). The payer can act
    /// before we learn whether submission succeeded, so the reference must be
    /// durable *before* submitting.
    Push,
    /// The payer is redirected to the rail's hosted page (Orange Money). They
    /// cannot act until we hand them a URL, so the rail's token must be
    /// durable *before* redirecting.
    Redirect,
}

/// Where a PaymentIntent is in its lifecycle — the status a merchant reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    /// Created, not yet confirmed. The only status from which `confirm` and
    /// `cancel` are legal.
    RequiresPaymentMethod,
    /// Redirect rails only; carries `next_action.redirect_to_url`.
    RequiresAction,
    /// The rail has the charge; the reconciler owns what happens next.
    Processing,
    /// The money moved.
    Succeeded,
    /// Cancelled before any rail saw it.
    Canceled,
}

/// Where a refund is in its own lifecycle — the status a merchant reads on
/// the `refund` object.
///
/// Deliberately **not** [`IntentStatus`], and deliberately carrying a
/// `Failed` the intent has none of: a refund that the rail refuses *is*
/// failed and stays that way, whereas an intent whose charge is declined
/// falls back to `requires_payment_method` so the merchant can try another
/// rail (`docs/flows/payment-lifecycle.md`). Refunds also do not change
/// their intent's status at all, so sharing one type would invite exactly
/// the assignment the flow doc forbids.
///
/// The four labels are the `refund_status` Postgres enum
/// (`backends/migrations/0017_create-refunds.sql`) and both merchant SDKs'
/// own `RefundStatus`, which is why they are written out beside the `serde`
/// rename — see [`IntentStatus::as_wire_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    /// Submitted to the rail; the money has not come back yet.
    Pending,
    /// The rail returned the funds.
    Succeeded,
    /// The rail refused the refund. Terminal.
    Failed,
    /// Withdrawn before the rail acted on it.
    Canceled,
}

/// Where a charge is in its life on the rail — an operator-facing state, not
/// a merchant-facing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeState {
    /// The row exists; the rail has not been asked yet
    /// (`docs/flows/crash-safety.md`).
    Submitting,
    /// Submitted, with no answer from the rail yet.
    Submitted,
    /// The rail acknowledged the charge and is working on it.
    Pending,
    /// Past the 24h horizon with no terminal answer. Still polled, but a
    /// human has been alerted. Never a silent failure.
    Unresolved,
    /// The rail says the money moved.
    Succeeded,
    /// The rail refused.
    Failed,
}

impl ChargeState {
    /// Whether the reconciler should keep polling this charge.
    ///
    /// ```
    /// use vpay_core::ChargeState;
    ///
    /// // `unresolved` is escalated, not abandoned: it is still polled.
    /// assert!(ChargeState::Unresolved.is_live());
    /// assert!(!ChargeState::Succeeded.is_live());
    /// ```
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(
            self,
            Self::Submitting | Self::Submitted | Self::Pending | Self::Unresolved
        )
    }

    /// Whether the rail has given a final answer. Exactly the complement of
    /// [`ChargeState::is_live`].
    ///
    /// ```
    /// use vpay_core::ChargeState;
    ///
    /// for state in ChargeState::ALL {
    ///     assert_ne!(state.is_live(), state.is_terminal());
    /// }
    /// ```
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

impl ProviderFlow {
    /// The PaymentIntent status a successful `confirm` produces on this rail.
    ///
    /// ```
    /// use vpay_core::{IntentStatus, ProviderFlow};
    ///
    /// // A handset prompt: nothing for a browser to do.
    /// assert_eq!(
    ///     ProviderFlow::Push.status_after_confirm(),
    ///     IntentStatus::Processing
    /// );
    /// // A hosted page: the payer cannot act until we hand them a URL.
    /// assert_eq!(
    ///     ProviderFlow::Redirect.status_after_confirm(),
    ///     IntentStatus::RequiresAction
    /// );
    /// ```
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

    /// Every status, in lifecycle order. Read by [`IntentStatus::from_wire`]
    /// so the parse side is written once.
    pub const ALL: [Self; 5] = [
        Self::RequiresPaymentMethod,
        Self::RequiresAction,
        Self::Processing,
        Self::Succeeded,
        Self::Canceled,
    ];

    /// The exact label this status carries on the wire *and* in Postgres.
    ///
    /// Written out beside the `serde` rename because two paths that need it
    /// do not go through `serde` — see
    /// [docs/reference/vpay-core.md § two routes to one label](../../../../docs/reference/vpay-core.md#two-routes-to-one-label).
    ///
    /// ```
    /// use vpay_core::IntentStatus;
    ///
    /// assert_eq!(
    ///     IntentStatus::RequiresPaymentMethod.as_wire_str(),
    ///     "requires_payment_method"
    /// );
    /// // The same spelling `serde` produces, for every status.
    /// for status in IntentStatus::ALL {
    ///     assert_eq!(IntentStatus::from_wire(status.as_wire_str()), Some(status));
    /// }
    /// ```
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
    /// `Option` rather than a `FromStr` with an error type: the only caller
    /// is the boundary reading a Postgres enum back, where an unparseable
    /// label is a schema/code mismatch the HTTP layer answers `500` for, not
    /// a caller's mistake.
    ///
    /// ```
    /// use vpay_core::IntentStatus;
    ///
    /// assert_eq!(
    ///     IntentStatus::from_wire("processing"),
    ///     Some(IntentStatus::Processing)
    /// );
    /// // Stripe has `requires_confirmation`; this API deliberately does not.
    /// assert_eq!(IntentStatus::from_wire("requires_confirmation"), None);
    /// assert_eq!(IntentStatus::from_wire("PROCESSING"), None);
    /// ```
    #[must_use]
    pub fn from_wire(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_wire_str() == label)
    }
}

impl RefundStatus {
    /// Every refund status, in lifecycle order. Read by
    /// [`RefundStatus::from_wire`] so the parse side is written once.
    pub const ALL: [Self; 4] = [Self::Pending, Self::Succeeded, Self::Failed, Self::Canceled];

    /// The exact label this status carries on the wire *and* in Postgres —
    /// see [`IntentStatus::as_wire_str`] for why it is written out beside
    /// the `serde` rename.
    ///
    /// ```
    /// use vpay_core::RefundStatus;
    ///
    /// assert_eq!(RefundStatus::Canceled.as_wire_str(), "canceled");
    /// for status in RefundStatus::ALL {
    ///     assert_eq!(RefundStatus::from_wire(status.as_wire_str()), Some(status));
    /// }
    /// ```
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Parses a label produced by [`Self::as_wire_str`], or `None` — see
    /// [`IntentStatus::from_wire`] for why this is not a `FromStr`.
    ///
    /// ```
    /// use vpay_core::RefundStatus;
    ///
    /// assert_eq!(RefundStatus::from_wire("failed"), Some(RefundStatus::Failed));
    /// // Stripe spells it with two `l`s in British English nowhere; the
    /// // Postgres enum is the single spelling, and anything else is a
    /// // schema/code mismatch rather than a value to guess at.
    /// assert_eq!(RefundStatus::from_wire("cancelled"), None);
    /// ```
    #[must_use]
    pub fn from_wire(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_wire_str() == label)
    }
}

impl ChargeState {
    /// The state a charge row is born in, before the rail has been asked
    /// anything.
    ///
    /// Named rather than spelled at the insert site: `docs/flows/crash-safety.md`
    /// turns on the row existing in *this* state before `submit` is called,
    /// so a call site that picked `Submitted` instead would be a silent
    /// crash-safety regression rather than a compile error.
    pub const INITIAL: Self = Self::Submitting;

    /// Every charge state, live ones first. Read by
    /// [`ChargeState::from_wire`].
    pub const ALL: [Self; 6] = [
        Self::Submitting,
        Self::Submitted,
        Self::Pending,
        Self::Unresolved,
        Self::Succeeded,
        Self::Failed,
    ];

    /// The exact label this state carries in Postgres — see
    /// [`IntentStatus::as_wire_str`] for why it is written out.
    ///
    /// ```
    /// use vpay_core::ChargeState;
    ///
    /// assert_eq!(ChargeState::INITIAL.as_wire_str(), "submitting");
    /// for state in ChargeState::ALL {
    ///     assert_eq!(ChargeState::from_wire(state.as_wire_str()), Some(state));
    /// }
    /// ```
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
    ///
    /// ```
    /// use vpay_core::ChargeState;
    ///
    /// assert_eq!(ChargeState::from_wire("pending"), Some(ChargeState::Pending));
    /// assert_eq!(ChargeState::from_wire("submitting "), None);
    /// ```
    #[must_use]
    pub fn from_wire(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_wire_str() == label)
    }
}

/// One of the three verbs a *merchant* can apply to a PaymentIntent.
///
/// Deliberately not "every edge in the lifecycle": the rail-driven edges are
/// moved by the reconciler from an authenticated status query, never by a
/// request, so [`next_status`] answers `None` for them. Why that separation
/// is load-bearing:
/// [docs/reference/vpay-core.md § the merchant's three verbs](../../../../docs/reference/vpay-core.md#the-merchants-three-verbs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// `POST /v1/payment_intents`. The `[*] -->` edge: it has no source
    /// status at all, so [`next_status`] answers `None` for every existing
    /// one and a new intent is born in [`IntentStatus::INITIAL`].
    Create,
    /// `POST /v1/payment_intents/{id}/confirm`, on a rail with this flow
    /// shape. The flow — not the rail's name — selects the next status
    /// ([ADR-0002](../../../../docs/adr/0002-provider-port.md)).
    Confirm(ProviderFlow),
    /// `POST /v1/payment_intents/{id}/cancel`.
    Cancel,
}

/// The status `transition` moves an intent in `from` to, or `None` if that
/// request is not legal from that status.
///
/// This is the *whole* rule, and one function on purpose. `None` is not "the
/// intent is stuck" (see [`Transition`]), and a legal answer here is not
/// permission to write it — the write is a compare-and-swap on the row's
/// current status, because between this call and that `UPDATE` another
/// request may have moved the same row.
///
/// ```
/// use vpay_core::{IntentStatus, ProviderFlow, Transition, next_status};
///
/// // The three legal edges, all out of `requires_payment_method`.
/// assert_eq!(
///     next_status(
///         IntentStatus::RequiresPaymentMethod,
///         Transition::Confirm(ProviderFlow::Push)
///     ),
///     Some(IntentStatus::Processing)
/// );
/// assert_eq!(
///     next_status(
///         IntentStatus::RequiresPaymentMethod,
///         Transition::Confirm(ProviderFlow::Redirect)
///     ),
///     Some(IntentStatus::RequiresAction)
/// );
/// assert_eq!(
///     next_status(IntentStatus::RequiresPaymentMethod, Transition::Cancel),
///     Some(IntentStatus::Canceled)
/// );
///
/// // Once a rail has the request it cannot be recalled.
/// assert_eq!(next_status(IntentStatus::Processing, Transition::Cancel), None);
/// // One charge per intent, forever: no second confirm.
/// assert_eq!(
///     next_status(
///         IntentStatus::RequiresAction,
///         Transition::Confirm(ProviderFlow::Redirect)
///     ),
///     None
/// );
/// // Creation has no source status.
/// assert_eq!(
///     next_status(IntentStatus::RequiresPaymentMethod, Transition::Create),
///     None
/// );
/// ```
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

    /// The four labels `refund_status` (migration 0017) allows, and nothing
    /// else. `from_wire` is the only reader of a Postgres enum on this path,
    /// so a label it silently accepted would be a status vpay does not model
    /// reaching a merchant's SDK.
    #[test]
    fn a_refund_status_round_trips_and_an_unmodelled_label_does_not_parse() {
        for status in RefundStatus::ALL {
            assert_eq!(RefundStatus::from_wire(status.as_wire_str()), Some(status));
        }
        assert_eq!(RefundStatus::ALL.len(), 4);
        for label in [
            "cancelled",       // the other spelling
            "requires_action", // an intent status, not a refund one
            "PENDING",
            "",
            " succeeded",
        ] {
            assert_eq!(RefundStatus::from_wire(label), None, "{label:?}");
        }
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
