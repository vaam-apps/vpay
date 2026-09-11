//! The `invoices` and `invoice_items` repository
//! (`backends/migrations/0036_create-invoices.sql`) — the reads and writes
//! behind `/v1/invoices` and `/v1/invoice_items`.
//!
//! It keeps [`crate::customers`]' rules unchanged — every merchant-facing
//! query is merchant-scoped **in SQL**, state changes are compare-and-swap —
//! and adds the two this table family has and no other does:
//!
//! * **the state machine is the `WHERE` clause.** There is no
//!   `can_transition_to` anywhere in this crate and none in `vpay-core`
//!   either ([`vpay_core::InvoiceStatus`] says why). Every transition below
//!   is `UPDATE invoices SET … WHERE id = $n AND merchant_id = $n AND
//!   status = '<from>'`, and "matched no row" is the refusal. A validation
//!   function beside the write is the thing a future writer calls *instead
//!   of* taking the lock, which is how a paid invoice gets voided.
//! * **an issued document is frozen.** Every write to `invoice_items`
//!   carries `EXISTS (SELECT 1 FROM invoices WHERE id = … AND status =
//!   'draft')` in the same statement. Not a preceding read: between a read
//!   and a write a concurrent `finalize` can commit, and the line that
//!   changed afterwards would be a line on a document that has already been
//!   sent.
//!
//! # The split between CrateStack and hand-written SQL, and what decides it
//!
//! Two of the twelve methods below run through `schemas/vpay.cstack`'s
//! `model Invoice` / `model InvoiceItem` ([`Invoices::mark_uncollectible`],
//! [`Invoices::items_for_invoice`]); the rest are hand-written `sqlx`.
//! `schemas/vpay.cstack`'s own "invoices (S4b)" section carries the argument
//! in full — the short form is three separate blockers, and each method's
//! doc names the one that applies to it:
//!
//! 1. `invoices.metadata` is `JSONB NOT NULL` and undeclared
//!    (`model Customer`'s two measured costs, unchanged), so no read of
//!    `invoices` can go through the generated layer at all;
//! 2. three transitions write an `events` row in the same transaction, and
//!    `crate::events`' insert is itself blocked on `events.data`;
//! 3. every `invoice_items` write guards on a **different table's** column,
//!    and `cratestack::Filter` compares columns of the model's own table.
//!
//! # Where the transactions are opened, and why not all of them are here
//!
//! The three transitions that emit an event — create, finalize, void — are
//! **not** methods on [`Invoices`]. They are [`crate::TxRepositories`]
//! methods, so `vpay-api` opens the transaction, gets the written row back,
//! renders the wire object *from that row*, and appends the event beside it.
//!
//! That is a departure from [`crate::settlement`] and
//! [`crate::customers::Customers::erase_idle`], which take a caller-rendered
//! `event_data` as a parameter, and the reason is specific rather than
//! stylistic: those two describe an object whose post-write shape the caller
//! can *project* exactly (an intent that is about to be `succeeded`; a
//! customer that is about to be deleted). A finalized invoice cannot be
//! projected — its `number` comes out of a sequence the statement itself
//! advances and its `amount_due` is summed by the statement from the lines —
//! so a projection would be a second implementation of the assignment, and
//! the first thing it would get wrong is the number.
//!
//! `docs/reference/vpay-db.md` §"`invoices`" carries the rest.

use sqlx::{AssertSqlSafe, PgConnection};
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};
use crate::persistence::{classify_cratestack, system_context};

/// The `.cstack` model [`Invoices::mark_uncollectible`] names, for
/// [`crate::persistence::classify_cratestack`]'s `model` slot.
const INVOICE_MODEL: &str = "Invoice";

/// The `.cstack` model [`Invoices::items_for_invoice`] names.
const INVOICE_ITEM_MODEL: &str = "InvoiceItem";

/// Every column of `invoices`, in one place so the statements below cannot
/// drift on the shape they decode into [`InvoiceRow`].
const COLUMNS: &str = "id, seq, merchant_id, livemode, customer_id, currency_code, status, number, \
                       amount_due, amount_paid, amount_remaining, amount_refunded, due_date, \
                       description, metadata, \
                       payment_intent_id, finalized_at, paid_at, voided_at, \
                       marked_uncollectible_at, created_at, updated_at";

/// [`COLUMNS`], every name qualified with `invoices.`.
///
/// Needed by exactly the statements that put a second relation in scope — the
/// `UPDATE … FROM (SELECT …)` pair that sums the lines. An unqualified
/// `RETURNING` list happens to be unambiguous there today (the sub-select
/// projects one column, `total`), and that is precisely the kind of accident
/// that stops being true when somebody adds a second column to the
/// sub-select. Spelled out rather than computed, because
/// [`crate::sql_audit`] requires every interpolation to be a `const … : &str`
/// in this crate.
const QUALIFIED_COLUMNS: &str = "invoices.id, invoices.seq, invoices.merchant_id, invoices.livemode, invoices.customer_id, \
     invoices.currency_code, invoices.status, invoices.number, invoices.amount_due, \
     invoices.amount_paid, invoices.amount_remaining, invoices.amount_refunded, \
     invoices.due_date, invoices.description, \
     invoices.metadata, invoices.payment_intent_id, invoices.finalized_at, invoices.paid_at, \
     invoices.voided_at, invoices.marked_uncollectible_at, invoices.created_at, \
     invoices.updated_at";

/// Every column of `invoice_items`, for [`InvoiceItemRow`].
const ITEM_COLUMNS: &str = "id, seq, invoice_id, merchant_id, livemode, description, quantity, \
                            unit_amount, amount, currency_code, created_at, updated_at";

/// The guard that makes an issued document immutable, shared by the three
/// `invoice_items` writes so they cannot drift.
///
/// Correlated on `invoice_items.invoice_id`, so it works in a `WHERE` on that
/// table without a join and without the caller naming the parent twice.
///
/// A `const` rather than a function for [`crate::customers`]' `UNREFERENCED`
/// reason: [`crate::sql_audit`] resolves every `{…}` to a `const … : &str` in
/// this crate, so a computed fragment fails the audit by construction.
const PARENT_IS_A_DRAFT: &str = "EXISTS (SELECT 1 FROM invoices \
     WHERE invoices.id = invoice_items.invoice_id AND invoices.status = 'draft')";

/// The condition under which an invoice is **not** in the middle of being
/// paid, and may therefore be voided, written off, or handed a new intent.
///
/// # What this rule is, and why it is one rule and not three
///
/// An invoice with a payment intent attached is a document somebody is paying
/// right now. Voiding it, writing it off, or minting a *second* intent for it
/// are three different ways to end up with money moved against a document
/// that says it is not owed — the classic double-charge. All three refuse on
/// the same condition, spelled once here, so they cannot drift apart.
///
/// The condition is **`canceled`**, and deliberately not "not `processing`".
/// A rail-declined intent lands back on `requires_payment_method`
/// (`docs/flows/payment-lifecycle.md`) which is also where a *fresh,
/// unconfirmed* intent sits, so "not processing" would let a merchant mint a
/// second intent one millisecond after the first, before it was ever
/// confirmed. `canceled` is the one status that says a human decided this
/// attempt is over, and `POST /v1/payment_intents/{id}/cancel` is the
/// merchant-facing action that produces it.
///
/// **The recovery path is therefore explicit and is the same for all three:**
/// cancel the intent, then retry. That is consistent with this repository's
/// standing rule that a retry is a *new* PaymentIntent (`AGENTS.md`), and
/// `docs/flows/invoices.md` documents it as the answer to "my invoice is
/// stuck". It is listed as a maintainer decision in
/// `docs/plans/exp32-invoices-notes/opus.md` because the alternative —
/// treating a declined intent as immediately re-payable — is defensible and
/// is not this repository's to choose unilaterally.
const NO_LIVE_INTENT: &str = "(invoices.payment_intent_id IS NULL \
     OR EXISTS (SELECT 1 FROM payment_intents \
                WHERE payment_intents.id = invoices.payment_intent_id \
                  AND payment_intents.status = 'canceled'))";

/// The `type` of the event a settlement emits when it pays an invoice.
///
/// One of migration `0036`'s four new `type_is_a_documented_event` allows,
/// spelled here rather than passed in by the caller for
/// [`crate::settlement`]'s reason: the event type is a property of *which
/// transition this is*, and a caller free to choose it could report a paid
/// invoice as a voided one.
pub(crate) const EVENT_INVOICE_PAID: &str = "invoice.paid";

/// One `invoices` row, exactly as stored.
///
/// Not the wire object: `vpay-api` owns that shape (`created` as unix
/// seconds, `lines` expanded from a second table, `hosted_invoice_url`
/// derived). One-to-one with the table so a change to either is a compile
/// error rather than a silently dropped column.
///
/// `Debug` is derived, unlike [`crate::CustomerRow`]'s: an invoice carries
/// the merchant's own description and metadata and the *ids* of the payer and
/// the intent, not the payer's name, email or phone. The personal data stays
/// on `customers`, which redacts it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct InvoiceRow {
    /// Public `in_…` id, supplied by the caller before the insert.
    pub id: String,
    /// Pagination order. Database-generated, never written by this crate, and
    /// never exposed on the wire.
    pub seq: i64,
    /// The owning merchant. Every merchant-facing query here filters on it.
    pub merchant_id: String,
    /// Live or test money, copied from the deployment at creation.
    pub livemode: bool,
    /// The customer this invoice bills. **Never `None`** — migration `0036`
    /// makes the column `NOT NULL`, unlike `payment_intents.customer_id`.
    pub customer_id: String,
    /// ISO-4217, and the currency every line of this invoice is in.
    pub currency_code: String,
    /// One of [`vpay_core::InvoiceStatus`]' five labels, as stored.
    ///
    /// A `String` and not the enum, exactly as `ChargeRow::state` is: this
    /// struct is one-to-one with the table, and a decode that rejected an
    /// unknown label would turn a schema/code disagreement into a read
    /// failure rather than into a rendering failure a handler can classify.
    pub status: String,
    /// `{prefix}-{000001}`, or `None` while this is a draft. Migration
    /// `0036`'s `number_is_assigned_at_finalize` makes the correspondence an
    /// invariant rather than a convention.
    pub number: Option<String>,
    /// Integer minor units (`docs/flows/money.md`). Summed from the lines
    /// while the invoice is a draft, frozen at finalize.
    pub amount_due: i64,
    /// How much has been paid. `0` until a settlement pays this invoice, and
    /// then exactly [`Self::amount_due`] — partial payments are out of scope.
    pub amount_paid: i64,
    /// `amount_due - amount_paid`, stored rather than computed; migration
    /// `0036`'s `amounts_add_up` is what keeps it honest.
    pub amount_remaining: i64,
    /// What has been given back out of [`Self::amount_paid`], as a **gross**
    /// running total (migration `0042`).
    ///
    /// It is deliberately *not* subtracted from [`Self::amount_paid`] and
    /// takes no part in `amounts_add_up`, so a refunded invoice is still
    /// `paid` with nothing remaining — `payment_intents.amount_refunded`'s
    /// shape since migration `0003`, applied to the document. Migration
    /// `0042`'s header carries the argument against the alternative.
    ///
    /// `0` on every row in every deployment today: the only statement that
    /// moves it is reached from [`crate::Settlement::apply_refund_succeeded`],
    /// which no shipping binary calls because no rail can refund
    /// (`docs/status.md`).
    pub amount_refunded: i64,
    /// When the merchant says this is due, or `None`. **Advisory**: nothing
    /// in vpay reads it. See migration `0036`'s column comment.
    pub due_date: Option<OffsetDateTime>,
    /// The merchant's own note on the document.
    pub description: Option<String>,
    /// The merchant's own key/value pairs, as stored. `metadata_is_object`
    /// guarantees this is a JSON object.
    pub metadata: serde_json::Value,
    /// The intent paying, or that paid, this invoice. At most one at a time,
    /// and `invoices_payment_intent_key` is what makes that a fact about the
    /// database.
    pub payment_intent_id: Option<String>,
    /// When [`crate::TxRepositories::finalize_invoice_in_tx`] issued it.
    pub finalized_at: Option<OffsetDateTime>,
    /// When the settlement transaction paid it.
    pub paid_at: Option<OffsetDateTime>,
    /// When the merchant voided it.
    pub voided_at: Option<OffsetDateTime>,
    /// When the merchant wrote it off.
    pub marked_uncollectible_at: Option<OffsetDateTime>,
    /// When the invoice was created, as supplied to [`NewInvoice`].
    pub created_at: OffsetDateTime,
    /// When the row last changed. Maintained by the writers here, not by a
    /// trigger — 0034's rule.
    pub updated_at: OffsetDateTime,
}

/// One `invoice_items` row, exactly as stored.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct InvoiceItemRow {
    /// Public `ii_…` id, supplied by the caller before the insert.
    pub id: String,
    /// The order lines are rendered in — the order they were added.
    pub seq: i64,
    /// The invoice this line belongs to.
    pub invoice_id: String,
    /// Copied from the parent, so this table's tenancy filter is one table's
    /// `WHERE` and never a join.
    pub merchant_id: String,
    /// Copied from the parent.
    pub livemode: bool,
    /// The text on the document. `NOT NULL`, unlike the invoice's own
    /// description: a line nobody can identify is a charge nobody can query.
    pub description: String,
    /// How many. At least 1 (`quantity_positive`).
    pub quantity: i64,
    /// The price of one, in integer minor units.
    pub unit_amount: i64,
    /// `quantity * unit_amount`, stored and checked by the database
    /// (`amount_is_the_product`).
    pub amount: i64,
    /// Copied from the parent invoice, never taken from the request.
    pub currency_code: String,
    /// When the line was added.
    pub created_at: OffsetDateTime,
    /// When it last changed.
    pub updated_at: OffsetDateTime,
}

/// The columns a caller supplies when creating an invoice: [`InvoiceRow`]
/// minus everything the draft state fixes.
///
/// There is no `status`, no `number` and no amount field, and none of them is
/// an omission: a new invoice is a `draft`, has no number and totals zero,
/// always. A parameter for any of the three would be a way to create an
/// invoice that is already `open` without a number, or `paid` without a
/// payment — states migration `0036`'s CHECKs refuse, reached through an
/// argument nobody meant to pass.
#[derive(Debug, Clone, PartialEq)]
pub struct NewInvoice {
    /// Public `in_…` id, from `vpay_core::ids::invoice_id`, generated before
    /// the insert — never by the database, so a crash mid-insert still leaves
    /// a name to reconcile by.
    pub id: String,
    /// The owning merchant, from the authenticated client's mapping.
    pub merchant_id: String,
    /// From `config.deployment.livemode`; never inferred per request.
    pub livemode: bool,
    /// The customer being billed. Required, and already resolved through
    /// [`crate::Customers::get_for_merchant`] by the caller, so the foreign
    /// key can only fail on a race.
    pub customer_id: String,
    /// ISO-4217, already checked against the deployment's admitted
    /// currencies by `vpay-api`.
    pub currency_code: String,
    /// Advisory. See [`InvoiceRow::due_date`].
    pub due_date: Option<OffsetDateTime>,
    /// The merchant's note on the document.
    pub description: Option<String>,
    /// A JSON **object**; `metadata_is_object` refuses anything else.
    pub metadata: serde_json::Value,
    /// Creation instant, supplied by the caller.
    pub created_at: OffsetDateTime,
}

/// The fields `POST /v1/invoices/{id}` may change **while it is a draft**, in
/// the wire's own three-state shape.
///
/// Why every field is an `Option<Option<…>>`: [`crate::CustomerPatch`]'s
/// argument verbatim — *leave it alone*, *set it*, and *clear it* are three
/// answers a merchant depends on, and `Option<T>` carries two.
///
/// `customer` and `currency` are **not** patchable and that is deliberate.
/// Changing either on an invoice that has lines would make the lines
/// (which carry their own `currency_code`, copied at insert) disagree with
/// their parent, and there is no honest answer to "who is this bill for
/// now?" that does not amount to creating a different invoice.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InvoicePatch {
    /// `None` = the request did not mention `description`. `Some(None)` =
    /// clear it. `Some(Some(v))` = set it to `v`.
    pub description: Option<Option<String>>,
    /// See [`Self::description`].
    pub due_date: Option<Option<OffsetDateTime>>,
    /// `None` = the request did not mention `metadata`; `Some(map)` = the
    /// merged map to store. Two states rather than three, for
    /// [`crate::CustomerPatch`]'s reason: Stripe's `metadata` is merged
    /// key-wise and `vpay_api` does the merge.
    pub metadata: Option<serde_json::Value>,
}

impl InvoicePatch {
    /// Whether this patch would change nothing at all.
    ///
    /// The API uses it to answer a bodiless `POST /v1/invoices/{id}` with the
    /// object unchanged rather than with an `UPDATE` that writes only
    /// `updated_at` — [`crate::CustomerPatch::is_empty`]'s contract, and
    /// Stripe's own behaviour.
    ///
    /// ```
    /// use vpay_db::InvoicePatch;
    ///
    /// assert!(InvoicePatch::default().is_empty());
    ///
    /// // Clearing a due date is a change, even though the value is `None`.
    /// let clear_due_date = InvoicePatch {
    ///     due_date: Some(None),
    ///     ..InvoicePatch::default()
    /// };
    /// assert!(!clear_due_date.is_empty());
    /// ```
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.description.is_none() && self.due_date.is_none() && self.metadata.is_none()
    }
}

/// The columns a caller supplies when adding a line.
///
/// `merchant_id`, `livemode` and `currency_code` are **absent**, and that is
/// the point: [`Invoices::add_item`] copies all three out of the parent
/// invoice inside the insert itself, so a line can never claim a tenant, a
/// mode or a currency its invoice does not have.
#[derive(Debug, Clone, PartialEq)]
pub struct NewInvoiceItem {
    /// Public `ii_…` id, from `vpay_core::ids::invoice_item_id`.
    pub id: String,
    /// The parent invoice. Must be a **draft** of `merchant_id`'s, which the
    /// insert checks rather than the caller.
    pub invoice_id: String,
    /// The merchant the caller is authenticated as. Checked against the
    /// parent inside the statement, never compared in Rust.
    pub merchant_id: String,
    /// The text on the document.
    pub description: String,
    /// At least 1.
    pub quantity: i64,
    /// The price of one, in integer minor units.
    pub unit_amount: i64,
    /// Creation instant, supplied by the caller.
    pub created_at: OffsetDateTime,
}

/// The fields `POST /v1/invoice_items/{id}` may change, while the parent is a
/// draft.
///
/// Single `Option`s and not double ones, unlike [`InvoicePatch`]: all three
/// columns are `NOT NULL`, so there is no "clear it" state to carry. Sending
/// `description=` is a `400` naming the parameter, not a clear.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InvoiceItemPatch {
    /// `None` = leave it. `Some(v)` = set it.
    pub description: Option<String>,
    /// See [`Self::description`].
    pub quantity: Option<i64>,
    /// See [`Self::description`].
    pub unit_amount: Option<i64>,
}

impl InvoiceItemPatch {
    /// Whether this patch would change nothing at all — [`InvoicePatch::is_empty`]'s
    /// contract.
    ///
    /// ```
    /// use vpay_db::InvoiceItemPatch;
    ///
    /// assert!(InvoiceItemPatch::default().is_empty());
    /// assert!(
    ///     !InvoiceItemPatch {
    ///         quantity: Some(2),
    ///         ..InvoiceItemPatch::default()
    ///     }
    ///     .is_empty()
    /// );
    /// ```
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.description.is_none() && self.quantity.is_none() && self.unit_amount.is_none()
    }
}

/// One page request for [`Invoices::list_page`].
///
/// The same cursor rule as [`crate::CustomerListPage`] — cursors are public
/// `in_…` ids, never `seq` values — plus the two filters
/// `GET /v1/invoices` documents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InvoiceListPage {
    /// How many rows the caller wants. `vpay-api` applies the product limits.
    pub limit: i64,
    /// Return invoices strictly *older* than this id.
    pub starting_after: Option<String>,
    /// Return invoices strictly *newer* than this id — scans ascending and is
    /// reversed in Rust, so `data` is newest-first either way.
    pub ending_before: Option<String>,
    /// Only this customer's invoices. A `cus_…`, **not** validated for
    /// existence: an unknown customer yields an empty page, exactly as an
    /// unknown `payment_intent` does on `GET /v1/checkout/sessions`, because
    /// answering `404` would make the filter an existence oracle for another
    /// merchant's ids.
    pub customer: Option<String>,
    /// Only invoices in this status. Already checked against
    /// [`vpay_core::InvoiceStatus::from_wire`] by `vpay-api`, which answers
    /// `400` for a label vpay does not have — an unknown status must not
    /// read as "you have no invoices like that".
    pub status: Option<String>,
}

#[async_trait::async_trait]
pub trait Invoices: Send + Sync {
    /// Reads one invoice *for this merchant*. `None` means "no such invoice
    /// for you", which covers both a missing id and another merchant's.
    ///
    /// Hand-written: every read of this table has to carry `metadata`, and
    /// `model Invoice` does not declare that column (blocker 1 in the module
    /// header).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<InvoiceRow>, DbError>;

    /// One page of this merchant's invoices, newest first, plus whether more
    /// exist beyond it.
    ///
    /// Ordering, cursors and `has_more` work exactly as
    /// [`crate::Customers::list_page`]'s do, and it stays raw SQL for the
    /// same reason: the cursor is a correlated sub-select
    /// (`seq < (SELECT seq FROM invoices WHERE id = $2 AND merchant_id =
    /// $1)`), which `cratestack::Filter` has no constructor for.
    ///
    /// Both filters are applied **in the same `WHERE` as `merchant_id`**, so
    /// a filter can never widen the scope it is applied inside.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn list_page(
        &self,
        merchant_id: &str,
        page: &InvoiceListPage,
    ) -> Result<(Vec<InvoiceRow>, bool), DbError>;

    /// Applies a patch to a **draft** invoice and returns the row as it now
    /// stands. `None` means this merchant has no such *draft* invoice — a
    /// finalized one included.
    ///
    /// # The refusal is the statement, and it has to be
    ///
    /// `AND status = 'draft'` is in the `WHERE`. A handler that read the
    /// invoice, checked the status in Rust and then wrote would have a window
    /// in which a concurrent `finalize` commits between the two, and the
    /// merchant's edit would land on a document that has already been sent
    /// with a number on it. `a_finalized_invoice_refuses_a_patch` and the
    /// mutation that deletes this clause are in
    /// `backends/tests/integration/tests/invoices.rs`.
    ///
    /// The caller cannot tell "not yours", "no such invoice" and "not a
    /// draft" apart from the return value alone, which is deliberate for the
    /// first two and *not* good enough for the third — so `vpay-api` re-reads
    /// on `None` to decide between a `404` and a `409` that says the invoice
    /// is no longer a draft. That read is a diagnosis of a refusal that has
    /// already happened; it is never what decides the write.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], including `description_length` and
    /// `metadata_is_object`, both of which `vpay-api` refuses first with a
    /// `400` naming the parameter.
    async fn update_draft(
        &self,
        merchant_id: &str,
        id: &str,
        patch: &InvoicePatch,
        now: OffsetDateTime,
    ) -> Result<Option<InvoiceRow>, DbError>;

    /// Deletes a **draft** invoice and its lines. `false` means this merchant
    /// has no such draft invoice.
    ///
    /// The lines go with it through `ON DELETE CASCADE` — the schema's only
    /// cascade, and migration `0036` argues for it: a draft's lines are not
    /// billable, not numbered and referenced by nothing, so leaving them
    /// behind would be rows no query can reach.
    ///
    /// A finalized invoice is **never** deleted, by anything, ever. It is
    /// voided instead ([`crate::TxRepositories::void_invoice_in_tx`]), which
    /// keeps the number and the document.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the delete fails.
    async fn delete_draft(&self, merchant_id: &str, id: &str) -> Result<bool, DbError>;

    /// Writes an **open** invoice off as uncollectible. `false` means this
    /// merchant has no such open invoice, or one whose intent is still live.
    ///
    /// # Why this one goes through CrateStack when the other transitions do
    /// not
    ///
    /// It is the only transition with **no event**.
    /// `invoice.marked_uncollectible` is Stripe's fifth invoice event and is
    /// deliberately outside migration `0036`'s vocabulary, because this
    /// method is a single statement and putting a label in a closed
    /// vocabulary that no code produces is what that mechanism exists to
    /// prevent. With no event there is no transaction, and a
    /// compare-and-swap `update_many` expresses the whole operation:
    /// `where_(id).where_(merchant_id).where_(status.eq(Open))`.
    ///
    /// # What it refuses, beyond the status
    ///
    /// An invoice whose intent has not been canceled. See
    /// [`NO_LIVE_INTENT`](self) for why that condition is `canceled` and not
    /// "not processing", and for the recovery path.
    ///
    /// That guard is **not** expressible through CrateStack (it is a
    /// correlated sub-select over `payment_intents`), so it is applied by
    /// `vpay-api` from the row it has already read, and the *window* it
    /// leaves is closed by the settlement's own compare-and-swap rather than
    /// by this statement: a settlement that lands first flips the invoice to
    /// `paid`, and this `update_many`'s `status = 'open'` filter then matches
    /// nothing. Neither ordering can produce a paid invoice that is also
    /// written off. That is stated here because it is the one place in this
    /// module where a guard is not in the statement it guards, and
    /// `an_invoice_being_paid_cannot_be_written_off` is what pins it.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] wrapping [`crate::PersistenceError::Denied`]
    /// if `model Invoice` loses its `@@allow("update", …)`. **That refusal is
    /// silent**: `update_many` compiles the policy into the statement's own
    /// `WHERE`, so the arm's loss shows up as `Ok(false)` and not as this
    /// error — see the model's own comment, and
    /// `every_action_this_module_calls_has_an_allow_arm`.
    async fn mark_uncollectible(
        &self,
        merchant_id: &str,
        id: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// Every line of one invoice, in the order they were added.
    ///
    /// # Why this goes through CrateStack
    ///
    /// Every column of `invoice_items` is declared on `model InvoiceItem`
    /// (the table was shaped so it could be — no `jsonb`, no `bytea`, no
    /// native enum, no `int4`), so the generated row carries the whole line
    /// and none of blockers 1–3 applies.
    ///
    /// # Unscoped, and named for it
    ///
    /// Deliberately no `merchant_id`. Its callers hold an invoice id they
    /// have *already* resolved through [`Invoices::get_for_merchant`] —
    /// [`crate::CheckoutSessions::find_open_by_intent`]'s argument,
    /// unchanged: re-filtering by a tenant derived from that same id would be
    /// an authorisation check against itself.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] wrapping [`crate::PersistenceError::Denied`]
    /// if `model InvoiceItem` loses its `@@allow("read", …)`. Unlike
    /// [`Invoices::mark_uncollectible`]'s refusal, this one is **loud**:
    /// `find_many` authorises before the statement runs, so the arm's loss
    /// turns every invoice read into a `500` rather than into an empty line
    /// list. That asymmetry is why the two arms are tested differently — see
    /// the model's own comment.
    async fn items_for_invoice(&self, invoice_id: &str) -> Result<Vec<InvoiceItemRow>, DbError>;

    /// Reads one line *for this merchant*. `None` means "no such line for
    /// you".
    ///
    /// Scoped on `invoice_items.merchant_id` — the column migration `0036`
    /// denormalises from the parent — so this is one table's `WHERE` and
    /// never a join. Hand-written because a `find_unique` cannot carry the
    /// tenant filter (it takes a primary key), and `find_many` returning
    /// "not yours" and "no such line" as the same empty vector would work but
    /// would put a second read shape on this table for no gain.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn get_item_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<InvoiceItemRow>, DbError>;

    /// Adds a line to a **draft** invoice of this merchant's and returns the
    /// line beside the invoice as it now totals. `None` means there is no
    /// such draft invoice for this merchant.
    ///
    /// # One statement decides three things
    ///
    /// The insert is `INSERT … SELECT … FROM invoices WHERE id = $2 AND
    /// merchant_id = $3 AND status = 'draft'`, so the tenancy check, the
    /// draft check, and the copy of `merchant_id`/`livemode`/`currency_code`
    /// off the parent all happen in the same statement that writes the row.
    /// There is no preceding `SELECT` to race against, and no path on which a
    /// line acquires a currency its invoice does not have.
    ///
    /// # The invoice's totals move with it
    ///
    /// A draft's `amount_due` is always the sum of its lines, so this method
    /// re-sums and rewrites the parent inside the same transaction. Two
    /// statements and one commit, because a merchant reading the invoice back
    /// between them would see a total that does not match the lines they are
    /// looking at.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], including `quantity_positive`,
    /// `unit_amount_non_negative`, `description_length` and
    /// `amount_is_the_product` — every one of which `vpay-api` refuses first
    /// with a `400` naming the parameter, so reaching one here is a vpay bug.
    async fn add_item(
        &self,
        new: &NewInvoiceItem,
    ) -> Result<Option<(InvoiceItemRow, InvoiceRow)>, DbError>;

    /// Changes a line of a **draft** invoice and returns it beside the
    /// re-totalled invoice. `None` means no such line for this merchant, or a
    /// parent that is no longer a draft.
    ///
    /// `amount` is recomputed inside the statement from whichever of
    /// `quantity` and `unit_amount` the patch supplies, so it cannot be sent
    /// by a caller and cannot disagree with its own factors —
    /// `amount_is_the_product` is the backstop.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`]; see [`Invoices::add_item`].
    async fn update_item(
        &self,
        merchant_id: &str,
        id: &str,
        patch: &InvoiceItemPatch,
        now: OffsetDateTime,
    ) -> Result<Option<(InvoiceItemRow, InvoiceRow)>, DbError>;

    /// Removes a line from a **draft** invoice and returns the re-totalled
    /// invoice. `None` means no such line for this merchant, or a parent that
    /// is no longer a draft.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`]; see [`Invoices::add_item`].
    async fn delete_item(&self, merchant_id: &str, id: &str)
    -> Result<Option<InvoiceRow>, DbError>;

    /// Binds a payment intent to an **open** invoice, so a settlement can
    /// find it. `None` means the invoice is not open, is not this merchant's,
    /// or already has a live intent.
    ///
    /// # This is where "no partial payments" is enforced
    ///
    /// The `WHERE` carries [`NO_LIVE_INTENT`](self), so a second
    /// `POST /v1/invoices/{id}/pay` against a document somebody is already
    /// paying matches no row. It is a compare-and-swap and not a check,
    /// because two concurrent `pay` calls both reading "no intent yet" is
    /// exactly the race that produces two charges for one bill.
    /// `invoices_payment_intent_key` is the second enforcer and the one that
    /// survives a future writer who drops this clause.
    ///
    /// # The intent is created first, and that is deliberate
    ///
    /// `vpay-api` mints the intent, then attaches it. The failure mode of
    /// that order is an orphan intent nobody paid — visible, cancellable,
    /// and costing nobody anything. The other order's failure mode is an
    /// invoice pointing at an intent that does not exist, which the foreign
    /// key would refuse anyway. `docs/flows/crash-safety.md`'s standing rule
    /// ("never let a payer act on a transaction you cannot name") is
    /// satisfied either way, because the payer acts on the *intent*.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the write fails, including the foreign key on
    /// `payment_intent_id` if the intent was not committed first.
    async fn attach_intent(
        &self,
        merchant_id: &str,
        id: &str,
        payment_intent_id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<InvoiceRow>, DbError>;

    /// The **open** invoice this intent is paying, if there is one.
    ///
    /// The worker's read, taken before the settlement transaction opens so
    /// the `invoice.paid` event can be stamped with the invoice's tenant and
    /// its wire object rendered — `vpay_db::settlement`'s `apply_succeeded`
    /// takes both as parameters for `crate::Settlement`'s standing reason
    /// (this crate does not know the wire shape).
    ///
    /// # Unscoped, and named for it
    ///
    /// [`crate::CheckoutSessions::find_open_by_intent`]'s argument verbatim:
    /// the caller holds a `pi_…` it reached through the job queue, and any
    /// merchant it could filter by would be derived from this same row.
    ///
    /// # `Ok(None)` is the normal answer
    ///
    /// Most intents pay no invoice at all. It also covers an invoice that
    /// stopped being `open` between this read and now, which the settlement's
    /// own compare-and-swap re-checks inside the transaction — this read is
    /// never what decides the write.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn find_open_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<InvoiceRow>, DbError>;
}

#[async_trait::async_trait]
impl Invoices for crate::repository::PgRepositories {
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<InvoiceRow>, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM invoices WHERE merchant_id = $1 AND id = $2");

        sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn list_page(
        &self,
        merchant_id: &str,
        page: &InvoiceListPage,
    ) -> Result<(Vec<InvoiceRow>, bool), DbError> {
        let limit = page.limit.max(1);
        let backwards = page.ending_before.is_some();
        let direction = if backwards { "ASC" } else { "DESC" };

        // Both filters are `$n IS NULL OR …`, so the statement is one shape
        // whatever the caller asked for — `customers::list_page`'s device,
        // and the reason `crate::sql_audit` can prove this `format!`
        // interpolates only constants.
        let sql = format!(
            "SELECT {COLUMNS} FROM invoices \
             WHERE merchant_id = $1 \
               AND ($2::TEXT IS NULL \
                    OR seq < (SELECT seq FROM invoices \
                              WHERE id = $2 AND merchant_id = $1)) \
               AND ($3::TEXT IS NULL \
                    OR seq > (SELECT seq FROM invoices \
                              WHERE id = $3 AND merchant_id = $1)) \
               AND ($4::TEXT IS NULL OR customer_id = $4) \
               AND ($5::TEXT IS NULL OR status = $5) \
             ORDER BY seq {direction} \
             LIMIT $6"
        );

        let mut rows = sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(page.starting_after.as_deref())
            .bind(page.ending_before.as_deref())
            .bind(page.customer.as_deref())
            .bind(page.status.as_deref())
            .bind(limit.saturating_add(1))
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)?;

        let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        if backwards {
            rows.reverse();
        }

        Ok((rows, has_more))
    }

    async fn update_draft(
        &self,
        merchant_id: &str,
        id: &str,
        patch: &InvoicePatch,
        now: OffsetDateTime,
    ) -> Result<Option<InvoiceRow>, DbError> {
        // `CASE WHEN $n::BOOLEAN` per column, exactly as
        // `customers::update` builds it, and for the same reason: one static
        // statement rather than a `SET` list whose shape is a function of the
        // request. The three-state semantics live entirely in the binds.
        let sql = format!(
            "UPDATE invoices SET \
                description = CASE WHEN $3::BOOLEAN THEN $4::TEXT ELSE description END, \
                due_date = CASE WHEN $5::BOOLEAN THEN $6::TIMESTAMPTZ ELSE due_date END, \
                metadata = CASE WHEN $7::BOOLEAN THEN $8::JSONB ELSE metadata END, \
                updated_at = $9 \
             WHERE merchant_id = $1 AND id = $2 AND status = 'draft' \
             RETURNING {COLUMNS}"
        );

        sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .bind(patch.description.is_some())
            .bind(patch.description.clone().flatten())
            .bind(patch.due_date.is_some())
            .bind(patch.due_date.flatten())
            .bind(patch.metadata.is_some())
            .bind(patch.metadata.clone())
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(classify_write)
    }

    async fn delete_draft(&self, merchant_id: &str, id: &str) -> Result<bool, DbError> {
        let sql = "DELETE FROM invoices WHERE merchant_id = $1 AND id = $2 AND status = 'draft'";

        let result = sqlx::query(sql)
            .bind(merchant_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(classify_write)?;

        Ok(result.rows_affected() == 1)
    }

    async fn mark_uncollectible(
        &self,
        merchant_id: &str,
        id: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        // THROUGH CRATESTACK. `update_many` and not `update(pk)`, for
        // `customers::touch_last_used`'s reason: `update` by primary key has
        // nowhere to put the tenant filter or the `status = 'open'` guard,
        // and both of those are the whole operation.
        use crate::schema::cratestack_schema as cs;

        let summary = self
            .cs
            .invoice()
            .update_many()
            .where_(cs::invoice::id().eq(id.to_owned()))
            .where_(cs::invoice::merchant_id().eq(merchant_id.to_owned()))
            .where_(cs::invoice::status().eq(cs::InvoiceStatus::open))
            .set(cs::UpdateInvoiceInput {
                status: Some(cs::InvoiceStatus::uncollectible),
                marked_uncollectible_at: Some(Some(to_chrono(now))),
                updated_at: Some(to_chrono(now)),
                ..cs::UpdateInvoiceInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(INVOICE_MODEL, "update", error)))?;

        // `id` is the primary key, so this is 0 or 1 and never more.
        Ok(summary.ok == 1)
    }

    async fn items_for_invoice(&self, invoice_id: &str) -> Result<Vec<InvoiceItemRow>, DbError> {
        // THROUGH CRATESTACK. The one read in this module that can be: every
        // column of the table is declared on `model InvoiceItem`.
        use crate::schema::cratestack_schema as cs;

        let rows = self
            .cs
            .invoice_item()
            .find_many()
            .where_(cs::invoice_item::invoice_id().eq(invoice_id.to_owned()))
            .order_by(cs::invoice_item::seq().asc())
            .run(&system_context())
            .await
            .map_err(|error| {
                DbError::from(classify_cratestack(INVOICE_ITEM_MODEL, "read", error))
            })?;

        Ok(rows.into_iter().map(InvoiceItemRow::from).collect())
    }

    async fn get_item_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<InvoiceItemRow>, DbError> {
        let sql =
            format!("SELECT {ITEM_COLUMNS} FROM invoice_items WHERE merchant_id = $1 AND id = $2");

        sqlx::query_as::<_, InvoiceItemRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn add_item(
        &self,
        new: &NewInvoiceItem,
    ) -> Result<Option<(InvoiceItemRow, InvoiceRow)>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // `INSERT … SELECT … FROM invoices WHERE …`: the tenancy check, the
        // draft check and the copy of the parent's three columns are the same
        // statement as the write. `$7` is bound twice — `created_at` is also
        // this line's `updated_at`.
        let sql = format!(
            "INSERT INTO invoice_items \
                (id, invoice_id, merchant_id, livemode, description, quantity, unit_amount, \
                 amount, currency_code, created_at, updated_at) \
             SELECT $1, invoices.id, invoices.merchant_id, invoices.livemode, $4, $5, $6, \
                    $5 * $6, invoices.currency_code, $7, $7 \
             FROM invoices \
             WHERE invoices.id = $2 AND invoices.merchant_id = $3 AND invoices.status = 'draft' \
             RETURNING {ITEM_COLUMNS}"
        );

        let item = sqlx::query_as::<_, InvoiceItemRow>(AssertSqlSafe(sql))
            .bind(&new.id)
            .bind(&new.invoice_id)
            .bind(&new.merchant_id)
            .bind(&new.description)
            .bind(new.quantity)
            .bind(new.unit_amount)
            .bind(new.created_at)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?;

        let Some(item) = item else {
            // No such draft invoice for this merchant. Nothing was written;
            // dropping the transaction rolls it back.
            return Ok(None);
        };

        let invoice = resum_draft(&mut tx, &new.invoice_id, new.created_at).await?;
        tx.commit().await.map_err(DbError::Query)?;
        Ok(invoice.map(|invoice| (item, invoice)))
    }

    async fn update_item(
        &self,
        merchant_id: &str,
        id: &str,
        patch: &InvoiceItemPatch,
        now: OffsetDateTime,
    ) -> Result<Option<(InvoiceItemRow, InvoiceRow)>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // `amount` repeats the two `CASE` expressions rather than reading the
        // columns it just assigned: an `UPDATE`'s `SET` list sees the row as
        // it was, so `amount = quantity * unit_amount` after assigning them
        // would multiply the OLD values and trip `amount_is_the_product`.
        // That is a real Postgres behaviour and not a style choice, which is
        // why `a_line_amount_follows_its_own_factors` exercises a patch that
        // changes both.
        let sql = format!(
            "UPDATE invoice_items SET \
                description = CASE WHEN $3::BOOLEAN THEN $4::TEXT ELSE description END, \
                quantity = CASE WHEN $5::BOOLEAN THEN $6::BIGINT ELSE quantity END, \
                unit_amount = CASE WHEN $7::BOOLEAN THEN $8::BIGINT ELSE unit_amount END, \
                amount = (CASE WHEN $5::BOOLEAN THEN $6::BIGINT ELSE quantity END) \
                       * (CASE WHEN $7::BOOLEAN THEN $8::BIGINT ELSE unit_amount END), \
                updated_at = $9 \
             WHERE merchant_id = $1 AND id = $2 AND {PARENT_IS_A_DRAFT} \
             RETURNING {ITEM_COLUMNS}"
        );

        let item = sqlx::query_as::<_, InvoiceItemRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .bind(patch.description.is_some())
            .bind(patch.description.clone())
            .bind(patch.quantity.is_some())
            .bind(patch.quantity)
            .bind(patch.unit_amount.is_some())
            .bind(patch.unit_amount)
            .bind(now)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?;

        let Some(item) = item else {
            return Ok(None);
        };

        let invoice = resum_draft(&mut tx, &item.invoice_id, now).await?;
        tx.commit().await.map_err(DbError::Query)?;
        Ok(invoice.map(|invoice| (item, invoice)))
    }

    async fn delete_item(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<InvoiceRow>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // `RETURNING invoice_id` rather than a preceding read: the parent to
        // re-sum is the parent of the row that was actually deleted, and a
        // value carried from a read taken before the guard was evaluated
        // would be a value about a different row.
        let sql = format!(
            "DELETE FROM invoice_items \
             WHERE merchant_id = $1 AND id = $2 AND {PARENT_IS_A_DRAFT} \
             RETURNING invoice_id"
        );

        let invoice_id = sqlx::query_scalar::<_, String>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?;

        let Some(invoice_id) = invoice_id else {
            return Ok(None);
        };

        let invoice = resum_draft(&mut tx, &invoice_id, OffsetDateTime::now_utc()).await?;
        tx.commit().await.map_err(DbError::Query)?;
        Ok(invoice)
    }

    async fn attach_intent(
        &self,
        merchant_id: &str,
        id: &str,
        payment_intent_id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<InvoiceRow>, DbError> {
        let sql = format!(
            "UPDATE invoices SET payment_intent_id = $3, updated_at = $4 \
             WHERE merchant_id = $1 AND id = $2 AND status = 'open' AND {NO_LIVE_INTENT} \
             RETURNING {COLUMNS}"
        );

        sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .bind(payment_intent_id)
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(classify_write)
    }

    async fn find_open_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<InvoiceRow>, DbError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM invoices WHERE payment_intent_id = $1 AND status = 'open'"
        );

        sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
            .bind(payment_intent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}

/// Re-sums a **draft** invoice's lines and rewrites its three amounts, inside
/// the caller's transaction.
///
/// `Ok(None)` means the invoice stopped being a draft between the line write
/// and this statement — a concurrent `finalize` — which the caller turns into
/// the same refusal a non-draft parent gets. It cannot mean "no such
/// invoice": the line write in the same transaction proved it exists.
///
/// `amount_paid = 0` is written rather than left alone, and that is not
/// defensive: a draft has never been paid (migration `0036`'s
/// `only_a_live_invoice_has_an_intent` says a draft has no intent, and
/// nothing else moves `amount_paid`), so the only value it can hold is zero,
/// and writing it is what lets `amounts_add_up` be a real check on this
/// statement rather than a check on a column this statement ignores.
/// `amount_refunded = 0` is written for the same reason and buys the same
/// thing on migration `0042`'s `refunded_at_most_paid`: a draft has never
/// been paid, so it can never have been refunded, and naming the column here
/// is what makes that CHECK evaluate against this statement.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails — including `amounts_add_up`,
/// which would mean this function and the CHECK disagree.
async fn resum_draft(
    conn: &mut PgConnection,
    invoice_id: &str,
    now: OffsetDateTime,
) -> Result<Option<InvoiceRow>, DbError> {
    let sql = format!(
        "UPDATE invoices SET \
            amount_due = lines.total, \
            amount_paid = 0, \
            amount_remaining = lines.total, \
            amount_refunded = 0, \
            updated_at = $2 \
         FROM (SELECT COALESCE(SUM(amount), 0) AS total FROM invoice_items \
               WHERE invoice_id = $1) lines \
         WHERE invoices.id = $1 AND invoices.status = 'draft' \
         RETURNING {QUALIFIED_COLUMNS}"
    );

    sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
        .bind(invoice_id)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// Inserts an invoice inside the caller's transaction.
///
/// `pub(crate)` and reached only from [`crate::repository::PendingTransaction`]'s
/// [`crate::TxRepositories`] impl, for [`crate::events::insert_in_tx`]'s
/// reason: the point of it being transactional is that the `events` row is
/// committed beside it, and a `pub` free function would make "create an
/// invoice with nobody told" expressible.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] naming the primary key if `id` is already
/// taken — which cannot happen for a freshly minted `in_…`.
/// [`DbError::Persistence`] wrapping [`crate::PersistenceError::ForeignKey`]
/// if `customer_id` names no customer or `currency_code` no admitted
/// currency; `vpay-api` resolves both first, so reaching either means the row
/// went away under the request. [`DbError::Query`] otherwise.
pub(crate) async fn insert_in_tx(
    conn: &mut PgConnection,
    new: &NewInvoice,
) -> Result<InvoiceRow, DbError> {
    // `status`, `number` and the three amounts are literals rather than
    // parameters: a new invoice is a draft with no number and no money on it,
    // always. See `NewInvoice`'s own doc.
    let sql = format!(
        "INSERT INTO invoices \
            (id, merchant_id, livemode, customer_id, currency_code, status, number, \
             amount_due, amount_paid, amount_remaining, amount_refunded, due_date, \
             description, metadata, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 'draft', NULL, 0, 0, 0, 0, $6, $7, $8, $9, $9) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
        .bind(&new.id)
        .bind(&new.merchant_id)
        .bind(new.livemode)
        .bind(&new.customer_id)
        .bind(&new.currency_code)
        .bind(new.due_date)
        .bind(new.description.as_deref())
        .bind(&new.metadata)
        .bind(new.created_at)
        .fetch_one(&mut *conn)
        .await
        .map_err(classify_write)
}

/// Takes the next number out of this merchant's sequence, inside the caller's
/// transaction, and renders it.
///
/// # This is the lock
///
/// `INSERT … ON CONFLICT (merchant_id) DO UPDATE SET next_number =
/// invoice_number_sequences.next_number + 1` takes a row lock on the
/// merchant's sequence row and holds it until the caller's transaction ends.
/// A second finalize for the same merchant blocks on that lock, and — under
/// `READ COMMITTED`, which is what this deployment runs — re-evaluates
/// against the committed value when it is released. Two concurrent finalizes
/// therefore get two consecutive numbers, in some order, with no gap and no
/// reuse. `two_concurrent_finalizes_take_consecutive_numbers` in
/// `backends/tests/integration/tests/invoices.rs` proves it, and deleting
/// this statement's `ON CONFLICT` clause is the mutation that makes it fail.
///
/// # A rolled-back finalize burns nothing
///
/// This is an ordinary table row and **not** a Postgres `SEQUENCE`, so the
/// increment is rolled back with everything else in the caller's transaction
/// and the next finalize takes the same number. Migration `0036`'s own
/// comment says why that matters more here than it does for Stripe (whose
/// numbering does have holes): a Cameroonian merchant's invoice numbers are
/// read by a tax authority that treats a missing number as a destroyed
/// document.
///
/// `prefix` is used **only** on the branch that creates the row — the first
/// time this merchant ever finalizes anything. Afterwards the stored prefix
/// wins, which is what makes it stable across the life of the merchant, and
/// is why the `RETURNING` reads `prefix` back rather than the caller
/// formatting with the value it passed in.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails, including `prefix_shape` for a
/// prefix `vpay_core::ids::invoice_number_prefix` did not produce.
async fn next_number_in_tx(
    conn: &mut PgConnection,
    merchant_id: &str,
    prefix: &str,
) -> Result<String, DbError> {
    // `VALUES (…, 2)` and `RETURNING next_number - 1`: on the insert branch
    // the row is born pointing at 2 and this finalize takes 1; on the
    // conflict branch the row is incremented and this finalize takes the
    // value it had. One statement, one expression, both branches consistent.
    let sql = "INSERT INTO invoice_number_sequences (merchant_id, prefix, next_number) \
               VALUES ($1, $2, 2) \
               ON CONFLICT (merchant_id) DO UPDATE \
                   SET next_number = invoice_number_sequences.next_number + 1 \
               RETURNING prefix, next_number - 1";

    let (prefix, assigned): (String, i64) = sqlx::query_as(sql)
        .bind(merchant_id)
        .bind(prefix)
        .fetch_one(&mut *conn)
        .await
        .map_err(classify_write)?;

    // Six digits, zero-padded, and *not* truncated beyond them: a merchant
    // who issues more than 999,999 invoices gets `PREFIX-1000000`, which is
    // still unique, still sorts correctly as a number, and still fits
    // `number_length`. Padding is for the common case, not a ceiling.
    Ok(format!("{prefix}-{assigned:06}"))
}

/// Finalizes a **draft** invoice inside the caller's transaction: assigns the
/// number, sums the lines once, and opens it.
///
/// `Ok(None)` means this merchant has no such draft invoice.
///
/// # Why the number is taken first, and why that is safe
///
/// The `UPDATE` needs the number as a bind parameter, so the sequence is
/// advanced first. If the `UPDATE` then matches nothing — the invoice was not
/// a draft, or not this merchant's — the caller returns `TxOutcome::Abandon`,
/// the transaction rolls back, and the sequence goes back with it. That is
/// the whole reason the sequence is a table row rather than a `SEQUENCE`,
/// stated as a live consequence rather than as a design note:
/// `a_refused_finalize_does_not_burn_a_number` is the test, and it is the one
/// that would fail if a future change moved the sequence onto `nextval`.
///
/// It is also why this function cannot be split into "get a number" and
/// "apply it": a caller holding a number outside the transaction that
/// consumed it is a caller who can drop it.
///
/// # `amount_due` is computed here and never again
///
/// The `FROM (SELECT COALESCE(SUM(amount), 0) …)` sums the lines in the same
/// statement that opens the invoice, so there is no instant at which the
/// invoice is `open` with a total that does not match its (now frozen) lines.
///
/// `amount_paid = 0` and `amount_refunded = 0` are named for [`resum_draft`]'s
/// reason: the statement writes every amount it is responsible for, so
/// `amounts_add_up` and `refunded_at_most_paid` are checks on this statement
/// rather than on columns it happened not to touch.
///
/// # Errors
///
/// [`DbError::Query`] if any statement fails, including
/// `number_is_assigned_at_finalize` and `amounts_add_up` — both of which
/// would mean this function and migration `0036` disagree.
pub(crate) async fn finalize_in_tx(
    conn: &mut PgConnection,
    merchant_id: &str,
    id: &str,
    prefix: &str,
    now: OffsetDateTime,
) -> Result<Option<InvoiceRow>, DbError> {
    let number = next_number_in_tx(conn, merchant_id, prefix).await?;

    let sql = format!(
        "UPDATE invoices SET \
            status = 'open', \
            number = $3, \
            amount_due = lines.total, \
            amount_paid = 0, \
            amount_remaining = lines.total, \
            amount_refunded = 0, \
            finalized_at = $4, \
            updated_at = $4 \
         FROM (SELECT COALESCE(SUM(amount), 0) AS total FROM invoice_items \
               WHERE invoice_id = $2) lines \
         WHERE invoices.merchant_id = $1 AND invoices.id = $2 AND invoices.status = 'draft' \
         RETURNING {QUALIFIED_COLUMNS}"
    );

    sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .bind(&number)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// Voids a `draft` or `open` invoice inside the caller's transaction.
///
/// `Ok(None)` means this merchant has no such invoice in a voidable state, or
/// it has a payment intent that has not been canceled — see
/// [`NO_LIVE_INTENT`](self).
///
/// # A voided invoice keeps its number
///
/// `number` is not cleared, and migration `0036`'s
/// `number_is_assigned_at_finalize` is what makes that unavoidable rather
/// than a choice this function could get wrong: the constraint says a
/// non-draft invoice **has** a number. A void that blanked it would be
/// refused by the database. That is the constraint doing the job the
/// paragraph in `docs/flows/invoices.md` describes — a number that vanished
/// would be a hole an accountant reads as a destroyed document.
///
/// A *draft* voided this way keeps `number IS NULL`, which the same
/// constraint refuses… so it does not: a draft cannot be voided, and this
/// statement's `status = 'open'` filter is why. See below.
///
/// # Only an `open` invoice can be voided
///
/// The `WHERE` names `'open'` alone. A draft is **deleted**, not voided
/// ([`Invoices::delete_draft`]), which is Stripe's own shape and is forced
/// here by `number_is_assigned_at_finalize`: a voided draft would be a
/// non-draft row with no number, which the database refuses. Rather than
/// discover that as a `23514` at runtime, the statement never attempts it.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails.
pub(crate) async fn void_in_tx(
    conn: &mut PgConnection,
    merchant_id: &str,
    id: &str,
    now: OffsetDateTime,
) -> Result<Option<InvoiceRow>, DbError> {
    let sql = format!(
        "UPDATE invoices SET status = 'void', voided_at = $3, updated_at = $3 \
         WHERE merchant_id = $1 AND id = $2 AND status = 'open' AND {NO_LIVE_INTENT} \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// Marks the invoice an intent was paying as paid, **inside the settlement
/// transaction**.
///
/// `Ok(None)` means this intent is not paying an invoice, or the invoice it
/// was paying is no longer `open`. Both are normal: most intents have no
/// invoice at all.
///
/// # Why this is here and not in the worker
///
/// [`crate::settlement`]'s `flip_session` argument, unchanged and stronger:
/// a second write after the settlement commits would leave a window in which
/// the intent is `succeeded` and the invoice still says money is owed, and a
/// crash in that window would make it permanent. There is no job that would
/// notice and this change adds none — an invoice has no poller.
///
/// # `amount_paid = amount_due`, and what that encodes
///
/// Partial payments are out of scope, and this statement is where that stops
/// being a sentence in a document: the intent was created for
/// `amount_remaining` and there is at most one intent per invoice, so a
/// settled intent pays the invoice in full or the invariant is already
/// broken. `paid_means_nothing_remaining` refuses the row otherwise.
///
/// # `pub(crate)` and reached only from `settlement`
///
/// `crate::checkout_sessions::settle_for_intent`'s visibility argument
/// verbatim: the point is that this write is not reachable without the
/// settlement it belongs to.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails, which **aborts the whole
/// settlement** — deliberately. An invoice that could not be flipped is an
/// invoice a merchant would chase for money that has already been taken;
/// rolling back means the poll job simply runs again.
pub(crate) async fn mark_paid_for_intent_in_tx(
    conn: &mut PgConnection,
    payment_intent_id: &str,
    now: OffsetDateTime,
) -> Result<Option<InvoiceRow>, DbError> {
    let sql = format!(
        "UPDATE invoices SET \
            status = 'paid', \
            amount_paid = amount_due, \
            amount_remaining = 0, \
            paid_at = $2, \
            updated_at = $2 \
         WHERE payment_intent_id = $1 AND status = 'open' \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
        .bind(payment_intent_id)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// Adds a succeeded refund to a **paid** invoice's running refunded total,
/// **inside the refund settlement transaction**.
///
/// `Ok(None)` means this intent is not paying an invoice, or the invoice it
/// paid is not `paid` — a `void` or `uncollectible` document, or one whose
/// settlement has not committed yet. Both are normal answers rather than
/// errors: most refunds will be against intents with no invoice at all.
///
/// # The compare-and-swap is `status = 'paid'`, and it is the whole guard
///
/// There is no `can_refund` beside this statement, for the module header's
/// reason. `WHERE payment_intent_id = $1 AND status = 'paid'` is what refuses
/// a refund recorded against a document that was voided, written off, or
/// never settled; "matched no row" is the refusal, and it cannot be raced
/// past because it is evaluated by the same statement that writes.
///
/// # The increment is an expression, not a value
///
/// `amount_refunded = amount_refunded + $2` rather than a total the caller
/// computed from a read. A read-then-write would let two refunds settling
/// concurrently both read `0` and both write their own amount, losing one of
/// them; the expression makes the second writer block on the row lock and
/// re-evaluate against the first's committed value. Migration `0042`'s
/// `refunded_at_most_paid` is then a real ceiling rather than an advisory
/// one: the over-refund is refused by the database, the error propagates, and
/// the transaction — including the refund row's own `pending` -> `succeeded`
/// flip — rolls back.
///
/// # What it deliberately does not do
///
/// It does not change `status`, `amount_paid`, `amount_remaining` or
/// `paid_at`, and it emits no event. The invoice stays `paid` and
/// `invoice.paid` is **not** re-emitted (D5): a merchant that received the
/// event once must not be told a second time that a bill was settled because
/// part of it came back.
///
/// # `pub(crate)` and reached only from [`crate::settlement`]
///
/// [`mark_paid_for_intent_in_tx`]'s visibility argument verbatim: the point
/// is that this write is not reachable without the settlement it belongs to.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails, including
/// `refunded_at_most_paid` for a refund larger than what was collected —
/// which **aborts the whole refund settlement**, deliberately. A refund total
/// that could not be written is a document that would understate what a payer
/// has been given back.
pub(crate) async fn add_refund_for_intent_in_tx(
    conn: &mut PgConnection,
    payment_intent_id: &str,
    amount: i64,
    now: OffsetDateTime,
) -> Result<Option<InvoiceRow>, DbError> {
    let sql = format!(
        "UPDATE invoices SET \
            amount_refunded = amount_refunded + $2, \
            updated_at = $3 \
         WHERE payment_intent_id = $1 AND status = 'paid' \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, InvoiceRow>(AssertSqlSafe(sql))
        .bind(payment_intent_id)
        .bind(amount)
        .bind(now)
        .fetch_optional(&mut *conn)
        .await
        .map_err(classify_write)
}

/// `time::OffsetDateTime` as the `chrono::DateTime<Utc>` CrateStack's
/// generated inputs take.
///
/// The third place this crate crosses the chrono/time boundary;
/// [`crate::customers`]' `to_chrono` carries the argument for why the
/// conversion is a reinterpretation rather than an approximation, and why the
/// unreachable fallback is `MAX_UTC`.
///
/// The fallback's *direction* is chosen for this module's own reason and it
/// is not the same one: the only value converted here is
/// [`Invoices::mark_uncollectible`]'s `now`, which lands in
/// `marked_uncollectible_at` — a timestamp, not a filter. `MAX_UTC` there
/// would be a visibly absurd date on a merchant's object rather than a silent
/// wrong answer, which is the failure mode to prefer. It cannot be reached:
/// `time::OffsetDateTime`'s range is four orders of magnitude narrower than
/// `chrono`'s, which [`crate::customers`]' own test proves at both extremes.
fn to_chrono(at: OffsetDateTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(at.unix_timestamp(), at.nanosecond())
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
}

/// `chrono::DateTime<Utc>` back to `time::OffsetDateTime`, for the rows
/// [`Invoices::items_for_invoice`] reads back through CrateStack.
///
/// The inverse of [`to_chrono`] and the first time this crate has needed one:
/// `customers`' two CrateStack calls are both writes, so nothing came back.
///
/// The fallback is `UNIX_EPOCH` and is likewise unreachable — a value that
/// left Postgres as a `timestamptz` is inside `time`'s range by construction.
/// `UNIX_EPOCH` rather than `MAX_UTC` here because these two columns are
/// rendered as `created` on a line object: an absurd *past* date is
/// recognisable as broken, where a date 200,000 years out would be read as
/// deliberate.
fn from_chrono(at: chrono::DateTime<chrono::Utc>) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(at.timestamp_nanos_opt().unwrap_or(0).into())
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

impl From<crate::schema::cratestack_schema::InvoiceItem> for InvoiceItemRow {
    /// The generated row, as this crate's own.
    ///
    /// A conversion rather than using the generated type directly, because
    /// the generated type is private to `vpay-db` (ADR-0016 standard 5,
    /// `cargo xtask verify-repositories`) and `InvoiceItemRow` is what
    /// crosses the crate boundary. It is also where the chrono/time boundary
    /// is crossed exactly once for this table.
    fn from(row: crate::schema::cratestack_schema::InvoiceItem) -> Self {
        Self {
            id: row.id,
            seq: row.seq,
            invoice_id: row.invoice_id,
            merchant_id: row.merchant_id,
            livemode: row.livemode,
            description: row.description,
            quantity: row.quantity,
            unit_amount: row.unit_amount,
            amount: row.amount,
            currency_code: row.currency_code,
            created_at: from_chrono(row.created_at),
            updated_at: from_chrono(row.updated_at),
        }
    }
}

#[cfg(test)]
mod tests {
    //! No database. Everything here is either a render of a statement
    //! (`preview_sql` does no I/O) or a question about the **compiled**
    //! `ModelDescriptor`, which is `schemas/vpay.cstack` as rustc saw it.
    //!
    //! `backends/tests/integration/tests/invoices.rs` is where a real
    //! Postgres proves the behaviour these assertions only imply, and
    //! `backends/tests/integration/tests/postgres_smoke.rs` is where
    //! migration `0036`'s constraints are proved to fire.

    use sqlx::postgres::PgPoolOptions;

    use super::{InvoiceItemPatch, InvoicePatch};

    /// A pool that has never opened a connection, and cannot: the port is
    /// unroutable. `connect_lazy` does no I/O, and neither does
    /// `preview_sql`. [`crate::customers`]' device.
    fn lazy_cratestack() -> crate::schema::cratestack_schema::Cratestack {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its URL and connects to nothing");
        crate::schema::cratestack_schema::Cratestack::builder(pool).build()
    }

    /// The generated create input has **no `metadata` field**, which is what
    /// keeps every write of `invoices` except `mark_uncollectible` a
    /// hand-written statement.
    ///
    /// This is the tripwire migration `0036` left the column's `DEFAULT` off
    /// to arm — `a_generated_customer_insert_cannot_carry_metadata`'s twin,
    /// and it should move for exactly the two reasons that one lists:
    /// somebody declared `metadata Json` on `model Invoice` (in which case
    /// re-read the GAP note about `Value::from_plain_json`'s number
    /// demotion), or upstream learned to carry an undeclared column.
    #[tokio::test]
    async fn a_generated_invoice_insert_cannot_carry_metadata() {
        use crate::schema::cratestack_schema as cs;

        let db = lazy_cratestack();
        let input = cs::CreateInvoiceInput {
            id: "in_0123456789abcdefghjkmnpq".to_owned(),
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            customer_id: "cus_0123456789abcdefghjkmnpq".to_owned(),
            currency_code: "XAF".to_owned(),
            status: cs::InvoiceStatus::draft,
            number: None,
            amount_due: 0,
            amount_paid: 0,
            amount_remaining: 0,
            amount_refunded: 0,
            due_date: None,
            description: None,
            payment_intent_id: None,
            finalized_at: None,
            paid_at: None,
            voided_at: None,
            marked_uncollectible_at: None,
        };

        let sql = db.invoice().create(input).preview_sql();

        // Split on `RETURNING` for `customers`' measured reason: the
        // `RETURNING` clause names every column of the model, so an assertion
        // over the whole statement would be about the wrong half.
        let (written, _returned) = sql
            .split_once(" RETURNING ")
            .expect("the generated insert returns the model projection");

        assert!(
            written.starts_with("INSERT INTO invoices ("),
            "this test is no longer looking at an invoices insert: {sql}"
        );
        assert!(
            !written.contains("metadata"),
            "`metadata` became expressible through the generated input. Before moving the \
             invoice writes onto it, re-read `model Invoice`'s GAP note: \
             `Value::from_plain_json` demotes any JSON number outside i64 to f64, and this \
             column is merchant-authored and is echoed back inside every `invoice.*` webhook \
             body. Statement: {sql}"
        );
    }

    /// Every action this module calls through CrateStack has an
    /// `@@allow` arm on its model.
    ///
    /// `customers`' test of the same name, and it exists for the same reason:
    /// `model Invoice`'s `update` arm fails **silently** if it is deleted
    /// (`update_many` compiles the policy into the statement's `WHERE`), so
    /// the error channel cannot be the signal. This one fails in
    /// milliseconds with no container.
    ///
    /// `model InvoiceItem`'s `read` arm is checked here too even though its
    /// loss *is* loud, because "the two arms are checked in one place" is
    /// what makes the list maintainable — and because a loud failure that
    /// only a container test can see is still an hour of somebody's day.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use crate::schema::cratestack_schema::models::{INVOICE_ITEM_MODEL, INVOICE_MODEL};

        assert!(
            !INVOICE_MODEL.update_allow_policies.is_empty(),
            "`@@allow(\"update\", auth().isSystem())` is missing from `model Invoice`: \
             `update_many` compiles its policy into the statement's own WHERE, so \
             `mark_uncollectible` would match zero rows and return Ok(false) forever — every \
             `POST /v1/invoices/{{id}}/mark_uncollectible` answering 404 for an invoice that is \
             sitting there open, with nothing in the error channel"
        );
        assert!(
            !INVOICE_ITEM_MODEL.read_allow_policies.is_empty(),
            "`@@allow(\"read\", auth().isSystem())` is missing from `model InvoiceItem`: \
             `find_many` authorises before the statement runs, so `items_for_invoice` would \
             return Forbidden and every read of an invoice would be a 500"
        );
    }

    /// The two patch types agree with their own `is_empty`.
    ///
    /// A unit test rather than a doctest for the exhaustive half: it walks
    /// every field, so adding a field to either struct without teaching
    /// `is_empty` about it fails here. The doctests on the two methods carry
    /// the *meaning*; this carries the completeness.
    #[test]
    fn a_patch_is_empty_only_when_no_field_is_set() {
        assert!(InvoicePatch::default().is_empty());
        for patch in [
            InvoicePatch {
                description: Some(None),
                ..InvoicePatch::default()
            },
            InvoicePatch {
                due_date: Some(Some(time::OffsetDateTime::UNIX_EPOCH)),
                ..InvoicePatch::default()
            },
            InvoicePatch {
                metadata: Some(serde_json::json!({})),
                ..InvoicePatch::default()
            },
        ] {
            assert!(!patch.is_empty(), "{patch:?} sets a field");
        }

        assert!(InvoiceItemPatch::default().is_empty());
        for patch in [
            InvoiceItemPatch {
                description: Some("a line".to_owned()),
                ..InvoiceItemPatch::default()
            },
            InvoiceItemPatch {
                quantity: Some(2),
                ..InvoiceItemPatch::default()
            },
            InvoiceItemPatch {
                unit_amount: Some(500),
                ..InvoiceItemPatch::default()
            },
        ] {
            assert!(!patch.is_empty(), "{patch:?} sets a field");
        }
    }
}
