//! `GET /v1/events` and `GET /v1/events/{id}` — the merchant's read-only
//! view of what vpay has told them about.
//!
//! # Why this exists in the same step webhooks do
//!
//! Delivery is at-least-once and can also be at-*most*-zero: an endpoint
//! that refuses eight times in a row exhausts the ladder
//! (`vpay_worker::delivery_delay`) and the merchant is never told. This is
//! the surface they are pointed at when that happens, and
//! `docs/api/README.md` has documented it as the fallback since before it
//! existed. Shipping the deliverer without it would leave a merchant with a
//! missed webhook and nothing to poll.
//!
//! # One renderer, and why that is the point
//!
//! Both handlers below render through [`crate::model::EventObject`], which is
//! the same type `vpay_worker::webhooks::event_bytes` serialises to get the
//! bytes it signs. The fallback therefore answers exactly the question the
//! webhook asked; two renderers would let it answer a different one, and the
//! difference would surface as a merchant's signature check failing against
//! a body they re-fetched.
//!
//! # `?type=` is documented and deliberately not implemented
//!
//! `docs/api/README.md` lists a `type` filter. It is **not** here
//! (`docs/plans/2026-09-03-step5-webhooks.md`, decision 5): a filter
//! interacts with the cursor — `has_more` and the `seq` window both have to
//! be computed over the *filtered* set or paging silently skips rows — and
//! half of that is worse than none. Unknown query parameters are ignored, as
//! every other handler on this surface ignores them, so `?type=…` returns an
//! unfiltered page rather than a `400`. That is stated in `docs/status.md`
//! and in `docs/api/README.md` rather than left for a merchant to discover
//! by counting rows.
//!
//! # No writes, ever
//!
//! There is no `POST /v1/events` and there will not be one: an event is a
//! record of something vpay did, and a merchant who could create one could
//! forge their own history. Retrying a *delivery* is an operator action
//! against `webhook_deliveries` (`docs/runbooks/`), not a merchant one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use vpay_db::{PgPool, events};

use crate::error::ApiError;
use crate::form::VpayQuery;
use crate::model::{EventObject, ListObject};
use crate::v1::MerchantScope;
use crate::v1::paging::{self, CursorKind};

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a 404 for an event can never be spelled two ways.
const RESOURCE: &str = "event";

/// The list envelope's `url`, and the path a cursor page is read from.
const LIST_URL: &str = "/v1/events";

/// This resource's cursor vocabulary — `evt_…`, per `vpay_core::ids`.
///
/// `pub(crate)` so `paging`'s own tests can prove that this list refuses a
/// `pi_…` cursor and the intent list refuses an `evt_…` one, which is the
/// whole reason the prefix is a parameter rather than a constant in there.
pub(crate) const CURSOR: CursorKind = CursorKind {
    prefix: vpay_core::ids::EVENT_PREFIX,
    noun: "an event id",
};

/// `GET /v1/events`'s query parameters — text for the same reason
/// `payment_intents::ListParams`'s fields are: the wire carries strings, and
/// typing them here would hand "not a number" to serde, which answers with a
/// sentence about the request's shape instead of naming `limit`.
#[derive(Debug, Deserialize)]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
}

/// `GET /v1/events` — this merchant's events, newest first.
///
/// Scoped by [`MerchantScope`], which the authentication middleware puts on
/// the request and a handler cannot construct: `vpay_db::events::list_page`
/// takes the merchant id as a required argument and has no unscoped variant,
/// so an unfiltered read of every merchant's payment history does not
/// compile rather than merely being unwritten.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] for a malformed `limit` or cursor (see
/// [`paging`]); [`ApiError::Db`] if the read fails; [`ApiError::Internal`]
/// for a row that will not render, which migration 0018's `data_is_object`
/// CHECK makes unreachable.
pub(crate) async fn list(
    State(pool): State<PgPool>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        CURSOR,
    )?;

    let (rows, has_more) = events::list_page(&pool, scope.merchant_id(), &page).await?;
    let data = rows
        .iter()
        .map(EventObject::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    crate::v1::payment_intents::json_response(
        StatusCode::OK,
        &ListObject::new(data, has_more, LIST_URL),
    )
}

/// `GET /v1/events/{id}`.
///
/// A foreign merchant's event id is a `404`, byte for byte the same `404` a
/// nonexistent one gets — `vpay_db::events::get_by_id` folds both into
/// `None` on purpose. Telling them apart would let anyone holding one
/// credential enumerate which `evt_…` ids exist across the whole deployment,
/// which is the same reason `GET /v1/payment_intents/{id}` answers the way
/// it does.
///
/// The id is **not** shape-checked first: unlike a cursor, a malformed id
/// here produces a `404` naming the id the caller sent, which is already the
/// right answer and already says what is wrong. The cursor case is different
/// precisely because its wrong answer is a silent empty page.
///
/// # Errors
///
/// [`ApiError::NotFound`] for an id this merchant has no event under;
/// [`ApiError::Db`] if the read fails; [`ApiError::Internal`] for a row that
/// will not render.
pub(crate) async fn retrieve(
    State(pool): State<PgPool>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = events::get_by_id(&pool, scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| ApiError::NotFound {
            resource: RESOURCE,
            id: id.clone(),
        })?;

    crate::v1::payment_intents::json_response(StatusCode::OK, &EventObject::try_from(&row)?)
}

#[cfg(test)]
mod tests {
    use super::{CURSOR, LIST_URL, RESOURCE};

    /// The wire vocabulary this module is written against, pinned as
    /// literals: `resource` is what a `404` envelope's message names, and
    /// `url` is what a client pages against. A rename that only touched the
    /// constants would compile and change both without a test noticing.
    #[test]
    fn the_resource_names_are_the_ones_the_documented_envelope_uses() {
        assert_eq!(RESOURCE, "event");
        assert_eq!(LIST_URL, "/v1/events");
        // The same prefix `vpay_core::ids::event_id` mints and the database
        // stores — not a second spelling of it.
        assert_eq!(CURSOR.prefix, "evt_");
    }
}
