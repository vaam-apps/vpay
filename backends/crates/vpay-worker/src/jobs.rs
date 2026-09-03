//! The job vocabulary: what kinds of job exist, what each one's payload
//! looks like on the wire, how a job is deduplicated, and what a handler
//! answers when it did not fail.
//!
//! Everything here is data. The transactions live in `vpay-db` and the
//! decisions live in [`crate::recovery`] and `vpay_core::settlement`; this
//! module exists so that the `jobs` table's `kind` and `payload` columns have
//! exactly one Rust spelling and a typo cannot enqueue a job nothing will ever
//! run.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::Decision;

/// The kinds of job this step can enqueue.
///
/// Closed, and closed against a database constraint rather than a convention:
/// migration 0021's `kind_is_known` CHECK lists exactly these four strings, so
/// a fifth spelled here and not there is refused by Postgres at the insert
/// rather than discovered by a worker that claims a job it cannot dispatch.
/// `deliver_webhook` is deliberately absent from both — Step 5 adds it with
/// its own migration, so this step cannot enqueue one by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Ask the rail what happened to one charge, and settle it if the answer
    /// is terminal. The workhorse: every live charge has one.
    PollCharge,
    /// Send a charge to the rail again, with the **same**
    /// `provider_reference_id`. Enqueued only by the recovery table.
    ResubmitCharge,
    /// The housekeeping sweep: expired idempotency records, expired
    /// client-assertion `jti`s, and job leases whose worker died.
    SweepExpired,
    /// The backstop scan that finds live charges nothing is polling — rows
    /// written before the queue existed, or a job lost to operator error.
    ScanLiveCharges,
}

impl JobKind {
    /// The exact string in `jobs.kind`, and in migration 0021's
    /// `kind_is_known` CHECK.
    ///
    /// Written out beside the `serde` rename for the same reason
    /// `vpay_core::IntentStatus::as_wire_str` is: the column is bound as a
    /// plain `String`, not through `serde`, so the two spellings would
    /// otherwise be free to drift. `the_wire_spelling_is_the_same_by_both_routes`
    /// pins them together.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::PollCharge => "poll_charge",
            Self::ResubmitCharge => "resubmit_charge",
            Self::SweepExpired => "sweep_expired",
            Self::ScanLiveCharges => "scan_live_charges",
        }
    }

    /// Parses a `jobs.kind` value, or `None` for one this build does not
    /// know.
    ///
    /// `Option` rather than an error type, mirroring
    /// `vpay_core::ChargeState::from_wire`: the only caller is the worker
    /// reading a row it just claimed, where an unknown kind is not anybody's
    /// mistake but a row written by a newer build. That is a poisoned job
    /// ([`crate::JobError::Poisoned`]), and the caller says so in its own
    /// vocabulary instead of this module inventing a second one.
    #[must_use]
    pub fn from_wire(kind: &str) -> Option<Self> {
        [
            Self::PollCharge,
            Self::ResubmitCharge,
            Self::SweepExpired,
            Self::ScanLiveCharges,
        ]
        .into_iter()
        .find(|candidate| candidate.as_wire_str() == kind)
    }
}

/// `poll_charge`'s payload.
///
/// The two `not_found` fields are the recovery ladder's *state*, and they live
/// in the payload rather than in a column because they belong to the poll
/// ladder, not to the charge: a charge the rail has temporarily lost is
/// unchanged by our failure to find it, and the moment any non-`NotFound`
/// answer arrives both are reset (`docs/flows/crash-safety.md`). Putting them
/// on `charges` would make a rail's indexing lag look like a property of the
/// payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollChargePayload {
    /// The `ch_…` id to poll.
    pub charge_id: String,
    /// How many consecutive `ChargeStatus::NotFound` answers this ladder has
    /// seen. Compared against [`crate::recovery::RecoveryPolicy::not_found_streak`].
    ///
    /// Defaulted so the enqueue sites that have no ladder yet — the confirm
    /// path's `insert_charge`, the callback route, the backstop scan — can
    /// write `{"charge_id": "ch_…"}` and mean it.
    #[serde(default)]
    pub not_found_streak: u32,
    /// When the current streak started, or `None` if there is none.
    ///
    /// RFC 3339 on the wire, not `time`'s own default encoding: this value is
    /// read by an operator looking at a `jobs` row in `psql` while deciding
    /// whether a charge is about to be resubmitted, and a timestamp they have
    /// to decode is a timestamp they will misread.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub first_not_found_at: Option<OffsetDateTime>,
}

impl PollChargePayload {
    /// A fresh ladder for `charge_id`: no `NotFound` seen yet.
    #[must_use]
    pub fn new(charge_id: impl Into<String>) -> Self {
        Self {
            charge_id: charge_id.into(),
            not_found_streak: 0,
            first_not_found_at: None,
        }
    }

    /// The payload for the next poll after the rail answered `NotFound` at
    /// `now`.
    ///
    /// The first `NotFound` stamps the window's start; later ones only
    /// increment, so the window measures the age of the *streak* and not the
    /// age of the last answer.
    #[must_use]
    pub fn saw_not_found(&self, now: OffsetDateTime) -> Self {
        Self {
            charge_id: self.charge_id.clone(),
            not_found_streak: self.not_found_streak.saturating_add(1),
            first_not_found_at: Some(self.first_not_found_at.unwrap_or(now)),
        }
    }

    /// The payload for the next poll after any answer that was not
    /// `NotFound`.
    ///
    /// `docs/flows/crash-safety.md`: "any non-`NotFound` answer resets both
    /// fields". A streak that survived an intervening `Pending` would not be
    /// *consecutive*, and consecutiveness is the whole evidence for
    /// concluding the rail never received the charge.
    #[must_use]
    pub fn reset(&self) -> Self {
        Self::new(self.charge_id.clone())
    }
}

/// `resubmit_charge`'s payload.
///
/// Only the charge id. The step 4 design's §2 table also lists the two
/// `not_found` fields here; they are deliberately omitted, because a resubmit
/// *is* the reset — the poll job it schedules starts a new ladder, and
/// carrying an already-satisfied threshold across the resubmit would make the
/// very next `NotFound` resubmit again. The reference to send is read from
/// `charges.provider_reference_id` and never from a payload, so no queue row
/// can ever cause a second reference to be minted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResubmitPayload {
    /// The `ch_…` id to submit again, with its existing reference.
    pub charge_id: String,
}

impl ResubmitPayload {
    /// A resubmit for `charge_id`.
    #[must_use]
    pub fn new(charge_id: impl Into<String>) -> Self {
        Self {
            charge_id: charge_id.into(),
        }
    }
}

/// What a handler that did **not** fail wants the loop to do with the row.
///
/// Two values, because a successful handler has only two things to say: this
/// job is finished, or ask me again later. Everything else — dead-lettering,
/// alerting, counting attempts — is derived from a [`crate::JobError`] through
/// [`crate::JobError::decision`], so there is exactly one retry policy in the
/// worker and it is the one ADR-0011 derives from `Classify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Delete the row. The work is done, or was already done by an earlier
    /// run of the same job — the compare-and-swap writes make those
    /// indistinguishable on purpose.
    Done,
    /// Put the row back with `run_at = now() + this`, releasing the lease.
    RescheduleAfter(Duration),
}

impl Outcome {
    /// The [`Outcome`] a failed job's [`Decision`] implies, or `None` when the
    /// row must not be re-run as-is.
    ///
    /// `None` covers two *different* dispositions — [`Decision::Terminal`]
    /// deletes the row, [`Decision::DeadLetter`] parks it for a human — and
    /// this function deliberately does not collapse them into a third
    /// `Outcome` variant. The difference belongs to the loop that owns the
    /// row, not to a handler's answer, and inventing an `Outcome::DeadLetter`
    /// here would let a handler dead-letter a job by returning `Ok`.
    ///
    /// The `alert` flag is dropped for the same reason: it selects a log
    /// level and an `alert = true` field at the loop's own emit site
    /// ([`crate::tracing_level`]), which is not something an `Outcome` can
    /// carry.
    #[must_use]
    pub const fn from_decision(decision: Decision) -> Option<Self> {
        match decision {
            Decision::RetryAfter { delay, alert: _ } => Some(Self::RescheduleAfter(delay)),
            Decision::Terminal | Decision::DeadLetter => None,
        }
    }
}

/// The `jobs.dedupe_key` for polling one charge.
///
/// One live poll job per charge, forever: the unique index on `dedupe_key` is
/// what turns "a callback arrived, and so did a retry of it, and the backstop
/// scan also noticed" into a single row. `docs/flows/reconciler.md`: "the
/// `dedupe_key` is what stops duplicate callbacks becoming a job storm".
#[must_use]
pub fn poll_dedupe_key(charge_id: &str) -> String {
    format!("poll:{charge_id}")
}

/// The `jobs.dedupe_key` for resubmitting one charge.
///
/// A *different* namespace from [`poll_dedupe_key`] on purpose: a charge being
/// resubmitted still has a poll job, and one key for both would make the
/// resubmit silently lose to the poll's `ON CONFLICT DO NOTHING`.
#[must_use]
pub fn resubmit_dedupe_key(charge_id: &str) -> String {
    format!("resubmit:{charge_id}")
}

/// The `jobs.dedupe_key` of the one and only housekeeping sweep.
///
/// A constant rather than a function because the job is a singleton: seeding
/// it at worker boot with `ON CONFLICT DO NOTHING` means N workers produce one
/// row, and the row reschedules itself hourly for as long as the deployment
/// lives.
pub const SWEEP_DEDUPE_KEY: &str = "sweep:expired";

/// The `jobs.dedupe_key` of the one and only backstop scan. A singleton, on
/// the same terms as [`SWEEP_DEDUPE_KEY`].
pub const SCAN_DEDUPE_KEY: &str = "scan:live";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KINDS: [JobKind; 4] = [
        JobKind::PollCharge,
        JobKind::ResubmitCharge,
        JobKind::SweepExpired,
        JobKind::ScanLiveCharges,
    ];

    /// Transcribed from migration 0021's `kind_is_known` CHECK. If these four
    /// strings and that constraint ever disagree, every enqueue of the odd one
    /// out fails at the database — so the list is written out here rather than
    /// generated from the enum.
    const KIND_IS_KNOWN: [&str; 4] = [
        "poll_charge",
        "resubmit_charge",
        "sweep_expired",
        "scan_live_charges",
    ];

    #[test]
    fn the_kinds_are_exactly_the_check_constraints() {
        let ours: Vec<&str> = KINDS.iter().map(|k| k.as_wire_str()).collect();
        assert_eq!(ours, KIND_IS_KNOWN.to_vec());
        assert!(
            !ours.contains(&"deliver_webhook"),
            "deliver_webhook belongs to Step 5 and must not be enqueueable yet"
        );
    }

    #[test]
    fn the_wire_spelling_is_the_same_by_both_routes() {
        for kind in KINDS {
            let via_serde = serde_json::to_value(kind).expect("a fieldless enum always serialises");
            assert_eq!(via_serde, json!(kind.as_wire_str()));
            assert_eq!(JobKind::from_wire(kind.as_wire_str()), Some(kind));
        }
    }

    #[test]
    fn an_unknown_kind_is_not_guessed_at() {
        assert_eq!(JobKind::from_wire("deliver_webhook"), None);
        assert_eq!(JobKind::from_wire("poll_charges"), None);
        assert_eq!(JobKind::from_wire(""), None);
    }

    #[test]
    fn a_poll_payload_round_trips_through_jsonb() {
        let payload = PollChargePayload {
            charge_id: "ch_abc".to_owned(),
            not_found_streak: 2,
            first_not_found_at: Some(
                OffsetDateTime::from_unix_timestamp(1_756_857_600)
                    .expect("a fixed, valid timestamp"),
            ),
        };
        let json = serde_json::to_value(&payload).expect("serialises");
        assert_eq!(
            json,
            json!({
                "charge_id": "ch_abc",
                "not_found_streak": 2,
                "first_not_found_at": "2025-09-03T00:00:00Z",
            }),
            "the on-disk shape changed; a job enqueued by an older build would \
             no longer deserialise"
        );
        let back: PollChargePayload = serde_json::from_value(json).expect("deserialises");
        assert_eq!(back, payload);
    }

    /// The enqueue sites outside the ladder — `insert_charge`, the callback
    /// route, the backstop scan — write only the charge id. That has to keep
    /// deserialising, or a crash-safety enqueue produces a job the worker
    /// cannot read.
    #[test]
    fn the_minimal_poll_payload_a_confirm_writes_still_parses() {
        let back: PollChargePayload =
            serde_json::from_value(json!({ "charge_id": "ch_abc" })).expect("deserialises");
        assert_eq!(back, PollChargePayload::new("ch_abc"));
        assert_eq!(back.not_found_streak, 0);
        assert_eq!(back.first_not_found_at, None);
    }

    #[test]
    fn a_resubmit_payload_round_trips_and_carries_no_reference() {
        let payload = ResubmitPayload::new("ch_abc");
        let json = serde_json::to_value(&payload).expect("serialises");
        assert_eq!(json, json!({ "charge_id": "ch_abc" }));
        assert_eq!(
            json.as_object().map(serde_json::Map::len),
            Some(1),
            "a resubmit payload must not carry a provider reference: the reference \
             comes from the charge row, so no queue row can mint a second one"
        );
        let back: ResubmitPayload = serde_json::from_value(json).expect("deserialises");
        assert_eq!(back, payload);
    }

    #[test]
    fn the_first_not_found_stamps_the_window_and_later_ones_do_not_move_it() {
        let start = OffsetDateTime::from_unix_timestamp(1_000_000).expect("valid");
        let later = start + time::Duration::seconds(30);

        let first = PollChargePayload::new("ch_abc").saw_not_found(start);
        assert_eq!(first.not_found_streak, 1);
        assert_eq!(first.first_not_found_at, Some(start));

        let second = first.saw_not_found(later);
        assert_eq!(second.not_found_streak, 2);
        assert_eq!(
            second.first_not_found_at,
            Some(start),
            "the window measures the age of the streak, not of the last answer"
        );

        let reset = second.reset();
        assert_eq!(reset.not_found_streak, 0);
        assert_eq!(reset.first_not_found_at, None);
        assert_eq!(reset.charge_id, "ch_abc");
    }

    #[test]
    fn the_dedupe_keys_are_the_documented_ones_and_never_collide() {
        assert_eq!(poll_dedupe_key("ch_abc"), "poll:ch_abc");
        assert_eq!(resubmit_dedupe_key("ch_abc"), "resubmit:ch_abc");
        assert_eq!(SWEEP_DEDUPE_KEY, "sweep:expired");
        assert_eq!(SCAN_DEDUPE_KEY, "scan:live");

        let keys = [
            poll_dedupe_key("ch_abc"),
            resubmit_dedupe_key("ch_abc"),
            SWEEP_DEDUPE_KEY.to_owned(),
            SCAN_DEDUPE_KEY.to_owned(),
        ];
        let unique: std::collections::BTreeSet<&String> = keys.iter().collect();
        assert_eq!(
            unique.len(),
            keys.len(),
            "two kinds share a dedupe key, so one of them can never be enqueued \
             while the other exists"
        );
        // Two charges are two jobs. The obvious property, and the one a
        // `format!` typo would break.
        assert_ne!(poll_dedupe_key("ch_a"), poll_dedupe_key("ch_b"));
    }

    /// The bridge between the error contract and the queue: a retryable
    /// failure becomes a reschedule at exactly the delay `decision` chose,
    /// and the two give-up decisions become "not an outcome at all".
    #[test]
    fn the_outcome_of_a_failed_job_is_the_decisions_own_delay() {
        assert_eq!(
            Outcome::from_decision(Decision::RetryAfter {
                delay: Duration::from_secs(10),
                alert: false,
            }),
            Some(Outcome::RescheduleAfter(Duration::from_secs(10)))
        );
        // The alerting flag changes the log line, never the schedule.
        assert_eq!(
            Outcome::from_decision(Decision::RetryAfter {
                delay: crate::UNRESOLVED_POLL_INTERVAL,
                alert: true,
            }),
            Some(Outcome::RescheduleAfter(crate::UNRESOLVED_POLL_INTERVAL))
        );
        assert_eq!(Outcome::from_decision(Decision::Terminal), None);
        assert_eq!(Outcome::from_decision(Decision::DeadLetter), None);
    }

    /// The 24-hour escalation, end to end through the two types this module
    /// bridges: `Exhausted` must keep the job alive, hourly — never delete it,
    /// never park it — because a late success at hour 30 is a normal
    /// transition (`docs/flows/reconciler.md`).
    #[test]
    fn the_unresolved_escalation_reschedules_hourly_rather_than_giving_up() {
        let err = crate::JobError::Exhausted {
            job_id: uuid::Uuid::nil(),
            attempts: 240,
        };
        let outcome = Outcome::from_decision(err.decision(240));
        assert_eq!(
            outcome,
            Some(Outcome::RescheduleAfter(crate::UNRESOLVED_POLL_INTERVAL))
        );
    }
}
