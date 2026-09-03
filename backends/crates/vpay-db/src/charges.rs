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
//! The writes that take a charge to a *terminal* state from anywhere in the
//! live set — what the worker's poll ladder decides — are not here: they
//! move the charge, the intent and an `events` row together and therefore
//! belong to the one transaction that does all three
//! ([`crate::settlement`]). Splitting them across this module would have
//! made it possible to call one without the others, which is the specific
//! thing that transaction exists to prevent.
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
//! # This module owns `vpay_charge_transitions_total`'s labels — but not,
//! for the three writes below, its timing
//!
//! `record_transition` — private, because the *only* correct callers are the
//! six statements' own modules — backs each of the three writes below and
//! the three in [`crate::settlement`], and nothing else. The database layer
//! rather than the caller, because *every* transition passes through these
//! six statements and only some of them pass through the worker: a confirm
//! opens and submits a charge inside `vpay-api`, so a counter mounted on the
//! worker's settlement points would be silently blind to the busiest half of
//! the state machine.
//!
//! Two rules make the count mean what it says, and the second one is why
//! this module has [`record_opened`] and [`record_left_submitting`] at all.
//!
//! **Every label is read back off the returned row**, never off the caller's
//! argument, and the recording happens only after the statement returned a
//! row — a compare-and-swap that matched nothing is a transition that did
//! not happen.
//!
//! **A transition is counted after it is committed, never before.** The
//! three writes in [`crate::settlement`] own their own transaction, so they
//! record after their own `COMMIT`. The three below run inside a
//! *caller's* transaction — that is the whole point of taking a
//! `PgConnection` (`docs/flows/crash-safety.md` requires the charge row to
//! be committed before any network call, and the caller owns that commit) —
//! so they cannot record at all: a `ROLLBACK` after the insert, from a later
//! statement in the same transaction failing, would otherwise leave a
//! counter claiming a charge that does not exist. Instead each returns its
//! row and the caller calls [`record_opened`] or [`record_left_submitting`]
//! **after** `tx.commit()`. The seam is still this module — the label
//! vocabulary and the metric name are here and the callers pass no strings —
//! but the *timing* has to belong to whoever owns the commit, because
//! nothing inside a transaction can know whether it will be committed.
//!
//! (Until 2026-09-03 all three recorded inline, and this header claimed the
//! metric "cannot claim a transition the database refused" while a
//! rolled-back insert was counted. The claim is now true rather than
//! qualified.)
//!
//! What that timing costs: a caller can now *forget* to record, which an
//! inline call could not; and a process that dies between `tx.commit()` and
//! the recorder loses that transition for good, so the counter is at-most-once
//! against `charges`, never exactly-once — expect drift after a crash. Both directions are pinned by tests rather than by
//! review —
//! `a_rolled_back_charge_insert_counts_nothing_and_a_committed_one_counts_once`
//! (`tests/repositories.rs`) fails if the recording moves back inside the
//! statement, and
//! `a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`
//! (`backends/tests/integration/tests/worker_e2e.rs`) scrapes the running
//! server and fails if any of the four edges of one charge's walk goes
//! uncounted, which is what happens when a caller drops its call.
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

/// Counts the charge [`insert_for_intent`] opened — **after** the caller's
/// transaction has committed.
///
/// Counted as a transition out of nothing. A charge being opened is the
/// first edge of the state machine and the one every later edge is a
/// fraction of, so leaving it out would mean a dashboard could show the
/// failures without the denominator.
///
/// Call it once, immediately after the `COMMIT` that made the row real. See
/// this module's header for why the caller and not the insert.
pub fn record_opened(charge: &ChargeRow) {
    record_transition(&charge.provider_code, NO_PRIOR_STATE, &charge.state);
}

/// Counts the move out of `submitting` that [`mark_submitted`] or
/// [`mark_failed`] performed — **after** the caller's transaction has
/// committed.
///
/// `from` is the literal in both statements' `WHERE` clauses, not a guess:
/// neither matches a row unless the charge was in `submitting`. `to` and the
/// rail come off the row the statement returned.
///
/// One function for both writes because the *transition* is what is counted
/// and both leave the same state; which of the two happened is the `to`
/// label. See this module's header for why the caller and not the write.
pub fn record_left_submitting(charge: &ChargeRow) {
    record_transition(
        &charge.provider_code,
        ChargeState::Submitting.as_wire_str(),
        &charge.state,
    );
}

/// Every column of `charges`, shared by both queries so they cannot drift
/// on what [`ChargeRow`] decodes.
///
/// `state` and `failure_code` are cast to `TEXT` for the same reason
/// `payment_intents`' enums are: this crate carries Postgres enums as
/// `String` and `vpay-core` parses them (D4).
///
/// `pub(crate)` because the settlement transaction
/// ([`crate::settlement::apply_succeeded`]) writes this table too, and a
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
    /// [`crate::settlement::apply_succeeded`]. `None` until the charge
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
/// caller calls [`record_opened`] once it has one. See this module's header.
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

    let row = sqlx::query_as::<_, ChargeRow>(&sql)
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

/// Reads one charge by its own id.
///
/// # Why this exists alongside [`get_for_intent`]
///
/// The worker addresses charges directly: a `poll_charge` job's payload
/// carries a `charge_id`, because that is what was known at enqueue time and
/// what stays true across a crash. Reaching it through its intent would mean
/// the job payload had to carry the *intent* id and the worker had to hope
/// the one-charge-per-intent invariant holds — which it does, but making the
/// lookup depend on an invariant it does not need is how a repair path that
/// runs on a broken database stops working exactly when it is needed.
///
/// Not merchant-scoped, for the reason [`get_for_intent`] gives: `charges`
/// has no `merchant_id`, and the caller here is a background worker with no
/// merchant to scope to at all.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn get_by_id(pool: &PgPool, id: &str) -> Result<Option<ChargeRow>, DbError> {
    let sql = format!("SELECT {COLUMNS} FROM charges WHERE id = $1");

    sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(id)
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
/// # Why `provider_ref_extra` is merged and not assigned
///
/// The column is `vpay_provider::RefExtra` — *rail key material* (migration
/// 0019's header), and on a redirect rail the `pay_token` in it is the only
/// thing that can ever query the charge again. `vpay_worker`'s
/// `resubmit_charge` calls this function with whatever the rail answered the
/// **second** submit with, and a push rail answers with an empty map; a plain
/// `provider_ref_extra = $3` would then overwrite key material with `{}` and
/// leave a charge nobody can ask about. So the new map is merged over the
/// stored one (`||`, right-hand wins per key), and a `NULL` argument — "the
/// answer carried nothing" — leaves the column exactly as it was rather than
/// erasing it. Merging cannot lose a key; assigning can, and the loss is
/// silent and permanent.
///
/// The state guard makes that unreachable on today's paths (only a
/// `submitting` charge matches, and nothing writes key material before the
/// first answer), which is precisely why it is worth writing down: this is a
/// defence against the *next* caller, not against a bug that exists.
///
/// # `redirect_url` follows the same rule, for the same reason
///
/// `redirect_url = COALESCE($4, redirect_url)`: a `NULL` argument means "this
/// answer carried no URL", never "there is no URL". The two are the same
/// statement's two audiences — the payer, who may already be holding that URL
/// (`docs/flows/crash-safety.md`, "the commit is the gate on the redirect"),
/// and `GET /v1/payment_intents/{id}`, whose `next_action` is rendered from
/// this column (`vpay_api::v1::payment_intents`). A plain assignment would
/// let a second `mark_submitted` — a resubmit whose answer had no URL —
/// blank the only address the payer can pay at, while leaving the charge
/// live: an intent in `requires_action` with nothing to act on, which is a
/// state the API answers `500` for by design.
///
/// Unreachable today for the same reason as above and one more: a redirect
/// charge still in `submitting` is failed rather than resubmitted
/// (`vpay_worker::recovery`, `RecoveryAction::FailDeadOrder`), so the only
/// caller that could pass a second answer never runs on the rail that has
/// URLs. It is written as a merge anyway because "unreachable" is a property
/// of today's callers and this is a column a payer is standing on.
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
/// Does **not** count the transition — [`record_left_submitting`], after the
/// caller's commit. See this module's header.
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
             provider_ref_extra = CASE \
                 WHEN $3::JSONB IS NULL THEN provider_ref_extra \
                 ELSE COALESCE(provider_ref_extra, '{{}}'::JSONB) || $3::JSONB \
             END, \
             redirect_url = COALESCE($4, redirect_url), \
             updated_at = now() \
         WHERE id = $1 AND state = 'submitting'::charge_state \
         RETURNING {COLUMNS}"
    );

    let row = sqlx::query_as::<_, ChargeRow>(&sql)
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
/// As [`mark_submitted`], and it does not count its transition either —
/// [`record_left_submitting`], after the caller's commit.
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

    let row = sqlx::query_as::<_, ChargeRow>(&sql)
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
