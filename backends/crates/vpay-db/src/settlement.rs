//! Settlement: the single transaction that takes a charge terminal, and the
//! two reads the worker's recovery table needs before it can decide to.
//!
//! # One transaction, three rows, no half-settled state
//!
//! A rail answering `SUCCESSFUL` moves three things: the charge to
//! `succeeded`, the intent to `succeeded` with `amount_received` filled in,
//! and an `events` row a merchant will be told about. [`apply_succeeded`]
//! and [`apply_failed`] each write all three inside one transaction, because
//! every way of splitting them is a lie a merchant can observe:
//!
//! * charge without intent — `GET /v1/payment_intents/{id}` says the payment
//!   is still processing while the money has moved;
//! * intent without event — the merchant's webhook never fires for a
//!   payment that succeeded, and nothing retries it, because nothing knows
//!   it was missed;
//! * event without the rows — a webhook for a payment that did not settle.
//!
//! # Idempotent by compare-and-swap, not by a flag
//!
//! Both functions guard the charge `UPDATE` on the charge still being in a
//! *live* state (`payment_intents::LIVE_CHARGE_STATES`). A re-run
//! after a commit — the poll job was rescheduled because the worker died
//! between committing and deleting the job, which is a normal outcome, not
//! an error — matches zero rows and returns `Ok(None)`. The caller finishes
//! the job. Nothing is written twice, and in particular no second `events`
//! row is written, so at-least-once job execution does not become
//! at-least-twice webhook delivery for distinct event ids.
//!
//! That guard has to be in the statement. A `SELECT` that checked the state
//! first would leave a window in which two workers — one holding a stale
//! lease, one that just claimed the reaped job — both see a live charge and
//! both settle it.
//!
//! # The charge is the record of a confirm; the intent may lag it
//!
//! A confirm does **not** move the charge and the intent together. It commits
//! the charge (and its poll job) in one transaction *before* calling the
//! rail, and moves the intent only afterwards, in a second transaction, once
//! the rail has answered (`vpay_api::v1::payment_intents`,
//! `docs/flows/crash-safety.md`). All three of that document's kill points
//! therefore leave a **live charge against an intent still reading
//! `requires_payment_method`** — that is not a corrupt database, it is the
//! ordinary state a crashed confirm leaves and the one the recovery pass
//! exists to resolve.
//!
//! So the question these functions answer is never "does the intent's status
//! agree that a confirm happened". The charge answers that: the compare-and-
//! swap above has already matched a row in `LIVE_CHARGE_STATES`, and only a
//! confirm writes one. The intent write follows, over
//! `payment_intents::SETTLEABLE_STATUSES` — the two confirmed statuses *and*
//! `requires_payment_method` — so a settlement lands whether or not the
//! confirm survived long enough to move the intent.
//!
//! # What `None` does *not* mean
//!
//! It never means "the intent guard refused". After the widening above, the
//! only statuses left outside it are `succeeded` and `canceled`, and neither
//! can coexist with a live charge (`cancel` refuses to run while one exists,
//! and "one charge per intent, forever" means a settled intent cannot acquire
//! another). Either of them appearing here is a broken invariant, and it is
//! reported as [`DbError::WriteMatchedNoRow`] — `Category::Internal`, which
//! pages — rather than being folded into the idempotent `None` a caller
//! treats as "already done". Committing the charge half and reporting success
//! would leave the merchant's intent permanently out of step with the money.

use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::charges::ChargeRow;
use crate::error::{DbError, classify_write};
use crate::events::{self, NewEvent};
use crate::payment_intents::{self, LIVE_CHARGE_STATES, PaymentIntentRow};

/// The `failure_raw_length` CHECK's ceiling on `charges.failure_raw`
/// (migration 0004), in characters.
///
/// Truncated here rather than left to the database for the same reason
/// `payment_intents::fail_after_submission` bounds its message: a rail whose
/// text runs long would abort the settlement transaction, leaving the charge
/// live and the job retrying forever against text that will be exactly as
/// long next time. The rail's words are captured for operators, not stored in
/// full.
///
/// That function is named without a link because it is `pub(crate)` — the
/// visibility is what enforces "only ever called inside this transaction",
/// and rustdoc refuses a public link to a private item.
const FAILURE_RAW_MAX_CHARS: usize = 2000;

/// The `type` of the event a successful settlement emits.
///
/// One of the seven `type_is_a_documented_event` allows (migration 0018),
/// spelled here rather than passed in by the caller: the event type is a
/// property of *which settlement this is*, and a caller free to choose it
/// could emit `payment_intent.succeeded` for a failure. The `data` — the
/// wire object, which only `vpay-api` knows how to shape — is the caller's.
const EVENT_SUCCEEDED: &str = "payment_intent.succeeded";

/// The `type` of the event a failed settlement emits. See
/// [`EVENT_SUCCEEDED`].
const EVENT_PAYMENT_FAILED: &str = "payment_intent.payment_failed";

/// One `provider_requests` row — an attempt to call a rail — as the recovery
/// table reads it.
///
/// Lives here rather than in [`crate::provider_requests`] because the
/// recovery table is the only thing that reads this table at all, and its
/// question is not "what attempts exist" but "what does the last submit tell
/// me to do next" (`docs/flows/crash-safety.md`'s table, keyed on exactly
/// the `status_code`/`responded_at` pair below). Putting the row type beside
/// its one reader keeps that pairing documented where it is used.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AttemptRow {
    /// The attempt's own identity, and the tiebreak that makes "the latest
    /// submit" a total order — see [`latest_submit_attempt`].
    pub id: i64,
    /// The charge this attempt belongs to.
    pub charge_id: String,
    /// The rail it was sent to.
    pub provider_code: String,
    /// The rail-facing reference the attempt carried. The recovery rule that
    /// matters most is that a resubmit reuses *this* value rather than
    /// generating a new one (`docs/flows/crash-safety.md`).
    pub provider_reference_id: Uuid,
    /// Which attempt this was, 1-based.
    pub attempt: i32,
    /// The rail's HTTP status, `0` when the port carried an answer with no
    /// status ([`crate::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT`]),
    /// or `None` when **no answer was received** — the case the recovery
    /// table sends to the poll ladder.
    pub status_code: Option<i32>,
    /// Operator-facing label for an attempt that failed without a status.
    pub error_kind: Option<String>,
    /// When the call was made — written before it, by construction.
    pub sent_at: OffsetDateTime,
    /// When the answer arrived. `NULL` exactly when `status_code` is
    /// (`response_is_paired`), which is what makes the pair trustworthy.
    pub responded_at: Option<OffsetDateTime>,
}

/// Settles a charge the rail reported as paid: charge → `succeeded`, intent
/// → `succeeded` with `amount_received = amount`, and one
/// `payment_intent.succeeded` event — in one transaction.
///
/// Returns the charge and intent rows as they now stand, or `Ok(None)` if
/// the charge was no longer live, which means this settlement already
/// happened. See the module comment for why that is a normal answer and not
/// an error.
///
/// `provider_txn_id` is written with `COALESCE`, so an answer that carries
/// no identifier does not erase one an earlier write recorded. Nothing
/// writes that column before this function today; the `COALESCE` is what
/// keeps that true if a callback repair ever does.
///
/// `event_data` is the wire object as it was at settlement time
/// (`vpay-api`'s shape — this crate does not know it), and `event_id` is a
/// caller-generated `evt_…` ([`crate::events::event_id`]).
///
/// # Errors
///
/// [`DbError::WriteMatchedNoRow`] on `payment_intents` if the charge was
/// live but its intent was outside
/// `payment_intents::SETTLEABLE_STATUSES` — i.e. `succeeded` or
/// `canceled`, a broken invariant, which pages rather than being reported as
/// a merchant's problem. [`DbError::UniqueViolation`] if `event_id` was
/// already emitted.
/// [`DbError::Query`] if any statement or the commit fails; the transaction
/// is rolled back, so a failure leaves the charge exactly where a retry
/// expects it.
pub async fn apply_succeeded(
    pool: &PgPool,
    charge_id: &str,
    provider_txn_id: Option<&str>,
    event_id: &str,
    event_data: &serde_json::Value,
) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError> {
    let mut tx = pool.begin().await.map_err(DbError::Query)?;

    let sql = format!(
        "UPDATE charges \
         SET state = 'succeeded'::charge_state, \
             provider_txn_id = COALESCE($2, provider_txn_id), \
             updated_at = now() \
         WHERE id = $1 AND state IN ({LIVE_CHARGE_STATES}) \
         RETURNING {columns}",
        columns = crate::charges::COLUMNS,
    );
    let charge = sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(charge_id)
        .bind(provider_txn_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)?;
    let Some(charge) = charge else {
        // Already settled. Nothing was written, so there is nothing to
        // commit and nothing to roll back either — but the transaction is
        // closed explicitly rather than dropped, so the connection returns
        // to the pool without waiting for a background rollback.
        tx.rollback().await.map_err(DbError::Query)?;
        return Ok(None);
    };

    let intent = payment_intents::succeed_after_submission(&mut tx, &charge.payment_intent_id)
        .await?
        .ok_or_else(|| DbError::WriteMatchedNoRow {
            table: "payment_intents",
            key: charge.payment_intent_id.clone(),
        })?;

    emit(&mut tx, EVENT_SUCCEEDED, event_id, &intent, event_data).await?;

    tx.commit().await.map_err(DbError::Query)?;

    Ok(Some((charge, intent)))
}

/// Settles a charge the rail declined after it was submitted: charge →
/// `failed` with the failure pair, intent back to `requires_payment_method`
/// carrying `last_payment_error`, and one `payment_intent.payment_failed`
/// event — in one transaction.
///
/// `code` is the closed vocabulary (`vpay_core::FailureCode`) as text, and
/// the column is the `failure_code` Postgres enum, so a value outside it is
/// refused by the database rather than by convention. `raw` is the rail's own
/// words for the charge row, truncated to `FAILURE_RAW_MAX_CHARS`; `message`
/// is what the merchant is shown on the intent, truncated to 512 by
/// `payment_intents::fail_after_submission`. They are two arguments and not
/// one because they have two audiences — the raw text is for an operator
/// reconciling against the rail, the message is on the wire.
///
/// `Ok(None)`, and the errors, mean exactly what they do in
/// [`apply_succeeded`].
///
/// # Errors
///
/// As [`apply_succeeded`].
pub async fn apply_failed(
    pool: &PgPool,
    charge_id: &str,
    code: &str,
    raw: &str,
    message: &str,
    event_id: &str,
    event_data: &serde_json::Value,
) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError> {
    let bounded_raw: String = raw.chars().take(FAILURE_RAW_MAX_CHARS).collect();

    let mut tx = pool.begin().await.map_err(DbError::Query)?;

    let sql = format!(
        "UPDATE charges \
         SET state = 'failed'::charge_state, \
             failure_code = $2::failure_code, \
             failure_raw = $3, \
             updated_at = now() \
         WHERE id = $1 AND state IN ({LIVE_CHARGE_STATES}) \
         RETURNING {columns}",
        columns = crate::charges::COLUMNS,
    );
    let charge = sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(charge_id)
        .bind(code)
        .bind(&bounded_raw)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)?;
    let Some(charge) = charge else {
        tx.rollback().await.map_err(DbError::Query)?;
        return Ok(None);
    };

    let intent =
        payment_intents::fail_after_submission(&mut tx, &charge.payment_intent_id, code, message)
            .await?
            .ok_or_else(|| DbError::WriteMatchedNoRow {
                table: "payment_intents",
                key: charge.payment_intent_id.clone(),
            })?;

    emit(&mut tx, EVENT_PAYMENT_FAILED, event_id, &intent, event_data).await?;

    tx.commit().await.map_err(DbError::Query)?;

    Ok(Some((charge, intent)))
}

/// Appends the settlement's event inside the settlement's own transaction.
///
/// `merchant_id` and `livemode` are taken from the intent row this
/// transaction just wrote, not from configuration or from the caller: the
/// event must describe what was true of the object at emit time, and the
/// intent is the object.
async fn emit(
    tx: &mut Transaction<'_, Postgres>,
    event_type: &str,
    event_id: &str,
    intent: &PaymentIntentRow,
    data: &serde_json::Value,
) -> Result<(), DbError> {
    events::insert_in_tx(
        tx,
        &NewEvent {
            id: event_id.to_owned(),
            merchant_id: intent.merchant_id.clone(),
            livemode: intent.livemode,
            event_type: event_type.to_owned(),
            object_id: intent.id.clone(),
            data: data.clone(),
        },
    )
    .await?;

    Ok(())
}

/// Moves a charge between two *live* states, as a compare-and-swap, and
/// reports whether it fired.
///
/// This is every non-terminal rung of the poll ladder: `submitting` or
/// `submitted` → `pending` when the rail says the payer has been prompted,
/// `pending` → `unresolved` when the ladder gives up waiting. It deliberately
/// cannot settle a charge — `succeeded` and `failed` move an intent and emit
/// an event, and a function that could write those labels without doing
/// either would make the settlement transaction optional. Passing a terminal
/// label as `new` is refused by the caller's own state machine
/// (`vpay_core::settlement`), and if one ever reached here the charge would
/// move with no intent and no event; that is why the two operations are
/// different functions rather than one with a `new` parameter that means
/// everything.
///
/// `Ok(false)` means the charge was not in `expected` — someone else has
/// already moved it, which for a job that may be running twice is
/// information, not an error.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails, including a label outside
/// the `charge_state` enum.
pub async fn set_live_state(
    pool: &PgPool,
    charge_id: &str,
    expected: &str,
    new: &str,
) -> Result<bool, DbError> {
    let updated = sqlx::query(
        "UPDATE charges SET state = $3::charge_state, updated_at = now() \
         WHERE id = $1 AND state = $2::charge_state",
    )
    .bind(charge_id)
    .bind(expected)
    .bind(new)
    .execute(pool)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(updated == 1)
}

/// The most recent `submit` attempt for a charge, or `None` if the rail was
/// never called.
///
/// This is the read the crash-safety recovery table branches on
/// (`docs/flows/crash-safety.md`): no row at all means the process died
/// before it could even record the intention to call, so the charge is
/// resubmitted under its existing reference; a row with `status_code IS
/// NULL` means the call was issued and no answer came back, so the charge is
/// polled; a row with a status means the answer is already recorded and the
/// ladder advances from the charge's own state.
///
/// Ordered `sent_at DESC, id DESC` rather than by `sent_at` alone, which is
/// a deliberate deviation from the design's sketch of this query: `sent_at`
/// defaults to `now()`, which is *transaction* time in Postgres, so two
/// attempts recorded inside one transaction share it exactly and "the
/// latest" would be whichever the planner happened to return. The identity
/// primary key is monotone per insert, so the pair is a total order.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn latest_submit_attempt(
    pool: &PgPool,
    charge_id: &str,
) -> Result<Option<AttemptRow>, DbError> {
    sqlx::query_as::<_, AttemptRow>(
        "SELECT id, charge_id, provider_code, provider_reference_id, attempt, status_code, \
         error_kind, sent_at, responded_at \
         FROM provider_requests \
         WHERE charge_id = $1 AND operation = 'submit' \
         ORDER BY sent_at DESC, id DESC \
         LIMIT 1",
    )
    .bind(charge_id)
    .fetch_optional(pool)
    .await
    .map_err(DbError::Query)
}

/// The ids of live charges that have not moved since `cutoff`, oldest first,
/// at most `limit` of them.
///
/// The `scan_live_charges` backstop, and **only** a backstop: the poll job
/// for a charge is enqueued in the same transaction that opens the charge,
/// so recovery does not depend on this scan finding anything. What it covers
/// is the two cases that transaction cannot — rows written before the queue
/// existed, and a job lost to operator error (a `DELETE FROM jobs`). If this
/// query ever returns a steady stream in a healthy deployment, the enqueue
/// is broken and that is the bug to fix, not this interval.
///
/// The live set is `LIVE_CHARGE_STATES`, which is also the predicate of
/// `charges_live_idx` (migration 0014) — the index that exists for exactly
/// this query.
///
/// Ordered by `updated_at` so a backlog is worked oldest-first: the charge
/// that has been waiting longest is the one a payer is most likely already
/// asking about.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn live_charges_stale_since(
    pool: &PgPool,
    cutoff: OffsetDateTime,
    limit: i64,
) -> Result<Vec<String>, DbError> {
    let limit = limit.max(1);
    let sql = format!(
        "SELECT id FROM charges \
         WHERE state IN ({LIVE_CHARGE_STATES}) AND updated_at < $1 \
         ORDER BY updated_at \
         LIMIT $2"
    );

    sqlx::query_scalar::<_, String>(&sql)
        .bind(cutoff)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(DbError::Query)
}
