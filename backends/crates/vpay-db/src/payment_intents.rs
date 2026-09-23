//! The `payment_intents` repository (`backends/migrations/
//! 0003_create-payment-intents.sql`, reshaped by `0014_payment-intent-api-
//! fields.sql`) — the reads and writes behind `/v1/payment_intents`.
//!
//! Two rules this module exists to keep: **every query is merchant-scoped in
//! SQL**, so a handler cannot forget to filter and a foreign id is
//! indistinguishable from a missing one; and **status changes are
//! compare-and-swap**, so [`PaymentIntents::transition`] *is* the check rather
//! than validating beside it. `docs/reference/vpay-db.md`
//! §"`payment_intents`" says what each rule prevents.

use std::fmt;

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::AssertSqlSafe;
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};

/// Every column of `payment_intents`, in one place so the four queries
/// below cannot drift on the shape they decode into [`PaymentIntentRow`].
///
/// `status` and `last_payment_error_code` are selected without a cast, and
/// that is what migration `0037` changed: both were native Postgres enums
/// (`intent_status`, `failure_code`) and every statement here had to spell
/// `status::TEXT AS status`, because `sqlx` refuses to decode a
/// user-defined type into `String`. They are `TEXT` + a membership CHECK
/// now, so the cast is a no-op and saying it would suggest a type that no
/// longer exists. What has *not* changed is that this crate carries the
/// vocabularies as `String` (D4 of Step 2's design — `vpay-core` owns
/// parsing them into `IntentStatus`/`FailureCode`).
const COLUMNS: &str = "id, seq, merchant_id, livemode, amount, amount_received, amount_refunded, \
                       amount_refund_pending, currency_code, status, last_payment_error_code, \
                       last_payment_error_message, payment_method_types, metadata, description, \
                       customer_id, client_secret_suffix, created_at, updated_at";

/// One `payment_intents` row, exactly as stored.
///
/// Not the wire object: `vpay-api` owns that shape (lowercase currency,
/// unix-seconds `created`, the nested `last_payment_error` object). This
/// struct is deliberately one-to-one with the table so a change to either
/// is a compile error rather than a silently dropped column.
///
/// `Debug` is **hand-written** below rather than derived, because
/// [`Self::client_secret_suffix`] is a live credential — see that impl.
#[derive(Clone, PartialEq, sqlx::FromRow)]
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
    /// The intent's status label. `String`, not an enum, per D4; the
    /// vocabulary is closed by `payment_intents_status_enum_check`
    /// (migration `0037`, which replaced the `intent_status` type).
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
    /// The customer this intent is for, or `None` (migration `0034`).
    ///
    /// A `cus_…` with a real foreign key onto `customers`, `NO ACTION` on
    /// delete — so a payment can never be detached from the payer it was
    /// taken from, and a customer with any payment history cannot be
    /// deleted. `vpay_api::v1::customers::delete` turns that refusal into a
    /// `409` that says so.
    ///
    /// The `customer` request field was accepted and **dropped** from Step 5b
    /// until 0034; `docs/api/README.md` said so, and this column is what
    /// stopped it being true.
    pub customer_id: Option<String>,
    /// The stored half of this intent's payer-facing `client_secret`
    /// (migration `0026`). Join it to [`Self::id`] with
    /// `vpay_core::ids::client_secret` — never by hand.
    ///
    /// **A credential, not an identifier.** Whoever holds the joined value
    /// can retrieve and *confirm* this intent through `/v1/browser` without
    /// any merchant token, so it is redacted in this struct's `Debug` and
    /// must never be rendered on the `/v1` list or into `events.data`
    /// (`vpay_api::model::PaymentIntentWithSecret`).
    pub client_secret_suffix: String,
    /// When the intent was created, as supplied to [`PaymentIntents::insert`].
    pub created_at: OffsetDateTime,
    /// When the row last changed. Maintained by [`PaymentIntents::transition`], not by a
    /// trigger — see migration 0014's closing note.
    pub updated_at: OffsetDateTime,
}

/// Redacts [`PaymentIntentRow::client_secret_suffix`], leaving every other
/// column visible.
///
/// A row is `{:?}`-ed in more places than anyone tracks: a `tracing` field on
/// a settlement, a `Result` unwrapped in a test failure message, a
/// `#[derive(Debug)]` on some future struct that happens to hold one. Every
/// one of those is a place a payer credential would otherwise be written to a
/// log an operator can read — and unlike a webhook secret, this one is
/// directly actionable: it confirms a payment, on a live intent, with no
/// merchant token in the picture.
///
/// The *length* stays, because "is this row's suffix the right shape?" is a
/// question an operator debugging migration `0026`'s CHECK actually asks and
/// answering it needs no secret. Mirrors `vpay_config::oauth::WebhookEndpoint`'s
/// impl in shape and in what it keeps.
///
/// Nothing else is hidden. `merchant_id`, the amounts and the status are what
/// every operator investigation starts from, and hiding them to be thorough
/// would make this impl worse at the job it exists for.
impl fmt::Debug for PaymentIntentRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentIntentRow")
            .field("id", &self.id)
            .field("seq", &self.seq)
            .field("merchant_id", &self.merchant_id)
            .field("livemode", &self.livemode)
            .field("amount", &self.amount)
            .field("amount_received", &self.amount_received)
            .field("amount_refunded", &self.amount_refunded)
            .field("amount_refund_pending", &self.amount_refund_pending)
            .field("currency_code", &self.currency_code)
            .field("status", &self.status)
            .field("last_payment_error_code", &self.last_payment_error_code)
            .field(
                "last_payment_error_message",
                &self.last_payment_error_message,
            )
            .field("payment_method_types", &self.payment_method_types)
            .field("metadata", &self.metadata)
            .field("description", &self.description)
            .field(
                "client_secret_suffix",
                &format_args!("[{} chars redacted]", self.client_secret_suffix.len()),
            )
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
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
    /// The customer this intent is for, or `None`.
    ///
    /// Must exist and belong to [`Self::merchant_id`] — the API resolves it
    /// through `Customers::get_for_merchant` first
    /// (`vpay_api::v1::customers::resolve_for_attachment`), and the foreign
    /// key is the backstop. An id from another tenant therefore fails as
    /// [`DbError::ForeignKeyViolation`] rather than attaching, which is a
    /// vpay bug and not a merchant's.
    pub customer_id: Option<String>,
    /// The payer credential's stored half, from
    /// `vpay_core::ids::client_secret_suffix` — generated by the caller
    /// before the insert, exactly as [`Self::id`] is.
    ///
    /// A field rather than a column default for [`Self::id`]'s reason and
    /// one more: migration `0026`'s backfill shows why a `DEFAULT` is the
    /// wrong tool here (Postgres evaluates an `ADD COLUMN` default once for
    /// the whole table), and a writer that could omit this would be a writer
    /// that can create an intent no browser can ever reach.
    pub client_secret_suffix: String,
    /// Creation instant — see the struct's own comment for why the caller
    /// supplies it.
    pub created_at: OffsetDateTime,
}

/// One page request for [`PaymentIntents::list_page`].
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

/// Extra predicates a list may narrow by, beyond the tenant and the cursor.
///
/// **On `/v1` only [`Self::customer`] is ever set** (RFC-0004 § 5,
/// 2026-09-23), and `/dash/v1`'s payments list is the only caller that sets
/// the other three, where an operator asking "what failed yesterday?" is the
/// ordinary question. `crate::PaymentIntents::list_page` is still
/// `Self::default()`. _(This said "Empty on `/v1`, which offers no filters at
/// all" until 2026-09-23, when `GET /v1/payment_intents?customer=` became the
/// first `/v1` caller.)_
///
/// Its own type rather than three arguments so that adding a fourth
/// predicate is one edit at each end rather than a signature change at
/// every call site — and so the empty case is spelled `Default::default()`
/// and reads as "no filter" rather than as three `None`s in an order
/// nobody can check.
///
/// # Why filtering is here and not in the caller
///
/// A caller cannot filter a *page*. `list_page` fetches `limit + 1` rows to
/// learn `has_more`, and a filter applied to the returned `Vec` would drop
/// rows out of a page that has already been counted — so a page of ten
/// intents with three failures would answer three rows and `has_more:
/// false`, and the next cursor would skip the seven it discarded. The
/// predicate has to be in the statement the `LIMIT` applies to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntentFilter {
    /// One status label, as its wire text
    /// (`vpay_core::IntentStatus::as_wire_str`), compared against the
    /// `status` column directly.
    ///
    /// An unknown value is an empty result rather than an error, which is
    /// the answer a caller naming a status this deployment does not have
    /// should get. That used to need care — the column was the
    /// `intent_status` enum and this predicate deliberately compared
    /// `status::TEXT` rather than casting `$5`, because a cast to the enum
    /// turns an unknown label into a Postgres `invalid input value for
    /// enum` and a `500`. Migration `0037` made the column `TEXT`, so the
    /// property now holds for free. `vpay-api` still refuses an unknown
    /// status before it gets here; that stays a *policy* rather than the
    /// only thing between the query and a `500`.
    pub status: Option<String>,
    /// Only intents created at or after this instant.
    pub created_gte: Option<OffsetDateTime>,
    /// Only intents created at or before this instant.
    pub created_lte: Option<OffsetDateTime>,
    /// Only this customer's intents — `payment_intents.customer_id`, a
    /// `cus_…` — the `customer` filter of `GET /v1/payment_intents`.
    ///
    /// **Not validated for existence**, for
    /// [`crate::InvoiceListPage::customer`]'s reason: it is compared in the
    /// same `WHERE` as `merchant_id`, so another merchant's customer, an
    /// unknown one and one of this merchant's with no intents are all the
    /// same empty page, and the filter cannot become an oracle for which
    /// `cus_…` exist under some other tenant. An erased customer (migration
    /// `0041`) keeps its id and its intents keep pointing at it, so the
    /// filter finds them exactly as before the erasure.
    pub customer: Option<String>,
}

/// [`PaymentIntents::transition`], inside a transaction the caller owns.
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
/// Same compare-and-swap, same `Ok(None)` meaning, as [`PaymentIntents::transition`] — the
/// guard is in the statement, so it holds inside a transaction exactly as it
/// does outside one.
///
/// # Errors
///
/// As [`PaymentIntents::transition`].
pub(crate) async fn transition_in_tx(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
    expected: &str,
    new: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    transition_with(&mut *tx, merchant_id, id, expected, new).await
}

/// Cancels an intent that is still `requires_payment_method` **and** has no
/// charge the rail may still be acting on, **inside the caller's
/// transaction**.
///
/// # Why there is no pooled variant, since 2026-09-10 (issue #57)
///
/// A cancel emits `payment_intent.canceled`, and that event has to commit
/// with the status flip or not at all — the outbox rule this crate applies
/// to every other terminal transition ([`crate::settlement`],
/// [`crate::checkout_sessions::CheckoutSessions::expire_due`],
/// [`crate::Customers::erase_idle`]). There *was* a pooled
/// `PaymentIntents::cancel`, and it was the whole of the bug: the row moved
/// to `canceled` on its own connection and no merchant was ever told, so a
/// shop driven by webhooks could not reach a cancelled state at all. Deleting
/// it rather than leaving it beside this one is what makes "cancel without an
/// event" inexpressible instead of merely discouraged.
///
/// The live-charge check is a `NOT EXISTS` predicate of the `UPDATE` and not
/// a preceding `SELECT`, because a status of `requires_payment_method` is not
/// on its own enough to make a cancel safe and a check in the caller cannot
/// close the window — `docs/reference/vpay-db.md` §"`cancel` checks for a
/// live charge inside the statement". The four live labels are
/// [`LIVE_CHARGE_STATES`], next to this function.
///
/// `Ok(None)` therefore carries three meanings — no such intent for this
/// merchant, an illegal status, or a live charge — and the caller that must
/// tell them apart re-reads. It is also the answer that must write **no
/// event**: see `vpay_api::v1::payment_intents::cancel_once`.
///
/// # Errors
///
/// As [`PaymentIntents::transition`].
pub(crate) async fn cancel_in_tx(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents SET status = 'canceled', updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 \
           AND status = 'requires_payment_method' \
           AND NOT EXISTS (SELECT 1 FROM charges \
                           WHERE charges.payment_intent_id = payment_intents.id \
                             AND charges.state IN ({LIVE_CHARGE_STATES})) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

/// The one statement behind [`PaymentIntents::transition`] and [`crate::TxRepositories::transition_in_tx`], generic
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
        "UPDATE payment_intents SET status = $4, updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 AND status = $3 \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
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
/// is the error pair. Routing it through [`PaymentIntents::transition`] with
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
pub(crate) async fn record_payment_error(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
    expected: &str,
    code: &str,
    message: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents \
         SET last_payment_error_code = $4, \
             last_payment_error_message = $5, \
             updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 AND status = $3 \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .bind(expected)
        .bind(code)
        .bind(message)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

/// The `payment_intents.status` labels a live charge's intent can
/// legitimately be in when a settlement arrives.
///
/// Two of them are the confirmed statuses — a push rail leaves the intent
/// `processing`, a redirect rail leaves it `requires_action`. The third,
/// `requires_payment_method`, is in the set because a crash puts it there and
/// the charge, not the intent, is the record of whether a confirm happened.
///
/// `succeeded` and `canceled` stay out, and that is what keeps the guard a
/// guard: neither can coexist with a live charge, so either one appearing here
/// is a broken invariant that must page rather than settle.
///
/// `docs/reference/vpay-db.md` §"Which intent statuses a settlement may land
/// on" says what excluding `requires_payment_method` used to cost and why
/// including it is safe.
const SETTLEABLE_STATUSES: &str = "'processing', 'requires_action', 'requires_payment_method'";

/// The `lpe_message_length` CHECK's ceiling (migration 0014), in characters.
const LAST_PAYMENT_ERROR_MESSAGE_MAX_CHARS: usize = 512;

/// Settles the intent of a charge the rail reported as paid: any of
/// `SETTLEABLE_STATUSES` → `succeeded`, with `amount_received` set to the
/// full `amount`, in one statement.
///
/// `requires_payment_method` is one of those statuses, so this is also the
/// write that resolves a confirm that crashed before it could move the intent
/// — see `SETTLEABLE_STATUSES` for why that is not a hole in the guard.
///
/// `amount_received = amount` and not a parameter: no rail vpay speaks to can
/// settle *part* of a submitted amount. See `docs/reference/vpay-db.md`
/// §"Which intent statuses a settlement may land on".
///
/// `Ok(None)` means the guard refused: no such intent, or a status outside
/// `SETTLEABLE_STATUSES` — which, for the settlement transaction, leaves only
/// `succeeded` and `canceled`. Both are invariant violations rather than
/// races, and [`crate::settlement`] turns them into
/// [`DbError::WriteMatchedNoRow`] rather than committing half a settlement.
///
/// # Why `pub(crate)`
///
/// The widened guard's safety argument is "this is never called outside the
/// settlement transaction, after a charge compare-and-swap over
/// `LIVE_CHARGE_STATES` has already matched". `pub(crate)` is what makes the
/// compiler enforce that rather than this paragraph: [`crate::settlement`] is
/// the only module that can reach it, and a caller elsewhere — a handler, a
/// test, a future repair script — would have to move a `pub` here in the same
/// diff, which is the moment the argument has to be re-made.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub(crate) async fn succeed_after_submission(
    tx: &mut sqlx::PgConnection,
    id: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents \
         SET status = 'succeeded', \
             amount_received = amount, \
             updated_at = now() \
         WHERE id = $1 AND status IN ({SETTLEABLE_STATUSES}) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

/// Returns the intent of a declined charge to `requires_payment_method`
/// **and** stamps the failure that sent it there, in one statement.
///
/// `requires_payment_method` → `requires_payment_method` is a **real write**:
/// that status is itself in `SETTLEABLE_STATUSES`, so a confirm that crashed
/// before it could move the intent still matches one row and the write is the
/// error pair alone. It sits next to
/// [`crate::TxRepositories::record_payment_error`] because the two are
/// different moments — that one is a decline at submit, this one a decline the
/// *poll* discovered. Neither is merchant-scoped, and `message` is truncated
/// here rather than left to the `lpe_message_length` CHECK.
/// `docs/reference/vpay-db.md` §"Which intent statuses a settlement may land
/// on" carries all four arguments.
///
/// `Ok(None)` means the guard refused, exactly as in
/// [`succeed_after_submission`], and it is `pub(crate)` for the same reason:
/// the guard is only safe because a live charge has already been matched in
/// the same transaction, and visibility is how that stays true.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails, including a `code` outside
/// the `failure_code` enum — a vpay bug, since the vocabulary is closed.
pub(crate) async fn fail_after_submission(
    tx: &mut sqlx::PgConnection,
    id: &str,
    code: &str,
    message: &str,
) -> Result<Option<PaymentIntentRow>, DbError> {
    // Characters, not bytes: the CHECK counts characters, and slicing bytes
    // could split one.
    let bounded: String = message
        .chars()
        .take(LAST_PAYMENT_ERROR_MESSAGE_MAX_CHARS)
        .collect();

    let sql = format!(
        "UPDATE payment_intents \
         SET status = 'requires_payment_method', \
             last_payment_error_code = $2, \
             last_payment_error_message = $3, \
             updated_at = now() \
         WHERE id = $1 AND status IN ({SETTLEABLE_STATUSES}) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(id)
        .bind(code)
        .bind(&bounded)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

/// The intent statuses a refund may be reserved against.
///
/// One, and it is not a simplification. Money can only come back if it went
/// in: `amount_received` is set by [`succeed_after_submission`] and by nothing
/// else, so an intent outside `succeeded` has captured nothing, and a
/// reservation against it would be a promise to return money vpay never took.
/// Written as SQL text rather than derived from `vpay_core::IntentStatus` for
/// `SETTLEABLE_STATUSES`' reason — this crate carries the vocabularies as
/// text (D4).
const REFUNDABLE_STATUSES: &str = "'succeeded'";

/// Migration `0003`'s over-refund CHECK, by name.
///
/// Spelled once so the statement that trips it and the error that reports it
/// cannot name different rules.
const NO_OVER_REFUND: &str = "no_over_refund";

/// Turns the `no_over_refund` refusal into [`DbError::OverRefund`], and
/// leaves every other failure exactly as `classify_write` classified it.
///
/// A local narrowing rather than a new arm in `classify_write`, because the
/// general rule there is right and this is the one documented exception to
/// it: a CHECK violation is a vpay bug *except* this one, which is a
/// merchant's request being refused by the guard `docs/flows/ledger.md`
/// requires to live in the database. See [`DbError::OverRefund`].
///
/// The constraint is matched **by name**, not by SQLSTATE: `payment_intents`
/// carries four other CHECKs on these same columns
/// (`amount_refunded_non_negative` and friends), every one of which is a
/// genuine invariant violation, and folding them in here would hide a vpay
/// bug behind a merchant-facing `409`.
fn classify_reservation(error: sqlx::Error, payment_intent_id: &str) -> DbError {
    let is_over_refund = error
        .as_database_error()
        .is_some_and(|db| db.constraint() == Some(NO_OVER_REFUND));

    if is_over_refund {
        DbError::OverRefund {
            payment_intent_id: payment_intent_id.to_owned(),
            source: error,
        }
    } else {
        classify_write(error)
    }
}

/// Reserves `amount` minor units of this intent's refundable balance, inside
/// the caller's transaction (RFC-0003 § 3).
///
/// # This statement is the over-refund guard, and nothing beside it is
///
/// `amount_refund_pending = amount_refund_pending + $2` is evaluated by
/// Postgres against the row it has just locked, so migration `0003`'s
/// `no_over_refund` CHECK — `amount_refunded + amount_refund_pending <=
/// amount` — sees the *committed* total of every refund that got there first.
/// Two concurrent refunds of an intent with only one refund's worth left
/// therefore serialize on the row lock MVCC already takes, and the second one
/// fails the CHECK. An application `SELECT` followed by a comparison in Rust
/// would let both read the same balance and both pass; `docs/flows/ledger.md`
/// § "When refunds post" is explicit that the guard must be this UPDATE, and
/// `two_concurrent_refunds_race_and_the_database_refuses_the_second` in
/// `postgres_smoke.rs` is the case that proves the reservation reaches it.
///
/// # Merchant-scoped in SQL, unlike the two settlement writes above
///
/// This is the one refund statement a *merchant* drives — the others are
/// driven by a rail's answer about a movement vpay initiated — so it carries
/// the tenant predicate every merchant-facing query in this module carries. A
/// handler cannot forget to filter, and "not yours" is indistinguishable from
/// "no such intent", which is the property that stops the existence of
/// another tenant's intent leaking out of a refund request.
///
/// `Ok(None)` therefore means: not this merchant's, or no such intent, or an
/// intent that is not `succeeded`. All three are a refusal, never a race —
/// an intent that captured nothing has nothing to refund — and it is
/// deliberately not an error, so the caller can answer a merchant rather than
/// page.
///
/// # Why `pub(crate)`
///
/// [`succeed_after_submission`]'s reason. The reservation is only half a
/// refund — the `refunds` row is the other half — and the two must reach one
/// commit or an intent carries a reservation for a refund that does not
/// exist. Visibility is what keeps [`crate::refunds`] the only caller.
///
/// # Errors
///
/// [`DbError::OverRefund`] when the CHECK refuses the reservation.
/// [`DbError::Query`] if the statement fails for any other reason.
pub(crate) async fn reserve_refund_in_tx(
    conn: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
    amount: i64,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents \
         SET amount_refund_pending = amount_refund_pending + $3, \
             updated_at = now() \
         WHERE merchant_id = $1 AND id = $2 AND status IN ({REFUNDABLE_STATUSES}) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .bind(amount)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|error| classify_reservation(error, id))
}

/// Turns a reservation into a settled refund: `amount_refund_pending` down by
/// `amount`, `amount_refunded` up by the same, in one statement.
///
/// The pair moves together because `no_over_refund` reads their sum: a split
/// into two statements would pass through a moment where the sum is short by
/// `amount`, which is harmless, and the reverse ordering would pass through
/// one where it is over, which is not — the CHECK would abort the settlement
/// transaction for a refund that is entirely legitimate.
///
/// # The guard is `amount_refund_pending >= $2`
///
/// Not a read-then-subtract, and not a bare decrement trusting
/// `amount_refund_pending_non_negative` to catch the bad case. A bare
/// decrement that went negative would be a `23514`, and a failed statement
/// **aborts the whole transaction** — Postgres then turns the following
/// `COMMIT` into a silent `ROLLBACK`, which
/// `swallowing_a_duplicate_posting_inside_a_transaction_discards_the_whole_transaction`
/// in `postgres_smoke.rs` measures. As a `WHERE` clause the same condition is
/// `Ok(None)`, which the caller can act on with its transaction still alive.
///
/// `Ok(None)` therefore means the intent has no such reservation outstanding,
/// which is a broken invariant rather than a race — [`crate::settlement`]
/// reaches this only after the refund row's own compare-and-swap out of
/// `pending` has matched — and the caller turns it into
/// [`DbError::WriteMatchedNoRow`].
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails.
pub(crate) async fn settle_refund_in_tx(
    conn: &mut sqlx::PgConnection,
    id: &str,
    amount: i64,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents \
         SET amount_refund_pending = amount_refund_pending - $2, \
             amount_refunded = amount_refunded + $2, \
             updated_at = now() \
         WHERE id = $1 AND amount_refund_pending >= $2 \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(id)
        .bind(amount)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// Gives a reservation back: `amount_refund_pending` down by `amount`,
/// `amount_refunded` untouched.
///
/// The failed and canceled halves of [`settle_refund_in_tx`], and the reason
/// `amount_refund_pending` is a column at all. Nothing was posted to the
/// ledger for a refund that never succeeded, so there is no reversal entry to
/// write — `docs/flows/ledger.md` § "When refunds post" calls that out as the
/// point of reserving rather than posting optimistically and unwinding.
///
/// Guard, `Ok(None)` and errors are [`settle_refund_in_tx`]'s, unchanged.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails.
pub(crate) async fn release_refund_in_tx(
    conn: &mut sqlx::PgConnection,
    id: &str,
    amount: i64,
) -> Result<Option<PaymentIntentRow>, DbError> {
    let sql = format!(
        "UPDATE payment_intents \
         SET amount_refund_pending = amount_refund_pending - $2, \
             updated_at = now() \
         WHERE id = $1 AND amount_refund_pending >= $2 \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
        .bind(id)
        .bind(amount)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// The `charges.state` labels a charge is in while the rail may still act on
/// it — the four non-terminal members of the vocabulary migration 0004
/// created as the `charge_state` enum and migration `0037` re-closed as
/// `charges_state_enum_check`, and exactly the set the partial index
/// `charges_live_idx` (migration 0014, rebuilt unchanged by `0037`) is built
/// over, so the `NOT EXISTS` in [`cancel_in_tx`] is an index lookup.
///
/// Spelled as SQL text rather than built from `vpay_core::ChargeState`
/// because this crate carries the vocabularies as `String` (D4) and the list
/// has to appear inside a statement; `0004`'s CHECK, `0014`'s index and this
/// constant are the three places it is written, and `charges_live_idx` is
/// what ties the last two.
///
/// `pub(crate)` because [`crate::settlement`] guards its charge
/// compare-and-swaps on the same set — a settlement may only move a charge
/// the rail could still have been acting on — and two copies of this list
/// could drift, which would either let a settled charge be settled twice or
/// stop a cancel from seeing a live one.
pub(crate) const LIVE_CHARGE_STATES: &str = "'submitting', 'submitted', 'pending', 'unresolved'";

#[async_trait::async_trait]
pub trait PaymentIntents: Send + Sync {
    /// Inserts a new intent and returns the row the database actually stored —
    /// including the columns it filled in itself (`seq`, the `amount_*` totals,
    /// `updated_at`), so a caller never has to re-read to render its response.
    ///
    /// # Errors
    ///
    /// [`DbError::ForeignKeyViolation`] if `currency_code` is not in
    /// `currencies` (a merchant naming a currency this deployment does not
    /// know), [`DbError::UniqueViolation`] if `id` is already taken, and
    /// [`DbError::Query`] for anything else — including a `status` outside
    /// the vocabulary `payment_intents_status_enum_check` closes, which is a
    /// vpay bug rather than a caller error.
    async fn insert(&self, new: &NewPaymentIntent) -> Result<PaymentIntentRow, DbError>;

    /// Reads one intent *for this merchant*. `None` means "no such intent for
    /// you", which covers both a missing id and another merchant's id — see the
    /// module comment for why those two must be indistinguishable.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<PaymentIntentRow>, DbError>;

    /// Reads one intent by its own id, with no merchant scope.
    ///
    /// # Why this exists in a module whose whole rule is "merchant-scoped in SQL"
    ///
    /// [`PaymentIntents::get_for_merchant`] is the *handler's* read, and the scope is what stops
    /// a handler leaking another merchant's object. This one is the **worker's**
    /// read: it is reached from `charges.payment_intent_id`, a foreign key, so
    /// the caller already holds the row that names it and there is no request
    /// whose authorisation could be checked. Taking a `merchant_id` here would
    /// have to be a value the worker looked up from the very intent it is about
    /// to read — an authorisation check against itself, which reads as a
    /// guarantee while providing none.
    ///
    /// The worker needs it because the settlement transaction takes the event's
    /// wire object as an *input* (`docs/flows/webhooks.md`: the event is a
    /// snapshot of the object as it was when the transition happened), so the
    /// object has to be rendered before the write rather than from its result.
    ///
    /// **Not for use in a `/v1` handler.** A handler with this function and a
    /// merchant id in scope will eventually compare them in Rust, which is the
    /// read-then-compare this module exists to make impossible.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_by_id(&self, id: &str) -> Result<Option<PaymentIntentRow>, DbError>;

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
    async fn list_page(
        &self,
        merchant_id: &str,
        page: &ListPage,
    ) -> Result<(Vec<PaymentIntentRow>, bool), DbError>;

    /// [`Self::list_page`] with extra predicates — the `/dash/v1` payments
    /// list.
    ///
    /// The same tenancy filter and the same cursor semantics; see
    /// [`IntentFilter`] for why the predicates cannot be applied to the
    /// returned page instead. `list_page` is this function with
    /// [`IntentFilter::default`], not a second statement: two lists over one
    /// table that disagreed about the cursor would be a paging bug visible
    /// only on one surface.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn list_page_filtered(
        &self,
        merchant_id: &str,
        page: &ListPage,
        filter: &IntentFilter,
    ) -> Result<(Vec<PaymentIntentRow>, bool), DbError>;

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
    /// Returns [`DbError::Query`] if the write fails, including a `new`
    /// outside the vocabulary `payment_intents_status_enum_check` closes —
    /// a `23514`, which `classify_write` leaves as `Query` exactly as it
    /// left the `22P02` the `intent_status` enum used to raise.
    ///
    /// **One behaviour did change with migration `0037`, and it is not
    /// visible in this signature.** `expected` used to be bound as
    /// `$3::intent_status`, so a label outside the vocabulary was a
    /// Postgres error and reached a caller as a `500`. It is a plain `TEXT`
    /// comparison now, so it matches no row and answers `Ok(None)` — the
    /// same answer a genuine lost race gives. Every caller passes a label
    /// from `vpay_core`'s state machine, so the difference is between two
    /// spellings of "a vpay bug"; it is recorded because a `409` is quieter
    /// than a `500` and nothing else would say so.
    async fn transition(
        &self,
        merchant_id: &str,
        id: &str,
        expected: &str,
        new: &str,
    ) -> Result<Option<PaymentIntentRow>, DbError>;
}

#[async_trait::async_trait]
impl PaymentIntents for crate::repository::PgRepositories {
    async fn insert(&self, new: &NewPaymentIntent) -> Result<PaymentIntentRow, DbError> {
        let sql = format!(
            "INSERT INTO payment_intents (id, merchant_id, livemode, amount, currency_code, status, \
         last_payment_error_code, last_payment_error_message, payment_method_types, metadata, \
         description, customer_id, client_secret_suffix, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
         $13, $14) \
         RETURNING {COLUMNS}"
        );

        sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
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
            .bind(new.customer_id.as_deref())
            .bind(&new.client_secret_suffix)
            .bind(new.created_at)
            .fetch_one(&self.pool)
            .await
            .map_err(classify_write)
    }

    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<PaymentIntentRow>, DbError> {
        let sql =
            format!("SELECT {COLUMNS} FROM payment_intents WHERE merchant_id = $1 AND id = $2");

        sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<PaymentIntentRow>, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM payment_intents WHERE id = $1");

        sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn list_page(
        &self,
        merchant_id: &str,
        page: &ListPage,
    ) -> Result<(Vec<PaymentIntentRow>, bool), DbError> {
        self.list_page_filtered(merchant_id, page, &IntentFilter::default())
            .await
    }

    async fn list_page_filtered(
        &self,
        merchant_id: &str,
        page: &ListPage,
        filter: &IntentFilter,
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
        //
        // The four filter predicates are `$N IS NULL OR …`, not string
        // interpolation: an absent filter must produce the *same statement*
        // whichever surface sends it, so Postgres plans one query rather
        // than sixteen, and so a filter value can never reach the SQL text.
        // `status` is a plain column comparison since migration 0037 made
        // the column `TEXT`; see `IntentFilter::status` for what it cost
        // before.
        //
        // The cursor sub-selects are deliberately *not* narrowed by the
        // `customer` filter: a cursor is resolved to its position in this
        // merchant's list, whichever customer it belongs to, and the filtered
        // rows beyond that position are the page — `invoices::list_page`'s
        // behaviour for its own `customer` filter, kept identical so the two
        // lists answer the same cursor the same way.
        let sql = format!(
            "SELECT {COLUMNS} FROM payment_intents \
         WHERE merchant_id = $1 \
           AND ($2::TEXT IS NULL \
                OR seq < (SELECT seq FROM payment_intents WHERE id = $2 AND merchant_id = $1)) \
           AND ($3::TEXT IS NULL \
                OR seq > (SELECT seq FROM payment_intents WHERE id = $3 AND merchant_id = $1)) \
           AND ($5::TEXT IS NULL OR status = $5) \
           AND ($6::TIMESTAMPTZ IS NULL OR created_at >= $6) \
           AND ($7::TIMESTAMPTZ IS NULL OR created_at <= $7) \
           AND ($8::TEXT IS NULL OR customer_id = $8) \
         ORDER BY seq {direction} \
         LIMIT $4"
        );

        let mut rows = sqlx::query_as::<_, PaymentIntentRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(page.starting_after.as_deref())
            .bind(page.ending_before.as_deref())
            .bind(limit.saturating_add(1))
            .bind(filter.status.as_deref())
            .bind(filter.created_gte)
            .bind(filter.created_lte)
            .bind(filter.customer.as_deref())
            .fetch_all(&self.pool)
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

    async fn transition(
        &self,
        merchant_id: &str,
        id: &str,
        expected: &str,
        new: &str,
    ) -> Result<Option<PaymentIntentRow>, DbError> {
        transition_with(&self.pool, merchant_id, id, expected, new).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row whose secret suffix is a value no other test could produce, so
    /// the assertion below cannot pass by the string simply being absent.
    fn row() -> PaymentIntentRow {
        PaymentIntentRow {
            id: "pi_00000000000000000000000x".to_owned(),
            seq: 1,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            amount: 5000,
            amount_received: 0,
            amount_refunded: 0,
            amount_refund_pending: 0,
            currency_code: "XAF".to_owned(),
            status: "requires_payment_method".to_owned(),
            last_payment_error_code: None,
            last_payment_error_message: None,
            payment_method_types: serde_json::json!(["mtn_momo"]),
            metadata: serde_json::json!({}),
            description: None,
            customer_id: None,
            client_secret_suffix: "neverlogthispayercredential00000".to_owned(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// The suffix is a live payer credential: whoever reads it out of a log
    /// can confirm the payment through `/v1/browser` with no merchant token.
    /// A derived `Debug` would put it in every `tracing` field, every test
    /// failure message and every future struct that happens to hold a row.
    ///
    /// Decisive: replacing the hand-written impl with `#[derive(Debug)]`
    /// fails this test on its first assertion.
    #[test]
    fn a_payment_intent_rows_debug_output_never_contains_the_client_secret_suffix() {
        let row = row();
        let formatted = format!("{row:?}");

        assert!(
            !formatted.contains("neverlogthispayercredential00000"),
            "Debug output must not contain the client_secret_suffix"
        );
        // Not even a prefix of it: a redaction that truncated rather than
        // replaced would still hand a guesser most of the credential.
        assert!(
            !formatted.contains("neverlog"),
            "Debug output must not contain even a prefix of the client_secret_suffix"
        );
        assert!(
            formatted.contains("[32 chars redacted]"),
            "Debug output must contain the redaction marker"
        );
    }

    /// Everything an operator investigating a payment starts from stays
    /// visible — a `Debug` that redacted the whole row to be thorough would
    /// be worse at the job it exists for.
    #[test]
    fn a_payment_intent_rows_debug_output_still_names_the_row() {
        let formatted = format!("{:?}", row());

        for expected in [
            "pi_00000000000000000000000x",
            "acme-cameroon-tenant",
            "requires_payment_method",
            "5000",
            "XAF",
            "mtn_momo",
        ] {
            assert!(
                formatted.contains(expected),
                "{expected:?} is missing from Debug output"
            );
        }
    }
}
