//! Settlement: the single transaction that takes a charge terminal, and the
//! two reads the worker's recovery table needs before it can decide to.
//!
//! [`Settlement::apply_succeeded`] and [`Settlement::apply_failed`] each move
//! the charge, move the intent and insert the `events` row a merchant will be
//! told about, inside **one** transaction — every way of splitting them is a
//! lie a merchant can observe. Both are idempotent by compare-and-swap on the
//! charge still being live, so a re-run after a commit writes nothing and
//! answers `Ok(None)`.
//!
//! `docs/reference/vpay-db.md` §"`settlement`" carries the reasoning: what each
//! split would produce, why the intent guard is deliberately wider than the
//! charge's, where the `from` transition label comes from, and why `None` never
//! means "the intent guard refused".

use sqlx::{Postgres, Row as _, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::charges::{ChargeRow, record_transition};
use crate::checkout_sessions;
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

/// The extra `RETURNING` expression that carries the state a settled charge
/// was in *before* the statement that settled it.
///
/// A correlated sub-select in `RETURNING`, so the `WHERE` clause, the row lock
/// and the "matched no row" answer are exactly as they were and only a value
/// is added to the output list. It reads the statement's own snapshot — an
/// `UPDATE` cannot see its own writes — which is what makes it the *previous*
/// state, and `docs/reference/vpay-db.md` §"The `from` label degrades rather
/// than failing a settlement" says why it is a sub-select and why it is
/// aliased away from `state`.
const PREVIOUS_STATE: &str =
    "(SELECT prev.state::TEXT FROM charges prev WHERE prev.id = charges.id) AS previous_state";

/// The `from` label used when [`PREVIOUS_STATE`] came back `NULL`.
///
/// Not reachable through any query this module writes — the sub-select is
/// correlated on the row the `UPDATE` just matched, so it always finds one —
/// which is exactly why it is a fallback and not an error. See
/// [`decode_settled`].
const UNKNOWN_PREVIOUS_STATE: &str = "unknown";

/// Decodes a settlement row into the previous state and the settled charge.
///
/// Hand-written rather than a second `sqlx::FromRow` struct because the pair
/// is not a row type anybody stores: `previous_state` exists only inside these
/// two statements.
///
/// The label is decoded leniently — `Option<String>` and
/// `unwrap_or_default()`, so a `NULL` *or* a failed decode renders as
/// `from="unknown"` rather than failing a settlement that has already
/// committed. `docs/reference/vpay-db.md` §"The `from` label degrades rather
/// than failing a settlement" says what the strict version would cost.
///
/// # Errors
///
/// [`DbError::Query`] if the *charge* does not decode — which would mean
/// `crate::charges::COLUMNS` had drifted from [`ChargeRow`], i.e. a bug here
/// rather than anything an operator did. That one still fails: it is the
/// settled row itself, not a label.
fn decode_settled(row: &sqlx::postgres::PgRow) -> Result<(String, ChargeRow), DbError> {
    use sqlx::FromRow as _;

    let previous_state: Option<String> = row.try_get("previous_state").unwrap_or_default();
    let charge = ChargeRow::from_row(row).map_err(DbError::Query)?;
    Ok((
        previous_state.unwrap_or_else(|| UNKNOWN_PREVIOUS_STATE.to_owned()),
        charge,
    ))
}

/// The `type` of the event a successful settlement emits.
///
/// One of the eight `type_is_a_documented_event` allows (migrations 0018
/// and 0029),
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
    /// submit" a total order — see [`Settlement::latest_submit_attempt`].
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

/// Flips the checkout session driving this intent, if there is one, inside
/// the settlement transaction.
///
/// # Why it is here and not in the worker
///
/// `docs/plans/2026-09-04-step9-hosted-checkout.md` calls this "the worker
/// hook", and the worker is where the *decision* is made — `settle_succeeded`
/// or `settle_failed` — but the *write* belongs in this transaction and
/// nowhere else. `checkout_sessions.payment_status` is a denormalisation of
/// what the intent says, kept so a payer's page can render an outcome from
/// one read; a second write after the commit would leave a window in which
/// the intent is `succeeded` and the session still `open`/`unpaid`, and a
/// crash in that window would make it permanent. There is no job that would
/// notice, and D10 adds none.
///
/// Logged rather than counted, and never fatal: see
/// `checkout_sessions::settle_for_intent` for why `Ok(0)` is the normal
/// answer.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails, which aborts the whole settlement
/// — deliberately. A session that could not be flipped is a session whose
/// page would poll forever against an intent that has already moved, and
/// rolling back means the poll job simply runs again.
async fn flip_session(
    tx: &mut Transaction<'_, Postgres>,
    intent: &PaymentIntentRow,
    paid: bool,
) -> Result<(), DbError> {
    let flipped = checkout_sessions::settle_for_intent(tx, &intent.id, paid).await?;
    if flipped > 0 {
        tracing::info!(
            payment_intent_id = %intent.id,
            sessions = flipped,
            paid,
            "a checkout session was settled beside the intent it drives"
        );
    }
    Ok(())
}

#[async_trait::async_trait]
pub trait Settlement: Send + Sync {
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
    async fn apply_succeeded(
        &self,
        charge_id: &str,
        provider_txn_id: Option<&str>,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError>;

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
    /// [`Settlement::apply_succeeded`].
    ///
    /// # Errors
    ///
    /// As [`Settlement::apply_succeeded`].
    async fn apply_failed(
        &self,
        charge_id: &str,
        code: &str,
        raw: &str,
        message: &str,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError>;

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
    async fn set_live_state(
        &self,
        charge_id: &str,
        expected: &str,
        new: &str,
    ) -> Result<bool, DbError>;

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
    async fn latest_submit_attempt(&self, charge_id: &str) -> Result<Option<AttemptRow>, DbError>;

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
    async fn live_charges_stale_since(
        &self,
        cutoff: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<String>, DbError>;
}

#[async_trait::async_trait]
impl Settlement for crate::repository::PgRepositories {
    async fn apply_succeeded(
        &self,
        charge_id: &str,
        provider_txn_id: Option<&str>,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        let sql = format!(
            "UPDATE charges \
         SET state = 'succeeded'::charge_state, \
             provider_txn_id = COALESCE($2, provider_txn_id), \
             updated_at = now() \
         WHERE id = $1 AND state IN ({LIVE_CHARGE_STATES}) \
         RETURNING {PREVIOUS_STATE}, {columns}",
            columns = crate::charges::COLUMNS,
        );
        let row = sqlx::query(&sql)
            .bind(charge_id)
            .bind(provider_txn_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?;
        let charge = row.as_ref().map(decode_settled).transpose()?;
        let Some((previous_state, charge)) = charge else {
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

        // Before the emit rather than after, so the ordering inside the
        // transaction reads the way the object graph does: charge, intent,
        // the session that drove it, then the event that tells the merchant
        // about all three. Nothing observable depends on the order — it is
        // one commit — but a reader should not have to work that out.
        flip_session(&mut tx, &intent, true).await?;

        emit(&mut tx, EVENT_SUCCEEDED, event_id, &intent, event_data).await?;

        tx.commit().await.map_err(DbError::Query)?;

        // After the commit, deliberately: this counter is a record of settled
        // money, and a transaction that rolled back settled none.
        record_transition(&charge.provider_code, &previous_state, &charge.state);

        Ok(Some((charge, intent)))
    }

    async fn apply_failed(
        &self,
        charge_id: &str,
        code: &str,
        raw: &str,
        message: &str,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError> {
        let bounded_raw: String = raw.chars().take(FAILURE_RAW_MAX_CHARS).collect();

        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        let sql = format!(
            "UPDATE charges \
         SET state = 'failed'::charge_state, \
             failure_code = $2::failure_code, \
             failure_raw = $3, \
             updated_at = now() \
         WHERE id = $1 AND state IN ({LIVE_CHARGE_STATES}) \
         RETURNING {PREVIOUS_STATE}, {columns}",
            columns = crate::charges::COLUMNS,
        );
        let row = sqlx::query(&sql)
            .bind(charge_id)
            .bind(code)
            .bind(&bounded_raw)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?;
        let charge = row.as_ref().map(decode_settled).transpose()?;
        let Some((previous_state, charge)) = charge else {
            tx.rollback().await.map_err(DbError::Query)?;
            return Ok(None);
        };

        let intent = payment_intents::fail_after_submission(
            &mut tx,
            &charge.payment_intent_id,
            code,
            message,
        )
        .await?
        .ok_or_else(|| DbError::WriteMatchedNoRow {
            table: "payment_intents",
            key: charge.payment_intent_id.clone(),
        })?;

        // `paid: false` — the session becomes `expired`/`failed` (D10: there
        // is no `failed` session status). The *intent* goes back to
        // `requires_payment_method` and could in principle be confirmed
        // again, but this session cannot drive that: one charge per intent,
        // forever, so a retry is a new intent and therefore a new session.
        flip_session(&mut tx, &intent, false).await?;

        emit(&mut tx, EVENT_PAYMENT_FAILED, event_id, &intent, event_data).await?;

        tx.commit().await.map_err(DbError::Query)?;

        record_transition(&charge.provider_code, &previous_state, &charge.state);

        Ok(Some((charge, intent)))
    }

    async fn set_live_state(
        &self,
        charge_id: &str,
        expected: &str,
        new: &str,
    ) -> Result<bool, DbError> {
        let moved = sqlx::query(
            "UPDATE charges SET state = $3::charge_state, updated_at = now() \
         WHERE id = $1 AND state = $2::charge_state \
         RETURNING provider_code",
        )
        .bind(charge_id)
        .bind(expected)
        .bind(new)
        .fetch_optional(&self.pool)
        .await
        .map_err(classify_write)?;

        // `RETURNING provider_code` rather than counting affected rows: the
        // metric below needs the rail, and reading it off the row this statement
        // wrote is the only way it cannot disagree with what was written. The
        // `Ok(false)` case is unchanged — no row matched, nothing moved, and
        // nothing is counted.
        let Some(row) = moved else {
            return Ok(false);
        };
        let provider_code: String = row.try_get("provider_code").map_err(DbError::Query)?;
        record_transition(&provider_code, expected, new);

        Ok(true)
    }

    async fn latest_submit_attempt(&self, charge_id: &str) -> Result<Option<AttemptRow>, DbError> {
        sqlx::query_as::<_, AttemptRow>(
            "SELECT id, charge_id, provider_code, provider_reference_id, attempt, status_code, \
         error_kind, sent_at, responded_at \
         FROM provider_requests \
         WHERE charge_id = $1 AND operation = 'submit' \
         ORDER BY sent_at DESC, id DESC \
         LIMIT 1",
        )
        .bind(charge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn live_charges_stale_since(
        &self,
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
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}
