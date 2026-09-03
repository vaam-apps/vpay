//! The worker: submit, poll, reconcile, deliver.
//!
//! Everything that touches the network happens here, never in the API process.
//!
//! Read in this order: [`mod@run_loop`] owns the `jobs` row (claim, settle,
//! drain), [`handlers::handle`] does the work of one job, [`recovery`] and
//! `vpay_core::settlement` are the two pure decision tables it consults, and
//! [`error`] is the retry policy all of it derives from `Classify`.
//!
//! STATUS: polling, recovery and settlement are implemented and proven
//! against a real Postgres and a real WireMock rail in
//! `backends/tests/integration/tests/worker_{recovery,e2e}.rs`. **Webhook
//! delivery is not** — `deliver_webhook` is absent from [`jobs::JobKind`] and
//! from migration 0021's `kind_is_known` CHECK, so this build cannot enqueue
//! one by accident. See `docs/status.md`.

use std::time::Duration;

pub mod error;
pub mod handlers;
pub mod jobs;
pub mod recovery;
pub mod run_loop;
pub use error::{Decision, JobError, tracing_level};
pub use handlers::{Adapters, RailConfigs, handle};
pub use jobs::{JobKind, Outcome, PollChargePayload, ResubmitPayload};
pub use recovery::{RecoveryAction, RecoveryPolicy, SubmitAttempt, recovery_step};
pub use run_loop::{
    Disposition, Drain, LoopReport, Settled, run_loop, run_once, seed_singletons, worker_id,
};

/// Delay before poll number `n` (0-indexed), per `docs/flows/reconciler.md`.
///
/// Ladder: 10s, 20s, 30s, 45s, 60s, 90s, then 120s to the 30-minute mark, then
/// 15 minutes out to 24 hours.
#[must_use]
pub fn poll_delay(attempt: u32) -> Duration {
    const LADDER: [u64; 6] = [10, 20, 30, 45, 60, 90];
    match LADDER.get(attempt as usize) {
        Some(&secs) => Duration::from_secs(secs),
        None if attempt < 20 => Duration::from_secs(120),
        None => Duration::from_secs(15 * 60),
    }
}

/// How often an `unresolved` charge is polled after the 24-hour ladder has
/// run out — `docs/flows/reconciler.md`: "still polled, once an hour, and now
/// raising an alert for a human".
///
/// Deliberately *not* the last rung of [`poll_delay`] (15 minutes). Once a
/// human is reconciling the charge against the rail's settlement statement,
/// four polls an hour buy nothing; one keeps the charge live — and it must
/// stay live, because a late success at hour 30 is a normal transition, not
/// an exception. This is the delay [`JobError::decision`] pairs with
/// `alert: true` for [`JobError::Exhausted`].
///
/// "Polled" is literal: each of these hourly runs asks the rail again, and a
/// terminal answer settles the charge exactly as it would have on the first
/// rung ([`handlers::handle`]). The escalation changes the *interval* and adds
/// the alert; it never stops the question being asked, because a charge
/// nobody asks about is one whose late success is lost.
pub const UNRESOLVED_POLL_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unresolved_interval_is_the_documented_hour() {
        assert_eq!(UNRESOLVED_POLL_INTERVAL, Duration::from_secs(3_600));
        // Slower than every rung of the ladder, which is the point: the
        // escalated charge stays live without being hammered.
        assert!(UNRESOLVED_POLL_INTERVAL > poll_delay(1_000));
    }

    #[test]
    fn the_ladder_starts_fast() {
        assert_eq!(poll_delay(0), Duration::from_secs(10));
        assert_eq!(poll_delay(5), Duration::from_secs(90));
    }

    #[test]
    fn the_ladder_slows_but_never_stops() {
        assert_eq!(poll_delay(6), Duration::from_secs(120));
        assert_eq!(poll_delay(100), Duration::from_secs(900));
    }

    #[test]
    fn the_ladder_is_monotonically_non_decreasing() {
        let mut prev = Duration::ZERO;
        for n in 0..200 {
            let d = poll_delay(n);
            assert!(d >= prev, "delay went backwards at attempt {n}");
            prev = d;
        }
    }
}
