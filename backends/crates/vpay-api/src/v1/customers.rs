//! `/v1/customers` — create, retrieve, update, list, delete.
//!
//! The merchant-owned record of a payer they expect to see again (S4a). It is
//! Stripe's `customer`, narrowed in every field but one: `id`, `name`,
//! `email`, `phone`, `address`, `metadata`, `created`, `livemode` — and
//! `deleted`, which appears only on an erased one. The exception is
//! `address`, which is *wider* than Stripe's; see below.
//! `docs/flows/customers.md` is the long version of everything below; this
//! header is the short one.
//!
//! **Tenancy.** Every query takes the [`MerchantScope`] the authentication
//! middleware resolved. A merchant asking for another merchant's `cus_…` gets
//! the same 404, byte for byte, as one asking for an id that never existed —
//! and a merchant naming another merchant's `cus_…` in `customer` on an
//! intent or a session gets the same `400` as one naming an id that does not
//! exist.
//!
//! # The address is the formal address AND the GPS point
//!
//! The maintainer, 2026-09-11: *"address in our system means both formal as
//! well as GPS"*. `address` therefore carries `latitude_microdeg` and
//! `longitude_microdeg` beside Stripe's six components — a **deliberate
//! divergence**, since Stripe's address has no coordinate at all, and one
//! stated in `docs/api/README.md` and in both SDKs rather than left to be
//! discovered from a response.
//!
//! Two things about the wire shape are rules and not details. The value is a
//! whole number of **microdegrees** and the unit is in the field name, so no
//! JSON float ever appears on this object — a field called `latitude` would
//! be read as degrees, and the first `4.061` would be a value CrateStack's
//! `Value::from_plain_json` demotes to `f64`. And the two halves are sent
//! together or not at all: half a coordinate names no place, so
//! [`validated_address`] answers a `400` naming `address` rather than letting
//! `address_coordinates_are_both_or_neither` answer a `503`.
//!
//! # The three rules that are this resource and not the others
//!
//! 1. **Phone is stored by default and phone-only is legal.** The
//!    maintainer's decision of 2026-09-05. There is no opt-in, no
//!    configuration flag and no `store_phone` parameter; a merchant who sends
//!    `phone` gets it stored, and a customer whose only content is a phone
//!    number is a complete customer. What the API does add is
//!    *canonicalisation*: the number is stored in the same
//!    `2376XXXXXXXX` form a rail is given, through
//!    [`super::account_holders::canonical_msisdn`], so vpay never holds two
//!    spellings of one payer.
//! 2. **`DELETE` erases the payer; it never flags them.** There is no
//!    `deleted_at` and no tombstone, because a row that says "this person
//!    asked to be forgotten" is still the record of that person. Since
//!    migration `0041` the erasure has two shapes and neither is a soft
//!    delete: a customer nothing references is **hard-deleted** and a later
//!    `GET` is a `404`; one an intent, a session or an invoice references is
//!    **anonymised** — the row stays, every identifier on it becomes
//!    `[redacted]`, and a later `GET` answers `200` with `deleted: true`.
//!    `anonymized_customers_carry_the_marker` is what makes the second a
//!    database invariant rather than two call sites remembering.
//! 3. **A customer with payment history is never *detached* from it.** The
//!    foreign keys migration `0034` adds are `NO ACTION` and stay that way,
//!    so the payment record survives the erasure with no payer on it. What
//!    changed on 2026-09-10 (issues #68, #96 item 2) is what happens instead
//!    of refusing: this route used to answer a `409` advising the merchant
//!    to clear `name`, `email` and `phone` — advice
//!    `at_least_one_identifier` refuses, so it could never be followed. The
//!    `409` is gone; [`delete`] now always succeeds or answers the uniform
//!    `404`, and the erasure covers every copy of the payer vpay kept
//!    outside `customers` too (`vpay_db::customers::erase_in_tx`).
//!
//!    The one `409` left on this resource is the opposite fact: an **erased**
//!    customer cannot be updated or attached to a new payment ([`delete`]'s
//!    own doc, and `erased_customer`).
//!
//! # What is *not* here
//!
//! The twelve-month retention sweep is `vpay_worker::handlers`' —
//! `sweep_idle_customers`, a job kind of its own. This module never erases on
//! a timer and has no idea what the horizon is.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use vpay_core::ids;
use vpay_db::{
    CustomerAddress, CustomerListPage, CustomerPatch, CustomerRow, Customers, NewCustomer,
    Repositories, ResponseSubject, TxOutcome, UnitOfWork as _,
};

use crate::error::ApiError;
use crate::form::VpayQuery;
use crate::model::{CustomerObject, CustomerTag, DeletedObject, DeletedTrue, ListObject};
use crate::v1::paging::{self, CursorKind};
use crate::v1::payment_intents::{ClaimOutcome, PostRequest, json_response};
use crate::v1::{MerchantScope, ResourceConfig};

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a 404 for a customer can never be spelled two ways.
pub(crate) const RESOURCE: &str = "customer";

/// The list envelope's `url`, and the path a cursor page is read from.
const LIST_URL: &str = "/v1/customers";

/// This resource's cursor vocabulary — `cus_…`.
const CURSOR: CursorKind = CursorKind {
    prefix: ids::CUSTOMER_PREFIX,
    noun: "a customer id",
};

/// `metadata` bounds — `payment_intents`' three constants, which are Stripe's
/// own limits and `docs/api/README.md`'s.
///
/// Restated here rather than imported for one reason and it is not
/// convenience: `payment_intents`' copies are private to that module and
/// making them `pub(crate)` would put three numbers on the crate's internal
/// surface for a rule every resource already states. They are asserted
/// against each other in [`the_metadata_bounds_are_the_ones_every_other_resource_uses`](self).
const METADATA_MAX_KEYS: usize = 50;
/// See [`METADATA_MAX_KEYS`].
const METADATA_MAX_KEY_CHARS: usize = 40;
/// See [`METADATA_MAX_KEYS`].
const METADATA_MAX_VALUE_CHARS: usize = 500;

/// `name`'s ceiling — migration `0034`'s `name_length`, at the boundary where
/// it can name the parameter.
///
/// The CHECK is the backstop, exactly as `checkout_sessions`' URL bounds are:
/// trip the CHECK and an over-long name comes back as a `500` telling a
/// merchant vpay is broken; trip this and it comes back as a `400` naming the
/// parameter, which is the truth.
const NAME_MAX_CHARS: usize = 256;
/// `email`'s ceiling — migration `0034`'s `email_length`. See
/// [`NAME_MAX_CHARS`].
const EMAIL_MAX_CHARS: usize = 512;

/// Every free-text address component's ceiling — migration `0041`'s five
/// `address_*_length` CHECKs. See [`NAME_MAX_CHARS`] for why the bound is
/// here at all.
///
/// The same number as [`NAME_MAX_CHARS`] on purpose: a street line and a city
/// are the same order of thing as a person's name, and a merchant who hits
/// one bound and not the other for no reason they can see is a worse API than
/// one number.
const ADDRESS_MAX_CHARS: usize = 256;

/// `address[postal_code]`'s ceiling — migration `0041`'s
/// `address_postal_code_length`. Shorter than [`ADDRESS_MAX_CHARS`] because
/// the longest postal code in use anywhere is ten characters and a 256-
/// character one is a merchant putting the wrong field in the box.
const POSTAL_CODE_MAX_CHARS: usize = 64;

/// `|address[latitude_microdeg]|`'s ceiling — 90 degrees, in millionths.
///
/// The **definition of the unit** rather than a product limit, which is why
/// it is `90_000_000` here and `90000000` in migration `0041`'s
/// `address_latitude_microdeg_range` and nowhere else in between: a value
/// outside it is not a place. `the_coordinate_bounds_are_the_ones_the_migration_enforces`
/// reads the migration off disk and compares, so the two cannot drift into
/// disagreeing about what a latitude is.
const LATITUDE_MAX_MICRODEG: i64 = 90_000_000;

/// `|address[longitude_microdeg]|`'s ceiling — 180 degrees, in millionths.
/// See [`LATITUDE_MAX_MICRODEG`].
const LONGITUDE_MAX_MICRODEG: i64 = 180_000_000;

// ----------------------------------------------------------------- create

/// `POST /v1/customers`'s fields, as the form decoder produces them.
///
/// Every scalar is `Option<String>` for `payment_intents::CreateParams`'
/// reason: the wire is form-encoded, so every value arrives as text, and
/// typing them here would hand "not a phone number" to serde — which answers
/// with `param: "body"` and a sentence about the request's shape rather than
/// naming the field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateParams {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    /// Bracket-encoded (`address[line1]=…`) by [`crate::form::parse_form`],
    /// exactly as `payment_method_data[mtn_momo][msisdn]` is.
    address: Option<AddressParam>,
    /// Bracket-encoded (`metadata[order_id]=1234`) by
    /// [`crate::form::parse_form`], exactly as on an intent.
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

/// `address` as it arrives, in the two shapes the wire can spell it.
///
/// # Why an untagged enum and not `Option<AddressParams>`
///
/// The form decoder turns `address[city]=Douala` into an object and
/// `address=` into the empty **string**, and both are things a merchant
/// legitimately sends: the first sets the address and the second clears it.
/// An `Option<AddressParams>` could only decode the first, and `address=`
/// would come back as serde's "invalid type: string" with `param: "body"` —
/// a sentence about the request's shape rather than the name of the field
/// the merchant is trying to clear.
///
/// The variant order is load-bearing. `untagged` tries them in order, and
/// [`Self::Cleared`] is first because a `String` cannot absorb an object
/// while an all-optional struct *can* absorb almost anything: the other order
/// would make `address=` decode as an `AddressParams` with every field
/// absent, which is silently the same as "the request did not mention
/// address" and would make clearing an address impossible.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", untagged)]
enum AddressParam {
    /// `address=` — the whole address, cleared. Any *non*-blank scalar is
    /// refused by [`validated_address`] rather than accepted here, because
    /// `address=Douala` is a merchant reaching for `address[city]`.
    Cleared(String),
    /// `address[line1]=…` — the address, replaced by these components.
    ///
    /// Boxed for `clippy::large_enum_variant`: eight `Option<String>`s beside
    /// a `String` is exactly the imbalance that lint is about — six since
    /// 2026-09-10, eight since the GPS half landed on 2026-09-11.
    Components(Box<AddressParams>),
}

/// The eight components, as the form decoder produces them — Stripe's six
/// formal ones and vpay's coordinate.
///
/// Every one is `Option<String>` for [`CreateParams`]' reason — the wire is
/// text — and there is no `deny_unknown_fields`, which is this API's standing
/// behaviour rather than a decision taken here: `address[county]=…` is
/// ignored exactly as `nonsense=1` is on every other route.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AddressParams {
    line1: Option<String>,
    line2: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
    /// The GPS half, in whole microdegrees. `Option<String>` like every other
    /// field here and **not** an `Option<i64>`, for [`CreateParams`]' reason
    /// applied where it matters most: typing it would hand `4.061` to serde,
    /// which answers `param: "body"` and a sentence about the request's shape
    /// — where the merchant needs to be told that this field is millionths of
    /// a degree and that `4.061` is spelled `4061000`. See
    /// [`checked_microdeg`].
    latitude_microdeg: Option<String>,
    /// See [`Self::latitude_microdeg`].
    longitude_microdeg: Option<String>,
}

/// `POST /v1/customers`.
///
/// # Ordering: the key is claimed first, then the request
///
/// The `Idempotency-Key` claim runs before any rule, for
/// `payment_intents::create`'s reason: a replay must answer whatever the
/// original answered, whatever has changed since.
///
/// # Why every failure releases the key
///
/// Nothing is written before the insert, so re-executing a corrected retry is
/// exactly equivalent to the request never having been made.
/// `payment_intents::create`'s carve-out, applied here.
pub(crate) async fn create(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;

    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let validated = match validate_create(&post).await {
        Ok(validated) => validated,
        Err(error) => {
            post.release(repositories.as_ref(), &scope, claim_id).await;
            return Err(error);
        }
    };

    let new = NewCustomer {
        id: ids::customer_id(),
        merchant_id: scope.merchant_id().to_owned(),
        livemode: config.livemode(),
        name: validated.name,
        email: validated.email,
        phone: validated.phone,
        address: validated.address,
        metadata: Value::Object(validated.metadata),
        created_at: OffsetDateTime::now_utc(),
    };

    // The id is minted above rather than read back out of the response, which
    // is what lets the store be told what this body is about without anything
    // here inspecting it. See [`delete`] for the race the subject closes.
    let subject = ResponseSubject::Customer { id: &new.id };

    let outcome = create_with_event(repositories.as_ref(), &new)
        .await
        .and_then(|row| customer_response(StatusCode::CREATED, &row));

    post.finish(repositories.as_ref(), &scope, claim_id, outcome, subject)
        .await
}

/// A create request that has passed every rule, in the shape the insert
/// needs.
#[derive(Debug)]
struct ValidCreate {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    address: CustomerAddress,
    metadata: Map<String, Value>,
}

/// Every rule that can be decided from the request alone.
async fn validate_create(post: &PostRequest) -> Result<ValidCreate, ApiError> {
    let params: CreateParams = post.form().await?;

    let name = checked_text(present(params.name), "name", NAME_MAX_CHARS)?;
    let email = checked_text(present(params.email), "email", EMAIL_MAX_CHARS)?;
    let phone = checked_phone(present(params.phone))?;
    // `address=` on a **create** means "no address", not "clear the address":
    // it is what a client templating an optional field emits, and there is
    // nothing to clear. `present`'s rule applied one level up.
    let address = validated_address(params.address)?.unwrap_or_default();

    // The one-of rule, at the boundary, where it can name all three
    // parameters. Migration `0034`'s `at_least_one_identifier` is the
    // backstop — and, being multi-column, it is invisible to
    // `cratestack migrate baseline` in both directions, so
    // `postgres_smoke.rs` asserts it against a real Postgres directly.
    if name.is_none() && email.is_none() && phone.is_none() {
        return Err(at_least_one_identifier());
    }

    Ok(ValidCreate {
        name,
        email,
        phone,
        address,
        metadata: validated_metadata(&params.metadata)?,
    })
}

// --------------------------------------------------------------- retrieve

/// `GET /v1/customers/{id}`.
pub(crate) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = Customers::get_for_merchant(repositories.as_ref(), scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;
    customer_response(StatusCode::OK, &row)
}

// ----------------------------------------------------------------- update

/// `POST /v1/customers/{id}`'s fields.
///
/// # Why `Option<String>` carries three states here and two on create
///
/// On create, absent and empty both mean "no value". On update they mean
/// different things, and a merchant depends on the difference: an absent key
/// leaves the field alone, and `name=` **clears** it. That is Stripe's
/// contract and it is the only way an email a payer asked to have removed can
/// be removed without deleting the whole customer.
///
/// The decoder gives `None` for an absent key and `Some("")` for a present
/// empty one, so the two states are already distinguishable here; the
/// mapping into [`vpay_db::CustomerPatch`]'s double options is
/// [`patch_field`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct UpdateParams {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    /// Absent leaves the address alone; `address=` clears it; components
    /// **replace** it whole. See [`vpay_db::CustomerPatch::address`] for why
    /// the third is a replacement rather than a component-wise merge.
    address: Option<AddressParam>,
    /// Absent means "leave metadata alone". Present means "merge these keys",
    /// per key, which is Stripe's semantics and is why the merge needs the
    /// stored map — see [`update`].
    metadata: Option<BTreeMap<String, String>>,
}

/// `POST /v1/customers/{id}`.
///
/// # Why this reads before it writes, and why the read takes the row lock
///
/// `metadata` is **merged** key-wise, not replaced: Stripe's contract is that
/// `metadata[a]=1` on a customer that already has `b` leaves `b` alone, and
/// `metadata[b]=` removes `b`. Merging needs the stored map, so this handler
/// reads the customer first — which makes the update a read-modify-write.
///
/// **Until 2026-09-10 that window was open and documented** (issue #66): the
/// read ran on the pool, two concurrent updates each adding one key could
/// lose one of them, and this doc comment said so and left it. It could be
/// left while nothing depended on the result being definite. It cannot be
/// now, because the same transaction emits `customer.updated` and a merchant
/// acting on that event would be acting on a `metadata` the database no
/// longer holds.
///
/// So the read, the merge, the write and the event are one transaction, and
/// the read is `SELECT … FOR UPDATE`. A second request blocks on the lock,
/// re-reads the **committed** merge, and merges onto that; the two events
/// then describe the two states in the order they happened. The merge itself
/// stays in Rust rather than becoming a `jsonb ||` in the statement, for the
/// reason this comment always gave: Stripe's semantics belong in the layer
/// that documents them, not in a migration.
///
/// What was never at risk is the three scalar fields: each is written from
/// the request alone, so a concurrent update that touched a different field
/// cannot be clobbered by this one — the statement assigns only the columns
/// the request mentioned (`vpay_db::customers`' `CASE WHEN $n::BOOLEAN`
/// form).
///
/// # An empty patch writes nothing, and emits nothing
///
/// A bodiless `POST` — which is what an SDK sends for "touch this object" —
/// answers the customer unchanged rather than running an `UPDATE` that would
/// move `updated_at`. Stripe behaves the same way, and a merchant diffing on
/// timestamps depends on it. No `customer.updated` is written either: an
/// event about a change that did not happen is a webhook a merchant has to
/// work out how to ignore.
pub(crate) async fn update(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = update_once(&post, repositories.as_ref(), &scope, &id).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Customer { id: &id },
    )
    .await
}

/// The update itself, split out of [`update`] so "a failure from here still
/// ends the claim" is one call with one error path.
async fn update_once(
    post: &PostRequest,
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    let params: UpdateParams = post.form().await?;

    // The read, the merge, the write and the event are **one** transaction,
    // and the read takes the row's lock. See [`update_with_event`].
    let outcome: TxOutcome<UpdateOutcome> = repositories
        .transaction(|tx| {
            Box::pin(async move {
                // Scoped, so a foreign customer is indistinguishable from a
                // missing one and this handler never compares two merchant
                // ids in Rust. `FOR UPDATE`, so the merge below is computed
                // over a value no concurrent update can be halfway through
                // changing.
                let Some(current) = tx.lock_customer_for_update(scope.merchant_id(), id).await?
                else {
                    return Ok::<_, ApiError>(TxOutcome::Abandon(UpdateOutcome::NotFound));
                };

                if current.anonymized_at.is_some() {
                    // An anonymised customer is a record with no payer in it,
                    // kept only so a merchant's stored `cus_…` resolves. An
                    // update would put a name, an email or an address back on
                    // to it — undoing an erasure a payer was promised — and
                    // `at_least_one_identifier` would not object, because
                    // `[redacted]` is not NULL. It has to be refused here.
                    //
                    // A `409` and not a `404`: the object exists and a `GET`
                    // answers it, so a `404` would be two routes disagreeing
                    // about whether a `cus_…` is real. `Category::Conflict`'s
                    // own definition is "the object's state forbids it".
                    return Err(erased_customer());
                }

                let patch = validate_update(params, &current)?;
                if patch.is_empty() {
                    // Stripe's no-op: a bodiless `POST` answers the object
                    // unchanged. Abandoned rather than committed, because
                    // nothing was written — and **no event**, because
                    // nothing changed. Manufacturing a `customer.updated`
                    // here would be a webhook about a transition that did
                    // not happen.
                    return Ok(TxOutcome::Abandon(UpdateOutcome::Unchanged(Box::new(
                        current,
                    ))));
                }

                let Some(row) = tx
                    .update_customer_in_tx(
                        scope.merchant_id(),
                        id,
                        &patch,
                        OffsetDateTime::now_utc(),
                    )
                    .await?
                else {
                    // Unreachable: the locked read above found the row and
                    // holds it until this transaction ends, so nothing can
                    // have deleted it in between. Spelled as an outcome
                    // rather than an `expect` (ADR-0007), and it answers the
                    // 404 a merchant retrying would get anyway.
                    return Ok(TxOutcome::Abandon(UpdateOutcome::NotFound));
                };

                // The event's `data` is the row this transaction wrote —
                // including the merged `metadata`, which is the value the
                // lock exists to make definite.
                let object = CustomerObject::try_from(&row)?;
                let data =
                    serde_json::to_value(&object).map_err(ApiError::internal_serialization)?;
                tx.insert_in_tx(&vpay_db::NewEvent {
                    id: ids::event_id(),
                    merchant_id: row.merchant_id.clone(),
                    livemode: row.livemode,
                    event_type: EVENT_UPDATED.to_owned(),
                    object_id: row.id.clone(),
                    data,
                })
                .await?;

                Ok(TxOutcome::Commit(UpdateOutcome::Updated(Box::new(row))))
            })
        })
        .await?;

    match outcome.into_inner() {
        UpdateOutcome::NotFound => Err(not_found(id)),
        UpdateOutcome::Unchanged(row) | UpdateOutcome::Updated(row) => {
            customer_response(StatusCode::OK, &row)
        }
    }
}

/// The three endings [`update_once`]'s transaction has.
///
/// A named enum rather than an `Option<Option<CustomerRow>>`, because two of
/// the three are `TxOutcome::Abandon` for *different* reasons and the caller
/// answers them differently: one is a `404`, one is a `200` with the object
/// unchanged, and only the third emits anything.
///
/// The row is boxed because `CustomerRow` is the largest value in this enum
/// by a wide margin and `clippy::large_enum_variant` is right about it.
enum UpdateOutcome {
    /// No such customer for this merchant.
    NotFound,
    /// The patch would change nothing; the stored row, unwritten and
    /// un-evented.
    Unchanged(Box<CustomerRow>),
    /// Written, with one `customer.updated` in the same transaction.
    Updated(Box<CustomerRow>),
}

/// The `type` of the event a create emits.
///
/// Spelled as a constant for `vpay_db::settlement`'s reason: the type is a
/// property of *which transition this is*, and a caller free to choose it
/// could report a created customer as a deleted one. Both types entered
/// `type_is_a_documented_event` in migration `0039`, in the same change that
/// wrote them — migration `0023`'s lockstep rule, which
/// `payment_intent.canceled` is the repository's one counter-example to.
const EVENT_CREATED: &str = "customer.created";

/// See [`EVENT_CREATED`]. A **bodiless** update emits nothing: the patch is
/// empty, nothing is written, and an event about it would describe a
/// transition that did not happen.
const EVENT_UPDATED: &str = "customer.updated";

/// Inserts a customer and appends `customer.created` beside it, in one
/// transaction.
///
/// # Why the object is rendered from the returned row
///
/// `seq`, `created_at` and `updated_at` are the database's, so a projection
/// of the request would be a second implementation of the insert — the
/// mistake `vpay_api::v1::invoices::write_with_event` records at length. The
/// row this returns is also what the `201` renders, so the body a merchant
/// reads and the body their webhook carries are the same object by
/// construction.
///
/// # Errors
///
/// Whatever the insert returns, or [`ApiError::Internal`] if the written row
/// will not render — which migration `0034`'s CHECKs make impossible.
async fn create_with_event(
    repositories: &dyn Repositories,
    new: &NewCustomer,
) -> Result<CustomerRow, ApiError> {
    let outcome: TxOutcome<CustomerRow> = repositories
        .transaction(|tx| {
            Box::pin(async move {
                let row = tx.insert_customer_in_tx(new).await?;

                let object = CustomerObject::try_from(&row)?;
                let data =
                    serde_json::to_value(&object).map_err(ApiError::internal_serialization)?;
                tx.insert_in_tx(&vpay_db::NewEvent {
                    id: ids::event_id(),
                    merchant_id: row.merchant_id.clone(),
                    livemode: row.livemode,
                    event_type: EVENT_CREATED.to_owned(),
                    object_id: row.id.clone(),
                    data,
                })
                .await?;

                Ok::<_, ApiError>(TxOutcome::Commit(row))
            })
        })
        .await?;

    Ok(outcome.into_inner())
}

/// Turns the request into a patch, refusing anything the database would
/// refuse and one thing it would not.
///
/// The one thing: **clearing the last identifier**. `at_least_one_identifier`
/// would catch it as a `23514`, which reaches a merchant as a `500` saying
/// vpay is broken. Deciding it here means a `400` naming all three parameters
/// — the same message a create with none gets — and it needs the *stored* row
/// to decide, because "the last one" is a fact about the merge of the request
/// and the row rather than about either alone.
fn validate_update(params: UpdateParams, current: &CustomerRow) -> Result<CustomerPatch, ApiError> {
    let name = patch_field(params.name, "name", NAME_MAX_CHARS)?;
    let email = patch_field(params.email, "email", EMAIL_MAX_CHARS)?;
    let phone = match params.phone {
        None => None,
        Some(raw) => Some(checked_phone(present(Some(raw)))?),
    };
    let address = validated_address(params.address)?;

    // What each field will be once this patch lands: the patch's value where
    // it says something, the stored value where it does not.
    let after = |patched: &Option<Option<String>>, stored: &Option<String>| -> bool {
        patched
            .as_ref()
            .map_or_else(|| stored.is_some(), Option::is_some)
    };
    if !after(&name, &current.name)
        && !after(&email, &current.email)
        && !after(&phone, &current.phone)
    {
        return Err(at_least_one_identifier());
    }

    let metadata = match params.metadata {
        None => None,
        Some(sent) => Some(Value::Object(merged_metadata(&current.metadata, &sent)?)),
    };

    Ok(CustomerPatch {
        name,
        email,
        phone,
        address,
        metadata,
    })
}

/// One text field's three states, bounded.
///
/// `None` → the request did not mention it. `Some(None)` → `name=`, clear it.
/// `Some(Some(v))` → set it.
fn patch_field(
    raw: Option<String>,
    param: &'static str,
    max_chars: usize,
) -> Result<Option<Option<String>>, ApiError> {
    match raw {
        None => Ok(None),
        Some(raw) => Ok(Some(checked_text(present(Some(raw)), param, max_chars)?)),
    }
}

/// Stripe's key-wise metadata merge: the sent map's keys win, and a key sent
/// **empty** is removed.
///
/// The result is bounded as a whole rather than the delta being bounded, and
/// that is the direction that matters: a merchant adding one key to a
/// customer that already has fifty must be refused, and bounding the delta
/// would let them add one key fifty times.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `metadata`, or [`ApiError::Internal`]
/// for a stored `metadata` that is not an object — which
/// `metadata_is_object` makes impossible.
fn merged_metadata(
    stored: &Value,
    sent: &BTreeMap<String, String>,
) -> Result<Map<String, Value>, ApiError> {
    let mut merged = stored
        .as_object()
        .cloned()
        .ok_or_else(|| ApiError::Internal("customers.metadata is not an object".to_owned()))?;

    for (key, value) in sent {
        if key.chars().count() > METADATA_MAX_KEY_CHARS {
            return Err(metadata_key_too_long());
        }
        if value.is_empty() {
            // Stripe's per-key delete. Not `Value::Null`: a null would be a
            // stored key whose value is nothing, which reads back as a key
            // the merchant thinks they removed.
            merged.remove(key);
            continue;
        }
        if value.chars().count() > METADATA_MAX_VALUE_CHARS {
            return Err(metadata_value_too_long());
        }
        merged.insert(key.clone(), Value::String(value.clone()));
    }

    if merged.len() > METADATA_MAX_KEYS {
        return Err(metadata_too_many_keys());
    }
    Ok(merged)
}

// ------------------------------------------------------------------- list

/// `GET /v1/customers`'s query parameters — text for the same reason
/// [`CreateParams`]' fields are.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
}

/// `GET /v1/customers`.
///
/// **No filters.** Stripe's list takes `email`, and this one deliberately does
/// not: a filter on a payer identifier turns the list into a lookup, and a
/// lookup by email over a table that holds one merchant's payers is one
/// scoping mistake away from being a lookup over everybody's. It is a stated
/// gap (`docs/flows/customers.md`) rather than an oversight, and the shape
/// that would make it safe — a filter checked in the same `WHERE` as
/// `merchant_id`, as `checkout_sessions`' `payment_intent` filter is — is
/// available whenever it is wanted.
pub(crate) async fn list(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        CURSOR,
    )?;

    let page = CustomerListPage {
        limit: page.limit,
        starting_after: page.starting_after,
        ending_before: page.ending_before,
    };

    let (rows, has_more) =
        Customers::list_page(repositories.as_ref(), scope.merchant_id(), &page).await?;
    let data = rows
        .iter()
        .map(CustomerObject::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    json_response(StatusCode::OK, &ListObject::new(data, has_more, LIST_URL))
}

// ----------------------------------------------------------------- delete

/// `DELETE /v1/customers/{id}` → `{"id": …, "object": "customer",
/// "deleted": true}`.
///
/// # Why this carries an `Idempotency-Key` when Stripe's does not
///
/// Every write under `/v1` does (`docs/flows/merchant-auth.md`), and a
/// `DELETE` is the write where a replay is *most* confusing without one.
/// Since migration `0041` a second `DELETE` is idempotent on its own — the
/// customer is already erased, and this answers the same `{deleted: true}`
/// without writing anything or emitting a second event — but only for the
/// branch where a row survives to say so. A hard-deleted customer's second
/// `DELETE` is still a `404` without the key, and the key is what turns it
/// back into the stored answer.
///
/// # There is one refusal now, not two, and the `409` is gone
///
/// Until 2026-09-10 a customer an intent, a session or an invoice referenced
/// was refused with a `409` whose message advised clearing `name`, `email`
/// and `phone` instead — advice `at_least_one_identifier` refuses, so it
/// could not be followed. It has been replaced rather than reworded:
/// `DELETE` now **anonymises** such a customer (issues #68, #96 item 2).
/// The foreign keys are still `NO ACTION` and the payment record still
/// survives; what does not survive is the payer on it. So this route answers
/// the uniform `404` or it succeeds, and `ApiError::Conflict` is no longer
/// reachable from here.
///
/// # What a merchant sees afterwards, and why the two branches differ
///
/// A customer with **no** payment history is hard-deleted and a later `GET`
/// is the same `404` an id that never existed gets. One **with** history is
/// anonymised: the row stays, every identifier on it is `[redacted]`,
/// `metadata` is untouched because it is the merchant's own data, and a later
/// `GET` answers `200` with `deleted: true`. That asymmetry is Stripe's and
/// it is the one that keeps a merchant's stored `cus_…` resolving to
/// something for the customers vpay took money from.
///
/// The response body is the same either way, and it is deliberately **not**
/// the customer: a merchant confirming an erasure is the caller most likely
/// to log the whole response.
pub(crate) async fn delete(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = delete_once(repositories.as_ref(), &scope, &id).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Customer { id: &id },
    )
    .await
}

/// The three endings [`delete_once`]'s transaction has.
///
/// Two of them answer identically and are still two, because only one of them
/// wrote anything: naming them apart is what stops a future edit emitting a
/// second `customer.deleted` for a customer that was already erased.
enum DeleteOutcome {
    /// No such customer for this merchant.
    NotFound,
    /// Already anonymised. Answered `{deleted: true}`, nothing written, and —
    /// the part that matters — **no second event**.
    AlreadyErased,
    /// Hard-deleted or anonymised, with one `customer.deleted` in the same
    /// transaction.
    Erased,
}

/// The erasure itself: one transaction that takes the row's lock, decides the
/// `404`, renders what the merchant will be told, and hands the writes to
/// `vpay-db`.
///
/// # Why the body is rendered here and before the write
///
/// `customer.deleted` carries the **redacted** object, and the branch this
/// cannot see from here — hard delete or anonymise — decides whether there is
/// a row left to render it from afterwards. `CustomerRow::redacted` is a pure
/// projection of the row this transaction already holds, so one call serves
/// both branches and the two bodies are identical in shape. A merchant cannot
/// tell from the webhook which branch ran, and there is no reason they
/// should.
///
/// # Why the merchant is told the ids and not the payer
///
/// The body carries `id`, `object`, `created`, `livemode`, the merchant's own
/// `metadata` and `deleted: true`; every identifier in it is `[redacted]`.
/// That is a deliberate reversal of what this event used to carry, and the
/// argument that changed is written down in `docs/flows/customers.md`: the
/// merchant already received the payer's details in `customer.created` and in
/// every `customer.updated`, and holds their own copy — but the copy vpay
/// stores in `events` is vpay's, is never pruned, and is the one the
/// retention promise is about. `vpay_db::customers::erase_in_tx` redacts the
/// stored bodies of those earlier events in the same transaction, including
/// this one.
async fn delete_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    let outcome: TxOutcome<DeleteOutcome> = repositories
        .transaction(|tx| {
            Box::pin(async move {
                // Scoped and locked for `update_once`'s reasons: a foreign
                // `cus_…` is indistinguishable from a missing one, and
                // nothing may change the row between the branch decision and
                // the write that acts on it.
                let Some(row) = tx.lock_customer_for_update(scope.merchant_id(), id).await? else {
                    return Ok::<_, ApiError>(TxOutcome::Abandon(DeleteOutcome::NotFound));
                };

                if row.anonymized_at.is_some() {
                    // Idempotent by the object's own state rather than by the
                    // `Idempotency-Key`, which only covers a replay of *this*
                    // request. Abandoned, so no second `customer.deleted`
                    // describes an erasure that already happened.
                    return Ok(TxOutcome::Abandon(DeleteOutcome::AlreadyErased));
                }

                let now = OffsetDateTime::now_utc();
                let object = CustomerObject::try_from(&row.redacted(now))?;
                let data =
                    serde_json::to_value(&object).map_err(ApiError::internal_serialization)?;

                tx.erase_customer_in_tx(&row, now, &ids::event_id(), &data)
                    .await?;

                Ok(TxOutcome::Commit(DeleteOutcome::Erased))
            })
        })
        .await?;

    match outcome.into_inner() {
        DeleteOutcome::NotFound => Err(not_found(id)),
        DeleteOutcome::AlreadyErased | DeleteOutcome::Erased => json_response(
            StatusCode::OK,
            &DeletedObject {
                id: id.to_owned(),
                object: CustomerTag,
                deleted: DeletedTrue,
            },
        ),
    }
}

// --------------------------------------------------------------- plumbing

/// Blank is absent, on create and in a value position everywhere else:
/// `name=` is what a client templating an optional field emits when it has
/// none. On **update** the blank is meaningful — see [`UpdateParams`] — and
/// the distinction is drawn one level up, by whether the key was present at
/// all, before this function is reached.
fn present(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
}

/// One bounded text field, or a `400` naming it.
fn checked_text(
    value: Option<String>,
    param: &'static str,
    max_chars: usize,
) -> Result<Option<String>, ApiError> {
    // Characters, not bytes: the column's CHECK is `char_length`, and
    // counting bytes here would refuse a legal name whose letters are not
    // ASCII — which, for a Cameroonian payer, is most of them.
    if let Some(value) = value.as_deref()
        && value.chars().count() > max_chars
    {
        return Err(ApiError::invalid_param(
            param,
            format!("`{param}` must be at most {max_chars} characters."),
        ));
    }
    Ok(value)
}

/// The phone number, canonicalised, or a `400` naming `phone`.
///
/// Through [`super::account_holders::canonical_msisdn`] rather than a rule of
/// this module's own: see that function's own doc for why the two surfaces
/// must agree, and `docs/flows/customers.md` for why storing the canonical
/// form is a wire contract rather than an implementation detail.
fn checked_phone(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    super::account_holders::canonical_msisdn(&raw)
        .map(Some)
        .ok_or_else(|| {
            ApiError::invalid_param(
                "phone",
                "`phone` must be a Cameroon mobile number — `237` followed by `6` and eight \
                 digits, with or without a leading `+` and with or without separators.",
            )
        })
}

/// The address, bounded and shaped, or a `400` naming `address`.
///
/// `Ok(None)` means the request did not mention it. `Ok(Some(address))` means
/// "this is the address now" — and an empty one is how the wire spells
/// *clear it*, which is the same value `address=` produces. There is
/// deliberately no third state: see [`vpay_db::CustomerPatch::address`].
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `address` for a non-blank scalar, an
/// over-long component, a `country` that is not two letters, a coordinate
/// that is not a whole number or is out of range, or half a coordinate.
fn validated_address(param: Option<AddressParam>) -> Result<Option<CustomerAddress>, ApiError> {
    let components = match param {
        None => return Ok(None),
        // `address=` — every component absent, which is the clear.
        Some(AddressParam::Cleared(raw)) if raw.trim().is_empty() => {
            return Ok(Some(CustomerAddress::default()));
        }
        // `address=Douala` is a merchant reaching for `address[city]`, and
        // silently dropping it would store no address while answering `200`.
        Some(AddressParam::Cleared(_)) => {
            return Err(ApiError::invalid_param(
                "address",
                "`address` is an object: send its components as `address[line1]`, \
                 `address[city]`, `address[country]` and so on. Send `address=` with no \
                 value to remove the address entirely.",
            ));
        }
        Some(AddressParam::Components(components)) => components,
    };

    let address = CustomerAddress {
        line1: checked_text(present(components.line1), "address", ADDRESS_MAX_CHARS)?,
        line2: checked_text(present(components.line2), "address", ADDRESS_MAX_CHARS)?,
        city: checked_text(present(components.city), "address", ADDRESS_MAX_CHARS)?,
        state: checked_text(present(components.state), "address", ADDRESS_MAX_CHARS)?,
        postal_code: checked_text(
            present(components.postal_code),
            "address",
            POSTAL_CODE_MAX_CHARS,
        )?,
        country: checked_country(present(components.country))?,
        latitude_microdeg: checked_microdeg(
            present(components.latitude_microdeg),
            "latitude_microdeg",
            LATITUDE_MAX_MICRODEG,
        )?,
        longitude_microdeg: checked_microdeg(
            present(components.longitude_microdeg),
            "longitude_microdeg",
            LONGITUDE_MAX_MICRODEG,
        )?,
    };

    // The pair rule, decided here rather than left to
    // `address_coordinates_are_both_or_neither` for `checked_text`'s reason:
    // the CHECK is a `23514`, `classify_write` routes a CHECK violation to
    // `Category::Storage`, and a merchant who sent half a coordinate would be
    // told to wait for a database that is fine. It is asked of the value
    // `vpay-db` itself owns the rule for, so the two cannot disagree about
    // what `paired` means.
    if !address.coordinate_is_paired() {
        return Err(ApiError::invalid_param(
            "address",
            "`address[latitude_microdeg]` and `address[longitude_microdeg]` are sent \
             together or not at all. Half a coordinate names no place: a latitude on its \
             own is a line right round the planet, and storing one is worse than storing \
             nothing, because whoever completes it later produces a plausible wrong place.",
        ));
    }

    Ok(Some(address))
}

/// One coordinate, in whole microdegrees, or a `400` naming `address`.
///
/// # Why the unit is in the parameter name and the value is an integer
///
/// vpay stores a coordinate as a whole count of millionths of a degree and
/// never as a float — `vpay_db::CustomerAddress::latitude_microdeg` carries
/// the two measurements that decide it. This function is where that contract
/// is enforced against a merchant rather than against the database: `4.061`
/// is refused with a sentence that says what to send instead, because a
/// merchant who sent it and got a `200` would have had their payer's position
/// silently truncated or rounded by some layer.
///
/// # What the message does not contain, deliberately
///
/// **Not the value the merchant sent.** Every other refusal on this resource
/// names the parameter and states the rule without echoing the input, and
/// here there is a second reason on top of consistency: this field is a
/// payer's position, error bodies are the part of a response an integration
/// is most likely to log, and echoing it back would write a payer's
/// coordinates into a merchant's logs *because* they were malformed.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `address` — the top-level parameter a
/// merchant's error handler can act on, exactly as [`checked_country`]'s
/// does — for a value that is not a whole number or is outside the range of
/// the axis.
fn checked_microdeg(
    value: Option<String>,
    component: &'static str,
    max_microdeg: i64,
) -> Result<Option<i64>, ApiError> {
    let Some(raw) = value else {
        return Ok(None);
    };

    let Ok(microdeg) = raw.parse::<i64>() else {
        return Err(ApiError::invalid_param(
            "address",
            format!(
                "`address[{component}]` is a whole number of microdegrees — millionths \
                 of a degree — so 4.061 degrees is sent as `4061000` and -3.75 as \
                 `-3750000`. vpay stores no floating-point coordinate: a decimal \
                 point, an exponent or a unit suffix is refused rather than rounded."
            ),
        ));
    };

    // `-max..=max`, symmetric, because both axes are: the bound is the
    // definition of the unit and not a product limit. `-90000000` is the
    // South Pole and is as legal as the North.
    if microdeg < -max_microdeg || microdeg > max_microdeg {
        return Err(ApiError::invalid_param(
            "address",
            format!(
                "`address[{component}]` must be between -{max_microdeg} and \
                 {max_microdeg} — the whole range of the axis, in microdegrees. A \
                 value outside it is not a place."
            ),
        ));
    }

    Ok(Some(microdeg))
}

/// The country code, upper-cased and shape-checked, or a `400` naming
/// `address`.
///
/// # Why the shape and not a list of the 249 assigned codes
///
/// The list changes — South Sudan was assigned in 2011 and the Netherlands
/// Antilles withdrawn in 2010 — and nothing in vpay resolves a country code
/// to anything: it is stored, echoed back, and read by a human or by the
/// merchant's own address book. A list here would refuse a merchant's
/// perfectly real address until somebody shipped a release, which is the
/// trade `checked_phone` makes in the opposite direction and for the opposite
/// reason (a rail *does* resolve an MSISDN).
///
/// The value is **upper-cased** rather than refused for being lower case, and
/// that is the same wire contract `phone`'s canonicalisation is: `cm` and
/// `CM` are one country, so vpay must not hold two spellings of it. What is
/// refused is anything that is not two letters — `CMR`, `237`, `Cameroon`.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `address`.
fn checked_country(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let upper = raw.to_uppercase();
    if upper.len() == 2 && upper.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Ok(Some(upper));
    }
    Err(ApiError::invalid_param(
        "address",
        "`address[country]` must be an ISO 3166-1 alpha-2 country code — two letters, such \
         as `CM` or `FR`. Lower case is accepted and stored upper case; a three-letter code \
         or a country name is not.",
    ))
}

/// The refusal an update to — or an attachment of — an erased customer gets.
///
/// One function and two call sites, so a merchant cannot tell from the
/// message which route refused: both are the same fact about the object.
///
/// It is a `409` rather than a `404` because the object is still readable —
/// `GET /v1/customers/{id}` answers it with `deleted: true` — and a `404`
/// here would have two routes disagreeing about whether a `cus_…` exists.
/// It is not the `409` that went away in the same change: that one was about
/// a delete this API now performs, and this one is about a write to a record
/// there is deliberately nothing left in.
fn erased_customer() -> ApiError {
    ApiError::Conflict {
        message: "This Customer has been deleted: its identifiers are erased and it cannot be \
                  changed or attached to new payments. It is still readable, and answers \
                  `deleted: true`, so your own records keep resolving. Create a new Customer \
                  for a new payer."
            .to_owned(),
    }
}

/// The refusal a customer with no identifier gets, on create and on update.
///
/// One function, two call sites, and the message names **all three**
/// parameters rather than one: which of them the merchant meant to send is
/// their choice, and Stripe's `param` is a pointer to where to look rather
/// than a claim that this exact field is required.
fn at_least_one_identifier() -> ApiError {
    ApiError::invalid_param(
        "name",
        "A customer must have at least one of `name`, `email` or `phone`. A customer with \
         none of them names nobody: it can never be matched to a payer and never recognised \
         by you. `phone` alone is enough.",
    )
}

/// The three `metadata` refusals, one function each so the message a merchant
/// gets is identical to `/v1/payment_intents`' — a merchant reading it should
/// not be able to tell which resource refused.
fn metadata_too_many_keys() -> ApiError {
    ApiError::invalid_param("metadata", "`metadata` accepts at most 50 keys.")
}

/// See [`metadata_too_many_keys`].
fn metadata_key_too_long() -> ApiError {
    ApiError::invalid_param("metadata", "A `metadata` key is longer than 40 characters.")
}

/// See [`metadata_too_many_keys`].
fn metadata_value_too_long() -> ApiError {
    ApiError::invalid_param(
        "metadata",
        "A `metadata` value is longer than 500 characters.",
    )
}

/// The create path's `metadata`, bounded. The update path merges first and
/// bounds the result — see [`merged_metadata`].
fn validated_metadata(metadata: &BTreeMap<String, String>) -> Result<Map<String, Value>, ApiError> {
    if metadata.len() > METADATA_MAX_KEYS {
        return Err(metadata_too_many_keys());
    }
    for (key, value) in metadata {
        if key.chars().count() > METADATA_MAX_KEY_CHARS {
            return Err(metadata_key_too_long());
        }
        if value.chars().count() > METADATA_MAX_VALUE_CHARS {
            return Err(metadata_value_too_long());
        }
    }
    Ok(crate::model::metadata_from_pairs(metadata))
}

/// The 404 for a customer, built through one function so its call sites
/// cannot drift.
pub(crate) fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// Renders a stored customer.
///
/// # Errors
///
/// [`ApiError::Internal`] if the object will not render — see
/// [`CustomerObject`]'s `TryFrom`.
fn customer_response(status: StatusCode, row: &CustomerRow) -> Result<Response, ApiError> {
    json_response(status, &CustomerObject::try_from(row)?)
}

/// Resolves a merchant-supplied `customer=cus_…` on **another** resource's
/// create, and stamps the customer's retention clock.
///
/// # Why this lives here and not in `payment_intents`
///
/// Three call sites want it — an intent's create, a session's create, and the
/// confirm path — and every one of them has to make the same three decisions:
/// refuse a malformed id with a `400` naming `customer`, refuse a customer
/// that is not this merchant's with the *same* `400` (never a 404 and never a
/// different sentence, or the parameter becomes an oracle for which `cus_…`
/// exist under some other tenant), and record the use.
///
/// # The retention stamp is not optional, and a failure to make it is
///
/// `touch_last_used` is what keeps a customer alive: a customer the merchant
/// uses on every order but which nothing stamps is deleted by the twelve-month
/// sweep, with a `customer.deleted` event that is simply wrong. So every path
/// that resolves a customer stamps it.
///
/// A **failure** to stamp is logged and swallowed, deliberately. The stamp is
/// housekeeping and the request is a payment: failing a merchant's
/// `POST /v1/payment_intents` because a retention clock could not be moved
/// would trade a real payment for a bookkeeping row. The exposure that buys is
/// bounded and slow — the customer stays at its previous `last_used_at`, and
/// the *next* use of it stamps again — so a persistent failure costs at worst
/// a customer swept twelve months after its last successful stamp rather than
/// its last actual use.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `customer` for a malformed id or one
/// that is not this merchant's, or a storage error from the read.
pub(crate) async fn resolve_for_attachment(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    raw: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(customer) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    // The *shape*, before the lookup, for `paging::validated_cursor`'s
    // reason: a `cs_…` or a truncated paste is a typo the merchant can fix
    // from the message, and letting it reach the database would answer the
    // deliberately opaque "no such customer" instead.
    if !ids::is_well_formed(ids::CUSTOMER_PREFIX, customer) {
        return Err(ApiError::invalid_param(
            "customer",
            "`customer` must be a Customer id — `cus_` followed by 24 characters.",
        ));
    }

    // `get_for_merchant` and not an unscoped read: the tenant is a parameter
    // of the lookup, so a foreign customer is indistinguishable from a
    // missing one and this function has no way to compare tenants in Rust
    // and get it wrong.
    let row = Customers::get_for_merchant(repositories, scope.merchant_id(), customer)
        .await?
        .ok_or_else(unknown_customer)?;

    // An erased customer is not attachable. Taking a payment "from" a payer
    // whose identifiers vpay deleted would put a `NO ACTION` reference on a
    // record that exists only because the last such reference could not be
    // removed — and it would restart the twelve-month retention clock on it.
    // Refused with the state refusal rather than with `unknown_customer`,
    // because this id *is* this merchant's and answering "no such Customer"
    // would send them looking for a typo.
    if row.anonymized_at.is_some() {
        return Err(erased_customer());
    }

    touch(repositories, &row.id).await;
    Ok(Some(row.id))
}

/// Stamps a customer's retention clock, logging and swallowing a failure.
///
/// See [`resolve_for_attachment`] for why a failure here must not fail the
/// request that caused it. `pub(crate)` because the confirm path stamps a
/// customer it resolved from the *intent's* own column rather than from a
/// request field, and therefore has no id to validate.
pub(crate) async fn touch(repositories: &dyn Repositories, customer_id: &str) {
    match Customers::touch_last_used(repositories, customer_id, OffsetDateTime::now_utc()).await {
        // `false` is normal: the customer may have been deleted between the
        // read and this call, or already stamped at or after this instant by
        // a concurrent request.
        Ok(_) => {}
        Err(error) => tracing::warn!(
            customer_id,
            %error,
            "a customer's retention clock could not be stamped; the customer keeps its \
             previous last_used_at and the next use of it stamps again"
        ),
    }
}

/// The refusal for a `customer` this merchant cannot use.
///
/// **One function, two causes.** An id that names no customer and an id that
/// names another merchant's answer identically, for
/// `checkout_sessions::unknown_intent`'s reason applied to this parameter: a
/// distinct answer would make every create an oracle for which `cus_…` exist
/// under some other tenant.
fn unknown_customer() -> ApiError {
    ApiError::invalid_param(
        "customer",
        "No such Customer for this account. Create one with `POST /v1/customers` first.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(metadata: Value) -> CustomerRow {
        CustomerRow {
            id: "cus_0123456789abcdefghjkmnpq".to_owned(),
            seq: 1,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            name: Some("Ada Ngo".to_owned()),
            email: None,
            phone: Some("237600000200".to_owned()),
            address: CustomerAddress::default(),
            metadata,
            anonymized_at: None,
            last_used_at: OffsetDateTime::UNIX_EPOCH,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn param_of(error: &ApiError) -> Option<&str> {
        match error {
            ApiError::InvalidParam { param, .. } => Some(param),
            _ => None,
        }
    }

    fn sent(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// The maintainer's decision of 2026-09-05, at the layer that could
    /// silently reverse it: a phone number **alone** is a complete customer.
    ///
    /// The mutation this pins is one line — requiring `email` beside `phone`
    /// in [`validate_create`] — and it would refuse the payer population vpay
    /// exists for while every other test stayed green.
    #[test]
    fn a_phone_number_alone_is_a_complete_customer() {
        let params = UpdateParams {
            name: None,
            email: None,
            phone: None,
            address: None,
            metadata: None,
        };
        // The update path's version of the same rule: a customer whose only
        // identifier is a phone number is not refused for lacking the other
        // two.
        let patch = validate_update(params, &row(json_object()))
            .expect("a customer with only a phone number is complete");
        assert!(patch.is_empty());
    }

    fn json_object() -> Value {
        serde_json::json!({})
    }

    /// Clearing the **last** identifier is a `400` naming the parameters, not
    /// the `500` the database's own CHECK would produce.
    ///
    /// `at_least_one_identifier` is a `23514`, and `classify_write` routes a
    /// CHECK violation to `Category::Storage` — a `503` telling a merchant to
    /// wait for a database that is fine. This is the case that has to be
    /// decided above the statement, and it is the one that needs the stored
    /// row to decide: the request only says `name=` and `phone=`.
    #[test]
    fn clearing_every_identifier_is_refused_by_the_api_and_not_by_the_check() {
        let stored = row(json_object());
        let params = UpdateParams {
            name: Some(String::new()),
            email: None,
            phone: Some(String::new()),
            address: None,
            metadata: None,
        };

        let error = validate_update(params, &stored)
            .expect_err("clearing the last identifier leaves a customer naming nobody");
        assert_eq!(param_of(&error), Some("name"));
        let rendered = format!("{error}");
        for field in ["name", "email", "phone"] {
            assert!(
                rendered.contains(field),
                "the message names all three, since which one to send is the merchant's \
                 choice: {rendered}"
            );
        }
    }

    /// Clearing one identifier while another survives is allowed — the rule
    /// is "at least one", not "never fewer".
    #[test]
    fn clearing_one_identifier_of_two_is_allowed() {
        let stored = row(json_object());
        let params = UpdateParams {
            name: Some(String::new()),
            email: None,
            phone: None,
            address: None,
            metadata: None,
        };

        let patch = validate_update(params, &stored).expect("the phone number survives");
        assert_eq!(patch.name, Some(None), "`name=` clears the name");
        assert_eq!(patch.phone, None, "`phone` was not mentioned");
    }

    /// A patch that *adds* an identifier while clearing another is allowed,
    /// which is the case a naive "count the stored ones" check gets wrong.
    #[test]
    fn a_patch_may_clear_one_identifier_while_adding_another() {
        let stored = CustomerRow {
            name: None,
            email: None,
            phone: Some("237600000200".to_owned()),
            ..row(json_object())
        };
        let params = UpdateParams {
            name: Some("Ada Ngo".to_owned()),
            email: None,
            phone: Some(String::new()),
            address: None,
            metadata: None,
        };

        let patch = validate_update(params, &stored)
            .expect("the name arriving in the same request is what keeps this customer named");
        assert_eq!(patch.name, Some(Some("Ada Ngo".to_owned())));
        assert_eq!(patch.phone, Some(None));
    }

    /// Metadata is merged key-wise and an empty value removes a key —
    /// Stripe's contract, and the half a "replace the map" implementation
    /// gets wrong silently.
    #[test]
    fn metadata_is_merged_and_an_empty_value_removes_a_key() {
        let stored = serde_json::json!({ "order_id": "1234", "tier": "gold" });

        let merged = merged_metadata(&stored, &sent(&[("tier", ""), ("region", "CM")]))
            .expect("within bounds");

        assert_eq!(
            merged.get("order_id").and_then(Value::as_str),
            Some("1234"),
            "a key the request did not mention survives the merge"
        );
        assert_eq!(merged.get("region").and_then(Value::as_str), Some("CM"));
        assert!(
            !merged.contains_key("tier"),
            "`metadata[tier]=` removes the key rather than storing an empty string or a \
             null, which would read back as a key the merchant thinks they removed"
        );
    }

    /// The key ceiling bounds the **merged** map, not the delta.
    ///
    /// Bounding the delta is the easy mistake and it is unbounded in effect:
    /// a merchant could add one key fifty times and end with a hundred.
    #[test]
    fn the_metadata_ceiling_bounds_the_result_and_not_the_request() {
        let mut full = Map::new();
        for index in 0..METADATA_MAX_KEYS {
            full.insert(format!("k{index}"), Value::String("v".to_owned()));
        }
        let stored = Value::Object(full);

        let error = merged_metadata(&stored, &sent(&[("one_more", "v")]))
            .expect_err("fifty stored keys plus one is fifty-one");
        assert_eq!(param_of(&error), Some("metadata"));

        // And replacing an existing key is still fine at the ceiling, which
        // is what makes this a bound on the result rather than on the count.
        assert_eq!(
            merged_metadata(&stored, &sent(&[("k0", "changed")]))
                .expect("replacing a key does not grow the map")
                .len(),
            METADATA_MAX_KEYS
        );
    }

    /// The bounds are the ones every other resource uses, asserted rather
    /// than assumed — see [`METADATA_MAX_KEYS`] for why they are restated
    /// here at all.
    ///
    /// The numbers are `docs/api/README.md`'s and Stripe's, and a merchant
    /// must not be able to tell which resource refused them.
    #[test]
    fn the_metadata_bounds_are_the_ones_every_other_resource_uses() {
        assert_eq!(
            (
                METADATA_MAX_KEYS,
                METADATA_MAX_KEY_CHARS,
                METADATA_MAX_VALUE_CHARS
            ),
            (50, 40, 500)
        );

        // The messages, byte for byte, against `payment_intents`' own — the
        // property that actually reaches a merchant.
        assert!(format!("{}", metadata_too_many_keys()).contains("at most 50 keys"));
        assert!(format!("{}", metadata_key_too_long()).contains("longer than 40 characters"));
        assert!(format!("{}", metadata_value_too_long()).contains("longer than 500 characters"));
    }

    /// A phone number is stored canonical, whatever the merchant typed.
    ///
    /// The three spellings are the ones `account_holders` accepts, and the
    /// point is that all three land on one string: a customer created from a
    /// `+237 …` and an account-holder lookup for `600000200` are the same
    /// payer, and vpay must not hold two spellings of them.
    #[test]
    fn a_phone_number_is_stored_in_the_form_a_rail_is_given() {
        for typed in [
            "+237 6 00 00 02 00",
            "237600000200",
            "600000200",
            "6-00-00-02-00",
        ] {
            assert_eq!(
                checked_phone(Some(typed.to_owned())).expect("a Cameroon mobile number"),
                Some("237600000200".to_owned()),
                "`{typed}` must canonicalise like every other MSISDN this API takes"
            );
        }

        let error = checked_phone(Some("not-a-phone".to_owned()))
            .expect_err("a value that is not a phone number");
        assert_eq!(param_of(&error), Some("phone"));
    }

    /// The three things the wire can say about `address`, told apart at the
    /// **decoder**, from the JSON the form parser really produces.
    ///
    /// This is the assertion the untagged enum exists for, and it is written
    /// against `serde_json::from_value` rather than against `AddressParam`
    /// directly because the failure it guards is a *decoding* one: swap the
    /// variant order in [`AddressParam`] and `address=` starts decoding as an
    /// `AddressParams` with every field absent, which is byte-identical to
    /// "the request did not mention address". A merchant's `address=` would
    /// then answer `200` and change nothing, for ever, and every other test
    /// here would stay green.
    #[test]
    fn the_wire_tells_leave_the_address_alone_set_it_and_clear_it_apart() {
        let decode = |body: Value| {
            serde_json::from_value::<UpdateParams>(body).expect("the form decoder's own shape")
        };

        assert!(
            decode(serde_json::json!({ "name": "Ada" }))
                .address
                .is_none(),
            "an absent key leaves the address alone"
        );

        let cleared = decode(serde_json::json!({ "address": "" }));
        assert_eq!(
            validated_address(cleared.address).expect("`address=` is the clear"),
            Some(CustomerAddress::default()),
            "`address=` clears every component; decoding it as `None` would make an address \
             unremovable through the API that documents how to remove it"
        );

        let set = decode(serde_json::json!({
            "address": { "line1": "12 Rue Njo-Njo", "city": "Douala", "country": "cm" }
        }));
        assert_eq!(
            validated_address(set.address).expect("components set the address"),
            Some(CustomerAddress {
                line1: Some("12 Rue Njo-Njo".to_owned()),
                city: Some("Douala".to_owned()),
                country: Some("CM".to_owned()),
                ..CustomerAddress::default()
            }),
            "the components the request did not name are absent — the address is replaced \
             whole, never merged component-wise"
        );
    }

    /// `address=Douala` is a merchant reaching for `address[city]`, and is
    /// refused rather than silently storing no address.
    #[test]
    fn a_scalar_address_is_refused_and_not_read_as_a_clear() {
        let error = validated_address(Some(AddressParam::Cleared("Douala".to_owned())))
            .expect_err("a non-blank scalar names no component");
        assert_eq!(param_of(&error), Some("address"));
        assert!(
            format!("{error}").contains("address[line1]"),
            "the message has to show the merchant the shape it wanted"
        );
    }

    /// The country code is ISO 3166-1 alpha-2, upper-cased on the way in and
    /// refused when it is not two letters.
    ///
    /// The upper-casing is the same wire contract `phone`'s canonicalisation
    /// is: `cm` and `CM` are one country, and vpay must not hold two
    /// spellings of it. What this pins in the other direction is that a
    /// three-letter code, a dialling code and a country name are all refused
    /// naming `address` — the mutation being "delete `checked_country` and
    /// route `country` through `checked_text`", which compiles and stores
    /// `Cameroon`.
    #[test]
    fn the_country_is_two_letters_upper_cased_and_nothing_else() {
        for typed in ["CM", "cm", "cM"] {
            assert_eq!(
                checked_country(Some(typed.to_owned())).expect("an alpha-2 code"),
                Some("CM".to_owned()),
                "`{typed}` must store as `CM`"
            );
        }
        for refused in ["CMR", "237", "Cameroon", "C", "C1", "C-", "ÉM"] {
            let error = checked_country(Some(refused.to_owned()))
                .err()
                .unwrap_or_else(|| {
                    panic!("`{refused}` is not an ISO 3166-1 alpha-2 code and must be refused")
                });
            assert_eq!(
                param_of(&error),
                Some("address"),
                "the refusal points at the top-level parameter a merchant's error handler \
                 can act on"
            );
        }
    }

    /// An update to an **erased** customer is a `409`, on the route and on
    /// the attachment path both.
    ///
    /// It cannot be a `400` about a parameter and it cannot be a `404`: the
    /// object is readable, `GET` answers it with `deleted: true`, and the
    /// refusal is a fact about its state. What this pins is that the message
    /// says so — a merchant who reads it must not go looking for a typo in
    /// the id.
    #[test]
    fn a_write_to_an_erased_customer_is_a_conflict_that_says_why() {
        let error = erased_customer();
        assert!(
            matches!(error, ApiError::Conflict { .. }),
            "an erased customer is a state refusal, not a missing object"
        );
        let rendered = format!("{error}");
        assert!(rendered.contains("deleted"), "{rendered}");
        assert!(
            rendered.contains("Create a new Customer"),
            "the merchant needs to be told what to do instead: {rendered}"
        );
    }

    /// An address is bounded per component, and the refusal names `address`
    /// rather than the component.
    ///
    /// `param` is the top-level parameter Stripe's error shape carries, and
    /// pointing at `address` is what a merchant's error handler can act on;
    /// the sentence names the bound. The bounds themselves are migration
    /// `0041`'s CHECKs, and this is the `400` that keeps one from arriving as
    /// a `500`.
    #[test]
    fn an_over_long_address_component_is_a_400_naming_address() {
        let long = "é".repeat(ADDRESS_MAX_CHARS + 1);
        let error = validated_address(Some(AddressParam::Components(Box::new(AddressParams {
            line1: Some(long),
            line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            latitude_microdeg: None,
            longitude_microdeg: None,
        }))))
        .expect_err("over the component bound");
        assert_eq!(param_of(&error), Some("address"));

        // Characters and not bytes, for `checked_text`'s reason: 256
        // non-ASCII characters is 512 bytes and is a legal street address in
        // the market this API exists for.
        let at_bound = "é".repeat(ADDRESS_MAX_CHARS);
        assert!(
            validated_address(Some(AddressParam::Components(Box::new(AddressParams {
                line1: Some(at_bound),
                line2: None,
                city: None,
                state: None,
                postal_code: None,
                country: None,
                latitude_microdeg: None,
                longitude_microdeg: None,
            }))))
            .is_ok()
        );
    }

    /// A coordinate is a **whole number of microdegrees**, and a decimal
    /// degree is refused with a sentence that says how to spell it.
    ///
    /// This is the assertion the field's *name* exists for, made at the one
    /// place a merchant's `4.061` can still be turned into something. Every
    /// layer below is integer-typed, so if this accepted a float it would
    /// have to round it — and a payer's position rounded by an API that did
    /// not say so is the failure this whole shape is chosen to avoid.
    ///
    /// The refused list is the spellings a merchant actually reaches for:
    /// degrees with a point, degrees in exponent form, a unit suffix, a
    /// thousands separator, and a bare sign.
    #[test]
    fn a_coordinate_is_a_whole_number_of_microdegrees_and_never_a_degree() {
        for typed in ["4061000", "+4061000", "-3750000", "0", "90000000"] {
            assert_eq!(
                checked_microdeg(
                    Some(typed.to_owned()),
                    "latitude_microdeg",
                    LATITUDE_MAX_MICRODEG
                )
                .unwrap_or_else(|error| panic!("`{typed}` is a whole number: {error}")),
                Some(
                    typed
                        .trim_start_matches('+')
                        .parse::<i64>()
                        .expect("parses")
                ),
                "`{typed}` must be stored as the integer it is"
            );
        }

        let mut messages = std::collections::BTreeSet::new();
        for refused in [
            "4.061",
            "4061000.0",
            "4.061e6",
            "4061000m",
            "4_061_000",
            "-",
            "north",
        ] {
            let error = checked_microdeg(
                Some(refused.to_owned()),
                "latitude_microdeg",
                LATITUDE_MAX_MICRODEG,
            )
            .err()
            .unwrap_or_else(|| {
                panic!(
                    "`{refused}` is not a whole number of microdegrees and must be refused \
                     rather than rounded"
                )
            });
            assert_eq!(
                param_of(&error),
                Some("address"),
                "the refusal points at the top-level parameter a merchant's error handler \
                 can act on"
            );
            let rendered = format!("{error}");
            assert!(
                rendered.contains("latitude_microdeg") && rendered.contains("4061000"),
                "the message has to show the merchant how to spell 4.061: {rendered}"
            );
            messages.insert(rendered);
        }

        // ONE message for all seven, which is how "the refusal does not echo
        // the value" is asserted without a substring test that the message's
        // own example (`4.061`) would defeat. An error body is the part of a
        // response an integration is most likely to log, and this field is a
        // payer's position: a message that varied with the input would be
        // carrying it.
        assert_eq!(
            messages.len(),
            1,
            "the refusal varies with the value the merchant sent, which writes a payer's \
             position into their logs BECAUSE it was malformed: {messages:?}"
        );

        // A blank is not a refusal at all: `present` turns it into "absent"
        // one level up, which is what a client templating an optional field
        // emits. Asserted here because it is the one input that looks like it
        // belongs in the list above and does not.
        assert_eq!(
            checked_microdeg(
                present(Some("  ".to_owned())),
                "latitude_microdeg",
                LATITUDE_MAX_MICRODEG
            )
            .expect("a blank is absent, not malformed"),
            None
        );
    }

    /// Each axis is bounded by its own range, and the bound is symmetric.
    ///
    /// The South Pole is as legal as the North, and 180 degrees east is as
    /// legal as 180 west: the bound is the definition of the unit and not a
    /// product limit, so an off-by-one in the wrong direction would refuse a
    /// real place. What it refuses is one microdegree outside, on each axis
    /// and each sign — four cases, which is what catches a `max` used for
    /// both axes.
    #[test]
    fn each_axis_is_bounded_by_its_own_range_and_the_bound_is_symmetric() {
        for (component, max) in [
            ("latitude_microdeg", LATITUDE_MAX_MICRODEG),
            ("longitude_microdeg", LONGITUDE_MAX_MICRODEG),
        ] {
            for legal in [max, -max, 0] {
                assert_eq!(
                    checked_microdeg(Some(legal.to_string()), component, max)
                        .unwrap_or_else(|error| panic!("`{legal}` is on the axis: {error}")),
                    Some(legal)
                );
            }
            for over in [max + 1, -max - 1] {
                let error = checked_microdeg(Some(over.to_string()), component, max)
                    .expect_err("one microdegree outside the axis is not a place");
                assert_eq!(param_of(&error), Some("address"));
                assert!(
                    format!("{error}").contains(component),
                    "the message names the axis, because `which one and which bound` is \
                     the fact a merchant needs"
                );
            }
        }

        // And the two axes really are bounded differently: 100 degrees east
        // is a place and 100 degrees north is not. A single shared constant
        // would pass every assertion above and fail this one.
        assert!(
            checked_microdeg(
                Some("100000000".to_owned()),
                "longitude_microdeg",
                LONGITUDE_MAX_MICRODEG
            )
            .is_ok()
        );
        assert!(
            checked_microdeg(
                Some("100000000".to_owned()),
                "latitude_microdeg",
                LATITUDE_MAX_MICRODEG
            )
            .is_err()
        );
    }

    /// Half a coordinate is a `400` naming `address`, decided here and not by
    /// the database.
    ///
    /// `address_coordinates_are_both_or_neither` is the backstop and it is a
    /// `23514`; `classify_write` routes a CHECK violation to
    /// `Category::Storage`, which is a `503` telling a merchant to wait for a
    /// database that is fine. So this is one of the two rules on this
    /// resource that has to be decided above the statement — the other being
    /// "clearing the last identifier" — and for the same reason.
    ///
    /// Both directions, because the plausible mutation is a check written
    /// against one field.
    #[test]
    fn half_a_coordinate_is_refused_by_the_api_and_not_by_the_check() {
        for (latitude, longitude) in [
            (Some("4061000".to_owned()), None),
            (None, Some("9786000".to_owned())),
        ] {
            let error =
                validated_address(Some(AddressParam::Components(Box::new(AddressParams {
                    line1: None,
                    line2: None,
                    city: None,
                    state: None,
                    postal_code: None,
                    country: None,
                    latitude_microdeg: latitude,
                    longitude_microdeg: longitude,
                }))))
                .expect_err("half a coordinate names no place");
            assert_eq!(param_of(&error), Some("address"));
            let rendered = format!("{error}");
            assert!(
                rendered.contains("latitude_microdeg") && rendered.contains("longitude_microdeg"),
                "the message names both halves, since which one the merchant meant to send \
                 is theirs to decide: {rendered}"
            );
        }

        // The whole pair is accepted, so this is a test of the pair rule and
        // not of the coordinate being refused outright.
        assert_eq!(
            validated_address(Some(AddressParam::Components(Box::new(AddressParams {
                line1: None,
                line2: None,
                city: None,
                state: None,
                postal_code: None,
                country: None,
                latitude_microdeg: Some("4061000".to_owned()),
                longitude_microdeg: Some("9786000".to_owned()),
            }))))
            .expect("a whole coordinate is an address"),
            Some(CustomerAddress {
                latitude_microdeg: Some(4_061_000),
                longitude_microdeg: Some(9_786_000),
                ..CustomerAddress::default()
            })
        );

        // And `address=` still clears both halves with the rest, which is the
        // one path through `validated_address` that does not build the struct
        // field by field and could therefore have been missed.
        assert_eq!(
            validated_address(Some(AddressParam::Cleared(String::new())))
                .expect("`address=` is the clear"),
            Some(CustomerAddress::default()),
            "`address=` clears the point as well as the street: the address is one fact"
        );
    }

    /// The two bounds this module refuses with are the two migration `0041`
    /// enforces, read off disk rather than restated.
    ///
    /// The pair is what keeps a `400` from becoming a `503`: if this module's
    /// ceiling were the higher of the two, a value between them would pass
    /// the boundary and trip `address_latitude_microdeg_range` as a `23514`,
    /// which `classify_write` routes to `Category::Storage` — a merchant told
    /// to wait for a database that is fine. If it were the lower, vpay would
    /// refuse a real place. `the_redaction_marker_is_the_one_the_migration_enforces`
    /// in `vpay-db` is the same device applied to the marker.
    #[test]
    fn the_coordinate_bounds_are_the_ones_the_migration_enforces() {
        let migration =
            include_str!("../../../../migrations/0041_customers-address-and-anonymisation.sql");

        for (constant, name) in [
            (LATITUDE_MAX_MICRODEG, "address_latitude_microdeg_range"),
            (LONGITUDE_MAX_MICRODEG, "address_longitude_microdeg_range"),
        ] {
            let clause = migration
                .split_once(name)
                .and_then(|(_, rest)| rest.split_once(");"))
                .map(|(clause, _)| clause.to_owned())
                .unwrap_or_else(|| panic!("`{name}` is not in migration 0041"));
            assert!(
                clause.contains(&format!("BETWEEN -{constant} AND {constant}")),
                "`{name}` does not enforce the bound this module refuses with ({constant}); a \
                 value between the two would reach Postgres and come back as a 503. \
                 Clause: {clause}"
            );
        }
    }

    /// Blank is absent on create: `name=` from a client templating an
    /// optional field is not a name of length zero, which the column's
    /// `name_length` floor would refuse as a `500`.
    #[test]
    fn a_blank_field_is_absent_rather_than_an_empty_value() {
        assert_eq!(present(Some("   ".to_owned())), None);
        assert_eq!(present(Some(String::new())), None);
        assert_eq!(present(Some(" Ada ".to_owned())), Some("Ada".to_owned()));
    }

    /// The two text bounds are the column's, and they count **characters**.
    ///
    /// Bytes would refuse a legal name whose letters are not ASCII — which,
    /// for the payer population this API exists for, is most of them. The
    /// column's CHECK is `char_length`, so a byte count here would also
    /// disagree with the backstop it is supposed to front.
    #[test]
    fn the_text_bounds_count_characters_and_name_their_parameter() {
        let long_name = "é".repeat(NAME_MAX_CHARS);
        assert!(
            checked_text(Some(long_name.clone()), "name", NAME_MAX_CHARS).is_ok(),
            "256 non-ASCII characters is 512 bytes and is a legal name"
        );

        let over = "é".repeat(NAME_MAX_CHARS + 1);
        assert_eq!(
            param_of(&checked_text(Some(over), "name", NAME_MAX_CHARS).expect_err("over")),
            Some("name")
        );
        assert_eq!(
            param_of(
                &checked_text(
                    Some("a".repeat(EMAIL_MAX_CHARS + 1)),
                    "email",
                    EMAIL_MAX_CHARS
                )
                .expect_err("over")
            ),
            Some("email")
        );
    }
}
