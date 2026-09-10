//! `/v1/customers` — create, retrieve, update, list, delete.
//!
//! The merchant-owned record of a payer they expect to see again (S4a). It is
//! Stripe's `customer`, narrowed: `id`, `name`, `email`, `phone`, `metadata`,
//! `created`, `livemode`, and nothing else. `docs/flows/customers.md` is the
//! long version of everything below; this header is the short one.
//!
//! **Tenancy.** Every query takes the [`MerchantScope`] the authentication
//! middleware resolved. A merchant asking for another merchant's `cus_…` gets
//! the same 404, byte for byte, as one asking for an id that never existed —
//! and a merchant naming another merchant's `cus_…` in `customer` on an
//! intent or a session gets the same `400` as one naming an id that does not
//! exist.
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
//! 2. **`DELETE` is a hard delete.** No `deleted_at`, no status column, no
//!    tombstone. A row that says "this person asked to be forgotten" is still
//!    the record of that person.
//! 3. **A customer with payment history cannot be deleted.** The foreign keys
//!    migration `0034` adds are `NO ACTION`, so an intent or a session
//!    pinning the customer refuses both this route and the retention sweep.
//!    That is a deliberate trade and not a bug: the payment record survives,
//!    and "delete this customer" is therefore not a complete erasure of the
//!    payer. [`delete`] answers a `409` that says so.
//!
//! # What is *not* here
//!
//! The twelve-month retention sweep is `vpay_worker::handlers`' —
//! `sweep_idle_customers`, a job kind of its own. This module never deletes
//! on a timer and has no idea what the horizon is.

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
    CustomerListPage, CustomerPatch, CustomerRow, Customers, NewCustomer, Repositories, TxOutcome,
    UnitOfWork as _,
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
    /// Bracket-encoded (`metadata[order_id]=1234`) by
    /// [`crate::form::parse_form`], exactly as on an intent.
    #[serde(default)]
    metadata: BTreeMap<String, String>,
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
        metadata: Value::Object(validated.metadata),
        created_at: OffsetDateTime::now_utc(),
    };

    let outcome = create_with_event(repositories.as_ref(), &new)
        .await
        .and_then(|row| customer_response(StatusCode::CREATED, &row));

    post.finish(repositories.as_ref(), &scope, claim_id, outcome)
        .await
}

/// A create request that has passed every rule, in the shape the insert
/// needs.
#[derive(Debug)]
struct ValidCreate {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    metadata: Map<String, Value>,
}

/// Every rule that can be decided from the request alone.
async fn validate_create(post: &PostRequest) -> Result<ValidCreate, ApiError> {
    let params: CreateParams = post.form().await?;

    let name = checked_text(present(params.name), "name", NAME_MAX_CHARS)?;
    let email = checked_text(present(params.email), "email", EMAIL_MAX_CHARS)?;
    let phone = checked_phone(present(params.phone))?;

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
    post.finish(repositories.as_ref(), &scope, claim_id, outcome)
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
/// `DELETE` is the write where a replay is *most* confusing without one: the
/// second call would otherwise answer 404 for a deletion that succeeded, and
/// a merchant retrying a timed-out request cannot tell that from "somebody
/// else deleted it". With the key, the replay answers the stored
/// `{deleted: true}`.
///
/// # The two refusals, and why one of them is a `409`
///
/// * no such customer **for this merchant** → the uniform `404`;
/// * a customer an intent or a session references → `409`, because it is a
///   fact about the object's state rather than about the request's shape, and
///   because it may stop being true (the referencing objects are not
///   deletable either, so in practice it never does — which the message says
///   plainly rather than implying a retry will help).
///
/// The `409` is decided by the **foreign key**, not by a preceding `SELECT`.
/// A count-then-delete would let a concurrent `POST /v1/payment_intents`
/// commit a reference between the two, and the customer a merchant is about
/// to take a payment from would be erased.
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
    post.finish(repositories.as_ref(), &scope, claim_id, outcome)
        .await
}

/// The delete itself, and the mapping from the foreign key's refusal to the
/// merchant's `409`.
async fn delete_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    match Customers::delete(repositories, scope.merchant_id(), id).await {
        Ok(true) => json_response(
            StatusCode::OK,
            &DeletedObject {
                id: id.to_owned(),
                object: CustomerTag,
                deleted: DeletedTrue,
            },
        ),
        Ok(false) => Err(not_found(id)),
        Err(vpay_db::DbError::Persistence(vpay_db::PersistenceError::ForeignKey { .. })) => {
            Err(ApiError::Conflict {
                message: "This customer is referenced by a PaymentIntent or a Checkout \
                          Session and cannot be deleted. vpay keeps a payment attached to \
                          the payer it was taken from, so the payment record has to go \
                          first — and it never does. Clear the customer's `name`, `email` \
                          and `phone` instead if you need to remove the payer's details."
                    .to_owned(),
            })
        }
        Err(other) => Err(ApiError::from(other)),
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
            metadata,
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
