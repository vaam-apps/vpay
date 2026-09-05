//! The `charges` repository (`backends/migrations/0004_create-charges.sql`,
//! plus the `updated_at` column added by `0014` and `return_url` by `0019`).
//!
//! Three writes: [`crate::TxRepositories::insert_for_intent`] opens the charge
//! before the rail is called, and
//! [`crate::TxRepositories::mark_submitted`]/[`crate::TxRepositories::mark_failed`]
//! record what the rail answered as compare-and-swaps out of `submitting`. All
//! three run inside a *caller's* transaction, because
//! `docs/flows/crash-safety.md` requires the charge row to be committed before
//! any network call and the caller owns that commit. The writes that take a
//! charge terminal are [`crate::settlement`]'s, not these.
//!
//! [`Charges::record_opened`] and [`Charges::record_left_submitting`] are the
//! `vpay_charge_transitions_total` recorders the caller must invoke **after**
//! its `COMMIT` — nothing inside a transaction can know whether it will be
//! committed.
//!
//! `docs/reference/vpay-db.md` §"`charges`" carries the reasoning: why the
//! unique index and not a read is what enforces one charge per intent, why the
//! counter lives in this layer, and what the after-the-commit timing costs.

use sqlx::postgres::PgRow;
// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::{AssertSqlSafe, FromRow, PgConnection, Row};
use time::OffsetDateTime;
use uuid::Uuid;
use vpay_core::ChargeState;
use vpay_core::metrics::CHARGE_TRANSITIONS_TOTAL;

use crate::error::{DbError, classify_write};

/// The `from` label for a charge that had no previous state: the row is
/// being created.
///
/// The empty string rather than a word like `none`, and rather than omitting
/// the label: a Prometheus series is identified by its whole label *set*, so
/// dropping `from` here would put charge creation on a different series from
/// every other transition and break `sum by (to) (...)`. Empty is the same
/// convention `vpay_provider_requests_total`'s `error_kind` uses for "there
/// was no error".
pub(crate) const NO_PRIOR_STATE: &str = "";

/// Counts one charge state transition that the database actually performed.
///
/// The single seam for [`CHARGE_TRANSITIONS_TOTAL`] — see this module's
/// header for why it lives in `vpay-db` and not at the callers.
///
/// Call it *after* the statement returned a row, never before: these writes
/// are compare-and-swaps, and a swap that matched nothing is a transition
/// that did not happen. `to` and `provider` come off the returned row in
/// every caller, so they are the database's answer rather than the caller's
/// intention.
///
/// A no-op when no recorder is installed, which is the case in most of this
/// crate's own tests and in every process that has not called
/// `install_recorder` — the library never installs one
/// (`vpay_core::metrics`).
pub(crate) fn record_transition(provider_code: &str, from: &str, to: &str) {
    metrics::counter!(
        CHARGE_TRANSITIONS_TOTAL,
        "provider" => provider_code.to_owned(),
        "from" => from.to_owned(),
        "to" => to.to_owned(),
    )
    .increment(1);
}

/// Every column of `charges`, shared by both queries so they cannot drift
/// on what [`ChargeRow`] decodes.
///
/// `state` and `failure_code` are cast to `TEXT` for the same reason
/// `payment_intents`' enums are: this crate carries Postgres enums as
/// `String` and `vpay-core` parses them (D4).
///
/// `pub(crate)` because the settlement transaction
/// ([`crate::Settlement::apply_succeeded`]) writes this table too, and a
/// second column list there would let the two drift on what
/// [`ChargeRow`] decodes — the exact failure a shared constant exists to
/// prevent.
pub(crate) const COLUMNS: &str = "id, payment_intent_id, provider_code, provider_reference_id, \
                       provider_ref_extra, provider_txn_id, redirect_url, return_url, \
                       state::TEXT AS state, amount, currency_code, payer_ref, payer_ref_masked, \
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
    /// The rail's own identifier for the settled payment (migration 0021),
    /// learned from `ChargeStatus::Succeeded` and written only by
    /// [`crate::Settlement::apply_succeeded`]. `None` until the charge
    /// succeeds — and still `None` afterwards if the rail named no
    /// identifier, because there is nothing honest to put here.
    pub provider_txn_id: Option<String>,
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

/// One `charges` row, together with the database's own clock at the instant
/// it was read.
///
/// # Why the clock travels with the row
///
/// Two of the worker's decisions about a charge are *ages*: whether it is
/// old enough for `submitting` to be evidence of a crash
/// (`vpay_worker::recovery_step`), and whether it is past the 24-hour
/// escalation horizon (`vpay_worker`'s `past_the_horizon`). Both subtract
/// [`ChargeRow::created_at`] — written by Postgres' `now()` inside the
/// confirm's transaction — from "now", and a worker that reads that "now"
/// off its own host clock is subtracting two different clocks. A worker
/// sixty seconds ahead of Postgres then measures every charge as a minute
/// older than it is, which makes the age guard a silent no-op and hands
/// every live confirm back to the recovery table.
///
/// So the second operand comes from the same `SELECT` as the first: one
/// statement, one clock, and no way for a caller to supply the wrong one.
/// `docs/reference/vpay-db.md` §"The charge read carries Postgres' clock"
/// carries the argument, and `docs/reference/vpay-worker.md` §"Nothing
/// younger than the window is recovered" is what depends on it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChargeAsOf {
    /// The row itself, decoded exactly as [`Charges::get_by_id`] decodes it.
    pub charge: ChargeRow,
    /// Postgres' `now()`, evaluated by the statement that read the row.
    ///
    /// The transaction timestamp, which for a statement outside an explicit
    /// transaction is that statement's own start — near enough to "when the
    /// row was read" for an age measured in seconds, and the same value every
    /// other write in this crate stamps rows with.
    pub db_now: OffsetDateTime,
}

impl FromRow<'_, PgRow> for ChargeAsOf {
    /// Hand-written rather than `#[derive]`d with a flattened field: the
    /// derive would need [`ChargeRow`]'s columns to be a nested prefix, and
    /// what this decodes is one flat row that happens to carry one extra
    /// column.
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            charge: ChargeRow::from_row(row)?,
            db_now: row.try_get("db_now")?,
        })
    }
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
/// Does **not** count the transition: the caller owns the commit, so the
/// caller calls [`Charges::record_opened`] once it has one. See this module's header.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] with `constraint: "one_charge_per_intent"`
/// if this intent already has a charge — the load-bearing case, and the one
/// `a_second_charge_for_one_intent_is_refused_by_name` pins. Also
/// [`DbError::UniqueViolation`] on `charges_pkey` for a reused `id`,
/// [`DbError::ForeignKeyViolation`] for an unknown intent, provider or
/// currency, and [`DbError::Query`] otherwise.
pub(crate) async fn insert_for_intent(
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

    let row = sqlx::query_as::<_, ChargeRow>(AssertSqlSafe(sql))
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
        .map_err(classify_write)?;

    Ok(row)
}

/// Records what the rail answered a `submit` with, as a compare-and-swap out
/// of `submitting`, inside the caller's transaction.
///
/// Every field the rail answered with moves in one statement, and the state is
/// the guard: `provider_ref_extra` is **merged** rather than assigned, and
/// `redirect_url` is `COALESCE`d, so a second answer that carried neither
/// cannot erase the key material or the URL a payer is standing on.
/// `return_url` is deliberately absent — it is the merchant's, written at
/// insert (see [`NewCharge::return_url`]). `docs/reference/vpay-db.md`
/// §"`mark_submitted` merges rather than assigns" carries all three arguments.
///
/// # Errors
///
/// [`DbError::WriteMatchedNoRow`] if `id` names no charge in `submitting` —
/// which is either a charge someone else already advanced or an id that does
/// not exist. Neither is a merchant's doing, so it classifies as `Internal`
/// (`docs/flows/errors.md`) and the caller must not retry it as if the rail
/// had failed. [`DbError::Query`] if the write itself fails.
///
/// # Counting
///
/// Does **not** count the transition — [`Charges::record_left_submitting`], after the
/// caller's commit. See this module's header.
pub(crate) async fn mark_submitted(
    tx: &mut PgConnection,
    id: &str,
    state: &str,
    provider_ref_extra: Option<&serde_json::Value>,
    redirect_url: Option<&str>,
) -> Result<ChargeRow, DbError> {
    let sql = format!(
        "UPDATE charges \
         SET state = $2::charge_state, \
             provider_ref_extra = CASE \
                 WHEN $3::JSONB IS NULL THEN provider_ref_extra \
                 ELSE COALESCE(provider_ref_extra, '{{}}'::JSONB) || $3::JSONB \
             END, \
             redirect_url = COALESCE($4, redirect_url), \
             updated_at = now() \
         WHERE id = $1 AND state = 'submitting'::charge_state \
         RETURNING {COLUMNS}"
    );

    let row = sqlx::query_as::<_, ChargeRow>(AssertSqlSafe(sql))
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
        })?;

    Ok(row)
}

/// Fails a charge the rail declined, as the same compare-and-swap out of
/// `submitting`, inside the caller's transaction.
///
/// Separate from [`crate::TxRepositories::mark_submitted`] rather than one
/// function with an `Option<FailureCode>` — see `docs/reference/vpay-db.md`
/// §"`mark_submitted` merges rather than assigns".
///
/// `failure_code` is the closed vocabulary: the column is the `failure_code`
/// Postgres enum, so a value outside it is refused by the database and not by
/// a convention. `failure_raw` is the rail's own words, kept because
/// `docs/flows/failures.md` requires the unmapped reason to survive; the
/// database truncates nothing, so the caller bounds it against the
/// `failure_raw_length` CHECK.
///
/// # Errors
///
/// As [`crate::TxRepositories::mark_submitted`], and it does not count its transition either —
/// [`Charges::record_left_submitting`], after the caller's commit.
pub(crate) async fn mark_failed(
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

    let row = sqlx::query_as::<_, ChargeRow>(AssertSqlSafe(sql))
        .bind(id)
        .bind(failure_code)
        .bind(failure_raw)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)?
        .ok_or_else(|| DbError::WriteMatchedNoRow {
            table: "charges",
            key: id.to_owned(),
        })?;

    Ok(row)
}

#[async_trait::async_trait]
pub trait Charges: Send + Sync {
    /// Counts the charge [`crate::TxRepositories::insert_for_intent`] opened — **after** the caller's
    /// transaction has committed.
    ///
    /// Counted as a transition out of nothing. A charge being opened is the
    /// first edge of the state machine and the one every later edge is a
    /// fraction of, so leaving it out would mean a dashboard could show the
    /// failures without the denominator.
    ///
    /// Call it once, immediately after the `COMMIT` that made the row real. See
    /// this module's header for why the caller and not the insert.
    fn record_opened(&self, charge: &ChargeRow);

    /// Counts the move out of `submitting` that [`crate::TxRepositories::mark_submitted`] or
    /// [`crate::TxRepositories::mark_failed`] performed — **after** the caller's transaction has
    /// committed.
    ///
    /// `from` is the literal in both statements' `WHERE` clauses, not a guess:
    /// neither matches a row unless the charge was in `submitting`. `to` and the
    /// rail come off the row the statement returned.
    ///
    /// One function for both writes because the *transition* is what is counted
    /// and both leave the same state; which of the two happened is the `to`
    /// label. See this module's header for why the caller and not the write.
    fn record_left_submitting(&self, charge: &ChargeRow);

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
    async fn get_for_intent(&self, payment_intent_id: &str) -> Result<Option<ChargeRow>, DbError>;

    /// Reads one charge by its own id.
    ///
    /// # Why this exists alongside [`Charges::get_for_intent`]
    ///
    /// The worker addresses charges directly: a `poll_charge` job's payload
    /// carries a `charge_id`, because that is what was known at enqueue time and
    /// what stays true across a crash. Reaching it through its intent would mean
    /// the job payload had to carry the *intent* id and the worker had to hope
    /// the one-charge-per-intent invariant holds — which it does, but making the
    /// lookup depend on an invariant it does not need is how a repair path that
    /// runs on a broken database stops working exactly when it is needed.
    ///
    /// Not merchant-scoped, for the reason [`Charges::get_for_intent`] gives: `charges`
    /// has no `merchant_id`, and the caller here is a background worker with no
    /// merchant to scope to at all.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_by_id(&self, id: &str) -> Result<Option<ChargeRow>, DbError>;

    /// The same read as [`Charges::get_by_id`], plus the database's own clock
    /// at the instant of the read.
    ///
    /// # Why the worker's poll uses this one
    ///
    /// Everything the worker decides about a `submitting` charge is a
    /// *duration*: how long it has been sitting there. The subtrahend is
    /// `charges.created_at`, which Postgres wrote; the minuend used to be the
    /// worker host's own clock, so the two came from different machines and a
    /// worker a minute ahead of Postgres saw every charge as a minute older
    /// than it was — which turns the recovery window into a no-op precisely
    /// on the deployment whose clocks have drifted. [`ChargeAsOf`] says the
    /// rest.
    ///
    /// One statement rather than a `SELECT now()` beside the row read: two
    /// statements would be two transaction timestamps with a scheduling gap
    /// between them, and the gap would be exactly the quantity being
    /// measured.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_by_id_as_of(&self, id: &str) -> Result<Option<ChargeAsOf>, DbError>;

    /// Reads the charge a rail's callback names — the reference *we*
    /// generated, scoped to the rail that is speaking.
    ///
    /// # Why the rail is a parameter and not derived from the reference
    ///
    /// The one caller is `vpay_api::provider_callback`, serving an
    /// **unauthenticated** `POST /provider/{code}/callback`. Everything about
    /// that request is attacker-controlled except the path segment naming the
    /// rail, and the reference itself arrives inside a body anyone who can
    /// reach the URL could have written. Scoping the read by `provider_code`
    /// is what makes "a body posted to one rail's callback path can never
    /// name a charge on another rail" a property of the query rather than of
    /// a check a future handler could forget to write.
    ///
    /// Served by `charges_provider_reference_idx` (migration `0027`), which
    /// exists because this read is reachable without a credential — see that
    /// migration.
    ///
    /// # Why the answer is bounded rather than assumed unique
    ///
    /// `provider_reference_id` carries no unique constraint. Every insert
    /// path mints it with `Uuid::new_v4()` before committing
    /// (`docs/flows/crash-safety.md`), so two charges sharing one would be a
    /// vpay bug — but this statement must still be *deterministic* rather
    /// than returning whichever row the plan happened to reach first, so it
    /// orders newest-first and takes one. The newest is the charge a callback
    /// arriving now is about if that bug ever exists; and the blast radius is
    /// small by construction, because all the route does with the answer is
    /// enqueue an **authenticated** status query, which settles the charge the
    /// rail actually names or nothing at all.
    ///
    /// Not merchant-scoped, for the reason [`Charges::get_for_intent`] gives,
    /// and more sharply: the caller here is a rail, which has no merchant.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_by_provider_reference(
        &self,
        provider_code: &str,
        provider_reference_id: Uuid,
    ) -> Result<Option<ChargeRow>, DbError>;
}

#[async_trait::async_trait]
impl Charges for crate::repository::PgRepositories {
    fn record_opened(&self, charge: &ChargeRow) {
        record_transition(&charge.provider_code, NO_PRIOR_STATE, &charge.state);
    }

    fn record_left_submitting(&self, charge: &ChargeRow) {
        record_transition(
            &charge.provider_code,
            ChargeState::Submitting.as_wire_str(),
            &charge.state,
        );
    }

    async fn get_for_intent(&self, payment_intent_id: &str) -> Result<Option<ChargeRow>, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM charges WHERE payment_intent_id = $1");

        sqlx::query_as::<_, ChargeRow>(AssertSqlSafe(sql))
            .bind(payment_intent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<ChargeRow>, DbError> {
        // Delegated rather than written a second time: two statements
        // reading one row by its primary key would be free to drift on what
        // they select, and the only difference between the two answers is a
        // column this caller does not want.
        Ok(self.get_by_id_as_of(id).await?.map(|as_of| as_of.charge))
    }

    async fn get_by_id_as_of(&self, id: &str) -> Result<Option<ChargeAsOf>, DbError> {
        let sql = format!("SELECT {COLUMNS}, now() AS db_now FROM charges WHERE id = $1");

        sqlx::query_as::<_, ChargeAsOf>(AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn get_by_provider_reference(
        &self,
        provider_code: &str,
        provider_reference_id: Uuid,
    ) -> Result<Option<ChargeRow>, DbError> {
        // `ORDER BY … LIMIT 1` rather than a bare `fetch_optional`: see the
        // trait method's "bounded rather than assumed unique" section. `id`
        // breaks a `created_at` tie so the answer is total, not merely
        // usually-total.
        let sql = format!(
            "SELECT {COLUMNS} FROM charges \
             WHERE provider_code = $1 AND provider_reference_id = $2 \
             ORDER BY created_at DESC, id DESC LIMIT 1"
        );

        sqlx::query_as::<_, ChargeRow>(AssertSqlSafe(sql))
            .bind(provider_code)
            .bind(provider_reference_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}
