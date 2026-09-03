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
//! **`vpay-worker` has no `sqlx` dependency and must not grow one.** Every
//! statement lives in a `vpay-db` repository function. Where two of those
//! functions must commit together this module opens a transaction with
//! `PgPool::begin` and passes it straight through, so no `sqlx` *path* is
//! written here — the pool and the transaction arrive as `vpay_db::PgPool`
//! and are handed straight back, which is also why the two `enqueue_in_tx`
//! calls are written out at their call sites instead of behind a shared
//! helper: a helper would have to *name* `sqlx::PgConnection` in its own
//! signature. `vpay_db::PgPool` is `sqlx::PgPool` one re-export away, so this
//! is a rule about where statements live, not a claim that the type is
//! absent.
//!
//! # How these are tested
//!
//! The handlers are not unit-tested in this crate, and that is deliberate.
//! (The one `#[cfg(test)]` module at the bottom of this file covers
//! `past_the_horizon`, which is a pure function of a timestamp and a policy
//! and writes nothing — it is not one of the sequences this paragraph is
//! about, and it is here rather than in `recovery` because it reads a
//! `vpay_db::ChargeRow`.) Every handler below is a sequence of writes against
//! real Postgres and a real
//! rail; the only way to test it in-process would be to introduce a fake pool
//! or a fake [`ProviderAdapter`], and AGENTS.md's first rule forbids a test
//! double reachable from a shipping binary (ADR-0006: the stub rail is a
//! WireMock host in configuration, reached over HTTP exactly as a real rail
//! is). So the proofs live in `backends/tests/integration/tests/`, which
//! drives *these functions* against a Postgres container and a WireMock
//! container, and reproduces each crash-safety kill point by writing the state
//! a crash leaves behind. The pure parts they depend on — the settlement
//! table, the recovery table, the payload encoding — are unit-tested in their
//! own modules, which is why those are separate modules at all.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use vpay_core::state::{ChargeState, IntentStatus};
use vpay_core::{
    Classify, FailureCode, Money, Settlement, Severity, StatusKind, contradiction, ids, settle,
};
use vpay_db::{ChargeRow, DbError, PgPool};
use vpay_provider::{
    ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig, ProviderError, RefExtra,
};

use crate::error::JobError;
use crate::jobs::{
    JobKind, Outcome, PollChargePayload, ResubmitPayload, poll_dedupe_key, resubmit_dedupe_key,
};
use crate::recovery::{RecoveryAction, RecoveryPolicy, SubmitAttempt, recovery_step};

/// Every rail this deployment can talk to, by `providers.code`.
///
/// The same map `vpay_api::v1::boot::adapters_by_code` builds for the server.
pub type Adapters = BTreeMap<String, Box<dyn ProviderAdapter>>;

/// Each rail's deployment configuration, by `providers.code`.
///
/// Plain values rather than `vpay_api::v1::ResourceConfig`, although the
/// worker binary already links `vpay-api` and could pass the projection
/// itself. Two reasons, and neither is dogma about layering:
///
/// * a handler needs exactly one thing from that projection —
///   `RailConfig::provider_config()` for the charge's rail — while
///   `ResourceConfig` also carries the merchant-client table and the
///   deployment's currency list, neither of which any job may read;
/// * the flow shape, which is the *only* thing the recovery table branches on,
///   comes from `ProviderAdapter::capabilities()` and not from configuration
///   at all, so there is nothing left for a config type to supply.
///
/// The binary builds this in one line from the projection it already has:
/// `resource_config.rail(code).map(RailConfig::provider_config)` for each
/// adapter code.
pub type RailConfigs = BTreeMap<String, ProviderConfig>;

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

/// Runs one claimed job, and says what the loop should do with the row.
///
/// The loop owns the row: it claimed it, and it is what calls
/// [`vpay_db::jobs::finish`] or [`vpay_db::jobs::reschedule`] afterwards. This
/// function only says which. On `Err`, the loop derives the same answer from
/// [`JobError::decision`] — see [`Outcome::from_decision`].
///
/// Failures are logged here rather than only at the loop, at the level
/// [`Classify::severity`] implies and with `alert = true` when that severity
/// is [`Severity::Page`], because this is the frame that still knows *which*
/// job and which charge failed. `crate::tracing_level` documents why the flag
/// cannot be attached by a helper.
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
    pool: &PgPool,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let result = dispatch(pool, adapters, rails, policy, job).await;
    if let Err(error) = &result {
        log_failure(job, error);
    }
    result
}

async fn dispatch(
    pool: &PgPool,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
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
        JobKind::PollCharge => poll_charge(pool, adapters, rails, policy, job).await,
        JobKind::ResubmitCharge => resubmit_charge(pool, adapters, rails, job).await,
        JobKind::SweepExpired => sweep_expired(pool, policy).await,
        JobKind::ScanLiveCharges => scan_live_charges(pool, job).await,
    }
}

/// Asks the rail about one charge and settles it if the answer is terminal.
///
/// The order is the one `docs/flows/crash-safety.md` requires and never a
/// convenient rearrangement: the attempt row is written *before* the call and
/// answered *after* it, so an attempt that got no answer is distinguishable
/// from one that was never made — which is the single fact the recovery table
/// branches on.
async fn poll_charge(
    pool: &PgPool,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let payload: PollChargePayload = decode(job)?;
    let charge = load_charge(pool, job, &payload.charge_id).await?;
    let mut state = parse_state(job, &charge)?;
    if state.is_terminal() {
        // A duplicate job, a callback that arrived after the answer, or a
        // second worker that lost a race. Not an error: the work is done.
        return Ok(Outcome::Done);
    }

    let (adapter, config) = rail(rails, adapters, job, &charge.provider_code)?;
    let flow = adapter.capabilities().flow;
    let now = OffsetDateTime::now_utc();

    // The horizon is a property of the charge and the clock alone, so it is
    // evaluated once, here, and carried down: nothing the rail says can move
    // it, and there is exactly one place it is decided.
    //
    // It does **not** decide whether to ask the rail. Past the horizon the
    // charge is still polled — hourly rather than on the ladder — because "a
    // late success — minute 40, or hour 30 from `unresolved` — is the normal
    // transition" (`docs/flows/reconciler.md`), and a poll that stopped asking
    // would be the thing that lost it. What the horizon decides is what
    // happens to every answer *short of* a terminal one: escalate, instead of
    // taking another rung.
    //
    // Evaluated *above* the crash-recovery block and not below it, which is
    // the one thing about its placement that is load-bearing. That block can
    // return without ever reaching this line — and one of its arms,
    // `Resubmit`, returns a rung of the ladder. A `submitting` charge whose
    // resubmit job is dead-lettered comes back to it on every poll
    // (`SubmitAttempt::Never` → `Resubmit`, forever), so a horizon evaluated
    // afterwards would never be evaluated at all for exactly the charge that
    // has been stuck longest.
    let past_horizon = past_the_horizon(&charge, policy, now);

    // The crash-recovery branch, and only for `submitting`: past that state
    // the payer may already hold a redirect URL, which is what makes
    // `FailDeadOrder` safe there and unsafe everywhere else. See
    // `recovery_step`'s precondition.
    if state == ChargeState::Submitting {
        let evidence = submit_evidence(pool, &charge.id).await?;
        let action = recovery_step(
            flow,
            evidence,
            payload.not_found_streak,
            payload.first_not_found_at,
            now,
            policy,
        );
        match action {
            RecoveryAction::FailDeadOrder => return fail_dead_order(pool, job, &charge).await,
            RecoveryAction::Resubmit => {
                return resubmit_then_escalate_if_late(
                    pool,
                    job,
                    &charge,
                    state,
                    past_horizon,
                    &payload,
                )
                .await;
            }
            RecoveryAction::Advance(status_code) => {
                // Kill point 3 of `docs/flows/crash-safety.md`: the rail
                // answered and the state update was lost. Catch the
                // bookkeeping up, then poll as normal.
                if vpay_db::settlement::set_live_state(
                    pool,
                    &charge.id,
                    ChargeState::Submitting.as_wire_str(),
                    ChargeState::Submitted.as_wire_str(),
                )
                .await?
                {
                    state = ChargeState::Submitted;
                    tracing::info!(
                        job_id = %job.id,
                        charge_id = %charge.id,
                        status_code,
                        "recovered a charge whose submit was answered but never recorded"
                    );
                }
            }
            RecoveryAction::Poll => {}
        }
    }

    let status = match query_status(pool, adapter, config, job, &charge).await {
        Ok(status) => status,
        // A rail that will not answer must not be able to keep a charge off
        // the escalation. `ProviderError::Unavailable` is only
        // `Severity::Warn`, so a status endpoint answering `503` on every
        // rung rode the ladder quietly past the horizon forever: no
        // `unresolved`, no alert, nobody reconciling a charge a payer may
        // have paid. The rail's error is logged rather than returned, because
        // past the horizon the fact an operator needs is "this charge is
        // unreconciled after 24 hours", not "the last poll got a 503" — and
        // `Exhausted` is the error that says so, hourly and with an alert.
        //
        // **Only `Provider`.** This arm is the one place in the worker where a
        // composite replaces a leaf's classification with its own, and
        // ADR-0011 permits it here for one reason: `Exhausted` says something
        // *truer* about a rail that will not answer a day-old charge than the
        // rail's own transient error does. Nothing else it wraps is like that.
        // Written as a wildcard, the arm also swallowed `Poisoned` — a row
        // this build cannot interpret, `Retry::Never`, a bug — and
        // re-published it as `Category::Rail`, retried hourly with an alert,
        // forever, on work no retry can complete; and it swallowed `Db`, whose
        // own retry policy exists so the worker and the API cannot disagree
        // about whether Postgres is transient. Both now propagate untouched.
        Err(JobError::Provider(error)) if past_horizon => {
            tracing::warn!(
                job_id = %job.id,
                charge_id = %charge.id,
                error = %error,
                "the rail did not answer about a charge that is already past the horizon"
            );
            return escalate_to_unresolved(pool, job, &charge, state).await;
        }
        Err(error) => return Err(error),
    };
    let kind = status_kind(&status);

    let Some(settlement) = settle(kind, state) else {
        // `settle` answers `None` only for a terminal charge, and the guard
        // at the top of this function has already returned for one — so this
        // arm is unreachable today, and is written out rather than removed
        // because the alternative is an `unwrap`-shaped access on a fact two
        // separate `match`es agree on rather than the compiler.
        //
        // The contradiction check is here for the same reason it is in
        // `report_late_answer`, which is where a rail that reverses a settled
        // charge is *actually* caught: if this arm ever becomes reachable —
        // by the terminal guard moving, or by `settle` gaining a terminal
        // answer — a rail saying the money went the other way from what the
        // merchant was told must not become a silent `Done`.
        if contradiction(kind, state) {
            log_contradiction(job, &charge.id, state, kind);
        }
        return Ok(Outcome::Done);
    };

    match settlement {
        Settlement::Succeeded => {
            let provider_txn_id = match &status {
                ChargeStatus::Succeeded { provider_txn_id } => provider_txn_id.as_deref(),
                // Unreachable: `settle` only answers `Succeeded` for
                // `StatusKind::Succeeded`. Written as a `match` rather than an
                // `unwrap`-shaped access because the two types are joined by a
                // convention, not by the compiler.
                _ => None,
            };
            settle_succeeded(pool, job, &charge, provider_txn_id).await
        }
        Settlement::Failed(code) => settle_failed(pool, job, &charge, code, &status).await,
        Settlement::Live(next) => {
            vpay_db::settlement::set_live_state(
                pool,
                &charge.id,
                state.as_wire_str(),
                next.as_wire_str(),
            )
            .await?;
            keep_polling(pool, job, &charge, next, past_horizon, &payload.reset()).await
        }
        Settlement::Stay => {
            let next_payload = if kind == StatusKind::NotFound {
                payload.saw_not_found(now)
            } else {
                payload.reset()
            };
            keep_polling(pool, job, &charge, state, past_horizon, &next_payload).await
        }
        Settlement::Recover => {
            recover(
                pool,
                job,
                &charge,
                state,
                flow,
                policy,
                now,
                past_horizon,
                &payload,
            )
            .await
        }
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
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    flow: vpay_core::ProviderFlow,
    policy: &RecoveryPolicy,
    now: OffsetDateTime,
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
        return keep_polling(pool, job, charge, state, past_horizon, &next_payload).await;
    }

    let evidence = submit_evidence(pool, &charge.id).await?;
    match recovery_step(
        flow,
        evidence,
        next_payload.not_found_streak,
        next_payload.first_not_found_at,
        now,
        policy,
    ) {
        RecoveryAction::FailDeadOrder => fail_dead_order(pool, job, charge).await,
        RecoveryAction::Resubmit => {
            resubmit_then_escalate_if_late(pool, job, charge, state, past_horizon, &next_payload)
                .await
        }
        // `Advance` cannot follow a `NotFound` on an answered submit for a
        // charge still in `submitting` without the state having moved under
        // us; polling again is the harmless answer either way.
        RecoveryAction::Poll | RecoveryAction::Advance(_) => {
            keep_polling(pool, job, charge, state, past_horizon, &next_payload).await
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
    pool: &PgPool,
    adapters: &Adapters,
    rails: &RailConfigs,
    job: &vpay_db::JobRow,
) -> Result<Outcome, JobError> {
    let payload: ResubmitPayload = decode(job)?;
    let charge = load_charge(pool, job, &payload.charge_id).await?;
    let state = parse_state(job, &charge)?;
    if state != ChargeState::Submitting {
        // Something already resolved the ambiguity — a concurrent poll, or a
        // previous run of this job that committed and then lost its lease.
        return Ok(Outcome::Done);
    }

    let (adapter, config) = rail(rails, adapters, job, &charge.provider_code)?;

    // `attempt` is supplied rather than derived by the database
    // (`provider_requests::insert_pending`'s own reasoning): the ladder knows
    // how many times it has tried, and a `SELECT max(attempt) + 1` would race
    // two retries into the same number.
    let next_attempt = vpay_db::settlement::latest_submit_attempt(pool, &charge.id)
        .await?
        .map_or(1, |row| row.attempt.saturating_add(1));
    let attempt_id = vpay_db::provider_requests::insert_pending(
        pool,
        &charge.id,
        &charge.provider_code,
        "submit",
        charge.provider_reference_id,
        next_attempt,
    )
    .await?;

    let charge_ref = charge_ref(job, &charge)?;
    let submitted = match adapter.submit(&charge_ref, config).await {
        Ok(submitted) => {
            record_answer(pool, attempt_id, None).await?;
            submitted
        }
        Err(error) => {
            record_failure(pool, attempt_id, &error).await?;
            return Err(JobError::Provider(error));
        }
    };

    let ref_extra = serde_json::Value::Object(
        submitted
            .ref_extra
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect(),
    );

    let mut tx = pool.begin().await.map_err(DbError::Query)?;
    vpay_db::charges::mark_submitted(
        &mut tx,
        &charge.id,
        ChargeState::Submitted.as_wire_str(),
        Some(&ref_extra),
        submitted.redirect_url.as_deref(),
    )
    .await?;
    // In the same transaction as the state move, for the reason
    // `enqueue_in_tx` exists at all: a charge that reached `submitted` with no
    // job behind it is a charge nothing will ever drive to terminal. Normally
    // the poll job already exists and this writes nothing (`Ok(false)`).
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        JobKind::PollCharge.as_wire_str(),
        &poll_dedupe_key(&charge.id),
        &poll_payload(job, &charge.id)?,
        OffsetDateTime::now_utc(),
    )
    .await?;
    tx.commit().await.map_err(DbError::Query)?;

    tracing::info!(
        job_id = %job.id,
        charge_id = %charge.id,
        provider_reference_id = %charge.provider_reference_id,
        attempt = next_attempt,
        "resubmitted a charge under its existing reference"
    );
    Ok(Outcome::Done)
}

/// Deletes what has expired: idempotency records, client-assertion `jti`s, and
/// job leases whose worker died.
///
/// All three were previously run once at `vpay-server` boot, which meant a
/// process that stayed up for a month never swept anything
/// (`docs/status.md`). Three independent statements, each its own transaction:
/// they share nothing, and one failing should not roll back the other two's
/// work. Always reschedules — a sweep that found nothing is the healthy case.
async fn sweep_expired(pool: &PgPool, policy: &RecoveryPolicy) -> Result<Outcome, JobError> {
    let idempotency = vpay_db::idempotency::sweep_expired(pool).await?;
    let assertions = vpay_db::delete_expired_client_assertion_jtis(pool).await?;
    // Lease expiry is a separate reaper rather than a condition on `claim`,
    // so `jobs_claimable_idx`'s `locked_at IS NULL` predicate stays exact.
    // Not the *only* reaper, and it must not be: `crate::run_loop` reaps at
    // boot and on its own half-lease timer, because this job is itself a row
    // in `jobs` and a worker that died holding it would leave the sweep — and
    // therefore the reaping — unclaimable forever. Reaping here as well costs
    // one statement an hour and keeps the sweep's own description honest.
    let leases = vpay_db::jobs::reap_expired_leases(pool, policy.lease).await?;

    tracing::info!(
        idempotency_keys = idempotency,
        client_assertion_jtis = assertions,
        expired_leases = leases,
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
async fn scan_live_charges(pool: &PgPool, job: &vpay_db::JobRow) -> Result<Outcome, JobError> {
    let now = OffsetDateTime::now_utc();
    let cutoff = now - stale_after();
    let charge_ids =
        vpay_db::settlement::live_charges_stale_since(pool, cutoff, SCAN_BATCH).await?;

    let mut enqueued = 0_usize;
    if !charge_ids.is_empty() {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        for charge_id in &charge_ids {
            let inserted = vpay_db::jobs::enqueue_in_tx(
                &mut tx,
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
        tx.commit().await.map_err(DbError::Query)?;
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
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    // Recorded before the escalation, so what this poll learned survives even
    // when the answer is an escalation rather than a reschedule.
    remember(pool, job, payload).await?;

    if past_horizon {
        return escalate_to_unresolved(pool, job, charge, state).await;
    }

    Ok(Outcome::RescheduleAfter(crate::poll_delay(attempt_index(
        job,
    ))))
}

/// Has this charge been live longer than `docs/flows/reconciler.md`'s
/// 24-hour horizon?
///
/// Measured from `charges.created_at`, which is written *before* the rail is
/// called, by construction (`docs/flows/crash-safety.md`) — so it is the age
/// of the payer's exposure and not of our bookkeeping. A `Duration` that does
/// not fit `time::Duration` saturates to `MAX`, i.e. "never", which is the
/// safe direction: an unrepresentable horizon must not escalate every charge
/// at once.
///
/// Called from exactly one place, [`poll_charge`], which evaluates it before
/// the crash-recovery block and carries the answer down to [`keep_polling`]
/// and [`resubmit_then_escalate_if_late`] — the two frames that can conclude
/// "this charge is still live and still unanswered". It is not a gate on
/// asking the rail: past the horizon the charge is polled hourly rather than
/// not at all, and what escalates is every outcome short of a settlement.
fn past_the_horizon(charge: &ChargeRow, policy: &RecoveryPolicy, now: OffsetDateTime) -> bool {
    let horizon = time::Duration::try_from(policy.unresolved_after).unwrap_or(time::Duration::MAX);
    now - charge.created_at >= horizon
}

/// Marks a charge `unresolved` and fails the job with [`JobError::Exhausted`].
///
/// **Never returns `Ok`.** The `Result<Outcome, _>` return type is so callers
/// can `return escalate_to_unresolved(…).await;` from a function that
/// otherwise produces outcomes, and so the `set_live_state` write can
/// propagate a real [`DbError`] with `?` instead of being swallowed into the
/// exhaustion. A database failure here is a database failure, not an
/// escalation, and it must be retried as one.
///
/// The charge stays **live**: `unresolved` is an escalation, not a verdict.
/// "A late success — minute 40, or hour 30 from `unresolved` — is the normal
/// transition" (`docs/flows/reconciler.md`), which is why the error is
/// `Exhausted` (hourly retry + alert) rather than anything that parks the job.
///
/// Once a charge is `unresolved` every later hourly run *does* ask the rail
/// before arriving back here, so the late success has something to arrive
/// through — the crash-recovery block that can return earlier is entered only
/// from `submitting`, which this function has just left. The run that
/// escalates is the one exception, and it has two shapes: it either asked the
/// rail and got an answer short of terminal (or no answer at all), or it came
/// from [`resubmit_then_escalate_if_late`], which is reached without a query
/// because the recovery table concluded from `provider_requests` alone.
///
/// The state write is skipped when the charge is already `unresolved`, which
/// is what makes the hourly re-escalation idempotent: the alert repeats, the
/// row does not move, and `charges.updated_at` keeps naming the last time
/// anything actually changed. Proven by
/// `a_second_hourly_poll_of_an_unresolved_charge_re_alerts_without_writing_it_again`
/// in `backends/tests/integration/tests/worker_recovery.rs`, which asserts the
/// timestamp *and* the alert — a no-op that also stopped alerting would
/// satisfy half of this sentence.
async fn escalate_to_unresolved(
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
) -> Result<Outcome, JobError> {
    if state != ChargeState::Unresolved {
        vpay_db::settlement::set_live_state(
            pool,
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
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
) -> Result<Outcome, JobError> {
    const RAW: &str = "the rail's submit response was lost before its token could be \
                       committed; the payer was never handed a redirect URL, so this \
                       order can never be queried and no payment can have occurred \
                       (docs/flows/crash-safety.md)";
    settle_failed_with(
        pool,
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
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    provider_txn_id: Option<&str>,
) -> Result<Outcome, JobError> {
    let data = intent_snapshot(
        pool,
        job,
        &charge.payment_intent_id,
        IntentStatus::Succeeded,
        None,
    )
    .await?;
    let settled = vpay_db::settlement::apply_succeeded(
        pool,
        &charge.id,
        provider_txn_id,
        &ids::event_id(),
        &data,
    )
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
        None => report_late_answer(pool, job, charge, StatusKind::Succeeded).await,
    }
    Ok(Outcome::Done)
}

/// Commits a rail-reported decline.
async fn settle_failed(
    pool: &PgPool,
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
        pool,
        job,
        charge,
        code,
        raw,
        "the rail reported a charge as failed",
    )
    .await
}

async fn settle_failed_with(
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    code: FailureCode,
    raw: &str,
    log_message: &'static str,
) -> Result<Outcome, JobError> {
    let message = merchant_message(code, raw);
    let data = intent_snapshot(
        pool,
        job,
        &charge.payment_intent_id,
        IntentStatus::RequiresPaymentMethod,
        Some((code, &message)),
    )
    .await?;
    let settled = vpay_db::settlement::apply_failed(
        pool,
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
        None => report_late_answer(pool, job, charge, StatusKind::Failed(code)).await,
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
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    kind: StatusKind,
) {
    let stored = match vpay_db::charges::get_by_id(pool, &charge.id).await {
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
/// know about the horizon. Neither did the right thing at 24 hours — a
/// `submitting` charge whose rail answers `404` forever cycled resubmit →
/// ladder → resubmit, never `unresolved` and never alerting, which is the one
/// outcome `docs/flows/reconciler.md` rules out for a charge that has been
/// live for a day.
///
/// # What "escalate on top of it" is worth, exactly
///
/// The resubmit row is committed first and the escalation second, in two
/// transactions, and the escalation moves the charge to `unresolved`. So the
/// resubmit job usually finds the charge outside `submitting` and returns
/// `Outcome::Done` without calling the rail ([`resubmit_charge`]'s own guard):
/// past the horizon the escalation ordinarily *supersedes* the resubmit rather
/// than running alongside it. A concurrent worker that claims the resubmit
/// between the two commits does send it, under the charge's existing
/// reference. Both orders are safe — the reference never changes, and
/// [`escalate_to_unresolved`] is idempotent — but this is a real
/// non-determinism and not a detail: what the horizon guarantees here is the
/// alert and the hourly poll, not that a 25-hour-old charge is resent. That is
/// deliberate. Once a human is reconciling a charge against the rail's
/// settlement statement, whether to push another submission at it is their
/// call, not a queue's.
async fn resubmit_then_escalate_if_late(
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    state: ChargeState,
    past_horizon: bool,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    let rescheduled = schedule_resubmit(pool, job, charge, payload).await?;
    if past_horizon {
        return escalate_to_unresolved(pool, job, charge, state).await;
    }
    Ok(rescheduled)
}

/// Enqueues a resubmit and puts the ladder back on the clock.
///
/// The poll job is rescheduled rather than finished, so the resubmit's result
/// is polled by the job that was already tracking this charge instead of a
/// fresh ladder starting at rung zero.
async fn schedule_resubmit(
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
    payload: &PollChargePayload,
) -> Result<Outcome, JobError> {
    let mut tx = pool.begin().await.map_err(DbError::Query)?;
    let enqueued = vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        JobKind::ResubmitCharge.as_wire_str(),
        &resubmit_dedupe_key(&charge.id),
        &encode(job, &ResubmitPayload::new(charge.id.clone()))?,
        OffsetDateTime::now_utc(),
    )
    .await?;
    tx.commit().await.map_err(DbError::Query)?;

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
    remember(pool, job, &payload.reset()).await?;
    Ok(Outcome::RescheduleAfter(crate::poll_delay(attempt_index(
        job,
    ))))
}

/// The one authenticated status read, wrapped in the attempt row that makes
/// it auditable.
async fn query_status(
    pool: &PgPool,
    adapter: &dyn ProviderAdapter,
    config: &ProviderConfig,
    job: &vpay_db::JobRow,
    charge: &ChargeRow,
) -> Result<ChargeStatus, JobError> {
    let charge_ref = charge_ref(job, charge)?;
    let attempt_id = vpay_db::provider_requests::insert_pending(
        pool,
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
            record_answer(pool, attempt_id, Some(status_label(&status))).await?;
            Ok(status)
        }
        Err(error) => {
            record_failure(pool, attempt_id, &error).await?;
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
    pool: &PgPool,
    attempt_id: i64,
    label: Option<&'static str>,
) -> Result<(), JobError> {
    vpay_db::provider_requests::record_response(
        pool,
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
    pool: &PgPool,
    attempt_id: i64,
    error: &ProviderError,
) -> Result<(), JobError> {
    vpay_db::provider_requests::record_response(pool, attempt_id, None, Some(error.code())).await?;
    Ok(())
}

/// The rail-facing view of a charge.
///
/// `ref_extra` is read back out of the row rather than rebuilt, because on a
/// redirect rail it carries the `pay_token` without which the rail will not
/// answer at all.
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
async fn submit_evidence(pool: &PgPool, charge_id: &str) -> Result<SubmitAttempt, JobError> {
    let latest = vpay_db::settlement::latest_submit_attempt(pool, charge_id).await?;
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
/// *snapshot of the object* (migration 0018) and Step 5 delivers it verbatim
/// to a merchant's Stripe-shaped handler. A second, hand-written copy of that
/// shape here is exactly how the webhook body and the API response start
/// disagreeing about a field.
///
/// The projection is applied to the row **before** the write, because
/// `apply_succeeded`/`apply_failed` take `event_data` as an input — the event
/// is written inside the same transaction as the row it describes, so it
/// cannot be rendered from the result. The two fields patched here are exactly
/// the ones those functions change and that the object renders: `status`, and
/// the `last_payment_error` pair. `amount_received` is *not* patched because
/// the object does not carry it.
async fn intent_snapshot(
    pool: &PgPool,
    job: &vpay_db::JobRow,
    payment_intent_id: &str,
    status: IntentStatus,
    error: Option<(FailureCode, &str)>,
) -> Result<serde_json::Value, JobError> {
    let mut row = vpay_db::payment_intents::get_by_id(pool, payment_intent_id)
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
    pool: &PgPool,
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
    vpay_db::jobs::set_payload(pool, job.id, worker_id, &encoded).await?;
    Ok(())
}

/// A fresh poll payload for one charge, as JSONB.
///
/// The enqueue itself is written out at each call site rather than wrapped in
/// a helper, because a helper would have to name `sqlx::PgConnection` in its
/// signature and this crate deliberately does not depend on `sqlx` — every
/// statement belongs to `vpay-db`.
fn poll_payload(job: &vpay_db::JobRow, charge_id: &str) -> Result<serde_json::Value, JobError> {
    encode(job, &PollChargePayload::new(charge_id))
}

/// Loads the charge a job names, or says the row is poisoned.
async fn load_charge(
    pool: &PgPool,
    job: &vpay_db::JobRow,
    charge_id: &str,
) -> Result<ChargeRow, JobError> {
    vpay_db::charges::get_by_id(pool, charge_id)
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

/// Logs a job failure at the level its classification implies.
///
/// The four arms mirror [`crate::tracing_level`] exactly and exist separately
/// only because `alert = true` is an event *field*, which has to be written at
/// the macro call site — that function's own doc comment explains why it
/// cannot attach it for the caller. `alert` is set for [`Severity::Page`] and
/// nothing else, so an alerting rule can select pages without also firing on
/// every rail timeout.
fn log_failure(job: &vpay_db::JobRow, error: &JobError) {
    let severity = error.severity();
    let code = error.code();
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
    /// This and the skew case below are the only unit tests in this file, and
    /// they do not contradict the module header: `past_the_horizon` is a pure
    /// function of a timestamp and a policy, not one of the write sequences
    /// that header is about.
    #[test]
    fn the_horizon_is_twenty_four_hours_of_real_time() {
        let policy = RecoveryPolicy::default();
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(365);
        let aged = |d: time::Duration| past_the_horizon(&charge_created(now - d), &policy, now);

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
    /// `now - created_at` is signed, so a row written by a replica whose
    /// clock is ahead produces a negative age. It must read as "young", not
    /// wrap into an escalation: `time::Duration` is signed and the comparison
    /// is against a positive horizon, which is what makes that hold.
    #[test]
    fn a_clock_skewed_charge_is_not_escalated() {
        let policy = RecoveryPolicy::default();
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(365);
        assert!(!past_the_horizon(
            &charge_created(now + time::Duration::hours(1)),
            &policy,
            now
        ));
    }
}
