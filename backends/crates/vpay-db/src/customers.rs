//! The `customers` repository (`backends/migrations/0034_create-customers.sql`)
//! — the reads and writes behind `/v1/customers` and behind the twelve-month
//! retention sweep.
//!
//! It keeps [`crate::checkout_sessions`]' three rules unchanged — every
//! merchant-facing query is merchant-scoped **in SQL**, state changes are
//! compare-and-swap, and the one unscoped query is named for its caller — and
//! adds the one this table has and no other does:
//!
//! * **a customer is erased, never flagged.** There is no `deleted_at` and no
//!   `@@soft_delete`. `docs/flows/customers.md` says why in full: a row that
//!   says "this person asked to be forgotten" is still the record of that
//!   person.
//!
//!   Since migration `0041` the erasure has two shapes, and neither is a soft
//!   delete. A customer nothing references is **hard-deleted** and the row is
//!   gone. A customer an intent, a session or an invoice references cannot be
//!   — the foreign keys are `NO ACTION`, deliberately, so a payment is never
//!   detached from the payer it was taken from — and is **anonymised**
//!   instead: the row stays, `anonymized_at` is stamped, and every identifier
//!   column on it becomes [`REDACTED`]. The difference from a soft delete is
//!   the whole point and the database enforces it, in
//!   `anonymized_customers_carry_the_marker`: a flagged row still holds the
//!   person, and this one holds nothing of theirs.
//!
//!   [`erase_in_tx`] also rewrites every copy of those identifiers vpay keeps
//!   *outside* this table — stored `customer.*` event bodies, `payer_ref` and
//!   the rail's own `failure_raw` prose on the customer's charges and their
//!   refunds, and any stored `POST /v1/customers` response — in the same
//!   transaction, because "vpay erased this payer" may not be true of one
//!   table and false of five.
//!
//! # The split between CrateStack and hand-written SQL, and what decides it
//!
//! Two of the seven methods below run through `schemas/vpay.cstack`'s
//! `model Customer` ([`Customers::touch_last_used`] and the hard-delete half
//! of [`erase_in_tx`]); five are hand-written `sqlx`. The line is drawn by one
//! column — `metadata JSONB NOT NULL` — for the two costs `model Customer`'s
//! GAP note measures, and by one query shape (the `seq` cursor's correlated
//! subquery).
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
const COLUMNS: &str = "id, seq, merchant_id, livemode, name, email, phone, \
                       address_line1, address_line2, address_city, address_state, \
                       address_postal_code, address_country, metadata, \
                       last_used_at, anonymized_at, created_at, updated_at";

/// What every identifier column of an anonymised customer holds.
///
/// One constant, three writers — the `UPDATE` in [`erase_in_tx`], the
/// `events.data` rewrite beside it, and [`CustomerRow::redacted`], which is
/// what the `customer.deleted` body is rendered from. They have to agree
/// exactly: the body a merchant receives claims to describe the row, and a
/// marker spelled two ways would make that claim false in a way no test that
/// looked at one of them could see.
///
/// It is also spelled in migration `0041`, in
/// `anonymized_customers_carry_the_marker` — deliberately, because that is
/// what turns "the erasure wrote every column" from a promise into something
/// Postgres refuses to let be untrue. `an_anonymised_customer_carries_the_marker_in_every_identifier_column`
/// in `postgres_smoke.rs` writes *this* constant into a real row, so changing
/// it here without changing the migration fails rather than drifts.
///
/// Square brackets rather than a bare word so it can never be mistaken for a
/// value a payer supplied: `redacted` is a plausible surname and a plausible
/// city, and `[redacted]` is not a plausible anything.
pub const REDACTED: &str = "[redacted]";

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

/// A customer's postal address — Stripe's six components (migration `0041`,
/// [issue #67](https://github.com/vaam-apps/vpay/issues/67)).
///
/// # Why a struct here when the table has six columns
///
/// [`CustomerRow`] is otherwise one-to-one with `customers`, and this is the
/// one place it is not. The reason is that the six columns are never
/// meaningful apart: the wire object nests them under one `address` key, an
/// update replaces the whole address rather than merging components, and the
/// erasure writes all six or none. Six loose `Option<String>` fields on the
/// row would let a caller do five of those things, and every one of the five
/// is a bug that compiles.
///
/// Every component is `Option<String>` and an all-`None` value is the same
/// thing as "this customer has no address" — see [`Self::is_empty`], which is
/// what decides whether the object renders `address: null` or an object.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomerAddress {
    /// Street address, line 1.
    pub line1: Option<String>,
    /// Street address, line 2 — apartment, suite, PO box.
    pub line2: Option<String>,
    /// City, district, suburb, town or village.
    pub city: Option<String>,
    /// State, county, province or region.
    pub state: Option<String>,
    /// ZIP or postal code.
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2, upper case — `vpay_api::v1::customers` validates
    /// the shape on the way in and migration `0041`'s
    /// `address_country_is_iso_3166_1_alpha_2` is the backstop.
    pub country: Option<String>,
}

impl CustomerAddress {
    /// Whether this customer has no address at all.
    ///
    /// The wire object renders `address: null` for an empty address and a
    /// six-key object otherwise, so this is the function that decides which —
    /// and it is `Option`-free on purpose: "an address whose every component
    /// is absent" and "no address" are the same fact about the payer, and an
    /// `Option<CustomerAddress>` on the row would have made them two.
    ///
    /// ```
    /// use vpay_db::CustomerAddress;
    ///
    /// assert!(CustomerAddress::default().is_empty());
    /// assert!(!CustomerAddress {
    ///     city: Some("Douala".to_owned()),
    ///     ..CustomerAddress::default()
    /// }
    /// .is_empty());
    /// ```
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.line1.is_none()
            && self.line2.is_none()
            && self.city.is_none()
            && self.state.is_none()
            && self.postal_code.is_none()
            && self.country.is_none()
    }

    /// Every component replaced by [`REDACTED`] — the address of a customer
    /// that has been anonymised.
    ///
    /// **All six, including the ones the payer never filled in**, which is
    /// migration `0041`'s rule and not this function's convenience: which
    /// components a record carried is itself information about the person, so
    /// an erasure that left the absent ones `NULL` would publish the shape of
    /// the record it claims to have erased.
    fn redacted() -> Self {
        let marker = || Some(REDACTED.to_owned());
        Self {
            line1: marker(),
            line2: marker(),
            city: marker(),
            state: marker(),
            postal_code: marker(),
            country: marker(),
        }
    }
}

/// One `customers` row, exactly as stored.
///
/// Not the wire object: `vpay-api` owns that shape (`created` as unix
/// seconds, `last_used_at` omitted entirely). This struct is deliberately
/// one-to-one with the table so a change to either is a compile error rather
/// than a silently dropped column.
///
/// `Debug` is **hand-written** below rather than derived, because
/// [`Self::name`], [`Self::email`], [`Self::phone`] and [`Self::address`] are
/// a payer's personal data — see that impl.
///
/// `FromRow` is hand-written too, since 2026-09-10, and for a reason that is
/// not style: [`Self::address`] is one struct over six columns, and
/// `#[derive(sqlx::FromRow)]` has no way to say so — there is no field-prefix
/// attribute in sqlx 0.9, and `#[sqlx(flatten)]` looks for `line1`, not
/// `address_line1`. The impl below names every column exactly once, which is
/// the property the derive was here for.
#[derive(Clone, PartialEq)]
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
    /// The payer's postal address, decoded from the six `address_*` columns
    /// (migration `0041`). All-`None` means the customer has no address —
    /// see [`CustomerAddress::is_empty`].
    pub address: CustomerAddress,
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
    /// When this payer's identifiers were erased, or `None` for a live
    /// customer (migration `0041`).
    ///
    /// `Some` is a promise the database keeps rather than one this struct
    /// makes: `anonymized_customers_carry_the_marker` refuses a row whose
    /// `anonymized_at` is set and whose identifier columns are anything other
    /// than [`REDACTED`]. It is **not** a soft-delete flag — there is no
    /// predicate anywhere that hides such a row, and `GET
    /// /v1/customers/{id}` answers it with `deleted: true` rather than a
    /// `404`, which is the whole point: a merchant's stored `cus_…` keeps
    /// resolving to something.
    pub anonymized_at: Option<OffsetDateTime>,
    /// When the customer was created, as supplied to [`NewCustomer`].
    pub created_at: OffsetDateTime,
    /// When the row last changed. Maintained by the writers here, not by a
    /// trigger.
    pub updated_at: OffsetDateTime,
}

/// Decodes every column of `customers`, nesting the six `address_*` ones
/// into [`CustomerAddress`].
///
/// Hand-written rather than derived only because of that nesting — see
/// [`CustomerRow`]'s own doc. Each column is named once, so a column added to
/// [`COLUMNS`] and not here fails to compile rather than being silently
/// dropped, which is the property the derive provided.
impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for CustomerRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row as _;

        Ok(Self {
            id: row.try_get("id")?,
            seq: row.try_get("seq")?,
            merchant_id: row.try_get("merchant_id")?,
            livemode: row.try_get("livemode")?,
            name: row.try_get("name")?,
            email: row.try_get("email")?,
            phone: row.try_get("phone")?,
            address: CustomerAddress {
                line1: row.try_get("address_line1")?,
                line2: row.try_get("address_line2")?,
                city: row.try_get("address_city")?,
                state: row.try_get("address_state")?,
                postal_code: row.try_get("address_postal_code")?,
                country: row.try_get("address_country")?,
            },
            metadata: row.try_get("metadata")?,
            last_used_at: row.try_get("last_used_at")?,
            anonymized_at: row.try_get("anonymized_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

impl CustomerRow {
    /// This row as it will stand once it is erased: every identifier
    /// [`REDACTED`], `anonymized_at` set.
    ///
    /// # Why the caller renders the event body from this rather than from the
    /// row the `UPDATE` returns
    ///
    /// `customer.deleted` carries the **redacted** object, and it has to be
    /// built before the write for the branch where there is nothing left to
    /// read afterwards: a customer with no payment history is hard-deleted,
    /// so the row the event describes does not exist by the time the event is
    /// written. One projection serves both branches, which is what keeps the
    /// two bodies identical in shape — a merchant cannot tell from the
    /// webhook which branch ran, and there is no reason they should.
    ///
    /// `metadata`, `created_at`, `livemode` and the ids are untouched:
    /// `metadata` is the **merchant's** own data, not the payer's, and the
    /// rest describes the record rather than the person.
    #[must_use]
    pub fn redacted(&self, at: OffsetDateTime) -> Self {
        let marker = || Some(REDACTED.to_owned());
        Self {
            name: marker(),
            email: marker(),
            phone: marker(),
            address: CustomerAddress::redacted(),
            anonymized_at: Some(at),
            ..self.clone()
        }
    }
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
            // The address gets a component *count* and not six redacted
            // lengths: an operator debugging `address_line1_length` needs to
            // know an address is present and which component is over its
            // bound, and the bound that fired is in the CHECK's own name in
            // the error. Six more `[N chars redacted]` fields would treble the
            // width of every line this struct appears on and answer nothing
            // the CHECK name does not.
            .field(
                "address",
                &format_args!(
                    "{{{} component(s) redacted}}",
                    [
                        &self.address.line1,
                        &self.address.line2,
                        &self.address.city,
                        &self.address.state,
                        &self.address.postal_code,
                        &self.address.country,
                    ]
                    .iter()
                    .filter(|component| component.is_some())
                    .count()
                ),
            )
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
            // Printed in full: it is an instant vpay wrote, not anything of
            // the payer's, and "has this customer been erased?" is the first
            // question an operator looking at one of these rows has.
            .field("anonymized_at", &self.anonymized_at)
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
    /// The payer's postal address, or [`CustomerAddress::default`] for none
    /// (migration `0041`).
    ///
    /// Not one of the identifiers [`Self::name`] describes: an address alone
    /// does not name anybody, so `at_least_one_identifier` ignores it and a
    /// create carrying only an address is refused exactly as one carrying
    /// nothing is.
    pub address: CustomerAddress,
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
    /// The address, **replaced whole** rather than merged component-wise.
    ///
    /// `None` = the request did not mention `address`. `Some(address)` = this
    /// is the address now, and every component the request did not name is
    /// cleared; `Some(CustomerAddress::default())` is therefore how
    /// `address=` clears it.
    ///
    /// # Why replacement and not the three-state merge the scalars get
    ///
    /// An address is one fact, not six. A merchant correcting a payer's
    /// street who left `city` out of the request meant "this is the address",
    /// and a component-wise merge would silently keep the old city beside the
    /// new street — an address that was never anybody's, assembled by vpay
    /// out of two. The failure mode of replacement is visible on the next
    /// read; the failure mode of merging is a plausible wrong address.
    ///
    /// It also makes the two states enough. Merging would need the third
    /// (`address[city]=` clearing one component while leaving the rest),
    /// which is exactly the shape that produces the half-updated address
    /// above. `docs/flows/customers.md` records this as a decision.
    pub address: Option<CustomerAddress>,
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
            && self.address.is_none()
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
/// [`crate::CheckoutSessions::expire_due`], [`Customers::erase_idle`],
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
    // `anonymized_at` is not in the column list at all, and that is the
    // point rather than an omission: a customer is created live, always, and
    // a parameter for it would be a way to insert a row that claims a payer
    // was erased before they ever existed.
    let sql = format!(
        "INSERT INTO customers (id, merchant_id, livemode, name, email, phone, \
         address_line1, address_line2, address_city, address_state, \
         address_postal_code, address_country, metadata, \
         last_used_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14) \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
        .bind(&new.id)
        .bind(&new.merchant_id)
        .bind(new.livemode)
        .bind(new.name.as_deref())
        .bind(new.email.as_deref())
        .bind(new.phone.as_deref())
        .bind(new.address.line1.as_deref())
        .bind(new.address.line2.as_deref())
        .bind(new.address.city.as_deref())
        .bind(new.address.state.as_deref())
        .bind(new.address.postal_code.as_deref())
        .bind(new.address.country.as_deref())
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
    // The address is **one** flag over six columns, which is the statement
    // saying what `CustomerPatch::address` says: an address is replaced
    // whole, so a request that named `address[line1]` and not `address[city]`
    // clears the city. Six flags would be six components a caller could move
    // independently, and the shape this crate would then have to defend is
    // "half of the payer's old address and half of their new one".
    let sql = format!(
        "UPDATE customers SET \
            name = CASE WHEN $3::BOOLEAN THEN $4::TEXT ELSE name END, \
            email = CASE WHEN $5::BOOLEAN THEN $6::TEXT ELSE email END, \
            phone = CASE WHEN $7::BOOLEAN THEN $8::TEXT ELSE phone END, \
            address_line1 = CASE WHEN $9::BOOLEAN THEN $10::TEXT ELSE address_line1 END, \
            address_line2 = CASE WHEN $9::BOOLEAN THEN $11::TEXT ELSE address_line2 END, \
            address_city = CASE WHEN $9::BOOLEAN THEN $12::TEXT ELSE address_city END, \
            address_state = CASE WHEN $9::BOOLEAN THEN $13::TEXT ELSE address_state END, \
            address_postal_code = \
                CASE WHEN $9::BOOLEAN THEN $14::TEXT ELSE address_postal_code END, \
            address_country = CASE WHEN $9::BOOLEAN THEN $15::TEXT ELSE address_country END, \
            metadata = CASE WHEN $16::BOOLEAN THEN $17::JSONB ELSE metadata END, \
            last_used_at = GREATEST(last_used_at, $18), \
            updated_at = $18 \
         WHERE merchant_id = $1 AND id = $2 \
         RETURNING {COLUMNS}"
    );

    let address = patch.address.clone().unwrap_or_default();

    sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
        .bind(merchant_id)
        .bind(id)
        .bind(patch.name.is_some())
        .bind(patch.name.clone().flatten())
        .bind(patch.email.is_some())
        .bind(patch.email.clone().flatten())
        .bind(patch.phone.is_some())
        .bind(patch.phone.clone().flatten())
        .bind(patch.address.is_some())
        .bind(address.line1)
        .bind(address.line2)
        .bind(address.city)
        .bind(address.state)
        .bind(address.postal_code)
        .bind(address.country)
        .bind(patch.metadata.is_some())
        .bind(patch.metadata.clone())
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify_write)
}

/// Which of the two shapes an erasure took.
///
/// Returned rather than inferred by the caller, because the two are
/// observably different afterwards and a caller that guessed would be
/// guessing about personal data: after [`Self::HardDeleted`] a `GET` is a
/// `404`, and after [`Self::Anonymized`] it is a `200` carrying an object
/// whose every identifier is [`REDACTED`]. `vpay-api` does not currently
/// branch on it — both answer the same `{deleted: true}` — and it is returned
/// anyway so that the worker's log line can say which happened, which is the
/// only place the distinction is visible to an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomerErasure {
    /// The customer had no payment history and the row is gone.
    HardDeleted,
    /// An intent, a session or an invoice references the customer, so the row
    /// stays and every identifier on it is now [`REDACTED`].
    Anonymized,
}

/// The per-key rule that turns a stored `customer.*` event body — or a stored
/// `POST /v1/customers` response — into the redacted one.
///
/// # Why a stored event body is redacted at all
///
/// `events.data` is a snapshot of the whole rendered object (migration
/// `0018`) and nothing prunes `events`. So every `customer.created` and
/// `customer.updated` vpay has ever written holds the payer's name, email,
/// phone and address, forever, on the far side of a "deletion" that erased
/// the customer row. That was the largest surviving copy of a payer's
/// identifiers after a delete, and no code named it — issue #68 is written
/// about intents and sessions, which never carried one.
///
/// The merchant *received* those identifiers when the event was delivered and
/// holds their own copy; that is theirs and vpay cannot reach it. What vpay
/// can do is stop being a second store of it, and the retention promise to
/// the payer is what decides between the two.
///
/// # Why it names the keys instead of walking for "anything string-shaped"
///
/// `metadata` is the merchant's own data and stays — a redaction that
/// rewrote it would destroy a merchant's records to protect a payer whose
/// details are not in it. `id`, `object`, `created`, `livemode` and `deleted`
/// describe the record, not the person. So the set is closed and spelled, and
/// a tenth identifier added to `CustomerObject` without being added here is
/// what `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`
/// is for: it scans every text and JSONB column in `information_schema`
/// rather than the ones this constant happens to name.
///
/// Every key is rewritten unconditionally, `null` included, for
/// [`CustomerAddress::redacted`]'s reason: "which fields did this payer fill
/// in?" is information about them.
///
/// `$2` is [`REDACTED`] and `$3` the six-component redacted address, bound by
/// the caller from the same constant the row itself is written with.
const REDACT_CUSTOMER_KEY: &str = "CASE \
     WHEN field.key IN ('name', 'email', 'phone') THEN to_jsonb($2::TEXT) \
     WHEN field.key = 'address' THEN $3::JSONB \
     ELSE field.value END";

/// Erases one customer that the caller has already read **under this
/// transaction's row lock**, and writes the `customer.deleted` that tells its
/// merchant so.
///
/// # The branch, and why the database decides it rather than the caller
///
/// `payment_intents.customer_id`, `checkout_sessions.customer_id` (migration
/// `0034`) and `invoices.customer_id` (`0036`) are `NO ACTION` foreign keys.
/// A customer any of them references cannot be deleted — that is the property
/// that keeps a payment attached to the payer it was taken from, and it is
/// not being given up. What changes (migration `0041`) is what vpay does
/// instead of refusing: the row stays and every identifier on it becomes
/// [`REDACTED`], so the payment record survives with **no payer on it**.
///
/// A customer with no history is still hard-deleted, through the generated
/// `delete_many` (see `hard_delete` below, named without a link because it
/// is private), so `model Customer`'s `@@allow("delete", …)` stays a
/// permission something exercises.
///
/// The `SELECT NOT (…)` that decides between them runs inside this
/// transaction and under the caller's lock on the customer row — but that
/// lock does **not** stop a concurrent `POST /v1/payment_intents` inserting a
/// reference, because the insert only takes a *share* lock on the customer.
/// So the branch can be wrong by one race, in exactly one direction, and the
/// database catches it: if history appears between the `SELECT` and the
/// `DELETE`, the delete raises `23503` and the whole transaction rolls back,
/// leaving the customer live and the merchant a `409`-free retry that will
/// take the other branch. The opposite race cannot happen — history is never
/// removed.
///
/// # What else is written, and why each one is here rather than in a sweep
///
/// Five more statements, all in this transaction:
///
/// 1. the `customer.deleted` event, whose body is the **redacted** object;
/// 2. every stored `customer.*` event body for this object — see
///    [`REDACT_CUSTOMER_KEY`] — **including the one step 1 just wrote**, so
///    the invariant is "no `events` row holds this payer's identifiers" and
///    not "no `events` row except the newest one";
/// 3. `charges.payer_ref` for every charge on this customer's intents,
///    replaced by the marker, with `payer_ref_masked` cleared and the rail's
///    verbatim `failure_raw` prose replaced too. `payer_ref` is the payer's
///    MSISDN as the rail was given it and is reachable from a customer only
///    through an intent, which is why nothing looking at `customers` alone
///    ever found it; `failure_raw` is unbounded text a rail wrote *about*
///    this payer and may quote their number back;
/// 4. `refunds.failure_raw` for the refunds of those charges, for step 3's
///    reason and reached the same way;
/// 5. `idempotency_keys.response_body` for any stored `POST /v1/customers`
///    response naming this customer — the exact JSON that was answered, kept
///    for 24 hours to replay. A replay after an erasure now answers the
///    redacted object, which is the same thing a fresh `GET` answers.
///
/// A sweep over these afterwards would be a window in which "vpay erased this
/// payer" is true of one table and false of five, on a promise a payer was
/// given. One transaction makes that window not exist.
///
/// # `event_data` is the caller's
///
/// [`Customers::idle_since`]'s reason, unchanged: the body is the *rendered*
/// wire object and only `vpay-api` knows that shape. It is rendered from
/// [`CustomerRow::redacted`] before this is called, which is also what makes
/// one projection serve the branch where the row no longer exists to read.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] on `events_pkey` for a replayed `event_id`.
/// [`DbError::Persistence`] wrapping [`crate::PersistenceError::Denied`] if
/// `model Customer` lost its `@@allow("delete", …)` — the one refusal that is
/// otherwise silent, spelled here because a hard delete that matched no row
/// while this transaction holds the row's lock cannot be anything else.
/// [`DbError::Query`] for any statement that fails, including the `23503`
/// above; the transaction is rolled back either way, so a failed erasure
/// leaves a live customer rather than a half-erased one.
pub(crate) async fn erase_in_tx(
    cs: &crate::schema::cratestack_schema::Cratestack,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &CustomerRow,
    now: OffsetDateTime,
    event_id: &str,
    event_data: &serde_json::Value,
) -> Result<CustomerErasure, DbError> {
    // The variable is `sql`, and shadowed per statement below, because
    // `crate::sql_audit` requires it: the injection waiver must wrap a
    // variable of exactly that name, so the interpolation audit and the
    // waiver are looking at the same string. (This comment cannot spell the
    // wrapper's name — the scanner matches the literal text and would read
    // its own quotation as a site.)
    let sql = format!("SELECT NOT ({UNREFERENCED}) FROM customers WHERE id = $1");
    let has_history: bool = sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(&row.id)
        .fetch_one(&mut **tx)
        .await
        .map_err(DbError::Query)?;

    let erasure = if has_history {
        anonymize(tx, row, now).await?;
        CustomerErasure::Anonymized
    } else {
        hard_delete(cs, tx, row).await?;
        CustomerErasure::HardDeleted
    };

    // The event first, so the redaction below covers it too — see this
    // function's own doc, point 2.
    crate::events::insert_in_tx(
        &mut *tx,
        &crate::NewEvent {
            id: event_id.to_owned(),
            merchant_id: row.merchant_id.clone(),
            livemode: row.livemode,
            event_type: EVENT_CUSTOMER_DELETED.to_owned(),
            object_id: row.id.clone(),
            data: event_data.clone(),
        },
    )
    .await?;

    redact_stored_copies(tx, &row.id, &row.merchant_id).await?;
    Ok(erasure)
}

/// Replaces every identifier column with [`REDACTED`] and stamps
/// `anonymized_at`.
///
/// One statement assigning all nine columns from **one** bind, which is the
/// shape migration `0041`'s `anonymized_customers_carry_the_marker` is
/// written to police: a `SET` list that missed a column produces a row the
/// database refuses outright rather than a row that says a payer was erased
/// while holding one of their details.
///
/// `anonymized_at IS NULL` is in the `WHERE` even though the caller checked
/// it under the row lock, for [`update_in_tx`]'s reason: a guard belongs in
/// the write. `last_used_at` is deliberately not moved — an erasure is not a
/// use, and moving it would reset the retention clock of a record there is
/// nothing left to retain.
async fn anonymize(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &CustomerRow,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let sql = format!(
        "UPDATE customers SET \
            name = $2, email = $2, phone = $2, \
            address_line1 = $2, address_line2 = $2, address_city = $2, \
            address_state = $2, address_postal_code = $2, address_country = $2, \
            anonymized_at = $3, updated_at = $3 \
         WHERE id = $1 AND merchant_id = $4 AND anonymized_at IS NULL \
         RETURNING {COLUMNS}"
    );

    sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
        .bind(&row.id)
        .bind(REDACTED)
        .bind(now)
        .bind(&row.merchant_id)
        .fetch_one(&mut **tx)
        .await
        .map(|_| ())
        .map_err(classify_write)
}

/// Removes a customer nothing references, through the generated
/// `delete_many`.
///
/// Through CrateStack rather than as a hand-written `DELETE` so that
/// `a_customer_delete_is_a_delete_and_not_a_soft_delete` goes on pinning the
/// statement that actually reaches Postgres: adding `@@soft_delete` to
/// `model Customer` is a one-line schema edit that compiles, passes
/// `check-schema`, and would turn every hard delete into a flag on a row that
/// keeps the payer's name, email and phone.
///
/// A summary of zero is impossible here — the caller holds this row's lock
/// and has just read it — with one exception, and it is the exception the
/// model's own comment calls the most dangerous line in the file: a lost
/// `@@allow("delete", …)` compiles its refusal into the statement's `WHERE`
/// and matches nothing, silently. That is why this returns an error rather
/// than `Ok(())` on zero.
async fn hard_delete(
    cs: &crate::schema::cratestack_schema::Cratestack,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &CustomerRow,
) -> Result<(), DbError> {
    let summary = cs
        .customer()
        .delete_many()
        .where_(crate::schema::cratestack_schema::customer::id().eq(row.id.clone()))
        .where_(
            crate::schema::cratestack_schema::customer::merchant_id().eq(row.merchant_id.clone()),
        )
        .run_in_tx(&mut *tx, &system_context())
        .await
        .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))?;

    // `.value`, because `run_in_tx` wraps the ordinary return in a
    // `RunInTxOutcome` carrying the audit events it persisted inside this
    // transaction (cratestack#534). `model Customer` is not `@@audit`-enabled,
    // so the vector is empty and there is nothing to fan out; the wrapper is
    // still what a caller who owns the transaction is handed.
    let summary = summary.value;
    if summary.ok == 1 {
        return Ok(());
    }
    Err(DbError::from(crate::PersistenceError::Denied {
        model: MODEL,
        action: "delete",
        detail: format!(
            "delete_many matched {} row(s) for a customer this transaction holds the lock on; \
             the only way that happens is a lost @@allow(\"delete\", …) on model Customer, \
             whose policy is compiled into the statement's own WHERE",
            summary.ok
        ),
    }))
}

/// Rewrites every copy of this payer's identifiers vpay keeps outside
/// `customers`.
///
/// Four statements, and the set is closed by measurement rather than by
/// intuition: `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`
/// scans every `text`, `varchar` and `jsonb` column `information_schema`
/// knows about, so a fifth store added later fails that test rather than
/// waiting to be noticed here.
///
/// # The rail's own words are a copy too, and they were missed on 2026-09-10
///
/// `charges.failure_raw` and `refunds.failure_raw` hold **the rail's
/// message, verbatim** — `"{code}: {message}"` from MTN's `Reason` and
/// Orange's `raw_reason` — kept so an unmapped decline survives for whoever
/// fixes the mapping table (`docs/flows/failures.md`). Neither is an
/// identifier column and that is exactly why the first pass's enumeration of
/// "the copies that survived a deletion, in full" did not list them: they are
/// unbounded text a mobile-money rail wrote about this payer, and a rail that
/// answers `PAYER_NOT_FOUND: subscriber 2376… is not registered` has put the
/// payer's MSISDN in vpay's database in a column nothing redacts.
///
/// They are replaced by the marker rather than parsed for numbers, because a
/// redaction that had to recognise every spelling a rail might use is a
/// redaction that fails silently on the first one it has not seen. The
/// `failure_code` beside each survives, so *why* the payment failed is still
/// answerable after the payer is gone; only the rail's prose goes. `NULL`
/// stays `NULL` — a charge that never failed must not grow a failure, and
/// `refunds.failure_paired` would refuse the row if it did.
///
/// `refunds.reason` is deliberately left alone: it is the **merchant's** free
/// text about their own refund ("duplicate", "requested\_by\_customer"), the
/// same kind of thing `metadata` is, and the same argument keeps it.
///
/// # `provider_requests` and `webhook_deliveries`
///
/// `provider_requests` deliberately has no redaction: migration `0016` stores
/// no request or response body, only a status code, an attempt number and an
/// operator-facing `error_kind`. `webhook_deliveries` likewise keeps
/// `payload_sha256` and not the payload.
async fn redact_stored_copies(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    customer_id: &str,
    merchant_id: &str,
) -> Result<(), DbError> {
    let address = serde_json::json!({
        "line1": REDACTED,
        "line2": REDACTED,
        "city": REDACTED,
        "state": REDACTED,
        "postal_code": REDACTED,
        "country": REDACTED,
    });

    // `COALESCE(…, events.data)` and not a bare subquery: `jsonb_object_agg`
    // over an empty object is NULL, and `events.data` is NOT NULL. A
    // `customer.*` body is never empty, so the fallback is unreachable — and
    // an unreachable fallback that keeps the row is the direction to fail in
    // when the alternative is a violated NOT NULL aborting the erasure.
    let sql = format!(
        "UPDATE events SET data = COALESCE( \
             (SELECT jsonb_object_agg(field.key, {REDACT_CUSTOMER_KEY}) \
              FROM jsonb_each(events.data) AS field(key, value)), \
             events.data) \
         WHERE object_id = $1 AND type LIKE 'customer.%'"
    );
    sqlx::query(AssertSqlSafe(sql))
        .bind(customer_id)
        .bind(REDACTED)
        .bind(&address)
        .execute(&mut **tx)
        .await
        .map_err(classify_write)?;

    // The stored `POST /v1/customers` response, kept for 24 hours so a
    // retried request answers what the original did. Matched on the body's
    // own `object`/`id` rather than on the request path, because the path is
    // stored as text and the body is the thing that actually holds the
    // identifiers. Merchant-scoped as well, because the primary key is
    // `(merchant_id, idempotency_key)` and this crate never reaches across a
    // tenant even when the id it holds could only belong to one.
    let sql = format!(
        "UPDATE idempotency_keys SET response_body = COALESCE( \
             (SELECT jsonb_object_agg(field.key, {REDACT_CUSTOMER_KEY}) \
              FROM jsonb_each(idempotency_keys.response_body) AS field(key, value)), \
             response_body) \
         WHERE merchant_id = $4 \
           AND response_body->>'object' = 'customer' \
           AND response_body->>'id' = $1"
    );
    sqlx::query(AssertSqlSafe(sql))
        .bind(customer_id)
        .bind(REDACTED)
        .bind(&address)
        .bind(merchant_id)
        .execute(&mut **tx)
        .await
        .map_err(classify_write)?;

    // `charges.payer_ref` is the payer's MSISDN as the rail was given it and
    // `payer_ref_masked` the display form of the same number. The marker
    // rather than NULL for the first, so a charge whose payer vpay *did* know
    // stays distinguishable from a redirect-rail charge where it never did;
    // NULL for the second, because a mask is a rendering of a value that no
    // longer exists and `[redacted]` is already the value.
    //
    // Reached through the intents, which is the only path from a customer to
    // a charge — and the reason this column survived every previous reading
    // of "what does a customer deletion leave behind?".
    let charges = "UPDATE charges SET payer_ref = $2, payer_ref_masked = NULL, \
             failure_raw = CASE WHEN failure_raw IS NULL THEN NULL ELSE $2 END \
         WHERE payment_intent_id IN \
               (SELECT id FROM payment_intents WHERE customer_id = $1)";
    sqlx::query(charges)
        .bind(customer_id)
        .bind(REDACTED)
        .execute(&mut **tx)
        .await
        .map_err(classify_write)?;

    // The refund's half of the same column. Reached through the charges,
    // which are reached through the intents. `failure_code` is untouched, so
    // `refunds.failure_paired` — "a code with no raw text is a half-written
    // failure" — still holds either way round.
    let refunds = "UPDATE refunds SET \
             failure_raw = CASE WHEN failure_raw IS NULL THEN NULL ELSE $2 END \
         WHERE charge_id IN \
               (SELECT c.id FROM charges c \
                JOIN payment_intents p ON p.id = c.payment_intent_id \
                WHERE p.customer_id = $1)";
    sqlx::query(refunds)
        .bind(customer_id)
        .bind(REDACTED)
        .execute(&mut **tx)
        .await
        .map_err(classify_write)?;

    Ok(())
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

    /// Every live customer idle since before `horizon`, oldest first, at most
    /// `limit` of them — the retention sweep's backlog query.
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
    /// # It no longer carries the `NOT EXISTS` guard, and that is the change
    ///
    /// This query excluded every customer an intent, a session or an invoice
    /// referenced, because the delete that followed it could not have removed
    /// one: the foreign keys are `NO ACTION`. Offering one would have minted
    /// an `evt_…` for a deletion Postgres was about to refuse.
    ///
    /// Since migration `0041` a referenced customer *is* erasable — it is
    /// anonymised rather than deleted — so excluding it here would have
    /// exempted from the twelve-month retention promise exactly the customers
    /// the promise is about: the ones vpay has taken money from. The
    /// `NOT EXISTS` triple survives as [`UNREFERENCED`], where
    /// `erase_in_tx` uses it to choose between the two shapes; this query
    /// no longer needs it, and `the_sweep_guard_names_every_table_that_can_reference_a_customer`
    /// still pins the set of tables at three.
    ///
    /// What it does exclude is `anonymized_at IS NOT NULL`. Without that
    /// clause an anonymised customer stays idle for ever and the sweep offers
    /// it again on the next pass, and the one after — an hourly
    /// `customer.deleted` about a payer already erased, for the life of the
    /// deployment.
    ///
    /// # Why this stays raw SQL
    ///
    /// The `IS NULL` filter alone would go through `find_many`, but this
    /// query returns a [`CustomerRow`] and that struct carries `metadata`,
    /// which `model Customer` does not declare and must not (see the model's
    /// GAP note). A generated read could not build the row at all.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn idle_since(
        &self,
        horizon: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<CustomerRow>, DbError>;

    /// Erases one idle customer — hard-deleting it if nothing references it
    /// and anonymising it if something does — **and** appends the
    /// `customer.deleted` event that tells its merchant so, in one
    /// transaction. `Ok(None)` means it was no longer eligible.
    ///
    /// The other half of [`Customers::idle_since`], and the same argument
    /// [`crate::CheckoutSessions::expire_due`] makes for being one function
    /// rather than two calls: a crash between the erasure and the event would
    /// erase a merchant's customer with nobody ever told, and there is no
    /// sweep over "customers erased without an event" and no way to build one
    /// for the branch where the row is gone. One transaction makes that
    /// window not exist.
    ///
    /// # The guard is the statement, and it is re-evaluated here
    ///
    /// `last_used_at < horizon` and `anonymized_at IS NULL`, re-checked
    /// inside this transaction under `SELECT … FOR UPDATE` rather than
    /// trusted from [`Customers::idle_since`]. A merchant can use the
    /// customer between the read and the write, and a customer a payment was
    /// just taken from must not be erased on the strength of a read taken
    /// before it. `Ok(None)` is the **normal** answer for that, for a
    /// concurrent sweep, and for a merchant who deleted it by hand — and no
    /// event is written on that path, which is what makes a second sweep
    /// produce no second `customer.deleted`.
    ///
    /// The `NOT EXISTS` triple is **not** part of the guard any more: since
    /// migration `0041` a referenced customer is anonymised rather than
    /// skipped, so it decides the shape of the erasure instead of whether one
    /// happens. See [`Customers::idle_since`].
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
    /// # `event_data` is rendered from a row this method has not read yet
    ///
    /// The caller renders it from the page [`Customers::idle_since`] handed
    /// it, redacted through [`CustomerRow::redacted`] — which is what lets
    /// one projection serve both branches, including the one where the row is
    /// gone by the time the event is written. Everything in a redacted object
    /// except `metadata` is immutable, so the only value that can be stale is
    /// a merchant's `metadata` changed between the two reads; that was true
    /// of the delete this replaces and is not made worse here.
    ///
    /// # Errors
    ///
    /// [`DbError::UniqueViolation`] on `events_pkey` if `event_id` has
    /// already been emitted, and everything `erase_in_tx` lists — named
    /// without a link because it is `pub(crate)`. **The transaction is rolled
    /// back on any of them, so the customer survives whole and the next sweep
    /// retries it.**
    async fn erase_idle(
        &self,
        id: &str,
        horizon: OffsetDateTime,
        now: OffsetDateTime,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<CustomerErasure>, DbError>;
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

/// The `NOT EXISTS` triple that decides which shape an erasure takes, read by
/// [`erase_in_tx`] and by nothing else since migration `0041`.
///
/// **It stopped being a guard and became a branch, and the difference is the
/// whole of issue #68.** Until 2026-09-10 this was in
/// [`Customers::idle_since`]'s `WHERE` as well, so a customer any of the
/// three tables referenced was skipped by the retention sweep and refused by
/// `DELETE /v1/customers/{id}` — which exempted from the twelve-month
/// promise exactly the payers the promise is about, the ones vpay had taken
/// money from. It now selects between hard-deleting the row and anonymising
/// it; every customer is erasable either way.
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
/// Without the third `NOT EXISTS`, nothing breaks *loudly*, and what breaks
/// changed with the meaning of the constant. It used to be a wasted `evt_…`
/// once an hour for a customer Postgres would refuse to delete. It is now
/// worse: [`erase_in_tx`] would take the **hard-delete** branch for an
/// invoiced customer, `invoices.customer_id`'s `NO ACTION` foreign key would
/// raise `23503`, and the whole erasure — event, redactions and all — would
/// roll back. The payer would not be erased, and the merchant would be told
/// nothing. `an_invoiced_customer_is_anonymised_rather_than_deleted` is the
/// test, and it is what a missing clause fails on.
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

    async fn idle_since(
        &self,
        horizon: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<CustomerRow>, DbError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM customers \
             WHERE last_used_at < $1 AND anonymized_at IS NULL \
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

    async fn erase_idle(
        &self,
        id: &str,
        horizon: OffsetDateTime,
        now: OffsetDateTime,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<CustomerErasure>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // `FOR UPDATE`, and the whole row rather than a `RETURNING` off the
        // write, because the branch `erase_in_tx` takes needs the row's
        // merchant and livemode *before* either write — and on the
        // hard-delete branch there is nothing left to return them from.
        // Re-evaluating the guard here rather than trusting `idle_since`'s
        // page is what makes a customer used between the two survive.
        let sql = format!(
            "SELECT {COLUMNS} FROM customers \
             WHERE id = $1 AND last_used_at < $2 AND anonymized_at IS NULL \
             FOR UPDATE"
        );

        let Some(row) = sqlx::query_as::<_, CustomerRow>(AssertSqlSafe(sql))
            .bind(id)
            .bind(horizon)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::Query)?
        else {
            // No longer eligible. The transaction is dropped, which rolls it
            // back; nothing was written, and no event describes an erasure
            // that did not happen.
            return Ok(None);
        };

        let erasure = erase_in_tx(&self.cs, &mut tx, &row, now, event_id, event_data).await?;

        tx.commit().await.map_err(DbError::Query)?;
        Ok(Some(erasure))
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
            // The six address columns and `anonymized_at` joined the model in
            // migration 0041 and are therefore fields of this input. Spelled
            // out rather than defaulted, because `CreateCustomerInput` has no
            // `Default` and — more to the point — a struct literal is what
            // makes a column added to the model without being thought about
            // here a compile error.
            address_line1: None,
            address_line2: None,
            address_city: None,
            address_state: None,
            address_postal_code: None,
            address_country: None,
            anonymized_at: None,
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
    /// (`a_customers_erasure_is_scoped_to_its_merchant`).
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

    /// The erasure's branch names **every** referencing table, and correlates
    /// to the alias its caller uses.
    ///
    /// One constant rather than a copy per call site, and since migration
    /// `0041` there is only one call site — `super::erase_in_tx`, which reads
    /// it to choose between hard-deleting a customer and anonymising one.
    /// What a missing table costs changed with that: it used to be a wasted
    /// `evt_…` per hour, and it is now a **failed erasure**. The branch would
    /// choose the hard delete, the `NO ACTION` foreign key would raise
    /// `23503`, and the whole transaction — event and redactions included —
    /// would roll back, leaving the payer un-erased and the merchant told
    /// nothing.
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

    /// The redaction rewrites every identifier key of a stored `customer.*`
    /// body and **no other key**.
    ///
    /// A statement-text assertion and not a behaviour one, deliberately: the
    /// behaviour is proved against a real Postgres by
    /// `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`,
    /// and what *this* pins is the pair of properties that test cannot
    /// distinguish. A rewrite that also flattened `metadata` would still
    /// leave no payer identifier anywhere and would have destroyed a
    /// merchant's own records to get there; a rewrite that dropped `id` would
    /// leave an event nothing can be correlated to. Both are silent.
    #[test]
    fn the_event_redaction_names_the_identifiers_and_spares_the_merchants_data() {
        for identifier in ["'name'", "'email'", "'phone'", "'address'"] {
            assert!(
                super::REDACT_CUSTOMER_KEY.contains(identifier),
                "{identifier} is a payer identifier on the customer object and is not \
                 rewritten by the stored-event redaction: {}",
                super::REDACT_CUSTOMER_KEY
            );
        }
        for merchants_own in ["metadata", "created", "livemode", "object"] {
            assert!(
                !super::REDACT_CUSTOMER_KEY.contains(merchants_own),
                "`{merchants_own}` is the merchant's own data or the record's own shape, and \
                 rewriting it would destroy a merchant's records to keep a promise made to \
                 somebody else: {}",
                super::REDACT_CUSTOMER_KEY
            );
        }
        assert!(
            super::REDACT_CUSTOMER_KEY.contains("ELSE field.value END"),
            "without the ELSE arm every key not named above is dropped from the stored body \
             — `id` included: {}",
            super::REDACT_CUSTOMER_KEY
        );
    }

    /// The marker is what migration `0041`'s CHECK spells, character for
    /// character.
    ///
    /// The two are a pair: the migration refuses an `anonymized_at` row whose
    /// identifiers are anything but this literal, so changing it here alone
    /// turns every erasure into a `23514` — at runtime, on the one path that
    /// must not fail. This reads the migration off disk rather than restating
    /// it, so the failure arrives at `cargo nextest` instead.
    #[test]
    fn the_redaction_marker_is_the_one_the_migration_enforces() {
        let migration =
            include_str!("../../../migrations/0041_customers-address-and-anonymisation.sql");
        // `concat`, not `format!`, and for `the_retention_stamp_moves_one_column_and_never_backwards`'s
        // reason one line up rather than one line back: `crate::sql_audit`
        // reads a `format!` as statement-building when the word `sql` appears
        // in the forty characters before it, and the migration's own
        // *filename* ends in `.sql`. The audit then reports a positional
        // capture in a test that touches no database.
        let quoted = ["'", super::REDACTED, "'"].concat();
        assert!(
            migration.contains(&quoted),
            "`vpay_db::customers::REDACTED` is {quoted}, which \
             anonymized_customers_carry_the_marker does not name. Every erasure would be a \
             23514 — a CHECK violation on the one write a payer was promised"
        );
        assert_eq!(
            migration.matches(&quoted).count(),
            // Nine, once per identifier column of
            // `anonymized_customers_carry_the_marker` — the migration's prose
            // spells the marker without quotes, so this counts the CHECK and
            // only the CHECK. The count rather than a `contains` so that a
            // column *dropped* from it fails here rather than quietly
            // stopping being checked.
            9,
            "the marker CHECK must name all nine identifier columns"
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
