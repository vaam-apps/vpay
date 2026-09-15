//! The `refunds` repository (`backends/migrations/0017_create-refunds.sql`,
//! plus `0031_refunds-fee.sql`) — two reads, two merchant-facing writes and
//! the two settlement writes that belong to somebody else's transaction.
//!
//! **This module creates refunds; no rail has ever executed one.** Since
//! RFC-0003 § 3 [`Refunds::create`] inserts the row and reserves its amount
//! against the intent in one transaction, and [`Refunds::cancel`] gives that
//! reservation back — the *database* half of a refund, which the state of the
//! rails does not postpone. The rail half is in two different states, and
//! neither is `Unsupported`:
//!
//! * `mtn_momo::refund` is **written** — since 2026-09-15 it makes MTN's
//!   Disbursements `transfer` call — but **no deployment holds the
//!   Disbursements subscription key and the product has never been called**,
//!   in sandbox or anywhere else, so it answers `ProviderError::Config` where
//!   the credential is missing and nothing has ever proved it against the
//!   rail.
//! * `orange_money::refund` answers `ProviderError::NotImplemented`
//!   (RFC-0003 § 5, 2026-09-15): an Orange refund is an outbound **transfer**
//!   and no Orange transfer API is documented in this repository, so the
//!   token is vpay's unbuilt work rather than a fact about the rail.
//!
//! `supports_refunds` is `true` on both rails, and `POST /v1/refunds` stays
//! unrouted until Wave 3 (`docs/status.md`).
//!
//! The other thing this module has is the **authoritative read** a refund has
//! to have once it exists at all: `docs/flows/provider-port.md` calls
//! `query_status` "the authoritative read", `docs/flows/webhooks.md` says
//! delivery is at-least-once and unordered, and a merchant holding a `re_…`
//! with no way to ask what happened to it has neither (issue #45).
//!
//! # The writes that are not on the trait, and why they are `pub(crate)`
//!
//! [`settle_in_tx`] and [`fail_in_tx`] are not creates: each moves an
//! **existing** `pending` refund, the first to `succeeded` inside the
//! transaction that also adds the amount to the invoice the refunded intent
//! paid (issue #91's D5) and posts the ledger entries for it, the second to
//! `failed` with the rail's reason, inside the transaction that releases the
//! reservation and posts nothing — because nothing was posted to reverse. Both are `pub(crate)` and reached only from
//! [`crate::settlement`], which is what keeps "a refund is never settled
//! without the document it came off being updated in the same commit" a
//! property of the type system rather than of a convention. A consumer of
//! this crate cannot settle a refund without the invoice update, the intent
//! counters and the ledger posting that go with it. The same is true of the
//! ledger write itself: it is not reachable from outside this crate either.
//!
//! What that costs, stated rather than hidden: **no rail call has ever been
//! made for a refund.** Nothing in a shipping binary calls
//! [`Refunds::create`] — `POST /v1/refunds` is not routed — so no rail can
//! produce the `pending` row [`settle_in_tx`] settles, nothing in
//! `vpay-server` reaches it, and every deployment's `refunds` table is empty
//! but for rows an operator or a test put there. `docs/status.md` says so.
//! The seam exists because D5's and RFC-0003's decisions are about what the
//! *database* does when a refund lands, and a decision with no statement
//! behind it is a sentence in a document.
//!
//! # Why the scope is a join and not a column
//!
//! `refunds` carries no `merchant_id`. Migration `0017` gives it a `NOT NULL`
//! foreign key onto `payment_intents (id)` instead, and the intent is where
//! the tenant lives — so [`Refunds::get_for_merchant`] joins rather than
//! filtering a column of its own. That is a deliberate choice not to migrate
//! the table: a denormalised `merchant_id` would be a second answer to "whose
//! refund is this?", and two answers to a tenancy question is how one of them
//! ends up stale. The cost is one index lookup on the primary key of
//! `payment_intents` per read.
//!
//! # The rule this module keeps
//!
//! The same one [`crate::payment_intents`] and [`crate::checkout_sessions`]
//! keep: **the merchant-facing query is merchant-scoped in SQL**, so a `/v1`
//! handler cannot forget to filter, and another tenant's refund is
//! indistinguishable from a missing one. There is no unscoped variant here —
//! unlike those two modules, nothing in this repository has a caller that
//! needs one.

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::AssertSqlSafe;
use time::OffsetDateTime;

use crate::error::DbError;

/// The columns [`RefundRow`] decodes, table-qualified because the one query
/// below joins.
///
/// `r.status` is selected without a cast. It was `refund_status`, a native
/// Postgres enum, until migration `0037` made it `TEXT` +
/// `refunds_status_enum_check`, and this list had to spell
/// `r.status::TEXT AS status` because `sqlx` refuses to decode a
/// user-defined type into a `String`. The vocabulary is unchanged and still
/// carried as text (D4); only the cast and the type it named are gone.
///
/// `r.fee` is here because it *is* on the wire object — the tenth key, added
/// by migration `0031` for issue #46. It is `NULL` on every row this
/// repository can currently return, because nothing writes it (no rail
/// reports a refund fee to us; `docs/status.md`), and selecting it anyway is
/// what makes the difference between "the rail said nothing" and "the rail
/// said it was free" a column rather than a renderer's guess.
///
/// **Not every column of the table**, and that is the choice
/// [`crate::events::EventRow`] makes for `fanout_attempts`: `charge_id`,
/// `failure_code`, `failure_raw`, `provider_reference_id` and `updated_at`
/// are on the table and no reader of a refund branches on them, because none
/// of them is on the wire object `docs/flows/merchant-auth.md` documents.
/// Selecting a column nothing uses is how a row struct starts mirroring the
/// table instead of its callers — and the writer that would fill those five
/// does not exist yet, so guessing at its shape now would be a claim about
/// code nobody has written.
const COLUMNS: &str = "r.id, r.payment_intent_id, r.amount, r.currency_code, \
                       r.status, r.reason, r.metadata, r.fee, \
                       r.created_at";

/// One `refunds` row, as the merchant read needs it.
///
/// Not the wire object: `vpay-api` owns that shape (lowercase currency,
/// unix-seconds `created`, `payment_intent` rather than `payment_intent_id`).
/// See [`COLUMNS`] for why this is a projection of the table rather than the
/// whole of it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct RefundRow {
    /// Public `re_…` id, supplied by whoever writes the row — never generated
    /// by Postgres, like every other object id in this schema.
    pub id: String,
    /// The intent this refunds. A real foreign key onto `payment_intents
    /// (id)`, and the only path from a refund to its tenant — see the module
    /// docs.
    pub payment_intent_id: String,
    /// Minor units, strictly positive (`amount_positive`, migration `0017`).
    pub amount: i64,
    /// Carried verbatim from the intent, never converted
    /// (`docs/flows/money.md`).
    pub currency_code: String,
    /// `pending`, `succeeded`, `failed` or `canceled` — the vocabulary
    /// `refunds_status_enum_check` closes (migration `0037`, which replaced
    /// the `refund_status` type), decoded as text.
    ///
    /// A `String` and not a typed enum for [`crate::events::EventRow`]'s
    /// reason: the vocabulary is closed by Postgres *where it is written*, and
    /// this crate carries a vocabulary as text so the parse belongs to the
    /// layer that renders it.
    pub status: String,
    /// The merchant's own free text (`"duplicate"`,
    /// `"requested_by_customer"`), or `NULL`. Deliberately not an enum in the
    /// schema either: the vocabulary is the merchant's, not vpay's.
    pub reason: Option<String>,
    /// The merchant's key/value pairs. The `metadata_is_object` CHECK
    /// guarantees it is a JSON object.
    pub metadata: serde_json::Value,
    /// What the rail charged **us** to execute this refund, in minor units of
    /// [`currency_code`](Self::currency_code) (migration `0031`).
    ///
    /// `None` means the rail reported no fee; `Some(0)` means it reported the
    /// movement was free. Those are different answers, the column has no
    /// `DEFAULT` so that they stay different, and a renderer that mapped
    /// `None` to `0` would reintroduce the hardcoded zero issue #46 was filed
    /// about. **Nothing writes it yet**, so every row this repository returns
    /// today carries `None`.
    pub fee: Option<i64>,
    /// When the refund was requested.
    pub created_at: OffsetDateTime,
}

/// What [`Refunds::create`] is given, and — just as importantly — what it is
/// not.
///
/// # Four fields the caller does **not** supply
///
/// `currency_code`, `merchant_id` and `charge_id` are read off the intent
/// inside the same transaction, and `status` is always `pending`. That is not
/// tidiness: every one of them is a fact about the intent, and a caller free
/// to pass its own answer is a caller that can disagree with the row.
/// `docs/flows/money.md` says a refund's currency is the intent's, carried
/// verbatim and never converted; `refunds` has no `merchant_id` at all (see
/// the module docs) so the tenant is the intent's by construction; and the
/// charge is "one charge per intent, forever" (`AGENTS.md`), which the
/// database already knows.
///
/// The same rule is what closes `docs/flows/ledger.md`'s merchant-attribution
/// gap one layer up, in [`crate::settlement`]: a ledger posting's merchant is
/// derived from the intent the charge belongs to and never from anything a
/// request carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRefund {
    /// The `re_…` this refund will be addressed by, minted by the caller
    /// (`vpay_core::ids::refund_id`) before the row exists, like every other
    /// object id in this schema.
    pub id: String,
    /// The intent whose money is coming back. The tenancy check, the
    /// currency, the charge and the over-refund guard all hang off it.
    pub payment_intent_id: String,
    /// Minor units, strictly positive (`amount_positive`, migration `0017`).
    /// A full refund is the caller resolving "no `amount` given" against the
    /// intent; this crate is never handed an absent amount.
    pub amount: i64,
    /// The merchant's own free text, or `None`. Bounded by `reason_length`
    /// (512) at the database.
    pub reason: Option<String>,
    /// The merchant's key/value pairs. Must be a JSON object
    /// (`metadata_is_object`).
    pub metadata: serde_json::Value,
    /// The rail-facing reference for this refund, generated **before** any
    /// rail call (`docs/flows/crash-safety.md`) and stored in the same
    /// transaction as the row.
    ///
    /// Not an `Option`, unlike the column. Migration `0017` made it nullable
    /// for "a rail call has not been attempted yet", and crash-safety's rule
    /// is the opposite one — the reference exists before the call so that a
    /// process which dies mid-call leaves something to reconcile by, and a
    /// resubmit reuses it rather than minting a second. A writer that could
    /// omit it is a writer that can create a refund no crash recovery can
    /// match against the rail.
    pub provider_reference_id: uuid::Uuid,
}

/// The `refunds` writes and reads a consumer of this crate may perform.
///
/// # What [`Refunds::create`] changed, and what it did not
///
/// Until RFC-0003 § 3 this trait carried two reads and no write at all, on
/// the grounds that "a `create` here would be a write path no shipping code
/// calls". [`Refunds::create`] is that write, and the reason it is here now
/// is that the *database* half of a refund is a decision with consequences —
/// the over-refund guard, the reservation, the tenancy join — which the
/// absence of a rail does not postpone.
///
/// **It is still true that no rail has ever executed what this creates**, and
/// on neither rail is the answer `Unsupported` — `supports_refunds` is `true`
/// for both. The two rails are in different states. `mtn_momo::refund` is
/// written: since 2026-09-15 it makes MTN's Disbursements `transfer` call,
/// but no deployment holds the Disbursements subscription key and the product
/// has never been called, so it has proved nothing against the rail.
/// `orange_money::refund` answers `ProviderError::NotImplemented` (RFC-0003
/// § 5, 2026-09-15) because an Orange refund is an outbound transfer and no
/// Orange transfer API is documented in this repository, which makes that
/// token vpay's unbuilt work rather than a fact about the rail.
/// `POST /v1/refunds` is unrouted until Wave 3, so nothing in a
/// shipping binary calls this method today and no rail call has ever been
/// made for a refund. `docs/status.md` says so; this doc says so rather than
/// letting the method's existence imply otherwise.
///
/// # Which writes are on this trait and which are not
///
/// `create` and `cancel` are, because each is a whole business operation that
/// leaves the database consistent on its own: a refund row and its
/// reservation, or a cancellation and the release of one. The three
/// settlement writes are not — [`settle_in_tx`], [`fail_in_tx`] and the
/// intent counters they move are `pub(crate)` and belong to
/// [`crate::settlement`]'s transaction, so a consumer of this crate cannot
/// settle a refund without the invoice update, the intent counters and the
/// ledger posting that go with it.
#[async_trait::async_trait]
pub trait Refunds: Send + Sync {
    /// Creates a `pending` refund **and reserves its amount against the
    /// intent**, in one transaction (RFC-0003 § 3).
    ///
    /// Returns the row as stored, or `Ok(None)` if this merchant has no such
    /// `succeeded` intent — which folds "not yours", "no such intent" and
    /// "that intent never captured anything" into one answer, exactly as
    /// [`Refunds::get_for_merchant`] folds the first two. A caller that never
    /// learns the intent exists cannot leak that it does.
    ///
    /// # The reservation is the point, and it is not a check
    ///
    /// `crate::payment_intents::reserve_refund_in_tx` increments
    /// `amount_refund_pending` in the same transaction and in a single
    /// `UPDATE`, so migration `0003`'s `no_over_refund` CHECK is what refuses
    /// an over-refund — under concurrency, at the database, against the
    /// committed total. There is deliberately no read-then-compare in Rust
    /// anywhere on this path: two concurrent refunds would both read the same
    /// balance and both pass it.
    ///
    /// # Errors
    ///
    /// [`DbError::OverRefund`] when the intent has less left to refund than
    /// was asked for — a `409`, never a retry.
    /// [`DbError::UniqueViolation`] on a replayed `re_…`.
    /// [`DbError::Query`] if any statement or the commit fails; the
    /// transaction rolls back whole, so a failure leaves neither the row nor
    /// the reservation behind.
    async fn create(
        &self,
        merchant_id: &str,
        new: &NewRefund,
    ) -> Result<Option<RefundRow>, DbError>;

    /// Cancels a refund that is still `pending` and gives its reservation
    /// back, in one transaction (RFC-0003 § 2).
    ///
    /// Returns the canceled row, or `Ok(None)` for a refund that is not this
    /// merchant's or is no longer `pending` — the same fold
    /// [`Refunds::create`] makes, and for the same reason.
    ///
    /// # Nothing is reversed, because nothing was posted
    ///
    /// A `pending` refund has moved no money: it holds a reservation on
    /// `amount_refund_pending` and has no ledger transaction. So the ledger
    /// is untouched here and there is no compensating entry to write, which
    /// `docs/flows/ledger.md` § "When refunds post" names as the whole reason
    /// the reservation column exists rather than posting optimistically and
    /// unwinding.
    ///
    /// # Errors
    ///
    /// [`DbError::WriteMatchedNoRow`] on `payment_intents` if the refund was
    /// `pending` but the intent carried no matching reservation — a broken
    /// invariant, which pages. [`DbError::Query`] if any statement or the
    /// commit fails.
    async fn cancel(&self, merchant_id: &str, id: &str) -> Result<Option<RefundRow>, DbError>;

    /// Reads one refund *for this merchant*.
    ///
    /// `None` means "no such refund for you", which covers both a missing id
    /// and another merchant's id. Those two must be indistinguishable, and
    /// folding them together **here** — rather than reading the row and
    /// comparing the tenant in the handler — is what makes them so: a caller
    /// that never learns the row exists cannot leak that it does. It is the
    /// same split `PaymentIntents::get_for_merchant` and
    /// `Events::get_by_id` draw, and the reason `GET /v1/refunds/{id}`
    /// answers `404` rather than `403`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<RefundRow>, DbError>;

    /// Every refund of one intent *for this merchant*, oldest first — the
    /// `/dash/v1` payment detail's refunds section.
    ///
    /// Tenant-scoped through the same join [`Refunds::get_for_merchant`]
    /// uses, and for the same reason: `refunds` carries no `merchant_id` of
    /// its own, so the only honest way to ask "may this caller see it" is
    /// through the intent. An empty `Vec` therefore means both "this intent
    /// has no refunds" and "this intent is not yours" — which is correct,
    /// because the caller reached this read by first retrieving the intent,
    /// and that read already answered `404` in the second case.
    ///
    /// Ascending, unlike every cursor list in this crate: this is a
    /// *timeline* an operator reads top to bottom, not a page. Unbounded for
    /// the same reason — the number of refunds of one intent is bounded by
    /// its amount, not by traffic. If that ever stops being true it needs a
    /// cursor, not a `LIMIT` that silently truncates a money history.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn list_for_intent(
        &self,
        merchant_id: &str,
        payment_intent_id: &str,
    ) -> Result<Vec<RefundRow>, DbError>;
}

/// The `refunds` columns [`settle_in_tx`] needs to finish the transaction it
/// is part of: which intent to charge the refund against, how much, and in
/// what currency.
///
/// A second, narrower projection rather than [`RefundRow`], and the split is
/// deliberate: [`RefundRow`] is *the merchant read*, shaped by what
/// `GET /v1/refunds/{id}` renders. This one is *the settlement's own
/// working set* — it exists so the statement that flips the row hands the
/// caller exactly the facts the next statements in the same transaction
/// need, with no round trip and nothing that could have changed in between.
/// Reusing the wire projection here would tie a settlement's inputs to a
/// rendering decision.
///
/// # Why `currency_code` is on it (2026-09-16)
///
/// [`crate::settlement`]'s refund posting used to build its ledger legs from
/// the *intent's* `currency_code`, because this projection carried none. But
/// `refunds.currency_code` is a real column whose only constraint is the
/// foreign key onto `currencies` (migration `0017`): **nothing in the schema
/// ties it to the intent's.** They agree today solely because
/// [`Refunds::create`] is the only writer and derives it from the intent.
///
/// A divergent writer would render one currency on the refund object and post
/// the legs in another, and `vpay_ledger::Transaction::validate` would not
/// notice — it balances each currency on its own book, and every leg in the
/// same *wrong* currency balances perfectly. An amount and the currency it is
/// denominated in are one fact (`docs/flows/money.md`), so they are now read
/// off one row in one statement.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SettledRefund {
    /// The refund that was settled.
    pub id: String,
    /// The intent the money came back off — the key
    /// `vpay_db::invoices`' refund statement matches the invoice on.
    pub payment_intent_id: String,
    /// Minor units, strictly positive (`amount_positive`, migration `0017`).
    pub amount: i64,
    /// The currency [`amount`](Self::amount) is denominated in, **as stored
    /// on the refund row itself** — see the type docs for why it is not read
    /// off the intent.
    pub currency_code: String,
}

/// Moves one `pending` refund to `succeeded`, inside the caller's
/// transaction.
///
/// `Ok(None)` means the refund was not `pending` — it has already been
/// settled, or it failed, or there is no such row. For a job that may be
/// running twice that is information rather than an error, exactly as
/// [`crate::Settlement::apply_succeeded`]'s `Ok(None)` is.
///
/// # The state machine is the `WHERE` clause
///
/// `AND status = 'pending'` is in the statement, not in a check beside it.
/// `refunds_status_enum_check` (migration `0037`) closes the vocabulary and
/// this clause closes the transition; between them there is no read whose
/// answer could be stale by the time the write lands.
///
/// # No tenant predicate, and that is not an omission
///
/// Unlike [`Refunds::get_for_merchant`], this statement filters on the
/// refund's id alone. It is not reachable from a merchant-facing route — it
/// is `pub(crate)`, called only by [`crate::settlement`], which is driven by
/// a rail's answer about a movement vpay itself initiated. There is no
/// caller-supplied id to scope, and a join added here would be a tenancy
/// check on a value no tenant chose.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails.
pub(crate) async fn settle_in_tx(
    conn: &mut sqlx::PgConnection,
    refund_id: &str,
    now: OffsetDateTime,
) -> Result<Option<SettledRefund>, DbError> {
    sqlx::query_as::<_, SettledRefund>(
        "UPDATE refunds SET status = 'succeeded', updated_at = $2 \
         WHERE id = $1 AND status = 'pending' \
         RETURNING id, payment_intent_id, amount, currency_code",
    )
    .bind(refund_id)
    .bind(now)
    .fetch_optional(&mut *conn)
    .await
    .map_err(crate::error::classify_write)
}

/// The `failure_raw_length` CHECK's ceiling on `refunds.failure_raw`
/// (migration `0017`), in characters.
///
/// The same number, and the same reason, as `settlement`'s ceiling on
/// `charges.failure_raw`: a rail whose text runs long would abort the
/// settlement transaction, leaving the refund `pending` and the job retrying
/// forever against text that will be exactly as long next time.
const FAILURE_RAW_MAX_CHARS: usize = 2000;

/// Moves one `pending` refund to `failed`, with the rail's reason, inside the
/// caller's transaction.
///
/// [`settle_in_tx`]'s twin in every structural respect — the compare-and-swap
/// is the `WHERE` clause, there is no tenant predicate for the same reason,
/// and `Ok(None)` means the refund was not `pending`. What differs is what
/// the caller must then do: a failure releases the reservation and posts
/// **nothing**, because nothing was ever posted.
///
/// `code` is `vpay_core::FailureCode` as text, closed by
/// `refunds_failure_code_enum_check` (migration `0037`), and `raw` is the
/// rail's own words, truncated here rather than left to the CHECK. They are
/// written together or not at all — `failure_paired` (migration `0017`)
/// refuses a code with no text and text with no code.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails, including a `code` outside the
/// vocabulary the CHECK closes — a vpay bug, since that vocabulary is vpay's.
pub(crate) async fn fail_in_tx(
    conn: &mut sqlx::PgConnection,
    refund_id: &str,
    code: &str,
    raw: &str,
    now: OffsetDateTime,
) -> Result<Option<SettledRefund>, DbError> {
    // Characters, not bytes: the CHECK counts characters, and slicing bytes
    // could split one.
    let bounded_raw: String = raw.chars().take(FAILURE_RAW_MAX_CHARS).collect();

    sqlx::query_as::<_, SettledRefund>(
        "UPDATE refunds \
         SET status = 'failed', failure_code = $3, failure_raw = $4, updated_at = $2 \
         WHERE id = $1 AND status = 'pending' \
         RETURNING id, payment_intent_id, amount, currency_code",
    )
    .bind(refund_id)
    .bind(now)
    .bind(code)
    .bind(&bounded_raw)
    .fetch_optional(&mut *conn)
    .await
    .map_err(crate::error::classify_write)
}

/// Inserts the `refunds` row, inside the caller's transaction.
///
/// Called only by [`Refunds::create`], which is what pairs it with the
/// reservation on the intent. It is `pub(crate)` for that reason and not
/// because the statement is dangerous on its own: a `refunds` row with no
/// matching `amount_refund_pending` is an intent that can be over-refunded,
/// and the pairing is the only thing that stops it.
///
/// `currency_code`, `merchant_id` and `charge_id` are the caller's *derived*
/// values, read off the intent this transaction has already locked — see
/// [`NewRefund`] for why none of them is a field a caller fills in.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] on a replayed `re_…`;
/// [`DbError::ForeignKeyViolation`] for an unknown intent, charge or
/// currency; [`DbError::Query`] otherwise, including
/// `metadata_is_object` for metadata that is not a JSON object.
async fn insert_in_tx(
    conn: &mut sqlx::PgConnection,
    new: &NewRefund,
    currency_code: &str,
    charge_id: Option<&str>,
) -> Result<RefundRow, DbError> {
    // `INSERT INTO refunds AS r` so the `RETURNING` list is `COLUMNS`
    // verbatim. The alias costs nothing and buys the property this module
    // already relies on everywhere else: one spelling of what a `RefundRow`
    // decodes, so the write and the two reads cannot drift apart.
    let sql = format!(
        "INSERT INTO refunds AS r \
             (id, payment_intent_id, charge_id, amount, currency_code, status, reason, \
              metadata, provider_reference_id) \
         VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7, $8) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, RefundRow>(AssertSqlSafe(sql))
        .bind(&new.id)
        .bind(&new.payment_intent_id)
        .bind(charge_id)
        .bind(new.amount)
        .bind(currency_code)
        .bind(new.reason.as_deref())
        .bind(&new.metadata)
        .bind(new.provider_reference_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(crate::error::classify_write)
}

/// Moves one `pending` refund to `canceled` **for this merchant**, inside the
/// caller's transaction.
///
/// # This one *does* carry a tenant predicate, unlike [`settle_in_tx`]
///
/// The difference is who supplied the id. A settlement is driven by a rail's
/// answer about a movement vpay itself initiated, so there is no
/// caller-chosen id to scope. A cancellation is a merchant naming a refund,
/// which is exactly the shape [`Refunds::get_for_merchant`] scopes — and the
/// scope is the same join through `payment_intents`, because `refunds` still
/// carries no `merchant_id` of its own.
///
/// `Ok(None)` folds "not yours", "no such refund" and "no longer pending"
/// into one answer, which is what stops a caller learning that another
/// tenant's refund exists.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails.
async fn cancel_in_tx(
    conn: &mut sqlx::PgConnection,
    merchant_id: &str,
    refund_id: &str,
    now: OffsetDateTime,
) -> Result<Option<RefundRow>, DbError> {
    // The tenant predicate is an `EXISTS` on the intent rather than a join in
    // the `FROM`: `UPDATE ... FROM` would make `payment_intents` a second
    // updatable relation in the statement, and the only thing this statement
    // may write is the refund.
    let sql = format!(
        "UPDATE refunds AS r \
         SET status = 'canceled', updated_at = $3 \
         WHERE r.id = $2 AND r.status = 'pending' \
           AND EXISTS (SELECT 1 FROM payment_intents p \
                       WHERE p.id = r.payment_intent_id AND p.merchant_id = $1) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, RefundRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(refund_id)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await
        .map_err(crate::error::classify_write)
}

#[async_trait::async_trait]
impl Refunds for crate::repository::PgRepositories {
    async fn create(
        &self,
        merchant_id: &str,
        new: &NewRefund,
    ) -> Result<Option<RefundRow>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // The reservation comes **first**, and the ordering is deliberate.
        // It is the statement that can be refused by `no_over_refund`, and a
        // refusal aborts the transaction — so doing it before the insert
        // means the refused case has written nothing anyone would have to
        // reason about. It is also the statement that locks the intent row,
        // which is what makes two concurrent creates serialize rather than
        // interleave.
        //
        // It is merchant-scoped in SQL. A handler cannot forget to filter,
        // and another tenant's intent is indistinguishable from a missing
        // one — this module's standing rule.
        let Some(intent) = crate::payment_intents::reserve_refund_in_tx(
            &mut tx,
            merchant_id,
            &new.payment_intent_id,
            new.amount,
        )
        .await?
        else {
            // Nothing was written, so the transaction is closed explicitly
            // rather than dropped: the connection returns to the pool
            // without waiting for a background rollback. Same reason
            // `crate::settlement`'s `Ok(None)` paths do it.
            tx.rollback().await.map_err(DbError::Query)?;
            return Ok(None);
        };

        // Both derived from the intent this transaction has just locked, and
        // neither from anything the caller passed: the currency because
        // `docs/flows/money.md` says a refund's currency is the intent's
        // carried verbatim, and the charge because it is the row a rail
        // movement — and, when this refund settles, a ledger posting — hangs
        // off. `None` is storable (migration `0017` made the column
        // nullable) and, for a `succeeded` intent, unreachable.
        let charge_id = crate::charges::id_for_intent_in_tx(&mut tx, &intent.id).await?;

        let row = insert_in_tx(&mut tx, new, &intent.currency_code, charge_id.as_deref()).await?;

        tx.commit().await.map_err(DbError::Query)?;

        tracing::info!(
            refund_id = %row.id,
            payment_intent_id = %row.payment_intent_id,
            amount = row.amount,
            currency_code = %row.currency_code,
            amount_refund_pending = intent.amount_refund_pending,
            "a refund was created and its amount reserved against the intent"
        );

        Ok(Some(row))
    }

    async fn cancel(&self, merchant_id: &str, id: &str) -> Result<Option<RefundRow>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;
        let now = OffsetDateTime::now_utc();

        let Some(row) = cancel_in_tx(&mut tx, merchant_id, id, now).await? else {
            tx.rollback().await.map_err(DbError::Query)?;
            return Ok(None);
        };

        // The reservation goes back. `Ok(None)` here is not a lost race — the
        // compare-and-swap above has already matched a `pending` refund, and
        // a `pending` refund whose amount is not reserved on its intent is an
        // invariant this crate is supposed to maintain — so it pages rather
        // than being reported as a merchant's problem, and the whole
        // transaction rolls back so the refund stays cancellable.
        crate::payment_intents::release_refund_in_tx(&mut tx, &row.payment_intent_id, row.amount)
            .await?
            .ok_or_else(|| DbError::WriteMatchedNoRow {
                table: "payment_intents",
                key: row.payment_intent_id.clone(),
            })?;

        tx.commit().await.map_err(DbError::Query)?;

        tracing::info!(
            refund_id = %row.id,
            payment_intent_id = %row.payment_intent_id,
            amount = row.amount,
            "a pending refund was canceled and its reservation released; nothing was posted, so \
             nothing was reversed"
        );

        Ok(Some(row))
    }

    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<RefundRow>, DbError> {
        // The tenant predicate is on the *joined* intent, which is the only
        // place it exists. `JOIN`, not `LEFT JOIN`: `payment_intent_id` is
        // `NOT NULL` and a foreign key, so a refund with no intent cannot
        // exist — and if one somehow did, answering `None` for it is the
        // right answer anyway, because there would be no tenant to attribute
        // it to.
        let sql = format!(
            "SELECT {COLUMNS} FROM refunds r \
             JOIN payment_intents p ON p.id = r.payment_intent_id \
             WHERE p.merchant_id = $1 AND r.id = $2"
        );

        sqlx::query_as::<_, RefundRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }
    async fn list_for_intent(
        &self,
        merchant_id: &str,
        payment_intent_id: &str,
    ) -> Result<Vec<RefundRow>, DbError> {
        // The same join and the same tenant predicate as `get_for_merchant`
        // above — see that method for why `refunds` has no `merchant_id` to
        // filter on directly.
        //
        // `ORDER BY r.created_at, r.id`: the id breaks a tie, because two
        // refunds of one intent written in the same transaction share a
        // timestamp and an unstable order would make a reloaded timeline
        // shuffle itself.
        let sql = format!(
            "SELECT {COLUMNS} FROM refunds r \
             JOIN payment_intents p ON p.id = r.payment_intent_id \
             WHERE p.merchant_id = $1 AND r.payment_intent_id = $2 \
             ORDER BY r.created_at, r.id"
        );

        sqlx::query_as::<_, RefundRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(payment_intent_id)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}
