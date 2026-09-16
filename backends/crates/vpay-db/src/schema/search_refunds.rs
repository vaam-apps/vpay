//! The body of `procedure searchRefunds` — the dashboard's refunds list,
//! paginated by offset and scoped to the caller's own tenant.
//!
//! `search_payment_intents.rs` is the template this follows: the module doc
//! there says what a procedure is for and why a *generated* read cannot
//! serve `refunds` (`model Refund` carries no `@@allow` arm, so
//! `db.refund().find_many()` renders the literal `FALSE` and answers zero
//! rows, forever). This file does not repeat that argument; it only states
//! where this body diverges from it.
//!
//! # The one place this MUST diverge: the tenancy predicate is a JOIN
//!
//! `model Refund` has **no `merchant_id` column at all**
//! (`docs/plans/2026-09-13-dashboard-nav-notes/slices.md` § 2). Tenancy
//! reaches a refund only through `payment_intent_id ->
//! PaymentIntent.merchant_id`, exactly as `vpay_db::refunds`'
//! `get_for_merchant` and `list_for_intent` already join rather than filter
//! a column of their own — this body uses the same shape, for the same
//! reason. `searchPaymentIntents` gets to write `WHERE merchant_id = $1`
//! because its table has the column; [`SEARCH_SQL`] below cannot, and a
//! body that forgot the join would not fail — it would answer every
//! tenant's refunds. No unit test can catch that: a predicate is only
//! observable through the rows it excluded, which is what
//! `against_postgres::the_page_is_the_tenants_own_rows_reached_through_the_join`
//! seeds a second tenant to prove.
//!
//! # What else is reused from the template, and why
//!
//! [`super::tenant_of`], [`super::resolve_page`], [`super::to_time`] and
//! [`super::to_chrono`] are called here rather than copied: none of the four
//! has any payment-intent-specific behaviour in it — the tenancy refusal,
//! the page-size clamp and the chrono/time seam are properties of this
//! procedure's *shape* (an authenticated, offset-paged, timestamp-filtered
//! list), not of the table underneath it. Copying them would be a second
//! place for the clamp's numbers, [`super::MAX_PAGE_LIMIT`] and
//! [`super::DEFAULT_PAGE_LIMIT`], to drift out of step with `/dash/v1`'s
//! page sizes — the exact hazard `search_payment_intents.rs` documents for
//! why they are not `cratestack::MAX_LIST_LIMIT` itself. What is NOT reused
//! is the SQL, the row projection and the status vocabulary: those are
//! genuinely refunds' own.
//!
//! # `failure_raw` is not in the projection
//!
//! `Refund.failure_raw` is `@sensitive`, up to 2 000 characters of a rail's
//! own prose, and is not selected by [`SEARCH_SQL`] and not a field of
//! `types::RefundSummary` — `schemas/vpay.cstack`'s `type RefundSummary`
//! comment says why, in the same words `PaymentIntentSummary`'s does for
//! `last_payment_error_message`: "a list is not where it is read".
//! `tests::the_statement_selects_no_sensitive_or_jsonb_column` pins the
//! statement text as well as the struct, for `search_payment_intents.rs`'s
//! reason: a column added to the SQL alone is a compile error today, but a
//! column added to both is not.

use std::str::FromStr as _;

use time::OffsetDateTime;
use vpay_core::RefundStatus;

use super::{
    CratestackContext, CratestackError, Page, PageInfo, cratestack_schema, procedures,
    resolve_page, tenant_of, to_chrono, to_time, types,
};

/// Every column [`RefundSummaryRow`] decodes, plus the window count.
///
/// `refunds` carries no `merchant_id`, so the tenancy predicate is a JOIN
/// onto `payment_intents` rather than a `WHERE` on a column of its own —
/// see this file's module doc. Every selected column is table-qualified
/// (`r.…`) because the join makes `id` ambiguous otherwise, matching
/// `vpay_db::refunds::COLUMNS`'s own convention.
///
/// `failure_raw` and `metadata` are not here — see the module doc — and
/// neither is `seq`, because `model Refund` has none: unlike
/// `payment_intents` and `checkout_sessions`, migration `0017` gave this
/// table no `GENERATED ALWAYS AS IDENTITY` column, so there is no single
/// column that is both a total order and safe to hand out. `ORDER BY
/// r.created_at DESC, r.id DESC` is the total order used instead: `id` is
/// the table's `PRIMARY KEY`, so appending it as a tiebreaker makes the
/// order total even when two refunds share a `created_at` — the same
/// tiebreak `vpay_db::refunds::list_for_intent` uses (ascending, there,
/// for a timeline rather than a page). Without the tiebreaker, two refunds
/// created in the same statement could swap places between one page and
/// the next the way `search_payment_intents.rs`'s module doc describes for
/// offset paging in general.
const SEARCH_SQL: &str = "SELECT r.id, r.payment_intent_id, r.charge_id, r.amount, \
                          r.currency_code, r.status, r.reason, r.failure_code, \
                          r.provider_reference_id, r.fee, r.created_at, r.updated_at, \
                          count(*) OVER () AS total_count \
                          FROM refunds r \
                          JOIN payment_intents pi ON pi.id = r.payment_intent_id \
                          WHERE pi.merchant_id = $1 \
                            AND ($2::TEXT IS NULL OR r.status = $2) \
                            AND ($3::TIMESTAMPTZ IS NULL OR r.created_at >= $3) \
                            AND ($4::TIMESTAMPTZ IS NULL OR r.created_at <= $4) \
                          ORDER BY r.created_at DESC, r.id DESC \
                          LIMIT $5 OFFSET $6";

/// The body `Payments::search_refunds` delegates to — see that method for
/// why this is a free function rather than a second trait impl: the
/// `ProcedureRegistry` trait itself lives in `search_payment_intents.rs`'s
/// private expansion, so this file cannot implement it directly.
///
/// # Errors
///
/// See [`tenant_of`], [`validated_status`] and [`RefundSummaryRow::into_summary`]
/// for the three ways this refuses.
pub(super) async fn run(
    db: &cratestack_schema::Cratestack,
    ctx: &CratestackContext,
    args: procedures::search_refunds::Args,
    _authorized: procedures::search_refunds::Authorized,
) -> Result<procedures::search_refunds::Output, CratestackError> {
    // The tenant is read from the context and never from `args` — there is
    // no `merchant_id` argument to this procedure, for
    // `search_payment_intents.rs`'s reason: an argument is something the
    // caller chooses, and a tenancy predicate a caller chooses is not a
    // tenancy predicate.
    let merchant_id = tenant_of(ctx)?;
    let status = validated_status(args.filter.status.as_deref())?;
    let (limit, offset) = resolve_page(args.page);

    // `limit + 1`, the same trick `search_payment_intents.rs` uses: one row
    // past the page is how `has_next_page` is learned without a second
    // statement.
    let rows: Vec<RefundSummaryRow> = sqlx::query_as::<_, RefundSummaryRow>(SEARCH_SQL)
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

/// Refuses a `status` filter outside [`RefundStatus`]'s vocabulary.
///
/// A `400` rather than an empty page, for `search_payment_intents.rs`'s
/// `validated_status`'s reason: the column is `TEXT`
/// (`refunds_status_enum_check`, migration `0037`), so an unknown label is a
/// perfectly valid query returning zero rows, and "you have no refunds in
/// that status" is the answer an operator would read as being about their
/// refunds rather than about their typo.
///
/// The vocabulary is asked of both owners and they must agree:
/// `vpay_core::RefundStatus` is what parses on this path, and
/// `cratestack_schema::types::RefundStatus` is what `schemas/vpay.cstack`
/// declares. `tests::the_two_refund_status_vocabularies_are_one_vocabulary`
/// fails if they ever diverge.
fn validated_status(raw: Option<&str>) -> Result<Option<String>, CratestackError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if types::RefundStatus::from_str(raw).is_ok() {
        return Ok(Some(raw.to_owned()));
    }
    Err(CratestackError::BadRequest(format!(
        "status: unknown refund status. Known statuses are: {}.",
        RefundStatus::ALL
            .iter()
            .map(|status| status.as_wire_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Assembles the answer from the `limit + 1` rows the statement returned.
///
/// Identical in shape to `search_payment_intents.rs`'s `page_of` — same
/// `has_next_page`-from-the-extra-row rule, same `offset == 0` exception for
/// an empty filtered set — and not shared with it, because the two operate
/// on different row and item types and CrateStack's generated `Page<T>` and
/// `PageInfo` give no generic seam to share through without one of the two
/// procedures reaching into the other's module.
fn page_of(
    mut rows: Vec<RefundSummaryRow>,
    limit: i64,
    offset: i64,
) -> Result<Page<types::RefundSummary>, CratestackError> {
    let has_next_page = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    let total_count = rows
        .first()
        .map(|row| row.total_count)
        .or_else(|| (offset == 0).then_some(0));
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));

    let items = rows
        .into_iter()
        .map(RefundSummaryRow::into_summary)
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
/// `status` and `failure_code` arrive as `String`, for
/// `search_payment_intents.rs::SummaryRow`'s reason: this crate carries both
/// vocabularies as text and parses them once, in [`Self::into_summary`], at
/// the point where the value becomes part of an answer.
#[derive(Debug, Clone, sqlx::FromRow)]
struct RefundSummaryRow {
    id: String,
    payment_intent_id: String,
    charge_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    reason: Option<String>,
    failure_code: Option<String>,
    provider_reference_id: Option<cratestack::uuid::Uuid>,
    fee: Option<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    /// `count(*) OVER ()` — the size of the whole filtered set, not of this
    /// page. See `search_payment_intents.rs::SummaryRow::total_count`.
    total_count: i64,
}

impl RefundSummaryRow {
    /// The stored row as the procedure's answer.
    ///
    /// # Errors
    ///
    /// [`CratestackError::Internal`] when the database holds a `status` or a
    /// `failure_code` outside the vocabulary this build declares — the
    /// database and the binary have drifted, which is an operator's problem
    /// and not a caller's, so the caller is told nothing beyond "internal
    /// error" while the detail reaches the log. Rendering a default status
    /// instead would put a refund in front of an operator under a state it
    /// is not in.
    fn into_summary(self) -> Result<types::RefundSummary, CratestackError> {
        let status = types::RefundStatus::from_str(&self.status).map_err(|error| {
            CratestackError::Internal(format!(
                "refunds.status for {} is outside this build's vocabulary: {error}",
                self.id
            ))
        })?;
        let failure_code = self
            .failure_code
            .as_deref()
            .map(|code| {
                types::FailureCode::from_str(code).map_err(|error| {
                    CratestackError::Internal(format!(
                        "refunds.failure_code for {} is outside this build's vocabulary: {error}",
                        self.id
                    ))
                })
            })
            .transpose()?;

        Ok(types::RefundSummary {
            id: self.id.clone(),
            payment_intent_id: self.payment_intent_id,
            charge_id: self.charge_id,
            amount: self.amount,
            currency_code: self.currency_code,
            status,
            reason: self.reason,
            failure_code,
            provider_reference_id: self.provider_reference_id,
            fee: self.fee,
            created_at: to_chrono(self.created_at),
            updated_at: to_chrono(self.updated_at),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Follows `search_payment_intents.rs::tests`'s split: unit cases with no
    //! database, plus one container-backed case in [`against_postgres`] for
    //! the one property only rows can prove — the tenancy JOIN.

    use cratestack::PageInput;

    use super::*;

    /// An unknown status is a refusal naming `status`, never an empty page.
    ///
    /// Decisive: making `validated_status` return `Ok(Some(raw))` for
    /// anything fails on the first unknown value.
    #[test]
    fn an_unknown_status_is_refused_rather_than_answered_with_an_empty_page() {
        for status in RefundStatus::ALL {
            assert_eq!(
                validated_status(Some(status.as_wire_str())).ok(),
                Some(Some(status.as_wire_str().to_owned())),
            );
        }

        for unknown in ["succeded", "SUCCEEDED", "", "pending ", "'; DROP TABLE"] {
            let error = validated_status(Some(unknown))
                .expect_err("an unknown status must be refused, not answered with an empty list");
            assert!(
                matches!(error, CratestackError::BadRequest(_)),
                "{unknown:?} produced {error:?}"
            );
            let message = error.public_message().into_owned();
            assert!(message.starts_with("status:"), "{message}");
            assert!(message.contains("pending"), "{message}");
        }

        assert_eq!(validated_status(None).ok(), Some(None));
    }

    /// `schemas/vpay.cstack`'s `enum RefundStatus` and
    /// `vpay_core::RefundStatus` are one vocabulary, in both directions —
    /// `search_payment_intents.rs`'s
    /// `the_two_intent_status_vocabularies_are_one_vocabulary` for the same
    /// reason and the same admitted limit (a variant added to the schema
    /// alone still escapes both halves of this test).
    #[test]
    fn the_two_refund_status_vocabularies_are_one_vocabulary() {
        for status in RefundStatus::ALL {
            let parsed = types::RefundStatus::from_str(status.as_wire_str())
                .unwrap_or_else(|error| panic!("{}: {error}", status.as_wire_str()));
            assert_eq!(parsed.as_str(), status.as_wire_str());
        }

        for label in ["pending", "succeeded", "failed", "canceled"] {
            assert!(
                RefundStatus::from_wire(label).is_some(),
                "the schema declares {label}, vpay-core does not"
            );
            assert!(types::RefundStatus::from_str(label).is_ok(), "{label}");
        }
        assert_eq!(RefundStatus::ALL.len(), 4);
    }

    /// `has_next_page` comes from the extra row, and the total from the
    /// window count — including the two cases where there is no extra row
    /// and no row at all. Mirrors
    /// `search_payment_intents.rs::the_envelope_reports_the_page_it_actually_has`.
    #[test]
    fn the_envelope_reports_the_page_it_actually_has() {
        let full = page_of(rows(3, 7), 2, 0).expect("fixture rows assemble");
        assert_eq!(full.items.len(), 2);
        assert_eq!(full.total_count, Some(7));
        assert!(full.page_info.has_next_page);
        assert!(!full.page_info.has_previous_page);

        let last = page_of(rows(2, 7), 2, 5).expect("fixture rows assemble");
        assert_eq!(last.items.len(), 2);
        assert!(!last.page_info.has_next_page);
        assert!(last.page_info.has_previous_page);

        let past_end = page_of(rows(0, 0), 2, 99).expect("an empty result assembles");
        assert!(past_end.items.is_empty());
        assert_eq!(past_end.total_count, None);

        // An empty FIRST page is a different fact — see
        // `search_payment_intents.rs`'s identical exp54-review case.
        let empty_set = page_of(rows(0, 0), 2, 0).expect("an empty result assembles");
        assert!(empty_set.items.is_empty());
        assert_eq!(empty_set.total_count, Some(0));
    }

    /// A stored status this build cannot name is an error, not a default.
    #[test]
    fn a_row_whose_status_this_build_cannot_name_is_an_error_not_a_default() {
        let mut unknown_status = row("re_unknown_status", 1);
        unknown_status.status = "settled".to_owned();
        let error = unknown_status
            .into_summary()
            .expect_err("a status outside the vocabulary must not be rendered as another status");
        assert!(matches!(error, CratestackError::Internal(_)), "{error:?}");
        assert_eq!(error.public_message(), "internal error");

        let mut unknown_failure = row("re_unknown_failure", 1);
        unknown_failure.failure_code = Some("gremlins".to_owned());
        assert!(unknown_failure.into_summary().is_err());
    }

    /// The projection carries no `@sensitive` prose column and no `jsonb`
    /// column. Asserted over the statement text, matching
    /// `search_payment_intents.rs`'s
    /// `the_statement_selects_no_credential_and_no_jsonb_column`: a column
    /// added to `SEARCH_SQL` alone is a compile error today, but a column
    /// added to both the SQL and the type is not.
    #[test]
    fn the_statement_selects_no_sensitive_or_jsonb_column() {
        for forbidden in ["failure_raw", "metadata"] {
            assert!(
                !SEARCH_SQL.contains(forbidden),
                "{forbidden} is in the refunds-list projection: {SEARCH_SQL}"
            );
        }
        assert!(SEARCH_SQL.contains("WHERE pi.merchant_id = $1"));
        assert!(SEARCH_SQL.contains("JOIN payment_intents pi ON pi.id = r.payment_intent_id"));
        assert!(SEARCH_SQL.contains("ORDER BY r.created_at DESC, r.id DESC"));
    }

    /// `count` rows, each claiming the filtered set holds `total`.
    fn rows(count: usize, total: i64) -> Vec<RefundSummaryRow> {
        (0..count)
            .map(|index| {
                let mut row = row(&format!("re_fixture_{index}"), total);
                row.total_count = total;
                row
            })
            .collect()
    }

    fn row(id: &str, total_count: i64) -> RefundSummaryRow {
        RefundSummaryRow {
            id: id.to_owned(),
            payment_intent_id: "pi_fixture".to_owned(),
            charge_id: None,
            amount: 500,
            currency_code: "XAF".to_owned(),
            status: RefundStatus::Pending.as_wire_str().to_owned(),
            reason: None,
            failure_code: None,
            provider_reference_id: None,
            fee: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            total_count,
        }
    }

    /// The halves of this procedure that only rows can prove, against a real
    /// Postgres. One container, one test — `search_payment_intents.rs`'s
    /// `against_postgres` module doc gives the reason (one seed, several
    /// questions, one container start).
    mod against_postgres {
        use anyhow::Context as _;
        use sqlx::postgres::PgPoolOptions;
        use testcontainers::ContainerAsync;
        use testcontainers_modules::postgres::Postgres as PostgresImage;

        use super::*;

        /// `merchant_a`'s intent with two refunds, and `merchant_b`'s intent
        /// with one, in a migrated database, plus a `Cratestack` over the
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

            let day = time::Duration::days(1);
            let base = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
            for (id, merchant) in [("pi_a", "merchant_a"), ("pi_b", "merchant_b")] {
                repositories
                    .insert(&crate::NewPaymentIntent {
                        id: id.to_owned(),
                        merchant_id: merchant.to_owned(),
                        livemode: false,
                        amount: 5_000,
                        currency_code: "XAF".to_owned(),
                        status: "succeeded".to_owned(),
                        last_payment_error_code: None,
                        last_payment_error_message: None,
                        payment_method_types: serde_json::json!(["mtn_momo"]),
                        metadata: serde_json::json!({}),
                        description: None,
                        customer_id: None,
                        client_secret_suffix: vpay_core::ids::client_secret_suffix(),
                        created_at: base,
                    })
                    .await
                    .with_context(|| format!("seeding {id}"))?;
            }

            // Raw inserts, still, and by choice rather than by necessity:
            // this comment read "this crate's `refunds` module deliberately
            // has no `create`" until 2026-09-15, when RFC-0003 section 3
            // added `Refunds::create`. That writer needs a `succeeded`
            // intent with a charge behind it and reserves against the intent
            // as it writes, which is a fixture this read-only procedure has
            // no use for and a second thing whose failure could fail these
            // cases. What the procedure is measured on is the rows it
            // returns, so the rows go in by statement.
            let pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .context(
                    "a second pool, for both the seed inserts and the procedure's own \
                          Cratestack",
                )?;

            for (id, intent_id, status, amount, created_at) in [
                ("re_a_old", "pi_a", "pending", 100_i64, base),
                ("re_a_new", "pi_a", "succeeded", 200_i64, base + day),
                // The row that must never appear in `merchant_a`'s page.
                ("re_b_only", "pi_b", "failed", 300_i64, base + day + day),
            ] {
                sqlx::query(
                    "INSERT INTO refunds (id, payment_intent_id, amount, currency_code, status, \
                     created_at, updated_at) VALUES ($1, $2, $3, 'XAF', $4, $5, $5)",
                )
                .bind(id)
                .bind(intent_id)
                .bind(amount)
                .bind(status)
                .bind(created_at)
                .execute(&pool)
                .await
                .with_context(|| format!("seeding {id}"))?;
            }

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
            filter: types::RefundListFilter,
        ) -> Result<Page<types::RefundSummary>, CratestackError> {
            let args = procedures::search_refunds::Args { page, filter };
            let authorized = procedures::search_refunds::authorize_with_db(cs, &args, ctx).await?;
            super::super::run(cs, ctx, args, authorized).await
        }

        fn no_filter() -> types::RefundListFilter {
            types::RefundListFilter {
                status: None,
                created_gte: None,
                created_lte: None,
            }
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

        /// The decisive question: **`merchant_b`'s refund is not in
        /// `merchant_a`'s page**, reached only through the JOIN onto
        /// `payment_intents`. Deleting or replacing `WHERE pi.merchant_id =
        /// $1` in [`super::SEARCH_SQL`] with a tautology is what this
        /// catches — no unit test can, because the predicate is only
        /// observable through the row it excluded.
        #[tokio::test]
        async fn the_page_is_the_tenants_own_rows_reached_through_the_join() -> anyhow::Result<()> {
            let (_container, cs) = seeded().await?;
            let a = context_for_tenant("merchant_a");
            let b = context_for_tenant("merchant_b");

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
                ["re_a_new", "re_a_old"],
                "newest first, and merchant_b's refund is not merchant_a's business"
            );
            assert!(
                page.items
                    .iter()
                    .all(|item| item.payment_intent_id == "pi_a"),
                "{ids:?}"
            );
            assert_eq!(page.total_count, Some(2));
            assert!(!page.page_info.has_next_page);
            assert!(!page.page_info.has_previous_page);

            // The other tenant sees exactly its own one row through the same
            // code path — so the assertion above is about the predicate and
            // not about an empty table.
            let theirs = search(&cs, &b, PageInput::default(), no_filter())
                .await
                .context("merchant_b's default page")?;
            assert_eq!(
                theirs
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["re_b_only"]
            );

            // `failure_raw` cannot appear: `types::RefundSummary` has no
            // such field, so this is a compile-time property as much as a
            // runtime one — this assertion is here so the container test
            // reads as proof of the whole slice's contract, not only the
            // join.
            let status_filter = search(
                &cs,
                &a,
                PageInput::default(),
                types::RefundListFilter {
                    status: Some("failed".to_owned()),
                    ..no_filter()
                },
            )
            .await
            .context("a status only the other tenant has")?;
            assert!(status_filter.items.is_empty(), "{:?}", status_filter.items);
            assert_eq!(status_filter.total_count, Some(0));

            // The clamp, against a real `LIMIT`/`OFFSET`.
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
                    limit: Some(super::super::super::MAX_PAGE_LIMIT + 1),
                    offset: None,
                },
                no_filter(),
            )
            .await
            .context("an oversized limit must be clamped, not sent")?;
            assert_eq!(
                oversized.page_info.limit,
                Some(super::super::super::MAX_PAGE_LIMIT)
            );

            Ok(())
        }
    }
}
