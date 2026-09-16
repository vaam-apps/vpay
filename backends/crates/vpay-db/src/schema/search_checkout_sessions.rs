//! The body of `procedure searchCheckoutSessions` — the dashboard's checkout
//! sessions list, paginated by offset and scoped to the caller's own tenant.
//!
//! `super::search_customers`' sibling: `model CheckoutSession` carries
//! `merchant_id` directly, so the tenancy predicate is a plain `WHERE` rather
//! than the join `search_refunds` and `search_webhook_deliveries` need. It
//! also carries a `seq` with a UNIQUE index (migration 0028), so
//! `ORDER BY seq DESC` is a total order — one statement never breaks a tie
//! and never returns a row twice.
//!
//! # The one rule that outranks everything else here
//!
//! **`client_secret_suffix` and `return_token` never leave the database.**
//! Neither is in [`SEARCH_SQL`]'s projection, neither is a field on
//! `types::CheckoutSessionSummary`, and neither is filterable. Both are live
//! payer credentials: the client secret authorises the payer's browser
//! against this checkout and the return token authorises the return leg, so
//! a list carrying either hands them to everyone who can read the list.
//!
//! `@sensitive` on both columns gives this **no** protection and must not be
//! mistaken for it — `cratestack-macros`' `is_sensitive_field` is read in one
//! place, to redact a value from the audit log. It excludes nothing from a
//! response. `tests::against_postgres::neither_payer_credential_is_in_the_projection`
//! is what actually holds the line, by reading the statement's own text and
//! the answered rows rather than trusting either declaration.
//!
//! # What offset paging cannot promise
//!
//! `search_payment_intents`' note applies unchanged: `OFFSET` is counted
//! afresh on every call, so a session created between two pages shifts the
//! window by one. `total_count` is exact for the statement that returned it
//! and says nothing about the next one. Nothing here compensates for that,
//! and the dashboard's pager reads `has_next_page` rather than arithmetic
//! over the count for exactly that reason.

use time::OffsetDateTime;

use cratestack::{CratestackContext, CratestackError, Page, PageInfo, PageInput};

use super::cratestack_schema::{self, procedures, types};

/// The largest page this procedure will answer — `search_customers`' twin,
/// copied for that constant's stated reason.
const MAX_PAGE_LIMIT: i64 = 100;

/// The page answered when the caller names no `limit`.
const DEFAULT_PAGE_LIMIT: i64 = 10;

/// Every `status` migration 0028's `status_is_known` CHECK admits.
///
/// A copy of a database constraint, and the copy is pinned rather than
/// trusted: `tests::against_postgres::the_declared_status_vocabulary_is_the_one_the_database_enforces`
/// inserts every value below and asserts the CHECK accepts it, then asserts
/// a value outside it is rejected. A vocabulary that drifted from the
/// constraint would otherwise fail closed in the worst way — refusing a
/// status an operator can see in their own data.
const KNOWN_STATUSES: [&str; 3] = ["open", "complete", "expired"];

/// Every `payment_status` migration 0028's `payment_status_is_known` CHECK
/// admits. Pinned by the same test as [`KNOWN_STATUSES`].
const KNOWN_PAYMENT_STATUSES: [&str; 3] = ["unpaid", "paid", "failed"];

/// Every column [`SummaryRow`] decodes, plus the window count.
///
/// A `&'static str`, not a `format!` — sqlx 0.9 takes it directly, so no site
/// here needs `crate::sql_audit`'s injection waiver.
///
/// **`client_secret_suffix` and `return_token` are not in this projection**,
/// and that is the file's one load-bearing omission — see the module doc.
const SEARCH_SQL: &str = "SELECT id, merchant_id, payment_intent_id, livemode, ui_mode, status, \
                          payment_status, customer_id, expires_at, created_at, updated_at, \
                          count(*) OVER () AS total_count \
                          FROM checkout_sessions \
                          WHERE merchant_id = $1 \
                            AND ($2::TEXT IS NULL OR status = $2) \
                            AND ($3::TEXT IS NULL OR payment_status = $3) \
                            AND ($4::TIMESTAMPTZ IS NULL OR created_at >= $4) \
                            AND ($5::TIMESTAMPTZ IS NULL OR created_at <= $5) \
                          ORDER BY seq DESC \
                          LIMIT $6 OFFSET $7";

/// This body's implementation of the shared `ProcedureRegistry` — a free
/// function for `search_customers::search_customers`' stated reason.
pub(super) async fn search_checkout_sessions(
    db: &cratestack_schema::Cratestack,
    ctx: &CratestackContext,
    args: procedures::search_checkout_sessions::Args,
) -> Result<procedures::search_checkout_sessions::Output, CratestackError> {
    // From the context, never from `args`: there is no `merchant_id`
    // argument to this procedure and there must not be one, because a
    // tenancy predicate the caller chooses is not a tenancy predicate.
    let merchant_id = tenant_of(ctx)?;
    let status = known(args.filter.status.as_deref(), &KNOWN_STATUSES, "status")?;
    let payment_status = known(
        args.filter.payment_status.as_deref(),
        &KNOWN_PAYMENT_STATUSES,
        "payment_status",
    )?;
    let (limit, offset) = resolve_page(args.page);

    let rows: Vec<SummaryRow> = sqlx::query_as::<_, SummaryRow>(SEARCH_SQL)
        .bind(merchant_id)
        .bind(status)
        .bind(payment_status)
        .bind(to_time(args.filter.created_gte)?)
        .bind(to_time(args.filter.created_lte)?)
        .bind(limit.saturating_add(1))
        .bind(offset)
        .fetch_all(db.pool())
        .await
        .map_err(cratestack::cratestack_error_from_sqlx)?;

    page_of(rows, limit, offset)
}

/// A filter value that is in the vocabulary, or a `400` naming the parameter.
///
/// **A refusal and not an empty page**, which is `search_payment_intents`'
/// reasoning applied to a second vocabulary: "no sessions are `expird`" is a
/// sentence an operator reads as an answer about their sessions rather than
/// about their typo. The message lists the vocabulary so the answer is
/// actionable without opening the schema.
fn known<'a>(
    value: Option<&'a str>,
    vocabulary: &[&str],
    parameter: &str,
) -> Result<Option<&'a str>, CratestackError> {
    match value {
        None => Ok(None),
        Some(value) if vocabulary.contains(&value) => Ok(Some(value)),
        Some(value) => Err(CratestackError::BadRequest(format!(
            "{parameter}: `{value}` is not one of {}",
            vocabulary.join(", ")
        ))),
    }
}

/// [`PageInput::resolve`]'s clamp, with vpay's default page size.
fn resolve_page(page: PageInput) -> (i64, i64) {
    PageInput {
        limit: Some(page.limit.unwrap_or(DEFAULT_PAGE_LIMIT)),
        offset: page.offset,
    }
    .resolve(MAX_PAGE_LIMIT)
}

/// The merchant a context may read, or a refusal — `search_customers`' exact
/// body and reasoning. Never `Ok` with an empty string, which would be a
/// predicate matching no rows rather than a refusal.
fn tenant_of(ctx: &CratestackContext) -> Result<&str, CratestackError> {
    match ctx.tenant_id() {
        Some(tenant) if !tenant.is_empty() => Ok(tenant),
        _ => Err(CratestackError::Forbidden(
            "procedure policy denied this operation".to_owned(),
        )),
    }
}

/// Assembles the answer from the `limit + 1` rows the statement returned.
fn page_of(
    mut rows: Vec<SummaryRow>,
    limit: i64,
    offset: i64,
) -> Result<Page<types::CheckoutSessionSummary>, CratestackError> {
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
///
/// Neither payer credential has a field here, which is the second of the
/// three places that omission is expressed — the statement, this struct, and
/// the generated summary type.
#[derive(Debug, Clone, sqlx::FromRow)]
struct SummaryRow {
    id: String,
    merchant_id: String,
    payment_intent_id: String,
    livemode: bool,
    ui_mode: String,
    status: String,
    payment_status: String,
    customer_id: Option<String>,
    expires_at: OffsetDateTime,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    /// `count(*) OVER ()` — the whole filtered set, not this page.
    total_count: i64,
}

impl SummaryRow {
    /// The stored row as the procedure's answer. Infallible for
    /// `search_customers::SummaryRow::into_summary`'s reason: every column is
    /// a plain scalar or a timestamp, and [`to_chrono`] is total.
    fn into_summary(self) -> types::CheckoutSessionSummary {
        types::CheckoutSessionSummary {
            id: self.id,
            merchant_id: self.merchant_id,
            payment_intent_id: self.payment_intent_id,
            livemode: self.livemode,
            ui_mode: self.ui_mode,
            status: self.status,
            payment_status: self.payment_status,
            customer_id: self.customer_id,
            expires_at: to_chrono(self.expires_at),
            created_at: to_chrono(self.created_at),
            updated_at: to_chrono(self.updated_at),
        }
    }
}

/// A filter bound, in the type sqlx binds a `TIMESTAMPTZ` from —
/// `search_customers::to_time`'s exact body, including the nanosecond clamp
/// that keeps a chrono leap second from moving a bound into the next second.
///
/// # Errors
///
/// [`CratestackError::BadRequest`] for an instant outside `time`'s range.
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

/// A stored `TIMESTAMPTZ` as the procedure's answer — total over every
/// instant `time` can hold, proved once in `customers::tests`.
fn to_chrono(stored: OffsetDateTime) -> cratestack::chrono::DateTime<cratestack::chrono::Utc> {
    cratestack::chrono::DateTime::from_timestamp(stored.unix_timestamp(), stored.nanosecond())
        .unwrap_or(cratestack::chrono::DateTime::<cratestack::chrono::Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    //! No-database cases first, then [`against_postgres`]. The split is
    //! `search_customers::tests`' own, and for the same reason: a tenancy
    //! predicate is only observable through the rows it excluded, and a
    //! credential's absence from a *response* cannot be proved by reading
    //! the struct that was supposed to omit it.

    use super::*;

    /// A context carrying one tenant claim, which is all [`tenant_of`] reads.
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

    /// The refusal a caller with no tenant gets, and that it is the *same*
    /// refusal the policy produces.
    ///
    /// Decisive: replacing `tenant_of`'s `Err` arm with `Ok("")` fails the
    /// first assertion, and dropping the emptiness check fails the second —
    /// an empty `merchant_id` is a predicate matching nothing, which an
    /// operator reads as "this merchant has no checkouts".
    #[test]
    fn a_context_without_a_tenant_cannot_read_any_merchants_sessions() {
        let error = tenant_of(&CratestackContext::anonymous())
            .expect_err("a context with no tenant must be refused, not read unscoped");
        assert!(matches!(error, CratestackError::Forbidden(_)), "{error:?}");
        assert_eq!(
            error.public_message(),
            "procedure policy denied this operation"
        );

        assert!(
            tenant_of(&context_for_tenant("")).is_err(),
            "an empty merchant_id is a predicate that matches nothing"
        );
        assert_eq!(
            tenant_of(&context_for_tenant("acme-cameroon-tenant")).ok(),
            Some("acme-cameroon-tenant")
        );
    }

    /// An unknown filter value is refused, and the refusal names the
    /// parameter and lists the vocabulary.
    ///
    /// Decisive: making [`known`] return `Ok(value)` for anything — the
    /// cheapest way to "support" a new status — fails here, because the
    /// point is that a typo is answered as a typo rather than as an empty
    /// page.
    #[test]
    fn an_unknown_status_is_refused_rather_than_answered_with_an_empty_page() {
        for (value, vocabulary, parameter) in [
            ("expird", KNOWN_STATUSES, "status"),
            ("pald", KNOWN_PAYMENT_STATUSES, "payment_status"),
        ] {
            let error = known(Some(value), &vocabulary, parameter)
                .expect_err("a value outside the vocabulary is a 400");
            assert!(matches!(error, CratestackError::BadRequest(_)), "{error:?}");
            let message = error.public_message();
            assert!(message.contains(parameter), "{message}");
            assert!(message.contains(value), "{message}");
            for known_value in vocabulary {
                assert!(
                    message.contains(known_value),
                    "the refusal lists the vocabulary: {message}"
                );
            }
        }

        for value in KNOWN_STATUSES {
            assert_eq!(
                known(Some(value), &KNOWN_STATUSES, "status").ok().flatten(),
                Some(value)
            );
        }
        assert_eq!(known(None, &KNOWN_STATUSES, "status").ok().flatten(), None);
    }

    /// [`resolve_page`]'s contract at every boundary a caller can reach.
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
        ];
        for (limit, offset, expected_limit, expected_offset) in cases {
            assert_eq!(
                resolve_page(PageInput { limit, offset }),
                (expected_limit, expected_offset),
                "limit={limit:?} offset={offset:?}"
            );
        }
    }

    /// The ceiling stays inside CrateStack's own.
    #[test]
    fn the_page_ceiling_stays_inside_cratestacks_own() {
        const { assert!(MAX_PAGE_LIMIT > 0) };
        const { assert!(MAX_PAGE_LIMIT <= cratestack::MAX_LIST_LIMIT) };
        const { assert!(DEFAULT_PAGE_LIMIT > 0) };
        const { assert!(DEFAULT_PAGE_LIMIT <= MAX_PAGE_LIMIT) };
    }

    /// **Neither payer credential is named anywhere in the statement.**
    ///
    /// A text assertion, which is weaker than the container case below and
    /// is kept because it fails *at compile-and-unit time* rather than
    /// waiting for a container — the one place a careless projection edit
    /// gets caught in seconds.
    #[test]
    fn the_statement_names_neither_payer_credential() {
        assert!(
            !SEARCH_SQL.contains("client_secret_suffix"),
            "the client secret authorises the payer's browser; a list must not carry it"
        );
        assert!(
            !SEARCH_SQL.contains("return_token"),
            "the return token authorises the return leg; a list must not carry it"
        );
        assert!(SEARCH_SQL.contains("WHERE merchant_id = $1"));
        assert!(SEARCH_SQL.contains("ORDER BY seq DESC"));
    }

    mod against_postgres {
        use anyhow::Context as _;
        use sqlx::postgres::PgPoolOptions;
        use testcontainers::ContainerAsync;
        use testcontainers_modules::postgres::Postgres as PostgresImage;

        use super::*;

        /// `merchant_a`'s two sessions and `merchant_b`'s one, in a migrated
        /// database, plus a `Cratestack` over it.
        ///
        /// The sessions are created through the **real** repository, not by
        /// hand-crafted `INSERT`s, so `client_secret_suffix` and
        /// `return_token` hold whatever the production path puts there. That
        /// matters for the credential case below: a hand-built row could
        /// carry an empty string and the assertion would pass for the wrong
        /// reason.
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
                .context("XAF exists, because each session needs an intent")?;

            let base = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);

            for (session, merchant, intent) in [
                ("cs_a_old", "merchant_a", "pi_a_old"),
                ("cs_a_new", "merchant_a", "pi_a_new"),
                // The row that must never appear in `merchant_a`'s page.
                ("cs_b_only", "merchant_b", "pi_b_only"),
            ] {
                repositories
                    .insert(&crate::NewPaymentIntent {
                        id: intent.to_owned(),
                        merchant_id: merchant.to_owned(),
                        livemode: false,
                        amount: 5_000,
                        currency_code: "XAF".to_owned(),
                        status: "requires_payment_method".to_owned(),
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
                    .context("the intent each session hangs off")?;

                // Disambiguated: several repository traits expose `create`
                // on the same handle.
                crate::CheckoutSessions::create(
                    &*repositories,
                    &crate::NewCheckoutSession {
                        id: session.to_owned(),
                        merchant_id: merchant.to_owned(),
                        payment_intent_id: intent.to_owned(),
                        livemode: false,
                        ui_mode: "hosted".to_owned(),
                        success_url: Some("https://shop.example.cm/ok".to_owned()),
                        cancel_url: Some("https://shop.example.cm/no".to_owned()),
                        return_url: None,
                        customer_id: None,
                        // Through the real repository with real credential
                        // material, so the credential case below can fail:
                        // an empty string here would let it pass for the
                        // wrong reason.
                        publishable_key: "pk_test_shopmerchantsandbox1".to_owned(),
                        client_secret_suffix: vpay_core::ids::client_secret_suffix(),
                        return_token: vpay_core::ids::client_secret_suffix(),
                        expires_at: base + time::Duration::hours(24),
                        created_at: base,
                    },
                )
                .await
                .context("a checkout session through the real repository")?;
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

        /// **The tenancy predicate, which no unit test can observe.**
        ///
        /// Decisive mutation: replace `WHERE merchant_id = $1` in
        /// [`SEARCH_SQL`] with a tautology and this reddens with
        /// `cs_b_only` — another merchant's checkout, carrying that
        /// merchant's payment intent id — inside `merchant_a`'s page.
        #[tokio::test]
        async fn the_page_is_the_tenants_own_sessions_newest_first() -> anyhow::Result<()> {
            let (_container, db) = seeded().await?;
            let page = search_checkout_sessions(
                &db,
                &context_for_tenant("merchant_a"),
                procedures::search_checkout_sessions::Args {
                    page: PageInput {
                        limit: Some(10),
                        offset: Some(0),
                    },
                    filter: types::CheckoutSessionListFilter {
                        status: None,
                        payment_status: None,
                        created_gte: None,
                        created_lte: None,
                    },
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;

            let ids: Vec<&str> = page.items.iter().map(|item| item.id.as_str()).collect();
            assert_eq!(
                ids,
                vec!["cs_a_new", "cs_a_old"],
                "newest first, and merchant_b's session is not merchant_a's business"
            );
            assert_eq!(page.total_count, Some(2));
            Ok(())
        }

        /// **Neither payer credential reaches the answer.**
        ///
        /// The rows come back through the real repository, so both columns
        /// hold real credentials in the database — this asserts they are not
        /// in what the procedure returns. Serialising the page and searching
        /// the JSON is deliberate: a field added to the summary type later
        /// would be caught here even though no assertion names it.
        ///
        /// Decisive: add `client_secret_suffix` to [`SEARCH_SQL`]'s
        /// projection, to [`SummaryRow`] and to `into_summary`, and this
        /// reddens.
        #[tokio::test]
        async fn neither_payer_credential_is_in_the_projection() -> anyhow::Result<()> {
            let (_container, db) = seeded().await?;

            let stored: (String, String) = sqlx::query_as(
                "SELECT client_secret_suffix, return_token FROM checkout_sessions WHERE id = $1",
            )
            .bind("cs_a_new")
            .fetch_one(db.pool())
            .await
            .context("the stored credentials exist to be leaked")?;
            assert!(
                stored.0.len() >= 32 && stored.1.len() >= 32,
                "the real repository stored real credentials, so this test can fail"
            );

            let page = search_checkout_sessions(
                &db,
                &context_for_tenant("merchant_a"),
                procedures::search_checkout_sessions::Args {
                    page: PageInput {
                        limit: Some(10),
                        offset: Some(0),
                    },
                    filter: types::CheckoutSessionListFilter {
                        status: None,
                        payment_status: None,
                        created_gte: None,
                        created_lte: None,
                    },
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;

            let answered = serde_json::to_string(&page.items).context("the page serialises")?;
            assert!(
                !answered.contains(&stored.0),
                "the client secret suffix reached the answer"
            );
            assert!(
                !answered.contains(&stored.1),
                "the return token reached the answer"
            );
            assert!(
                !answered.contains("client_secret") && !answered.contains("return_token"),
                "no field of either name is on the summary: {answered}"
            );
            Ok(())
        }

        /// **The declared vocabularies are the ones the database enforces.**
        ///
        /// [`KNOWN_STATUSES`] and [`KNOWN_PAYMENT_STATUSES`] are copies of
        /// migration 0028's `status_is_known` and `payment_status_is_known`
        /// CHECKs. A copy drifts; this pins it in both directions — every
        /// declared value is accepted, and a value outside is rejected.
        #[tokio::test]
        async fn the_declared_vocabularies_are_the_ones_the_database_enforces() -> anyhow::Result<()>
        {
            let (_container, db) = seeded().await?;

            for status in KNOWN_STATUSES {
                sqlx::query("UPDATE checkout_sessions SET status = $1 WHERE id = $2")
                    .bind(status)
                    .bind("cs_a_new")
                    .execute(db.pool())
                    .await
                    .with_context(|| format!("`{status}` is a status this table admits"))?;
            }
            for payment_status in KNOWN_PAYMENT_STATUSES {
                sqlx::query("UPDATE checkout_sessions SET payment_status = $1 WHERE id = $2")
                    .bind(payment_status)
                    .bind("cs_a_new")
                    .execute(db.pool())
                    .await
                    .with_context(|| format!("`{payment_status}` is admitted"))?;
            }

            let refused = sqlx::query("UPDATE checkout_sessions SET status = $1 WHERE id = $2")
                .bind("expird")
                .bind("cs_a_new")
                .execute(db.pool())
                .await;
            assert!(
                refused.is_err(),
                "a status outside the vocabulary must be refused by status_is_known"
            );
            Ok(())
        }
    }
}
