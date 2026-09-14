//! The body of `procedure searchWebhookDeliveries` — the dashboard's answer
//! to "did my webhook actually arrive?"
//!
//! Modelled on `search_payment_intents.rs`, which that file's own header
//! calls "the first hand-written procedure body in vpay" and which this one
//! copies the shape of deliberately: the same offset paging, the same
//! `limit + 1` trick, the same `count(*) OVER ()` window count, the same
//! refusal-not-empty-page treatment of a caller with no tenant. What differs
//! is the one thing `docs/plans/2026-09-13-dashboard-nav-notes/slices.md`
//! calls out by name.
//!
//! # The tenancy predicate is a JOIN, and this is the thing that goes wrong
//!
//! `model WebhookDelivery` has **no `merchant_id` column at all** — see its
//! own header in `schemas/vpay.cstack`. A delivery's tenant is reachable only
//! through `event_id -> events.merchant_id`, so [`SEARCH_SQL`] joins to
//! `events` and predicates on **the joined table's column**, not a column
//! `webhook_deliveries` owns. A body that wrote `WHERE merchant_id = $1`
//! against `webhook_deliveries` alone would not fail to compile and would
//! not fail at runtime — Postgres has no `merchant_id` column on that table
//! to bind against, so that particular mistake is at least a compile-time
//! "column does not exist". The mistake this comment is actually guarding
//! against is subtler: dropping the `JOIN events e ON e.id = wd.event_id`
//! and the `WHERE e.merchant_id = $1` that depends on it, and replacing both
//! with nothing — a query that reads every row in `webhook_deliveries`,
//! including the endpoint URLs another tenant's merchant registered. Exactly
//! as `search_payment_intents.rs` documents, no unit test can catch that: a
//! predicate is only observable through the rows it excluded. The mutation
//! this crate's container test runs is the join's analogue of that file's —
//! replace `e.merchant_id = $1` with a tautology (`TRUE`) and assert another
//! merchant's delivery appears in this merchant's page — and its output is
//! reported in this change's commit message, not merely asserted green.
//!
//! # What is deliberately not in the summary
//!
//! `response_excerpt` — up to 2 000 characters of a merchant endpoint's own
//! response body — is not selected by [`SEARCH_SQL`] and has no field on
//! `types::WebhookDeliverySummary`. It belongs to a detail read if anywhere,
//! exactly as `search_payment_intents.rs` refuses
//! `last_payment_error_message` for the same reason. Nor is
//! `payload_sha256`, a forensic detail nobody reads from a list.
//! `events.data` is not carried either — it is `jsonb`, unmapped by
//! CrateStack's introspection (see `model Event`'s GAP note), and the only
//! thing this procedure takes from the join is `events.type`, a plain
//! `TEXT` column.
//!
//! # What offset paging cannot promise here either
//!
//! `search_payment_intents.rs`'s own section on this applies verbatim, with
//! one difference: `webhook_deliveries` carries no `seq`-shaped unique
//! sequence column at all, so [`SEARCH_SQL`] orders by `(created_at, id)` —
//! `id` is the primary key and breaks every tie `created_at` could produce
//! (two deliveries created in the same transaction share a `created_at`),
//! giving the same total order `seq DESC` gives `payment_intents` without a
//! column that does not exist on this table. `OFFSET` is still counted
//! afresh on every call and a delivery inserted between two calls still
//! shifts the window; nothing here changes that property of offset paging.

use cratestack::{CratestackContext, CratestackError, Page, PageInfo, PageInput};
use time::OffsetDateTime;
use uuid::Uuid;

use super::cratestack_schema::{self, procedures, types};

/// The largest page this procedure will answer.
///
/// A deliberate copy of `search_payment_intents::MAX_PAGE_LIMIT` rather than
/// a shared constant — that file's own comment gives the reason
/// (`vpay-db` cannot depend on `vpay-api`, so this is a copy of
/// `vpay_api::v1::paging::MAX_LIMIT` and nothing gates the two into
/// agreement) and it applies unchanged to a second procedure using
/// `PageInput`, per `docs/reference/vpay-db/cratestack-procedure.md`'s
/// closing instruction: "a second procedure using `PageInput` must do the
/// same or say why not."
const MAX_PAGE_LIMIT: i64 = 100;

/// The page this procedure answers when the caller names no `limit`. A copy
/// of `search_payment_intents::DEFAULT_PAGE_LIMIT`, separate from
/// [`MAX_PAGE_LIMIT`] for the same reason that file's constant is: handing
/// `PageInput::resolve` the ceiling alone would default an absent `limit` to
/// the ceiling, not to vpay's own default page size.
const DEFAULT_PAGE_LIMIT: i64 = 10;

/// The four `webhook_deliveries.state` values migration 0022's
/// `state_is_known` CHECK admits, in the order that CHECK lists them.
///
/// Not a `.cstack` enum and not a `vpay_core` type — see
/// `types::WebhookDeliverySummary.state`'s own comment in
/// `schemas/vpay.cstack` for why: no enum here could be renamed onto a
/// hand-named CHECK without a migration, and there is no existing Rust
/// vocabulary to keep an enum honest against the way `vpay_core::IntentStatus`
/// keeps `IntentStatus` honest. This is the one place that vocabulary is
/// spelled out for this procedure, and [`validated_state`] is the one place
/// it is read.
const KNOWN_STATES: [&str; 4] = ["pending", "succeeded", "failed", "exhausted"];

/// Every column [`SummaryRow`] decodes, plus the window count, from a join
/// of `webhook_deliveries` to the `events` row that carries its tenant.
///
/// **`e.merchant_id`, not any column on `webhook_deliveries`,** is the
/// tenancy predicate — see this module's own header. `wd.id` is the
/// deliberate tie-breaker after `wd.created_at`: this table has no `seq`
/// column, and `(created_at, id)` is a total order for the same reason
/// `payment_intents.seq DESC` is one, without inventing a column that is not
/// there.
///
/// A `&'static str`, so no `AssertSqlSafe` site is added here either — see
/// `search_payment_intents.rs`'s identical note on `sql_audit`'s pinned site
/// count.
const SEARCH_SQL: &str = "SELECT wd.id, wd.event_id, e.merchant_id, e.type AS event_type, \
                          wd.endpoint_id, wd.url, wd.state, wd.created_at, wd.sent_at, \
                          wd.responded_at, wd.next_attempt_at, \
                          count(*) OVER () AS total_count \
                          FROM webhook_deliveries wd \
                          JOIN events e ON e.id = wd.event_id \
                          WHERE e.merchant_id = $1 \
                            AND ($2::TEXT IS NULL OR wd.state = $2) \
                            AND ($3::TEXT IS NULL OR e.type = $3) \
                            AND ($4::TIMESTAMPTZ IS NULL OR wd.created_at >= $4) \
                            AND ($5::TIMESTAMPTZ IS NULL OR wd.created_at <= $5) \
                          ORDER BY wd.created_at DESC, wd.id DESC \
                          LIMIT $6 OFFSET $7";

/// The real body of `procedure searchWebhookDeliveries`, called from a thin
/// delegation on `search_payment_intents::Payments` — see that file's
/// `impl procedures::ProcedureRegistry for Payments` and its comment on why
/// the delegation is thin and the body lives here.
pub(super) async fn search(
    db: &cratestack_schema::Cratestack,
    ctx: &CratestackContext,
    args: procedures::search_webhook_deliveries::Args,
) -> Result<procedures::search_webhook_deliveries::Output, CratestackError> {
    // The tenant is read from the context and never from `args`. There is no
    // `merchant_id` argument to this procedure — an argument is something
    // the caller chooses, and a tenancy predicate a caller chooses is not a
    // tenancy predicate. `webhook_deliveries` itself has no such column to
    // have named one after in the first place.
    let merchant_id = tenant_of(ctx)?;
    let state = validated_state(args.filter.state.as_deref())?;
    let (limit, offset) = resolve_page(args.page);

    let rows: Vec<SummaryRow> = sqlx::query_as::<_, SummaryRow>(SEARCH_SQL)
        .bind(merchant_id)
        .bind(state)
        .bind(args.filter.event_type)
        .bind(to_time(args.filter.created_gte)?)
        .bind(to_time(args.filter.created_lte)?)
        .bind(limit.saturating_add(1))
        .bind(offset)
        .fetch_all(db.pool())
        .await
        .map_err(cratestack::cratestack_error_from_sqlx)?;

    Ok(page_of(rows, limit, offset))
}

/// [`PageInput::resolve`]'s clamp, with vpay's default page size —
/// byte-for-byte `search_payment_intents::resolve_page`'s body, and kept as
/// a separate copy for the same reason [`MAX_PAGE_LIMIT`] is: this file
/// cannot name a private function in a sibling module.
fn resolve_page(page: PageInput) -> (i64, i64) {
    PageInput {
        limit: Some(page.limit.unwrap_or(DEFAULT_PAGE_LIMIT)),
        offset: page.offset,
    }
    .resolve(MAX_PAGE_LIMIT)
}

/// The merchant a context is allowed to read, or a refusal — identical in
/// behaviour to `search_payment_intents::tenant_of` and kept as a separate
/// copy for the same reason.
///
/// `Forbidden`, and deliberately the same `Forbidden` the `@allow` check
/// produces: a caller with no tenant and a caller the policy rejected are
/// told the same thing. Never `Ok` with an empty string — an empty
/// `merchant_id` would be a well-formed predicate matching no rows, and "no
/// results" is the answer an operator would believe.
fn tenant_of(ctx: &CratestackContext) -> Result<&str, CratestackError> {
    match ctx.tenant_id() {
        Some(tenant) if !tenant.is_empty() => Ok(tenant),
        _ => Err(CratestackError::Forbidden(
            "procedure policy denied this operation".to_owned(),
        )),
    }
}

/// Refuses a `state` filter outside [`KNOWN_STATES`].
///
/// A `400` naming `state` and listing the known values, never an empty page
/// — "no deliveries are `sent`" (a plausible-looking typo of `succeeded`)
/// is an answer about a caller's spelling, not about their deliveries, and
/// this is the refusal `PaymentIntentListFilter.status`'s validation gives
/// for the identical reason.
fn validated_state(raw: Option<&str>) -> Result<Option<String>, CratestackError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if KNOWN_STATES.contains(&raw) {
        return Ok(Some(raw.to_owned()));
    }
    Err(CratestackError::BadRequest(format!(
        "state: unknown webhook delivery state. Known states are: {}.",
        KNOWN_STATES.join(", ")
    )))
}

/// Assembles the answer from the `limit + 1` rows the statement returned —
/// identical in shape to `search_payment_intents::page_of`, which carries
/// the doc comment explaining `has_next_page` and the `offset == 0` total
/// count exception; both apply unchanged here.
///
/// **Not fallible, unlike `search_payment_intents::page_of`.** That one
/// returns `Result` because `SummaryRow::into_summary` there can fail (a
/// stored `status` outside this build's vocabulary). This file's
/// [`SummaryRow::into_summary`] cannot fail — `state` and `event_type` are
/// carried as `String` precisely so there is no closed vocabulary to fall
/// out of — so this returns the page directly rather than wrapping it in an
/// `Ok` that could never be an `Err`.
fn page_of(
    mut rows: Vec<SummaryRow>,
    limit: i64,
    offset: i64,
) -> Page<types::WebhookDeliverySummary> {
    let has_next_page = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    let total_count = rows
        .first()
        .map(|row| row.total_count)
        .or_else(|| (offset == 0).then_some(0));
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));

    let items = rows.into_iter().map(SummaryRow::into_summary).collect();

    Page::new(
        items,
        PageInfo {
            limit: Some(limit),
            offset: Some(offset),
            has_next_page,
            has_previous_page: offset > 0,
        },
    )
    .with_total_count(total_count)
}

/// One row of [`SEARCH_SQL`], in the types Postgres hands back.
///
/// `state` and `event_type` arrive as `String`, exactly as
/// `search_payment_intents::SummaryRow.status` does, and for the same
/// reason: neither has a Rust vocabulary on this side to decode into, so
/// carrying them as `String` is honest about what this procedure actually
/// knows rather than inventing a decode that could fail.
#[derive(Debug, Clone, sqlx::FromRow)]
struct SummaryRow {
    id: Uuid,
    event_id: String,
    merchant_id: String,
    event_type: String,
    endpoint_id: String,
    url: String,
    state: String,
    created_at: OffsetDateTime,
    sent_at: Option<OffsetDateTime>,
    responded_at: Option<OffsetDateTime>,
    next_attempt_at: Option<OffsetDateTime>,
    /// `count(*) OVER ()` — the size of the whole filtered set, not of this
    /// page. See `search_payment_intents::SummaryRow.total_count`.
    total_count: i64,
}

impl SummaryRow {
    /// The stored row as the procedure's answer. Infallible: unlike
    /// `search_payment_intents::SummaryRow::into_summary`, nothing here
    /// decodes into a closed Rust vocabulary that the database could hold a
    /// value outside of — `state` and `event_type` are carried as `String`
    /// precisely so that this conversion cannot fail on a value this build
    /// does not recognise.
    fn into_summary(self) -> types::WebhookDeliverySummary {
        types::WebhookDeliverySummary {
            id: self.id,
            merchant_id: self.merchant_id,
            event_id: self.event_id,
            event_type: self.event_type,
            endpoint_id: self.endpoint_id,
            url: self.url,
            state: self.state,
            created_at: to_chrono(self.created_at),
            sent_at: self.sent_at.map(to_chrono),
            responded_at: self.responded_at.map(to_chrono),
            next_attempt_at: self.next_attempt_at.map(to_chrono),
        }
    }
}

/// A filter bound, in the type sqlx binds a `TIMESTAMPTZ` from — identical
/// to `search_payment_intents::to_time`, including the leap-second clamp,
/// and kept as a separate copy for the reason that file's own doc comment on
/// `to_chrono` gives: a per-caller judgement (`MAX_UTC` and why) that is not
/// actually shared logic even where the code looks it could be lifted
/// wholesale.
///
/// # Errors
///
/// [`CratestackError::BadRequest`] for an instant outside `time`'s roughly
/// ±9999-year range.
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

/// A stored `TIMESTAMPTZ` as the procedure's answer. [`to_time`]'s inverse —
/// see `search_payment_intents::to_chrono` for the totality argument, which
/// applies unchanged (it is a property of `time`'s and `chrono`'s ranges,
/// not of which table the timestamp came from).
fn to_chrono(stored: OffsetDateTime) -> cratestack::chrono::DateTime<cratestack::chrono::Utc> {
    cratestack::chrono::DateTime::from_timestamp(stored.unix_timestamp(), stored.nanosecond())
        .unwrap_or(cratestack::chrono::DateTime::<cratestack::chrono::Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    //! Mirrors `search_payment_intents::tests`'s split: unit cases for
    //! everything decided before a statement runs, and one container test
    //! for the half only rows can prove — the join-based tenancy predicate.

    use super::*;

    #[test]
    fn a_context_without_a_tenant_cannot_read_any_merchants_deliveries() {
        let anonymous = CratestackContext::anonymous();
        let error = tenant_of(&anonymous)
            .expect_err("a context with no tenant must be refused, not read unscoped");
        assert!(matches!(error, CratestackError::Forbidden(_)), "{error:?}");
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

    #[test]
    fn an_unknown_state_is_refused_rather_than_answered_with_an_empty_page() {
        for state in KNOWN_STATES {
            assert_eq!(
                validated_state(Some(state)).ok(),
                Some(Some(state.to_owned())),
            );
        }

        for unknown in ["sent", "SUCCEEDED", "", "succeeded ", "'; DROP TABLE"] {
            let error = validated_state(Some(unknown))
                .expect_err("an unknown state must be refused, not answered with an empty list");
            assert!(
                matches!(error, CratestackError::BadRequest(_)),
                "{unknown:?} produced {error:?}"
            );
            let message = error.public_message().into_owned();
            assert!(message.starts_with("state:"), "{message}");
            assert!(message.contains("exhausted"), "{message}");
        }

        assert_eq!(validated_state(None).ok(), Some(None));
    }

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

    #[test]
    fn the_page_ceiling_stays_inside_cratestacks_own() {
        const { assert!(MAX_PAGE_LIMIT > 0) };
        const { assert!(MAX_PAGE_LIMIT <= cratestack::MAX_LIST_LIMIT) };
        const { assert!(DEFAULT_PAGE_LIMIT > 0) };
        const { assert!(DEFAULT_PAGE_LIMIT <= MAX_PAGE_LIMIT) };
    }

    #[test]
    fn the_envelope_reports_the_page_it_actually_has() {
        let full = page_of(rows(3, 7), 2, 0);
        assert_eq!(full.items.len(), 2);
        assert_eq!(full.total_count, Some(7));
        assert!(full.page_info.has_next_page);
        assert!(!full.page_info.has_previous_page);

        let last = page_of(rows(2, 7), 2, 5);
        assert_eq!(last.items.len(), 2);
        assert!(!last.page_info.has_next_page);
        assert!(last.page_info.has_previous_page);

        let past_end = page_of(rows(0, 0), 2, 99);
        assert!(past_end.items.is_empty());
        assert_eq!(past_end.total_count, None);

        let empty_set = page_of(rows(0, 0), 2, 0);
        assert!(empty_set.items.is_empty());
        assert_eq!(empty_set.total_count, Some(0));
    }

    /// The projection carries no forensic detail and no `jsonb` column.
    ///
    /// Asserted over the statement text rather than over the struct, for
    /// `search_payment_intents::tests::the_statement_selects_no_credential_and_no_jsonb_column`'s
    /// reason: a column added to `SEARCH_SQL` alone is a compile error
    /// today, but a column added to both this file and the summary type is
    /// not, and this is what would notice.
    #[test]
    fn the_statement_selects_no_response_excerpt_and_no_jsonb_column() {
        for forbidden in ["response_excerpt", "payload_sha256", "e.data", "wd.data"] {
            assert!(
                !SEARCH_SQL.contains(forbidden),
                "{forbidden} is in the webhook-deliveries projection: {SEARCH_SQL}"
            );
        }
        assert!(SEARCH_SQL.contains("WHERE e.merchant_id = $1"));
        assert!(SEARCH_SQL.contains("JOIN events e ON e.id = wd.event_id"));
    }

    /// A `WebhookDeliverySummary` field-by-field check that `response_excerpt`
    /// has no way to reach the wire: the type this test constructs is the
    /// exact type the procedure answers with, so a field added to it without
    /// a matching addition here is a compile error, and there is deliberately
    /// no `response_excerpt` field to omit.
    #[test]
    fn the_summary_type_has_no_response_excerpt_field() {
        let summary = row("delivery-fixture", 1).into_summary();
        // Exhaustive construction: if `types::WebhookDeliverySummary` ever
        // grows a `response_excerpt` (or any other) field, this literal
        // stops compiling until it is named here — which is the point.
        let types::WebhookDeliverySummary {
            id: _,
            merchant_id,
            event_id: _,
            event_type: _,
            endpoint_id: _,
            url: _,
            state: _,
            created_at: _,
            sent_at: _,
            responded_at: _,
            next_attempt_at: _,
        } = summary;
        assert_eq!(merchant_id, "acme-cameroon-tenant");
    }

    #[tokio::test]
    async fn the_tenancy_refusal_happens_before_the_statement_does() {
        use procedures::ProcedureRegistry as _;

        let cs = super::super::tests::lazy_cratestack();
        let args = procedures::search_webhook_deliveries::Args {
            page: PageInput::default(),
            filter: types::WebhookDeliveryListFilter {
                state: None,
                event_type: None,
                created_gte: None,
                created_lte: None,
            },
        };

        let anonymous = CratestackContext::anonymous();
        let refused =
            procedures::search_webhook_deliveries::authorize_with_db(&cs, &args, &anonymous)
                .await
                .expect_err("@allow(auth() != null) must refuse an anonymous caller");
        assert!(
            matches!(refused, CratestackError::Forbidden(_)),
            "{refused:?}"
        );

        let tenantless = CratestackContext::authenticated([(
            "sub".to_owned(),
            cratestack::Value::String("staff-with-no-merchant".to_owned()),
        )]);
        let authorized =
            procedures::search_webhook_deliveries::authorize_with_db(&cs, &args, &tenantless)
                .await
                .expect("an authenticated caller satisfies the only @allow arm");

        let error = super::super::search_payment_intents::Payments
            .search_webhook_deliveries(&cs, &tenantless, args, authorized)
            .await
            .expect_err("no tenant means no rows may be read, not all of them");
        assert!(matches!(error, CratestackError::Forbidden(_)), "{error:?}");
        assert_eq!(
            error.public_message(),
            refused.public_message(),
            "a caller must not be able to tell 'you are not signed in' from \
             'you have no tenant'"
        );
    }

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

    fn rows(count: usize, total: i64) -> Vec<SummaryRow> {
        (0..count)
            .map(|index| {
                let mut row = row(&format!("delivery-fixture-{index}"), total);
                row.total_count = total;
                row
            })
            .collect()
    }

    fn row(event_id: &str, total_count: i64) -> SummaryRow {
        SummaryRow {
            id: Uuid::new_v4(),
            event_id: event_id.to_owned(),
            merchant_id: "acme-cameroon-tenant".to_owned(),
            event_type: "payment_intent.succeeded".to_owned(),
            endpoint_id: "primary".to_owned(),
            url: "https://merchant.example/webhooks".to_owned(),
            state: "pending".to_owned(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            sent_at: None,
            responded_at: None,
            next_attempt_at: None,
            total_count,
        }
    }

    /// The halves of this procedure that only rows can prove, against a real
    /// Postgres — the join-based tenancy predicate above everything else.
    mod against_postgres {
        use anyhow::Context as _;
        use sqlx::postgres::PgPoolOptions;
        use testcontainers::ContainerAsync;
        use testcontainers_modules::postgres::Postgres as PostgresImage;

        use super::*;

        /// `merchant_a`'s event and delivery, `merchant_b`'s event and
        /// delivery, in a migrated database, plus a `Cratestack` over the
        /// same database.
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

            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .context("a pool for seeding and for the procedure's own Cratestack")?;

            let day = time::Duration::days(1);
            let base = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);

            for (
                event_id,
                merchant,
                event_type,
                delivery_id,
                endpoint,
                url_value,
                state,
                created,
            ) in [
                (
                    "evt_a_old",
                    "merchant_a",
                    "payment_intent.created",
                    "11111111-1111-1111-1111-111111111111",
                    "primary",
                    "https://merchant-a.example/webhooks",
                    "succeeded",
                    base,
                ),
                (
                    "evt_a_new",
                    "merchant_a",
                    "payment_intent.succeeded",
                    "22222222-2222-2222-2222-222222222222",
                    "primary",
                    "https://merchant-a.example/webhooks",
                    "pending",
                    base + day,
                ),
                // The row that must never appear in `merchant_a`'s page —
                // and the only `exhausted` delivery in the database, so an
                // `exhausted` filter under `merchant_a` answering anything is
                // a leak rather than a coincidence.
                (
                    "evt_b_only",
                    "merchant_b",
                    "payment_intent.succeeded",
                    "33333333-3333-3333-3333-333333333333",
                    "primary",
                    "https://merchant-b.example/webhooks-secret-path",
                    "exhausted",
                    base + day + day,
                ),
            ] {
                sqlx::query(
                    "INSERT INTO events (id, merchant_id, livemode, type, object_id, data, \
                     fanout_state, created_at) VALUES ($1, $2, false, $3, $1, $5, 'done', $4)",
                )
                .bind(event_id)
                .bind(merchant)
                .bind(event_type)
                .bind(created)
                .bind(serde_json::json!({}))
                .execute(&pool)
                .await
                .with_context(|| format!("seeding event {event_id}"))?;

                sqlx::query(
                    "INSERT INTO webhook_deliveries (id, event_id, endpoint_id, url, state, \
                     created_at) VALUES ($1::uuid, $2, $3, $4, $5, $6)",
                )
                .bind(delivery_id)
                .bind(event_id)
                .bind(endpoint)
                .bind(url_value)
                .bind(state)
                .bind(created)
                .execute(&pool)
                .await
                .with_context(|| format!("seeding delivery {delivery_id}"))?;
            }

            Ok((
                container,
                cratestack_schema::Cratestack::builder(pool).build(),
            ))
        }

        async fn search(
            cs: &cratestack_schema::Cratestack,
            ctx: &CratestackContext,
            page: PageInput,
            filter: types::WebhookDeliveryListFilter,
        ) -> Result<Page<types::WebhookDeliverySummary>, CratestackError> {
            use procedures::ProcedureRegistry as _;

            let args = procedures::search_webhook_deliveries::Args { page, filter };
            let authorized =
                procedures::search_webhook_deliveries::authorize_with_db(cs, &args, ctx).await?;
            super::super::super::search_payment_intents::Payments
                .search_webhook_deliveries(cs, ctx, args, authorized)
                .await
        }

        fn no_filter() -> types::WebhookDeliveryListFilter {
            types::WebhookDeliveryListFilter {
                state: None,
                event_type: None,
                created_gte: None,
                created_lte: None,
            }
        }

        /// **The decisive test.** `merchant_b`'s delivery — including the
        /// URL its endpoint is registered at — must not appear in
        /// `merchant_a`'s page. The mutation this test is written to catch:
        /// replacing `SEARCH_SQL`'s `WHERE e.merchant_id = $1` with a
        /// tautology (`WHERE TRUE`). Run by hand for this change and its
        /// output is reported in the commit message, not merely asserted
        /// here.
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
                .map(|item| item.event_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                ids,
                ["evt_a_new", "evt_a_old"],
                "newest first, and merchant_b's delivery is not merchant_a's business"
            );
            assert!(
                page.items
                    .iter()
                    .all(|item| item.merchant_id == "merchant_a"),
                "{ids:?}"
            );
            assert!(
                page.items
                    .iter()
                    .all(|item| !item.url.contains("merchant-b")),
                "merchant_b's endpoint URL must never appear in merchant_a's page: {:?}",
                page.items.iter().map(|item| &item.url).collect::<Vec<_>>()
            );
            assert_eq!(page.total_count, Some(2));

            // 2. The other tenant sees exactly its own one row through the
            //    same code path.
            let theirs = search(&cs, &b, PageInput::default(), no_filter())
                .await
                .context("merchant_b's default page")?;
            assert_eq!(
                theirs
                    .items
                    .iter()
                    .map(|item| item.event_id.as_str())
                    .collect::<Vec<_>>(),
                ["evt_b_only"]
            );

            // 3. A state only the *other* tenant has is an empty page for
            //    `merchant_a`, not that tenant's row.
            let exhausted = search(
                &cs,
                &a,
                PageInput::default(),
                types::WebhookDeliveryListFilter {
                    state: Some("exhausted".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("a state filter naming a state only merchant_b has")?;
            assert!(exhausted.items.is_empty(), "{:?}", exhausted.items);
            assert_eq!(exhausted.total_count, Some(0));

            // 4. `event_type`, through the join.
            let succeeded_type = search(
                &cs,
                &a,
                PageInput::default(),
                types::WebhookDeliveryListFilter {
                    event_type: Some("payment_intent.succeeded".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("an event_type filter that matches")?;
            assert_eq!(
                succeeded_type
                    .items
                    .iter()
                    .map(|item| item.event_id.as_str())
                    .collect::<Vec<_>>(),
                ["evt_a_new"]
            );
            assert_eq!(
                succeeded_type
                    .items
                    .first()
                    .map(|item| item.event_type.as_str()),
                Some("payment_intent.succeeded")
            );

            // 5. `response_excerpt` is absent from the response — checked
            //    against the actual answer this procedure gives, not only
            //    against the statement text.
            assert!(
                page.items.iter().all(|item| {
                    let json =
                        serde_json::to_value(item).expect("WebhookDeliverySummary serialises");
                    json.get("response_excerpt").is_none()
                }),
                "response_excerpt leaked into a serialised summary"
            );

            // 6. The clamp, against a real `LIMIT`.
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

            Ok(())
        }
    }
}
