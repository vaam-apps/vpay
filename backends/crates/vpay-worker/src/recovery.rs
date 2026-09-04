//! Recovering a charge stuck in `submitting`, and the policy numbers that
//! decision reads.
//!
//! [`recovery_step`] is `docs/flows/crash-safety.md`'s "Recovering a
//! `submitting` charge" table as one pure function. The whole of it is a
//! disambiguation: `submitting` covers "we crashed before the POST" and "the
//! POST went out and the answer was lost", and `provider_requests` is the only
//! evidence that tells them apart.
//!
//! Every duration this module reads is measured by **Postgres**, never by the
//! worker's host clock: the ages arrive as `time::Duration`s and there is no
//! instant in [`recovery_step`]'s signature at all — see that function's
//! §"Both ages are measured by one clock".
//!
//! The branch is on [`ProviderFlow`], a capability *value*, never on a rail
//! code (ADR-0002). `docs/reference/vpay-worker.md` §"Recovering a
//! `submitting` charge" says why the flow shape decides first, why the
//! precondition is a precondition, why nothing is recovered until the charge
//! is older than [`RecoveryPolicy::not_found_window`], and why
//! [`RecoveryPolicy`] is a plain struct rather than a test seam.

use std::time::Duration;

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
    ///
    /// It is also the **minimum age of a `submitting` charge** before
    /// [`recovery_step`] will recover it at all — see that function's
    /// "Nothing younger than the window is recovered". One number for both,
    /// because both answer the same question: how long must a state have
    /// persisted before it stops being explicable by something still in
    /// flight?
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
    /// Do nothing, and come back later: the charge has not been `submitting`
    /// long enough for that state to be evidence of a crash. Carries **how
    /// much longer** it has to wait.
    ///
    /// The only action that touches neither the charge nor the rail. It is
    /// not "poll" — a poll writes a `provider_requests` row and asks a rail
    /// about a submission that may be in flight at that very moment — and it
    /// is not "give up". See [`recovery_step`] §"Nothing younger than the
    /// window is recovered" for what it prevents.
    ///
    /// # Why it carries a duration rather than leaving the delay to the caller
    ///
    /// Because a reschedule is not free. `vpay_db::Jobs::claim` increments
    /// `jobs.attempts`, and [`crate::poll_delay`] is indexed by that count,
    /// so every time the worker comes back and waits again it spends a rung
    /// of the ladder. The first shape of this arm rescheduled at
    /// `poll_delay(0)` — ten seconds — which took *six* claims to cross a
    /// sixty-second window and left a genuinely crashed charge starting its
    /// real recovery at `poll_delay(6)`, two minutes a rung, having already
    /// burned the fast end of the ladder on doing nothing.
    ///
    /// One wait of exactly the remaining time costs one rung instead of six:
    /// the first real recovery pass runs at the second claim, and the rung
    /// after it is `poll_delay(1)`, twenty seconds. That holds when nothing
    /// else claims the job: a parked `Wait` is still eligible for the
    /// callback route's pull-forward (its floor is ten seconds, the wait is
    /// up to sixty), so a rail's duplicate notifications — or anyone holding
    /// the reference — can force extra claims inside the window. Each one
    /// asks the rail nothing and moves no state; it only spends an attempt,
    /// so the ladder rung after the window is later than twenty seconds.
    /// Nothing is lost by it — the 24 h horizon still escalates — and it is
    /// recorded in `docs/reference/vpay-worker.md` rather than clamped here.
    ///
    /// The duration is `not_found_window - charge_age`, clamped into
    /// `[0, not_found_window]` — never negative, and never longer than the
    /// window itself, because a row whose `created_at` is *ahead* of the
    /// database's `now()` would otherwise park its job for however far the
    /// skew reaches. `docs/reference/vpay-worker.md` §"Nothing younger than
    /// the window is recovered".
    Wait(Duration),
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
/// # Nothing younger than the window is recovered
///
/// `submitting` is not only the state a crash leaves behind. It is also the
/// ordinary state of a **confirm that is still running**: `vpay-api` commits
/// the charge and its `poll_charge` job in one transaction with
/// `run_at = now()`, calls the rail, and only then compare-and-swaps
/// `submitting → submitted`. A worker that claims that job in between sees
/// exactly the rows a crash leaves, and every branch below would move the
/// charge out from under a live confirm — which is not a hypothetical: Step
/// 8's demo hit it in four runs of six on a loaded machine, answering the
/// merchant `500 write_matched_no_row` while delivering a
/// `payment_intent.succeeded` webhook on a push rail, and killing a live
/// order as `provider_unavailable` on a redirect one.
///
/// So the first question is not "what does the evidence say" but "is this
/// charge old enough for the evidence to mean anything", and the answer is
/// [`RecoveryPolicy::not_found_window`] — the same 60 seconds the
/// `Unanswered` row already waits before concluding a rail never received a
/// charge, and three times the 20-second `vpay_provider::DEFAULT_REQUEST_TIMEOUT`
/// that bounds how long a confirm can be inside the rail call. Younger than
/// that, the answer is [`RecoveryAction::Wait`] whatever the flow shape and
/// whatever `provider_requests` holds. The cost is that a charge orphaned by
/// a genuine crash waits up to a minute for its first recovery pass; the
/// charge is live and polled either way, and nothing about it is lost by
/// asking a minute later.
///
/// **The clock is `charges.created_at`**, not the first
/// `provider_requests.sent_at`, for a reason that decides it: the branch this
/// most has to protect is `SubmitAttempt::Never`, where there is no attempt
/// row *at all*, and a clock that reads `None` exactly in the case where it is
/// needed is not a clock. `created_at` is also written by Postgres' own
/// `now()` inside the confirm's first transaction — the one clock every
/// replica shares, before any network call by construction
/// (`vpay_db::NewCharge`) — so it dates the window from the moment the race
/// opens rather than from a row written some way into it. It is the same
/// column `past_the_horizon` measures the 24-hour escalation from.
///
/// # Both ages are measured by one clock
///
/// `charge_age` and `not_found_streak_age` are **durations**, not a pair of
/// instants and a `now`, and that shape is a fix rather than a preference.
/// This function used to take `now: OffsetDateTime` — which the caller read
/// from the *worker host's* clock — and subtract `charges.created_at`, which
/// Postgres had written. Those are two clocks. A worker sixty seconds ahead
/// of the database measured every charge as a minute older than it was, so
/// the guard above passed for every live confirm and became a silent no-op,
/// on precisely the deployment whose fleet clocks had drifted. The same
/// subtraction sat under the 24-hour horizon.
///
/// Taking ages makes that unrepresentable here: there is no instant in this
/// signature for a caller to read off the wrong clock, and the age comes from
/// `vpay_db::ChargeAsOf`, where Postgres' `now()` is selected by the same
/// statement that reads `created_at`. The parameters are values rather than
/// clock calls for the older reason too — this stays pure, and the table
/// below pins both boundaries exactly.
///
/// ```
/// use time::Duration;
/// use vpay_core::ProviderFlow;
/// use vpay_worker::{RecoveryAction, RecoveryPolicy, SubmitAttempt, recovery_step};
///
/// let policy = RecoveryPolicy::default();
/// // Older than `not_found_window`, so the table below applies at all.
/// let old_enough = Duration::seconds(90);
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
///         old_enough,
///         &policy,
///     ),
///     RecoveryAction::FailDeadOrder,
/// );
///
/// // Push rail, nothing was ever sent: same reference, second attempt.
/// assert_eq!(
///     recovery_step(ProviderFlow::Push, SubmitAttempt::Never, 0, None, old_enough, &policy),
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
///         old_enough,
///         &policy,
///     ),
///     RecoveryAction::Advance(0),
/// );
///
/// // The same charge a second after it was opened: a confirm may still be
/// // holding it, so nothing is recovered and nothing is asked.
/// let just_opened = Duration::seconds(1);
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Push,
///         SubmitAttempt::Answered(0),
///         0,
///         None,
///         just_opened,
///         &policy,
///     ),
///     // A second old, so fifty-nine of the sixty are left to wait: one
///     // reschedule, not six rungs of the ladder.
///     RecoveryAction::Wait(std::time::Duration::from_secs(59)),
/// );
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Redirect,
///         SubmitAttempt::Answered(200),
///         0,
///         None,
///         just_opened,
///         &policy,
///     ),
///     RecoveryAction::Wait(std::time::Duration::from_secs(59)),
/// );
///
/// // The POST went out and the answer was lost. The streak and the window
/// // are both required, never either: three denials inside a minute is a
/// // rail that is merely slow to index, so the answer is still "ask again".
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Push,
///         SubmitAttempt::Unanswered,
///         policy.not_found_streak,
///         Some(Duration::seconds(30)),
///         old_enough,
///         &policy,
///     ),
///     RecoveryAction::Poll,
/// );
/// assert_eq!(
///     recovery_step(
///         ProviderFlow::Push,
///         SubmitAttempt::Unanswered,
///         policy.not_found_streak,
///         Some(Duration::seconds(90)),
///         old_enough,
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
    not_found_streak_age: Option<time::Duration>,
    charge_age: time::Duration,
    policy: &RecoveryPolicy,
) -> RecoveryAction {
    let window = window(policy);

    // Before the flow shape and before the evidence: a charge that has been
    // `submitting` for less than the window is one whose confirm may still be
    // running, and none of the branches below are true of it. See this
    // function's §"Nothing younger than the window is recovered".
    //
    // Strictly younger, so recovery becomes legal *at* the window — the same
    // boundary the streak's own comparison below draws, where crash-safety.md's
    // "over >=60s" is satisfied at sixty seconds rather than after them.
    if charge_age < window {
        return RecoveryAction::Wait(remaining(window, charge_age));
    }

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
            // Both conditions, never either: see `RecoveryPolicy::not_found_streak`.
            let long_enough = not_found_streak_age.is_some_and(|age| age >= window);
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

/// How much longer a charge this young has to wait, for
/// [`RecoveryAction::Wait`].
///
/// `window - charge_age`, clamped into `[0, window]`. Both ends of that clamp
/// are load-bearing rather than defensive: the lower one is what the `<`
/// comparison above already guarantees and costs nothing to state, and the
/// upper one bounds the damage a **negative age** can do. A charge whose
/// `created_at` is ahead of the `now()` read beside it — a replica whose
/// clock leads the primary's — has an age below zero, and an unclamped
/// subtraction would schedule its next poll that far into the future, parking
/// a live charge for as long as the skew lasts. Waiting one window and asking
/// again is the answer that cannot lose a payment.
fn remaining(window: time::Duration, charge_age: time::Duration) -> Duration {
    let left = window
        .saturating_sub(charge_age)
        .clamp(time::Duration::ZERO, window);
    // Unreachable for a clamped, non-negative `time::Duration`; `ZERO` is the
    // right answer anyway, since it means "claimable now".
    Duration::try_from(left).unwrap_or(Duration::ZERO)
}

/// [`RecoveryPolicy::not_found_window`] as a `time::Duration`.
///
/// One conversion for both of the window's jobs — the minimum charge age and
/// the `NotFound` streak's span — so the two cannot drift on the saturation
/// behaviour. `MAX` rather than `ZERO` as the fallback, which is unreachable
/// for any `Duration` a policy can hold (`time::Duration` spans ~292 billion
/// years) and is the safe direction in both readings: a conversion that
/// somehow failed would make every charge "too young to recover" and every
/// streak "too short to resubmit", never the reverse.
fn window(policy: &RecoveryPolicy) -> time::Duration {
    time::Duration::try_from(policy.not_found_window).unwrap_or(time::Duration::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RecoveryPolicy {
        RecoveryPolicy::default()
    }

    /// A `NotFound` streak that has been going on long enough to satisfy
    /// `not_found_window` at the default policy.
    fn well_inside_the_window() -> Option<time::Duration> {
        Some(time::Duration::seconds(90))
    }

    /// A charge opened long enough ago that the age guard is out of the way,
    /// at the default policy and at every tightened one below.
    ///
    /// Every case whose subject is the *evidence* passes this, so that the
    /// table cases and the age cases fail for different reasons.
    const OLD_ENOUGH: time::Duration = time::Duration::seconds(90);

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

    /// The whole table for a push rail, as data. `not_found_streak_age` and
    /// the streak only matter in the `Unanswered` row, which is exactly what
    /// `docs/flows/crash-safety.md` says.
    #[test]
    fn the_push_recovery_table_is_the_document() {
        let p = policy();
        let cases: [(SubmitAttempt, u32, Option<time::Duration>, RecoveryAction); 6] = [
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
        for (attempt, streak, streak_age, expected) in cases {
            assert_eq!(
                recovery_step(
                    ProviderFlow::Push,
                    attempt,
                    streak,
                    streak_age,
                    OLD_ENOUGH,
                    &p
                ),
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
        let step = |streak, streak_age| {
            recovery_step(
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                streak,
                streak_age,
                OLD_ENOUGH,
                &p,
            )
        };
        // Streak reached, but the first NotFound was 59s ago: too fast to
        // conclude the rail never received it.
        assert_eq!(
            step(3, Some(time::Duration::seconds(59))),
            RecoveryAction::Poll
        );
        // Window satisfied, but only two NotFound answers so far.
        assert_eq!(step(2, well_inside_the_window()), RecoveryAction::Poll);
        // Exactly on the boundary, both ways: ">= 60s" and ">= 3".
        assert_eq!(
            step(3, Some(time::Duration::seconds(60))),
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
            for (streak, streak_age) in [(0, None), (9, well_inside_the_window())] {
                assert_eq!(
                    recovery_step(
                        ProviderFlow::Redirect,
                        attempt,
                        streak,
                        streak_age,
                        OLD_ENOUGH,
                        &p
                    ),
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
                    OLD_ENOUGH,
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

    /// The age guard, as a table: every branch of the recovery decision,
    /// below the window, exactly at it, and above it.
    ///
    /// This is the case that fails if the guard is deleted, and it is written
    /// over *all four* answers the table can otherwise give — `Resubmit`
    /// (`Never`), `Poll` and `Resubmit` (`Unanswered`), `Advance`
    /// (`Answered`) and `FailDeadOrder` (any evidence on a redirect rail) —
    /// because the race it prevents does not care which one would have run:
    /// each of them writes to a charge whose confirm may still be inside its
    /// rail call. `docs/plans/step8-notes/lane-g.md`.
    #[test]
    fn nothing_younger_than_the_window_is_recovered() {
        let p = policy();
        // The evidence, and what it means once the charge is old enough.
        let branches: [(ProviderFlow, SubmitAttempt, u32, RecoveryAction); 5] = [
            (
                ProviderFlow::Push,
                SubmitAttempt::Never,
                0,
                RecoveryAction::Resubmit,
            ),
            (
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                0,
                RecoveryAction::Poll,
            ),
            (
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                p.not_found_streak,
                RecoveryAction::Resubmit,
            ),
            (
                ProviderFlow::Push,
                SubmitAttempt::Answered(0),
                0,
                RecoveryAction::Advance(0),
            ),
            (
                ProviderFlow::Redirect,
                SubmitAttempt::Answered(201),
                0,
                RecoveryAction::FailDeadOrder,
            ),
        ];
        // Ages, and whether the table applies at that age. The boundary is
        // `>=`, the same comparison the streak's own window uses.
        let ages: [(time::Duration, bool); 4] = [
            (time::Duration::ZERO, false),
            (time::Duration::seconds(59), false),
            (time::Duration::seconds(60), true),
            (time::Duration::seconds(61), true),
        ];

        for (flow, attempt, streak, recovered) in branches {
            for (age, old_enough) in ages {
                let expected = if old_enough {
                    recovered
                } else {
                    RecoveryAction::Wait(Duration::from_secs(60).saturating_sub(age.unsigned_abs()))
                };
                assert_eq!(
                    recovery_step(flow, attempt, streak, well_inside_the_window(), age, &p),
                    expected,
                    "recovery_step({flow:?}, {attempt:?}, streak {streak}) at age {age}"
                );
            }
        }
    }

    /// A wait is the **rest of the window**, not a rung of the poll ladder.
    ///
    /// The arithmetic at three ages: the instant the charge was opened, one
    /// second short of the window, and the window itself (where there is
    /// nothing left to wait for and the table applies).
    ///
    /// The `assert_ne!` is the regression this case exists for.
    /// `RecoveryAction::Wait` used to carry nothing and the caller
    /// rescheduled at `poll_delay(0)`, ten seconds — but every reschedule is
    /// re-claimed, `vpay_db::Jobs::claim` increments `jobs.attempts`, and
    /// `poll_delay` is indexed by that count. A genuinely crashed charge
    /// therefore spent six claims waiting out the window and began its real
    /// recovery at `poll_delay(6)`, two minutes a rung, with the fast end of
    /// the ladder already gone. One wait of the remaining time costs one.
    #[test]
    fn a_wait_carries_the_rest_of_the_window_and_not_a_ladder_rung() {
        let p = policy();
        let step = |age: time::Duration| {
            recovery_step(ProviderFlow::Push, SubmitAttempt::Never, 0, None, age, &p)
        };

        assert_eq!(
            step(time::Duration::ZERO),
            RecoveryAction::Wait(Duration::from_secs(60)),
            "a charge opened this instant waits the whole window"
        );
        assert_eq!(
            step(time::Duration::seconds(59)),
            RecoveryAction::Wait(Duration::from_secs(1)),
            "one second short of the window, one second left to wait"
        );
        assert_eq!(
            step(time::Duration::seconds(60)),
            RecoveryAction::Resubmit,
            "at the window the charge is old enough and the table decides; there is no \
             wait to carry"
        );

        assert_ne!(
            step(time::Duration::ZERO),
            RecoveryAction::Wait(crate::poll_delay(0)),
            "the wait must not be the ladder's first rung: six of those cross the window \
             and each one spends an attempt"
        );
    }

    /// The guard reads the charge's age and nothing else — not the streak,
    /// not the attempt row, not the flow shape.
    ///
    /// Written separately from the table above because it is the property a
    /// "fix" that only guarded, say, the `Answered` branch would still pass
    /// the table on: here *every* combination the enum can produce is walked
    /// at one young age, and none of them may move a charge.
    #[test]
    fn a_young_charge_is_left_alone_on_every_shape_of_evidence() {
        let p = policy();
        let young = time::Duration::seconds(1);
        for flow in [ProviderFlow::Push, ProviderFlow::Redirect] {
            for attempt in [
                SubmitAttempt::Never,
                SubmitAttempt::Unanswered,
                SubmitAttempt::Answered(0),
                SubmitAttempt::Answered(201),
            ] {
                for (streak, streak_age) in [(0, None), (99, well_inside_the_window())] {
                    assert_eq!(
                        recovery_step(flow, attempt, streak, streak_age, young, &p),
                        RecoveryAction::Wait(Duration::from_secs(59)),
                        "recovery_step({flow:?}, {attempt:?}, streak {streak}) on a \
                         one-second-old charge"
                    );
                }
            }
        }
    }

    /// The guard moves with the policy, so an integration test can cross it
    /// without sleeping a minute — the same argument
    /// [`a_tightened_policy_resubmits_without_waiting_a_minute`] makes for the
    /// streak window, and the same *number*, which is why it is one field.
    #[test]
    fn a_tightened_window_shortens_the_age_guard_too() {
        let p = RecoveryPolicy {
            not_found_window: Duration::from_millis(50),
            ..RecoveryPolicy::default()
        };
        let step = |age: time::Duration| {
            recovery_step(
                ProviderFlow::Push,
                SubmitAttempt::Answered(0),
                0,
                None,
                age,
                &p,
            )
        };
        assert_eq!(
            step(time::Duration::milliseconds(49)),
            RecoveryAction::Wait(Duration::from_millis(1)),
            "a charge younger than the tightened window is still left alone, and waits \
             the rest of *that* window"
        );
        assert_eq!(
            step(time::Duration::milliseconds(50)),
            RecoveryAction::Advance(0),
            "at the boundary the table applies, exactly as it does at 60s by default"
        );
    }

    /// A **negative** age — a row whose `created_at` is ahead of the `now()`
    /// selected beside it — reads as "younger than the window", never as
    /// "old enough".
    ///
    /// One statement produces both halves of that subtraction now
    /// (`vpay_db::ChargeAsOf`), so this needs a replica whose clock is ahead
    /// of the primary's to happen at all; the point is only that the
    /// comparison is signed and saturates toward doing nothing rather than
    /// toward failing a live charge.
    #[test]
    fn a_charge_created_in_the_future_is_not_recovered() {
        assert_eq!(
            recovery_step(
                ProviderFlow::Redirect,
                SubmitAttempt::Answered(201),
                0,
                None,
                time::Duration::seconds(-5),
                &policy(),
            ),
            RecoveryAction::Wait(Duration::from_secs(60)),
            "a negative age waits one window, never the window plus the skew"
        );
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
                Some(time::Duration::milliseconds(50)),
                OLD_ENOUGH,
                &p,
            ),
            RecoveryAction::Resubmit
        );
        assert_eq!(
            recovery_step(
                ProviderFlow::Push,
                SubmitAttempt::Unanswered,
                3,
                Some(time::Duration::milliseconds(49)),
                OLD_ENOUGH,
                &p,
            ),
            RecoveryAction::Poll
        );
    }
}
