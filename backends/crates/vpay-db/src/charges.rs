//! The `charges` repository (`backends/migrations/0004_create-charges.sql`,
//! plus the `updated_at` column added by `0014` and `return_url` by `0019`).
//!
//! # Three writes, and only one of them is unguarded
//!
//! [`insert_for_intent`] opens the charge before the rail is called;
//! [`mark_submitted`] and [`mark_failed`] record what the rail answered.
//! Both of the latter are compare-and-swaps out of `submitting` rather than
//! blind updates, so a recovery pass and a live confirm cannot overwrite
//! each other's answer — see their own docs.
//!
//! # "One charge per intent, forever" is the database's job, not this
//! module's
//!
//! [`insert_for_intent`] does **not** check whether a charge already exists
//! before inserting one. The unique index `one_charge_per_intent` does that,
//! and it is the only thing that can: a `SELECT` followed by an `INSERT`
//! leaves a window in which two concurrent confirmations both see nothing
//! and both write, which is precisely the double-charge this rule exists to
//! prevent. The `INSERT` is the check. What this module adds is that the
//! resulting `23505` arrives as [`DbError::UniqueViolation`] naming
//! `one_charge_per_intent`, so a handler can answer `409` instead of the
//! `503`-with-retry-advice an unclassified storage error would produce.
//!
//! A handler may still read first (`get_for_intent`) to answer a *friendly*
//! `409` without attempting the write — but that read is an optimisation,
//! never the guard.
//!
//! # Why the insert takes a connection, not a pool
//!
//! `docs/flows/crash-safety.md` requires the charge row — carrying the
//! `provider_reference_id` the rail will be given — to be committed
//! *before* any network call. The confirm path therefore owns a
//! transaction, and this function has to run inside it rather than on a
//! second connection from the pool that would commit independently.

use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{DbError, classify_write};

/// Every column of `charges`, shared by both queries so they cannot drift
/// on what [`ChargeRow`] decodes.
///
/// `state` and `failure_code` are cast to `TEXT` for the same reason
/// `payment_intents`' enums are: this crate carries Postgres enums as
/// `String` and `vpay-core` parses them (D4).
const COLUMNS: &str = "id, payment_intent_id, provider_code, provider_reference_id, \
                       provider_ref_extra, redirect_url, return_url, state::TEXT AS state, \
                       amount, currency_code, payer_ref, payer_ref_masked, \
                       failure_code::TEXT AS failure_code, failure_raw, created_at, updated_at";

/// One `charges` row, exactly as stored.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ChargeRow {
    /// Public `ch_…` id, supplied before the insert.
    pub id: String,
    /// The intent this charge settles. Unique across the table —
    /// `one_charge_per_intent`.
    pub payment_intent_id: String,
    /// The rail that was chosen, e.g. `mtn_momo`.
    pub provider_code: String,
    /// The reference generated *before* the first network call and sent to
    /// the rail as its idempotency key (`vpay_provider::ChargeRef`).
    pub provider_reference_id: Uuid,
    /// Rail key material captured from a previous call (Orange's
    /// `pay_token`, and similar) — `vpay_provider::RefExtra` as JSON.
    pub provider_ref_extra: Option<serde_json::Value>,
    /// Set only on redirect rails, and only once the rail has issued it.
    pub redirect_url: Option<String>,
    /// Where the *merchant* asked the rail to send the payer back
    /// (migration 0019), written before the rail is called and rendered as
    /// `next_action.redirect_to_url.return_url` on every later read. Not
    /// rail material — see the migration for why it is a column and not one
    /// more key inside `provider_ref_extra`.
    pub return_url: Option<String>,
    /// `charge_state` as text (D4). A charge starts at `submitting`.
    pub state: String,
    /// Integer minor units, carried verbatim from the intent (D2 of this
    /// step: no conversion, ever).
    pub amount: i64,
    /// The intent's currency, unchanged.
    pub currency_code: String,
    /// The payer's instrument, where the rail's flow tells us who they are.
    pub payer_ref: Option<String>,
    /// The same instrument, masked for display.
    pub payer_ref_masked: Option<String>,
    /// Closed failure vocabulary as text, once the charge has failed.
    pub failure_code: Option<String>,
    /// The rail's own words for that failure, kept so nothing is dropped.
    pub failure_raw: Option<String>,
    /// When the charge row was written — which is before the rail was
    /// called, by construction.
    pub created_at: OffsetDateTime,
    /// When the row last changed (migration 0014).
    pub updated_at: OffsetDateTime,
}

/// The columns a caller supplies when opening a charge: [`ChargeRow`] minus
/// the failure pair (a charge is never born failed) and minus the two
/// timestamps.
///
/// Unlike [`crate::NewPaymentIntent`], `created_at` is **not** a field:
/// this row's creation instant is load-bearing for crash recovery — it is
/// how long an unanswered `submitting` charge has been outstanding — and
/// the database's `now()` is the one clock every replica shares. Nothing
/// about the confirm path needs to choose it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCharge {
    /// Public `ch_…` id from `vpay_core::ids`, generated before the write.
    pub id: String,
    /// The intent being charged.
    pub payment_intent_id: String,
    /// The rail this charge goes to. Must exist in `providers`.
    pub provider_code: String,
    /// The rail-facing reference, generated before the write and never
    /// regenerated for the same charge (`docs/flows/crash-safety.md`).
    pub provider_reference_id: Uuid,
    /// Rail key material, when a previous call produced any.
    pub provider_ref_extra: Option<serde_json::Value>,
    /// The redirect target, when one is known at insert time. Never known
    /// at insert time today: only a rail's `submit` answer carries one.
    pub redirect_url: Option<String>,
    /// The merchant's return destination on a redirect rail. Supplied
    /// *here*, at insert, rather than by the later `mark_submitted`, because
    /// it is the caller's own input and is therefore knowable before the
    /// network call — the same write-before-network discipline the reference
    /// itself follows (`docs/flows/crash-safety.md`).
    pub return_url: Option<String>,
    /// Initial state — `submitting` for every charge vpay opens today. A
    /// parameter rather than a literal so the state machine stays in
    /// `vpay_core::state`.
    pub state: String,
    /// Integer minor units, the intent's amount.
    pub amount: i64,
    /// The intent's currency, carried verbatim (D2).
    pub currency_code: String,
    /// The payer's instrument, on rails that name one up front (a push
    /// rail's MSISDN).
    pub payer_ref: Option<String>,
    /// The masked form for display.
    pub payer_ref_masked: Option<String>,
}

/// Opens the single charge for an intent, inside the caller's transaction.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] with `constraint: "one_charge_per_intent"`
/// if this intent already has a charge — the load-bearing case, and the one
/// `a_second_charge_for_one_intent_is_refused_by_name` pins. Also
/// [`DbError::UniqueViolation`] on `charges_pkey` for a reused `id`,
/// [`DbError::ForeignKeyViolation`] for an unknown intent, provider or
/// currency, and [`DbError::Query`] otherwise.
pub async fn insert_for_intent(
    tx: &mut PgConnection,
    new: &NewCharge,
) -> Result<ChargeRow, DbError> {
    let sql = format!(
        "INSERT INTO charges (id, payment_intent_id, provider_code, provider_reference_id, \
         provider_ref_extra, redirect_url, return_url, state, amount, currency_code, payer_ref, \
         payer_ref_masked) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::charge_state, $9, $10, $11, $12) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(&new.id)
        .bind(&new.payment_intent_id)
        .bind(&new.provider_code)
        .bind(new.provider_reference_id)
        .bind(new.provider_ref_extra.as_ref())
        .bind(new.redirect_url.as_deref())
        .bind(new.return_url.as_deref())
        .bind(&new.state)
        .bind(new.amount)
        .bind(&new.currency_code)
        .bind(new.payer_ref.as_deref())
        .bind(new.payer_ref_masked.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(classify_write)
}

/// Reads the charge belonging to an intent, if one has been opened.
///
/// Not merchant-scoped, unlike every query in
/// [`crate::payment_intents`]: a charge is reached *through* its intent,
/// and the caller has already established that the intent belongs to the
/// requesting merchant. Taking a `merchant_id` here would suggest this
/// function performs that check, which would be a worse lie than not
/// offering it — `charges` has no `merchant_id` column to filter on.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn get_for_intent(
    pool: &PgPool,
    payment_intent_id: &str,
) -> Result<Option<ChargeRow>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM charges WHERE payment_intent_id = $1");

    sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(payment_intent_id)
        .fetch_optional(pool)
        .await
        .map_err(DbError::Query)
}

/// Records what the rail answered a `submit` with, as a compare-and-swap out
/// of `submitting`, inside the caller's transaction.
///
/// # Why every field moves in one statement, and why the state is a guard
///
/// `docs/flows/crash-safety.md`'s redirect-rail rule — "the commit is the
/// gate on the redirect" — is a statement about *this* write: the rail's
/// `pay_token` (`ref_extra`) and the URL the payer is sent to must become
/// durable together, before anyone is handed the URL. Splitting them across
/// two statements would create a window in which a crash leaves a payer
/// stranded on the rail's page against a charge vpay cannot query.
///
/// `state = 'submitting'` in the `WHERE` is what makes this a state machine
/// rather than a hope. A concurrent recovery pass (Step 4) may have already
/// advanced the same charge; a blind `UPDATE … WHERE id = $1` would silently
/// drag it back to `submitted` and re-open a charge the rail has already
/// settled. Matching nothing is therefore an answer, not an error to
/// swallow — see the `Errors` section.
///
/// `return_url` is deliberately absent: it is the merchant's, written at
/// insert (see [`NewCharge::return_url`]), and a rail's answer has no
/// business overwriting it.
///
/// # Errors
///
/// [`DbError::WriteMatchedNoRow`] if `id` names no charge in `submitting` —
/// which is either a charge someone else already advanced or an id that does
/// not exist. Neither is a merchant's doing, so it classifies as `Internal`
/// (`docs/flows/errors.md`) and the caller must not retry it as if the rail
/// had failed. [`DbError::Query`] if the write itself fails.
pub async fn mark_submitted(
    tx: &mut PgConnection,
    id: &str,
    state: &str,
    provider_ref_extra: Option<&serde_json::Value>,
    redirect_url: Option<&str>,
) -> Result<ChargeRow, DbError> {
    let sql = format!(
        "UPDATE charges \
         SET state = $2::charge_state, \
             provider_ref_extra = $3, \
             redirect_url = $4, \
             updated_at = now() \
         WHERE id = $1 AND state = 'submitting'::charge_state \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(id)
        .bind(state)
        .bind(provider_ref_extra)
        .bind(redirect_url)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)?
        .ok_or_else(|| DbError::WriteMatchedNoRow {
            table: "charges",
            key: id.to_owned(),
        })
}

/// Fails a charge the rail declined, as the same compare-and-swap out of
/// `submitting`, inside the caller's transaction.
///
/// Separate from [`mark_submitted`] rather than one function with an
/// `Option<FailureCode>`, because the two writes are not variants of one
/// decision: a decline moves a charge to a **terminal** state and records
/// the taxonomy (`docs/flows/failures.md`), while a submit moves it to a
/// live one and records the rail's key material. A single function would
/// have to take five arguments of which three are always `None`, and the
/// call site would stop saying which of the two happened.
///
/// `failure_code` is the closed vocabulary — the column is the
/// `failure_code` Postgres enum, so a value outside it is refused by the
/// database and not by a convention. `failure_raw` is the rail's own words,
/// kept because `docs/flows/failures.md` requires the unmapped reason to
/// survive; the database truncates nothing, so the caller bounds it (the
/// `failure_raw_length` CHECK is 2000 characters).
///
/// # Errors
///
/// As [`mark_submitted`].
pub async fn mark_failed(
    tx: &mut PgConnection,
    id: &str,
    failure_code: &str,
    failure_raw: &str,
) -> Result<ChargeRow, DbError> {
    let sql = format!(
        "UPDATE charges \
         SET state = 'failed'::charge_state, \
             failure_code = $2::failure_code, \
             failure_raw = $3, \
             updated_at = now() \
         WHERE id = $1 AND state = 'submitting'::charge_state \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(id)
        .bind(failure_code)
        .bind(failure_raw)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)?
        .ok_or_else(|| DbError::WriteMatchedNoRow {
            table: "charges",
            key: id.to_owned(),
        })
}
