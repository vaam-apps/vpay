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

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{DbError, classify_write};
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{Cratestack, CreateWebhookDeliveryInput, UpdateEventInput};

/// The `.cstack` model behind `webhook_deliveries`, named once so a policy
/// denial says which model refused rather than which table.
const DELIVERY_MODEL: &str = "WebhookDelivery";

/// The `.cstack` model behind `events`. Named here, not in
/// [`crate::events`], because the only CrateStack call against it is
/// [`mark_fanned_out_in_tx`] below — the fan-out's closing write lives in
/// this module for the same reason the rest of the two-step outbox does.
const EVENT_MODEL: &str = "Event";

/// The `fanout_state` a freshly emitted event carries, and the only value
/// [`mark_fanned_out_in_tx`]'s compare-and-swap will move.
const FANOUT_PENDING: &str = "pending";

/// The `fanout_state` of an event whose deliveries have all been created.
const FANOUT_DONE: &str = "done";

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
/// **That contract holds for a delivery an earlier, *committed* pass created,
/// and only for one.** Since this call moved to CrateStack on 2026-09-06, a
/// second call for the same `(event_id, endpoint_id)` inside the **same,
/// still-open** transaction is refused with
/// [`crate::PersistenceError::Denied`] rather than answered `None` — the
/// statement it replaced answered `None` for that too. The cause is that
/// `.do_nothing()` splits its branch decision across two connections:
/// `resolve_pre_probe` reads through the caller's transaction and sees the
/// uncommitted row, and `authorize_existing_row`'s update-policy re-check is
/// a `SELECT` on a **pool** connection that cannot
/// (`upsert_do_nothing_authorize.rs` -> `upsert_sql.rs`), so it concludes the
/// policy denied a row it simply could not see.
///
/// Nothing reaches it today, by two guards in other crates rather than by
/// anything here: `vpay_config` refuses a duplicate webhook endpoint `id` at
/// boot, and `EndpointRegistry::from_pairs` dedups by id, so `fan_out_one`'s
/// loop cannot call this twice for one pair.
/// `a_repeat_creation_inside_one_transaction_is_refused_rather_than_reported_missing`
/// pins both halves — the refusal and the unchanged committed-row `None` —
/// so the day either changes it is a red test rather than a duplicate
/// webhook. `docs/plans/exp18-notes/opus-review.md` F2 has the measurement.
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
    cs: &Cratestack,
    tx: &mut Transaction<'_, Postgres>,
    event_id: &str,
    endpoint_id: &str,
    url: &str,
) -> Result<Option<Uuid>, DbError> {
    // `.upsert(..).do_nothing()`, not `.create(..)`, and the difference is
    // the whole contract of this function. A bare INSERT raises `23505`
    // against `webhook_deliveries_event_endpoint` for a pair that already
    // exists — and a failed statement aborts the enclosing Postgres
    // transaction, so the caller could not turn that back into the `Ok(None)`
    // its at-least-once drain depends on. `ON CONFLICT DO NOTHING` is what
    // keeps a re-run of a crashed fan-out pass silent instead of fatal.
    //
    // `on_conflict` names the pair rather than the primary key because the
    // primary key is a fresh `Uuid` that has never collided with anything.
    // `ConflictTarget::columns` needs no `@@unique` in the model: it is a
    // caller-supplied tuple, validated only against the "a predicate needs
    // columns" rule (`cratestack-sql`'s `conflict.rs::validate`). The index
    // that makes it legal is `webhook_deliveries_event_endpoint`, and it is
    // migration 0022's, not this schema's.
    //
    // The id is minted here rather than by `DEFAULT gen_random_uuid()`,
    // because a `@default(...)` on the primary key is exactly what makes
    // `CreateWebhookDeliveryInput` carry no `id` and `UpsertModelInput` not
    // exist — `schemas/vpay.cstack`'s `model WebhookDelivery` carries the
    // measurement and the one drift line it costs.
    let input = CreateWebhookDeliveryInput {
        id: Uuid::new_v4(),
        event_id: event_id.to_owned(),
        endpoint_id: endpoint_id.to_owned(),
        url: url.to_owned(),
        // Explicitly `None` rather than absent, and not the same thing as
        // the columns that are missing from this literal. `state` and
        // `created_at` carry `@default(...)` and are therefore filtered out
        // of `CreateWebhookDeliveryInput` altogether by
        // `inputs.rs::create_input_fields` — they take their column
        // defaults, which is what the hand-written INSERT relied on too.
        // These five have no default; the input requires them, and binding
        // `NULL` is exactly what omitting them from the old three-column
        // INSERT did. A delivery is born owing an attempt and knowing
        // nothing about one.
        payload_sha256: None,
        response_excerpt: None,
        sent_at: None,
        responded_at: None,
        next_attempt_at: None,
    };

    let outcome = cs
        .webhook_delivery()
        .upsert(input)
        .do_nothing()
        .on_conflict(cratestack::ConflictTarget::columns(&[
            "event_id",
            "endpoint_id",
        ]))
        // `run_in_tx`, not `run`. `run` opens its own transaction off
        // `runtime.pool()`, which would commit a delivery the caller's own
        // rollback could no longer take back — and the fan-out's
        // `TxOutcome::Abandon` path exists precisely to take it back.
        // `a_delivery_written_through_cratestack_is_rolled_back_with_the_fan_out`
        // is the test that fails when this word changes.
        .run_in_tx(&mut *tx, &system_context())
        .await
        .map_err(|error| DbError::from(classify_cratestack(DELIVERY_MODEL, "upsert", error)))?;

    // `Inserted` and `Existing` are why `.do_nothing()` returns an enum at
    // all: Postgres RETURNs nothing for a row `DO NOTHING` skipped, so the
    // framework resolves it under the conflict probe's row lock. Mapping
    // `Existing` to `None` is what the previous `fetch_optional` on
    // `RETURNING id` did, and the caller's meaning is unchanged — "somebody
    // else already owns this delivery, enqueue nothing".
    Ok(match outcome.value {
        cratestack::UpsertOutcome::Inserted(row) => Some(row.id),
        cratestack::UpsertOutcome::Existing(_) => None,
    })
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
    cs: &Cratestack,
    tx: &mut Transaction<'_, Postgres>,
    event_id: &str,
) -> Result<bool, DbError> {
    // `update_many` and not `update(pk)`, because the guard is the point:
    // `update` by primary key has nowhere to put `AND fanout_state =
    // 'pending'`, and without that half two drains racing on one page both
    // "succeed" and both commit.
    //
    // The policy is compiled into this statement's own `WHERE`
    // (`update_many_exec.rs` calls `push_action_policy_query` with
    // `update_allow_policies`), which makes a missing
    // `@@allow("update", ...)` on `model Event` SILENT in the error channel
    // and total in effect: zero rows match, this returns `false`, and
    // `fan_out_one` abandons every transaction it ever opens. That is the
    // mutation `an_event_flip_denied_by_policy_abandons_the_fan_out` exists
    // for.
    let summary = cs
        .event()
        .update_many()
        .where_(crate::schema::cratestack_schema::event::id().eq(event_id.to_owned()))
        .where_(
            crate::schema::cratestack_schema::event::fanout_state().eq(FANOUT_PENDING.to_owned()),
        )
        .set(UpdateEventInput {
            fanout_state: Some(FANOUT_DONE.to_owned()),
            ..UpdateEventInput::default()
        })
        .run_in_tx(&mut *tx, &system_context())
        .await
        .map_err(|error| DbError::from(classify_cratestack(EVENT_MODEL, "update", error)))?;

    // `BatchSummary::ok` is `updated.len()` — `update_many_exec.rs` builds
    // the summary from the rows its `RETURNING` actually handed back, so it
    // is the same number `rows_affected()` gave the statement this replaced.
    //
    // Exactly one, or none. `id` is the primary key, so the two filters can
    // never match a second row; `== 1` rather than `> 0` so a future filter
    // change that widened the match would read as "not fanned out" instead
    // of silently claiming several events at once.
    Ok(summary.value.ok == 1)
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

        sqlx::query_as::<_, DeliveryRow>(AssertSqlSafe(sql))
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

        sqlx::query_as::<_, DeliveryRow>(AssertSqlSafe(sql))
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

        sqlx::query_as::<_, DeliveryRow>(AssertSqlSafe(sql))
            .bind(event_id)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::PgPoolOptions;

    use super::{DELIVERY_MODEL, EVENT_MODEL, EXCERPT_MAX_CHARS, bounded_excerpt};
    use crate::schema::cratestack_schema;

    /// A pool that has never opened a connection, and cannot: the port is
    /// unroutable. `connect_lazy` does no I/O and neither does
    /// `preview_sql`. Same shape and same reason as `config_reconcile.rs`'s
    /// and `disabled_clients.rs`'s helpers of the same name.
    fn lazy_cratestack() -> cratestack_schema::Cratestack {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its URL and connects to nothing");
        cratestack_schema::Cratestack::builder(pool).build()
    }

    /// The two statements the outbox now runs through CrateStack, pinned as
    /// rendered text, with no database.
    ///
    /// `docs/flows/webhooks.md`'s duplicate-delivery guard is a property of
    /// these two statements' *shapes* rather than of any value in them:
    /// `ON CONFLICT (event_id, endpoint_id) DO NOTHING` is what makes a
    /// re-run of a crashed drain silent, and `AND fanout_state = 'pending'`
    /// is what makes a lost race detectable. A generator change that dropped
    /// either would leave every container test green until two drains
    /// actually raced.
    #[tokio::test]
    async fn the_outbox_statements_keep_their_conflict_target_and_their_guard() {
        let cs = lazy_cratestack();

        let create = cs
            .webhook_delivery()
            .upsert(super::CreateWebhookDeliveryInput {
                id: uuid::Uuid::nil(),
                event_id: "evt_1".to_owned(),
                endpoint_id: "ep_1".to_owned(),
                url: "https://example.test/hook".to_owned(),
                payload_sha256: None,
                response_excerpt: None,
                sent_at: None,
                responded_at: None,
                next_attempt_at: None,
            })
            .do_nothing()
            .on_conflict(cratestack::ConflictTarget::columns(&[
                "event_id",
                "endpoint_id",
            ]))
            .preview_sql();

        assert!(
            create.contains("ON CONFLICT (event_id, endpoint_id) DO NOTHING"),
            "the conflict target is the (event, endpoint) pair, not the primary key — a fresh \
             UUID conflicts with nothing and every re-run would insert a duplicate: {create}"
        );
        assert!(
            create.starts_with("INSERT INTO webhook_deliveries (id,"),
            "the id must be in the INSERT list: it is minted by this crate precisely so that \
             `.upsert(..)` exists at all (schemas/vpay.cstack, model WebhookDelivery): {create}"
        );
        // The columns migration 0022 defaults (`state`, `created_at`,
        // `attempt`) plus the one the model does not declare at all
        // (`status_code`, `int4`). If one of these appears, a fresh delivery
        // has started being born with an explicit value instead of the
        // column default — `state` in particular would then be whatever this
        // crate last thought it was.
        //
        // Sliced rather than matched with a `format!` over the loop
        // variable, because `crate::sql_audit` refuses an interpolation next
        // to statement-shaped text and is right to.
        // Tokenised rather than substring-matched: `next_attempt_at`
        // contains `attempt`, and a `contains` check silently passed on the
        // wrong column until this test was run.
        let columns: Vec<&str> = create
            .split_once(") VALUES")
            .map_or(create.as_str(), |(head, _)| head)
            .rsplit_once('(')
            .map_or("", |(_, tail)| tail)
            .split(", ")
            .collect();
        for defaulted in ["state", "created_at", "attempt", "status_code"] {
            assert!(
                !columns.contains(&defaulted),
                "`{defaulted}` must not be in the generated INSERT column list: {create}"
            );
        }

        let flip = cs
            .event()
            .update_many()
            .where_(cratestack_schema::event::id().eq("evt_1".to_owned()))
            .where_(cratestack_schema::event::fanout_state().eq(super::FANOUT_PENDING.to_owned()))
            .set(super::UpdateEventInput {
                fanout_state: Some(super::FANOUT_DONE.to_owned()),
                ..super::UpdateEventInput::default()
            })
            .preview_sql();

        assert!(
            flip.starts_with("UPDATE events SET fanout_state = $1"),
            "the flip must set exactly one column — a second one here would mean \
             `UpdateEventInput` picked up a field the compare-and-swap never meant to write: \
             {flip}"
        );
        // `preview_sql` renders the filter list and the policy clause as the
        // literal placeholders `<filters>` and `<update_policy>` rather than
        // expanding them, so this test cannot assert the guard's *contents*
        // — `an_abandoned_fan_out_leaves_no_delivery_and_the_event_still_pending`
        // and `a_pending_event_is_flipped_once_and_a_second_flip_reports_false`
        // are what prove those against a real database. What it CAN assert is
        // the thing no container test can show: that a policy clause is
        // compiled into this statement's own WHERE at all.
        assert!(
            flip.contains("<update_policy>"),
            "the update policy is part of the statement's WHERE, which is exactly why a \
             missing `@@allow(\"update\", …)` on `model Event` matches zero rows instead of \
             raising — the claim `every_action_this_module_calls_has_an_allow_arm` rests on: \
             {flip}"
        );
        assert!(
            flip.contains("<filters>"),
            "the id and fanout_state guards reach the statement as filters: {flip}"
        );
        // `data` is not a column of `model Event`, so the generated
        // `RETURNING` projection cannot select it — which is what keeps the
        // JSONB column away from `cratestack::Value` entirely, on the read
        // side as well as the write side. This is the assertion that notices
        // the day somebody adds it; see `crate::events`' pinned blocker test
        // for why it is not there.
        assert!(
            !flip.contains("data"),
            "the generated projection must not touch events.data: {flip}"
        );
        assert!(
            flip.contains("RETURNING") && flip.contains("seq AS"),
            "update_many RETURNs the model projection — that is where the row count comes \
             from (`BatchSummary::ok` is `updated.len()`): {flip}"
        );
    }

    /// Every action this module calls on `Event` and `WebhookDelivery` has an
    /// `@@allow` arm, asserted against the **compiled descriptor** and with
    /// no database.
    ///
    /// The same test `config_reconcile.rs` and `disabled_clients.rs` carry,
    /// for the same measured reason: an `@@allow` line can be deleted from
    /// `schemas/vpay.cstack` and every gate stays green. The three arms below
    /// fail in three different ways, and only one of them is loud:
    ///
    ///   `WebhookDelivery` `create` -> LOUD, on every fan-out.
    ///       `run_upsert_do_nothing_in_tx` calls `evaluate_create_policies`
    ///       before any SQL runs.
    ///   `WebhookDelivery` `update` -> LOUD, but **only on a re-run**.
    ///       `upsert_do_nothing_authorize.rs` re-checks the update policy
    ///       solely on the already-exists branch, so a first fan-out
    ///       succeeds and only crash recovery fails. That is the arm most
    ///       likely to be deleted and least likely to be noticed.
    ///   `Event` `update` -> SILENT, and the catastrophic one.
    ///       `update_many_exec.rs` compiles the policy into the statement's
    ///       own `WHERE`, so the compare-and-swap matches zero rows,
    ///       `mark_fanned_out_in_tx` answers `false`, and `fan_out_one`
    ///       abandons every transaction it opens. Every log line says the
    ///       drain ran; no merchant ever receives anything.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::{EVENT_MODEL as event, WEBHOOK_DELIVERY_MODEL as delivery};

        assert!(
            !delivery.create_allow_policies.is_empty(),
            "`@@allow(\"create\", …)` is missing from `model WebhookDelivery`: every fan-out \
             would fail with `Forbidden` -> `PersistenceError::Denied` -> `Category::Internal`"
        );
        assert!(
            !delivery.update_allow_policies.is_empty(),
            "`@@allow(\"update\", …)` is missing from `model WebhookDelivery`: the first \
             fan-out of an event would succeed and only a RE-RUN would fail, because \
             `upsert_do_nothing_authorize.rs` consults this slot on the already-exists branch \
             alone. That is the crash-recovery path"
        );
        assert!(
            !event.update_allow_policies.is_empty(),
            "`@@allow(\"update\", …)` is missing from `model Event`: the fanout \
             compare-and-swap compiles the policy into its own WHERE, matches zero rows, and \
             every fan-out transaction is abandoned — silently, and no merchant is ever \
             delivered anything"
        );

        // No arm this module does not use. `model Provider`'s rule, applied
        // here: a permission no call site exercises is a permission nothing
        // can measure, and both of these models are one write path each.
        assert!(
            event.create_allow_policies.is_empty() && event.delete_allow_policies.is_empty(),
            "`model Event` grew a create/delete arm. The events INSERT is still raw sqlx (see \
             `crate::events`) and nothing deletes an event; grant one in the commit that moves \
             the write, not before it"
        );
        assert!(
            delivery.delete_allow_policies.is_empty(),
            "`model WebhookDelivery` grew a `@@allow(\"delete\", …)`; nothing deletes a \
             delivery, and the row is the durable record of what vpay owes a merchant"
        );

        // A `@@deny` wins over every arm above (`push_action_policy_query`
        // wraps the allow list in `NOT (<deny>) AND (…)`), so an accidental
        // one is worth failing on here rather than in a container.
        for (model, slot, denies) in [
            (EVENT_MODEL, "update", event.update_deny_policies),
            (EVENT_MODEL, "read", event.read_deny_policies),
            (DELIVERY_MODEL, "create", delivery.create_deny_policies),
            (DELIVERY_MODEL, "update", delivery.update_deny_policies),
            (DELIVERY_MODEL, "read", delivery.read_deny_policies),
        ] {
            assert!(
                denies.is_empty(),
                "`model {model}` grew a `@@deny(\"{slot}\", …)`; it overrides every `@@allow` \
                 for that action and no call site expects one"
            );
        }
    }

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
