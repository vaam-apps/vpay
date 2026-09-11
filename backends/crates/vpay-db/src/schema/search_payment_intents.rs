//! The body of `procedure searchPaymentIntents` — the dashboard's payments
//! list, paginated by offset and scoped to the caller's own tenant.
//!
//! This is the first hand-written procedure body in vpay, and it exists
//! because a *generated* read cannot serve this table: `model PaymentIntent`
//! carries no `@@allow` arm, so `db.payment_intent().find_many()` renders the
//! literal `FALSE` into its `WHERE` and answers zero rows, forever. A
//! procedure is the one shape CrateStack offers where the statement is ours
//! — the macro emits the signature, the `Args` struct, the policy check and
//! the `Authorized` witness, and nothing else
//! (`cratestack-macros-0.12.0/src/procedure.rs`).
//!
//! # What this is not
//!
//! **No transport serves it.** Nothing in this workspace mounts a CrateStack
//! axum router or an RPC dispatcher, `vpay_db::persistence::system_context`
//! is the only [`CratestackContext`] vpay mints anywhere, and a system
//! context carries no tenant — so `ProcedureRegistry::search_payment_intents` would
//! refuse it. **The only callers are this module's own tests**, which is
//! also why `Payments` is `allow(dead_code)` in a non-test build — see
//! `crate::schema`'s note on the `mod` declaration. `docs/status.md` says
//! the same thing in the same words; the declaration in
//! `schemas/vpay.cstack` is not an endpoint.
//!
//! # What offset paging cannot promise, and what `seq` does not fix
//!
//! Added by the exp54 review, 2026-09-11, because nothing in this branch
//! said it and the surrounding prose read as if the opposite were true.
//!
//! `payment_intents.seq` carries a UNIQUE index (`payment_intents_seq_key`,
//! migration `0014`), so `ORDER BY seq DESC` is a **total** order: one
//! statement never has a tie to break and never returns a row twice. That is
//! the whole of what the index buys, and it is a smaller thing than it
//! looks. **`OFFSET` is counted afresh on every call**, so a payment created
//! between one page and the next shifts the window by one: a caller walking
//! `offset = 0, 10, 20` over a list that is being written to sees the last
//! row of a page again at the top of the next one, and misses a row at the
//! tail for every intent inserted behind its back. `seq DESC` puts new rows
//! at offset 0, which is the direction that makes this happen on a busy
//! merchant rather than a quiet one.
//!
//! This is a property of offset paging, not a defect in this body, and
//! nothing here compensates for it — which is why `GET /v1/payment_intents`
//! and `GET /dash/v1/payment_intents` are cursor-paged: `starting_after`
//! resolves an id to a `seq` and asks for rows *below* it, and an insert
//! cannot move that. `total_count` is exact for the statement that returned
//! it and says nothing about the next one. Anything that must not
//! double-count a payment — a reconciliation, an export, a sum — reads the
//! cursor list, not this.
//!
//! # The three things that would be wrong if they were missing
//!
//! Each is named after the mutation it catches, and each mutation was run:
//!
//! 1. **The tenancy predicate.** `merchant_id` comes from
//!    `ctx.tenant_id()`, never from the arguments, and its absence is a
//!    refusal rather than an unscoped read. A tenant that owns nothing and
//!    a tenant that does not exist get the same empty page, which is the
//!    uniform refusal `PaymentIntents::get_for_merchant` gives a foreign
//!    id. Replacing `WHERE merchant_id = $1` with a tautology puts
//!    `pi_b_only` in `merchant_a`'s page and reddens
//!    `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded`
//!    — no unit test can catch it, because a predicate is only observable
//!    through the rows it excluded.
//! 2. **The limit clamp.** [`cratestack::PageInput::resolve`], reached
//!    through [`resolve_page`], is what turns an absent or hostile `limit`
//!    into a bounded one; without it a caller chooses how many rows Postgres
//!    materialises. Dropping the call is a Postgres `OFFSET must not be
//!    negative` and an unclamped `MAX_PAGE_LIMIT + 1`, both in that same
//!    container test — and it leaves
//!    `tests::a_hostile_limit_is_clamped_before_it_reaches_postgres` green,
//!    which is why both exist.
//! 3. **The status refusal.** An unknown status is a `400` naming `status`,
//!    not an empty page — "no payments are `succeded`" is a sentence an
//!    operator reads as an answer about their payments rather than about
//!    their typo. `vpay_api::dash::payment_intents` has said so since it was
//!    written; this says it in the same vocabulary, and accepting anything
//!    reddens
//!    `tests::an_unknown_status_is_refused_rather_than_answered_with_an_empty_page`.

use std::str::FromStr as _;

use cratestack::{CratestackContext, CratestackError, Page, PageInfo, PageInput};
use time::OffsetDateTime;
use vpay_core::IntentStatus;

use super::cratestack_schema::{self, procedures, types};

/// The largest page this procedure will answer.
///
/// **100, which is a deliberate copy of `vpay_api::v1::paging::MAX_LIMIT`**
/// rather than [`cratestack::MAX_LIST_LIMIT`] (1000). The two surfaces are
/// read by the same operator, and a procedure that hands out ten times the
/// page the REST list does would make "one page" mean two different things
/// on one screen. It is a copy because `vpay-db` cannot depend on
/// `vpay-api`, and **nothing gates the two into agreement** — if that
/// ceiling moves, move this one. Only the weaker half is testable here —
/// `tests::the_page_ceiling_stays_inside_cratestacks_own` checks that this
/// never exceeds CrateStack's own resource-exhaustion ceiling.
const MAX_PAGE_LIMIT: i64 = 100;

/// The page this procedure answers when the caller names no `limit`, and a
/// copy of `vpay_api::v1::paging::DEFAULT_LIMIT` for the reason
/// [`MAX_PAGE_LIMIT`] copies `MAX_LIMIT`.
///
/// **Separate from the ceiling, and it has to be, because
/// [`cratestack::PageInput::resolve`] fuses the two.** `resolve(max)`
/// defaults an absent `limit` *to `max`*, so handing it `MAX_PAGE_LIMIT`
/// alone answers 100 rows to a caller who named no page size, where
/// `GET /dash/v1/payment_intents` answers 10. That is exactly the "ten times
/// the page the REST list does" [`MAX_PAGE_LIMIT`] rules out, arriving
/// through the default instead of through the ceiling. Found by the exp54
/// review, 2026-09-11; the ceiling was already right and the default was
/// not.
const DEFAULT_PAGE_LIMIT: i64 = 10;

/// Every column [`SummaryRow`] decodes, plus the window count.
///
/// A `&'static str` and not a `format!`, which is why no `AssertSqlSafe`
/// appears in this file: sqlx 0.9 takes a `&'static str` directly, and
/// `crate::sql_audit`'s pinned site count is a count of the places that
/// check is switched off. Adding one here would have meant raising that
/// number to buy nothing.
///
/// **`metadata`, `payment_method_types` and `client_secret_suffix` are not
/// in the projection**, and none of the three is an oversight — see the
/// `type PaymentIntentSummary` comment in `schemas/vpay.cstack`. Neither is
/// `seq`: it orders the page (`ORDER BY seq DESC`, the index
/// `payment_intents_merchant_seq_idx` exists for) without being selected.
///
/// `count(*) OVER ()` is the exact size of the filtered set, computed in the
/// same snapshot as the rows themselves — a second `SELECT count(*)` would
/// be a consistency window in which the total and the page disagree. It
/// travels *on a row*, so a page past the end of the set carries no total at
/// all; [`page_of`] answers `total_count: None` there rather than `0`, which
/// would be a lie about a set that has rows in it.
///
/// **`offset == 0` is the exception, and the exp54 review added it.** The
/// statement asks for `limit + 1` rows starting at the first one, so an
/// empty result at offset zero is not "the count fell off the page" — it is
/// proof the filtered set is empty, and `0` is the truth. Answering `None`
/// there told an operator filtering by a status they have none of that the
/// total was unknown when it was known to be none.
const SEARCH_SQL: &str = "SELECT id, merchant_id, livemode, amount, amount_received, \
                          amount_refunded, amount_refund_pending, currency_code, status, \
                          last_payment_error_code, description, customer_id, created_at, \
                          updated_at, count(*) OVER () AS total_count \
                          FROM payment_intents \
                          WHERE merchant_id = $1 \
                            AND ($2::TEXT IS NULL OR status = $2) \
                            AND ($3::TIMESTAMPTZ IS NULL OR created_at >= $3) \
                            AND ($4::TIMESTAMPTZ IS NULL OR created_at <= $4) \
                          ORDER BY seq DESC \
                          LIMIT $5 OFFSET $6";

/// vpay's implementation of the schema's `ProcedureRegistry`.
///
/// A unit struct: the trait hands the body its [`cratestack_schema::Cratestack`]
/// and its [`CratestackContext`] on every call, so there is no state a
/// registry could usefully hold, and holding a pool here would be a second
/// way to reach the database that nothing audits.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Payments;

impl procedures::ProcedureRegistry for Payments {
    async fn search_payment_intents(
        &self,
        db: &cratestack_schema::Cratestack,
        ctx: &CratestackContext,
        args: procedures::search_payment_intents::Args,
        _authorized: procedures::search_payment_intents::Authorized,
    ) -> Result<procedures::search_payment_intents::Output, CratestackError> {
        // The tenant is read from the context and never from `args`. There
        // is no `merchant_id` argument to this procedure and there must not
        // be one: an argument is something the caller chooses, and a tenancy
        // predicate a caller chooses is not a tenancy predicate.
        let merchant_id = tenant_of(ctx)?;
        let status = validated_status(args.filter.status.as_deref())?;
        let (limit, offset) = resolve_page(args.page);

        // `limit + 1`, the same trick `PaymentIntents::list_page_filtered`
        // uses: one row past the page is how `has_next_page` is learned
        // without a second statement, and it also means a `limit` of zero
        // still fetches a row and so still carries the window count.
        let rows: Vec<SummaryRow> = sqlx::query_as::<_, SummaryRow>(SEARCH_SQL)
            .bind(merchant_id)
            .bind(status)
            .bind(to_time(args.filter.created_gte)?)
            .bind(to_time(args.filter.created_lte)?)
            .bind(limit.saturating_add(1))
            .bind(offset)
            .fetch_all(db.pool())
            .await
            .map_err(cratestack::cratestack_error_from_sqlx)?;

        page_of(rows, limit, offset)
    }
}

/// [`PageInput::resolve`]'s clamp, with vpay's default page size rather
/// than CrateStack's.
///
/// The clamp is **not** reimplemented and must not be: an absent `limit`
/// becomes [`DEFAULT_PAGE_LIMIT`] here, and everything that makes the
/// arguments safe — the `[0, MAX_PAGE_LIMIT]` clamp on `limit`, the `>= 0`
/// clamp on `offset` — is still `resolve`'s. Replacing this body with
/// `(page.limit.unwrap_or(DEFAULT_PAGE_LIMIT), page.offset.unwrap_or(0))`
/// still reddens
/// `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded`
/// with Postgres' own `OFFSET must not be negative`, which is the mutation
/// that matters.
fn resolve_page(page: PageInput) -> (i64, i64) {
    PageInput {
        limit: Some(page.limit.unwrap_or(DEFAULT_PAGE_LIMIT)),
        offset: page.offset,
    }
    .resolve(MAX_PAGE_LIMIT)
}

/// The merchant a context is allowed to read, or a refusal.
///
/// **`Forbidden`, and deliberately the same `Forbidden` the `@allow` check
/// produces.** A caller with no tenant and a caller the policy rejected are
/// told the same thing, because the difference between them is a fact about
/// vpay's own authentication that the caller has no business learning from
/// an error code.
///
/// It is never `Ok` with an empty string: an empty `merchant_id` would be a
/// perfectly well-formed predicate matching no rows, and "no results" is the
/// answer an operator would believe.
fn tenant_of(ctx: &CratestackContext) -> Result<&str, CratestackError> {
    match ctx.tenant_id() {
        Some(tenant) if !tenant.is_empty() => Ok(tenant),
        _ => Err(CratestackError::Forbidden(
            "procedure policy denied this operation".to_owned(),
        )),
    }
}

/// Refuses a `status` filter outside [`IntentStatus`]'s vocabulary.
///
/// The refusal is the whole point, and it is a `400` rather than an empty
/// page for the reason `vpay_api::dash::payment_intents::parse_status` gives:
/// the column is `TEXT` since migration `0037`, so an unknown label is a
/// perfectly valid query returning zero rows, and that is the answer an
/// operator would read as being about their payments.
///
/// The vocabulary is asked of **both** owners and they must agree:
/// `vpay_core::IntentStatus` is what the API parses with, and
/// `cratestack_schema::types::IntentStatus` is what `schemas/vpay.cstack`
/// declares. `tests::the_two_intent_status_vocabularies_are_one_vocabulary`
/// fails if they ever diverge, which is the failure that would otherwise let
/// this surface accept a status the rest of vpay cannot name.
fn validated_status(raw: Option<&str>) -> Result<Option<String>, CratestackError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if types::IntentStatus::from_str(raw).is_ok() {
        return Ok(Some(raw.to_owned()));
    }
    Err(CratestackError::BadRequest(format!(
        "status: unknown payment intent status. Known statuses are: {}.",
        IntentStatus::ALL
            .iter()
            .map(|status| status.as_wire_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Assembles the answer from the `limit + 1` rows the statement returned.
///
/// `has_next_page` is "we saw a row past the page", not arithmetic over
/// `total_count`: the extra row is observed in the same snapshot as the page,
/// and it is still right when the total is unknown.
fn page_of(
    mut rows: Vec<SummaryRow>,
    limit: i64,
    offset: i64,
) -> Result<Page<types::PaymentIntentSummary>, CratestackError> {
    let has_next_page = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    // The window count is the same on every row of the result, so the first
    // one answers for all of them. No row means no total — see `SEARCH_SQL`
    // — *except* at `offset == 0`, where no row is itself the answer: the
    // statement asked for `limit + 1` rows starting at the first one, so an
    // empty result proves the filtered set is empty and `0` is the truth
    // rather than a guess. Without this arm an operator who filters by a
    // status they have none of is told the total is unknown, which is a
    // worse answer than "none" and the one a table would render as a blank.
    let total_count = rows
        .first()
        .map(|row| row.total_count)
        .or_else(|| (offset == 0).then_some(0));
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));

    let items = rows
        .into_iter()
        .map(SummaryRow::into_summary)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Page::new(
        items,
        PageInfo {
            limit: Some(limit),
            offset: Some(offset),
            has_next_page,
            has_previous_page: offset > 0,
        },
    )
    .with_total_count(total_count))
}

/// One row of [`SEARCH_SQL`], in the types Postgres hands back.
///
/// `status` and `last_payment_error_code` arrive as `String`: this crate
/// carries both vocabularies as text and `vpay-core` owns parsing them
/// (`crate::payment_intents::COLUMNS` records the same division). The
/// parsing happens in [`Self::into_summary`], once, at the point where the
/// value becomes part of an answer.
#[derive(Debug, Clone, sqlx::FromRow)]
struct SummaryRow {
    id: String,
    merchant_id: String,
    livemode: bool,
    amount: i64,
    amount_received: i64,
    amount_refunded: i64,
    amount_refund_pending: i64,
    currency_code: String,
    status: String,
    last_payment_error_code: Option<String>,
    description: Option<String>,
    customer_id: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    /// `count(*) OVER ()` — the size of the whole filtered set, not of this
    /// page.
    total_count: i64,
}

impl SummaryRow {
    /// The stored row as the procedure's answer.
    ///
    /// # Errors
    ///
    /// [`CratestackError::Internal`] when the database holds a `status` or a
    /// `last_payment_error_code` outside the vocabulary this build declares.
    /// Both mean the row and the binary disagree, which is an operator's
    /// problem and not a caller's — so the caller is told nothing beyond
    /// "internal error" ([`CratestackError::public_message`]) while the
    /// detail reaches the log. Rendering a default status instead would put a
    /// payment in front of an operator under a state it is not in.
    ///
    /// The two timestamps cannot fail: see [`to_chrono`].
    fn into_summary(self) -> Result<types::PaymentIntentSummary, CratestackError> {
        let status = types::IntentStatus::from_str(&self.status).map_err(|error| {
            CratestackError::Internal(format!(
                "payment_intents.status for {} is outside this build's vocabulary: {error}",
                self.id
            ))
        })?;
        let last_payment_error_code = self
            .last_payment_error_code
            .as_deref()
            .map(|code| {
                types::FailureCode::from_str(code).map_err(|error| {
                    CratestackError::Internal(format!(
                        "payment_intents.last_payment_error_code for {} is outside this build's \
                         vocabulary: {error}",
                        self.id
                    ))
                })
            })
            .transpose()?;

        Ok(types::PaymentIntentSummary {
            id: self.id.clone(),
            merchant_id: self.merchant_id,
            livemode: self.livemode,
            amount: self.amount,
            amount_received: self.amount_received,
            amount_refunded: self.amount_refunded,
            amount_refund_pending: self.amount_refund_pending,
            currency_code: self.currency_code,
            status,
            last_payment_error_code,
            description: self.description,
            customer_id: self.customer_id,
            created_at: to_chrono(self.created_at),
            updated_at: to_chrono(self.updated_at),
        })
    }
}

/// A filter bound, in the type sqlx binds a `TIMESTAMPTZ` from.
///
/// The conversion exists because the two halves of this file speak different
/// calendars and neither choice was free: `schemas/vpay.cstack`'s `DateTime`
/// is `chrono::DateTime<Utc>` (`cratestack-macros-0.12.0/src/shared/types.rs:37`,
/// not negotiable — the macro emits it), and every `TIMESTAMPTZ` this crate
/// binds is `time::OffsetDateTime`, because vpay deliberately does not enable
/// sqlx's `chrono` feature (`vpay-db/Cargo.toml` says why).
///
/// **The nanosecond clamp is not defensive rounding.** `chrono` represents a
/// leap second as a subsecond in `1_000_000_000..=1_999_999_999` and `time`
/// has no leap-second slot at all, so carrying the raw value into
/// `from_unix_timestamp_nanos` would move the bound into the *next second* —
/// a filter silently off by up to a second on exactly the instant a caller
/// is most likely to have copied from somewhere. Clamping to the last
/// representable nanosecond of the same second is what
/// `client_assertion::chrono_to_offset_date_time` does, for this reason; it
/// is not shared with this because that one answers in `OpError`, which is
/// authkestra-op's vocabulary and not a procedure's.
///
/// # Errors
///
/// [`CratestackError::BadRequest`] for an instant outside `time`'s range —
/// roughly ±9999 years, four orders of magnitude narrower than `chrono`'s, so
/// this is genuinely reachable from a caller. A caller sent it, so a caller
/// is told which parameter was wrong.
fn to_time(
    bound: Option<cratestack::chrono::DateTime<cratestack::chrono::Utc>>,
) -> Result<Option<OffsetDateTime>, CratestackError> {
    let Some(bound) = bound else {
        return Ok(None);
    };
    let seconds = OffsetDateTime::from_unix_timestamp(bound.timestamp()).map_err(|error| {
        CratestackError::BadRequest(format!(
            "created_gte/created_lte: {error} — the bound is outside the range vpay stores"
        ))
    })?;
    seconds
        .replace_nanosecond(bound.timestamp_subsec_nanos().min(999_999_999))
        .map(Some)
        .map_err(|error| CratestackError::BadRequest(format!("created_gte/created_lte: {error}")))
}

/// A stored `TIMESTAMPTZ` as the procedure's answer. [`to_time`]'s inverse,
/// and the same calendar seam.
///
/// **Total, and that is measured rather than hoped.** `chrono`'s range is
/// roughly ±262,000 years and `time::OffsetDateTime`'s, at this workspace's
/// default features, is −9999..=9999 — so every value sqlx could have decoded
/// into the argument converts. In `customers::tests`,
/// `the_chrono_conversion_is_total_over_every_instant_time_can_hold` proves
/// that at both extremes, and this function deliberately does not repeat the
/// proof.
///
/// It is a second copy of `customers::to_chrono` rather than a shared one on
/// purpose: the two differ only in the value of an unreachable branch, and
/// that value is a per-caller judgement. There it is `MAX_UTC` because
/// "freshly used" is the direction that *keeps* a personal-data record;
/// here it is `MAX_UTC` for a different reason — a payment stamped year
/// 262143 cannot be mistaken for a real one, where a `UNIX_EPOCH` fallback
/// would render as a plausible 1970 date in an operator's table. ADR-0007
/// denies `expect`, so the branch has to answer something.
fn to_chrono(stored: OffsetDateTime) -> cratestack::chrono::DateTime<cratestack::chrono::Utc> {
    cratestack::chrono::DateTime::from_timestamp(stored.unix_timestamp(), stored.nanosecond())
        .unwrap_or(cratestack::chrono::DateTime::<cratestack::chrono::Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    //! Eleven cases with no database — the parts of the body that decide
    //! something *before* a statement runs, plus the assembly of the answer
    //! from rows a test supplies — and one, in [`against_postgres`], that
    //! starts a container. The split is not a preference: a tenancy
    //! predicate is only observable through the rows it excluded, so the
    //! half that matters most cannot be a unit test.
    //!
    //! They live here rather than in `tests/repositories.rs` because
    //! `Payments` is `pub(crate)`: the `ProcedureRegistry` trait it
    //! implements is inside `crate::schema`'s private expansion, so no
    //! integration-test crate can name either.

    use super::*;

    /// The refusal a caller with no tenant gets, and the fact that it is the
    /// *same* refusal the policy produces.
    ///
    /// Decisive: replacing `tenant_of`'s `Err` arm with `Ok("")` — the
    /// cheapest way to make an unscoped read compile — fails the first
    /// assertion, and widening it to `Ok(tenant)` without the emptiness
    /// check fails the third.
    #[test]
    fn a_context_without_a_tenant_cannot_read_any_merchants_payments() {
        let anonymous = CratestackContext::anonymous();
        let error = tenant_of(&anonymous)
            .expect_err("a context with no tenant must be refused, not read unscoped");
        assert!(matches!(error, CratestackError::Forbidden(_)), "{error:?}");

        // Byte for byte what `cratestack_policy::eval` writes, so a caller
        // cannot tell "you are not signed in" from "you have no tenant".
        assert_eq!(
            error.public_message(),
            "procedure policy denied this operation"
        );

        let empty_tenant = context_for_tenant("");
        assert!(
            tenant_of(&empty_tenant).is_err(),
            "an empty merchant_id is a predicate that matches nothing, which reads as an answer"
        );

        assert_eq!(
            tenant_of(&context_for_tenant("acme-cameroon-tenant")).ok(),
            Some("acme-cameroon-tenant")
        );
    }

    /// An unknown status is a refusal naming `status`, never an empty page.
    ///
    /// Decisive: making `validated_status` return `Ok(Some(raw))` for
    /// anything — which is what "just pass it to the query, the column is
    /// TEXT" looks like — fails on the first unknown value.
    #[test]
    fn an_unknown_status_is_refused_rather_than_answered_with_an_empty_page() {
        for status in IntentStatus::ALL {
            assert_eq!(
                validated_status(Some(status.as_wire_str())).ok(),
                Some(Some(status.as_wire_str().to_owned())),
            );
        }

        for unknown in ["succeded", "SUCCEEDED", "", "succeeded ", "'; DROP TABLE"] {
            let error = validated_status(Some(unknown))
                .expect_err("an unknown status must be refused, not answered with an empty list");
            assert!(
                matches!(error, CratestackError::BadRequest(_)),
                "{unknown:?} produced {error:?}"
            );
            let message = error.public_message().into_owned();
            assert!(message.starts_with("status:"), "{message}");
            assert!(message.contains("requires_payment_method"), "{message}");
        }

        assert_eq!(validated_status(None).ok(), Some(None));
    }

    /// `schemas/vpay.cstack`'s `enum IntentStatus` and
    /// `vpay_core::IntentStatus` are one vocabulary, in both directions.
    ///
    /// They are two transcriptions of the same thing and nothing but this
    /// test makes them agree. A variant added to one alone would make this
    /// surface accept a status the rest of vpay cannot name, or refuse one
    /// it can.
    #[test]
    fn the_two_intent_status_vocabularies_are_one_vocabulary() {
        for status in IntentStatus::ALL {
            let parsed = types::IntentStatus::from_str(status.as_wire_str())
                .unwrap_or_else(|error| panic!("{}: {error}", status.as_wire_str()));
            assert_eq!(parsed.as_str(), status.as_wire_str());
        }

        // And back: every schema variant is one the core knows. Walking the
        // generated enum needs a list and it offers none — no `ALL`, no
        // `IntoEnumIterator` — so the five labels are written out by hand.
        //
        // What that costs, said plainly rather than glossed: the `len()`
        // assertion below catches a variant added to `vpay_core` without
        // this list being updated, and a variant added to
        // `schemas/vpay.cstack` ALONE would still escape both halves. The
        // schema's enum is a transcription of the core's — every variant
        // carries the comment saying so — and the direction that actually
        // breaks this surface (the core learning a status the schema cannot
        // parse) is the one the loop above covers.
        for label in [
            "requires_payment_method",
            "requires_action",
            "processing",
            "succeeded",
            "canceled",
        ] {
            assert!(
                IntentStatus::from_wire(label).is_some(),
                "the schema declares {label}, vpay-core does not"
            );
            assert!(types::IntentStatus::from_str(label).is_ok(), "{label}");
        }
        assert_eq!(IntentStatus::ALL.len(), 5);
    }

    /// `schemas/vpay.cstack`'s `enum FailureCode` and
    /// `vpay_core::FailureCode` are one vocabulary, in both directions.
    ///
    /// **Added by the exp54 review, and the gap it closes is not symmetric
    /// with the status one.** An unknown `status` is a caller's typo and
    /// [`validated_status`] answers `400`. An unknown
    /// `last_payment_error_code` is a *stored* value, and
    /// [`SummaryRow::into_summary`] answers [`CratestackError::Internal`] —
    /// correct when the database really does hold something this build
    /// cannot name, and badly wrong when the two transcriptions of vpay's
    /// **own** vocabulary have merely drifted. In that case every payment
    /// that failed for the missing reason becomes a `500` on an operator's
    /// list, and before this test nothing would have noticed:
    /// `search_payment_intents.rs` is the only consumer of
    /// `types::FailureCode` anywhere in the workspace, so the schema's copy
    /// had no other reader to disagree with.
    ///
    /// Decisive: delete any line from `enum FailureCode` in
    /// `schemas/vpay.cstack` and the first loop fails naming it.
    #[test]
    fn the_two_failure_code_vocabularies_are_one_vocabulary() {
        for code in vpay_core::FailureCode::ALL {
            let parsed = types::FailureCode::from_str(code.as_str())
                .unwrap_or_else(|error| panic!("{}: {error}", code.as_str()));
            assert_eq!(parsed.as_str(), code.as_str());
        }

        // And back. The generated enum offers no `ALL` and no iterator, so
        // the eleven labels are written out — the same cost, and the same
        // admitted limit, as `the_two_intent_status_vocabularies_are_one_
        // vocabulary`: a variant added to `schemas/vpay.cstack` ALONE still
        // escapes both halves. The direction that breaks this surface is a
        // rail code `vpay-core` knows and the schema cannot parse, and the
        // loop above covers that one.
        for label in [
            "insufficient_funds",
            "payer_timeout",
            "payer_declined",
            "invalid_payer",
            "payer_limit_reached",
            "payer_account_blocked",
            "invalid_payee",
            "payee_account_blocked",
            "provider_account_blocked",
            "provider_unavailable",
            "provider_error",
        ] {
            assert!(
                vpay_core::FailureCode::ALL
                    .iter()
                    .any(|code| code.as_str() == label),
                "the schema declares {label}, vpay-core does not"
            );
            assert!(types::FailureCode::from_str(label).is_ok(), "{label}");
        }
        assert_eq!(vpay_core::FailureCode::ALL.len(), 11);
    }

    /// [`resolve_page`]'s contract, at every boundary a caller can reach.
    ///
    /// **This pins `resolve_page`, not the body's use of it, and the
    /// difference is measured rather than assumed.** Replacing the body's
    /// `resolve_page(args.page)` with
    /// `(args.page.limit.unwrap_or(DEFAULT_PAGE_LIMIT), args.page.offset.unwrap_or(0))`
    /// leaves this test green — it is
    /// `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded`
    /// that goes red, with `OFFSET must not be negative` from Postgres and a
    /// `limit` of `MAX_PAGE_LIMIT + 1` coming back unclamped. Both tests are
    /// needed: this one says what the clamp must do, that one says the body
    /// still asks for it.
    ///
    /// The first row is the one the exp54 review added and the reason
    /// [`DEFAULT_PAGE_LIMIT`] exists: it was `MAX_PAGE_LIMIT` before, because
    /// `PageInput::resolve` defaults an absent `limit` to its ceiling, and a
    /// caller who named no page size got 100 rows where `/dash/v1` gives 10.
    #[test]
    fn a_hostile_limit_is_clamped_before_it_reaches_postgres() {
        let cases = [
            (None, None, DEFAULT_PAGE_LIMIT, 0),
            (None, Some(30), DEFAULT_PAGE_LIMIT, 30),
            (Some(10), Some(20), 10, 20),
            (Some(MAX_PAGE_LIMIT), None, MAX_PAGE_LIMIT, 0),
            (Some(MAX_PAGE_LIMIT + 1), None, MAX_PAGE_LIMIT, 0),
            (Some(i64::MAX), None, MAX_PAGE_LIMIT, 0),
            (Some(-1), Some(-1), 0, 0),
            (Some(i64::MIN), Some(i64::MIN), 0, 0),
            (Some(0), None, 0, 0),
        ];

        for (limit, offset, expected_limit, expected_offset) in cases {
            assert_eq!(
                resolve_page(PageInput { limit, offset }),
                (expected_limit, expected_offset),
                "limit={limit:?} offset={offset:?}"
            );
        }
    }

    /// The ceiling this procedure applies is never looser than CrateStack's
    /// own resource-exhaustion ceiling, and the default never exceeds the
    /// ceiling.
    ///
    /// The weaker half of `MAX_PAGE_LIMIT`'s contract, and the only half a
    /// test can hold: `vpay-db` cannot see `vpay_api::v1::paging::MAX_LIMIT`
    /// or `DEFAULT_LIMIT` to compare against them. The strong half — that
    /// these two are the same numbers `/dash/v1` uses — is still ungated,
    /// and the constants' own doc comments are the only thing saying so.
    #[test]
    fn the_page_ceiling_stays_inside_cratestacks_own() {
        const { assert!(MAX_PAGE_LIMIT > 0) };
        const { assert!(MAX_PAGE_LIMIT <= cratestack::MAX_LIST_LIMIT) };
        const { assert!(DEFAULT_PAGE_LIMIT > 0) };
        const { assert!(DEFAULT_PAGE_LIMIT <= MAX_PAGE_LIMIT) };
    }

    /// `has_next_page` comes from the extra row, and the total from the
    /// window count — including the two cases where there is no extra row
    /// and no row at all.
    #[test]
    fn the_envelope_reports_the_page_it_actually_has() {
        let full = page_of(rows(3, 7), 2, 0).expect("fixture rows assemble");
        assert_eq!(full.items.len(), 2);
        assert_eq!(full.total_count, Some(7));
        assert!(full.page_info.has_next_page);
        assert!(!full.page_info.has_previous_page);
        assert_eq!(full.page_info.limit, Some(2));
        assert_eq!(full.page_info.offset, Some(0));

        let last = page_of(rows(2, 7), 2, 5).expect("fixture rows assemble");
        assert_eq!(last.items.len(), 2);
        assert!(
            !last.page_info.has_next_page,
            "no row past the page was seen"
        );
        assert!(last.page_info.has_previous_page);

        // Past the end: the window count travels on a row and there is none,
        // so the total is unknown rather than zero.
        let past_end = page_of(rows(0, 0), 2, 99).expect("an empty result assembles");
        assert!(past_end.items.is_empty());
        assert_eq!(past_end.total_count, None);
        assert!(!past_end.page_info.has_next_page);
        assert!(past_end.page_info.has_previous_page);

        // An empty FIRST page is a different fact, and the exp54 review is
        // why it is one: the statement asked for `limit + 1` rows starting at
        // the first one, so no row at `offset == 0` proves the filtered set
        // is empty. `0` is the truth; `None` was an operator being told the
        // total was unknown when it was known to be none.
        //
        // Decisive: dropping `page_of`'s `(offset == 0).then_some(0)` arm
        // fails the assertion below and nothing else in this file.
        let empty_set = page_of(rows(0, 0), 2, 0).expect("an empty result assembles");
        assert!(empty_set.items.is_empty());
        assert_eq!(empty_set.total_count, Some(0));
        assert!(!empty_set.page_info.has_next_page);
        assert!(!empty_set.page_info.has_previous_page);
    }

    /// A stored status this build cannot name is an error, not a default.
    ///
    /// Decisive: `types::IntentStatus::from_str(...).unwrap_or_default()`
    /// compiles and would render `requires_payment_method` for a succeeded
    /// payment. This fails on it.
    #[test]
    fn a_row_whose_status_this_build_cannot_name_is_an_error_not_a_default() {
        let mut unknown_status = row("pi_unknown_status", 1);
        unknown_status.status = "settled".to_owned();
        let error = unknown_status
            .into_summary()
            .expect_err("a status outside the vocabulary must not be rendered as another status");
        assert!(matches!(error, CratestackError::Internal(_)), "{error:?}");
        assert_eq!(error.public_message(), "internal error");

        let mut unknown_failure = row("pi_unknown_failure", 1);
        unknown_failure.last_payment_error_code = Some("gremlins".to_owned());
        assert!(unknown_failure.into_summary().is_err());
    }

    /// A filter bound survives the chrono/time seam in both directions,
    /// to the nanosecond.
    #[test]
    fn a_timestamp_bound_round_trips_across_the_chrono_time_seam() {
        let chrono_bound = cratestack::chrono::DateTime::from_timestamp(1_788_652_800, 123_456_789)
            .expect("a representable instant");
        let as_time = to_time(Some(chrono_bound))
            .expect("a representable instant converts")
            .expect("Some in, Some out");
        assert_eq!(as_time.unix_timestamp(), 1_788_652_800);
        assert_eq!(as_time.nanosecond(), 123_456_789);
        assert_eq!(to_chrono(as_time), chrono_bound);

        assert_eq!(to_time(None).expect("None converts"), None);

        // A chrono leap second has no `time` slot: it must land on the last
        // nanosecond of its own second, not on the next one. Without the
        // clamp, `from_unix_timestamp_nanos` would move the bound a whole
        // second forward and the filter would quietly exclude a payment
        // created in between.
        let leap = cratestack::chrono::NaiveDate::from_ymd_opt(2026, 12, 31)
            .and_then(|date| date.and_hms_nano_opt(23, 59, 59, 1_500_000_000))
            .expect("chrono spells a leap second as a subsecond above 1e9")
            .and_utc();
        assert_eq!(leap.timestamp_subsec_nanos(), 1_500_000_000);
        let clamped = to_time(Some(leap))
            .expect("a leap second is clamped, not refused")
            .expect("Some in, Some out");
        assert_eq!(clamped.unix_timestamp(), leap.timestamp());
        assert_eq!(clamped.nanosecond(), 999_999_999);

        // And the reachable refusal: beyond `time`'s ±9999 years, which is
        // four orders of magnitude narrower than chrono's range.
        let far = cratestack::chrono::DateTime::<cratestack::chrono::Utc>::MAX_UTC;
        let error = to_time(Some(far)).expect_err("beyond year 9999 is not storable");
        assert!(matches!(error, CratestackError::BadRequest(_)), "{error:?}");
    }

    /// The projection carries no credential and no `jsonb` column.
    ///
    /// Asserted over the statement text rather than over the struct: a
    /// column added to `SEARCH_SQL` alone is a compile error today, but a
    /// column added to both is not, and this is what would notice.
    #[test]
    fn the_statement_selects_no_credential_and_no_jsonb_column() {
        for forbidden in [
            "client_secret_suffix",
            "metadata",
            "payment_method_types",
            "last_payment_error_message",
        ] {
            assert!(
                !SEARCH_SQL.contains(forbidden),
                "{forbidden} is in the payments-list projection: {SEARCH_SQL}"
            );
        }
        assert!(SEARCH_SQL.contains("WHERE merchant_id = $1"));
        assert!(SEARCH_SQL.contains("ORDER BY seq DESC"));
    }

    /// The policy refuses an anonymous caller before any argument is read,
    /// and the body refuses an authenticated caller with no tenant before
    /// any statement runs.
    ///
    /// **The pool cannot connect** — port 1 on loopback, `crate::schema`'s
    /// own device — so the second half is decisive in a way an assertion on
    /// the error alone would not be: if the tenancy check moved below the
    /// `sqlx` call, this test would fail with a connection error instead of
    /// a `Forbidden`, which is exactly the ordering a reviewer cannot see by
    /// reading.
    ///
    /// It is also the only place `Payments` is constructed inside this
    /// crate, which is the honest shape of a procedure nothing serves.
    #[tokio::test]
    async fn the_tenancy_refusal_happens_before_the_statement_does() {
        use procedures::ProcedureRegistry as _;

        let cs = super::super::tests::lazy_cratestack();
        let args = procedures::search_payment_intents::Args {
            page: PageInput::default(),
            filter: types::PaymentIntentListFilter {
                status: None,
                created_gte: None,
                created_lte: None,
            },
        };

        let anonymous = CratestackContext::anonymous();
        let refused = procedures::search_payment_intents::authorize_with_db(&cs, &args, &anonymous)
            .await
            .expect_err("@allow(auth() != null) must refuse an anonymous caller");
        assert!(
            matches!(refused, CratestackError::Forbidden(_)),
            "{refused:?}"
        );

        // Authenticated, but with no tenant facet: the policy passes and
        // hands out the witness, and the body is what refuses.
        let tenantless = CratestackContext::authenticated([(
            "sub".to_owned(),
            cratestack::Value::String("staff-with-no-merchant".to_owned()),
        )]);
        let authorized =
            procedures::search_payment_intents::authorize_with_db(&cs, &args, &tenantless)
                .await
                .expect("an authenticated caller satisfies the only @allow arm");

        let error = Payments
            .search_payment_intents(&cs, &tenantless, args, authorized)
            .await
            .expect_err("no tenant means no rows may be read, not all of them");
        assert!(matches!(error, CratestackError::Forbidden(_)), "{error:?}");

        // **The two refusals compared against each other, not against a
        // literal** — the exp54 review's change, and the reason is that
        // "byte for byte what the policy writes" is a claim about
        // `cratestack_policy::eval`, which this repository does not own.
        // `eval` builds the message as `format!("{construct} policy denied
        // this operation")`; the day a CrateStack release rewords it, a
        // literal here keeps passing while the indistinguishability the
        // whole design rests on is silently gone. Asserting equality is the
        // only spelling that fails on that.
        assert_eq!(
            error.public_message(),
            refused.public_message(),
            "a caller must not be able to tell 'you are not signed in' from \
             'you have no tenant'"
        );
        assert_eq!(
            error.public_message(),
            "procedure policy denied this operation",
            "and the wording itself, so a change in it is seen rather than \
             absorbed by the equality above"
        );
    }

    /// A context carrying one tenant claim, which is the only thing
    /// [`tenant_of`] reads.
    fn context_for_tenant(tenant: &str) -> CratestackContext {
        CratestackContext::with_principal(cratestack::PrincipalContext {
            tenant: Some(cratestack::PrincipalFacet {
                fields: std::collections::BTreeMap::from([(
                    "id".to_owned(),
                    cratestack::Value::String(tenant.to_owned()),
                )]),
            }),
            ..cratestack::PrincipalContext::default()
        })
    }

    /// `count` rows, each claiming the filtered set holds `total`.
    fn rows(count: usize, total: i64) -> Vec<SummaryRow> {
        (0..count)
            .map(|index| {
                let mut row = row(&format!("pi_fixture_{index}"), total);
                row.total_count = total;
                row
            })
            .collect()
    }

    /// The halves of this procedure that only rows can prove, against a real
    /// Postgres.
    ///
    /// **One container and one test, deliberately** — though not for the
    /// reason `tests/postgres.rs`' header gave until the exp54 review
    /// corrected it. `vpay-db` **is** covered by `.config/nextest.toml`'s
    /// one-at-a-time filter and has been since 2026-09-02: the
    /// `postgres-containers` override names `package(vpay-db)` explicitly,
    /// so no container start in this crate races another. The reason that
    /// survives is cheaper and still real — under a `max-threads = 1` group,
    /// one chunky test that seeds two tenants once and asks it six questions
    /// costs one container and one start; six tests would cost six, in
    /// series.
    mod against_postgres {
        use anyhow::Context as _;
        use sqlx::postgres::PgPoolOptions;
        use testcontainers::ContainerAsync;
        use testcontainers_modules::postgres::Postgres as PostgresImage;

        // No `use crate::{Migrations, PaymentIntents, ConfigReconcile}`:
        // `Repositories` has all three as supertraits, and `crate::connect`
        // hands back an `Arc<dyn Repositories>`.
        use super::*;

        /// `merchant_a`'s two intents and `merchant_b`'s one, in a migrated
        /// database, plus a `Cratestack` over the same database.
        ///
        /// Returns the container guard: dropping it removes the database, so
        /// the caller has to keep it alive for as long as it queries.
        ///
        /// `anyhow` and `?` rather than `.expect`, because `clippy.toml`'s
        /// test exemption covers a `#[test]` body and not a helper it calls.
        async fn seeded()
        -> anyhow::Result<(ContainerAsync<PostgresImage>, cratestack_schema::Cratestack)> {
            let container = vpay_testkit::containers::start_postgres_with_retry()
                .await
                .context("postgres:16-alpine starts")?;
            let host = container.get_host().await.context("container host")?;
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .context("container port")?;
            let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

            let repositories = crate::connect(&url).await.context("pool connects")?;
            repositories
                .run_migrations()
                .await
                .context("migrations apply")?;
            repositories
                .reconcile(
                    &[crate::CurrencySeed {
                        code: "XAF".to_owned(),
                        exponent: 0,
                    }],
                    &[],
                )
                .await
                .context("XAF exists, because payment_intents.currency_code is a foreign key")?;

            // Oldest first, so `ORDER BY seq DESC` has something to reverse.
            let day = time::Duration::days(1);
            let base = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
            for (id, merchant, status, created_at) in [
                ("pi_a_old", "merchant_a", "requires_payment_method", base),
                ("pi_a_new", "merchant_a", "succeeded", base + day),
                // The row that must never appear in `merchant_a`'s page, and
                // the only `canceled` row in the database — so a `canceled`
                // filter under `merchant_a` answering anything is a leak
                // rather than a coincidence.
                ("pi_b_only", "merchant_b", "canceled", base + day + day),
            ] {
                repositories
                    .insert(&crate::NewPaymentIntent {
                        id: id.to_owned(),
                        merchant_id: merchant.to_owned(),
                        livemode: false,
                        amount: 5_000,
                        currency_code: "XAF".to_owned(),
                        status: status.to_owned(),
                        last_payment_error_code: None,
                        last_payment_error_message: None,
                        payment_method_types: serde_json::json!(["mtn_momo"]),
                        metadata: serde_json::json!({ "merchant_ref": 7 }),
                        description: None,
                        customer_id: None,
                        client_secret_suffix: vpay_core::ids::client_secret_suffix(),
                        created_at,
                    })
                    .await
                    .with_context(|| format!("seeding {id}"))?;
            }

            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .context("a second pool for the procedure's own Cratestack")?;
            Ok((
                container,
                cratestack_schema::Cratestack::builder(pool).build(),
            ))
        }

        /// Runs the procedure the way a transport would: authorize first,
        /// then call with the witness that produced.
        async fn search(
            cs: &cratestack_schema::Cratestack,
            ctx: &CratestackContext,
            page: PageInput,
            filter: types::PaymentIntentListFilter,
        ) -> Result<Page<types::PaymentIntentSummary>, CratestackError> {
            use procedures::ProcedureRegistry as _;

            let args = procedures::search_payment_intents::Args { page, filter };
            let authorized =
                procedures::search_payment_intents::authorize_with_db(cs, &args, ctx).await?;
            Payments
                .search_payment_intents(cs, ctx, args, authorized)
                .await
        }

        fn no_filter() -> types::PaymentIntentListFilter {
            types::PaymentIntentListFilter {
                status: None,
                created_gte: None,
                created_lte: None,
            }
        }

        /// Six questions, one database.
        ///
        /// The first is the one that matters: **`merchant_b`'s row is not in
        /// `merchant_a`'s page**, and deleting `WHERE merchant_id = $1` from
        /// [`SEARCH_SQL`] is what it catches — no unit test can, because the
        /// predicate is only observable through a row it excluded.
        #[tokio::test]
        async fn the_page_is_the_tenants_own_rows_filtered_and_bounded() -> anyhow::Result<()> {
            let (_container, cs) = seeded().await?;
            let a = context_for_tenant("merchant_a");
            let b = context_for_tenant("merchant_b");

            // 1. The tenancy predicate, stated as rows.
            let page = search(&cs, &a, PageInput::default(), no_filter())
                .await
                .context("merchant_a's default page")?;
            let ids = page
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                ids,
                ["pi_a_new", "pi_a_old"],
                "newest first, and merchant_b's row is not merchant_a's business"
            );
            assert!(
                page.items
                    .iter()
                    .all(|item| item.merchant_id == "merchant_a"),
                "{ids:?}"
            );
            assert_eq!(page.total_count, Some(2));
            assert!(!page.page_info.has_next_page);
            assert!(!page.page_info.has_previous_page);

            // 2. The other tenant sees exactly its own one row through the
            //    same code path — so the first assertion is about the
            //    predicate and not about an empty table.
            let theirs = search(&cs, &b, PageInput::default(), no_filter())
                .await
                .context("merchant_b's default page")?;
            assert_eq!(
                theirs
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["pi_b_only"]
            );

            // 3. A status only the *other* tenant has is an empty page, not
            //    that tenant's row. Indistinguishable from "you have no
            //    canceled payments", which is the uniform refusal
            //    `get_for_merchant` gives a foreign id.
            let canceled = search(
                &cs,
                &a,
                PageInput::default(),
                types::PaymentIntentListFilter {
                    status: Some("canceled".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("a status filter")?;
            assert!(canceled.items.is_empty(), "{:?}", canceled.items);
            // `Some(0)`, not `None`: this is the first page of a set that is
            // genuinely empty, and "you have no canceled payments" is a
            // different answer from "the total is unknown". `None` is
            // reserved for a page past the end, which the unit test covers.
            assert_eq!(canceled.total_count, Some(0));

            // 4. A status this tenant does have.
            let succeeded = search(
                &cs,
                &a,
                PageInput::default(),
                types::PaymentIntentListFilter {
                    status: Some("succeeded".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("a status filter that matches")?;
            assert_eq!(
                succeeded
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["pi_a_new"]
            );
            assert_eq!(
                succeeded.items.first().map(|item| item.status),
                Some(types::IntentStatus::succeeded)
            );

            // 5. Paging: one row per page, and `has_next_page` is real.
            let first = search(
                &cs,
                &a,
                PageInput {
                    limit: Some(1),
                    offset: None,
                },
                no_filter(),
            )
            .await
            .context("the first page of one")?;
            assert_eq!(first.items.len(), 1);
            assert_eq!(
                first.items.first().map(|item| item.id.as_str()),
                Some("pi_a_new")
            );
            assert_eq!(first.total_count, Some(2));
            assert!(first.page_info.has_next_page);
            assert!(!first.page_info.has_previous_page);

            let second = search(
                &cs,
                &a,
                PageInput {
                    limit: Some(1),
                    offset: Some(1),
                },
                no_filter(),
            )
            .await
            .context("the second page of one")?;
            assert_eq!(
                second.items.first().map(|item| item.id.as_str()),
                Some("pi_a_old")
            );
            assert!(!second.page_info.has_next_page);
            assert!(second.page_info.has_previous_page);

            // 6. The clamp, against a real `LIMIT`. Postgres refuses a
            //    negative one outright, so an unclamped -1 would be a `22023`
            //    here rather than a bounded page.
            let clamped = search(
                &cs,
                &a,
                PageInput {
                    limit: Some(-1),
                    offset: Some(-1),
                },
                no_filter(),
            )
            .await
            .context("a negative limit must be clamped, not sent")?;
            assert!(clamped.items.is_empty());
            assert_eq!(clamped.page_info.limit, Some(0));
            assert_eq!(clamped.page_info.offset, Some(0));

            // The other end of the clamp, which the negative case does not
            // cover: an oversized limit comes back as the ceiling rather
            // than as what was asked for.
            let oversized = search(
                &cs,
                &a,
                PageInput {
                    limit: Some(MAX_PAGE_LIMIT + 1),
                    offset: None,
                },
                no_filter(),
            )
            .await
            .context("an oversized limit must be clamped, not sent")?;
            assert_eq!(oversized.page_info.limit, Some(MAX_PAGE_LIMIT));

            // And the created bounds, which need no second container.
            let cutoff = cratestack::chrono::DateTime::from_timestamp(
                (time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000)).unix_timestamp()
                    + 1,
                0,
            )
            .context("a representable cutoff")?;
            let recent = search(
                &cs,
                &a,
                PageInput::default(),
                types::PaymentIntentListFilter {
                    created_gte: Some(cutoff),
                    ..no_filter()
                },
            )
            .await
            .context("a created_gte bound")?;
            assert_eq!(
                recent
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["pi_a_new"]
            );

            Ok(())
        }
    }

    fn row(id: &str, total_count: i64) -> SummaryRow {
        SummaryRow {
            id: id.to_owned(),
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            amount: 5_000,
            amount_received: 0,
            amount_refunded: 0,
            amount_refund_pending: 0,
            currency_code: "XAF".to_owned(),
            status: IntentStatus::INITIAL.as_wire_str().to_owned(),
            last_payment_error_code: None,
            description: None,
            customer_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            total_count,
        }
    }
}
