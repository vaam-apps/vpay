//! What each job kind actually does.
//!
//! One `async fn` per [`JobKind`], behind a single [`handle`] entry point that
//! dispatches on the row's `kind`. Everything here is composition: the
//! decisions come from `vpay_core::settlement::settle` and
//! [`crate::recovery::recovery_step`], the transactions come from `vpay-db`,
//! and the retry policy comes from [`JobError::decision`]. This module holds
//! no policy of its own, on purpose — a second retry rule here is how the API
//! and the worker end up disagreeing about whether a Postgres failure is
//! transient (ADR-0011).
//!
//! **`vpay-worker` has no `sqlx` dependency and cannot grow one here.** Every
//! statement is reached through `&dyn Repositories`, and two writes that must
//! commit together go through `UnitOfWork::transaction`, so no `sqlx` type is
//! nameable.
//!
//! The handlers are deliberately **not** unit-tested; their proofs are the
//! integration suites, and `docs/reference/vpay-worker.md` §"How this crate is
//! tested" says why. That section, §"One poll" and §"The outbox drain" also
//! carry the orderings this module depends on.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use vpay_core::state::{ChargeState, IntentStatus};
use vpay_core::{
    Classify, FailureCode, Money, Settlement, Severity, StatusKind, contradiction, ids, settle,
};
use vpay_db::{
    ChargeAsOf, ChargeRow, Charges, DbError, PaymentIntents, Repositories, TxOutcome,
    UnitOfWork as _,
};
use vpay_provider::{
    ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig, ProviderError, RefExtra,
};

use crate::error::JobError;
use crate::jobs::{
    JobKind, Outcome, PollChargePayload, ResubmitPayload, poll_dedupe_key, resubmit_dedupe_key,
};
use crate::recovery::{RecoveryAction, RecoveryPolicy, SubmitAttempt, recovery_step};
use crate::webhooks::{EndpointRegistry, handle_deliver, handle_fan_out, handle_scan_deliveries};

/// Every rail this deployment can talk to, by `providers.code`.
///
/// The same map `vpay_api::v1::boot::adapters_by_code` builds for the server.
pub type Adapters = BTreeMap<String, Box<dyn ProviderAdapter>>;

/// Each rail's deployment configuration, by `providers.code`.
///
/// Plain values rather than `vpay_api::v1::ResourceConfig`, although the
/// worker binary links `vpay-api` and could pass the projection itself: a
/// handler needs exactly one thing from it, and `ResourceConfig` also carries
/// the merchant-client table and the currency list, which no job may read. The
/// flow shape the recovery table branches on comes from
/// `ProviderAdapter::capabilities()`, not from configuration at all.
pub type RailConfigs = BTreeMap<String, ProviderConfig>;

/// What the two webhook jobs need and no other job may touch: the endpoint
/// table and the deployment's egress policy.
///
/// Borrowed as one struct rather than added as two parameters to every
/// handler, for the reason [`RailConfigs`] is a projection: a poll job has no
/// business reaching a merchant's signing secrets, and grouping the two makes
/// the dispatch signature say so.
///
/// # Why there is no shared client here any more
///
/// Until Step 8 this carried a `&reqwest::Client` the binary built once and
/// cloned. It cannot: pinning a delivery to the addresses
/// [`crate::ssrf::vet`] classified is a property of the *builder*
/// (`resolve_to_addrs`), so the client that connects has to be built after the
/// lookup, per delivery. The shared one would have resolved the host a second
/// time — which is the TOCTOU the guard exists to close — so keeping it as
/// well would have meant a field that is never the client anything sends on.
///
/// The cost is stated where it is paid
/// (`vpay_provider::http::client_pinned_to`): two deliveries to one receiver
/// no longer share a pooled connection. The budgets are unchanged and still
/// single-sourced — [`crate::WEBHOOK_CONNECT_TIMEOUT`] and
/// [`crate::WEBHOOK_REQUEST_TIMEOUT`], read by [`crate::ssrf::pinned_client`]
/// rather than by the binary.
#[derive(Debug, Clone, Copy)]
pub struct WebhookContext<'a> {
    /// Every merchant's configured endpoints, by `events.merchant_id`.
    pub endpoints: &'a EndpointRegistry,
    /// Whether this deployment may deliver to a non-public address —
    /// `webhooks.allow_private_targets`, projected out of YAML by the binary.
    ///
    /// By value, not by reference: it is one `bool` and [`Copy`], and a
    /// borrowed policy would only add a lifetime to every signature that
    /// carries it.
    pub egress: crate::ssrf::EgressPolicy,
}

/// How often the housekeeping sweep runs. Hourly, and unconditionally: the
/// three things it deletes are all bounded by their own expiry timestamps, so
/// a sweep that finds nothing is the healthy case rather than a reason to
/// back off.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How often the backstop scan runs, and how stale a live charge must be
/// before the scan considers it unattended.
///
/// The same ten minutes on both counts, matching `charges_live_idx`'s
/// intended query (migration 0014). It is a *backstop*: the poll job is
/// enqueued in the same transaction that opens the charge, so a healthy
/// deployment's scan finds nothing. A steady stream from it means the enqueue
/// is broken — that is the bug, not this interval.
const SCAN_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// How many unattended charges one scan will enqueue.
///
/// Bounded so that a deployment which somehow accumulated a large backlog
/// re-enqueues it over several runs rather than writing an unbounded number of
/// rows in one transaction. The scan repeats every [`SCAN_INTERVAL`], so a
/// backlog drains rather than being dropped.
const SCAN_BATCH: i64 = 500;

/// Added to a [`RecoveryAction::Wait`]'s remaining time before the job is
/// rescheduled.
///
/// The wait is `not_found_window - charge_age` measured at the moment the
/// charge was read, and the reschedule writes `run_at = now() + delay` a
/// moment later, so the next claim already lands past the window without any
/// margin at all. One second is there so that "past" is not measured in
/// microseconds: the guard's comparison is `<`, and a claim that arrives
/// exactly on the boundary of it would answer `Wait(0)` and spend a second
/// rung of the ladder proving the same thing. Small enough to be invisible
/// against a sixty-second window, and it is added to the *delay*, never to
/// the window — the policy number stays the one number.
const RECOVERY_WAIT_MARGIN: Duration = Duration::from_secs(1);

/// Runs one claimed job, and says what the loop should do with the row.
///
/// The loop owns the row: it claimed it, and it is what calls
/// [`vpay_db::Jobs::finish`] or [`vpay_db::Jobs::reschedule`] afterwards. This
/// function only says which; on `Err` the loop derives the same answer from
/// [`JobError::decision`].
///
/// Failures are logged here as well as at the loop, because this is the frame
/// that still knows *which* job and which charge failed —
/// `docs/reference/vpay-worker.md` §"Where a job failure is logged" says what
/// each of the two lines is for.
///
/// # Errors
///
/// Every Postgres failure becomes [`JobError::Db`] and every rail failure
/// [`JobError::Provider`], both by delegation rather than re-classification. A
/// job row this build cannot interpret — an unknown `kind`, a payload of the
/// wrong shape, a `charge_id` naming no row, a `charges.state` outside the
/// enum — is [`JobError::Poisoned`], which dead-letters: re-running cannot fix
/// data that is already wrong. A charge that has been live past
/// [`RecoveryPolicy::unresolved_after`] is [`JobError::Exhausted`], which is
/// neither of those — it keeps polling hourly and alerts.
pub async fn handle(
    repositories: &dyn Repositories,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
    webhooks: &WebhookContext<'_>,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let result = dispatch(repositories, adapters, rails, policy, webhooks, job).await;
    if let Err(error) = &result {
        log_failure(job, error);
    }
    result
}

async fn dispatch(
    repositories: &dyn Repositories,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
    webhooks: &WebhookContext<'_>,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let kind = JobKind::from_wire(&job.kind).ok_or_else(|| {
        poisoned(
            job,
            format!(
                "`{}` is not a job kind this build knows; the row was written by a \
                 different version",
                job.kind
            ),
        )
    })?;

    match kind {
        JobKind::PollCharge => poll_charge(repositories, adapters, rails, policy, job).await,
        JobKind::ResubmitCharge => resubmit_charge(repositories, adapters, rails, job).await,
        JobKind::SweepExpired => sweep_expired(repositories, policy).await,
        JobKind::ScanLiveCharges => scan_live_charges(repositories, job).await,
        // Both live in `crate::webhooks`, which owns every decision they make.
        // This module only routes: a second copy of the delivery ladder or of
        // the fan-out's transaction boundary here is exactly the duplication
        // this file's header refuses.
        JobKind::FanOutEvents => handle_fan_out(repositories, webhooks.endpoints, job).await,
        JobKind::DeliverWebhook => {
            handle_deliver(repositories, webhooks.egress, webhooks.endpoints, job).await
        }
        // The delivery queue's backstop. `policy.lease` and not a constant of
        // its own: the never-attempted arm of the scan's query asks "has this
        // delivery's job been outstanding longer than a claim may legitimately
        // be?", and that is the same number the reaper compares against.
        JobKind::ScanDeliveries => handle_scan_deliveries(repositories, policy.lease, job).await,
    }
}

/// Asks the rail about one charge and settles it if the answer is terminal.
///
/// The order is the one `docs/flows/crash-safety.md` requires and never a
/// convenient rearrangement: the attempt row is written *before* the call and
/// answered *after* it, so an attempt that got no answer is distinguishable
/// from one that was never made — which is the single fact the recovery table
/// branches on.
///
/// The six steps below are one each: the terminal guard, the horizon, the
/// crash-recovery block, the rail, the settlement table's answer, and what to
/// do with it. `docs/reference/vpay-worker.md` §"One poll" carries the
/// reasoning behind the two orderings that are load-bearing — the horizon
/// above the recovery block, and the attempt row around the query.
async fn poll_charge(
    repositories: &dyn Repositories,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let payload: PollChargePayload = decode(job)?;
    let as_of = load_charge(repositories, job, &payload.charge_id).await?;
    let charge_age = charge_age(&as_of);
    let ChargeAsOf { charge, db_now } = as_of;
    let mut state = parse_state(job, &charge)?;
    if state.is_terminal() {
        // A duplicate job, a callback that arrived after the answer, or a
        // second worker that lost a race. Not an error: the work is done.
        return Ok(Outcome::Done);
    }

    let (adapter, config) = rail(rails, adapters, job, &charge.provider_code)?;
    let flow = adapter.capabilities().flow;
    // **Postgres' clock, not this host's**, and every duration below is
    // measured against it. `charges.created_at` and `jobs.run_at` are written
    // by the database, so a `now` read here from `OffsetDateTime::now_utc()`
    // would put the two ends of every age on two different machines — which
    // is how a worker sixty seconds fast turned the recovery window into a
    // no-op. It arrives on the same `SELECT` as the row
    // (`vpay_db::ChargeAsOf`), and `first_not_found_at` is stamped from it
    // too, so the streak's window is on that clock as well.
    let now = db_now;

    // Evaluated once, here, and carried down. **Above** the crash-recovery
    // block, which can return without ever reaching this line — see
    // [`past_the_horizon`].
    let past_horizon = past_the_horizon(charge_age, policy);

    // Only for `submitting`: past that state the payer may already hold a
    // redirect URL, which is what makes `FailDeadOrder` safe there and unsafe
    // everywhere else. See `recovery_step`'s precondition — and its age
    // guard, which is why a charge whose confirm is still running comes out
    // of this block as `Wait` rather than as a recovery.
    if state == ChargeState::Submitting {
        let action = recovery_action(
            repositories,
            &charge,
            flow,
            policy,
            now,
            charge_age,
            &payload,
        )
        .await?;
        match act_on_recovery(repositories, job, &charge, action, past_horizon, &payload).await? {
            Recovered::Answered(outcome) => return Ok(outcome),
            Recovered::StillPolling(recovered) => state = recovered,
        }
    }

    let status = match query_status(repositories, adapter, config, job, &charge).await {
        Ok(status) => status,
        Err(error) => {
            return rail_did_not_answer(repositories, job, &charge, state, past_horizon, error)
                .await;
        }
    };
    let kind = status_kind(&status);

    let Some(settlement) = settle(kind, state) else {
        return Ok(nothing_to_settle(job, &charge.id, state, kind));
    };

    match settlement {
        Settlement::Succeeded => {
            settle_succeeded(repositories, job, &charge, succeeded_txn_id(&status)).await
        }
        Settlement::Failed(code) => settle_failed(repositories, job, &charge, code, &status).await,
        Settlement::Live(next) => {
            advance_and_keep_polling(
                repositories,
                job,
                &charge,
                state,
                next,
                past_horizon,
                &payload.reset(),
            )
            .await
        }
        Settlement::Stay => {
            keep_polling(
                repositories,
                job,
                &charge,
                state,
                past_horizon,
                &stayed(&payload, kind, now),
            )
            .await
        }
        Settlement::Recover => {
            recover(
                repositories,
                job,
                &charge,
                state,
                flow,
                policy,
                now,
                charge_age,
                past_horizon,
                &payload,
            )
            .await
        }
    }
}

/// Reads `provider_requests` for this charge and asks the recovery table what
/// to do about it.
///
/// The two callers reach the same decision from different evidence — the
/// crash-recovery step at the top of a poll, and a `NotFound` the poll itself
/// received — and they must not drift, which is why the read and the table are
/// one step rather than two lines written twice. That is also why the charge's
/// age is passed here and not tested at either call site: a guard written
/// twice is a guard one caller can lose (`recovery_step` §"Nothing younger
/// than the window is recovered").
///
/// Both durations are Postgres-measured: `charge_age` is computed in
/// [`poll_charge`] from the `now()` that came back with the row, and the
/// streak's age is `now - first_not_found_at` where the stored instant was
/// stamped from that same clock. [`recovery_step`] takes no instant at all —
/// see its §"Both ages are measured by one clock".
///
/// # Errors
///
/// [`JobError::Db`] if the attempt row cannot be read.
async fn recovery_action(
    repositories: &dyn Repositories,
    charge: &ChargeRow,
    flow: vpay_core::ProviderFlow,
    policy: &RecoveryPolicy,
    now: OffsetDateTime,
    charge_age: time::Duration,
    payload: &PollChargePayload,
) -> Result<RecoveryAction, JobError> {
    let evidence = submit_evidence(repositories, &charge.id).await?;
    Ok(recovery_step(
        flow,
        evidence,
        payload.not_found_streak,
        payload.first_not_found_at.map(|first| now - first),
        charge_age,
        policy,
    ))
}

/// What the crash-recovery block left for the rest of the poll to do.
///
/// Two values rather than an `Option<Outcome>`, because the second one
/// carries a fact the caller must not discard: the state the charge is in
/// *after* recovery, which `Advance` moves.
enum Recovered {
    /// The recovery table answered for the whole job — the charge was
    /// abandoned as a dead order, or a resubmit was scheduled.
    Answered(Outcome),
    /// Carry on to the rail, in this state.
    StillPolling(ChargeState),
}

/// Performs the action [`recovery_step`] chose for a `submitting` charge.
///
/// The `Advance` arm is kill point 3 of `docs/flows/crash-safety.md`: the rail
/// answered and the state update was lost, so the bookkeeping catches up and
/// the poll continues as normal. A compare-and-swap that matched nothing means
/// something else moved the row first, and this poll carries on in the state it
/// read.
///
/// The `Wait` arm is the one that ends the job without writing anything: the
/// charge is too young for its `submitting` state to be evidence of a crash,
/// so the poll returns a rung of the ladder and the rail is never asked. See
/// [`crate::recovery::recovery_step`] §"Nothing younger than the window is
/// recovered".
///
/// # Errors
///
/// [`JobError::Db`] for the state write, and whatever
/// [`fail_dead_order`]/[`resubmit_then_escalate_if_late`] raise — including the
/// [`JobError::Exhausted`] a past-the-horizon resubmit escalates with.
async fn act_on_recovery(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    action: RecoveryAction,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Recovered, JobError> {
    match action {
        RecoveryAction::FailDeadOrder => Ok(Recovered::Answered(
            fail_dead_order(repositories, job, charge).await?,
        )),
        RecoveryAction::Resubmit => Ok(Recovered::Answered(
            resubmit_then_escalate_if_late(
                repositories,
                job,
                charge,
                ChargeState::Submitting,
                past_horizon,
                payload,
            )
            .await?,
        )),
        RecoveryAction::Advance(status_code) => {
            if repositories
                .set_live_state(
                    &charge.id,
                    ChargeState::Submitting.as_wire_str(),
                    ChargeState::Submitted.as_wire_str(),
                )
                .await?
            {
                tracing::info!(
                    job_id = %job.id,
                    charge_id = %charge.id,
                    status_code,
                    "recovered a charge whose submit was answered but never recorded"
                );
                return Ok(Recovered::StillPolling(ChargeState::Submitted));
            }
            Ok(Recovered::StillPolling(ChargeState::Submitting))
        }
        RecoveryAction::Poll => Ok(Recovered::StillPolling(ChargeState::Submitting)),
        // Too young to be evidence of anything: a confirm may be inside its
        // rail call right now. The rail is deliberately **not** asked either
        // — see [`RecoveryAction::Wait`] — so this poll ends here, having
        // written nothing, and comes back **once**, when the charge is old
        // enough to be evidence of something.
        //
        // The horizon is not consulted, and no escalation can be lost to
        // this: a charge this arm sees is at most `not_found_window` — sixty
        // seconds — old, and the horizon it would be escalated at is
        // twenty-four hours (`RecoveryPolicy::unresolved_after`).
        RecoveryAction::Wait(remaining) => {
            let delay = remaining.saturating_add(RECOVERY_WAIT_MARGIN);
            tracing::debug!(
                job_id = %job.id,
                charge_id = %charge.id,
                delay_secs = delay.as_secs_f64(),
                "a `submitting` charge younger than the recovery window was left alone"
            );
            Ok(Recovered::Answered(Outcome::RescheduleAfter(delay)))
        }
    }
}

/// A rail that would not answer, read against the horizon.
///
/// A rail that will not answer must not be able to keep a charge off the
/// escalation: `ProviderError::Unavailable` is only [`Severity::Warn`], so a
/// status endpoint answering `503` on every rung rode the ladder quietly past
/// the horizon forever. Past it the fact an operator needs is "this charge is
/// unreconciled after 24 hours", not "the last poll got a 503", so the rail's
/// error is logged rather than returned and [`JobError::Exhausted`] says so
/// hourly and with an alert.
///
/// **Only [`JobError::Provider`].** This is the one place in the worker where
/// a composite replaces a leaf's classification with its own;
/// `docs/reference/vpay-worker.md` §"One poll" records why ADR-0011 permits it
/// here and what a wildcard arm swallowed before.
///
/// # Errors
///
/// The error it was given, untouched, for anything that is not a rail failure
/// short of the horizon; [`JobError::Exhausted`] for the escalation, or
/// [`JobError::Db`] if the escalation's state write fails.
async fn rail_did_not_answer(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    past_horizon: bool,
    error: JobError,
) -> Result<Outcome, JobError> {
    match error {
        JobError::Provider(error) if past_horizon => {
            tracing::warn!(
                job_id = %job.id,
                charge_id = %charge.id,
                error = %error,
                "the rail did not answer about a charge that is already past the horizon"
            );
            escalate_to_unresolved(repositories, job, charge, state).await
        }
        error => Err(error),
    }
}

/// `vpay_core::settle` had no answer, which today means the charge is already
/// terminal.
///
/// Unreachable, because [`poll_charge`]'s own guard returns for a terminal
/// charge before the rail is asked. It is written out rather than removed
/// because the alternative is an `unwrap`-shaped access on a fact two separate
/// `match`es agree on rather than the compiler — and because if it ever does
/// become reachable, a rail saying the money went the other way from what the
/// merchant was told must not become a silent `Done`.
fn nothing_to_settle(
    job: &vpay_db::JobRow,
    charge_id: &str,
    state: ChargeState,
    kind: StatusKind,
) -> Outcome {
    if contradiction(kind, state) {
        log_contradiction(job, charge_id, state, kind);
    }
    Outcome::Done
}

/// The rail's own transaction id, off a `Succeeded` answer.
///
/// A `match` rather than an `unwrap`-shaped access because the answer and the
/// settlement are joined by a convention rather than by the compiler:
/// `vpay_core::settle` answers `Succeeded` only for `StatusKind::Succeeded`,
/// which this build only produces from [`ChargeStatus::Succeeded`].
fn succeeded_txn_id(status: &ChargeStatus) -> Option<&str> {
    match status {
        ChargeStatus::Succeeded { provider_txn_id } => provider_txn_id.as_deref(),
        _ => None,
    }
}

/// Moves a live charge to the state the rail's answer implies, then takes the
/// next rung of the ladder in that new state.
///
/// # Errors
///
/// [`JobError::Db`] for the state write or for what [`keep_polling`] records,
/// and [`JobError::Exhausted`] when the charge is past the horizon.
async fn advance_and_keep_polling(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    from: ChargeState,
    to: ChargeState,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    repositories
        .set_live_state(&charge.id, from.as_wire_str(), to.as_wire_str())
        .await?;
    keep_polling(repositories, job, charge, to, past_horizon, payload).await
}

/// The ladder's own state after an answer that moved nothing.
///
/// A `NotFound` extends the streak the recovery table counts; anything else
/// resets it, because the streak means *consecutive* denials.
fn stayed(payload: &PollChargePayload, kind: StatusKind, now: OffsetDateTime) -> PollChargePayload {
    if kind == StatusKind::NotFound {
        payload.saw_not_found(now)
    } else {
        payload.reset()
    }
}

/// The rail says it has no record of a charge whose submission we are not sure
/// reached it.
///
/// Splits on the charge's state, because the two cases have different
/// evidence. In `submitting` the recovery table applies in full and may
/// conclude the rail never received the charge. Past that — `submitted`, where
/// the rail *answered* our submit — the only honest reading is that the rail
/// has lost track of a charge it acknowledged, so the ladder keeps running and
/// the streak is recorded for an operator rather than acted on. Resubmitting
/// there would be safe on a push rail and wrong on a redirect one, where the
/// payer already holds a URL.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is a distinct fact the decision needs; bundling them into a \
              struct would hide which ones the branch actually reads"
)]
async fn recover(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    flow: vpay_core::ProviderFlow,
    policy: &RecoveryPolicy,
    now: OffsetDateTime,
    charge_age: time::Duration,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    let next_payload = payload.saw_not_found(now);

    if state != ChargeState::Submitting {
        tracing::warn!(
            job_id = %job.id,
            charge_id = %charge.id,
            state = %state.as_wire_str(),
            not_found_streak = next_payload.not_found_streak,
            "the rail has no record of a charge it had already acknowledged"
        );
        return keep_polling(
            repositories,
            job,
            charge,
            state,
            past_horizon,
            &next_payload,
        )
        .await;
    }

    match recovery_action(
        repositories,
        charge,
        flow,
        policy,
        now,
        charge_age,
        &next_payload,
    )
    .await?
    {
        RecoveryAction::FailDeadOrder => fail_dead_order(repositories, job, charge).await,
        RecoveryAction::Resubmit => {
            resubmit_then_escalate_if_late(
                repositories,
                job,
                charge,
                state,
                past_horizon,
                &next_payload,
            )
            .await
        }
        // `Advance` cannot follow a `NotFound` on an answered submit for a
        // charge still in `submitting` without the state having moved under
        // us; polling again is the harmless answer either way.
        //
        // `Wait` cannot be reached here at all: `poll_charge` evaluates the
        // same table over the same `now` and the same `charges.created_at`
        // before it asks the rail, and a `Wait` there returns without ever
        // reaching a rail answer. It is written into this arm rather than
        // given one of its own because the answer would be identical if the
        // clock ever made it reachable — take another rung, touch nothing.
        RecoveryAction::Poll | RecoveryAction::Advance(_) | RecoveryAction::Wait(_) => {
            keep_polling(
                repositories,
                job,
                charge,
                state,
                past_horizon,
                &next_payload,
            )
            .await
        }
    }
}

/// Sends a charge to the rail again under its **existing** reference.
///
/// The reference is read from `charges.provider_reference_id` and never
/// minted here: "a fresh reference on retry is how you double-charge a
/// customer" (`docs/flows/crash-safety.md`). The port's contract — a duplicate
/// submission is reported as `Submitted`, not as an error — is what makes this
/// safe even when the rail did receive the first one.
async fn resubmit_charge(
    repositories: &dyn Repositories,
    adapters: &Adapters,
    rails: &RailConfigs,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let payload: ResubmitPayload = decode(job)?;
    // The clock that comes back with the row is discarded here, and only
    // here: a resubmit decides nothing from an age — the decision that
    // scheduled it did — so there is no subtraction for it to be the second
    // half of.
    let charge = load_charge(repositories, job, &payload.charge_id)
        .await?
        .charge;
    let state = parse_state(job, &charge)?;
    if state != ChargeState::Submitting {
        // Something already resolved the ambiguity — a concurrent poll, or a
        // previous run of this job that committed and then lost its lease.
        return Ok(Outcome::Done);
    }

    let (adapter, config) = rail(rails, adapters, job, &charge.provider_code)?;
    let attempt = next_submit_attempt(repositories, &charge).await?;
    let submitted = submit_again(repositories, adapter, config, job, &charge, attempt).await?;
    commit_resubmission(repositories, job, &charge, submitted).await?;

    tracing::info!(
        job_id = %job.id,
        charge_id = %charge.id,
        provider_reference_id = %charge.provider_reference_id,
        attempt,
        "resubmitted a charge under its existing reference"
    );
    Ok(Outcome::Done)
}

/// The `provider_requests.attempt` number this resubmission will carry.
///
/// Supplied rather than derived by the database
/// (`vpay_db::provider_requests`' own reasoning): the ladder knows how many
/// times it has tried, and a `SELECT max(attempt) + 1` would race two retries
/// into the same number.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure.
async fn next_submit_attempt(
    repositories: &dyn Repositories,
    charge: &ChargeRow,
) -> Result<i32, JobError> {
    Ok(repositories
        .latest_submit_attempt(&charge.id)
        .await?
        .map_or(1, |row| row.attempt.saturating_add(1)))
}

/// The submit itself, wrapped in the attempt row that makes it auditable.
///
/// The same shape as [`query_status`], and for the same reason: the row is
/// written before the call and answered after it, so an attempt that got no
/// answer is distinguishable from one that was never made
/// (`docs/flows/crash-safety.md`).
///
/// # Errors
///
/// [`JobError::Db`] for the attempt row, and [`JobError::Provider`] for a rail
/// that refused or would not answer — recorded against the attempt row before
/// it is raised.
async fn submit_again(
    repositories: &dyn Repositories,
    adapter: &dyn ProviderAdapter,
    config: &ProviderConfig,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    attempt: i32,
) -> Result<vpay_provider::Submitted, JobError> {
    let attempt_id = repositories
        .insert_pending(
            &charge.id,
            &charge.provider_code,
            "submit",
            charge.provider_reference_id,
            attempt,
        )
        .await?;

    let charge_ref = charge_ref(job, charge)?;
    match adapter.submit(&charge_ref, config).await {
        Ok(submitted) => {
            record_answer(repositories, attempt_id, None).await?;
            Ok(submitted)
        }
        Err(error) => {
            record_failure(repositories, attempt_id, &error).await?;
            Err(JobError::Provider(error))
        }
    }
}

/// Commits what the rail answered: the charge leaves `submitting`, and its
/// poll job is there when it does.
///
/// One transaction for both, for the reason `enqueue_in_tx` exists at all: a
/// charge that reached `submitted` with no job behind it is a charge nothing
/// will ever drive to terminal. Normally the poll job already exists and the
/// enqueue writes nothing.
///
/// The transition counter is recorded **after** the commit, never inside it —
/// see `vpay_db::charges`' header.
///
/// # Errors
///
/// [`JobError::Poisoned`] for a payload that will not encode — raised before
/// the transaction opens, so the unit of work stays a unit of *storage* work —
/// and [`JobError::Db`] for the writes.
async fn commit_resubmission(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    submitted: vpay_provider::Submitted,
) -> Result<(), JobError> {
    let ref_extra = serde_json::Value::Object(
        submitted
            .ref_extra
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect(),
    );
    let payload = poll_payload(job, &charge.id)?;
    let dedupe_key = poll_dedupe_key(&charge.id);

    let submitted_charge = repositories
        .transaction(|tx| {
            // Borrowed, not moved: the caller's log line reads `charge` again.
            let charge = &charge;
            Box::pin(async move {
                let submitted_charge = tx
                    .mark_submitted(
                        &charge.id,
                        ChargeState::Submitted.as_wire_str(),
                        Some(&ref_extra),
                        submitted.redirect_url.as_deref(),
                    )
                    .await?;
                tx.enqueue_in_tx(
                    JobKind::PollCharge.as_wire_str(),
                    &dedupe_key,
                    &payload,
                    OffsetDateTime::now_utc(),
                )
                .await?;
                Ok::<_, DbError>(TxOutcome::Commit(submitted_charge))
            })
        })
        .await?
        .into_inner();

    repositories.record_left_submitting(&submitted_charge);
    Ok(())
}

/// Retires what has expired: idempotency records, client-assertion `jti`s,
/// job leases whose worker died, and checkout sessions past their horizon.
///
/// The first three were previously run once at `vpay-server` boot, which meant
/// a process that stayed up for a month never swept anything
/// (`docs/status.md`). Four independent statements, each its own transaction:
/// they share nothing, and one failing should not roll back the others' work.
/// Always reschedules — a sweep that found nothing is the healthy case.
///
/// The fourth is **not** a delete. `checkout_sessions.expire_due` moves rows
/// from `open` to `expired` (D10's 24 hours) and touches `payment_status`
/// never: a merchant asking a session what happened must still be told, and
/// only the label "is this still payable?" changes. Until Step 9's lane 1b
/// `expires_at` was written and read by nothing, so a session past its horizon
/// reported `open` until a merchant expired it by hand or the intent settled.
///
/// It lives here rather than in a job of its own because it is the same shape
/// as the other three — one unconditional statement, hourly, whose healthy
/// answer is zero — and a fifth `jobs.kind` would have needed a migration to
/// say nothing this one does not.
async fn sweep_expired(
    repositories: &dyn Repositories,
    policy: &RecoveryPolicy,
) -> Result<Outcome, JobError> {
    let idempotency = repositories.sweep_expired().await?;
    let assertions = repositories.delete_expired_client_assertion_jtis().await?;
    // Lease expiry is a separate reaper rather than a condition on `claim`,
    // so `jobs_claimable_idx`'s `locked_at IS NULL` predicate stays exact.
    // Not the *only* reaper, and it must not be: `crate::run_loop` reaps at
    // boot and on its own half-lease timer, because this job is itself a row
    // in `jobs` and a worker that died holding it would leave the sweep — and
    // therefore the reaping — unclaimable forever. Reaping here as well costs
    // one statement an hour and keeps the sweep's own description honest.
    let leases = repositories.reap_expired_leases(policy.lease).await?;
    // The instant is this process's, not Postgres's, because the horizon it
    // is compared against was computed in Rust at create — see
    // `vpay_db::CheckoutSessions::expire_due`.
    let sessions = repositories.expire_due(OffsetDateTime::now_utc()).await?;

    tracing::info!(
        idempotency_keys = idempotency,
        client_assertion_jtis = assertions,
        expired_leases = leases,
        checkout_sessions = sessions,
        "housekeeping sweep"
    );
    Ok(Outcome::RescheduleAfter(SWEEP_INTERVAL))
}

/// Enqueues a poll for every live charge nothing appears to be driving.
///
/// A backstop, never the mechanism: `vpay_db::jobs::enqueue_in_tx` inside the
/// confirm path's charge transaction is what makes all three crash-safety kill
/// points leave a job behind. This covers only what that transaction cannot —
/// charges written before the queue existed, and a job lost to operator error.
/// Every insert is `ON CONFLICT (dedupe_key) DO NOTHING`, so a charge that
/// already has a poll job is untouched, including one scheduled an hour out.
async fn scan_live_charges(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let now = OffsetDateTime::now_utc();
    let cutoff = now - stale_after();
    let charge_ids = repositories
        .live_charges_stale_since(cutoff, SCAN_BATCH)
        .await?;

    let mut enqueued = 0_usize;
    if !charge_ids.is_empty() {
        enqueued = repositories
            .transaction(|tx| {
                // Borrowed, not moved: the count below reads `charge_ids` again.
                let charge_ids = &charge_ids;
                Box::pin(async move {
                    let mut enqueued = 0_usize;
                    for charge_id in charge_ids {
                        let inserted = tx
                            .enqueue_in_tx(
                                JobKind::PollCharge.as_wire_str(),
                                &poll_dedupe_key(charge_id),
                                &poll_payload(job, charge_id)?,
                                now,
                            )
                            .await?;
                        if inserted {
                            enqueued += 1;
                        }
                    }
                    Ok::<_, JobError>(TxOutcome::Commit(enqueued))
                })
            })
            .await?
            .into_inner();
    }

    if enqueued > 0 {
        // Deliberately a warning, not an info line: in a healthy deployment
        // this number is zero, because the enqueue happens with the charge.
        // Any other number means a poll job went missing.
        tracing::warn!(
            unattended = charge_ids.len(),
            enqueued,
            "the backstop scan found live charges with no poll job"
        );
    }
    Ok(Outcome::RescheduleAfter(SCAN_INTERVAL))
}

// ---------------------------------------------------------------- helpers --

/// What to do with an answer that left the charge live: take the next rung of
/// the ladder, or escalate to `unresolved`.
///
/// Reached only for a non-terminal answer — a terminal one has already
/// settled, past the horizon exactly as before it. `past_horizon` is
/// [`poll_charge`]'s single evaluation of [`past_the_horizon`], passed down
/// rather than recomputed: two evaluations of a predicate over a clock that
/// has moved between them can disagree, and this one decides whether a human
/// is told about a charge.
///
/// Escalation returns [`JobError::Exhausted`] rather than an [`Outcome`], and
/// that is the whole point: `Exhausted` is the one error that keeps
/// [`crate::Decision::RetryAfter`] with `alert: true` at
/// [`crate::UNRESOLVED_POLL_INTERVAL`]. The charge stays live and stays polled
/// — hourly instead of on the ladder — because "a late success at hour 30 is
/// the normal transition".
async fn keep_polling(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    // Recorded before the escalation, so what this poll learned survives even
    // when the answer is an escalation rather than a reschedule.
    remember(repositories, job, payload).await?;

    if past_horizon {
        return escalate_to_unresolved(repositories, job, charge, state).await;
    }

    Ok(Outcome::RescheduleAfter(crate::poll_delay(attempt_index(
        job,
    ))))
}

/// How long this charge has been live, as **Postgres** measures it.
///
/// Both operands come off one `SELECT` (`vpay_db::Charges::get_by_id_as_of`):
/// `created_at` is what the confirm's transaction wrote, and `db_now` is the
/// `now()` the read evaluated. Nothing here consults
/// [`OffsetDateTime::now_utc`], and that is the whole point — this quantity
/// decides whether a `submitting` charge is a crash or a confirm still in
/// flight (`crate::recovery::recovery_step`) and whether a live charge is
/// escalated to `unresolved` ([`past_the_horizon`]), and a worker host whose
/// clock is a minute ahead of the database gets both wrong while every
/// timestamp involved still looks perfectly ordinary in `psql`.
fn charge_age(as_of: &ChargeAsOf) -> time::Duration {
    as_of.db_now - as_of.charge.created_at
}

/// Has this charge been live longer than `docs/flows/reconciler.md`'s
/// 24-hour horizon?
///
/// `charge_age` is measured from `charges.created_at`, which is written
/// *before* the rail is called — so it is the age of the payer's exposure and
/// not of our bookkeeping. A `Duration` that does not fit `time::Duration`
/// saturates to "never", which is the safe direction.
///
/// **The age is Postgres', not this host's**, for the reason
/// [`crate::recovery::recovery_step`] §"Both ages are measured by one clock"
/// gives: `created_at` is written by the database, so subtracting it from a
/// host clock compares two machines. Here the direction of that defect was
/// the milder one — a fast worker escalated a charge to `unresolved` early,
/// waking an operator rather than losing a payment — but it is the same
/// subtraction, so it takes the same age.
///
/// Called from exactly one place, [`poll_charge`], which evaluates it *above*
/// the crash-recovery block and carries the answer down. Both the placement
/// and the fact that this is not a gate on asking the rail are load-bearing:
/// `docs/reference/vpay-worker.md` §"The horizon is evaluated above the
/// crash-recovery block".
fn past_the_horizon(charge_age: time::Duration, policy: &RecoveryPolicy) -> bool {
    let horizon = time::Duration::try_from(policy.unresolved_after).unwrap_or(time::Duration::MAX);
    charge_age >= horizon
}

/// Marks a charge `unresolved` and fails the job with [`JobError::Exhausted`].
///
/// **Never returns `Ok`.** The charge stays *live*: `unresolved` is an
/// escalation, not a verdict, and a late success at hour 30 is the normal
/// transition (`docs/flows/reconciler.md`) — which is why the error is
/// `Exhausted`, hourly retry plus an alert, rather than anything that parks
/// the job. The state write is skipped when the charge is already
/// `unresolved`, which is what makes the hourly re-escalation idempotent.
///
/// `docs/reference/vpay-worker.md` §"One poll" says why the return type is a
/// `Result<Outcome, _>` that never answers `Ok`, and which test would fail if
/// the idempotence became a silent no-op.
///
/// # Errors
///
/// Always: [`JobError::Exhausted`], or [`JobError::Db`] if the state write
/// fails — a database failure here is a database failure, not an escalation.
async fn escalate_to_unresolved(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
) -> Result<Outcome, JobError> {
    if state != ChargeState::Unresolved {
        repositories
            .set_live_state(
                &charge.id,
                state.as_wire_str(),
                ChargeState::Unresolved.as_wire_str(),
            )
            .await?;
    }
    Err(JobError::Exhausted {
        job_id: job.id,
        attempts: attempt_index(job).saturating_add(1),
    })
}

/// Fails a redirect-rail charge whose submit response was lost.
///
/// `docs/flows/crash-safety.md`: "that `order_id` is dead: abandon it and let
/// the merchant create a new PaymentIntent." Safe only because the payer is
/// redirected strictly after the rail's token is committed, so a charge still
/// in `submitting` is one nobody could have paid — see
/// [`crate::recovery::recovery_step`].
async fn fail_dead_order(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
) -> Result<Outcome, JobError> {
    const RAW: &str = "the rail's submit response was lost before its token could be \
                       committed; the payer was never handed a redirect URL, so this \
                       order can never be queried and no payment can have occurred \
                       (docs/flows/crash-safety.md)";
    settle_failed_with(
        repositories,
        job,
        charge,
        FailureCode::ProviderUnavailable,
        RAW,
        "abandoned a redirect charge whose submit response was lost",
    )
    .await
}

/// Commits a rail-reported success.
async fn settle_succeeded(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    provider_txn_id: Option<&str>,
) -> Result<Outcome, JobError> {
    let data = intent_snapshot(
        repositories,
        job,
        &charge.payment_intent_id,
        IntentStatus::Succeeded,
        None,
    )
    .await?;
    let settled = repositories
        .apply_succeeded(&charge.id, provider_txn_id, &ids::event_id(), &data)
        .await?;

    match settled {
        Some(_) => tracing::info!(
            job_id = %job.id,
            charge_id = %charge.id,
            payment_intent_id = %charge.payment_intent_id,
            "the rail reported a charge as paid"
        ),
        // The compare-and-swap matched nothing, so the charge went terminal
        // between this run's read and its write. Usually that is a duplicate
        // run agreeing with itself; if the charge is `failed`, the rail has
        // just contradicted what the merchant was told.
        None => report_late_answer(repositories, job, charge, StatusKind::Succeeded).await,
    }
    Ok(Outcome::Done)
}

/// Commits a rail-reported decline.
async fn settle_failed(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    code: FailureCode,
    status: &ChargeStatus,
) -> Result<Outcome, JobError> {
    let raw = match status {
        ChargeStatus::Failed { raw, .. } => raw.as_str(),
        // Unreachable: `settle` answers `Failed` only for
        // `StatusKind::Failed`, which this build only produces from
        // `ChargeStatus::Failed`. The fallback carries the code rather than an
        // empty string so `charges.failure_raw` is never blank.
        _ => code.as_str(),
    };
    settle_failed_with(
        repositories,
        job,
        charge,
        code,
        raw,
        "the rail reported a charge as failed",
    )
    .await
}

async fn settle_failed_with(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    code: FailureCode,
    raw: &str,
    log_message: &'static str,
) -> Result<Outcome, JobError> {
    let message = merchant_message(code, raw);
    let data = intent_snapshot(
        repositories,
        job,
        &charge.payment_intent_id,
        IntentStatus::RequiresPaymentMethod,
        Some((code, &message)),
    )
    .await?;
    let settled = repositories
        .apply_failed(
            &charge.id,
            code.as_str(),
            raw,
            &message,
            &ids::event_id(),
            &data,
        )
        .await?;

    match settled {
        Some(_) => tracing::warn!(
            job_id = %job.id,
            charge_id = %charge.id,
            payment_intent_id = %charge.payment_intent_id,
            failure_code = %code,
            "{log_message}"
        ),
        // As in `settle_succeeded`: nothing was written because the charge is
        // already terminal. A `succeeded` charge here is the rail reversing a
        // payment the merchant has been told about.
        None => report_late_answer(repositories, job, charge, StatusKind::Failed(code)).await,
    }
    Ok(Outcome::Done)
}

/// Says whether a rail answer that arrived too late to be written *disagrees*
/// with what was written instead.
///
/// The charge's state is re-read rather than taken from the poll's own copy,
/// because that copy is exactly what is out of date: the settlement's
/// compare-and-swap matched nothing precisely because something else moved
/// the row after this run read it. Reading the stored state is the only way
/// to know which terminal state won.
///
/// A read that fails is logged and dropped rather than failing the job. The
/// job's work is finished either way — this is a diagnosis, and turning a
/// completed settlement into a retry because a *diagnostic* read failed would
/// re-run the whole poll for nothing.
async fn report_late_answer(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    kind: StatusKind,
) {
    let stored = match Charges::get_by_id(repositories, &charge.id).await {
        Ok(Some(row)) => row,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                job_id = %job.id,
                charge_id = %charge.id,
                error = %error,
                "could not re-read a charge that settled under us; a rail contradiction \
                 would go unreported"
            );
            return;
        }
    };
    let Some(state) = ChargeState::from_wire(&stored.state) else {
        return;
    };
    if contradiction(kind, state) {
        log_contradiction(job, &charge.id, state, kind);
    }
}

/// The alert for a rail that reports the opposite of what vpay recorded.
///
/// `alert = true` and `Level::ERROR`: this is money moving differently from
/// what a merchant has already been told, and it is the one thing in this
/// module that a human must reconcile against the rail's settlement statement
/// (`docs/runbooks/unresolved-charges.md`). vpay deliberately does **not**
/// act on it — a charge settles once, and a poll that could flip `failed` to
/// `succeeded` would make the settlement compare-and-swap decorative — so the
/// log line is the entire response, and it names both states and the rail's
/// answer so the reconciliation can start from it alone.
fn log_contradiction(job: &vpay_db::JobRow, charge_id: &str, state: ChargeState, kind: StatusKind) {
    tracing::error!(
        alert = true,
        job_id = %job.id,
        charge_id = %charge_id,
        charge_state = %state.as_wire_str(),
        rail_answer = %status_kind_label(kind),
        "the rail reports the opposite of this charge's settled state; vpay has not changed \
         the charge — reconcile it against the rail's settlement statement"
    );
}

/// A [`StatusKind`] as one operator-facing word, for the contradiction alert.
///
/// Separate from [`status_label`], which takes the port's [`ChargeStatus`]:
/// by the time a contradiction is reported the answer has already been
/// reduced to the state machine's own vocabulary, and re-deriving it from the
/// port type would mean carrying the whole answer through two more frames for
/// one string.
const fn status_kind_label(kind: StatusKind) -> &'static str {
    match kind {
        StatusKind::Pending => "pending",
        StatusKind::Succeeded => "succeeded",
        StatusKind::Failed(_) => "failed",
        StatusKind::NotFound => "not_found",
    }
}

/// The recovery table said "resubmit". Do that — and, past the horizon,
/// escalate on top of it.
///
/// Both `Resubmit` arms go through here, which is the point: the arm in
/// [`poll_charge`]'s crash-recovery block and the one in [`recover`] are the
/// same decision reached from different evidence, and only one of them used to
/// know about the horizon.
///
/// The resubmit row commits first and the escalation second, in two
/// transactions, so past the horizon the escalation ordinarily *supersedes*
/// the resubmit rather than running alongside it — but not always.
/// `docs/reference/vpay-worker.md` §"Resubmit and escalate is a real
/// non-determinism" states what the horizon does and does not guarantee here,
/// and why that is deliberate.
async fn resubmit_then_escalate_if_late(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    let rescheduled = schedule_resubmit(repositories, job, charge, payload).await?;
    if past_horizon {
        return escalate_to_unresolved(repositories, job, charge, state).await;
    }
    Ok(rescheduled)
}

/// Enqueues a resubmit and puts the ladder back on the clock.
///
/// The poll job is rescheduled rather than finished, so the resubmit's result
/// is polled by the job that was already tracking this charge instead of a
/// fresh ladder starting at rung zero.
async fn schedule_resubmit(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    let enqueued = repositories
        .transaction(|tx| {
            // Borrowed, not moved: the log line below reads `charge` again.
            let charge = &charge;
            Box::pin(async move {
                let enqueued = tx
                    .enqueue_in_tx(
                        JobKind::ResubmitCharge.as_wire_str(),
                        &resubmit_dedupe_key(&charge.id),
                        &encode(job, &ResubmitPayload::new(charge.id.clone()))?,
                        OffsetDateTime::now_utc(),
                    )
                    .await?;
                Ok::<_, JobError>(TxOutcome::Commit(enqueued))
            })
        })
        .await?
        .into_inner();

    if enqueued {
        tracing::warn!(
            job_id = %job.id,
            charge_id = %charge.id,
            not_found_streak = payload.not_found_streak,
            "the rail has repeatedly denied all knowledge of a charge; resubmitting \
             under the same reference"
        );
    }

    // A resubmit *is* the reset: the ladder that follows it starts fresh, so
    // a threshold that has just been satisfied cannot immediately re-fire.
    remember(repositories, job, &payload.reset()).await?;
    Ok(Outcome::RescheduleAfter(crate::poll_delay(attempt_index(
        job,
    ))))
}

/// The one authenticated status read, wrapped in the attempt row that makes
/// it auditable.
async fn query_status(
    repositories: &dyn Repositories,
    adapter: &dyn ProviderAdapter,
    config: &ProviderConfig,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
) -> Result<ChargeStatus, JobError> {
    let charge_ref = charge_ref(job, charge)?;
    let attempt_id = repositories
        .insert_pending(
            &charge.id,
            &charge.provider_code,
            "query_status",
            charge.provider_reference_id,
            attempt_number(job),
        )
        .await?;

    match adapter.query_status(&charge_ref, config).await {
        Ok(status) => {
            // The answer's own label, so an operator can tell a `NotFound`
            // run of the ladder from a `Pending` one without replaying it.
            record_answer(repositories, attempt_id, Some(status_label(&status))).await?;
            Ok(status)
        }
        Err(error) => {
            record_failure(repositories, attempt_id, &error).await?;
            Err(JobError::Provider(error))
        }
    }
}

/// Records an attempt the rail answered.
///
/// The status is [`vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT`]
/// and never a plausible HTTP number: the port deliberately does not expose
/// the rail's status line, and inventing one here would be a per-rail guess in
/// the layer that is forbidden to know which rail it is talking to.
async fn record_answer(
    repositories: &dyn Repositories,
    attempt_id: i64,
    label: Option<&'static str>,
) -> Result<(), JobError> {
    repositories
        .record_response(
            attempt_id,
            Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT),
            label,
        )
        .await?;
    Ok(())
}

/// Records an attempt that got no answer at all.
///
/// `status_code` stays `NULL`, which is the encoding for "no response was
/// received" and the thing a recovery sweep looks for. `error_kind` carries
/// the error's own classification code — operator-facing, never a merchant's
/// vocabulary.
async fn record_failure(
    repositories: &dyn Repositories,
    attempt_id: i64,
    error: &ProviderError,
) -> Result<(), JobError> {
    repositories
        .record_response(attempt_id, None, Some(error.code()))
        .await?;
    Ok(())
}

/// The rail-facing view of a charge.
///
/// `ref_extra` is read back out of the row rather than rebuilt, because on a
/// redirect rail it carries the `pay_token` without which the rail will not
/// answer at all.
///
/// `return_url` is `charges.return_url`, the merchant's own — the durable
/// value written before the first submit — and it is deliberately *not* the
/// checkout session's return page that `vpay_api`'s confirm path may have
/// sent instead (Step 9, D2). The two cannot disagree on anything this
/// function feeds: only a **push** rail is ever resubmitted, and
/// [`crate::recovery::recovery_step`] answers `FailDeadOrder` for a redirect
/// rail before it looks at anything else, so no redirect charge reaches
/// `submit` from here. A push rail has no browser and ignores the field
/// entirely. The day a redirect rail becomes resubmittable, the session's URL
/// has to become readable from this row or this line is a silent divergence
/// — `docs/plans/step9-notes/lane-2.md` records that as the condition.
fn charge_ref(job: &vpay_db::JobRow, charge: &ChargeRow) -> Result<ChargeRef, JobError> {
    let currency = vpay_core::Currency::from_code(&charge.currency_code).map_err(|error| {
        poisoned(
            job,
            format!("charge {} holds an unknown currency: {error}", charge.id),
        )
    })?;
    Ok(ChargeRef {
        reference_id: charge.provider_reference_id,
        amount: Money::new(charge.amount, currency)?,
        payer_ref: charge.payer_ref.clone(),
        ref_extra: ref_extra_of(charge),
        return_url: charge.return_url.clone(),
    })
}

/// `charges.provider_ref_extra` as the port's own map.
///
/// Non-string values are dropped rather than stringified: [`RefExtra`] is rail
/// key material this crate must not interpret, and a number that arrived as
/// `"12"` on one path and `12` on another would be two different tokens.
/// Nothing in this repository writes anything but strings there.
fn ref_extra_of(charge: &ChargeRow) -> RefExtra {
    charge
        .provider_ref_extra
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|text| (key.clone(), text.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What `provider_requests` says about the last attempt to submit this charge.
async fn submit_evidence(
    repositories: &dyn Repositories,
    charge_id: &str,
) -> Result<SubmitAttempt, JobError> {
    let latest = repositories.latest_submit_attempt(charge_id).await?;
    Ok(match latest {
        None => SubmitAttempt::Never,
        Some(row) => match row.status_code {
            None => SubmitAttempt::Unanswered,
            Some(code) => SubmitAttempt::Answered(code),
        },
    })
}

/// The wire object as it will stand once the settlement transaction commits.
///
/// Rendered through `vpay_api::model::PaymentIntentObject`, the same type
/// `GET /v1/payment_intents/{id}` returns, because `events.data` is a
/// *snapshot of the object* (migration 0018) delivered verbatim to a
/// merchant's handler. A second, hand-written copy of that shape is how the
/// webhook body and the API response start disagreeing about a field.
///
/// The projection is applied **before** the write, because the settlement
/// takes `event_data` as an input — the event is written inside the same
/// transaction as the row it describes, so it cannot be rendered from the
/// result. The two fields patched are exactly the ones the settlement changes
/// and the object renders; `amount_received` is not patched because the object
/// does not carry it.
async fn intent_snapshot(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    payment_intent_id: &str,
    status: IntentStatus,
    error: Option<(FailureCode, &str)>,
) -> Result<serde_json::Value, JobError> {
    let mut row = PaymentIntents::get_by_id(repositories, payment_intent_id)
        .await?
        .ok_or_else(|| {
            poisoned(
                job,
                format!("charge names payment intent {payment_intent_id}, which does not exist"),
            )
        })?;

    row.status = status.as_wire_str().to_owned();
    if let Some((code, message)) = error {
        row.last_payment_error_code = Some(code.as_str().to_owned());
        row.last_payment_error_message = Some(message.to_owned());
    }

    let object = vpay_api::model::PaymentIntentObject::try_from(&row).map_err(|error| {
        // Only reachable for a row the schema should make impossible — an
        // unparseable status, a `payment_method_types` that is not an array.
        // That is a poisoned job in the precise sense: re-running cannot fix
        // data that is already wrong.
        poisoned(
            job,
            format!("payment intent {payment_intent_id} cannot be rendered: {error}"),
        )
    })?;
    encode(job, &object)
}

/// The sentence a merchant is shown for a decline this worker discovered.
///
/// Built through `ProviderError::Rejected::public_message` rather than written
/// out, so a decline found by the poll ladder reads *identically* to the same
/// decline found at submit time (`vpay-api`'s confirm path uses the same
/// call). The rail's raw words are never in it — they go to
/// `charges.failure_raw`, for an operator (`docs/flows/errors.md`).
fn merchant_message(code: FailureCode, raw: &str) -> String {
    ProviderError::Rejected {
        code,
        message: raw.to_owned(),
    }
    .public_message()
}

/// Writes the ladder's own state back to the job row, under this worker's
/// lease.
///
/// A no-op when nothing changed, so an ordinary `Pending` poll does not write
/// to `jobs` at all. `Ok(false)` from the write means the lease has moved on
/// and this worker's answer is being discarded anyway — the loop discovers the
/// same thing when it tries to reschedule, so this does not raise it twice.
async fn remember(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    payload: &PollChargePayload,
) -> Result<(), JobError> {
    let encoded = encode(job, payload)?;
    if encoded == job.payload {
        return Ok(());
    }
    let Some(worker_id) = job.locked_by.as_deref() else {
        // `lock_is_paired` makes this unreachable for a claimed row.
        return Err(poisoned(
            job,
            "a claimed job row carries no `locked_by`".to_owned(),
        ));
    };
    repositories
        .set_payload(job.id, worker_id, &encoded)
        .await?;
    Ok(())
}

/// A fresh poll payload for one charge, as JSONB.
///
/// The enqueue itself is still written out at each call site: the payload is
/// the only part the three sites share, and each one belongs to a different
/// transaction. Before Step 7 a shared helper was not even expressible — it
/// would have had to name `sqlx::PgConnection`, which this crate deliberately
/// does not depend on.
fn poll_payload(job: &vpay_db::JobRow, charge_id: &str) -> Result<serde_json::Value, JobError> {
    encode(job, &PollChargePayload::new(charge_id))
}

/// Loads the charge a job names, or says the row is poisoned.
async fn load_charge(
    repositories: &dyn Repositories,
    job: &vpay_db::JobRow,
    charge_id: &str,
) -> Result<ChargeAsOf, JobError> {
    Charges::get_by_id_as_of(repositories, charge_id)
        .await?
        .ok_or_else(|| poisoned(job, format!("charge {charge_id} does not exist")))
}

/// Parses `charges.state`, or says the row is poisoned.
///
/// A label outside the enum is a schema/code mismatch, not a merchant's
/// mistake and not something a retry can fix.
fn parse_state(job: &vpay_db::JobRow, charge: &ChargeRow) -> Result<ChargeState, JobError> {
    ChargeState::from_wire(&charge.state).ok_or_else(|| {
        poisoned(
            job,
            format!(
                "charge {} holds state `{}`, which is not a ChargeState",
                charge.id, charge.state
            ),
        )
    })
}

/// The adapter and configuration for a rail, or a configuration error.
///
/// A charge naming a rail this deployment has no adapter or no configuration
/// for is [`ProviderError::Config`] rather than a poisoned job: the row is
/// fine, the *deployment* is missing a rail, and that is fixed by editing YAML
/// and restarting rather than by deleting a job.
fn rail<'a>(
    rails: &'a RailConfigs,
    adapters: &'a Adapters,
    job: &vpay_db::JobRow,
    code: &str,
) -> Result<(&'a dyn ProviderAdapter, &'a ProviderConfig), JobError> {
    let adapter = adapters.get(code).ok_or_else(|| {
        ProviderError::Config(format!(
            "job {} names rail `{code}`, which this build has no adapter for",
            job.id
        ))
    })?;
    let config = rails.get(code).ok_or_else(|| {
        ProviderError::Config(format!(
            "job {} names rail `{code}`, which this deployment has no configuration for",
            job.id
        ))
    })?;
    Ok((adapter.as_ref(), config))
}

/// The port's answer, reduced to what the state machine reads.
fn status_kind(status: &ChargeStatus) -> StatusKind {
    match status {
        ChargeStatus::Pending => StatusKind::Pending,
        ChargeStatus::Succeeded { .. } => StatusKind::Succeeded,
        ChargeStatus::Failed { code, .. } => StatusKind::Failed(*code),
        ChargeStatus::NotFound => StatusKind::NotFound,
    }
}

/// The same answer as an `error_kind` label for `provider_requests`.
///
/// Operator-facing, and bounded by the column's 128 characters by
/// construction: these are the only four values this code ever writes.
const fn status_label(status: &ChargeStatus) -> &'static str {
    match status {
        ChargeStatus::Pending => "pending",
        ChargeStatus::Succeeded { .. } => "succeeded",
        ChargeStatus::Failed { .. } => "failed",
        ChargeStatus::NotFound => "not_found",
    }
}

/// This job's position on the poll ladder, 0-indexed.
///
/// `jobs.attempts` counts from one and is incremented by the claim itself, so
/// the run that is executing now is attempt `attempts` and its rung is one
/// less. Same indexing as [`JobError::decision`], so a handler that reschedules
/// and a handler that fails wait the same length of time.
fn attempt_index(job: &vpay_db::JobRow) -> u32 {
    u32::try_from(job.attempts)
        .unwrap_or(u32::MAX)
        .saturating_sub(1)
}

/// The `provider_requests.attempt` number for this run. One-based, because
/// `attempt_is_positive` requires it.
fn attempt_number(job: &vpay_db::JobRow) -> i32 {
    job.attempts.max(1)
}

/// Deserialises a job payload into the shape its `kind` promises.
fn decode<T: DeserializeOwned>(job: &vpay_db::JobRow) -> Result<T, JobError> {
    serde_json::from_value(job.payload.clone()).map_err(|error| {
        poisoned(
            job,
            format!("payload does not match kind `{}`: {error}", job.kind),
        )
    })
}

/// Serialises a payload or a wire object for a JSONB column.
///
/// Fallible only for shapes this crate does not build (a map with non-string
/// keys, a float that is not a number); the failure is reported as a poisoned
/// job because, like a payload of the wrong shape, no retry changes it.
fn encode<T: Serialize>(job: &vpay_db::JobRow, value: &T) -> Result<serde_json::Value, JobError> {
    serde_json::to_value(value)
        .map_err(|error| poisoned(job, format!("value could not be encoded as JSON: {error}")))
}

fn poisoned(job: &vpay_db::JobRow, reason: String) -> JobError {
    JobError::Poisoned {
        job_id: job.id,
        reason,
    }
}

/// [`SCAN_INTERVAL`] as the `time` crate spells durations, for comparing
/// against a column.
fn stale_after() -> time::Duration {
    time::Duration::try_from(SCAN_INTERVAL).unwrap_or(time::Duration::MAX)
}

/// Logs a job failure at the level its classification implies, and counts it.
///
/// `alert = true` for [`Severity::Page`] and nothing else, so an alerting rule
/// can select pages without also firing on every rail timeout. This is also
/// where a job failure reaches `vpay_error_events_total` and, for a `Page`,
/// `vpay_alert_events_total` — the counter and the field are one decision read
/// off one [`Classify`] impl, so no log-scraping layer can classify the same
/// failure differently.
///
/// The four arms exist separately because `alert` is an event *field* and has
/// to be written at the macro call site; see [`crate::tracing_level`].
/// `docs/reference/vpay-worker.md` §"Where a job failure is logged" explains
/// the second line `crate::run_loop` writes for the same failure, and why it
/// deliberately does not increment.
fn log_failure(job: &vpay_db::JobRow, error: &JobError) {
    let severity = error.severity();
    let code = error.code();
    vpay_core::metrics::record_error_event(error);
    match severity {
        Severity::Page => tracing::error!(
            alert = true,
            job_id = %job.id,
            kind = %job.kind,
            attempts = job.attempts,
            code,
            "job failed: {error}"
        ),
        Severity::Error => tracing::error!(
            job_id = %job.id,
            kind = %job.kind,
            attempts = job.attempts,
            code,
            "job failed: {error}"
        ),
        Severity::Warn => tracing::warn!(
            job_id = %job.id,
            kind = %job.kind,
            attempts = job.attempts,
            code,
            "job failed: {error}"
        ),
        Severity::Info => tracing::info!(
            job_id = %job.id,
            kind = %job.kind,
            attempts = job.attempts,
            code,
            "job failed: {error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A charge row that is `created_at` old and otherwise inert.
    ///
    /// Only `created_at` is read by the function under test; every other
    /// field is filled with the narrowest value the type allows, so a future
    /// reader is not misled into thinking any of them matters here.
    fn charge_created(created_at: OffsetDateTime) -> ChargeRow {
        ChargeRow {
            id: "ch_horizon".to_owned(),
            payment_intent_id: "pi_horizon".to_owned(),
            provider_code: "mtn_momo".to_owned(),
            provider_reference_id: uuid::Uuid::nil(),
            provider_ref_extra: None,
            provider_txn_id: None,
            redirect_url: None,
            return_url: None,
            state: ChargeState::Submitted.as_wire_str().to_owned(),
            amount: 5_000,
            currency_code: "XAF".to_owned(),
            payer_ref: None,
            payer_ref_masked: None,
            failure_code: None,
            failure_raw: None,
            created_at,
            updated_at: created_at,
        }
    }

    /// The 24-hour horizon at the documented policy, on both sides of it and
    /// exactly on it.
    ///
    /// Written against a real interval rather than `unresolved_after: ZERO` —
    /// the trick the integration cases use to put the horizon behind a charge
    /// created a moment ago — because zero makes the comparison true for
    /// *every* age and so proves nothing about where the boundary is. A
    /// predicate that decides whether a human is alerted about a payment is
    /// worth pinning at its edge: `>=`, so a charge exactly 24 hours old is
    /// past it, and one a second short is not.
    ///
    /// These three are the only unit tests in this file, and they do not
    /// contradict the module header: `charge_age` and `past_the_horizon` are
    /// pure functions of a timestamp pair and a policy, not one of the write
    /// sequences that header is about.
    #[test]
    fn the_horizon_is_twenty_four_hours_of_real_time() {
        let policy = RecoveryPolicy::default();
        let aged = |d: time::Duration| past_the_horizon(d, &policy);

        assert!(
            !aged(time::Duration::hours(23) + time::Duration::minutes(59)),
            "a charge 23h59m old is still on the ladder; escalating early puts charges on \
             an operator's list that the rail is still working on"
        );
        assert!(
            !aged(time::Duration::hours(24) - time::Duration::seconds(1)),
            "one second short of the horizon is short of the horizon"
        );
        assert!(
            aged(time::Duration::hours(24)),
            "`reconciler.md` says at 24 hours, and the comparison is `>=`"
        );
        assert!(
            aged(time::Duration::hours(30)),
            "hour 30 is where the document's own example of a late success lives, and it \
             is reached by staying past the horizon rather than by leaving it"
        );
    }

    /// A charge from the future is not past the horizon.
    ///
    /// `db_now - created_at` is signed, so a row written by a replica whose
    /// clock is ahead produces a negative age. It must read as "young", not
    /// wrap into an escalation: `time::Duration` is signed and the comparison
    /// is against a positive horizon, which is what makes that hold.
    #[test]
    fn a_clock_skewed_charge_is_not_escalated() {
        let policy = RecoveryPolicy::default();
        assert!(!past_the_horizon(-time::Duration::hours(1), &policy));
    }

    /// **The age is the database's measurement, and this host's clock has no
    /// vote.**
    ///
    /// The charge was written at the epoch and read by a statement whose
    /// `now()` was five seconds later, so its age is five seconds — however
    /// far from the epoch the machine running this test happens to be. The
    /// second half is what makes the case decisive rather than tautological:
    /// the age the *host* clock would have produced for the same row is
    /// measured too, and it is past the 24-hour horizon, so an implementation
    /// that reached for `OffsetDateTime::now_utc()` here would escalate this
    /// charge to `unresolved` on its first poll instead of leaving it on the
    /// ladder. The same subtraction decides the recovery window, where the
    /// same skew silently disables the guard
    /// (`crate::recovery::recovery_step` §"Both ages are measured by one
    /// clock").
    #[test]
    fn the_age_is_measured_by_the_database_and_not_by_this_host() {
        let policy = RecoveryPolicy::default();
        let created_at = OffsetDateTime::UNIX_EPOCH;
        let as_of = ChargeAsOf {
            charge: charge_created(created_at),
            db_now: created_at + time::Duration::seconds(5),
        };

        assert_eq!(
            charge_age(&as_of),
            time::Duration::seconds(5),
            "the age must be `db_now - created_at`, both off the one statement that read \
             the row"
        );
        assert!(!past_the_horizon(charge_age(&as_of), &policy));

        let by_this_host = OffsetDateTime::now_utc() - created_at;
        assert!(
            past_the_horizon(by_this_host, &policy),
            "the case is only decisive while the host clock disagrees with the fixture's \
             `db_now`; a machine set to 1970 would make it prove nothing"
        );
    }
}
