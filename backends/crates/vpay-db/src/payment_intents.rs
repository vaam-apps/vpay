//! The `payment_intents` repository (`backends/migrations/
//! 0003_create-payment-intents.sql`, reshaped by `0014_payment-intent-api-
//! fields.sql`) — the reads and writes behind `/v1/payment_intents`.
//!
//! # Two rules this module exists to keep
//!
//! **Every query is merchant-scoped in SQL, not in Rust.** There is no
//! `get(pool, id)`: the merchant is a parameter of the lookup itself, so a
//! handler cannot forget to filter and cannot leak another merchant's
//! object by reading first and comparing afterwards. A foreign id therefore
//! comes back as `None` — indistinguishable from a missing one, which is
//! what `docs/flows/merchant-auth.md` requires (an authorisation failure
//! that answers differently from a missing object is an existence oracle).
//!
//! **Status changes are compare-and-swap, never read-then-write.**
//! [`transition`] carries the expected status into the `UPDATE`'s own
//! `WHERE`, so two concurrent requests cannot both observe
//! `requires_payment_method` and both act on it. A validation function that
//! is not part of the write statement enforces nothing under concurrency;
//! this one is the write statement.

use sqlx::PgPool;
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};

/// Every column of `payment_intents`, in one place so the four queries
/// below cannot drift on the shape they decode into [`PaymentIntentRow`].
///
/// `status` and `last_payment_error_code` are cast to `TEXT`: both are
/// native Postgres enums, and this crate carries them as `String` (D4 of
/// this step's design — `vpay-core` owns parsing them into
/// `IntentStatus`/`FailureCode`). Without the cast `sqlx` refuses to decode
/// a user-defined type into `String` at runtime, which is a failure this
/// crate would only discover against a real database.
const COLUMNS: &str = "id, seq, merchant_id, livemode, amount, amount_received, amount_refunded, \
                       amount_refund_pending, currency_code, status::TEXT AS status, \
                       last_payment_error_code::TEXT AS last_payment_error_code, \
                       last_payment_error_message, payment_method_types, metadata, description, \
                       created_at, updated_at";

/// One `payment_intents` row, exactly as stored.
///
/// Not the wire object: `vpay-api` owns that shape (lowercase currency,
/// unix-seconds `created`, the nested `last_payment_error` object). This
/// struct is deliberately one-to-one with the table so a change to either
/// is a compile error rather than a silently dropped column.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PaymentIntentRow {
    /// Public `pi_…` id, supplied by the caller before the insert.
    pub id: String,
    /// Pagination order (migration 0014). Database-generated, never
    /// written by this crate, and never exposed on the wire.
    pub seq: i64,
    /// The owning merchant. Every query in this module filters on it.
    pub merchant_id: String,
    /// Live or test money. Fixed at creation from the deployment's own
    /// configuration and never updated.
    pub livemode: bool,
    /// Integer minor units (`docs/flows/money.md`), never a float.
    pub amount: i64,
    /// How much of `amount` has actually been captured.
    pub amount_received: i64,
    /// Refunded to date, in the same minor units.
    pub amount_refunded: i64,
    /// Refunds submitted but not yet settled. `amount_refunded +
    /// amount_refund_pending <= amount` is a database CHECK (`0003`).
    pub amount_refund_pending: i64,
    /// ISO-4217 code, uppercase as stored (`vpay-api` lowercases it on the
    /// wire, Stripe-style).
    pub currency_code: String,
    /// `intent_status` as text. `String`, not an enum, per D4.
    pub status: String,
    /// Closed failure vocabulary as text, or `None`. Paired with
    /// `last_payment_error_message` by the `lpe_paired` CHECK.
    pub last_payment_error_code: Option<String>,
    /// The rail's own text for the last failure, truncated to 512 chars by
    /// the database.
    pub last_payment_error_message: Option<String>,
    /// JSON array of provider codes the merchant asked for.
    pub payment_method_types: serde_json::Value,
    /// Merchant metadata, always a JSON object (`metadata_is_object`).
    pub metadata: serde_json::Value,
    /// Merchant description, at most 1000 characters.
    pub description: Option<String>,
    /// When the intent was created, as supplied to [`insert`].
    pub created_at: OffsetDateTime,
    /// When the row last changed. Maintained by [`transition`], not by a
    /// trigger — see migration 0014's closing note.
    pub updated_at: OffsetDateTime,
}

/// The columns a caller supplies when creating an intent: [`PaymentIntentRow`]
/// minus `seq`, `updated_at` and the three `amount_*` running totals, all of
/// which the database or a later transition owns.
///
/// `created_at` **is** a field here, and that is a deliberate reading of
/// this step's design (which removes only `seq`, `updated_at` and
/// `amount_*`): the creating request stamps the instant, so an intent
/// created inside a transaction that later retries does not silently change
/// its `created` timestamp between attempts. The column also has a `now()`
/// default, so a future writer that has no meaningful instant of its own
/// can be given one — but this struct does not let a caller forget to
/// decide.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPaymentIntent {
    /// Public `pi_…` id, generated by `vpay_core::ids` before the insert —
    /// never by the database, so a crash mid-insert still leaves a name to
    /// reconcile by (`docs/flows/crash-safety.md`).
    pub id: String,
    /// The owning merchant, from the authenticated client's mapping.
    pub merchant_id: String,
    /// From `config.deployment.livemode`; never inferred per request.
    pub livemode: bool,
    /// Integer minor units.
    pub amount: i64,
    /// ISO-4217 code, uppercase. Must exist in `currencies` or the insert
    /// fails as [`DbError::ForeignKeyViolation`].
    pub currency_code: String,
    /// Initial status — `requires_payment_method` for every intent vpay
    /// creates today. Taken as a parameter rather than hard-coded so the
    /// state machine stays in `vpay-core` (`Transition::Create`) instead of
    /// being duplicated as a string literal in the persistence layer.
    pub status: String,
    /// Present only if an intent is created already carrying a failure.
    /// Nothing does that today; the field exists because the column and its
    /// pairing CHECK do.
    pub last_payment_error_code: Option<String>,
    /// The message half of the pair above. Both or neither.
    pub last_payment_error_message: Option<String>,
    /// JSON array of provider codes.
    pub payment_method_types: serde_json::Value,
    /// JSON object of merchant metadata.
    pub metadata: serde_json::Value,
    /// Merchant description.
    pub description: Option<String>,
    /// Creation instant — see the struct's own comment for why the caller
    /// supplies it.
    pub created_at: OffsetDateTime,
}

/// One page request for [`list_page`].
///
/// Cursors are public object ids (`pi_…`), not `seq` values: `seq` is an
/// internal counter, and handing it out would both leak how many intents
/// vpay has ever created and let a merchant walk another merchant's range
/// by arithmetic. The id is resolved to a `seq` by a merchant-scoped
/// subquery inside the same statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    /// How many rows the caller wants. `vpay-api` applies the product
    /// limits (default 10, ceiling 100); this layer only refuses a
    /// non-positive limit, which Postgres would reject outright.
    pub limit: i64,
    /// Return intents strictly *older* than this id (the forward
    /// direction; the default page has neither cursor).
    pub starting_after: Option<String>,
    /// Return intents strictly *newer* than this id — the backward
    /// direction, which scans ascending and is reversed in Rust so `data`
    /// is newest-first either way (D8).
    pub ending_before: Option<String>,
}

/// Inserts a new intent and returns the row the database actually stored —
/// including the columns it filled in itself (`seq`, the `amount_*` totals,
/// `updated_at`), so a caller never has to re-read to render its response.
///
/// # Errors
///
/// [`DbError::ForeignKeyViolation`] if `currency_code` is not in
/// `currencies` (a merchant naming a currency this deployment does not
/// know), [`DbError::UniqueViolation`] if `id` is already taken, and
/// [`DbError::Query`] for anything else — including a `status` that is not
/// a member of the `intent_status` enum, which is a vpay bug rather than a
/// caller error.
pub async fn insert(pool: &PgPool, new: &NewPaymentIntent) -> Result<PaymentIntentRow, DbError> {
    let sql = format!(
        "INSERT INTO payment_intents (id, merchant_id, livemode, amount, currency_code, status, \
         last_payment_error_code, last_payment_error_message, payment_method_types, metadata, \
         description, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6::intent_status, $7::failure_code, $8, $9, $10, $11, $12) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(&sql)
        .bind(&new.id)
        .bind(&new.merchant_id)
        .bind(new.livemode)
        .bind(new.amount)
        .bind(&new.currency_code)
        .bind(&new.status)
        .bind(new.last_payment_error_code.as_deref())
        .bind(new.last_payment_error_message.as_deref())
        .bind(&new.payment_method_types)
        .bind(&new.metadata)
        .bind(new.description.as_deref())
        .bind(new.created_at)
        .fetch_one(pool)
        .await
        .map_err(classify_write)
}

/// Reads one intent *for this merchant*. `None` means "no such intent for
/// you", which covers both a missing id and another merchant's id — see the
/// module comment for why those two must be indistinguishable.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn get_for_merchant(
    pool: &PgPool,
    merchant_id: &str,
    id: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM payment_intents WHERE merchant_id = $1 AND id = $2");

    sqlx::query_as::<_, PaymentIntentRow>(&sql)
        .bind(merchant_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(DbError::Query)
}

/// One page of this merchant's intents, newest first, plus whether more
/// exist beyond it.
///
/// # Ordering and cursors (D8)
///
/// `data` is **always** newest-first, in both directions, because that is
/// what the list envelope promises regardless of which cursor a client
/// used. Forward paging (`starting_after`, or no cursor at all) scans
/// `seq DESC` and returns the scan order as-is. Backward paging
/// (`ending_before`) has to scan `seq ASC` — otherwise "the ten rows
/// immediately newer than this one" would be a `LIMIT` taken from the wrong
/// end of the range — and is reversed in Rust before it is returned.
///
/// `has_more` is computed by asking for one row more than the caller wanted
/// and checking whether it arrived; the extra row is dropped. It therefore
/// means "there are further rows *in the direction of travel*", which on
/// the last page is `false` without a second count query.
///
/// Both cursors are applied when both are given, even though the boundary
/// refuses that combination before it reaches here
/// (`vpay_api::v1::payment_intents::list`, `400` on `starting_after`): a
/// repository that silently ignored one of its arguments would return a page
/// that is wrong in a way no error reports. The direction of travel is
/// chosen by `ending_before`.
///
/// An unknown, deleted or foreign cursor id resolves to `NULL` and every
/// comparison against it is `NULL`, so the page comes back **empty** rather
/// than falling back to the newest rows. That is the safe direction — the
/// merchant-scoped subquery is also what stops one merchant's cursor from
/// positioning a scan inside another merchant's range — but it does mean a
/// cursor that resolves to nothing looks like the end of the list. Which is
/// why the same boundary checks the cursor's *shape* first
/// (`vpay_core::ids::is_well_formed`), so a typo is a `400` and only a
/// well-formed id that names no row of this merchant's reaches this silence.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn list_page(
    pool: &PgPool,
    merchant_id: &str,
    page: &ListPage,
) -> Result<(Vec<PaymentIntentRow>, bool), DbError> {
    // Postgres rejects a negative LIMIT outright, and a zero-row page is
    // never what a caller means. `vpay-api` owns the real ceiling.
    let limit = page.limit.max(1);
    let backwards = page.ending_before.is_some();
    let direction = if backwards { "ASC" } else { "DESC" };

    // Cursors are ids; the subqueries turn them into `seq` values without a
    // second round trip, and both are scoped to the same merchant as the
    // outer query so a cursor from elsewhere resolves to NULL rather than
    // to a position in someone else's range.
    let sql = format!(
        "SELECT {COLUMNS} FROM payment_intents \
         WHERE merchant_id = $1 \
           AND ($2::TEXT IS NULL \
                OR seq < (SELECT seq FROM payment_intents WHERE id = $2 AND merchant_id = $1)) \
           AND ($3::TEXT IS NULL \
                OR seq > (SELECT seq FROM payment_intents WHERE id = $3 AND merchant_id = $1)) \
         ORDER BY seq {direction} \
         LIMIT $4"
    );

    let mut rows = sqlx::query_as::<_, PaymentIntentRow>(&sql)
        .bind(merchant_id)
        .bind(page.starting_after.as_deref())
        .bind(page.ending_before.as_deref())
        .bind(limit.saturating_add(1))
        .fetch_all(pool)
        .await
        .map_err(DbError::Query)?;

    let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    if has_more {
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    if backwards {
        rows.reverse();
    }

    Ok((rows, has_more))
}

/// Moves an intent from `expected` to `new`, atomically, and returns the
/// row as it now stands.
///
/// `Ok(None)` means the compare-and-swap did not fire: no such intent for
/// this merchant, **or** its status was not `expected` any more. The two
/// are deliberately one answer here — distinguishing them needs a second
/// read, and the caller that cares (a handler choosing between `404` and
/// `409`) is the one that should decide how much it is willing to reveal.
///
/// The state machine itself lives in `vpay_core::state`; this function only
/// applies a transition that machine has already approved. Passing a
/// `new` status the machine would not allow is a vpay bug, not something
/// this layer second-guesses — but note that the *guard* is real either
/// way: no concurrent writer can slip a different status in between the
/// check and the write, because there is no gap.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails, including when
/// `expected` or `new` is not a member of the `intent_status` enum.
pub async fn transition(
    pool: &PgPool,
    merchant_id: &str,
    id: &str,
    expected: &str,
    new: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    transition_with(pool, merchant_id, id, expected, new).await
}

/// [`transition`], inside a transaction the caller owns.
///
/// Exists because a confirm that a rail accepted has **two** rows to move —
/// the charge out of `submitting` and the intent into
/// `processing`/`requires_action` — and a merchant must never be able to
/// observe one without the other. `docs/flows/crash-safety.md`'s
/// "the commit is the gate on the redirect" is a statement about a single
/// commit; two pooled statements would leave a window in which the intent
/// says `requires_action` while the charge carries no `redirect_url`, and
/// `GET /v1/payment_intents/{id}` would render a `next_action` with no URL
/// in it.
///
/// Same compare-and-swap, same `Ok(None)` meaning, as [`transition`] — the
/// guard is in the statement, so it holds inside a transaction exactly as it
/// does outside one.
///
/// # Errors
///
/// As [`transition`].
pub async fn transition_in_tx(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
    expected: &str,
    new: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    transition_with(&mut *tx, merchant_id, id, expected, new).await
}

/// The one statement behind [`transition`] and [`transition_in_tx`], generic
/// over where it runs so the two cannot drift on their guard.
async fn transition_with<'e, E>(
    executor: E,
    merchant_id: &str,
    id: &str,
    expected: &str,
    new: &str,
) -> Result<Option<PaymentIntentRow>, DbError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "UPDATE payment_intents SET status = $4::intent_status, updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 AND status = $3::intent_status \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(&sql)
        .bind(merchant_id)
        .bind(id)
        .bind(expected)
        .bind(new)
        .fetch_optional(executor)
        .await
        .map_err(classify_write)
}

/// Records why the last charge on this intent was refused, **without**
/// moving its status.
///
/// # Why this is not a `transition`
///
/// `docs/flows/payment-lifecycle.md` has no `failed` status: "a rail-reported
/// failure ... returns the intent to `requires_payment_method` with
/// `last_payment_error` populated". A decline at submit never left that
/// status in the first place, so there is nothing to move — the whole write
/// is the error pair. Routing it through [`transition`] with
/// `expected == new` would read as a state change that is deliberately not
/// one, and would have to pass the same label twice.
///
/// The status is still in the `WHERE`, for the reason every write in this
/// module carries its guard: between the rail's answer and this statement, a
/// cancel may have moved the intent, and stamping a payment error onto a
/// `canceled` intent would tell a merchant a payment they withdrew was
/// declined.
///
/// Both halves are written together because the `lpe_paired` CHECK
/// (migration 0014) refuses a code without a message. The caller supplies
/// the message already bounded to the column's 512 characters.
///
/// `Ok(None)` means the guard refused: no such intent for this merchant, or
/// its status is no longer `expected`.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails, including a `code` outside the
/// `failure_code` enum — which is a vpay bug, since the vocabulary is closed
/// and owned by `vpay_core::FailureCode`.
pub async fn record_payment_error(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
    expected: &str,
    code: &str,
    message: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents \
         SET last_payment_error_code = $4::failure_code, \
             last_payment_error_message = $5, \
             updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 AND status = $3::intent_status \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(&sql)
        .bind(merchant_id)
        .bind(id)
        .bind(expected)
        .bind(code)
        .bind(message)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

/// The `charge_state` labels a charge is in while the rail may still act on
/// it — the four non-terminal members of the enum created by migration 0004,
/// and exactly the set the partial index `charges_live_idx` (migration 0014)
/// is built over, so the `NOT EXISTS` in [`cancel`] is an index lookup.
///
/// Spelled as SQL text rather than built from `vpay_core::ChargeState`
/// because this crate carries Postgres enums as `String` (D4) and the list
/// has to appear inside a statement; the migration and this constant are the
/// two places it is written, and `charges_live_idx` is what ties them.
const LIVE_CHARGE_STATES: &str = "'submitting', 'submitted', 'pending', 'unresolved'";

/// Cancels an intent that is still `requires_payment_method` **and** has no
/// charge the rail may still be acting on.
///
/// # Why the charge check is inside the statement
///
/// `requires_payment_method` is not on its own enough to make a cancel safe.
/// A `confirm` commits its charge row — carrying the
/// `provider_reference_id` it is about to submit under — *before* it calls
/// the rail, and leaves the intent's status alone until it knows what
/// happened (`docs/flows/crash-safety.md`). So there is a real, reachable
/// window in which the status still says `requires_payment_method` while a
/// live charge exists; today it is not even a window but a resting state,
/// because no adapter implements `submit` and a confirm's `501` leaves that
/// `submitting` charge behind on purpose.
///
/// Cancelling there would tell a merchant the payment was withdrawn while
/// the rail may hold it. A check in the caller would not fix it: between
/// reading "no charge" and writing `canceled`, a concurrent confirm can
/// commit one. Only the write statement can decide this, which is why the
/// `NOT EXISTS` is a predicate of the `UPDATE` and not a preceding `SELECT`.
///
/// Charges in a terminal state (`succeeded`, `failed`) do not block the
/// cancel: nothing is in flight, and "one charge per intent, forever" means
/// the intent cannot get another. The four live labels are named in
/// `LIVE_CHARGE_STATES`, next to this function.
///
/// `Ok(None)` therefore carries **three** meanings now — no such intent for
/// this merchant, a status this transition is not legal from, or a live
/// charge — and a caller that needs to tell them apart re-reads with
/// [`get_for_merchant`] and `crate::charges::get_for_intent`. That is
/// `vpay_api::v1::payment_intents::cancel_once`, which turns them into a
/// `404` and two different `409`s.
///
/// The pair (`requires_payment_method` → `canceled`) mirrors
/// `vpay_core::state`'s `Transition::Cancel`; the two must agree, and the
/// integration test `cancel_is_legal_only_from_requires_payment_method`
/// is what proves they do end to end, with
/// `a_confirmed_intent_cannot_be_canceled` covering the live-charge half.
///
/// # Errors
///
/// As [`transition`].
pub async fn cancel(
    pool: &PgPool,
    merchant_id: &str,
    id: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents SET status = 'canceled'::intent_status, updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 \
           AND status = 'requires_payment_method'::intent_status \
           AND NOT EXISTS (SELECT 1 FROM charges \
                           WHERE charges.payment_intent_id = payment_intents.id \
                             AND charges.state IN ({LIVE_CHARGE_STATES})) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(&sql)
        .bind(merchant_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(classify_write)
}
