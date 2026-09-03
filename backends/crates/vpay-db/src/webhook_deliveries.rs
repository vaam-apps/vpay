//! The `webhook_deliveries` repository
//! (`backends/migrations/0022_create-webhook-deliveries.sql`) — what vpay owes
//! each merchant endpoint, and what happened on each attempt.
//!
//! One row per (event, endpoint), created by the fan-out's per-event
//! transaction together with the `deliver_webhook` jobs and the
//! [`crate::TxRepositories::mark_fanned_out_in_tx`] that closes it — which is
//! why that `events` write lives in this module and why neither it nor
//! [`crate::TxRepositories::create_in_tx`] has a pooled variant.
//!
//! Every column but `created_at` describes the most recent attempt: this is a
//! *state* row, not an append-only attempt log. `payload_sha256` is the one
//! column that deliberately does not move.
//!
//! The process is `docs/flows/webhooks.md`; the reasoning behind this module's
//! shape is `docs/reference/vpay-db.md` §"`webhook_deliveries`".

use std::time::Duration;

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{DbError, classify_write};

/// The `response_excerpt` ceiling from the `excerpt_length` CHECK
/// (migration 0022), in characters.
///
/// The writers truncate to it rather than letting the database refuse the
/// write, for the same reason [`crate::Jobs::reschedule`] bounds
/// `last_error`: the whole point of the write is to record *what the
/// receiver said*, and a receiver that answers with a long HTML error page
/// would otherwise turn a recorded failure into no record at all — the
/// delivery would keep its old `state` and `next_attempt_at`, and nothing
/// anywhere would say why.
///
/// This is the column's own bound, not the delivery handler's: the handler
/// already cuts the body to a shorter excerpt (design §4). This is the
/// backstop for a caller that does not.
const EXCERPT_MAX_CHARS: usize = 2000;

/// The columns [`DeliveryRow`] decodes, spelled once so the four reads
/// cannot drift on what they select.
///
/// `created_at`, `sent_at` and `responded_at` are deliberately absent: no
/// caller branches on them, they are read by operators in `psql` and in the
/// runbook, and selecting a column no Rust code uses invites a row struct
/// that grows to mirror the table rather than to serve its callers.
const COLUMNS: &str = "id, event_id, endpoint_id, url, attempt, state, status_code, \
                       response_excerpt, payload_sha256, next_attempt_at";

/// One `webhook_deliveries` row, as the delivery handler and the runbook
/// queries see it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DeliveryRow {
    /// Database-generated identity. Like a job, a delivery has no public
    /// `wd_…` id: nothing outside vpay ever names one, so there is nothing
    /// for a readable prefix to help with. It is the `deliver_webhook`
    /// job's payload and its dedupe key (`webhook:{id}`).
    pub id: Uuid,
    /// The `evt_…` this delivery carries. A foreign key, unlike
    /// `endpoint_id`, because the event really is a row in this database.
    pub event_id: String,
    /// The operator-authored endpoint id from
    /// `merchant_clients[].webhooks[].id`. References no table — endpoints
    /// are YAML (ADR-0003) — so an id naming an endpoint that has since
    /// been removed from configuration is a real and expected state, and
    /// the handler must cope with it rather than assume a join.
    pub endpoint_id: String,
    /// Where the bytes were sent, as configured when the delivery was
    /// created. Read from the row rather than re-resolved from
    /// configuration on each attempt would be the other choice; it is not
    /// made here, because this crate cannot see configuration at all.
    pub url: String,
    /// How many attempts have **failed**. Zero until the first failure, so
    /// it is directly the retry-ladder index, and the ladder running out is
    /// what `state = 'exhausted'` records.
    pub attempt: i32,
    /// `pending`, `succeeded` or `exhausted`. Carried as `String` for the
    /// same reason every other closed vocabulary in this crate is: the
    /// vocabulary belongs above the persistence layer, and the database
    /// (`state_is_known`) is what actually closes it.
    ///
    /// `failed` is in the CHECK's vocabulary and no writer here produces
    /// it: a failure that has not exhausted the ladder stays `pending`,
    /// because "an attempt is still owed" is the fact the delivery loop
    /// reads.
    pub state: String,
    /// The HTTP status of the most recent attempt. `None` for a transport
    /// failure — nothing was received — and for a delivery never attempted.
    pub status_code: Option<i32>,
    /// The first part of the most recent response body, truncated to
    /// `EXCERPT_MAX_CHARS`. For an operator reading a runbook, not for a
    /// branch: nothing in vpay parses a receiver's body.
    pub response_excerpt: Option<String>,
    /// Hex SHA-256 of the exact bytes signed by the first attempt that
    /// **rendered and signed a body**, written once and never rewritten.
    /// `None` for a delivery that has not had one — including one with
    /// `attempt > 0`, whose attempts so far were abandoned before rendering.
    /// The handler compares its re-rendered body against this before sending;
    /// see [`WebhookDeliveries::record_attempt`] for why the column is `COALESCE`d rather than
    /// assigned.
    pub payload_sha256: Option<String>,
    /// When the next attempt is due. `None` for a delivery that has never
    /// been attempted — its `deliver_webhook` job was enqueued in the same
    /// transaction that created it, so the queue owns it — and `None` once
    /// no further attempt is owed.
    pub next_attempt_at: Option<OffsetDateTime>,
}

/// Creates the delivery for one (event, endpoint) pair **inside the
/// caller's transaction**, returning its id, or `None` if it already
/// existed.
///
/// `Ok(None)` is the normal answer for a re-run of a fan-out pass that
/// crashed before it could commit, **not** an error: the unique index
/// `webhook_deliveries_event_endpoint` is what makes the drain's
/// at-least-once execution deliver exactly once, and a caller seeing `None`
/// should enqueue nothing further — the earlier pass's job already exists
/// under the same `jobs.dedupe_key`.
///
/// Deliberately not an upsert. `DO UPDATE SET url = …` would let a re-run
/// silently re-point a delivery that has already been attempted, so the row
/// would no longer say where the bytes actually went.
///
/// # Errors
///
/// [`DbError::ForeignKeyViolation`] if `event_id` names no event — which
/// can only be a vpay bug, since the caller reached this row from the event
/// backlog. [`DbError::Query`] otherwise, including an `endpoint_id` or
/// `url` outside the length CHECKs, which is a configuration value that
/// should have been refused at boot.
pub(crate) async fn create_in_tx(
    tx: &mut PgConnection,
    event_id: &str,
    endpoint_id: &str,
    url: &str,
) -> Result<Option<Uuid>, DbError> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO webhook_deliveries (event_id, endpoint_id, url) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (event_id, endpoint_id) DO NOTHING \
         RETURNING id",
    )
    .bind(event_id)
    .bind(endpoint_id)
    .bind(url)
    .fetch_optional(&mut *tx)
    .await
    .map_err(classify_write)
}

/// Marks an event fanned out **inside the caller's transaction**, and only
/// if it is still `pending`.
///
/// `Ok(false)` means some other pass already fanned this event out. The
/// caller must roll back rather than commit: its inserts were computed
/// against a backlog entry that is no longer its to claim, and committing
/// them would be a second set of deliveries for an event already delivered.
///
/// The `AND fanout_state = 'pending'` half is what makes that detectable at
/// all. Without it, two drains racing on the same page both "succeed", both
/// commit, and the only thing standing between the merchant and two
/// deliveries is the unique index — which would hold, but would report the
/// collision as an error from a write the caller believed was new, rather
/// than as this quiet, expected `false`.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub(crate) async fn mark_fanned_out_in_tx(
    tx: &mut PgConnection,
    event_id: &str,
) -> Result<bool, DbError> {
    let updated = sqlx::query(
        "UPDATE events SET fanout_state = 'done' \
         WHERE id = $1 AND fanout_state = 'pending'",
    )
    .bind(event_id)
    .execute(&mut *tx)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(updated == 1)
}

/// The excerpt as the column will accept it: at most `EXCERPT_MAX_CHARS`
/// characters, cut on a character boundary.
///
/// Counted in `char`s because the CHECK is `char_length`, and because
/// truncating a UTF-8 string by bytes either panics (`String`) or produces
/// mojibake in an operator's log. A receiver's error page is exactly the
/// kind of body that is long and not ASCII.
fn bounded_excerpt(excerpt: Option<&str>) -> Option<String> {
    excerpt.map(|text| text.chars().take(EXCERPT_MAX_CHARS).collect())
}

#[async_trait::async_trait]
pub trait WebhookDeliveries: Send + Sync {
    /// Reads one delivery by its own id, which is what the `deliver_webhook`
    /// job's payload carries.
    ///
    /// No merchant scope, and none is possible: this is the worker's read,
    /// reached from a job it has just claimed, and the merchant is a property of
    /// the event this row points at. The same argument as
    /// [`crate::PaymentIntents::get_by_id`]'s.
    ///
    /// `None` means the row is gone — a delivery whose event was deleted, or a
    /// job whose row it names no longer exists. The handler must treat that as
    /// "nothing to do" rather than as a failure to retry.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get(&self, id: Uuid) -> Result<Option<DeliveryRow>, DbError>;

    /// Records that the receiver answered `2xx`: the delivery is finished.
    ///
    /// Guarded on `state = 'pending'`, so `Ok(false)` means the delivery was
    /// already settled by someone else — a second worker running the same job
    /// after a lease was reaped, or a re-run of a job whose delete was lost.
    /// That guard is in the statement rather than in a preceding `SELECT` for
    /// the reason every compare-and-swap in this crate is: a read-then-write
    /// leaves a window in which both workers see `pending` and both write, and
    /// the second one would resurrect `sent_at`/`responded_at` for an attempt
    /// that had already been superseded.
    ///
    /// `attempt` is **not** incremented: it counts failures, and it is the retry
    /// ladder's index. `next_attempt_at` is cleared, because a succeeded
    /// delivery is owed nothing.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the write fails. A `status` outside `INT`
    /// or an over-long excerpt cannot arise — the excerpt is truncated here.
    async fn record_success(
        &self,
        id: Uuid,
        status: i32,
        excerpt: Option<&str>,
        sha: &str,
    ) -> Result<bool, DbError>;

    /// Records one *failed* attempt: the receiver refused, or nothing came back.
    ///
    /// `status` is `None` for a transport failure and `responded_at` is cleared
    /// to match; `exhausted` is the caller's decision, because the retry ladder
    /// is `vpay_worker::delivery_delay`'s; `sha` is `None` for an attempt that
    /// signed nothing and `COALESCE`d when it is `Some`, so the first signed
    /// body's digest is the one that survives. `docs/reference/vpay-db.md`
    /// §"`record_attempt`" says what each of those three would otherwise make
    /// the row claim.
    ///
    /// Guarded on `state = 'pending'` for the same reason as
    /// [`WebhookDeliveries::record_success`], with the same meaning for
    /// `Ok(false)`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the write fails.
    async fn record_attempt(
        &self,
        id: Uuid,
        status: Option<i32>,
        excerpt: Option<&str>,
        sha: Option<&str>,
        next_attempt_at: Option<OffsetDateTime>,
        exhausted: bool,
    ) -> Result<bool, DbError>;

    /// Deliveries that are owed an attempt nothing appears to be making, oldest
    /// first, at most `limit` of them.
    ///
    /// The **backstop scan**'s whole query — `vpay_worker`'s `scan_deliveries`
    /// job (migration 0023) reads it every pass. It is not the scheduler and
    /// must never become one; in a healthy deployment it returns nothing.
    ///
    /// Two shapes qualify: `next_attempt_at <= now()`, and
    /// `next_attempt_at IS NULL AND created_at < now() - lease` — a delivery
    /// that has never been attempted and whose job is not simply young.
    /// `docs/reference/vpay-db.md` §"`pending_due` is a backstop, never a
    /// scheduler" says why the second clause exists, why the `lease` is a
    /// parameter, and what a dead-lettered job does to the caller's insert.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn pending_due(&self, lease: Duration, limit: i64) -> Result<Vec<DeliveryRow>, DbError>;

    /// Every delivery created for one event, in `endpoint_id` order.
    ///
    /// The fan-out's own claim — "one event becomes exactly one delivery per
    /// configured endpoint, however many times the drain runs" — is only
    /// checkable by reading all of them back, so this is the read that makes it
    /// assertable rather than assumed. Ordered by `endpoint_id` so an assertion
    /// about *which* endpoints were fanned out does not depend on insert order.
    ///
    /// Also the query a runbook needs when a merchant asks why one of their two
    /// endpoints saw an event and the other did not.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn for_event(&self, event_id: &str) -> Result<Vec<DeliveryRow>, DbError>;
}

#[async_trait::async_trait]
impl WebhookDeliveries for crate::repository::PgRepositories {
    async fn get(&self, id: Uuid) -> Result<Option<DeliveryRow>, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM webhook_deliveries WHERE id = $1");

        sqlx::query_as::<_, DeliveryRow>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn record_success(
        &self,
        id: Uuid,
        status: i32,
        excerpt: Option<&str>,
        sha: &str,
    ) -> Result<bool, DbError> {
        let updated = sqlx::query(
            "UPDATE webhook_deliveries \
         SET state = 'succeeded', \
             status_code = $2, \
             response_excerpt = $3, \
             payload_sha256 = COALESCE(payload_sha256, $4), \
             sent_at = now(), \
             responded_at = now(), \
             next_attempt_at = NULL \
         WHERE id = $1 AND state = 'pending'",
        )
        .bind(id)
        .bind(status)
        .bind(bounded_excerpt(excerpt))
        .bind(sha)
        .execute(&self.pool)
        .await
        .map_err(classify_write)?
        .rows_affected();

        Ok(updated == 1)
    }

    async fn record_attempt(
        &self,
        id: Uuid,
        status: Option<i32>,
        excerpt: Option<&str>,
        sha: Option<&str>,
        next_attempt_at: Option<OffsetDateTime>,
        exhausted: bool,
    ) -> Result<bool, DbError> {
        let updated = sqlx::query(
            "UPDATE webhook_deliveries \
         SET attempt = attempt + 1, \
             state = CASE WHEN $6 THEN 'exhausted' ELSE 'pending' END, \
             status_code = $2, \
             response_excerpt = $3, \
             payload_sha256 = COALESCE(payload_sha256, $4), \
             sent_at = now(), \
             responded_at = CASE WHEN $2::INT IS NULL THEN NULL ELSE now() END, \
             next_attempt_at = $5 \
         WHERE id = $1 AND state = 'pending'",
        )
        .bind(id)
        .bind(status)
        .bind(bounded_excerpt(excerpt))
        .bind(sha)
        .bind(next_attempt_at)
        .bind(exhausted)
        .execute(&self.pool)
        .await
        .map_err(classify_write)?
        .rows_affected();

        Ok(updated == 1)
    }

    async fn pending_due(&self, lease: Duration, limit: i64) -> Result<Vec<DeliveryRow>, DbError> {
        // Postgres refuses a negative LIMIT, and a zero-row page is never what a
        // caller means — same guard as `events::pending_page`.
        let limit = limit.max(1);
        // Seconds rather than a bound `INTERVAL`: sqlx has no `Duration` encoder
        // for `interval`, and the arithmetic is the same one
        // `crate::jobs::reap_expired_leases` writes for the same reason.
        let lease_seconds = i64::try_from(lease.as_secs()).unwrap_or(i64::MAX);
        let sql = format!(
            "SELECT {COLUMNS} FROM webhook_deliveries \
         WHERE state = 'pending' \
           AND ( \
             (next_attempt_at IS NOT NULL AND next_attempt_at <= now()) \
             OR (next_attempt_at IS NULL \
                 AND created_at < now() - ($1::BIGINT * INTERVAL '1 second')) \
           ) \
         ORDER BY next_attempt_at NULLS FIRST, created_at \
         LIMIT $2"
        );

        sqlx::query_as::<_, DeliveryRow>(&sql)
            .bind(lease_seconds)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn for_event(&self, event_id: &str) -> Result<Vec<DeliveryRow>, DbError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM webhook_deliveries WHERE event_id = $1 ORDER BY endpoint_id"
        );

        sqlx::query_as::<_, DeliveryRow>(&sql)
            .bind(event_id)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}

#[cfg(test)]
mod tests {
    use super::{EXCERPT_MAX_CHARS, bounded_excerpt};

    /// The bound is characters, not bytes, and it must hold for a body made
    /// entirely of multi-byte characters — the case where a byte-wise
    /// truncation would both overshoot the `char_length` CHECK and split a
    /// character.
    #[test]
    fn an_excerpt_is_bounded_in_characters_and_cut_on_a_boundary() {
        assert_eq!(bounded_excerpt(None), None);
        assert_eq!(bounded_excerpt(Some("ok")), Some("ok".to_owned()));

        let long = "é".repeat(EXCERPT_MAX_CHARS + 500);
        let bounded = bounded_excerpt(Some(&long)).expect("Some in, Some out");
        assert_eq!(
            bounded.chars().count(),
            EXCERPT_MAX_CHARS,
            "the CHECK counts characters, so this must too"
        );
        // Every character survived whole: a byte-wise cut would have left a
        // lone continuation byte, which cannot be a `char`.
        assert!(bounded.chars().all(|c| c == 'é'));
        assert!(long.starts_with(&bounded));
    }
}
