//! Recovering a charge stuck in `submitting`, and the policy numbers that
//! decision reads.
//!
//! [`recovery_step`] is `docs/flows/crash-safety.md`'s "Recovering a
//! `submitting` charge" table as one pure function. The whole of it is a
//! disambiguation: `submitting` covers "we crashed before the POST" and "the
//! POST went out and the answer was lost", and `provider_requests` is the only
//! evidence that tells them apart.
//!
//! The branch is on [`ProviderFlow`], a capability *value*, never on a rail
//! code (ADR-0002). `docs/reference/vpay-worker.md` §"Recovering a
//! `submitting` charge" says why the flow shape decides first, why the
//! precondition is a precondition, and why [`RecoveryPolicy`] is a plain
//! struct rather than a test seam.

use std::time::Duration;

use time::OffsetDateTime;
use vpay_core::ProviderFlow;

/// The knobs the recovery table reads, constructed once in `main` and passed
/// down to every handler.
///
/// A plain struct with a [`Default`], deliberately not a `#[cfg(test)]` seam —
/// see `docs/reference/vpay-worker.md` §"Recovering a `submitting` charge".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPolicy {
    /// How many consecutive `NotFound` answers count as "the rail never
    /// received it". Three, per `docs/flows/crash-safety.md`.
    ///
    /// Paired with [`Self::not_found_window`], and both must be satisfied: a
    /// count alone cannot tell a rail that never got the charge from one that
    /// is merely slow to index it.
    pub not_found_streak: u32,
    /// How long those `NotFound` answers must have been going on. Sixty
    /// seconds, per the same table's "3 consecutive `NotFound` over ≥60s".
    pub not_found_window: Duration,
    /// How long a claimed job may stay claimed before the reaper hands it
    /// back. At least four times `vpay_provider::DEFAULT_REQUEST_TIMEOUT`, so
    /// a worker merely waiting on a slow rail never has its job stolen and run
    /// twice.
    pub lease: Duration,
    /// How long a charge may stay live before it is escalated to `unresolved`
    /// and a human is alerted. Twenty-four hours, per
    /// `docs/flows/reconciler.md`.
    ///
    /// Escalation is not a give-up and not a decision to stop asking: past
    /// this horizon a poll still queries the rail, and a terminal answer
    /// settles the charge through the ordinary path. What re-raises the alert
    /// is every outcome short of a settlement —
    /// `docs/reference/vpay-worker.md` §"One poll" lists them, and the two
    /// that deliberately do not.
    pub unresolved_after: Duration,
}

impl Default for RecoveryPolicy {
    /// The documented numbers, and the only ones any deployment has asked
    /// for. Spelled as arithmetic on `from_secs` rather than as raw second
    /// counts so a reader checks them against the documents without a
    /// calculator.
    ///
    /// ```
    /// use std::time::Duration;
    /// use vpay_worker::RecoveryPolicy;
    ///
    /// let policy = RecoveryPolicy::default();
    /// // docs/flows/crash-safety.md: 3 consecutive NotFound over >= 60s.
    /// assert_eq!(policy.not_found_streak, 3);
    /// assert_eq!(policy.not_found_window, Duration::from_secs(60));
    /// // A lease of at least four rail request timeouts, so a worker merely
    /// // waiting on a slow rail never has its job stolen and run twice.
    /// assert_eq!(policy.lease, Duration::from_secs(5 * 60));
    /// assert!(policy.lease >= 4 * vpay_provider::DEFAULT_REQUEST_TIMEOUT);
    /// // docs/flows/reconciler.md: the 24-hour horizon.
    /// assert_eq!(policy.unresolved_after, Duration::from_secs(24 * 60 * 60));
    /// ```
    fn default() -> Self {
        Self {
            not_found_streak: 3,
            not_found_window: Duration::from_secs(60),
            lease: Duration::from_secs(5 * 60),
            unresolved_after: Duration::from_secs(24 * 60 * 60),
        }
    }
}

/// What `provider_requests` says about the most recent attempt to *submit*
/// this charge.
///
/// Three values because the recovery table has three rows, and the middle one
/// is the reason the table exists at all: `status_code IS NULL` is the
/// encoding for "the POST was issued and no answer was received" (migration
/// 0016's `response_is_paired` CHECK keeps that honest), which is materially
/// different from both "we never sent anything" and "the rail answered".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitAttempt {
    /// No `provider_requests` row with `operation = 'submit'` for this
    /// charge. We crashed between committing the charge and issuing the POST.
    Never,
    /// A row exists and carries no status. The POST was issued; the answer
    /// was lost.
    Unanswered,
    /// A row exists and carries a status — including
    /// `vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT`
    /// (the `0` sentinel for "the rail answered and the port does not carry
    /// its status line"). The rail answered; we crashed before recording what
    /// it meant.
    Answered(i32),
}

/// What to do with a charge stuck in `submitting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Fail the charge (`provider_unavailable`) and put the intent back to
    /// `requires_payment_method`. Reached only on a redirect rail — see this
    /// module's header for why that is safe there and nowhere else.
    FailDeadOrder,
    /// Submit again with the **same** `provider_reference_id`. Never a fresh
    /// one: "a fresh reference on retry is how you double-charge a customer"
    /// (`docs/flows/crash-safety.md`), and the port's contract — a duplicate
    /// submission is reported as `Submitted`, not as an error — is what makes
    /// this safe to do even when the rail did receive the first one.
    Resubmit,
    /// Ask the rail. The ambiguity resolves toward "find out", never toward
    /// "give up".
    Poll,
    /// The rail answered the submit; only our own bookkeeping is behind.
    /// Advance the charge out of `submitting` and carry on polling. Carries
    /// the status code purely so the log line names the evidence.
    Advance(i32),
}

/// The recovery table, as a total function.
///
/// # Precondition: the charge is in `submitting`
///
/// It must not be used for a charge that has reached `submitted`, and the
/// reason is [`RecoveryAction::FailDeadOrder`] — see
/// `docs/reference/vpay-worker.md` §"Recovering a `submitting` charge". A
/// `NotFound` past `submitting` is handled as an ordinary pending answer by
/// the caller, through [`vpay_core::Settlement::Stay`].
///
/// `now` and `first_not_found_at` are parameters rather than a call to the
/// clock so that this stays pure and the table below can pin the window
/// boundary exactly.
///
/// ```
/// use time::{Duration, OffsetDateTime};
/// use vpay_core::ProviderFlow;
/// use vpay_worker::{RecoveryAction, RecoveryPolicy, SubmitAttempt, recovery_step};
///
/// let policy = RecoveryPolicy::default();
/// let now = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
///
/// // The flow shape decides first and unconditionally: on a redirect rail
/// // the payer was never handed a URL, so the order is dead whatever the
/// // attempt row says.
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Redirect,
///         SubmitAttempt::Answered(200),
///         0,
///         None,
///         now,
///         &policy,
///     ),
///     RecoveryAction::FailDeadOrder,
/// );
///
/// // Push rail, nothing was ever sent: same reference, second attempt.
/// assert_eq!(
///     recovery_step(ProviderFlow::Push, SubmitAttempt::Never, 0, None, now, &policy),
///     RecoveryAction::Resubmit,
/// );
///
/// // The rail answered; only our own bookkeeping is behind.
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Push,
///         SubmitAttempt::Answered(0),
///         0,
///         None,
///         now,
///         &policy,
///     ),
///     RecoveryAction::Advance(0),
/// );
///
/// // The POST went out and the answer was lost. The streak and the window
/// // are both required, never either: three denials inside a minute is a
/// // rail that is merely slow to index, so the answer is still "ask again".
/// let recent = now - Duration::seconds(30);
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Push,
///         SubmitAttempt::Unanswered,
///         policy.not_found_streak,
///         Some(recent),
///         now,
///         &policy,
///     ),
///     RecoveryAction::Poll,
/// );
/// let old = now - Duration::seconds(90);
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Push,
///         SubmitAttempt::Unanswered,
///         policy.not_found_streak,
///         Some(old),
///         now,
///         &policy,
///     ),
///     RecoveryAction::Resubmit,
/// );
/// ```
#[must_use]
pub fn recovery_step(
    flow: ProviderFlow,
    latest_submit_attempt: SubmitAttempt,
    not_found_streak: u32,
    first_not_found_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
    policy: &RecoveryPolicy,
) -> RecoveryAction {
    // First and unconditionally: on a redirect rail there is nothing to ask
    // and nothing to lose. This is `docs/flows/crash-safety.md`'s "that
    // `order_id` is dead: abandon it", and it holds whatever the attempt row
    // says — the `pay_token` the rail needs was in the response, so even an
    // *answered* submit we failed to persist leaves a charge no one can query
    // and a payer who was never redirected.
    if matches!(flow, ProviderFlow::Redirect) {
        return RecoveryAction::FailDeadOrder;
    }

    match latest_submit_attempt {
        // Nothing was ever sent. Same reference, second attempt.
        SubmitAttempt::Never => RecoveryAction::Resubmit,
        SubmitAttempt::Unanswered => {
            // Unreachable for any `Duration` a policy can hold (`time::Duration`
            // spans ~292 billion years); `MAX` rather than `ZERO` as the
            // fallback so a conversion that somehow failed would *delay* a
            // resubmit rather than trigger one early.
            let window =
                time::Duration::try_from(policy.not_found_window).unwrap_or(time::Duration::MAX);
            // Both conditions, never either: see `RecoveryPolicy::not_found_streak`.
            let long_enough = first_not_found_at.is_some_and(|first| now - first >= window);
            if not_found_streak >= policy.not_found_streak && long_enough {
                RecoveryAction::Resubmit
            } else {
                RecoveryAction::Poll
            }
        }
        // The rail answered; our write is what is missing.
        SubmitAttempt::Answered(code) => RecoveryAction::Advance(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An arbitrary fixed instant. Nothing depends on which one it is; the
    /// tests move relative to it.
    const NOW: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

    fn policy() -> RecoveryPolicy {
        RecoveryPolicy::default()
    }

    /// Long enough ago to satisfy `not_found_window` at the default policy.
    fn well_inside_the_window() -> Option<OffsetDateTime> {
        Some(NOW - time::Duration::seconds(90))
    }

    #[test]
    fn the_default_policy_is_the_documented_numbers() {
        let p = RecoveryPolicy::default();
        assert_eq!(p.not_found_streak, 3, "crash-safety.md: 3 consecutive");
        assert_eq!(
            p.not_found_window,
            Duration::from_secs(60),
            "crash-safety.md: over >=60s"
        );
        assert_eq!(
            p.lease,
            Duration::from_secs(300),
            "step 4 design: 5 minutes, >= 4x the 20s request timeout"
        );
        assert!(
            p.lease >= 4 * vpay_provider::DEFAULT_REQUEST_TIMEOUT,
            "a lease shorter than four rail timeouts lets a slow job be run twice"
        );
        assert_eq!(
            p.unresolved_after,
            Duration::from_secs(86_400),
            "reconciler.md: at 24 hours still pending"
        );
    }

    /// The whole table for a push rail, as data. `first_not_found_at` and the
    /// streak only matter in the `Unanswered` row, which is exactly what
    /// `docs/flows/crash-safety.md` says.
    #[test]
    fn the_push_recovery_table_is_the_document() {
        let p = policy();
        let cases: [(SubmitAttempt, u32, Option<OffsetDateTime>, RecoveryAction); 6] = [
            // No evidence of a POST at all.
            (SubmitAttempt::Never, 0, None, RecoveryAction::Resubmit),
            // Even a long NotFound streak does not change the "never sent"
            // answer: it is already the strongest one.
            (
                SubmitAttempt::Never,
                9,
                well_inside_the_window(),
                RecoveryAction::Resubmit,
            ),
            // POST issued, answer lost: ask.
            (SubmitAttempt::Unanswered, 0, None, RecoveryAction::Poll),
            // Streak reached, window satisfied: treat as never received.
            (
                SubmitAttempt::Unanswered,
                3,
                well_inside_the_window(),
                RecoveryAction::Resubmit,
            ),
            // The rail answered; only our bookkeeping is behind.
            (
                SubmitAttempt::Answered(0),
                0,
                None,
                RecoveryAction::Advance(0),
            ),
            (
                SubmitAttempt::Answered(202),
                5,
                well_inside_the_window(),
                RecoveryAction::Advance(202),
            ),
        ];
        for (attempt, streak, first, expected) in cases {
            assert_eq!(
                recovery_step(ProviderFlow::Push, attempt, streak, first, NOW, &p),
                expected,
                "recovery_step(Push, {attempt:?}, streak {streak})"
            );
        }
    }

    /// Both conditions, and each one alone is not enough. This is the test
    /// that fails if someone "simplifies" the `&&` into an `||`.
    #[test]
    fn a_resubmit_needs_the_streak_and_the_window() {
        let p = policy();
        let step = |streak, first| {
            recovery_step(
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                streak,
                first,
                NOW,
                &p,
            )
        };
        // Streak reached, but the first NotFound was 59s ago: too fast to
        // conclude the rail never received it.
        assert_eq!(
            step(3, Some(NOW - time::Duration::seconds(59))),
            RecoveryAction::Poll
        );
        // Window satisfied, but only two NotFound answers so far.
        assert_eq!(step(2, well_inside_the_window()), RecoveryAction::Poll);
        // Exactly on the boundary, both ways: ">= 60s" and ">= 3".
        assert_eq!(
            step(3, Some(NOW - time::Duration::seconds(60))),
            RecoveryAction::Resubmit
        );
        // A streak with no recorded start is not a window at all.
        assert_eq!(step(99, None), RecoveryAction::Poll);
    }

    /// S2 of the design: a redirect charge stuck in `submitting` is failed,
    /// on every shape of evidence, because the payer was never handed a URL
    /// and the token needed to ask the rail is gone.
    #[test]
    fn a_redirect_rail_never_polls_a_stuck_submission() {
        let p = policy();
        for attempt in [
            SubmitAttempt::Never,
            SubmitAttempt::Unanswered,
            SubmitAttempt::Answered(0),
            SubmitAttempt::Answered(201),
        ] {
            for (streak, first) in [(0, None), (9, well_inside_the_window())] {
                assert_eq!(
                    recovery_step(ProviderFlow::Redirect, attempt, streak, first, NOW, &p),
                    RecoveryAction::FailDeadOrder,
                    "recovery_step(Redirect, {attempt:?}, streak {streak})"
                );
            }
        }
    }

    /// A push rail is never dead-lettered by this function: every push answer
    /// is one of "ask", "send it again", or "catch our own bookkeeping up".
    #[test]
    fn a_push_rail_is_never_abandoned() {
        let p = policy();
        for attempt in [
            SubmitAttempt::Never,
            SubmitAttempt::Unanswered,
            SubmitAttempt::Answered(0),
        ] {
            for streak in 0..5 {
                let action = recovery_step(
                    ProviderFlow::Push,
                    attempt,
                    streak,
                    well_inside_the_window(),
                    NOW,
                    &p,
                );
                assert_ne!(
                    action,
                    RecoveryAction::FailDeadOrder,
                    "a push charge was abandoned on {attempt:?} at streak {streak}"
                );
            }
        }
    }

    /// A policy an integration test can satisfy without sleeping still
    /// behaves like the production one — same code path, different numbers
    /// (AGENTS.md rule 1: no test seam).
    #[test]
    fn a_tightened_policy_resubmits_without_waiting_a_minute() {
        let p = RecoveryPolicy {
            not_found_streak: 3,
            not_found_window: Duration::from_millis(50),
            ..RecoveryPolicy::default()
        };
        assert_eq!(
            recovery_step(
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                3,
                Some(NOW - time::Duration::milliseconds(50)),
                NOW,
                &p,
            ),
            RecoveryAction::Resubmit
        );
        assert_eq!(
            recovery_step(
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                3,
                Some(NOW - time::Duration::milliseconds(49)),
                NOW,
                &p,
            ),
            RecoveryAction::Poll
        );
    }
}
