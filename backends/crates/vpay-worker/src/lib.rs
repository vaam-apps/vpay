//! The worker: submit, poll, reconcile, deliver.
//!
//! Everything that touches the network happens here, never in the API process.
//!
//! Read in this order: [`mod@run_loop`] owns the `jobs` row (claim, settle,
//! drain), [`handlers::handle`] does the work of one job, [`recovery`],
//! `vpay_core::settlement` and [`webhooks`] are the decision tables it
//! consults, and [`error`] is the retry policy all of it derives from
//! `Classify`.
//!
//! STATUS: polling, recovery and settlement are implemented and proven
//! against a real Postgres and a real WireMock rail in
//! `backends/tests/integration/tests/worker_{recovery,e2e}.rs`; webhook
//! fan-out, signing and delivery in `webhooks.rs` against a real WireMock
//! receiver. `docs/status.md` is the record of what is actually wired up,
//! and it is the one to trust — this crate holds *handlers*, and a handler
//! no loop calls is not a running worker.
//!
//! Two independent retry ladders live here: [`poll_delay`] for charge
//! polling and [`delivery_delay`] for webhook delivery. They are
//! deliberately separate — see [`delivery_delay`].

use std::time::Duration;

pub mod error;
pub mod handlers;
pub mod jobs;
pub mod recovery;
pub mod run_loop;
pub mod signing;
pub mod webhooks;
pub use error::{Decision, JobError, tracing_level};
pub use handlers::{Adapters, RailConfigs, WebhookContext, handle};
pub use jobs::{DeliverWebhookPayload, JobKind, Outcome, PollChargePayload, ResubmitPayload};
pub use recovery::{RecoveryAction, RecoveryPolicy, SubmitAttempt, recovery_step};
pub use run_loop::{
    Disposition, Drain, LoopReport, Settled, run_loop, run_once, seed_singletons, worker_id,
};
pub use signing::signature_header;
pub use webhooks::{
    Endpoint, EndpointRegistry, FANOUT_MAX_ATTEMPTS, WEBHOOK_CONNECT_TIMEOUT,
    WEBHOOK_REQUEST_TIMEOUT, handle_deliver, handle_fan_out, handle_scan_deliveries,
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

/// Delay before webhook delivery attempt number `n` (0-indexed), or `None`
/// when the ladder has run out — `docs/flows/webhooks.md`: "delivery, with
/// retries: 10s → 30s → 2m → 10m → 1h → 6h → 24h".
///
/// # Why this is not [`poll_delay`], and not [`JobError::decision`]
///
/// Polling asks a *rail* what happened to money; delivering tells a
/// *merchant* what already happened. The two have no failure vocabulary in
/// common: a merchant's `500` is not a `ProviderError`, nothing about it is
/// classified by ADR-0011's table, and pushing it through
/// [`JobError::decision`] would give a webhook receiver the poll ladder's
/// 24-hour horizon and its hourly `unresolved` escalation. So delivery keeps
/// its own ladder and never consults `Classify::retry`
/// (`docs/flows/reconciler.md`'s Status says the same in the other
/// direction).
///
/// # Why `Option` rather than a final rung
///
/// "The ladder ran out" is the `exhausted` transition of a
/// `webhook_deliveries` row, and it must not be expressible as another
/// delay. A `Duration` return would make the seventh failure and the eighth
/// indistinguishable at the type level, and a delivery that keeps
/// rescheduling forever is a queue that never drains.
#[must_use]
pub fn delivery_delay(attempt: u32) -> Option<Duration> {
    const LADDER: [u64; 7] = [10, 30, 120, 600, 3_600, 21_600, 86_400];
    LADDER
        .get(attempt as usize)
        .copied()
        .map(Duration::from_secs)
}

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

    /// Transcribed rung by rung from `docs/flows/webhooks.md`: "10s → 30s →
    /// 2m → 10m → 1h → 6h → 24h". Written out in the document's own units and
    /// converted here, rather than reused from the implementation's `LADDER`,
    /// so this asserts the *document* and not whatever numbers the code
    /// chose.
    #[test]
    fn the_delivery_ladder_is_the_documented_one() {
        assert_eq!(delivery_delay(0), Some(Duration::from_secs(10)));
        assert_eq!(delivery_delay(1), Some(Duration::from_secs(30)));
        assert_eq!(delivery_delay(2), Some(Duration::from_secs(2 * 60)));
        assert_eq!(delivery_delay(3), Some(Duration::from_secs(10 * 60)));
        assert_eq!(delivery_delay(4), Some(Duration::from_secs(60 * 60)));
        assert_eq!(delivery_delay(5), Some(Duration::from_secs(6 * 60 * 60)));
        assert_eq!(delivery_delay(6), Some(Duration::from_secs(24 * 60 * 60)));
    }

    /// Seven rungs, then the delivery is `exhausted` — never an eighth
    /// attempt, and never a silent repeat of the last rung.
    #[test]
    fn the_delivery_ladder_ends_after_seven_rungs() {
        assert_eq!(delivery_delay(7), None);
        assert_eq!(delivery_delay(8), None);
        assert_eq!(delivery_delay(u32::MAX), None);
        // The rung *before* the end is a real delay, so `None` marks the end
        // of the ladder rather than an off-by-one that lost the last rung.
        assert!(delivery_delay(6).is_some());
    }

    #[test]
    fn the_delivery_ladder_is_strictly_increasing_while_it_lasts() {
        let mut prev = Duration::ZERO;
        for n in 0..7 {
            let d = delivery_delay(n).expect("the first seven rungs exist");
            assert!(d > prev, "delay did not increase at attempt {n}");
            prev = d;
        }
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
