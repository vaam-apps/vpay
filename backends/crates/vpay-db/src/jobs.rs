//! The `jobs` repository (`backends/migrations/0021_create-jobs.sql`) — the
//! worker's durable queue.
//!
//! # The lease is the whole design
//!
//! A job is *claimed* by an `UPDATE` that stamps `locked_at`/`locked_by` on
//! exactly one runnable row, and it is only ever finished or rescheduled by a
//! statement that also names the same `locked_by`. That guard is not
//! decoration: without it, a worker whose lease was reaped mid-run (it hung,
//! the reaper freed the row, another worker picked it up) would `DELETE` a
//! job the second worker is in the middle of executing, or reschedule it out
//! from under them. This is ABA, and `idempotency::claim` closes the same
//! hole the same way with its `claim_id` — see that module's own comment.
//!
//! # Why claiming does not consider lease expiry
//!
//! [`claim`]'s predicate is `locked_at IS NULL`, full stop, so it matches
//! `jobs_claimable_idx` exactly. "Unlocked *or* the lease has expired"
//! depends on `now()` and cannot be an index predicate, so it would turn
//! every claim into a scan over every leased row. Expiry is therefore a
//! separate, periodic pass — [`reap_expired_leases`] — which frees a stale
//! lease *once* and lets the ordinary claim path pick the row up on its next
//! turn.
//!
//! Its callers are `vpay_worker::run_loop`, which reaps once at boot (a
//! worker that has just restarted after a crash cannot wait) and then on its
//! own timer at half a lease, and the hourly `sweep_expired` job. Deliberately
//! not the sweep alone: the sweep is itself a row in this table, so a worker
//! that died holding it would leave the only reaper unclaimable — the one
//! stranded lease nothing could ever free.
//!
//! # Why a dead letter is parked and not deleted, and why with no new column
//!
//! A job that is done is deleted ([`finish`]); a job that is not done is
//! rescheduled with its error recorded ([`reschedule`]). A job that *cannot*
//! be done — `vpay_worker::JobError::Poisoned`, or anything else
//! `Classify::retry` answers `Retry::Never` for — is neither, and
//! [`dead_letter`] is the third write.
//!
//! It exists because deleting one is not safe for a *payment* queue.
//! `poll_charge` is the only thing driving a live charge to a terminal
//! state; delete its row and the charge is unattended, with nothing in the
//! database saying why. The backstop scan would then re-enqueue the same
//! `dedupe_key` at its next pass and the same failure would repeat every ten
//! minutes, forever, with a fresh `attempts = 1` each time — a hot loop that
//! reads as a flapping rail rather than as a permanently broken row.
//!
//! Parking is `run_at = 'infinity'` (a real `timestamptz` value, not a
//! sentinel year) with the lease cleared. That single write is all four
//! properties at once: [`claim`]'s `run_at <= now()` can never match it,
//! [`reap_expired_leases`]' `locked_at` predicate can never resurrect it, the
//! `dedupe_key` stays occupied so no scan or callback re-creates the work,
//! and `last_error` keeps the reason where the operator handling the page is
//! already looking. A `dead_lettered_at` column would carry no fact these do
//! not, and every reader of the table would have to learn to exclude it.
//!
//! The cost, stated plainly: a parked job is invisible to
//! [`oldest_runnable_run_at`] and to every other `run_at`-ordered query, so
//! the *only* way an operator learns one exists is the alert the loop raises
//! when it parks it, and `SELECT * FROM jobs WHERE run_at = 'infinity'`.
//! Requeuing one is an `UPDATE jobs SET run_at = now()` by hand, which is
//! deliberate: it should follow a human deciding the underlying data is
//! fixed.

use std::time::Duration;

use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{DbError, classify_write};

/// The `last_error` ceiling from the `last_error_length` CHECK
/// (migration 0021), in characters.
///
/// [`reschedule`] truncates to it rather than letting the database refuse
/// the write: the whole point of that write is to record *why* a job did not
/// finish, and a rail whose error text runs long would otherwise turn a
/// recorded failure into an unrecorded one — the job would stay leased until
/// the reaper freed it, with nothing anywhere saying what happened.
const LAST_ERROR_MAX_CHARS: usize = 2000;

/// The columns [`claim`] returns, spelled once so the statement and
/// [`JobRow`] cannot drift.
///
/// `locked_at` and `created_at` are selected but are not fields of
/// [`JobRow`]: `sqlx`'s derived `FromRow` reads the fields it has by name and
/// ignores the rest. They are in the `RETURNING` list because an operator
/// reading a slow-query log or `pg_stat_statements` should see the whole row
/// the claim produced, and because a future caller that needs the lease
/// instant should not have to change the statement to get it.
const CLAIM_RETURNING: &str = "id, kind, dedupe_key, payload, run_at, attempts, locked_at, \
                               locked_by, last_error, created_at";

/// One `jobs` row as a worker sees it.
///
/// `locked_by` is `Some` for every row [`claim`] returns — it is the id this
/// worker must present to [`finish`] or [`reschedule`] the job — and the
/// `Option` exists because the column is nullable for unclaimed rows, not
/// because a claimed job might lack one (`lock_is_paired` forbids that).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct JobRow {
    /// Database-generated identity. Unlike every other object in this
    /// schema, a job has no `job_…` public id: nothing outside vpay ever
    /// names one, so there is nothing for a readable prefix to help with.
    pub id: Uuid,
    /// One of the four labels `kind_is_known` allows. Carried as `String`
    /// for the same reason the Postgres enums are (D4): the closed
    /// vocabulary lives above this crate.
    pub kind: String,
    /// The idempotency key of the *work*, e.g. `poll:ch_…`.
    pub dedupe_key: String,
    /// Handler input, always a JSON object (`payload_is_object`).
    pub payload: serde_json::Value,
    /// When this job became (or becomes) claimable.
    pub run_at: OffsetDateTime,
    /// How many times this job has been claimed, *including* the claim that
    /// produced this row — the counter is incremented by the claim itself,
    /// so a job that kills its worker before it can reschedule still counts
    /// up and cannot spin forever at zero.
    pub attempts: i32,
    /// The worker holding the lease. Every write that ends the lease must
    /// present this value.
    pub locked_by: Option<String>,
    /// Why the previous attempt did not finish, truncated to the column's
    /// 2000 characters by [`reschedule`].
    pub last_error: Option<String>,
}

/// Enqueues a job **inside the caller's transaction**, returning whether a
/// row was actually inserted.
///
/// # Why this only exists in the transactional form
///
/// The queue's one hard requirement is that the job and the write that
/// creates the work commit together. `confirm` opens its charge row before
/// calling the rail (`docs/flows/crash-safety.md`); enqueueing the poll in
/// that same transaction is what makes *all three* of that document's kill
/// points leave a job behind. A pooled `enqueue(pool, …)` would let a caller
/// write the job on a second connection that commits independently, which
/// reintroduces both halves of the failure it exists to prevent — a job for
/// a charge that rolled back, and a committed charge with nothing to drive
/// it. So there is no such function.
///
/// `Ok(false)` means the `dedupe_key` was already queued and this call
/// changed nothing. That is the normal, expected answer for the backstop
/// scan and for a re-enqueue after a crash, **not** an error: `dedupe_key`
/// names the work, so a second row would be a second worker doing the same
/// thing at the same time.
///
/// Deliberately *not* an upsert. `DO UPDATE SET run_at = …` would let a
/// backstop scan drag a job that is already scheduled for an hour's time
/// back to now, which is how a poll ladder silently becomes a hot loop.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails — including a `kind` outside
/// `kind_is_known` or a `payload` that is not a JSON object, both of which
/// are vpay bugs rather than anything a caller of the API can cause.
pub async fn enqueue_in_tx(
    tx: &mut PgConnection,
    kind: &str,
    dedupe_key: &str,
    payload: &serde_json::Value,
    run_at: OffsetDateTime,
) -> Result<bool, DbError> {
    let inserted = sqlx::query(
        "INSERT INTO jobs (kind, dedupe_key, payload, run_at) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (dedupe_key) DO NOTHING",
    )
    .bind(kind)
    .bind(dedupe_key)
    .bind(payload)
    .bind(run_at)
    .execute(&mut *tx)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(inserted == 1)
}

/// Takes a lease on the single oldest runnable job, or returns `None` if
/// there is none.
///
/// # Why the subquery, and why `FOR UPDATE SKIP LOCKED`
///
/// This is the one statement that makes N concurrent workers safe. The inner
/// `SELECT … FOR UPDATE SKIP LOCKED LIMIT 1` locks one candidate row and
/// *skips* rows another transaction has already locked, so two workers
/// claiming at the same instant take two different jobs instead of both
/// picking the same one and one of them failing (or, worse, both proceeding).
/// A plain `UPDATE … WHERE locked_at IS NULL … LIMIT`-shaped statement has no
/// such guarantee: under `READ COMMITTED` the second writer blocks on the row
/// lock, re-evaluates its predicate, finds the row claimed and matches
/// nothing — turning a claim into a silent miss while a queue full of work
/// waits. `SKIP LOCKED` is what turns that into "take the next one".
///
/// `attempts` is incremented by this statement rather than by whatever
/// finishes the job, so a job that panics or kills its worker before it can
/// report anything still counts up.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the claim fails.
pub async fn claim(pool: &PgPool, worker_id: &str) -> Result<Option<JobRow>, DbError> {
    let sql = format!(
        "UPDATE jobs SET locked_at = now(), locked_by = $1, attempts = attempts + 1 \
         WHERE id = (SELECT id FROM jobs WHERE run_at <= now() AND locked_at IS NULL \
                     ORDER BY run_at FOR UPDATE SKIP LOCKED LIMIT 1) \
         RETURNING {CLAIM_RETURNING}"
    );

    sqlx::query_as::<_, JobRow>(&sql)
        .bind(worker_id)
        .fetch_optional(pool)
        .await
        .map_err(DbError::Query)
}

/// Deletes a finished job, but only if `worker_id` still holds its lease.
///
/// `Ok(false)` means this worker no longer holds the job — its lease was
/// reaped as stale and someone else has it, or the row is already gone. The
/// caller must **not** treat that as a failure of the work (the work is
/// done; that is why it is calling this), but it must not treat it as
/// success either: another worker is now running the same job, which is the
/// signal that the lease is too short for this handler. Hence a `bool` and
/// not `()`.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the delete fails.
pub async fn finish(pool: &PgPool, id: Uuid, worker_id: &str) -> Result<bool, DbError> {
    let deleted = sqlx::query("DELETE FROM jobs WHERE id = $1 AND locked_by = $2")
        .bind(id)
        .bind(worker_id)
        .execute(pool)
        .await
        .map_err(DbError::Query)?
        .rows_affected();

    Ok(deleted == 1)
}

/// Releases the lease and moves the job `delay` into the future, recording
/// why.
///
/// This is the poll ladder: every rung is this statement with a different
/// `delay`. Releasing and rescheduling are one write on purpose — a job that
/// was unlocked first and moved second would be claimable, at its *old*
/// `run_at`, for the width of that gap, which is how a ladder collapses into
/// a spin.
///
/// `last_error` is truncated to the column's 2000 characters
/// (`LAST_ERROR_MAX_CHARS`) rather than left to the CHECK: see that
/// constant for why a refused write here loses the very information the
/// write exists to record.
///
/// `Ok(false)` has the same meaning as in [`finish`], and matters more: the
/// job is still leased by whoever holds it now, and this worker's answer has
/// been discarded.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub async fn reschedule(
    pool: &PgPool,
    id: Uuid,
    worker_id: &str,
    delay: Duration,
    last_error: Option<&str>,
) -> Result<bool, DbError> {
    let bounded: Option<String> = last_error.map(|error| {
        // `char_length` in the CHECK counts characters, so this counts
        // characters too — truncating by bytes could also split a multi-byte
        // character, which is a panic in `String` and a mojibake in the log.
        error.chars().take(LAST_ERROR_MAX_CHARS).collect()
    });

    let updated = sqlx::query(
        "UPDATE jobs \
         SET run_at = now() + ($3::BIGINT * INTERVAL '1 microsecond'), \
             locked_at = NULL, \
             locked_by = NULL, \
             last_error = $4 \
         WHERE id = $1 AND locked_by = $2",
    )
    .bind(id)
    .bind(worker_id)
    .bind(as_micros(delay))
    .bind(bounded.as_deref())
    .execute(pool)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(updated == 1)
}

/// Replaces a leased job's `payload`, without touching its schedule.
///
/// # Why the payload is a separate write from [`reschedule`]
///
/// The recovery table keeps per-job state in the payload — the
/// `not_found_streak` and `first_not_found_at` that decide when a charge the
/// rail claims never to have seen is resubmitted
/// (`docs/flows/crash-safety.md`). That state has to survive the *current*
/// attempt even when the job is not being rescheduled at all (it is being
/// finished, or it is about to fail), so it cannot ride along on the
/// rescheduling statement.
///
/// The two writes are therefore not atomic with each other, and that is
/// deliberate rather than overlooked: the worst a crash between them can do
/// is lose one increment of a counter whose only effect is *when* a resubmit
/// happens. Making them one statement would mean either a `reschedule` that
/// silently rewrites a payload its caller did not mean to touch, or a
/// payload update that cannot happen without also moving the schedule.
/// Neither trade is worth the atomicity of a retry heuristic.
///
/// Guarded on `locked_by` like every other write that follows a claim:
/// `Ok(false)` means this worker no longer holds the job and its bookkeeping
/// is being discarded along with the rest of its answer.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails, including a payload that is not a
/// JSON object (`payload_is_object`).
pub async fn set_payload(
    pool: &PgPool,
    id: Uuid,
    worker_id: &str,
    payload: &serde_json::Value,
) -> Result<bool, DbError> {
    let updated = sqlx::query("UPDATE jobs SET payload = $3 WHERE id = $1 AND locked_by = $2")
        .bind(id)
        .bind(worker_id)
        .bind(payload)
        .execute(pool)
        .await
        .map_err(classify_write)?
        .rows_affected();

    Ok(updated == 1)
}

/// Releases every lease held by `worker_id`, returning how many.
///
/// The drain path: a worker that is shutting down and could not finish in
/// its grace period hands its jobs back rather than leaving them leased
/// until the reaper notices, which would delay every one of them by the
/// lease interval for no reason. `last_error` is deliberately left alone —
/// the previous attempt's error is still the last thing that went wrong, and
/// "this worker exited" is a fact about the process, recorded in its log,
/// not about the job.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub async fn release_all(pool: &PgPool, worker_id: &str) -> Result<u64, DbError> {
    let released =
        sqlx::query("UPDATE jobs SET locked_at = NULL, locked_by = NULL WHERE locked_by = $1")
            .bind(worker_id)
            .execute(pool)
            .await
            .map_err(DbError::Query)?
            .rows_affected();

    Ok(released)
}

/// Frees every lease older than `lease`, returning how many.
///
/// This is the counterpart to [`claim`]'s exact-index predicate (see the
/// module comment): a worker that died holding a job leaves a row nothing
/// else can ever claim, and this is the only thing that recovers it. Called
/// by `vpay_worker::run_loop` at boot and every half-lease, and by the hourly
/// `sweep_expired` job — see the module comment for why the job alone is not
/// enough.
///
/// `lease` must be comfortably larger than the longest a handler can
/// legitimately take — the design's default is 5 minutes against a 20-second
/// provider request timeout (`vpay_provider::DEFAULT_REQUEST_TIMEOUT`) —
/// because reaping a lease that is merely *slow* hands the same job to a
/// second worker while the first is still running it.
///
/// The freed rows keep their `run_at`, so they are claimable immediately;
/// their `attempts` was already incremented by the claim that stranded them,
/// which is what stops a job that reliably kills its worker from being
/// retried forever with no trace.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub async fn reap_expired_leases(pool: &PgPool, lease: Duration) -> Result<u64, DbError> {
    let reaped = sqlx::query(
        "UPDATE jobs \
         SET locked_at = NULL, locked_by = NULL, last_error = 'lease expired' \
         WHERE locked_at < now() - ($1::BIGINT * INTERVAL '1 microsecond')",
    )
    .bind(as_micros(lease))
    .execute(pool)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(reaped)
}

/// Parks a job nothing can fix, recording why, and releases its lease.
///
/// The write the module comment describes: `run_at = 'infinity'` and the
/// lease cleared, in one statement. See that comment for why this is a park
/// rather than a `DELETE`, why `'infinity'` rather than a `dead_lettered_at`
/// column, and what it costs.
///
/// `Ok(false)` has the same meaning as in [`finish`] and [`reschedule`]: this
/// worker no longer holds the job, so its verdict — including this one — is
/// being discarded. The job stays claimable by whoever holds it now, which is
/// the right outcome: a second worker that reaches the same conclusion parks
/// it under its own lease.
///
/// `last_error` is truncated to `LAST_ERROR_MAX_CHARS` for exactly the reason
/// [`reschedule`] truncates it, and more sharply: this is the last thing ever
/// written about the row, so a refused write here loses the only record of
/// why a payment stopped being driven.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub async fn dead_letter(
    pool: &PgPool,
    id: Uuid,
    worker_id: &str,
    last_error: &str,
) -> Result<bool, DbError> {
    let bounded: String = last_error.chars().take(LAST_ERROR_MAX_CHARS).collect();

    let updated = sqlx::query(
        "UPDATE jobs \
         SET run_at = 'infinity'::TIMESTAMPTZ, \
             locked_at = NULL, \
             locked_by = NULL, \
             last_error = $3 \
         WHERE id = $1 AND locked_by = $2",
    )
    .bind(id)
    .bind(worker_id)
    .bind(&bounded)
    .execute(pool)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(updated == 1)
}

/// When the oldest *runnable* job becomes claimable, or `None` if the queue
/// holds none.
///
/// The one number the worker's periodic gauge line cannot count in process:
/// claimed/succeeded/rescheduled are this worker's own tallies, but "how far
/// behind is the queue" is a property of the table and of every worker
/// against it. A value drifting into the past is the backlog signal — the
/// queue has work whose time has come and nobody is taking it.
///
/// `run_at < 'infinity'` excludes parked rows for two reasons. They are not
/// backlog, so counting them would peg the gauge at "infinitely behind" from
/// the first dead letter onwards; and `'infinity'` has no [`OffsetDateTime`]
/// representation, so decoding one would fail the query rather than answer
/// it.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn oldest_runnable_run_at(pool: &PgPool) -> Result<Option<OffsetDateTime>, DbError> {
    sqlx::query_scalar::<_, Option<OffsetDateTime>>(
        "SELECT min(run_at) FROM jobs \
         WHERE locked_at IS NULL AND run_at < 'infinity'::TIMESTAMPTZ",
    )
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)
}

/// A [`Duration`] as whole microseconds, saturating.
///
/// Durations reach Postgres as a microsecond count multiplied by
/// `INTERVAL '1 microsecond'` rather than through `sqlx`'s own
/// `Duration → INTERVAL` encoding, which **refuses** any duration carrying
/// sub-microsecond precision (`sqlx-postgres`'s `TryFrom<Duration> for
/// PgInterval`: "PostgreSQL `INTERVAL` does not support nanoseconds
/// precision"). A backoff computed with jitter is exactly the kind of value
/// that lands on a stray nanosecond, and a scheduling write that fails
/// because of one is a job left leased until the reaper finds it. Rounding
/// down to the microsecond is invisible to a poll ladder measured in
/// seconds; refusing the write is not.
fn as_micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The conversion has to be total for every `Duration` a caller can
    /// construct — including `Duration::MAX`, which does not fit in an `i64`
    /// microsecond count. Saturating there parks the job effectively
    /// forever, which is what the caller asked for; panicking in a payment
    /// process is not (ADR-0007).
    #[test]
    fn a_duration_is_microseconds_and_saturates_rather_than_overflowing() {
        assert_eq!(as_micros(Duration::from_secs(1)), 1_000_000);
        assert_eq!(as_micros(Duration::from_millis(50)), 50_000);
        // Sub-microsecond precision rounds down instead of being refused —
        // the whole reason this function exists rather than binding the
        // `Duration` directly.
        assert_eq!(as_micros(Duration::from_nanos(1_500)), 1);
        assert_eq!(as_micros(Duration::MAX), i64::MAX);
    }
}
