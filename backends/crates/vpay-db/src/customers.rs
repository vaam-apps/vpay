//! The `customers` repository (`backends/migrations/0034_create-customers.sql`)
//! — the reads and writes behind `/v1/customers` and behind the twelve-month
//! retention sweep.
//!
//! It keeps [`crate::checkout_sessions`]' three rules unchanged — every
//! merchant-facing query is merchant-scoped **in SQL**, state changes are
//! compare-and-swap, and the one unscoped query is named for its caller — and
//! adds the one this table has and no other does:
//!
//! * **a customer is deleted, never flagged.** There is no `deleted_at`, no
//!   `@@soft_delete`, and no status column. `docs/flows/customers.md` says why
//!   in full: a row that says "this person asked to be forgotten" is still the
//!   record of that person.
//!
//! # The split between CrateStack and hand-written SQL, and what decides it
//!
//! Two of the seven methods below run through `schemas/vpay.cstack`'s
//! `model Customer` ([`Customers::touch_last_used`], [`Customers::delete`]);
//! five are hand-written `sqlx`. The line is drawn by one column —
//! `metadata JSONB NOT NULL` — for the two costs `model Customer`'s GAP note
//! measures, and by one query shape (the `seq` cursor's correlated subquery).
//! `docs/reference/vpay-db.md` §"`customers`" carries the argument; the
//! per-method docs below name which side each one is on and why.

use std::fmt;

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::AssertSqlSafe;
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};
use crate::persistence::{classify_cratestack, system_context};

/// The `.cstack` model the two CrateStack calls below name, for
/// [`crate::persistence::classify_cratestack`]'s `model` slot.
const MODEL: &str = "Customer";

/// Every column of `customers`, in one place so the statements below cannot
/// drift on the shape they decode into [`CustomerRow`].
const COLUMNS: &str = "id, seq, merchant_id, livemode, name, email, phone, metadata, \
                       last_used_at, created_at, updated_at";

/// The `type` of the event a swept deletion emits.
///
/// One of the three `customer.*` labels `type_is_a_documented_event` allows
/// — this one from migration `0034`, the other two from `0039` —
/// spelled here rather than passed in by the caller for
/// [`crate::checkout_sessions`]' reason: the event type is a property of
/// *which transition this is*, and a caller free to choose it could report a
/// deleted customer as an updated one. The `data` — the wire object, which
/// only `vpay-api` knows how to shape — is the caller's.
///
/// It is Stripe's own spelling, which is `docs/flows/webhooks.md`'s standing
/// rule, and it is the **only** way a merchant can learn about a hard delete:
/// a `GET` afterwards is byte-identical to a `GET` for an id that never
/// existed, so polling cannot tell them apart.
const EVENT_CUSTOMER_DELETED: &str = "customer.deleted";

/// One `customers` row, exactly as stored.
///
/// Not the wire object: `vpay-api` owns that shape (`created` as unix
/// seconds, `last_used_at` omitted entirely). This struct is deliberately
/// one-to-one with the table so a change to either is a compile error rather
/// than a silently dropped column.
///
/// `Debug` is **hand-written** below rather than derived, because
/// [`Self::name`], [`Self::email`] and [`Self::phone`] are a payer's personal
/// data — see that impl.
#[derive(Clone, PartialEq, sqlx::FromRow)]
pub struct CustomerRow {
    /// Public `cus_…` id, supplied by the caller before the insert.
    pub id: String,
    /// Pagination order. Database-generated, never written by this crate,
    /// and never exposed on the wire.
    pub seq: i64,
    /// The owning merchant. Every merchant-facing query here filters on it.
    pub merchant_id: String,
    /// Live or test money, copied from the deployment at creation.
    pub livemode: bool,
    /// The payer's name, or `None`. At least one of this,
    /// [`Self::email`] and [`Self::phone`] is present — migration `0034`'s
    /// `at_least_one_identifier`.
    pub name: Option<String>,
    /// The payer's email, or `None`.
    pub email: Option<String>,
    /// The payer's canonical MSISDN (`2376XXXXXXXX`, no `+`), or `None`.
    ///
    /// Present by default and with no opt-in anywhere: the maintainer's
    /// decision of 2026-09-05, recorded in `docs/flows/customers.md`.
    pub phone: Option<String>,
    /// The merchant's own key/value pairs, as stored. The
    /// `metadata_is_object` CHECK guarantees this is a JSON object.
    pub metadata: serde_json::Value,
    /// When this customer was last created, updated, or named by an intent
    /// or a session — the retention sweep's whole input.
    ///
    /// **Never rendered on the wire.** `vpay_api::model::CustomerObject`
    /// carries no field for it, and
    /// `every_documented_key_is_present_including_the_null_ones` is the
    /// tripwire that keeps it that way: it is an internal retention clock,
    /// and putting it on the object would invite a merchant to build on a
    /// value vpay moves for its own reasons.
    pub last_used_at: OffsetDateTime,
    /// When the customer was created, as supplied to [`NewCustomer`].
    pub created_at: OffsetDateTime,
    /// When the row last changed. Maintained by the writers here, not by a
    /// trigger.
    pub updated_at: OffsetDateTime,
}

/// Redacts the three identifier columns, leaving everything an operator
/// debugging this table actually needs.
///
/// A customer row is the one row in this crate whose *whole content* is
/// personal data a merchant collected from a payer. `CheckoutSessionRow`
/// redacts two credentials and prints the rest because the rest is the
/// merchant's own configuration; here the rest is somebody's name, email and
/// phone number, and `{:?}` on this struct appears in `tracing` fields, in
/// `anyhow` chains and in every failing assertion's output.
///
/// The lengths are printed rather than the values because they are what an
/// operator debugging `at_least_one_identifier` or `name_length` needs: which
/// of the three is present, and whether one of them is over its bound. That
/// is the question this impl exists for, and answering it needs no personal
/// data at all.
impl fmt::Debug for CustomerRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// `[N chars redacted]`, or `None` — so "which identifiers does this
        /// customer have?" is still answerable from a log line.
        fn redacted(value: Option<&String>) -> String {
            value.map_or_else(
                || "None".to_owned(),
                |value| format!("[{} chars redacted]", value.chars().count()),
            )
        }

        f.debug_struct("CustomerRow")
            .field("id", &self.id)
            .field("seq", &self.seq)
            .field("merchant_id", &self.merchant_id)
            .field("livemode", &self.livemode)
            .field("name", &format_args!("{}", redacted(self.name.as_ref())))
            .field("email", &format_args!("{}", redacted(self.email.as_ref())))
            .field("phone", &format_args!("{}", redacted(self.phone.as_ref())))
            // The keys, not the values: a merchant's metadata is theirs and
            // may hold anything, but "which keys are on this customer" is
            // what an operator needs and is the merchant's own vocabulary.
            .field(
                "metadata",
                &format_args!(
                    "{{{} key(s)}}",
                    self.metadata.as_object().map_or(0, serde_json::Map::len)
                ),
            )
            .field("last_used_at", &self.last_used_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// The columns a caller supplies when creating a customer: [`CustomerRow`]
/// minus `seq` and `updated_at`.
///
/// `last_used_at` is **not** a field, unlike `created_at`, and that is
/// deliberate rather than an omission: a customer's clock starts when it is
/// created, always, and a parameter would be a way to create one that is
/// already twelve months idle and therefore deletable by the next sweep.
/// `customers::insert_in_tx` sets it to `created_at` — named without a
/// link because it is `pub(crate)`, and rustdoc refuses a public link to a
/// private item ([`crate::settlement`]'s convention).
#[derive(Debug, Clone, PartialEq)]
pub struct NewCustomer {
    /// Public `cus_…` id, generated by `vpay_core::ids::customer_id` before
    /// the insert — never by the database, so a crash mid-insert still
    /// leaves a name to reconcile by.
    pub id: String,
    /// The owning merchant, from the authenticated client's mapping.
    pub merchant_id: String,
    /// From `config.deployment.livemode`; never inferred per request.
    pub livemode: bool,
    /// At least one of this, [`Self::email`] and [`Self::phone`] must be
    /// `Some` — the API refuses the all-absent request with a `400` naming
    /// all three parameters, and `at_least_one_identifier` is the backstop.
    pub name: Option<String>,
    /// See [`Self::name`].
    pub email: Option<String>,
    /// The canonical MSISDN, already canonicalised by the API. See
    /// [`Self::name`].
    pub phone: Option<String>,
    /// A JSON **object**; `metadata_is_object` refuses anything else.
    pub metadata: serde_json::Value,
    /// Creation instant, supplied by the caller — and also this customer's
    /// initial `last_used_at`. See the struct doc.
    pub created_at: OffsetDateTime,
}

/// The fields `POST /v1/customers/{id}` may change, in the wire's own
/// three-state shape.
///
/// # Why every field is an `Option<Option<…>>` and not an `Option<…>`
///
/// Stripe's update semantics have three answers per field and a merchant
/// depends on all three: *leave it alone* (the key is absent), *set it to
/// this* (`name=Ada`), and **clear it** (`name=`, the empty string). Only a
/// double option can carry them — `Option<String>` collapses "absent" and
/// "clear" into `None`, and a customer's email would then be unclearable
/// through the API that documents how to clear it.
///
/// This is the same shape `cratestack-macros` generates for a nullable
/// column on an `Update{Model}Input`
/// (`model/struct_only/field_definition.rs:74-84`), arrived at
/// independently and for the same reason. It is spelled by hand here because
/// this struct also carries `metadata`, which that input cannot
/// (`model Customer`'s GAP note).
///
/// `metadata` has only two states rather than three, and that is the wire
/// contract rather than an inconsistency: Stripe's `metadata` is merged
/// key-wise and cleared by sending `metadata[key]=` per key, so "absent"
/// means leave it and a present map is the new map. `vpay_api::v1::customers`
/// does the merge and hands the result down whole.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomerPatch {
    /// `None` = the request did not mention `name`. `Some(None)` = clear it.
    /// `Some(Some(v))` = set it to `v`.
    pub name: Option<Option<String>>,
    /// See [`Self::name`].
    pub email: Option<Option<String>>,
    /// See [`Self::name`].
    pub phone: Option<Option<String>>,
    /// `None` = the request did not mention `metadata`; `Some(map)` = the
    /// merged map to store. See the struct doc for why this one has two
    /// states and the others three.
    pub metadata: Option<serde_json::Value>,
}

impl CustomerPatch {
    /// Whether this patch would change nothing at all.
    ///
    /// The API uses it to answer a bodiless `POST /v1/customers/{id}` with
    /// the object unchanged rather than with an `UPDATE` that writes only
    /// `updated_at` — Stripe's own behaviour, and the one that keeps a
    /// merchant's no-op retry from moving a timestamp they are diffing on.
    ///
    /// ```
    /// use vpay_db::CustomerPatch;
    ///
    /// assert!(CustomerPatch::default().is_empty());
    ///
    /// // Clearing a field is a change, even though the value is `None`:
    /// // that is the whole reason the fields are double options.
    /// let clear_email = CustomerPatch {
    ///     email: Some(None),
    ///     ..CustomerPatch::default()
    /// };
    /// assert!(!clear_email.is_empty());
    /// ```
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.email.is_none()
            && self.phone.is_none()
            && self.metadata.is_none()
    }
}

/// One page request for [`Customers::list_page`].
///
/// The same shape and the same cursor rule as [`crate::ListPage`] — cursors
/// are public `cus_…` ids, never `seq` values. A type of its own rather than
/// a reuse of `ListPage` for [`crate::SessionListPage`]'s reason: `ListPage`
/// is the payment-intent list's contract, and this resource may grow a filter
/// that one would then silently ignore.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomerListPage {
    /// How many rows the caller wants. `vpay-api` applies the product limits
    /// (default 10, ceiling 100); this layer only refuses a non-positive
    /// limit, which Postgres would reject outright.
    pub limit: i64,
    /// Return customers strictly *older* than this id.
    pub starting_after: Option<String>,
    /// Return customers strictly *newer* than this id — scans ascending and
    /// is reversed in Rust, so `data` is newest-first either way.
    pub ending_before: Option<String>,
}

/// Inserts a customer **inside the caller's transaction**, returning the row
/// the database actually stored — `seq` and `updated_at` included, so a
/// caller never has to re-read to render its response.
///
/// # Why there is no pooled variant, since 2026-09-10 (issue #66)
///
/// `POST /v1/customers` emits `customer.created`, and that event has to
/// commit with the row or not at all — the rule every other outbox write in
/// this crate follows ([`crate::settlement`],
/// [`crate::CheckoutSessions::expire_due`], [`Customers::delete_idle`],
/// [`crate::payment_intents::cancel_in_tx`]). `Customers::create` used to be
/// one statement on the pool; deleting it rather than leaving it beside this
/// one makes "create a customer and tell nobody" inexpressible instead of
/// merely discouraged.
///
/// # Why this is one hand-written statement and not `create(..)`
///
/// `metadata` is `JSONB NOT NULL` with no `DEFAULT` and is not declared on
/// `model Customer`, so `CreateCustomerInput` has no field for it and the
/// generated `INSERT` would omit the column entirely — a `23502`, by
/// construction, which is exactly what migration `0034` left the `DEFAULT`
/// off to guarantee.
/// [`a_generated_customer_insert_cannot_carry_metadata`](self) pins the
/// rendered statement so an upstream change that made it expressible is
/// noticed rather than discovered.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] naming the primary key if `id` is already
/// taken — which cannot happen for a freshly minted `cus_…`.
/// [`DbError::Query`] for anything else, including `at_least_one_identifier`,
/// `name_length`, `email_length`, `phone_is_a_canonical_msisdn` and
/// `metadata_is_object`, every one of which the API refuses first with a
/// `400` naming the parameter — so reaching one here is a vpay bug rather
/// than a merchant's mistake.
pub(crate) async fn insert_in_tx(
    tx: &mut sqlx::PgConnection,
    new: &NewCustomer,
) -> Result<CustomerRow, DbError> {
    // `last_used_at` is `$8` twice over: it is `created_at`, always. See
    // `NewCustomer`'s own doc for why it is not a parameter.
    let sql = format!(
        "INSERT INTO customers (id, merchant_id, livemode, name, email, phone, metadata, \
         last_used_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
        .bind(&new.id)
        .bind(&new.merchant_id)
        .bind(new.livemode)
        .bind(new.name.as_deref())
        .bind(new.email.as_deref())
        .bind(new.phone.as_deref())
        .bind(&new.metadata)
        .bind(new.created_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(classify_write)
}

/// Reads one customer *for this merchant* and holds its row lock until the
/// caller's transaction ends — `SELECT … FOR UPDATE`.
///
/// # What the lock is for, and what it is not for
///
/// `POST /v1/customers/{id}` **merges** `metadata` key-wise (Stripe's
/// contract), so the new value is a function of the stored one and the write
/// is a read-modify-write. Until 2026-09-10 that read ran on the pool and the
/// window was real and documented: two concurrent updates each adding one key
/// could lose one of them, and the `customer.updated` event a merchant
/// received would describe a state the database no longer held. Taking the
/// row lock on the read is what closes it — the second transaction blocks
/// here, re-reads the *committed* merge, and merges onto that.
///
/// It is **not** a substitute for the tenancy filter, which is still in this
/// statement: a foreign `cus_…` answers `None`, the same as a missing one,
/// and this crate never compares two merchant ids in Rust.
///
/// It is also not a lock a *reader* takes. `Customers::get_for_merchant` is
/// unchanged and lock-free; only the path that is about to write on the
/// strength of what it read takes one, which is what keeps
/// `GET /v1/customers/{id}` off the update's critical section.
///
/// # Errors
///
/// [`DbError::Query`] if the read fails.
pub(crate) async fn lock_for_update(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
) -> Result<Option<CustomerRow>, DbError> {
    let sql =
        format!("SELECT {COLUMNS} FROM customers WHERE merchant_id = $1 AND id = $2 FOR UPDATE");

    sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::Query)
}

/// Applies a patch **inside the caller's transaction** and returns the row as
/// it now stands, or `None` if this merchant has no such customer.
///
/// # Why the merchant filter is in the statement and not a preceding read
///
/// A read-then-write would let a handler compare tenants in Rust, which is
/// the mistake this module exists to make inexpressible. The `UPDATE`
/// carries `WHERE merchant_id = $1 AND id = $2`, so a foreign customer
/// updates zero rows and `None` is the same answer a missing id gives. That
/// stays true even though the caller has already read the row through
/// [`lock_for_update`]: the guard belongs in the write.
///
/// `last_used_at` moves with the write: an update *is* a use (migration
/// `0034`'s definition), so a merchant who edits a customer every month never
/// has it swept.
///
/// An empty patch is refused **by the caller**, not here:
/// [`CustomerPatch::is_empty`] exists so `vpay-api` can answer a bodiless
/// `POST` with the object unchanged — and, since 2026-09-10, with **no**
/// `customer.updated`, because nothing changed. Reaching this method with one
/// would write `updated_at` and `last_used_at` and nothing else, which is
/// harmless and is not what the wire contract says happens.
///
/// # Why there is no pooled variant
///
/// [`insert_in_tx`]'s reason: the event commits with the write or not at all.
///
/// # Errors
///
/// [`DbError::Query`], including the same five CHECKs [`insert_in_tx`] lists.
pub(crate) async fn update_in_tx(
    tx: &mut sqlx::PgConnection,
    merchant_id: &str,
    id: &str,
    patch: &CustomerPatch,
    now: OffsetDateTime,
) -> Result<Option<CustomerRow>, DbError> {
    // Every column is assigned unconditionally and each assignment is
    // gated by its own `$n::BOOLEAN` "was this field mentioned?" flag,
    // rather than building a `SET` list from the fields that are present.
    //
    // That is the difference between one static statement and one whose
    // shape depends on the request. A built list would mean sixteen
    // possible statements, sixteen bind orders, and a `format!` whose
    // output is a function of caller input — which is exactly what
    // `AssertSqlSafe` and `crate::sql_audit` exist to stop this crate
    // doing. Here the `format!` interpolates two crate constants and
    // nothing else, and the three-state semantics live entirely in the
    // binds: flag false = leave it, flag true + NULL = clear it, flag
    // true + value = set it.
    let sql = format!(
        "UPDATE customers SET \
            name = CASE WHEN $3::BOOLEAN THEN $4::TEXT ELSE name END, \
            email = CASE WHEN $5::BOOLEAN THEN $6::TEXT ELSE email END, \
            phone = CASE WHEN $7::BOOLEAN THEN $8::TEXT ELSE phone END, \
            metadata = CASE WHEN $9::BOOLEAN THEN $10::JSONB ELSE metadata END, \
            last_used_at = GREATEST(last_used_at, $11), \
            updated_at = $11 \
         WHERE merchant_id = $1 AND id = $2 \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .bind(patch.name.is_some())
        .bind(patch.name.clone().flatten())
        .bind(patch.email.is_some())
        .bind(patch.email.clone().flatten())
        .bind(patch.phone.is_some())
        .bind(patch.phone.clone().flatten())
        .bind(patch.metadata.is_some())
        .bind(patch.metadata.clone())
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

#[async_trait::async_trait]
pub trait Customers: Send + Sync {
    /// Reads one customer *for this merchant*. `None` means "no such
    /// customer for you", which covers both a missing id and another
    /// merchant's id — see the module comment for why those two must be
    /// indistinguishable.
    ///
    /// Hand-written for `insert_in_tx`'s reason inverted (named without a
    /// link: it is `pub(crate)`): the row has
    /// to carry `metadata`, and the generated model struct has no field for
    /// it, so a CrateStack read could not render the wire object at all.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<CustomerRow>, DbError>;

    /// One page of this merchant's customers, newest first, plus whether more
    /// exist beyond it.
    ///
    /// Ordering, cursors and `has_more` work exactly as
    /// [`crate::PaymentIntents::list_page`]'s do — read that one for the
    /// direction-of-travel argument and for why an unknown cursor yields an
    /// empty page rather than the newest rows.
    ///
    /// # Why this stays raw SQL
    ///
    /// The cursor is `seq < (SELECT seq FROM customers WHERE id = $2 AND
    /// merchant_id = $1)` — a *correlated subquery*, which
    /// `cratestack::Filter` has no constructor for (`find_many`'s builder
    /// takes `where_`/`where_expr`/`where_any` over column comparisons only,
    /// `query/read/find_many.rs`). The two-statement alternative — resolve
    /// the cursor id to a `seq`, then filter on the literal — is a different
    /// query with a race in it: between the two reads the cursor row can be
    /// deleted, and this table's rows *are* deleted, by both
    /// `DELETE /v1/customers/{id}` and the retention sweep. The one-statement
    /// form degrades to an empty page there; the two-statement form would
    /// either error or silently page from the newest row.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn list_page(
        &self,
        merchant_id: &str,
        page: &CustomerListPage,
    ) -> Result<(Vec<CustomerRow>, bool), DbError>;

    /// Records that something used this customer, moving `last_used_at`
    /// forward to `now`.
    ///
    /// # This is the write the retention sweep reads
    ///
    /// "Used" is defined by migration `0034`'s column comment and means:
    /// created, updated, or named by a payment intent, a checkout session or
    /// an invoice (migration `0036` added the third).
    /// Every one of those call sites ends here, and a call site that is
    /// missing does not fail — it makes a live customer *look* idle, and the
    /// sweep deletes it twelve months later with nothing in any log saying
    /// anything unusual happened. That is why
    /// `a_customer_used_by_a_recent_intent_survives_the_sweep` exercises the
    /// confirm path rather than calling this method directly.
    ///
    /// # Monotonic, so a use can never move the clock backwards
    ///
    /// `now` is the *calling process's* instant and two vpay processes do not
    /// share a clock, so a plain assignment would let a server whose clock is
    /// a second behind rewind a customer's retention clock — and the sweep's
    /// horizon is twelve months, so a rewind is not a rounding error, it is
    /// the difference between a customer surviving a pass and not. This is
    /// also the reason migration `0034` carries no
    /// `CHECK (last_used_at >= created_at)`; see that migration.
    ///
    /// **The guard is the `WHERE`, not a `GREATEST`, and the distinction is
    /// worth stating here rather than only in the implementation.** This doc
    /// claimed `SET last_used_at = GREATEST(last_used_at, $2)` until
    /// 2026-09-07 and no such statement is ever rendered:
    /// `UpdateCustomerInput` renders a plain assignment and `cratestack`
    /// 0.11.1 has no way to express `GREATEST` in a `SET`. The monotonicity
    /// is enforced by `where_(last_used_at.lt(now))` instead — a stamp that
    /// would move the clock backwards matches zero rows. Observably the same,
    /// with one difference a caller can see: this returns `Ok(false)` for a
    /// backwards stamp, where a `GREATEST` assignment would return
    /// `Ok(true)`. That is why `Ok(false)` has three normal meanings below
    /// and not two. `a_retention_stamp_never_moves_a_customers_clock_backwards`
    /// proves the behaviour against a real Postgres; no unit test can, because
    /// `UpdateManySet::preview_sql` renders the predicate as the literal
    /// `<filters> AND <update_policy>`.
    ///
    /// # Unscoped, and named for it
    ///
    /// Deliberately no `merchant_id`. Its callers hold a customer id they
    /// have *already* resolved through [`Customers::get_for_merchant`] or
    /// through a foreign key on a row they own, so re-filtering by a tenant
    /// derived from that same id would be an authorisation check against
    /// itself — [`crate::CheckoutSessions::find_open_by_intent`]'s argument,
    /// unchanged.
    ///
    /// `Ok(false)` means no row moved, which is normal: the id may name a
    /// customer another request deleted between the resolve and this call.
    /// It is never an error, because a payment must not fail because a
    /// retention clock could not be stamped.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] wrapping a
    /// [`crate::PersistenceError::Denied`] if `model Customer` loses its
    /// `@@allow("update", …)`. **That refusal is silent**: `update_many`
    /// compiles the policy into the statement's own `WHERE`, so the arm's
    /// loss shows up as `Ok(false)` and not as this error — see the model's
    /// own comment, and `every_action_this_module_calls_has_an_allow_arm`.
    async fn touch_last_used(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError>;

    /// Hard-deletes one customer of this merchant's. `false` means there was
    /// no such customer for this merchant.
    ///
    /// # Why a hard delete, and why `delete_many`
    ///
    /// The object is personal data. `docs/flows/customers.md` carries the
    /// argument; the short form is that a soft-deleted customer is still the
    /// record of the person who asked to be forgotten, and a `deleted_at`
    /// column would make "erase this" a lie the schema tells.
    ///
    /// `delete_many().where_(id).where_(merchant_id)` and **not**
    /// `delete(pk)`, for [`crate::DisabledClients::enable_client`]'s measured
    /// reason: `delete_exec.rs` turns "matched no row" into
    /// `CratestackError::Forbidden`, so the single-row builder cannot tell
    /// "not yours" from "the policy refused" — and here the first of those is
    /// an ordinary `404` and the second is a page-an-operator bug. It is also
    /// the only builder that can carry the tenant filter at all.
    ///
    /// # What refuses this, and why that refusal is the point
    ///
    /// `payment_intents.customer_id` and `checkout_sessions.customer_id` are
    /// foreign keys with `NO ACTION`, so a customer with any payment history
    /// **cannot** be deleted and this returns
    /// [`DbError::Persistence`] wrapping [`crate::PersistenceError::ForeignKey`].
    /// The API turns that into a `409` explaining it. That is the deliberate
    /// trade migration `0034` records: a merchant's payment record survives,
    /// and "delete this customer" is therefore not a complete erasure of the
    /// payer. `docs/flows/customers.md` states it plainly rather than
    /// implying otherwise.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::ForeignKey`]
    /// when an intent or a session references the customer,
    /// [`crate::PersistenceError::Denied`] if the model loses its
    /// `@@allow("delete", …)` (silent: see [`Customers::touch_last_used`]),
    /// [`crate::PersistenceError::Backend`] otherwise.
    async fn delete(&self, merchant_id: &str, id: &str) -> Result<bool, DbError>;

    /// Every customer idle since before `horizon` that **nothing references**,
    /// oldest first, at most `limit` of them — the retention sweep's backlog
    /// query.
    ///
    /// The **read** half of the sweep, split from the delete for
    /// [`crate::CheckoutSessions::due_for_expiry`]'s reason: the
    /// `customer.deleted` event's `data` is the *rendered* wire object, which
    /// only `vpay-api` knows how to shape, so the caller has to hold the row
    /// before the write that describes it.
    ///
    /// # Not tenant-scoped, because its caller is a sweep
    ///
    /// It selects a *time*, not a tenant — exactly as `due_for_expiry` does.
    ///
    /// # It carries the same reference guard the delete does
    ///
    /// `NOT EXISTS` over `payment_intents`, `checkout_sessions` and — since
    /// migration `0036` — `invoices`.
    /// Duplicated rather than left to the delete alone so a referenced
    /// customer is never *rendered* either: rendering it would mint an
    /// `evt_…` and build an object claiming a payer's record had been erased,
    /// which the delete would then correctly refuse — work done for nothing,
    /// and one more place a future change could leak personal data out of.
    ///
    /// The guard is `NOT EXISTS` rather than a reliance on the foreign key
    /// because the two say different things. The FK refuses *any* reference,
    /// however old; this refuses any reference, full stop, and the pair is
    /// what makes the sweep's contract — "an unreferenced customer nobody has
    /// used for twelve months" — one sentence with two enforcers rather than
    /// an error path.
    ///
    /// # Why this stays raw SQL
    ///
    /// `NOT EXISTS (SELECT 1 FROM payment_intents WHERE customer_id =
    /// customers.id)` is a correlated subquery over a *different table*, and
    /// `cratestack::Filter` compares columns of the model's own table. There
    /// is no relation declared on `model Customer` to side-load either
    /// (`payment_intents` is not modelled, and `@relation` needs both ends).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn idle_since(
        &self,
        horizon: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<CustomerRow>, DbError>;

    /// Deletes one idle, unreferenced customer **and** appends the
    /// `customer.deleted` event that tells its merchant so — in one
    /// transaction. `Ok(false)` means it was no longer eligible.
    ///
    /// The other half of [`Customers::idle_since`], and the same argument
    /// [`crate::CheckoutSessions::expire_due`] makes for being one function
    /// rather than two calls: a crash between the delete and the event would
    /// erase a merchant's customer with nobody ever told, and there is no
    /// sweep over "customers deleted without an event" and no way to build
    /// one, because the row that would prove it is gone. One transaction
    /// makes that window not exist.
    ///
    /// # The guard is the statement, and it is re-evaluated here
    ///
    /// `last_used_at < horizon` **and** the `NOT EXISTS` pair, re-checked
    /// inside this transaction rather than trusted from
    /// [`Customers::idle_since`]. A merchant can create an intent for the
    /// customer between the read and the write, and a customer a payment was
    /// just taken from must not be erased on the strength of a read taken
    /// before it. `Ok(false)` is the **normal** answer for that, and for a
    /// concurrent sweep, and for a merchant who deleted it by hand — and no
    /// event is written on that path, which is what makes a second sweep
    /// produce no second `customer.deleted`.
    ///
    /// # `horizon` is the caller's
    ///
    /// For [`crate::CheckoutSessions::expire_due`]'s reason: twelve months is
    /// a **product** rule the worker owns
    /// (`vpay_worker::handlers::CUSTOMER_IDLE_AFTER`), and both sides of the
    /// comparison belonging to one layer is what keeps it one rule. It also
    /// means a test can sweep a horizon in the future instead of rewriting a
    /// stored timestamp.
    ///
    /// # Errors
    ///
    /// [`DbError::UniqueViolation`] on `events_pkey` if `event_id` has
    /// already been emitted. [`DbError::Query`] if any statement or the
    /// commit fails — including an `event_data` that is not a JSON object
    /// (`data_is_object`) or a `type` outside the documented vocabulary, both
    /// of which are vpay bugs. **The transaction is rolled back either way,
    /// so the customer survives and the next sweep retries it.**
    async fn delete_idle(
        &self,
        id: &str,
        horizon: OffsetDateTime,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<bool, DbError>;
}

/// `time::OffsetDateTime` (vpay's convention for every TIMESTAMPTZ this crate
/// binds) as the `chrono::DateTime<Utc>` CrateStack's generated inputs and
/// filters take.
///
/// The **second** place this crate crosses the chrono/time boundary, and the
/// first in this direction: `client_assertion::chrono_to_offset_date_time`
/// converts the other way for `authkestra-op`, and its comment carries the
/// argument that the two types model the same instant so the conversion is a
/// reinterpretation rather than an approximation.
///
/// # Why the fallback can never be taken, and why it is `MAX_UTC` anyway
///
/// `chrono::DateTime::<Utc>::from_timestamp` answers `None` only outside
/// chrono's own range, which is roughly ±262,000 years from the epoch.
/// `time::OffsetDateTime` (default features, as this workspace builds it)
/// spans years −9999..=9999 — four orders of magnitude narrower — so **no
/// value of the argument type can reach the `unwrap_or`**.
/// [`the_chrono_conversion_is_total_over_every_instant_time_can_hold`](self)
/// proves that at both extremes of the domain rather than asserting it here.
///
/// ADR-0007 denies `expect` in production code, so an unreachable branch
/// still has to answer something, and *which* answer is not arbitrary. Both
/// callers of this function are [`Customers::touch_last_used`]'s, and
/// `MAX_UTC` in either slot means "this customer is freshly used": the filter
/// `last_used_at < MAX` matches, and the assignment writes an instant no
/// horizon is ever after. So the unreachable branch fails in the direction
/// that **keeps** a merchant's personal-data record rather than the one that
/// deletes it. `UNIX_EPOCH` — the obvious other choice — would do the
/// opposite: it would stamp a live customer as maximally idle and hand it to
/// the next sweep.
fn to_chrono(at: OffsetDateTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(at.unix_timestamp(), at.nanosecond())
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
}

/// The `NOT EXISTS` pair that makes a customer sweepable, shared by
/// [`Customers::idle_since`] and [`Customers::delete_idle`] so the read's
/// guard and the write's cannot drift.
///
/// A `const` rather than a function taking the outer query's alias, and that
/// is a constraint from `crate::sql_audit` rather than a preference: every
/// `{…}` a statement interpolates has to resolve to a `const …: &str` in this
/// crate, so a computed fragment — however fixed its inputs — fails the audit
/// by construction. Both call sites spell the table `customers`, so there was
/// no alias to parameterise anyway; the first draft took one, and the gate is
/// what pointed out that it did not need to.
///
/// **Invoices joined this list on 2026-09-07** (migration `0036`), and they
/// are the reason to re-read this constant rather than skim it. This comment
/// said "invoices are the third object that would belong here and do not
/// exist" while that was true; the clause below is what makes it stop being
/// a comment.
///
/// Without the third `NOT EXISTS`, nothing breaks *loudly*:
/// `invoices.customer_id` is a `NO ACTION` foreign key, so `delete_idle`
/// would still be refused by Postgres. What would happen instead is that
/// [`Customers::idle_since`] would keep handing the sweep a customer it can
/// never delete — minting an `evt_…` and building a `customer.deleted` object
/// for a payer whose record is not going anywhere, once an hour, forever,
/// with the failure arriving as a `23503` inside a transaction that rolls
/// back. `an_invoiced_customer_is_never_offered_to_the_sweep` is the test.
const UNREFERENCED: &str = "NOT EXISTS (SELECT 1 FROM payment_intents WHERE customer_id = customers.id) \
     AND NOT EXISTS (SELECT 1 FROM checkout_sessions WHERE customer_id = customers.id) \
     AND NOT EXISTS (SELECT 1 FROM invoices WHERE customer_id = customers.id)";

#[async_trait::async_trait]
impl Customers for crate::repository::PgRepositories {
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<CustomerRow>, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM customers WHERE merchant_id = $1 AND id = $2");

        sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn list_page(
        &self,
        merchant_id: &str,
        page: &CustomerListPage,
    ) -> Result<(Vec<CustomerRow>, bool), DbError> {
        let limit = page.limit.max(1);
        let backwards = page.ending_before.is_some();
        let direction = if backwards { "ASC" } else { "DESC" };

        // The cursor subqueries are merchant-scoped exactly as
        // `payment_intents::list_page`'s are, so a cursor from another tenant
        // resolves to NULL rather than to a position in their range.
        let sql = format!(
            "SELECT {COLUMNS} FROM customers \
             WHERE merchant_id = $1 \
               AND ($2::TEXT IS NULL \
                    OR seq < (SELECT seq FROM customers \
                              WHERE id = $2 AND merchant_id = $1)) \
               AND ($3::TEXT IS NULL \
                    OR seq > (SELECT seq FROM customers \
                              WHERE id = $3 AND merchant_id = $1)) \
             ORDER BY seq {direction} \
             LIMIT $4"
        );

        // One more than asked for, so `has_more` is a fact about the database
        // rather than a guess — `payment_intents::list_page`'s device.
        let mut rows = sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(page.starting_after.as_deref())
            .bind(page.ending_before.as_deref())
            .bind(limit.saturating_add(1))
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)?;

        let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        // `ending_before` scanned ascending to find the rows *nearest* the
        // cursor; the page itself is newest-first either way.
        if backwards {
            rows.reverse();
        }

        Ok((rows, has_more))
    }

    async fn touch_last_used(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError> {
        // THROUGH CRATESTACK. `update_many` and not `update(pk)`, for
        // `webhook_deliveries::mark_fanned_out_in_tx`'s reason: `update` by
        // primary key has nowhere to put a guard, and this one needs
        // `GREATEST` semantics that the generated `SET` cannot express...
        //
        // — which it cannot, and that is worth being exact about rather than
        // glossing. `UpdateCustomerInput` renders `last_used_at = $n`, a
        // plain assignment. The `GREATEST` this method's doc promises is
        // therefore enforced by the *filter* instead: `last_used_at < now`
        // means a stamp that would move the clock backwards matches zero rows
        // and is a no-op, which is the same observable behaviour and is the
        // only form the generated statement can take. `Ok(false)` then covers
        // one more normal case — "already stamped at or after this instant" —
        // and the doc's "no row moved is normal" is why that is safe.
        let cs = &self.cs;
        let summary = cs
            .customer()
            .update_many()
            .where_(crate::schema::cratestack_schema::customer::id().eq(id.to_owned()))
            .where_(crate::schema::cratestack_schema::customer::last_used_at().lt(to_chrono(now)))
            .set(crate::schema::cratestack_schema::UpdateCustomerInput {
                last_used_at: Some(to_chrono(now)),
                ..crate::schema::cratestack_schema::UpdateCustomerInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        // `BatchSummary::ok` is `updated.len()` from the statement's own
        // `RETURNING` (`update_many_exec.rs:126-131`), so it is the number
        // `rows_affected()` would have given. `id` is the primary key, so
        // this is 0 or 1 and never more.
        Ok(summary.ok == 1)
    }

    async fn delete(&self, merchant_id: &str, id: &str) -> Result<bool, DbError> {
        // THROUGH CRATESTACK. Both filters are in the statement: the tenant
        // one is what makes another merchant's `cus_…` answer the same 404 a
        // missing one does, without this crate ever comparing two merchant
        // ids in Rust.
        let cs = &self.cs;
        let summary = cs
            .customer()
            .delete_many()
            .where_(crate::schema::cratestack_schema::customer::id().eq(id.to_owned()))
            .where_(
                crate::schema::cratestack_schema::customer::merchant_id()
                    .eq(merchant_id.to_owned()),
            )
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))?;

        Ok(summary.ok == 1)
    }

    async fn idle_since(
        &self,
        horizon: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<CustomerRow>, DbError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM customers \
             WHERE last_used_at < $1 AND {UNREFERENCED} \
             ORDER BY last_used_at ASC \
             LIMIT $2"
        );

        sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
            .bind(horizon)
            .bind(limit.max(1))
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn delete_idle(
        &self,
        id: &str,
        horizon: OffsetDateTime,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        let sql = format!(
            "DELETE FROM customers \
             WHERE id = $1 AND last_used_at < $2 AND {UNREFERENCED} \
             RETURNING merchant_id, livemode"
        );

        // The merchant and livemode come back from the delete itself rather
        // than from a preceding read, and that is the whole reason this is a
        // `RETURNING`: the event has to be stamped with the tenant of the row
        // that was actually removed, and a value carried from
        // `idle_since`'s page would be a value read before the guard was
        // re-evaluated.
        let Some((merchant_id, livemode)) = sqlx::query_as::<_, (String, bool)>(AssertSqlSafe(sql))
            .bind(id)
            .bind(horizon)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?
        else {
            // No longer eligible. The transaction is dropped, which rolls it
            // back; nothing was written, and no event describes a deletion
            // that did not happen.
            return Ok(false);
        };

        // The event, in the same transaction. `insert_in_tx` is
        // `crate::events`' own hand-written statement — the one `model Event`
        // records as blocked on `events.data` being JSONB — so this reaches
        // it through the free function rather than through `TxRepositories`,
        // which is a trait a caller outside this crate holds.
        crate::events::insert_in_tx(
            &mut tx,
            &crate::NewEvent {
                id: event_id.to_owned(),
                merchant_id,
                livemode,
                event_type: EVENT_CUSTOMER_DELETED.to_owned(),
                object_id: id.to_owned(),
                data: event_data.clone(),
            },
        )
        .await?;

        tx.commit().await.map_err(DbError::Query)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    //! No database. Everything here is either a render of a statement
    //! (`preview_sql` does no I/O) or a question about the **compiled**
    //! `ModelDescriptor`, which is `schemas/vpay.cstack` as rustc saw it.
    //!
    //! `backends/crates/vpay-db/tests/repositories.rs` is where a real
    //! Postgres proves the behaviour these assertions only imply, and
    //! `backends/tests/integration/tests/postgres_smoke.rs` is where
    //! migration `0034`'s constraints are proved to fire.

    use sqlx::postgres::PgPoolOptions;

    use super::{CustomerPatch, UNREFERENCED, to_chrono};

    /// A pool that has never opened a connection, and cannot: the port is
    /// unroutable. `connect_lazy` does no I/O, and neither does
    /// `preview_sql`. [`crate::disabled_clients`]' device.
    fn lazy_cratestack() -> crate::schema::cratestack_schema::Cratestack {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its URL and connects to nothing");
        crate::schema::cratestack_schema::Cratestack::builder(pool).build()
    }

    /// The generated create input has **no `metadata` field**, which is what
    /// keeps [`super::insert_in_tx`] a hand-written statement.
    ///
    /// This is the tripwire migration `0034` left the column's `DEFAULT` off
    /// to arm. Two things would move it, and each should:
    ///
    /// * declaring `metadata Json` on `model Customer` — the rendered
    ///   statement would name the column, this fails, and whoever did it has
    ///   to re-read the model's GAP note about
    ///   `Value::from_plain_json`'s number demotion before deleting this
    ///   test;
    /// * an upstream CrateStack that learned to carry an undeclared column,
    ///   which is not a thing 0.11.1 does and would be worth knowing about.
    ///
    /// It asserts the *absence* of a column from a rendered `INSERT`, which
    /// is a weaker signal than asserting a presence — so the second half
    /// asserts the statement is the one this test thinks it is looking at,
    /// and would fail loudly if `preview_sql` ever rendered something else
    /// entirely.
    #[tokio::test]
    async fn a_generated_customer_insert_cannot_carry_metadata() {
        let cs = lazy_cratestack();
        let input = crate::schema::cratestack_schema::CreateCustomerInput {
            id: "cus_0123456789abcdefghjkmnpq".to_owned(),
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            name: None,
            email: None,
            phone: Some("237600000200".to_owned()),
            last_used_at: super::to_chrono(time::OffsetDateTime::UNIX_EPOCH),
        };

        let sql = cs.customer().create(input).preview_sql();

        // The `RETURNING` clause names every column of the model, `seq` and
        // the two timestamps included, so the assertions below have to be
        // about the **insert column list** alone. Splitting on `RETURNING`
        // rather than substring-searching the whole statement is what makes
        // this test say what it means; the first version of it did not, and
        // failed on its own `RETURNING`.
        let (written, returned) = sql
            .split_once(" RETURNING ")
            .expect("the generated insert returns the model projection");

        assert!(
            written.starts_with("INSERT INTO customers ("),
            "this test is no longer looking at a customers insert: {sql}"
        );
        assert!(
            !written.contains("metadata"),
            "`metadata` became expressible through the generated input. Before moving \
             `customers::insert_in_tx` onto it, re-read `model Customer`'s GAP note: \
             `Value::from_plain_json` demotes any JSON number outside i64 to f64, and this \
             column is merchant-authored and is echoed back verbatim. Statement: {sql}"
        );
        assert!(
            !written.contains("seq"),
            "`seq` is GENERATED ALWAYS; an insert that named it would be refused outright. \
             `@default(dbgenerated())` on `model Customer` is what keeps it out: {sql}"
        );
        // `created_at`/`updated_at` carry `@default(now())`, so the generated
        // input drops them and the column defaults fire — the same property
        // `disabled_clients`' upsert test pins, and the reason those two
        // columns keep their `DEFAULT now()` while no other column has one.
        assert!(
            !written.contains("created_at") && !written.contains("updated_at"),
            "`created_at`/`updated_at` must be left to their column defaults: {sql}"
        );
        // And the split really did separate the two halves — otherwise every
        // assertion above would be vacuously true on an empty string.
        assert!(
            returned.contains("seq") && returned.contains("created_at"),
            "the RETURNING projection is not the model's: {sql}"
        );
        assert!(
            !returned.contains("metadata"),
            "an undeclared column cannot be in the model projection; if it is, the model \
             now declares `metadata` and the whole GAP note is stale: {sql}"
        );
    }

    /// `touch_last_used` assigns `last_used_at` and **no other column**.
    ///
    /// This is what stops a retention stamp from being a covert full update.
    /// `UpdateCustomerInput::default()` leaves every other field `None`, and
    /// `update_sql_value` emits nothing for a `None`
    /// (`cratestack-macros/src/shared/sql.rs:37-44`). If that ever changed —
    /// an upstream that rendered every field, a `Default` that stopped being
    /// all-`None` — a stamp on the **confirm path** would start writing NULL
    /// over a merchant's payer name, and nothing else in this repository
    /// would notice: the stamp's return value is a row count that would be
    /// exactly as correct as before.
    ///
    /// # What this test cannot see, and where that is proved instead
    ///
    /// `UpdateManySet::preview_sql` renders the predicate as the literal
    /// string `<filters> AND <update_policy>`
    /// (`cratestack-sqlx/src/query/write/update_many.rs:79-89`), so **no
    /// assertion here can say anything about the `id` filter or the
    /// `last_used_at <` filter**. The first version of this test asserted
    /// `sql.contains("last_used_at <")` and failed for that reason; a
    /// `contains("merchant_id")` would have *passed* against the `RETURNING`
    /// projection, which is worse. Both filters are proved by behaviour in
    /// `vpay-db/tests/repositories.rs`
    /// (`a_retention_stamp_never_moves_a_customers_clock_backwards`).
    #[tokio::test]
    async fn the_retention_stamp_moves_one_column_and_never_backwards() {
        let cs = lazy_cratestack();

        let sql = cs
            .customer()
            .update_many()
            .where_(
                crate::schema::cratestack_schema::customer::id()
                    .eq("cus_0123456789abcdefghjkmnpq".to_owned()),
            )
            .where_(
                crate::schema::cratestack_schema::customer::last_used_at()
                    .lt(super::to_chrono(time::OffsetDateTime::UNIX_EPOCH)),
            )
            .set(crate::schema::cratestack_schema::UpdateCustomerInput {
                last_used_at: Some(super::to_chrono(time::OffsetDateTime::UNIX_EPOCH)),
                ..crate::schema::cratestack_schema::UpdateCustomerInput::default()
            })
            .preview_sql();

        assert!(
            sql.starts_with("UPDATE customers SET last_used_at = $1"),
            "a retention stamp must assign `last_used_at` and nothing else: {sql}"
        );
        for column in ["name", "email", "phone", "merchant_id", "livemode"] {
            // `concat`, not `format!`, deliberately. `crate::sql_audit`
            // finds statement-building `format!`s by looking for the word
            // `sql` within forty characters *before* the macro, and every
            // line in this loop has `sql` in it — so a `format!` here is
            // read as a statement and the audit fails on `{column}`,
            // reporting a SQL-injection risk in an assertion that touches no
            // database. Measured twice: moving the `format!` out of the
            // `contains(..)` argument was not enough, because the previous
            // line's `{sql}` is still inside the lookback window.
            let assignment = [column, " = "].concat();
            assert!(
                !sql.contains(&assignment),
                "the retention stamp assigns `{column}`; a stamp on the confirm path would \
                 overwrite a merchant's payer data: {sql}"
            );
        }
        assert!(
            sql.contains("<filters> AND <update_policy>"),
            "the preview no longer elides the predicate, so this test's own doc about what \
             it cannot see is stale — and an assertion about the filters may now be \
             possible here: {sql}"
        );
    }

    /// `DELETE /v1/customers/{id}` really deletes: the statement is a
    /// `DELETE`, not a soft-delete `UPDATE`.
    ///
    /// `DeleteMany::preview_sql` branches on
    /// `descriptor.soft_delete_column` and renders
    /// `UPDATE customers SET <col> = NOW()` when the model carries
    /// `@@soft_delete` (`cratestack-sqlx/src/query/write/delete_many.rs:53-64`).
    /// So this assertion is a pin on the **absence** of that block attribute
    /// from `model Customer`, expressed as the thing that actually reaches
    /// Postgres.
    ///
    /// It matters more here than the phrasing suggests. Adding `@@soft_delete`
    /// is a one-line schema edit that changes no Rust, compiles, passes
    /// `check-schema`, and turns every erasure of a payer's personal data into
    /// a flag on a row that keeps it — while `DELETE /v1/customers/{id}` goes
    /// on answering `{deleted: true}` and `GET` goes on answering 404,
    /// because the policy clause hides the flagged row. A merchant would be
    /// told the data was erased, a payer would have been told the same, and
    /// the name, email and phone number would still be there.
    ///
    /// # What this test cannot see
    ///
    /// The `WHERE` is rendered as `<filters> AND <delete_policy>`, exactly as
    /// [`the_retention_stamp_moves_one_column_and_never_backwards`](self)
    /// records for updates, so **the tenant filter is not observable here**.
    /// A `contains("merchant_id")` would pass against the `RETURNING`
    /// projection whether or not the filter existed, which is a test that
    /// asserts nothing. It is proved by behaviour in
    /// `vpay-db/tests/repositories.rs`
    /// (`another_merchants_customer_cannot_be_deleted`).
    #[tokio::test]
    async fn a_customer_delete_is_a_delete_and_not_a_soft_delete() {
        let cs = lazy_cratestack();

        let sql = cs
            .customer()
            .delete_many()
            .where_(
                crate::schema::cratestack_schema::customer::id()
                    .eq("cus_0123456789abcdefghjkmnpq".to_owned()),
            )
            .where_(
                crate::schema::cratestack_schema::customer::merchant_id()
                    .eq("acme-cameroon-tenant".to_owned()),
            )
            .preview_sql();

        assert!(
            sql.starts_with("DELETE FROM customers WHERE "),
            "`model Customer` grew `@@soft_delete`. A customer is personal data and vpay \
             promises a hard delete on /v1 and in docs/flows/customers.md; a flagged row is \
             still the record of the person who asked to be forgotten. Statement: {sql}"
        );
        assert!(
            !sql.contains("SET "),
            "a delete that assigns anything is a soft delete wearing a DELETE's name: {sql}"
        );
    }

    /// Every action this module calls on `Customer` has an `@@allow` arm,
    /// asserted against the **compiled descriptor**, with no database.
    ///
    /// [`crate::disabled_clients`]' measurement is what this exists for and
    /// it applies here with more force, because *both* of this model's arms
    /// are silent at runtime: `update_many` and `delete_many` compile their
    /// policy into the statement's own `WHERE`
    /// (`push_action_policy_query` renders the literal `FALSE` for an empty
    /// slot), so a deleted arm raises nothing at all. It matches zero rows,
    /// returns `Ok`, and the two consequences are:
    ///
    /// * `update` — every customer's `last_used_at` freezes at creation and
    ///   the retention sweep deletes live customers twelve months later;
    /// * `delete` — `DELETE /v1/customers/{id}` answers `{deleted: true}`
    ///   while the row stays, telling a merchant personal data was erased
    ///   when it was not.
    ///
    /// Neither is visible to `cargo build`, to `just clippy`, to
    /// `check-schema` or to any of the ten `verify` gates. This test is what
    /// makes them visible in milliseconds instead of in a container.
    ///
    /// It also asserts the *absence* of the three arms this model
    /// deliberately does not grant, which is the direction a wildcard would
    /// have hidden: `@@allow("all", …)` would grant `create` and `read`, and
    /// nothing in this crate calls either, so the model would be carrying a
    /// permission no test could measure.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use crate::schema::cratestack_schema::models::CUSTOMER_MODEL as model;

        assert!(
            !model.update_allow_policies.is_empty(),
            "`@@allow(\"update\", auth().isSystem())` is missing from `model Customer`: \
             `touch_last_used` would match zero rows and return Ok(false) forever, every \
             customer's retention clock would freeze at creation, and `sweep_idle_customers` \
             would delete live customers twelve months later with nothing in any log \
             saying so"
        );
        assert!(
            !model.delete_allow_policies.is_empty(),
            "`@@allow(\"delete\", auth().isSystem())` is missing from `model Customer`: \
             `delete_many` compiles its policy into the WHERE, so `DELETE /v1/customers/{{id}}` \
             would answer `{{deleted: true}}` while the row stayed — vpay telling a merchant \
             personal data was erased when it was not"
        );

        assert!(
            model.create_allow_policies.is_empty()
                && model.read_allow_policies.is_empty()
                && model.detail_allow_policies.is_empty(),
            "`model Customer` grew a `create` or `read` arm. Nothing in this crate creates or \
             reads a customer through CrateStack — `create`, `get_for_merchant` and \
             `list_page` are hand-written because they carry `metadata` — so an arm here is a \
             permission nothing can measure. Grant it in the commit that moves the query"
        );

        for (slot, denies) in [
            ("read", model.read_deny_policies),
            ("detail", model.detail_deny_policies),
            ("create", model.create_deny_policies),
            ("update", model.update_deny_policies),
            ("delete", model.delete_deny_policies),
        ] {
            assert!(
                denies.is_empty(),
                "`model Customer` grew a `@@deny(\"{slot}\", …)`; it overrides every \
                 `@@allow` for that action and no call site in this crate expects one"
            );
        }
    }

    /// The sweep's guard names **every** referencing table, and correlates to
    /// the alias its caller uses.
    ///
    /// One function rather than two copies because
    /// [`super::Customers::idle_since`] and
    /// [`super::Customers::delete_idle`] have to agree exactly: a table in
    /// the read's guard and not the write's would render a
    /// `customer.deleted` object for a customer the write then refuses to
    /// delete, and a table in the write's and not the read's would make the
    /// sweep spend a transaction per pass on rows it can never remove.
    ///
    /// **This is the assertion that caught the invoice omission.** It said
    /// "two referencing tables today. A third — invoices, when they exist —
    /// belongs here" until 2026-09-07, and migration `0036` is when that
    /// stopped being a forecast. The set is closed at three now, and the
    /// count is what a fourth referencing table has to walk past.
    #[test]
    fn the_sweep_guard_names_every_table_that_can_reference_a_customer() {
        assert!(UNREFERENCED.contains("FROM payment_intents WHERE customer_id = customers.id"));
        assert!(UNREFERENCED.contains("FROM checkout_sessions WHERE customer_id = customers.id"));
        assert!(UNREFERENCED.contains("FROM invoices WHERE customer_id = customers.id"));
        assert_eq!(
            UNREFERENCED.matches("NOT EXISTS").count(),
            3,
            "three referencing tables since migration 0036. A fourth belongs here and in that \
             migration's foreign keys, in the same change: {UNREFERENCED}"
        );
    }

    /// [`super::to_chrono`]'s `unwrap_or` is unreachable, proved at both
    /// extremes of its domain rather than argued in a comment.
    ///
    /// This is the assertion that lets the fallback exist at all under
    /// ADR-0007's denial of `expect`. If a future `time` gained the
    /// `large-dates` feature — which widens `OffsetDateTime` to ±999,999
    /// years, past chrono's ±262,000 — the first two assertions go red and
    /// the conversion has to become fallible rather than silently start
    /// clamping a retention clock.
    #[test]
    fn the_chrono_conversion_is_total_over_every_instant_time_can_hold() {
        // `time::Date`'s own bounds, which are `OffsetDateTime`'s: years
        // -9999..=9999 without the `large-dates` feature. Built rather than
        // read off a constant because `OffsetDateTime` publishes none.
        let extreme = |year, month, day, time| {
            time::Date::from_calendar_date(year, month, day)
                .expect("a year inside time::Date's own range")
                .with_time(time)
                .assume_utc()
        };
        for (label, instant) in [
            (
                "MIN",
                extreme(-9999, time::Month::January, 1, time::Time::MIDNIGHT),
            ),
            (
                "MAX",
                extreme(9999, time::Month::December, 31, time::Time::MIDNIGHT),
            ),
            ("epoch", time::OffsetDateTime::UNIX_EPOCH),
        ] {
            assert!(
                chrono::DateTime::<chrono::Utc>::from_timestamp(
                    instant.unix_timestamp(),
                    instant.nanosecond()
                )
                .is_some(),
                "`to_chrono`'s unreachable fallback became reachable at {label}: every \
                 retention stamp would clamp to MAX_UTC and no customer would ever be swept"
            );
        }

        // And the ordinary case is exact, not merely representable: the
        // filter and the assignment are compared against a stored
        // TIMESTAMPTZ, so a conversion that lost the sub-second component
        // would make a stamp and its own read-back disagree.
        let at = time::OffsetDateTime::from_unix_timestamp(1_757_000_000)
            .expect("a fixed, in-range instant")
            .replace_nanosecond(123_456_789)
            .expect("an in-range nanosecond");
        let converted = to_chrono(at);
        assert_eq!(converted.timestamp(), at.unix_timestamp());
        assert_eq!(converted.timestamp_subsec_nanos(), at.nanosecond());
    }

    /// An empty patch is the one a bodiless `POST /v1/customers/{id}`
    /// produces, and the API answers it with the object unchanged rather
    /// than with a write.
    #[test]
    fn a_patch_that_mentions_nothing_is_empty_and_one_that_clears_is_not() {
        assert!(CustomerPatch::default().is_empty());

        // The three-state distinction, in the direction that is easy to lose:
        // `Some(None)` is "clear this", which is a change.
        assert!(
            !CustomerPatch {
                phone: Some(None),
                ..CustomerPatch::default()
            }
            .is_empty(),
            "clearing a payer's phone number is a change; collapsing it into `is_empty` \
             would make `phone=` a silent no-op and leave personal data a merchant asked to \
             be removed"
        );
        assert!(
            !CustomerPatch {
                metadata: Some(serde_json::json!({})),
                ..CustomerPatch::default()
            }
            .is_empty()
        );
    }
}
