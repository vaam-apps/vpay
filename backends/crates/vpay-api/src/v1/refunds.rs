//! `GET /v1/refunds/{id}` — the merchant's authoritative read of one refund.
//!
//! # Why a read exists before a create does
//!
//! It is the only observation of a refund there is. `POST /v1/refunds` is
//! declared in `docs/flows/merchant-auth.md` and routed nowhere, because
//! creating a refund needs `ProviderAdapter::refund` and neither rail has one
//! (`mtn_momo::refund` is `NotImplemented`; Orange Money's Web Payment
//! product documents no refund API at all, so the port's default
//! `Unsupported` stands). `charge.refunded` and `charge.refund.updated` are
//! documented event types that **nothing emits** (`docs/status.md`). A
//! merchant that eventually holds a `re_…` in `pending` therefore has, today,
//! no call and no event that answers "what happened to it?".
//!
//! `docs/flows/provider-port.md` calls `query_status` "**the authoritative
//! read** … Must work indefinitely", and the merchant surface mirrors that
//! for every other money movement: `GET /v1/payment_intents/{id}`,
//! `GET /v1/checkout/sessions/{id}`, `GET /v1/events/{id}`. Refunds were the
//! one exception, which is what issue #45 asked the maintainer to decide.
//! The decision was that the route is part of the `/v1` contract **and is
//! served** — a webhook is not a substitute for a read when delivery is
//! at-least-once and unordered (`docs/flows/webhooks.md`).
//!
//! # What this module deliberately does not do
//!
//! No creation, and no events. `POST /v1/refunds` is untouched and still
//! answers the nest's honest `404`; nothing here writes a `refunds` row, and
//! `vpay_db::Refunds` exposes no way to. Until a rail can refund, every row
//! this route can read is one an operator or a test put there.
//!
//! # One renderer
//!
//! [`crate::model::RefundObject`] is what this route returns and what the
//! writer of `charge.refund.updated` will have to put in `data.object`, for
//! the reason [`crate::v1::events`] gives about its own: two renderers would
//! let the documented fallback answer a different question from the one the
//! webhook asked.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;

use vpay_db::{Refunds, Repositories};

use crate::error::ApiError;
use crate::model::RefundObject;
use crate::v1::MerchantScope;

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a `404` for a refund can never be spelled two ways.
const RESOURCE: &str = "refund";

/// `GET /v1/refunds/{id}`.
///
/// A foreign merchant's refund id is a `404`, byte for byte the same `404` a
/// nonexistent one gets — `vpay_db::Refunds::get_for_merchant` folds both
/// into `None` on purpose, and the scope is a join onto the owning intent
/// because `refunds` carries no `merchant_id` of its own. Telling the two
/// apart would let anyone holding one credential enumerate which `re_…` ids
/// exist across the whole deployment, which is the same reason
/// `GET /v1/payment_intents/{id}` answers the way it does.
///
/// # Why the prefix is checked, and why it is checked *into the same 404*
///
/// An id that is not `re_…` cannot name a row in `refunds`
/// (`vpay_core::ids::refund_id` is the only minter, and the `id_length`
/// CHECK bounds the rest), so the check saves a database round trip for a
/// caller who pasted a `pi_…`. It answers the identical `404` rather than a
/// `400`, and that is the load-bearing half: `crate::v1::events`'s own
/// `retrieve` explains why a malformed id here is a `404` and not a shape
/// error, and a distinguishable answer would be one more thing this route
/// tells a caller than `/v1/events/{id}` does. The check changes what
/// Postgres is asked, never what the merchant is told.
///
/// # Errors
///
/// [`ApiError::NotFound`] for an id this merchant has no refund under;
/// [`ApiError::Db`] if the read fails; [`ApiError::Internal`] for a row that
/// will not render, which migration `0017`'s `metadata_is_object` CHECK makes
/// unreachable.
pub(crate) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = lookup(repositories.as_ref(), scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;

    crate::v1::payment_intents::json_response(StatusCode::OK, &RefundObject::try_from(&row)?)
}

/// The read, with the prefix short-circuit in front of it.
///
/// Split out of [`retrieve`] so the short-circuit is a value a unit test can
/// reason about rather than a branch only an HTTP round trip reaches, and so
/// the handler reads as one sentence.
async fn lookup(
    repositories: &dyn Repositories,
    merchant_id: &str,
    id: &str,
) -> Result<Option<vpay_db::RefundRow>, ApiError> {
    if !id.starts_with(vpay_core::ids::REFUND_PREFIX) {
        return Ok(None);
    }
    Ok(Refunds::get_for_merchant(repositories, merchant_id, id).await?)
}

/// The one `404` this module produces, so the envelope cannot be built two
/// ways with two different `resource` strings.
fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{RESOURCE, not_found};
    use crate::error::ApiError;

    /// The wire vocabulary this module is written against, pinned as a
    /// literal: `resource` is what a `404` envelope's message names
    /// (`No such refund: …`), and a rename that only touched the constant
    /// would compile and change the message with nothing noticing.
    #[test]
    fn the_resource_name_is_the_one_the_documented_envelope_uses() {
        assert_eq!(RESOURCE, "refund");
        match not_found("re_x") {
            ApiError::NotFound { resource, id } => {
                assert_eq!(resource, "refund");
                assert_eq!(id, "re_x");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// The prefix this route reads is the one `vpay_core` mints, not a second
    /// spelling of it. A literal `"re_"` here and a changed `REFUND_PREFIX`
    /// would make every real id a `404`.
    #[test]
    fn the_prefix_short_circuit_uses_the_minters_own_vocabulary() {
        assert_eq!(vpay_core::ids::REFUND_PREFIX, "re_");
        assert!(vpay_core::ids::refund_id().starts_with(vpay_core::ids::REFUND_PREFIX));
    }
}
