//! The `jobs` repository (`backends/migrations/0021_create-jobs.sql`) — the
//! worker's durable queue.
//!
//! A job is *claimed* by an `UPDATE` that stamps `locked_at`/`locked_by` on one
//! runnable row, and every write that ends that lease names the same
//! `locked_by`. Four writes end one: [`Jobs::finish`] deletes a job that is
//! done, [`Jobs::reschedule`] releases one that is not, [`Jobs::dead_letter`]
//! parks one that *cannot* be done at `run_at = 'infinity'`, and
//! [`Jobs::reap_expired_leases`] frees one whose worker died.
//!
//! `docs/reference/vpay-db.md` §"`jobs`" carries the reasoning: why the
//! `locked_by` guard is ABA protection rather than decoration, why claiming
//! ignores lease expiry, and why a dead letter is parked rather than deleted
//! or given a column of its own.

use std::time::Duration;

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::{AssertSqlSafe, PgConnection};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{DbError, classify_write};

/// The `last_error` ceiling from the `last_error_length` CHECK
/// (migration 0021), in characters.
///
/// [`Jobs::reschedule`] truncates to it rather than letting the database refuse
/// the write: the whole point of that write is to record *why* a job did not
/// finish, and a rail whose error text runs long would otherwise turn a
/// recorded failure into an unrecorded one — the job would stay leased until
/// the reaper freed it, with nothing anywhere saying what happened.
const LAST_ERROR_MAX_CHARS: usize = 2000;

/// The columns [`Jobs::claim`] returns, spelled once so the statement and
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
/// `locked_by` is `Some` for every row [`Jobs::claim`] returns — it is the id this
/// worker must present to [`Jobs::finish`] or [`Jobs::reschedule`] the job — and the
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
    /// 2000 characters by [`Jobs::reschedule`].
    pub last_error: Option<String>,
}

/// Enqueues a job **inside the caller's transaction**, returning whether a
/// row was actually inserted.
///
/// There is deliberately no pooled variant and it is deliberately not an
/// upsert — `docs/reference/vpay-db.md` §"`enqueue_in_tx` exists only in the
/// transactional form" says what each would reintroduce.
///
/// `Ok(false)` means the `dedupe_key` was already queued and this call changed
/// nothing: the normal answer for the backstop scan and for a re-enqueue after
/// a crash, **not** an error.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails — including a `kind` outside
/// `kind_is_known` or a `payload` that is not a JSON object, both of which
/// are vpay bugs rather than anything a caller of the API can cause.
pub(crate) async fn enqueue_in_tx(
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

/// Brings an already-queued job's `run_at` forward to now, **inside the
/// caller's transaction**, returning whether a row moved — unless it is
/// already due within `floor`, in which case it is left where it is.
///
/// The one write that makes a rail callback worth anything. `enqueue_in_tx`
/// is `ON CONFLICT DO NOTHING` and deliberately not an upsert (see
/// `docs/reference/vpay-db.md` §"`enqueue_in_tx` exists only in the
/// transactional form"), so a callback arriving while a poll job sits at
/// `now() + 10s` — the ladder's first rung, `vpay_worker::poll_delay` —
/// changes nothing at all without this. That is the whole difference between
/// "the rail told us and we asked immediately" and "the rail told us and we
/// asked at the next rung anyway".
///
/// # Why this is not `enqueue_in_tx` growing a `DO UPDATE`
///
/// Because the argument against the upsert is still right: the backstop scan
/// (`scan_live_charges`) re-enqueues every live charge's key every ten
/// minutes, and an upserting enqueue would drag a job scheduled for a
/// quarter of an hour's time back to now on every pass — a poll ladder that
/// silently becomes a hot loop against a rail. A caller has to *ask* for the
/// pull-forward, and exactly one does.
///
/// # The three guards, and what each refuses
///
/// * `locked_at IS NULL` — a leased job is being run **right now**, by a
///   worker that will see the rail's answer without any help from here.
///   Moving `run_at` under someone else's lease would also be the one write
///   in this module that ends up outside the `locked_by` discipline the
///   module header describes.
/// * `run_at > now() + floor` — a job that is already claimable, *or due
///   within `floor`*, needs nothing: it is about to run. Skipping the write
///   is what makes a burst of duplicate callbacks (which both rails send)
///   free rather than a row-lock queue on one job, and `floor` is what stops
///   an unauthenticated caller from converting each POST into a rail request
///   for a charge the queue was going to ask about in a moment anyway. See
///   below.
/// * `run_at < 'infinity'` — a **dead letter** is parked precisely so that
///   nothing re-creates the work on a timer; `docs/reference/vpay-db.md`
///   §"Why a dead letter is parked and not deleted" names a callback as one
///   of the things the parked `dedupe_key` must keep out. Un-parking one
///   stays a human's `UPDATE`.
///
/// `Ok(false)` therefore means "nothing to do", never a failure: it is the
/// answer for a job that was just inserted at `now()`, for one due inside
/// `floor`, for one a worker holds, and for one an operator parked.
///
/// # Why `floor` is a parameter and not a constant here
///
/// The number that belongs in it is the *poll ladder's* fastest rung, and
/// the ladder is `vpay_worker::poll_delay` — a rail-facing retry policy this
/// crate has no business knowing (ADR-0002: nothing outside the adapters and
/// the worker's own policy decides how often a rail is asked anything).
/// `vpay-db` is handed a duration and enforces it; the caller
/// (`vpay_api::provider_callback`) is where the value is written down and
/// justified.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails.
pub(crate) async fn pull_forward_in_tx(
    tx: &mut PgConnection,
    dedupe_key: &str,
    floor: Duration,
) -> Result<bool, DbError> {
    let moved = sqlx::query(
        "UPDATE jobs SET run_at = now() \
         WHERE dedupe_key = $1 \
           AND locked_at IS NULL \
           AND run_at > now() + ($2::BIGINT * INTERVAL '1 microsecond') \
           AND run_at < 'infinity'::TIMESTAMPTZ",
    )
    .bind(dedupe_key)
    .bind(as_micros(floor))
    .execute(&mut *tx)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(moved == 1)
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

#[async_trait::async_trait]
pub trait Jobs: Send + Sync {
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
    async fn claim(&self, worker_id: &str) -> Result<Option<JobRow>, DbError>;

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
    async fn finish(&self, id: Uuid, worker_id: &str) -> Result<bool, DbError>;

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
    /// `Ok(false)` has the same meaning as in [`Jobs::finish`], and matters more: the
    /// job is still leased by whoever holds it now, and this worker's answer has
    /// been discarded.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the write fails.
    async fn reschedule(
        &self,
        id: Uuid,
        worker_id: &str,
        delay: Duration,
        last_error: Option<&str>,
    ) -> Result<bool, DbError>;

    /// Replaces a leased job's `payload`, without touching its schedule.
    ///
    /// Separate from [`Jobs::reschedule`] because the recovery state in the
    /// payload has to survive the *current* attempt even when the job is not
    /// being rescheduled at all. The two writes are deliberately not atomic
    /// with each other — `docs/reference/vpay-db.md` §"`set_payload` is a
    /// separate write from `reschedule`" says what a crash between them can
    /// cost and what merging them would.
    ///
    /// Guarded on `locked_by` like every other write that follows a claim:
    /// `Ok(false)` means this worker no longer holds the job and its bookkeeping
    /// is being discarded along with the rest of its answer.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the write fails, including a payload that is not a
    /// JSON object (`payload_is_object`).
    async fn set_payload(
        &self,
        id: Uuid,
        worker_id: &str,
        payload: &serde_json::Value,
    ) -> Result<bool, DbError>;

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
    async fn release_all(&self, worker_id: &str) -> Result<u64, DbError>;

    /// Frees every lease older than `lease`, returning how many.
    ///
    /// This is the counterpart to [`Jobs::claim`]'s exact-index predicate (see the
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
    async fn reap_expired_leases(&self, lease: Duration) -> Result<u64, DbError>;

    /// Parks a job nothing can fix, recording why, and releases its lease.
    ///
    /// The write the module comment describes: `run_at = 'infinity'` and the
    /// lease cleared, in one statement. See that comment for why this is a park
    /// rather than a `DELETE`, why `'infinity'` rather than a `dead_lettered_at`
    /// column, and what it costs.
    ///
    /// `Ok(false)` has the same meaning as in [`Jobs::finish`] and [`Jobs::reschedule`]: this
    /// worker no longer holds the job, so its verdict — including this one — is
    /// being discarded. The job stays claimable by whoever holds it now, which is
    /// the right outcome: a second worker that reaches the same conclusion parks
    /// it under its own lease.
    ///
    /// `last_error` is truncated to `LAST_ERROR_MAX_CHARS` for exactly the reason
    /// [`Jobs::reschedule`] truncates it, and more sharply: this is the last thing ever
    /// written about the row, so a refused write here loses the only record of
    /// why a payment stopped being driven.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the write fails.
    async fn dead_letter(
        &self,
        id: Uuid,
        worker_id: &str,
        last_error: &str,
    ) -> Result<bool, DbError>;

    /// Which of these dedupe keys name a **parked** job (`run_at = 'infinity'`).
    ///
    /// The one read that makes a dead letter visible to something other than an
    /// operator running `SELECT * FROM jobs WHERE run_at = 'infinity'` by hand.
    /// The module comment states the cost of parking — a parked row is invisible
    /// to every `run_at`-ordered query, so the alert raised when it was parked is
    /// the only notice anyone gets — and this is how a backstop scan that *cannot
    /// recover* such a row can at least name it.
    ///
    /// # Why the caller supplies the keys
    ///
    /// A `dedupe_key` is the idempotency key of a piece of *work*, and its
    /// grammar (`poll:<charge_id>`, `webhook:<uuid>`) belongs to the crate that
    /// enqueues the work — `vpay_worker::jobs` — not here. A query that built one
    /// would put half of that vocabulary in the persistence layer, where a change
    /// to the other half could not reach it. So this asks a question about keys
    /// the caller already holds.
    ///
    /// Ordered by key so a caller logging the answer logs it deterministically.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn parked_dedupe_keys(&self, keys: &[String]) -> Result<Vec<String>, DbError>;

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
    async fn oldest_runnable_run_at(&self) -> Result<Option<OffsetDateTime>, DbError>;
}

#[async_trait::async_trait]
impl Jobs for crate::repository::PgRepositories {
    async fn claim(&self, worker_id: &str) -> Result<Option<JobRow>, DbError> {
        let sql = format!(
            "UPDATE jobs SET locked_at = now(), locked_by = $1, attempts = attempts + 1 \
         WHERE id = (SELECT id FROM jobs WHERE run_at <= now() AND locked_at IS NULL \
                     ORDER BY run_at FOR UPDATE SKIP LOCKED LIMIT 1) \
         RETURNING {CLAIM_RETURNING}"
        );

        sqlx::query_as::<_, JobRow>(AssertSqlSafe(sql))
            .bind(worker_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn finish(&self, id: Uuid, worker_id: &str) -> Result<bool, DbError> {
        let deleted = sqlx::query("DELETE FROM jobs WHERE id = $1 AND locked_by = $2")
            .bind(id)
            .bind(worker_id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Query)?
            .rows_affected();

        Ok(deleted == 1)
    }

    async fn reschedule(
        &self,
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
        .execute(&self.pool)
        .await
        .map_err(classify_write)?
        .rows_affected();

        Ok(updated == 1)
    }

    async fn set_payload(
        &self,
        id: Uuid,
        worker_id: &str,
        payload: &serde_json::Value,
    ) -> Result<bool, DbError> {
        let updated = sqlx::query("UPDATE jobs SET payload = $3 WHERE id = $1 AND locked_by = $2")
            .bind(id)
            .bind(worker_id)
            .bind(payload)
            .execute(&self.pool)
            .await
            .map_err(classify_write)?
            .rows_affected();

        Ok(updated == 1)
    }

    async fn release_all(&self, worker_id: &str) -> Result<u64, DbError> {
        let released =
            sqlx::query("UPDATE jobs SET locked_at = NULL, locked_by = NULL WHERE locked_by = $1")
                .bind(worker_id)
                .execute(&self.pool)
                .await
                .map_err(DbError::Query)?
                .rows_affected();

        Ok(released)
    }

    async fn reap_expired_leases(&self, lease: Duration) -> Result<u64, DbError> {
        let reaped = sqlx::query(
            "UPDATE jobs \
         SET locked_at = NULL, locked_by = NULL, last_error = 'lease expired' \
         WHERE locked_at < now() - ($1::BIGINT * INTERVAL '1 microsecond')",
        )
        .bind(as_micros(lease))
        .execute(&self.pool)
        .await
        .map_err(classify_write)?
        .rows_affected();

        Ok(reaped)
    }

    async fn dead_letter(
        &self,
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
        .execute(&self.pool)
        .await
        .map_err(classify_write)?
        .rows_affected();

        Ok(updated == 1)
    }

    async fn parked_dedupe_keys(&self, keys: &[String]) -> Result<Vec<String>, DbError> {
        // Postgres accepts an empty array, but the round trip is pure cost and
        // the answer is knowable here.
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        sqlx::query_scalar::<_, String>(
            "SELECT dedupe_key FROM jobs \
         WHERE dedupe_key = ANY($1) AND run_at = 'infinity'::TIMESTAMPTZ \
         ORDER BY dedupe_key",
        )
        .bind(keys)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn oldest_runnable_run_at(&self) -> Result<Option<OffsetDateTime>, DbError> {
        sqlx::query_scalar::<_, Option<OffsetDateTime>>(
            "SELECT min(run_at) FROM jobs \
         WHERE locked_at IS NULL AND run_at < 'infinity'::TIMESTAMPTZ",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Query)
    }
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
