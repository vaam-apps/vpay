//! `/v1/invoice_items` — the lines of a **draft** invoice.
//!
//! Its own file rather than four more handlers in [`super::invoices`], for
//! [`super::payment_intents`]' reason — one resource, one file — and because
//! the rule that is this resource and no other belongs beside the code it
//! constrains:
//!
//! **every write here is refused once its invoice is issued.** Not by a check
//! in this module: by `EXISTS (SELECT 1 FROM invoices WHERE id = … AND status
//! = 'draft')` inside each statement (`vpay_db::invoices`' `PARENT_IS_A_DRAFT`).
//! A handler that read the invoice, saw `draft`, and then wrote would have a
//! window in which a concurrent finalize commits — and the line it changed
//! afterwards would be a line on a document that has already been sent, with
//! a number on it, to a payer.
//!
//! # Why the object is a `line_item` and the route is `invoice_items`
//!
//! Stripe has two objects where vpay has one. An `invoiceitem` there is a
//! *pending* charge not yet attached to any document; a `line_item` is what
//! appears on `invoice.lines` once it is. vpay's `POST /v1/invoice_items`
//! writes straight onto a named draft — there is no pending-charge inbox,
//! because that is a subscription feature and subscriptions are not built.
//! So the route keeps Stripe's spelling (a merchant's existing client calls
//! it) and the object keeps Stripe's `lines` spelling (a Stripe-shaped
//! handler switches on it). [`crate::model::LineItemTag`] states the
//! consequence; `docs/flows/invoices.md` records the gap.

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use time::OffsetDateTime;
use vpay_core::ids;
use vpay_db::{InvoiceItemPatch, Invoices, NewInvoiceItem, Repositories, ResponseSubject};

use crate::error::ApiError;
use crate::model::{DeletedObject, DeletedTrue, InvoiceLineObject, LineItemTag};
use crate::v1::MerchantScope;
use crate::v1::invoices::{DESCRIPTION_MAX_CHARS, checked_description};
use crate::v1::payment_intents::{ClaimOutcome, PostRequest, json_response};

/// The object type this module speaks about, in the API's own vocabulary.
const RESOURCE: &str = "line_item";

/// `quantity`'s ceiling.
///
/// Not migration `0036`'s — the database only requires `quantity >= 1`. This
/// is an **overflow** bound and it is here because `amount = quantity *
/// unit_amount` is computed by Postgres in `BIGINT`: two values a merchant
/// can send freely can overflow that product, and a `BIGINT` overflow is a
/// `22003` a merchant reads as "vpay is broken" rather than as "that line
/// costs more than money".
///
/// A million of anything on one line, at a ceiling of a hundred million minor
/// units each ([`UNIT_AMOUNT_MAX`]), is 10^14 — four orders of magnitude
/// inside `i64`, so no combination this module accepts can overflow.
const QUANTITY_MAX: i64 = 1_000_000;

/// `unit_amount`'s ceiling, in integer minor units — 100,000,000, i.e. one
/// hundred million FCFA on a single unit.
///
/// See [`QUANTITY_MAX`] for why there is a ceiling at all. The number itself
/// is chosen to be far above any real Cameroonian transaction and far below
/// the point where the product matters.
const UNIT_AMOUNT_MAX: i64 = 100_000_000;

// ----------------------------------------------------------------- create

/// `POST /v1/invoice_items`'s fields — text for
/// [`super::invoices::CreateParams`]' reason.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateParams {
    invoice: Option<String>,
    description: Option<String>,
    quantity: Option<String>,
    unit_amount: Option<String>,
}

/// `POST /v1/invoice_items` — adds a line to a draft.
///
/// # There is no `currency` parameter, deliberately
///
/// A line is always in its invoice's currency. The insert copies it off the
/// parent in the same statement that writes the row, so a line cannot be
/// denominated in anything else — and a total summed across two currencies
/// would be a number that means nothing. A merchant who wants a second
/// currency creates a second invoice.
///
/// # Nor a `merchant` or `livemode` one
///
/// Both are copied off the parent for the same reason, and the parent is
/// found by a `WHERE` that already carries this request's authenticated
/// tenant — so a line can never claim a tenant its invoice does not have.
pub(crate) async fn create(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = create_once(&post, repositories.as_ref(), &scope).await;
    post.finish(
        repositories.as_ref(),
        &scope,
        claim_id,
        outcome,
        ResponseSubject::Verbatim,
    )
    .await
}

/// The create itself. See [`create`].
async fn create_once(
    post: &PostRequest,
    repositories: &dyn Repositories,
    scope: &MerchantScope,
) -> Result<Response, ApiError> {
    let params: CreateParams = post.form().await?;

    let invoice = required_invoice(params.invoice.as_deref())?;
    let description = required_description(params.description)?;
    let quantity = parse_quantity(params.quantity.as_deref())?.unwrap_or(1);
    let unit_amount = parse_unit_amount(params.unit_amount.as_deref())?;

    let new = NewInvoiceItem {
        id: ids::invoice_item_id(),
        invoice_id: invoice,
        merchant_id: scope.merchant_id().to_owned(),
        description,
        quantity,
        unit_amount,
        created_at: OffsetDateTime::now_utc(),
    };

    // The insert is the guard: tenancy, the draft status and the copy of the
    // parent's three columns are all in the one statement. `None` means there
    // is no such draft invoice for this merchant — a finalized one included.
    let (item, _invoice) = Invoices::add_item(repositories, &new)
        .await?
        .ok_or_else(|| no_such_draft(&new.invoice_id))?;

    line_response(StatusCode::CREATED, &item)
}

// --------------------------------------------------------------- retrieve

/// `GET /v1/invoice_items/{id}`.
///
/// Readable whatever the parent's status: a merchant reading a line off an
/// invoice they issued a month ago is asking a question, not making a change.
/// Only the *writes* carry the draft guard.
pub(crate) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = Invoices::get_item_for_merchant(repositories.as_ref(), scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;
    line_response(StatusCode::OK, &row)
}

// ----------------------------------------------------------------- update

/// `POST /v1/invoice_items/{id}`'s fields.
///
/// All three are single `Option`s and not the double options
/// [`super::invoices::UpdateParams`] uses, because all three columns are
/// `NOT NULL`: there is no "clear it" state to carry, and `description=` is a
/// `400` naming the parameter rather than a clear.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct UpdateParams {
    description: Option<String>,
    quantity: Option<String>,
    unit_amount: Option<String>,
}

/// `POST /v1/invoice_items/{id}` (and `PATCH`) — **draft parent only**.
///
/// `amount` is recomputed by the statement from whichever factor the patch
/// supplies. It is not a parameter and never will be: a caller-supplied
/// `amount` that disagreed with `quantity * unit_amount` is exactly the row
/// migration `0036`'s `amount_is_the_product` refuses, and offering the
/// parameter would make that refusal a merchant's problem.
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
        ResponseSubject::Verbatim,
    )
    .await
}

/// The update itself. See [`update`].
async fn update_once(
    post: &PostRequest,
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    let params: UpdateParams = post.form().await?;

    let patch = InvoiceItemPatch {
        description: match params.description {
            None => None,
            Some(raw) => Some(required_description(Some(raw))?),
        },
        quantity: parse_quantity(params.quantity.as_deref())?,
        unit_amount: match params.unit_amount.as_deref() {
            None => None,
            Some(raw) => Some(parse_unit_amount(Some(raw))?),
        },
    };

    if patch.is_empty() {
        // A bodiless `POST` answers the line unchanged —
        // `super::invoices::update`'s contract, and Stripe's.
        let row = Invoices::get_item_for_merchant(repositories, scope.merchant_id(), id)
            .await?
            .ok_or_else(|| not_found(id))?;
        return line_response(StatusCode::OK, &row);
    }

    let updated = Invoices::update_item(
        repositories,
        scope.merchant_id(),
        id,
        &patch,
        OffsetDateTime::now_utc(),
    )
    .await?;

    match updated {
        Some((item, _invoice)) => line_response(StatusCode::OK, &item),
        None => Err(refusal_reason(repositories, scope, id).await),
    }
}

// ----------------------------------------------------------------- delete

/// `DELETE /v1/invoice_items/{id}` — **draft parent only**.
///
/// The invoice's total is re-summed in the same transaction, so a merchant
/// who reads the invoice back never sees a total that does not match the
/// lines they are looking at.
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
        ResponseSubject::Verbatim,
    )
    .await
}

/// The delete itself. See [`delete`].
async fn delete_once(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    if Invoices::delete_item(repositories, scope.merchant_id(), id)
        .await?
        .is_some()
    {
        return json_response(
            StatusCode::OK,
            &DeletedObject {
                id: id.to_owned(),
                object: LineItemTag,
                deleted: DeletedTrue,
            },
        );
    }
    Err(refusal_reason(repositories, scope, id).await)
}

// --------------------------------------------------------------- plumbing

/// Renders a stored line.
///
/// # Errors
///
/// [`ApiError::Internal`] only if the object will not serialise, which for a
/// wire DTO means a bug in a `Serialize` impl:
/// [`InvoiceLineObject::from`] is infallible, unlike every other conversion
/// in `crate::model`, because `invoice_items` has no column that could be
/// stored in a shape a renderer would have to reject.
fn line_response(status: StatusCode, row: &vpay_db::InvoiceItemRow) -> Result<Response, ApiError> {
    json_response(status, &InvoiceLineObject::from(row))
}

/// Turns a write's zero-rows answer into the refusal a merchant should see.
///
/// Three causes are indistinguishable to the statement — no such line,
/// another merchant's line, and a parent that is no longer a draft — so this
/// reads afterwards to tell them apart. **The read runs after the refusal and
/// never instead of it**; see [`super::invoices::update`] for the same rule
/// stated in full.
async fn refusal_reason(
    repositories: &dyn Repositories,
    scope: &MerchantScope,
    id: &str,
) -> ApiError {
    match Invoices::get_item_for_merchant(repositories, scope.merchant_id(), id).await {
        Ok(Some(row)) => ApiError::Conflict {
            message: format!(
                "Invoice {} is no longer a draft, so its lines cannot be changed. An issued \
                 invoice's lines are what was sent to the payer; void it with \
                 POST /v1/invoices/{{id}}/void and issue a new one instead.",
                row.invoice_id
            ),
        },
        Ok(None) => not_found(id),
        Err(error) => ApiError::from(error),
    }
}

/// The refusal for a line this merchant does not have.
fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// The refusal for an `invoice=` that names no draft of this merchant's.
///
/// **One sentence for three causes** — no such invoice, another merchant's
/// invoice, and one that is no longer a draft — for
/// `super::customers::unknown_customer`'s reason applied to this parameter: a
/// distinct answer would make `invoice=` an oracle for which `in_…` exist
/// under some other tenant. It is a `400` naming the parameter rather than a
/// `404`, because the id came in the *body* and the route itself is fine.
fn no_such_draft(invoice_id: &str) -> ApiError {
    ApiError::invalid_param(
        "invoice",
        format!(
            "`invoice` must name one of your draft invoices. {invoice_id} is not one — it may \
             not exist, or it may already have been finalized, in which case its lines are \
             frozen."
        ),
    )
}

/// `invoice=`, shape-checked, or a `400` naming it.
///
/// The shape check runs before the lookup for `paging::validated_cursor`'s
/// reason: a `cus_…` or a truncated paste is a typo the merchant can fix from
/// the message, and letting it reach the database would answer the
/// deliberately opaque "not one of your drafts" instead.
fn required_invoice(raw: Option<&str>) -> Result<String, ApiError> {
    let raw = raw.map(str::trim).filter(|value| !value.is_empty());
    let raw = raw.ok_or_else(|| {
        ApiError::invalid_param(
            "invoice",
            "`invoice` is required: a line belongs to one invoice, and vpay has no \
             pending-line inbox to hold one that does not.",
        )
    })?;
    if !ids::is_well_formed(ids::INVOICE_PREFIX, raw) {
        return Err(ApiError::invalid_param(
            "invoice",
            "`invoice` must be an Invoice id — `in_` followed by 24 characters.",
        ));
    }
    Ok(raw.to_owned())
}

/// `description`, required and bounded.
///
/// Required, unlike an invoice's own: a line with no text is a charge a payer
/// cannot identify, and it is what prints on the document.
fn required_description(raw: Option<String>) -> Result<String, ApiError> {
    let trimmed = raw
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let value = trimmed.ok_or_else(|| {
        ApiError::invalid_param(
            "description",
            "`description` is required on a line: it is the text the payer reads.",
        )
    })?;
    checked_description(Some(value))?.ok_or_else(|| {
        // Unreachable — `checked_description` returns its input unchanged on
        // the `Ok` path — and spelled as a refusal rather than an `expect`
        // (ADR-0007).
        ApiError::invalid_param(
            "description",
            format!("`description` must be at most {DESCRIPTION_MAX_CHARS} characters."),
        )
    })
}

/// `quantity`, or `None` if the request did not mention it.
///
/// Absent means 1 on create ([`create_once`] applies the default) and "leave
/// it alone" on update. A merchant billing one of something should not have
/// to say so.
fn parse_quantity(raw: Option<&str>) -> Result<Option<i64>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let quantity: i64 = raw.parse().map_err(|_error| {
        ApiError::invalid_param("quantity", "`quantity` must be a whole number.")
    })?;
    if !(1..=QUANTITY_MAX).contains(&quantity) {
        return Err(ApiError::invalid_param(
            "quantity",
            format!("`quantity` must be between 1 and {QUANTITY_MAX}."),
        ));
    }
    Ok(Some(quantity))
}

/// `unit_amount`, required, in integer minor units.
///
/// Minor units and never a decimal: XAF is zero-decimal, `5000` means 5,000
/// FCFA, and this workspace denies float arithmetic outright
/// (`docs/flows/money.md`). A `1500.50` is refused by the parse rather than
/// rounded, because rounding somebody's price is a decision vpay must not
/// make silently.
///
/// Zero is **accepted**: a free line on an invoice is a real thing (a
/// discount described in words, an included item), and `amount_non_negative`
/// is what the database asks for.
fn parse_unit_amount(raw: Option<&str>) -> Result<i64, ApiError> {
    let raw = raw.map(str::trim).filter(|value| !value.is_empty());
    let raw = raw.ok_or_else(|| {
        ApiError::invalid_param(
            "unit_amount",
            "`unit_amount` is required, in the currency's smallest unit — `5000` is 5,000 \
             FCFA.",
        )
    })?;
    let amount: i64 = raw.parse().map_err(|_error| {
        ApiError::invalid_param(
            "unit_amount",
            "`unit_amount` must be a whole number of the currency's smallest unit — `5000`, \
             not `50.00`.",
        )
    })?;
    if !(0..=UNIT_AMOUNT_MAX).contains(&amount) {
        return Err(ApiError::invalid_param(
            "unit_amount",
            format!("`unit_amount` must be between 0 and {UNIT_AMOUNT_MAX}."),
        ));
    }
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::{
        QUANTITY_MAX, UNIT_AMOUNT_MAX, parse_quantity, parse_unit_amount, required_invoice,
    };

    /// The two numeric parameters refuse everything a `BIGINT` product could
    /// overflow on, and accept the boundary itself.
    ///
    /// The pairing is the point: `QUANTITY_MAX * UNIT_AMOUNT_MAX` is asserted
    /// to be inside `i64` here, so the two constants can never be raised
    /// independently into a combination Postgres would answer `22003` for.
    #[test]
    fn the_numeric_bounds_cannot_overflow_the_product_the_database_computes() {
        assert!(
            QUANTITY_MAX.checked_mul(UNIT_AMOUNT_MAX).is_some(),
            "a line at both ceilings overflows the BIGINT `amount = quantity * unit_amount`, \
             which reaches a merchant as a 22003 saying vpay is broken"
        );

        assert_eq!(parse_quantity(Some("1")).expect("1 is legal"), Some(1));
        assert_eq!(
            parse_quantity(Some(&QUANTITY_MAX.to_string())).expect("the ceiling is legal"),
            Some(QUANTITY_MAX)
        );
        assert_eq!(parse_quantity(None).expect("absent is None"), None);
        for wrong in ["0", "-1", "1.5", "lots"] {
            assert_eq!(
                parse_quantity(Some(wrong)).expect_err("refused").param(),
                Some("quantity"),
                "for {wrong}"
            );
        }
        assert!(parse_quantity(Some(&(QUANTITY_MAX + 1).to_string())).is_err());

        // Zero is a legal price; a decimal is not.
        assert_eq!(parse_unit_amount(Some("0")).expect("free lines exist"), 0);
        assert_eq!(parse_unit_amount(Some("5000")).expect("5,000 FCFA"), 5000);
        for wrong in ["50.00", "-1", "", "5 000"] {
            assert_eq!(
                parse_unit_amount(Some(wrong)).expect_err("refused").param(),
                Some("unit_amount"),
                "for {wrong}"
            );
        }
        assert!(parse_unit_amount(None).is_err(), "unit_amount is required");
    }

    /// `invoice=` is shape-checked before any lookup, and a *different*
    /// resource's id is refused rather than looked up and missed.
    #[test]
    fn an_invoice_reference_is_shape_checked_before_it_is_looked_up() {
        let good = vpay_core::ids::invoice_id();
        assert_eq!(
            required_invoice(Some(&good)).expect("a real invoice id passes"),
            good
        );

        for wrong in [
            vpay_core::ids::customer_id(),
            vpay_core::ids::invoice_item_id(),
            "in_short".to_owned(),
            String::new(),
        ] {
            assert_eq!(
                required_invoice(Some(&wrong)).expect_err("refused").param(),
                Some("invoice"),
                "for {wrong}"
            );
        }
        assert!(required_invoice(None).is_err(), "invoice is required");
    }
}
