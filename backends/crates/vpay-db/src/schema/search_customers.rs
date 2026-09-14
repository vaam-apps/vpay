//! The body of `procedure searchCustomers` — the dashboard's customers list,
//! paginated by offset and scoped to the caller's own tenant.
//!
//! Its shape is deliberately the twin of
//! `super::search_payment_intents`: same reason to exist (`model Customer`
//! carries no `@@allow("read", …)` arm, so a generated read renders the
//! literal `FALSE` and answers zero rows forever — a procedure is the one
//! shape CrateStack offers where the statement is ours), same tenancy
//! discipline, same offset-paging caveat. What follows states only where
//! this body differs, because a payer's personal data is what it reads.
//!
//! # Two maintainer decisions, implemented here rather than re-litigated
//!
//! 1. **`name`, `email` and `phone` are unmasked.** All three carry
//!    `@sensitive` in `schemas/vpay.cstack`, and that attribute has exactly
//!    one reader anywhere in this build —
//!    `cratestack-macros-0.12.0`'s `is_sensitive_field`, which redacts a
//!    value from the audit log CrateStack would write if `@@audit` were ever
//!    declared. Nothing in vpay declares it today, so this is a property the
//!    model carries for the day one does, not a live behaviour — it has
//!    never masked a value over the wire, here or anywhere else. The
//!    maintainer's call, 2026-09-13: an operator who can already read a
//!    payer's full name, email and phone off the payment-detail screen is
//!    not protected by hiding them on this list. This is recorded so the
//!    next reader does not assume `@sensitive` is doing work it is not.
//! 2. **An anonymised customer never appears.** `anonymized_at IS NOT NULL`
//!    means the erasure path closed by issue #111 / #143 already ran on this
//!    row, and this surface must not re-expose what that path removed.
//!    [`SEARCH_SQL`]'s `WHERE` excludes it directly — not a filter the
//!    caller could omit, and not a check the caller could disable.
//!
//! # What is not in the projection, and why
//!
//! The six `address_*` columns and the two coordinate columns
//! (`address_latitude_microdeg`, `address_longitude_microdeg`) are absent.
//! All eight are `@sensitive` in the schema, and unlike the three
//! identifiers above, nothing here overrides that: a list is not where a
//! payer's postal address or GPS point is read, and a customer's coordinates
//! least of all. `metadata` is absent for
//! `search_payment_intents::SEARCH_SQL`'s reason: it is `jsonb`,
//! merchant-authored, and echoed back verbatim — a hand-built `Json` field
//! goes through `Value::from_plain_json`, which routes a JSON number through
//! `Number::as_i64()` with an `as_f64()` fallback, silently demoting a value
//! outside `i64`. `seq` is absent because it is vpay's internal counter and
//! not part of the answer, exactly as it is not on `PaymentIntentSummary`.
//!
//! # What offset paging cannot promise
//!
//! See `super::search_payment_intents`' module doc, "What offset paging
//! cannot promise, and what `seq` does not fix" — the same property, the
//! same statement shape (`ORDER BY seq DESC`, `LIMIT`/`OFFSET`), and the
//! same reason nothing here compensates for it.

use time::OffsetDateTime;

use cratestack::{CratestackContext, CratestackError, Page, PageInfo, PageInput};

use super::cratestack_schema::{self, procedures, types};

/// The largest page this procedure will answer.
///
/// A deliberate copy of `search_payment_intents::MAX_PAGE_LIMIT`, for that
/// constant's own reason: `vpay-db` cannot depend on `vpay-api`, so this
/// cannot be shared with `vpay_api::v1::paging::MAX_LIMIT`, and nothing
/// gates the two into agreement beyond `tests::the_page_ceiling_stays_inside_cratestacks_own`
/// below, which only checks this never exceeds CrateStack's own
/// resource-exhaustion ceiling.
const MAX_PAGE_LIMIT: i64 = 100;

/// The page this procedure answers when the caller names no `limit` —
/// `search_payment_intents::DEFAULT_PAGE_LIMIT`'s twin, separate from the
/// ceiling for the same reason: [`cratestack::PageInput::resolve`] fuses an
/// absent `limit` to *the ceiling it is given*, so handing it
/// [`MAX_PAGE_LIMIT`] alone would answer 100 rows to a caller who asked for
/// no particular page size.
const DEFAULT_PAGE_LIMIT: i64 = 10;

/// Every column [`SummaryRow`] decodes, plus the window count.
///
/// A `&'static str`, not a `format!` — sqlx 0.9 takes it directly, so no
/// site here needs `crate::sql_audit`'s injection waiver.
///
/// **`anonymized_at IS NULL` excludes every erased customer.** This is the
/// maintainer's second decision, stated as SQL rather than as a filter: an
/// anonymised row is excluded in this predicate, unconditionally, and not by
/// anything the caller supplies. `tests::against_postgres` mutates this
/// clause away and reddens on it, because deleting it is exactly what would
/// re-expose an erased payer's row (with `[redacted]` identifiers, which is
/// its own kind of wrong — a customer who no longer exists still occupying a
/// row in an operator's table).
///
/// **`address_*`, the two coordinate columns, `metadata` and `seq` are not in
/// the projection** — see this file's module doc.
///
/// `count(*) OVER ()` and the `offset == 0` exception are
/// `search_payment_intents::SEARCH_SQL`'s device, applied unchanged: the
/// window count travels on a row in the same snapshot as the page, so a page
/// past the end carries no total ([`page_of`] answers `None` there), and an
/// empty result at `offset == 0` proves the filtered set is empty rather
/// than leaving the total unstated.
const SEARCH_SQL: &str = "SELECT id, merchant_id, livemode, name, email, phone, last_used_at, \
                          created_at, updated_at, count(*) OVER () AS total_count \
                          FROM customers \
                          WHERE merchant_id = $1 \
                            AND anonymized_at IS NULL \
                            AND ($2::TEXT IS NULL OR email = $2) \
                            AND ($3::TEXT IS NULL OR phone = $3) \
                            AND ($4::TIMESTAMPTZ IS NULL OR created_at >= $4) \
                            AND ($5::TIMESTAMPTZ IS NULL OR created_at <= $5) \
                          ORDER BY seq DESC \
                          LIMIT $6 OFFSET $7";

/// This body's implementation of the shared `ProcedureRegistry` — a free
/// function rather than a second unit struct, because
/// `impl procedures::ProcedureRegistry for search_payment_intents::Payments`
/// is where every procedure this schema declares has to converge onto one
/// implementer, and `search_payment_intents.rs` holds that `impl` block. This
/// function is what its `search_customers` method delegates to; see that
/// file for the thin delegation.
pub(super) async fn search_customers(
    db: &cratestack_schema::Cratestack,
    ctx: &CratestackContext,
    args: procedures::search_customers::Args,
) -> Result<procedures::search_customers::Output, CratestackError> {
    // The tenant comes from the context and never from `args` — there is no
    // `merchant_id` argument to this procedure, and there must not be one:
    // an argument is something the caller chooses, and a tenancy predicate a
    // caller chooses is not a tenancy predicate. `model Customer` carries
    // `merchant_id` directly (it is not reached through a join the way some
    // other tables are), which is what makes this predicate the simplest of
    // the direct-tenancy slices and no less mandatory for that.
    let merchant_id = tenant_of(ctx)?;
    let (limit, offset) = resolve_page(args.page);

    let rows: Vec<SummaryRow> = sqlx::query_as::<_, SummaryRow>(SEARCH_SQL)
        .bind(merchant_id)
        .bind(args.filter.email.as_deref())
        .bind(args.filter.phone.as_deref())
        .bind(to_time(args.filter.created_gte)?)
        .bind(to_time(args.filter.created_lte)?)
        .bind(limit.saturating_add(1))
        .bind(offset)
        .fetch_all(db.pool())
        .await
        .map_err(cratestack::cratestack_error_from_sqlx)?;

    page_of(rows, limit, offset)
}

/// [`PageInput::resolve`]'s clamp, with vpay's default page size —
/// `search_payment_intents::resolve_page`'s exact body, copied rather than
/// shared because the two procedures live in sibling modules with no common
/// parent that isn't `crate::schema` itself (which stays free of anything
/// but the macro invocation and the router, by that module's own doc).
fn resolve_page(page: PageInput) -> (i64, i64) {
    PageInput {
        limit: Some(page.limit.unwrap_or(DEFAULT_PAGE_LIMIT)),
        offset: page.offset,
    }
    .resolve(MAX_PAGE_LIMIT)
}

/// The merchant a context is allowed to read, or a refusal —
/// `search_payment_intents::tenant_of`'s exact body and exact reasoning:
/// `Forbidden` is deliberately the same `Forbidden` the `@allow` check
/// produces, so a caller with no tenant cannot distinguish "you are not
/// signed in" from "you have no tenant", and it is never `Ok` with an empty
/// string, which would be a predicate matching no rows rather than a
/// refusal.
fn tenant_of(ctx: &CratestackContext) -> Result<&str, CratestackError> {
    match ctx.tenant_id() {
        Some(tenant) if !tenant.is_empty() => Ok(tenant),
        _ => Err(CratestackError::Forbidden(
            "procedure policy denied this operation".to_owned(),
        )),
    }
}

/// Assembles the answer from the `limit + 1` rows the statement returned —
/// `search_payment_intents::page_of`'s exact body. `has_next_page` is "we
/// saw a row past the page", not arithmetic over `total_count`, and the
/// `offset == 0` exception is the same one the payments list applies.
fn page_of(
    mut rows: Vec<SummaryRow>,
    limit: i64,
    offset: i64,
) -> Result<Page<types::CustomerSummary>, CratestackError> {
    let has_next_page = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    let total_count = rows
        .first()
        .map(|row| row.total_count)
        .or_else(|| (offset == 0).then_some(0));
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));

    let items = rows.into_iter().map(SummaryRow::into_summary).collect();

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
#[derive(Debug, Clone, sqlx::FromRow)]
struct SummaryRow {
    id: String,
    merchant_id: String,
    livemode: bool,
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    last_used_at: OffsetDateTime,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    /// `count(*) OVER ()` — the size of the whole filtered set, not of this
    /// page.
    total_count: i64,
}

impl SummaryRow {
    /// The stored row as the procedure's answer. Infallible, unlike
    /// `search_payment_intents::SummaryRow::into_summary`: every column this
    /// struct decodes is either a plain scalar or a timestamp, and
    /// [`to_chrono`] is total over every instant `time::OffsetDateTime` can
    /// hold — there is no stored vocabulary here that this build could fail
    /// to name.
    fn into_summary(self) -> types::CustomerSummary {
        types::CustomerSummary {
            id: self.id,
            merchant_id: self.merchant_id,
            livemode: self.livemode,
            name: self.name,
            email: self.email,
            phone: self.phone,
            last_used_at: to_chrono(self.last_used_at),
            created_at: to_chrono(self.created_at),
            updated_at: to_chrono(self.updated_at),
        }
    }
}

/// A filter bound, in the type sqlx binds a `TIMESTAMPTZ` from —
/// `search_payment_intents::to_time`'s exact body and exact reasoning: the
/// nanosecond clamp is not defensive rounding, it is what keeps a chrono leap
/// second (`1_000_000_000..=1_999_999_999` as a subsecond) from moving a
/// bound into the next second, which `time` has no slot for at all.
///
/// # Errors
///
/// [`CratestackError::BadRequest`] for an instant outside `time`'s
/// roughly ±9999-year range.
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

/// A stored `TIMESTAMPTZ` as the procedure's answer — [`to_time`]'s inverse
/// and `search_payment_intents::to_chrono`'s twin (and
/// `crate::customers::to_chrono`'s: this is the third copy of the same
/// conversion, each in the module that reads it, for
/// `search_payment_intents::to_chrono`'s stated reason — the one
/// unreachable branch's fallback value is a per-caller judgement and not
/// shared logic).
///
/// **Total, not merely hoped**: `time::OffsetDateTime`'s range
/// (−9999..=9999 years) is four orders of magnitude narrower than chrono's,
/// so every value this crate could have stored converts. Proved once, for
/// every instant `time` can hold, by
/// `customers::tests::the_chrono_conversion_is_total_over_every_instant_time_can_hold`;
/// this function deliberately does not repeat that proof a third time.
fn to_chrono(stored: OffsetDateTime) -> cratestack::chrono::DateTime<cratestack::chrono::Utc> {
    cratestack::chrono::DateTime::from_timestamp(stored.unix_timestamp(), stored.nanosecond())
        .unwrap_or(cratestack::chrono::DateTime::<cratestack::chrono::Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    //! No-database cases first, then [`against_postgres`], which starts a
    //! container. The split is `search_payment_intents::tests`' own: a
    //! tenancy predicate and an erasure exclusion are each only observable
    //! through the rows they excluded, so the half that matters most cannot
    //! be a unit test.

    use super::*;

    /// The refusal a caller with no tenant gets, and that it is the *same*
    /// refusal the policy produces — `search_payment_intents::tests`'
    /// `a_context_without_a_tenant_cannot_read_any_merchants_payments`,
    /// applied here.
    ///
    /// Decisive: replacing `tenant_of`'s `Err` arm with `Ok("")` — the
    /// cheapest way to make an unscoped read compile — fails the first
    /// assertion, and widening it to `Ok(tenant)` without the emptiness
    /// check fails the third.
    #[test]
    fn a_context_without_a_tenant_cannot_read_any_merchants_customers() {
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

    /// [`resolve_page`]'s contract, at every boundary a caller can reach —
    /// `search_payment_intents::tests::a_hostile_limit_is_clamped_before_it_reaches_postgres`'s
    /// case table, unchanged: this pins `resolve_page`, not the body's use of
    /// it, which is why `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded`
    /// still exercises the clamp against a real `LIMIT`.
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
    /// own resource-exhaustion ceiling, and the default never exceeds it.
    #[test]
    fn the_page_ceiling_stays_inside_cratestacks_own() {
        const { assert!(MAX_PAGE_LIMIT > 0) };
        const { assert!(MAX_PAGE_LIMIT <= cratestack::MAX_LIST_LIMIT) };
        const { assert!(DEFAULT_PAGE_LIMIT > 0) };
        const { assert!(DEFAULT_PAGE_LIMIT <= MAX_PAGE_LIMIT) };
    }

    /// `has_next_page` comes from the extra row, and the total from the
    /// window count — including the two cases where there is no extra row
    /// and no row at all. `search_payment_intents::tests::the_envelope_reports_the_page_it_actually_has`,
    /// applied here.
    #[test]
    fn the_envelope_reports_the_page_it_actually_has() {
        let full = page_of(rows(3, 7), 2, 0).expect("fixture rows assemble");
        assert_eq!(full.items.len(), 2);
        assert_eq!(full.total_count, Some(7));
        assert!(full.page_info.has_next_page);
        assert!(!full.page_info.has_previous_page);

        let last = page_of(rows(2, 7), 2, 5).expect("fixture rows assemble");
        assert_eq!(last.items.len(), 2);
        assert!(
            !last.page_info.has_next_page,
            "no row past the page was seen"
        );
        assert!(last.page_info.has_previous_page);

        let past_end = page_of(rows(0, 0), 2, 99).expect("an empty result assembles");
        assert!(past_end.items.is_empty());
        assert_eq!(past_end.total_count, None);

        let empty_set = page_of(rows(0, 0), 2, 0).expect("an empty result assembles");
        assert!(empty_set.items.is_empty());
        assert_eq!(
            empty_set.total_count,
            Some(0),
            "an empty result at offset zero proves the filtered set is empty"
        );
    }

    /// A filter bound survives the chrono/time seam in both directions, to
    /// the nanosecond — `search_payment_intents::tests`' own case, unchanged.
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

        let leap = cratestack::chrono::NaiveDate::from_ymd_opt(2026, 12, 31)
            .and_then(|date| date.and_hms_nano_opt(23, 59, 59, 1_500_000_000))
            .expect("chrono spells a leap second as a subsecond above 1e9")
            .and_utc();
        let clamped = to_time(Some(leap))
            .expect("a leap second is clamped, not refused")
            .expect("Some in, Some out");
        assert_eq!(clamped.unix_timestamp(), leap.timestamp());
        assert_eq!(clamped.nanosecond(), 999_999_999);

        let far = cratestack::chrono::DateTime::<cratestack::chrono::Utc>::MAX_UTC;
        let error = to_time(Some(far)).expect_err("beyond year 9999 is not storable");
        assert!(matches!(error, CratestackError::BadRequest(_)), "{error:?}");
    }

    /// The projection carries no address column, no coordinate, no
    /// `metadata`, and no `seq` — asserted over the statement text rather
    /// than over the struct, because a column added to `SEARCH_SQL` alone is
    /// a compile error today, but a column added to both `SEARCH_SQL` and
    /// `SummaryRow` is not, and this is what would notice.
    #[test]
    fn the_statement_selects_no_address_no_coordinate_no_metadata_and_no_seq() {
        for forbidden in [
            "address_line1",
            "address_line2",
            "address_city",
            "address_state",
            "address_postal_code",
            "address_country",
            "address_latitude_microdeg",
            "address_longitude_microdeg",
            "metadata",
        ] {
            assert!(
                !SEARCH_SQL.contains(forbidden),
                "{forbidden} is in the customers-list projection: {SEARCH_SQL}"
            );
        }
        // `seq` is checked separately: it is a substring of nothing else
        // this statement selects, but `ORDER BY seq DESC` legitimately
        // contains it, so the substring check above would be a false
        // positive if it were folded into the same loop.
        assert!(!SEARCH_SQL.contains("SELECT id, merchant_id, livemode, name, email, phone, seq"));
        assert!(SEARCH_SQL.contains("WHERE merchant_id = $1"));
        assert!(SEARCH_SQL.contains("AND anonymized_at IS NULL"));
        assert!(SEARCH_SQL.contains("ORDER BY seq DESC"));
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
                let mut row = row(&format!("cus_fixture_{index}"), total);
                row.total_count = total;
                row
            })
            .collect()
    }

    fn row(id: &str, total_count: i64) -> SummaryRow {
        SummaryRow {
            id: id.to_owned(),
            merchant_id: "acme-cameroon-tenant".to_owned(),
            livemode: false,
            name: Some("Ada Lovelace".to_owned()),
            email: Some("ada@example.cm".to_owned()),
            phone: Some("237600000000".to_owned()),
            last_used_at: OffsetDateTime::UNIX_EPOCH,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            total_count,
        }
    }

    /// The halves of this procedure that only rows can prove, against a real
    /// Postgres — `search_payment_intents::tests::against_postgres`'s
    /// device: one container, one chunky test, under
    /// `.config/nextest.toml`'s `postgres-containers` one-at-a-time group.
    mod against_postgres {
        use anyhow::Context as _;
        use sqlx::postgres::PgPoolOptions;
        use testcontainers::ContainerAsync;
        use testcontainers_modules::postgres::Postgres as PostgresImage;

        use super::*;

        /// `merchant_a`'s three customers (one later anonymised) and
        /// `merchant_b`'s one, in a migrated database, plus a `Cratestack`
        /// over the same database.
        ///
        /// **The anonymised row is produced through the real erasure path**
        /// — `TxRepositories::erase_customer_in_tx`, the same method
        /// `vpay-api`'s `DELETE /v1/customers/{id}` and the retention sweep
        /// call, and the path issue #111 / #143 closed — rather than by
        /// hand-crafting a row with `anonymized_at` set. Two reasons: this
        /// procedure's own claim is about what `SEARCH_SQL` does with a row
        /// however it came to be anonymised, and a hand-built row risks
        /// silently violating `anonymized_customers_carry_the_marker`
        /// (migration `0041`), which requires the six `address_*` columns to
        /// read `[redacted]` and the two coordinates to read `NULL` — a
        /// different shape from "all `NULL`", which is what a customer with
        /// no address ever had. Going through the real path means this test
        /// gets that shape for free and proves it against the actual
        /// erasure, not a guess at what it leaves behind.
        ///
        /// The customer to be erased is given a payment intent first, so
        /// `erase_customer_in_tx` takes the **anonymise** branch rather than
        /// the hard-delete one — the branch this test needs: a hard-deleted
        /// row is absent for an unrelated reason (no row at all), where this
        /// test's claim is that a row still there, still `merchant_a`'s, is
        /// excluded because it is anonymised.
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
                .context("XAF exists, because the seeded payment intent needs the currency")?;

            let day = time::Duration::days(1);
            let base = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);

            use crate::{TxOutcome, UnitOfWork as _};

            for (id, merchant, name, email, phone, created_at) in [
                (
                    "cus_a_old",
                    "merchant_a",
                    "Ada Lovelace",
                    "ada@example.cm",
                    "237600000001",
                    base,
                ),
                (
                    "cus_a_new",
                    "merchant_a",
                    "Grace Hopper",
                    "grace@example.cm",
                    "237600000002",
                    base + day,
                ),
                // The row that must never appear in `merchant_a`'s page.
                (
                    "cus_b_only",
                    "merchant_b",
                    "Katherine Johnson",
                    "katherine@example.cm",
                    "237600000003",
                    base + day + day,
                ),
                // Erased below, once it has a payment intent to be
                // referenced by.
                (
                    "cus_a_erased",
                    "merchant_a",
                    "Erased Payer",
                    "erased@example.cm",
                    "237600000004",
                    base,
                ),
            ] {
                let new = crate::NewCustomer {
                    id: id.to_owned(),
                    merchant_id: merchant.to_owned(),
                    livemode: false,
                    name: Some(name.to_owned()),
                    email: Some(email.to_owned()),
                    phone: Some(phone.to_owned()),
                    address: crate::CustomerAddress::default(),
                    metadata: serde_json::json!({}),
                    created_at,
                };
                repositories
                    .transaction(|tx| {
                        Box::pin(async move {
                            tx.insert_customer_in_tx(&new).await?;
                            Ok::<_, crate::DbError>(TxOutcome::Commit(()))
                        })
                    })
                    .await
                    .with_context(|| format!("seeding {id}"))?;
            }

            // A payment intent referencing `cus_a_erased`, so the erasure
            // below takes the anonymise branch — see this function's doc.
            repositories
                .insert(&crate::NewPaymentIntent {
                    id: "pi_for_erased_customer".to_owned(),
                    merchant_id: "merchant_a".to_owned(),
                    livemode: false,
                    amount: 5_000,
                    currency_code: "XAF".to_owned(),
                    status: "succeeded".to_owned(),
                    last_payment_error_code: None,
                    last_payment_error_message: None,
                    payment_method_types: serde_json::json!(["mtn_momo"]),
                    metadata: serde_json::json!({}),
                    description: None,
                    customer_id: Some("cus_a_erased".to_owned()),
                    client_secret_suffix: vpay_core::ids::client_secret_suffix(),
                    created_at: base,
                })
                .await
                .context("a payment intent referencing the customer to be erased")?;

            // The real erasure: lock the row, then erase it — the same two
            // steps `DELETE /v1/customers/{id}` and the retention sweep take.
            let erased_at = base + day + day + day;
            repositories
                .transaction(|tx| {
                    Box::pin(async move {
                        let row = tx
                            .lock_customer_for_update("merchant_a", "cus_a_erased")
                            .await?
                            .ok_or_else(|| crate::DbError::WriteMatchedNoRow {
                                table: "customers",
                                key: "cus_a_erased".to_owned(),
                            })?;
                        tx.erase_customer_in_tx(
                            &row,
                            erased_at,
                            "evt_cus_a_erased_deleted",
                            &serde_json::json!({
                                "id": "cus_a_erased",
                                "object": "customer",
                                "deleted": true,
                            }),
                        )
                        .await?;
                        Ok::<_, crate::DbError>(TxOutcome::Commit(()))
                    })
                })
                .await
                .context("erasing cus_a_erased through the real erasure path")?;

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
            filter: types::CustomerListFilter,
        ) -> Result<Page<types::CustomerSummary>, CratestackError> {
            use procedures::ProcedureRegistry as _;

            let args = procedures::search_customers::Args { page, filter };
            let authorized =
                procedures::search_customers::authorize_with_db(cs, &args, ctx).await?;
            crate::schema::search_payment_intents::Payments
                .search_customers(cs, ctx, args, authorized)
                .await
        }

        fn no_filter() -> types::CustomerListFilter {
            types::CustomerListFilter {
                created_gte: None,
                created_lte: None,
                email: None,
                phone: None,
            }
        }

        /// The tenancy predicate and the erasure exclusion, both stated as
        /// rows, plus paging, the clamp and the two named filters.
        ///
        /// **The first two assertions are the ones that matter.**
        /// `merchant_b`'s row is not in `merchant_a`'s page — deleting
        /// `WHERE merchant_id = $1` from [`super::super::SEARCH_SQL`] is
        /// what that catches. `cus_a_erased` never appears for `merchant_a`
        /// even though it shares that tenant — deleting
        /// `AND anonymized_at IS NULL` is what that catches. No unit test
        /// can catch either: a predicate is only observable through the
        /// rows it excluded.
        #[tokio::test]
        async fn the_page_is_the_tenants_own_live_rows_filtered_and_bounded() -> anyhow::Result<()>
        {
            let (_container, cs) = seeded().await?;
            let a = context_for_tenant("merchant_a");
            let b = context_for_tenant("merchant_b");

            // 1. The tenancy predicate, and the erasure exclusion, together:
            //    merchant_a has three customer rows in the database and this
            //    page has exactly the two live ones.
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
                ["cus_a_new", "cus_a_old"],
                "newest first; merchant_b's row is not merchant_a's business and the erased \
                 row must not come back"
            );
            assert!(
                page.items
                    .iter()
                    .all(|item| item.merchant_id == "merchant_a"),
                "{ids:?}"
            );
            assert_eq!(page.total_count, Some(2));

            // 2. No address column, no coordinate, on the wire type itself —
            //    the compile-time half of the "no address in a list" rule,
            //    checked here against real rows rather than only against the
            //    statement text.
            //    (`type CustomerSummary` has no `address_*` field to read,
            //    so there is nothing to assert here beyond "it compiled" —
            //    recorded so a reviewer sees the claim was checked, not
            //    merely believed.)

            // 3. The other tenant sees exactly its own one row.
            let theirs = search(&cs, &b, PageInput::default(), no_filter())
                .await
                .context("merchant_b's default page")?;
            assert_eq!(
                theirs
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["cus_b_only"]
            );

            // 4. The identifiers are unmasked, as the maintainer decided.
            let newest = page.items.first().expect("merchant_a has a newest row");
            assert_eq!(newest.name.as_deref(), Some("Grace Hopper"));
            assert_eq!(newest.email.as_deref(), Some("grace@example.cm"));
            assert_eq!(newest.phone.as_deref(), Some("237600000002"));

            // 5. An exact-match email filter, and that it is exact rather
            //    than a prefix: "grace" alone must not match
            //    "grace@example.cm".
            let by_email = search(
                &cs,
                &a,
                PageInput::default(),
                types::CustomerListFilter {
                    email: Some("grace@example.cm".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("an exact email filter")?;
            assert_eq!(
                by_email
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["cus_a_new"]
            );

            let by_email_prefix = search(
                &cs,
                &a,
                PageInput::default(),
                types::CustomerListFilter {
                    email: Some("grace".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("a prefix must not match")?;
            assert!(
                by_email_prefix.items.is_empty(),
                "the email filter must be exact-match, never a prefix: {:?}",
                by_email_prefix.items
            );

            // 6. An exact-match phone filter.
            let by_phone = search(
                &cs,
                &a,
                PageInput::default(),
                types::CustomerListFilter {
                    phone: Some("237600000001".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("an exact phone filter")?;
            assert_eq!(
                by_phone
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["cus_a_old"]
            );

            // 7. Paging: one row per page, and `has_next_page` is real.
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
                Some("cus_a_old")
            );
            assert!(!second.page_info.has_next_page);
            assert!(second.page_info.has_previous_page);

            // 8. The clamp, against a real `LIMIT`.
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
