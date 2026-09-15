//! The `ledger_transactions` / `ledger_entries` repository — the first code
//! in this repository's history to write either table (RFC-0003 § 4).
//!
//! # What exists here, and what does not
//!
//! One write, [`post_in_tx`], and one read,
//! [`Ledger::merchant_payable_balance`]. The write is `pub(crate)` and has no
//! entry on any public trait at all, so what a consumer of this crate can
//! name is the business operation — [`crate::Settlement::apply_succeeded`],
//! [`crate::Settlement::apply_refund_succeeded`] — and never the raw double
//! entry. There is deliberately **no pooled variant** either; a `post` that
//! opened its own transaction would make "the ledger agrees with the charge"
//! a property of whoever remembered to call it in the right place.
//!
//! **That is [`crate::refunds::settle_in_tx`]'s shape exactly, and it was
//! briefly something wider.** From RFC-0003 § 4's first half until its second
//! this function was reachable through a `pub` method on
//! `crate::TxRepositories`, which let any caller holding a
//! `PendingTransaction` post an arbitrary balanced transaction against an
//! arbitrary `charge_id` under an id of its choosing, with no settlement
//! anywhere near it — while that trait's own doc claimed a consumer could not
//! post without the settlement that justified it. The method is gone;
//! [`crate::refunds::Refunds`]' doc states the rule both now follow.
//!
//! **Two shipping call sites post, and they are both in
//! [`crate::settlement`].** [`crate::Settlement::apply_succeeded`] records
//! the capture in the transaction that settles the charge, and
//! [`crate::Settlement::apply_refund_succeeded`] records the refund in the
//! transaction that settles the refund. Those two, and nothing else, which is
//! what makes every sentence in this module about "the caller's transaction"
//! checkable by reading one file.
//!
//! What is still absent, so that the presence of a writer does not imply more
//! than it should: no rail can execute a refund
//! (`ProviderAdapter::refund` is `NotImplemented` on both), `POST /v1/refunds`
//! is unrouted until Wave 3, and nothing schedules the nightly assertion of
//! invariants 2-4. `docs/status.md` and `docs/flows/ledger.md` § Status carry
//! the gaps.
//!
//! # The transaction id is minted, and what that does and does not buy
//!
//! Both call sites take their `transaction_id` from
//! `vpay_core::ids::ledger_transaction_id` (`lt_…`), added with migration
//! `0046`'s `id_length` CHECK on both tables — the two halves of the gap
//! migration `0045`'s header recorded and left open.
//!
//! It does **not** make `ledger_transactions_pkey` a guard of
//! `docs/flows/ledger.md` invariant 4. A random id cannot be; what stops a
//! charge growing a second capture transaction is
//! [`crate::Settlement::apply_succeeded`]'s compare-and-swap on the charge
//! still being live, which stops the settlement running twice at all.
//! `vpay_core::ids::ledger_transaction_id`'s own doc records the schema
//! change a second, independent guard would need, and that it is a
//! maintainer's decision rather than this branch's.
//!
//! # `Transaction::validate()` is called here, and its failure is an error
//!
//! `vpay_ledger::Transaction::validate()` has existed and been tested since
//! the crate was written, and nothing that moves money has ever called it.
//! [`post_in_tx`] calls it **before its first statement**, and returns
//! [`DbError::Ledger`] rather than panicking, because an unbalanced
//! transaction reaching this function is a vpay bug and ADR-0007's answer to
//! a vpay bug is an error the caller must handle, never an `expect`. The
//! caller's transaction is untouched when it fires — no `ledger_transactions`
//! row, no `ledger_entries` row — so the settlement that raised it rolls back
//! whole.
//!
//! Invariant 1 (`SUM(debit) = SUM(credit)` per transaction) stays
//! application-enforced for the reason `docs/flows/ledger.md` and migration
//! `0005`'s own GAP note give: it is an aggregate over sibling rows, no
//! row-level CHECK can see them, and a hand-written constraint trigger would
//! be new unexercised logic duplicating a tested check. This function being
//! the only writer is what makes "application-enforced" true rather than
//! aspirational.
//!
//! # Why the ids are the caller's, and the entries' are derived from them
//!
//! `ledger_transactions.id` is `TEXT PRIMARY KEY` with no default — migration
//! `0005` explains why Postgres mints no id for it — so [`post_in_tx`] takes
//! one. A caller that derives it deterministically from the thing being
//! settled gets idempotency for free: a second attempt at the same posting
//! fails the primary key ([`DbError::UniqueViolation`]) instead of writing the
//! ledger twice, which is the failure a settlement job that may run twice
//! needs to be loud rather than silent.
//!
//! Each entry's id is that transaction id with its leg's index appended.
//! Deterministic for the same reason, and it also means an entry id says which
//! transaction it belongs to without a join — which is what an operator
//! reading a row in isolation actually needs.
//!
//! **That derivation is injective for every pair of transaction ids, minted
//! or not**, because the index is decimal and decimal contains no `_`. See
//! [`entry_id`], which is where the argument and its test live; RFC-0003
//! § 4's amendment recorded an ambiguity here that does not exist, and that
//! function is the correction of record.

use async_trait::async_trait;
use vpay_ledger::{AccountKind, Direction, Transaction};

use crate::error::{DbError, classify_write};

/// The `account_kind` label for each [`AccountKind`] variant, as migration
/// `0005`'s `CREATE TYPE` spells it.
///
/// A `match` rather than a `Display` impl on the enum: the label is this
/// schema's vocabulary, not the ledger's, and `vpay-ledger` has no business
/// knowing how Postgres spells its accounts. It is the same split
/// [`crate::refunds::RefundRow::status`] draws in the other direction — a
/// vocabulary is closed by the database where it is written, and carried as
/// text at the layer that renders it.
fn account_label(account: &AccountKind) -> &'static str {
    match account {
        AccountKind::MerchantPayable { .. } => "merchant_payable",
        AccountKind::PayerClearing => "payer_clearing",
        AccountKind::PlatformFeeRevenue => "platform_fee_revenue",
    }
}

/// The `direction` label, for [`account_label`]'s reason.
fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::Debit => "debit",
        Direction::Credit => "credit",
    }
}

/// The `ledger_entries.id` for one leg: its transaction's id, then the leg's
/// index.
///
/// # Injective, and the reason is the separator rather than the minter
///
/// RFC-0003 § 4's amendment recorded this derivation as ambiguous "across two
/// transactions whose ids differ only by a `_N` suffix" and asked
/// `vpay_core::ids` for a minter that would make the ambiguity unreachable.
/// **There is no ambiguity to reach.** `index` is a `usize` rendered in
/// decimal, decimal contains no `_`, so the *last* `_` of the result splits
/// it back into exactly one `(transaction_id, index)` pair — for any
/// transaction ids at all, minted or hand-built. The pair the amendment
/// named is not a counterexample: `x` derives `x_0, x_1, …` and `x_0` derives
/// `x_0_0, x_0_1, …`, which are disjoint.
///
/// So this function, not the minter, is what
/// [`crate::Ledger::merchant_payable_balance`] and every operator reading one
/// row in isolation depend on, and [`tests::the_entry_id_derivation_is_injective`]
/// is the assertion. Change the separator to something the index can contain,
/// or render the index in a base that admits `_`, and it fails there rather
/// than at `ledger_entries_pkey` in production.
///
/// [`vpay_core::ids::ledger_transaction_id`] is still what both settlement
/// call sites mint through, for the reason its own doc gives — a vocabulary,
/// so the id is not invented at whichever call site writes one first — and
/// not for this.
fn entry_id(transaction_id: &str, index: usize) -> String {
    // Not a statement — `crate::sql_audit` scans only a `format!` whose
    // result is bound to a `sql` variable, and this one builds a value that
    // its caller passes as `$1`. It is spelled with `format!` for the same
    // reason every other id in this crate is a `&str`: the database is never
    // handed a fragment, only a parameter.
    format!("{transaction_id}_{index}")
}

/// Records one balanced ledger transaction and every leg of it, inside the
/// caller's transaction.
///
/// `transaction_id` is the `ledger_transactions.id` to write; `charge_id` is
/// the charge the posting is attributed to, which is a real foreign key.
///
/// # The validation is the first thing that happens
///
/// [`Transaction::validate`] runs before any statement, so an unbalanced or
/// single-legged posting writes nothing at all. See the module docs for why
/// that is an error and not an `expect`, and why no database constraint is
/// asked to do this job.
///
/// # Errors
///
/// [`DbError::Ledger`] if the posting does not balance or has fewer than two
/// legs — nothing is written in that case.
/// [`DbError::UniqueViolation`] if `transaction_id` has already been posted.
/// [`DbError::ForeignKeyViolation`] for an unknown `charge_id` or an entry in
/// a currency `currencies` does not carry.
/// [`DbError::Query`] otherwise — including the `23514` raised by
/// `ledger_entries_merchant_id_iff_merchant_payable`, which cannot fire while
/// [`AccountKind`] is the only source of both columns and is classified as a
/// vpay bug rather than a caller error for [`classify_write`]'s stated reason.
pub(crate) async fn post_in_tx(
    conn: &mut sqlx::PgConnection,
    transaction_id: &str,
    charge_id: &str,
    transaction: &Transaction,
) -> Result<(), DbError> {
    transaction.validate()?;

    sqlx::query("INSERT INTO ledger_transactions (id, charge_id) VALUES ($1, $2)")
        .bind(transaction_id)
        .bind(charge_id)
        .execute(&mut *conn)
        .await
        .map_err(classify_write)?;

    for (index, entry) in transaction.entries.iter().enumerate() {
        let entry_id = entry_id(transaction_id, index);
        sqlx::query(
            "INSERT INTO ledger_entries \
                 (id, transaction_id, account, direction, amount, currency_code, merchant_id) \
             VALUES ($1, $2, $3::account_kind, $4::direction, $5, $6, $7)",
        )
        .bind(&entry_id)
        .bind(transaction_id)
        .bind(account_label(&entry.account))
        .bind(direction_label(entry.direction))
        .bind(entry.amount.minor())
        .bind(entry.amount.currency().code())
        // `AccountKind` is the only source of both this and `account` above,
        // so the pair CHECK in migration `0045` and the sum type agree by
        // construction rather than by a check beside the write.
        .bind(entry.account.merchant_id())
        .execute(&mut *conn)
        .await
        .map_err(classify_write)?;
    }

    Ok(())
}

/// The `ledger` reads a consumer of this crate may perform.
///
/// One, and it is the one `docs/flows/ledger.md` invariant 2 names. The write
/// is **not** on this trait, and not on any other, for
/// [`crate::refunds::Refunds`]' reason: it is `pub(crate)` and belongs to
/// [`crate::settlement`]'s transaction, so a consumer of this crate cannot
/// post to the ledger without the settlement that justifies the posting.
///
/// That sentence was false for as long as
/// `TxRepositories::post_ledger_transaction_in_tx` existed, which is why the
/// method does not — see this module's own docs.
#[async_trait]
pub trait Ledger: Send + Sync {
    /// `balance(merchant_payable) = Σ credit − Σ debit` for one merchant in
    /// one currency, in minor units.
    ///
    /// The database half of `docs/flows/ledger.md` invariant 2, and the read
    /// the merchant dimension exists for: before migration `0045` this
    /// function could not have been written, because no column said which
    /// merchant a `merchant_payable` posting belonged to.
    ///
    /// `i64` rather than `vpay_core::Money`, matching
    /// `vpay_ledger::balance`: a merchant's payable balance is legitimately
    /// negative while a refund has outrun its capture, and `Money` is
    /// non-negative by construction. A merchant with no postings at all
    /// answers `0`, which is the true balance rather than an absence — the
    /// sum is computed with `COALESCE`, so "no rows" and "rows summing to
    /// zero" are the same answer on purpose.
    ///
    /// **Not merchant-scoped in the tenancy sense**, and it must not be
    /// confused with the merchant-facing reads: `merchant_id` here is *what
    /// is being asked about*, not a permission. Nothing routes this yet; a
    /// `/v1` handler that ever does must scope the caller to the merchant it
    /// passes, exactly as `crate::refunds` does.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn merchant_payable_balance(
        &self,
        merchant_id: &str,
        currency_code: &str,
    ) -> Result<i64, DbError>;
}

#[async_trait]
impl Ledger for crate::repository::PgRepositories {
    async fn merchant_payable_balance(
        &self,
        merchant_id: &str,
        currency_code: &str,
    ) -> Result<i64, DbError> {
        // `SUM(credit) - SUM(debit)` written as one conditional sum rather
        // than two aggregates, so a row can only be counted once and on one
        // side. `COALESCE` over the whole expression, not inside it: `SUM`
        // over no rows is `NULL`, and a merchant that has never traded has a
        // balance of zero rather than no balance.
        //
        // The predicate is exactly `ledger_entries_merchant_payable_idx`'s
        // (migration `0045`) — `account = 'merchant_payable'` partial, keyed
        // on `(merchant_id, currency_code)`.
        //
        // `::BIGINT` because Postgres's `SUM(bigint)` is `NUMERIC`, which
        // sqlx refuses to decode into an `i64` — a run-time failure the type
        // system cannot catch here, and one this crate's container tests
        // caught rather than reasoned about. The cast cannot lose anything:
        // every `amount` is bounded by `amount_non_negative` and the sum of
        // one merchant's postings in one currency is an amount of money.
        let balance: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(CASE WHEN direction = 'credit' THEN amount ELSE -amount END), 0)\
             ::BIGINT \
             FROM ledger_entries \
             WHERE account = 'merchant_payable' AND merchant_id = $1 AND currency_code = $2",
        )
        .bind(merchant_id)
        .bind(currency_code)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Query)?;

        Ok(balance)
    }
}

#[cfg(test)]
mod tests {
    //! Three container-free tests, and seven that start a Postgres.
    //!
    //! The three that need no container are the two label tests and
    //! [`the_entry_id_derivation_is_injective`]; the seven are the six
    //! `docs/status/backend.md` lists as the writer's evidence, plus
    //! [`swallowing_a_duplicate_posting_inside_a_transaction_discards_the_whole_transaction`],
    //! which that row names separately because its subject is the trap in the
    //! idempotency rather than the writer's own refusals.
    //!
    //! # Why the seven live here and not in `postgres_smoke.rs`
    //!
    //! They were there, driving [`post_in_tx`] through the `pub`
    //! `TxRepositories::post_ledger_transaction_in_tx` that existed between
    //! RFC-0003 § 4's two halves. That method is gone — see this module's
    //! header — so the raw writer is `pub(crate)` and nothing outside this
    //! crate can name it. Every one of the seven needs to: their subject is
    //! what the writer refuses (an unbalanced posting, a mixed-currency one,
    //! a replayed id) or what it derives (entry ids), and none of it is
    //! reachable through the business operations a consumer *can* call —
    //! `Settlement::apply_succeeded` and `apply_refund_succeeded` build their
    //! own postings and cannot be made to build a bad one.
    //!
    //! `config_reconcile.rs`'s container test states the rule this follows:
    //! adding a public method purely to give a test a door is publishing a
    //! capability vpay does not have. The alternative — leaving the trait
    //! method in place so the tests could stay where they were — is the
    //! widening this arm was asked to remove.
    //!
    //! **What did NOT move** is the pair of cases that write the two rows
    //! `ledger_entries_merchant_id_iff_merchant_payable` refuses. Those are
    //! hand-written SQL against a constraint, not against this writer, so
    //! they stay in `postgres_smoke.rs` beside the rest of migration 0045's
    //! evidence — and the writer could not produce either row in any case,
    //! which is why they are hand-written.

    use anyhow::Context as _;
    use sqlx::PgPool;

    use super::*;
    use crate::migrations::Migrations as _;
    use crate::repository::PgRepositories;

    /// A freshly migrated Postgres 16, and the repositories bound to it.
    ///
    /// Duplicated from `postgres_smoke.rs`'s helper of the same shape rather
    /// than shared: the two suites are in different crates, and a shared
    /// fixture crate would have to make `PgRepositories` — a `pub(crate)`
    /// type whose privacy is the point (ADR-0016 standard 5) — reachable from
    /// outside. The container itself comes from
    /// `vpay_testkit::containers::start_postgres_with_retry`, which is the
    /// one helper every Postgres-backed suite in this workspace shares and
    /// where the pinned image tag lives.
    async fn migrated_postgres() -> anyhow::Result<(
        testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
        PgPool,
        PgRepositories,
    )> {
        let container = vpay_testkit::containers::start_postgres_with_retry()
            .await
            .context("postgres:16-alpine container starts")?;
        let host = container.get_host().await.context("container host")?;
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .context("container port")?;
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = PgPool::connect(&url)
            .await
            .context("connecting to the container")?;
        let repositories = PgRepositories {
            pool: pool.clone(),
            cs: crate::schema::cratestack_schema::Cratestack::builder(pool.clone()).build(),
        };
        repositories
            .run_migrations()
            .await
            .context("every migration under backends/migrations applies cleanly")?;

        Ok((container, pool, repositories))
    }

    /// The four rows a ledger transaction's foreign key needs: two
    /// currencies, one rail, one intent, one charge.
    ///
    /// Hand-written SQL, and the reason is `postgres_smoke.rs`'s: the subject
    /// of every case below is the ledger writer, so a fixture that went
    /// through the confirm path would make these tests fail for reasons that
    /// have nothing to do with it.
    async fn seed_charge(
        pool: &PgPool,
        merchant_id: &str,
        intent_id: &str,
        charge_id: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('XAF', 0), ('EUR', 2)")
            .execute(pool)
            .await
            .context("seeding currencies")
            // Two intents in one test share the currency rows; the second
            // seed is a duplicate and not a failure.
            .ok();
        sqlx::query(
            "INSERT INTO providers \
                (code, display_name, flow, supports_refunds, supports_partial_refunds, \
                 delivers_callbacks, requires_ip_allowlist, enabled) \
             VALUES ('mtn_momo', 'MTN MoMo', 'push', true, true, true, true, true) \
             ON CONFLICT (code) DO NOTHING",
        )
        .execute(pool)
        .await
        .context("seeding the rail")?;
        sqlx::query(
            "INSERT INTO payment_intents \
                (id, merchant_id, livemode, amount, currency_code, status, payment_method_types, \
                 client_secret_suffix) \
             VALUES ($1, $2, false, 20000, 'XAF', 'requires_payment_method', '[]'::jsonb, \
                     replace(gen_random_uuid()::text, '-', ''))",
        )
        .bind(intent_id)
        .bind(merchant_id)
        .execute(pool)
        .await
        .context("seeding the payment intent")?;
        sqlx::query(
            "INSERT INTO charges \
                (id, payment_intent_id, provider_code, provider_reference_id, state, amount, \
                 currency_code) \
             VALUES ($1, $2, 'mtn_momo', gen_random_uuid(), 'submitting', 20000, 'XAF')",
        )
        .bind(charge_id)
        .bind(intent_id)
        .execute(pool)
        .await
        .context("seeding the charge")?;
        Ok(())
    }

    /// Posts through the writer under test, in its own committed
    /// transaction.
    ///
    /// The three lines every case below would otherwise repeat, and it is
    /// deliberately the *whole* of what a caller does: begin, post, commit.
    /// Nothing here catches anything — a case that wants a refusal gets the
    /// error out of this function, which is what makes "the refusal reaches
    /// the caller" part of what is being measured rather than swallowed by a
    /// helper.
    async fn post(
        pool: &PgPool,
        transaction_id: &str,
        charge_id: &str,
        transaction: &Transaction,
    ) -> Result<(), DbError> {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        post_in_tx(&mut tx, transaction_id, charge_id, transaction).await?;
        tx.commit().await.map_err(DbError::Query)
    }

    fn xaf(minor: i64) -> vpay_core::Money {
        vpay_core::Money::new(minor, vpay_core::Currency::Xaf).expect("non-negative")
    }

    /// The writer records a capture with a fee, and the result satisfies
    /// `docs/flows/ledger.md` invariant 1 as an aggregate over the rows it
    /// actually wrote.
    ///
    /// **Not a re-implementation of `Transaction::validate`.** That function
    /// is checked in `vpay-ledger`'s own unit tests; what this case proves is
    /// the half no unit test can — that the three legs reach three
    /// `ledger_entries` rows, in the right directions, with the amounts
    /// `docs/flows/ledger.md` § Postings gives, and that
    /// `SUM(debit) = SUM(credit)` holds **when read back out of Postgres**
    /// rather than when computed in memory.
    ///
    /// The three-leg form is built by hand here because no vpay call site
    /// produces one: nothing in this schema holds a capture-time platform
    /// fee, so `Settlement::apply_succeeded` passes `None` and posts two legs
    /// (see `settlement::post_capture`). The fee leg is still the documented
    /// shape and this is where it is checked against a real table.
    #[tokio::test]
    async fn a_balanced_posting_satisfies_invariant_1_in_the_database() -> anyhow::Result<()> {
        let (_container, pool, _repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_balanced", "ch_balanced").await?;

        let capture = Transaction::capture("merchant_1", xaf(5_000), Some(xaf(100)))
            .context("a 5 000 XAF capture with a 100 XAF fee")?;
        post(&pool, "lt_balanced", "ch_balanced", &capture)
            .await
            .context("posting a balanced capture must commit")?;

        // Invariant 1, as an aggregate over the sibling rows — the shape no
        // row-level CHECK can express, which is why `docs/flows/ledger.md`
        // commits to enforcing it in `Transaction::validate()` instead.
        // `::BIGINT` on both: Postgres's `SUM(bigint)` is `NUMERIC`, so
        // decoding either into an `i64` without the cast is a run-time type
        // mismatch rather than a compile error.
        let (debits, credits): (i64, i64) = sqlx::query_as(
            "SELECT \
                 COALESCE(SUM(amount) FILTER (WHERE direction = 'debit'), 0)::BIGINT, \
                 COALESCE(SUM(amount) FILTER (WHERE direction = 'credit'), 0)::BIGINT \
             FROM ledger_entries WHERE transaction_id = 'lt_balanced'",
        )
        .fetch_one(&pool)
        .await
        .context("summing the legs this posting wrote")?;
        assert_eq!(
            (debits, credits),
            (5_000, 5_000),
            "invariant 1: per transaction, SUM(debit) = SUM(credit)"
        );

        let legs: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
            "SELECT account::TEXT, direction::TEXT, amount, merchant_id FROM ledger_entries \
             WHERE transaction_id = 'lt_balanced' ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .context("reading the legs back")?;
        assert_eq!(
            legs,
            vec![
                ("payer_clearing".to_owned(), "debit".to_owned(), 5_000, None),
                (
                    "merchant_payable".to_owned(),
                    "credit".to_owned(),
                    4_900,
                    Some("merchant_1".to_owned())
                ),
                (
                    "platform_fee_revenue".to_owned(),
                    "credit".to_owned(),
                    100,
                    None
                ),
            ]
        );

        Ok(())
    }

    /// An unbalanced posting is refused **before any row is written**, by
    /// [`Transaction::validate`] inside the writer, and the refusal is a
    /// [`DbError`] the caller has to handle.
    ///
    /// This is the case that makes "invariant 1 stays application-enforced"
    /// (`docs/flows/ledger.md`) a fact about the write path rather than about
    /// a function nothing calls. Both halves matter and neither implies the
    /// other: the error, and the empty tables. A writer that inserted the
    /// parent row and then noticed would satisfy the first assertion and fail
    /// the second.
    #[tokio::test]
    async fn an_unbalanced_posting_is_refused_and_writes_nothing() -> anyhow::Result<()> {
        let (_container, pool, _repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_lopsided", "ch_lopsided").await?;

        // Hand-built, because neither `Transaction::capture` nor
        // `Transaction::refund` can produce an unbalanced posting — which is
        // the point of having them, and the reason this case has to assemble
        // the legs itself to reach the guard at all.
        let lopsided = Transaction {
            entries: vec![
                vpay_ledger::Entry {
                    account: AccountKind::PayerClearing,
                    direction: Direction::Debit,
                    amount: xaf(5_000),
                },
                vpay_ledger::Entry {
                    account: AccountKind::merchant_payable("merchant_1"),
                    direction: Direction::Credit,
                    amount: xaf(4_900),
                },
            ],
        };

        let refused = post(&pool, "lt_lopsided", "ch_lopsided", &lopsided)
            .await
            .expect_err("100 francs unaccounted for must not commit");
        assert!(
            matches!(
                refused,
                DbError::Ledger(vpay_ledger::LedgerError::Unbalanced {
                    currency: vpay_core::Currency::Xaf,
                    debits: 5_000,
                    credits: 4_900
                })
            ),
            "the refusal must name the imbalance rather than surface as a storage error: \
             {refused:?}"
        );

        let (transactions, entries): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM ledger_transactions), \
                    (SELECT COUNT(*) FROM ledger_entries)",
        )
        .fetch_one(&pool)
        .await
        .context("counting both tables")?;
        assert_eq!(
            (transactions, entries),
            (0, 0),
            "an unbalanced posting must write nothing at all, not a parent row with no legs"
        );

        Ok(())
    }

    /// A posting whose legs are in **different currencies** is refused, and
    /// writes nothing.
    ///
    /// `docs/flows/ledger.md` invariant 1 is `SUM(debit) = SUM(credit)` **per
    /// currency**, and 100 XAF debited against 100 EUR credited balances only
    /// if a franc is added to a euro. `Transaction::validate` summed minor
    /// units across currencies until 2026-09-15, and the fix was measured by
    /// watching this case commit against the code before it.
    ///
    /// Nothing in the schema would catch it either: `currency_code` is per
    /// row, and invariant 1 is deliberately not a database constraint — so
    /// `validate()` is the only guard there is.
    #[tokio::test]
    async fn a_mixed_currency_posting_is_refused_and_writes_nothing() -> anyhow::Result<()> {
        let (_container, pool, _repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_mixed", "ch_mixed").await?;

        let mixed = Transaction {
            entries: vec![
                vpay_ledger::Entry {
                    account: AccountKind::PayerClearing,
                    direction: Direction::Debit,
                    amount: xaf(100),
                },
                vpay_ledger::Entry {
                    account: AccountKind::merchant_payable("merchant_1"),
                    direction: Direction::Credit,
                    amount: vpay_core::Money::new(100, vpay_core::Currency::Eur)
                        .expect("non-negative"),
                },
            ],
        };

        let refused = post(&pool, "lt_mixed", "ch_mixed", &mixed)
            .await
            .expect_err("100 XAF against 100 EUR does not balance");
        assert!(
            matches!(
                refused,
                DbError::Ledger(vpay_ledger::LedgerError::Unbalanced { .. })
            ),
            "{refused:?}"
        );

        let (transactions, entries): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM ledger_transactions), \
                    (SELECT COUNT(*) FROM ledger_entries)",
        )
        .fetch_one(&pool)
        .await
        .context("counting both tables")?;
        assert_eq!(
            (transactions, entries),
            (0, 0),
            "a mixed-currency posting must write nothing at all"
        );

        Ok(())
    }

    /// `docs/flows/ledger.md` invariant 2, against a real database and for
    /// two merchants at once: `balance(merchant_payable)` is per merchant,
    /// and one merchant's postings do not move another's.
    ///
    /// **This is the case migration 0045 exists for**, and before it the
    /// question could not be asked — `ledger_entries` had no column saying
    /// which merchant a `merchant_payable` posting belonged to.
    ///
    /// **Decisive by construction.** Merchant 2 captures four times what
    /// merchant 1 does and only merchant 1 is refunded, so a balance that
    /// ignored the merchant dimension would answer 22 500 to every question
    /// below and fail every assertion rather than none.
    #[tokio::test]
    async fn two_merchants_payable_balances_do_not_mix() -> anyhow::Result<()> {
        let (_container, pool, repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_m1", "ch_m1").await?;
        seed_charge(&pool, "merchant_2", "pi_m2", "ch_m2").await?;

        let postings = [
            (
                "lt_m1_capture",
                "ch_m1",
                Transaction::capture("merchant_1", xaf(5_000), Some(xaf(100)))
                    .context("merchant 1's capture")?,
            ),
            (
                "lt_m2_capture",
                "ch_m2",
                Transaction::capture("merchant_2", xaf(20_000), Some(xaf(400)))
                    .context("merchant 2's capture")?,
            ),
            (
                "lt_m1_refund",
                "ch_m1",
                Transaction::refund("merchant_1", xaf(2_000)),
            ),
        ];
        for (id, charge_id, transaction) in &postings {
            post(&pool, id, charge_id, transaction)
                .await
                .context("every one of these postings balances and must commit")?;
        }

        // 4 900 credited, 2 000 debited.
        assert_eq!(
            repositories
                .merchant_payable_balance("merchant_1", "XAF")
                .await?,
            2_900,
            "merchant 1's balance is its own capture net of fee, less its own refund"
        );
        // 19 600 credited, nothing debited — merchant 1's refund is not
        // merchant 2's, which is the whole claim.
        assert_eq!(
            repositories
                .merchant_payable_balance("merchant_2", "XAF")
                .await?,
            19_600,
            "merchant 2 was never refunded; a balance that mixed the two would be short 2 000"
        );
        // A merchant with no postings has a balance of zero rather than no
        // balance, and certainly not the sum of everyone else's.
        assert_eq!(
            repositories
                .merchant_payable_balance("merchant_3", "XAF")
                .await?,
            0
        );
        // And a balance is per currency: nothing was posted in EUR.
        assert_eq!(
            repositories
                .merchant_payable_balance("merchant_1", "EUR")
                .await?,
            0
        );

        // The same two numbers, computed in Rust from the rows rather than by
        // the statement under test, so that a mistake shared between the
        // query and the assertions above cannot hide. `vpay_ledger::balance`
        // is the independent implementation `docs/flows/ledger.md`
        // invariant 2 is written against, and rebuilding the `AccountKind`
        // from the two columns is also what proves they round-trip.
        let rows: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
            "SELECT account::TEXT, direction::TEXT, amount, merchant_id FROM ledger_entries",
        )
        .fetch_all(&pool)
        .await
        .context("reading every posted leg back")?;
        let entries: Vec<vpay_ledger::Entry> = rows
            .into_iter()
            .map(|(account, direction, amount, merchant_id)| {
                let account = match (account.as_str(), merchant_id) {
                    ("merchant_payable", Some(id)) => AccountKind::merchant_payable(id),
                    ("payer_clearing", None) => AccountKind::PayerClearing,
                    ("platform_fee_revenue", None) => AccountKind::PlatformFeeRevenue,
                    (other, merchant) => {
                        anyhow::bail!("the pair CHECK should have refused ({other}, {merchant:?})")
                    }
                };
                Ok(vpay_ledger::Entry {
                    account,
                    direction: if direction == "debit" {
                        Direction::Debit
                    } else {
                        Direction::Credit
                    },
                    amount: vpay_core::Money::new(amount, vpay_core::Currency::Xaf)?,
                })
            })
            .collect::<anyhow::Result<_>>()?;

        assert_eq!(
            vpay_ledger::balance(
                &entries,
                &AccountKind::merchant_payable("merchant_1"),
                vpay_core::Currency::Xaf
            ),
            2_900
        );
        assert_eq!(
            vpay_ledger::balance(
                &entries,
                &AccountKind::merchant_payable("merchant_2"),
                vpay_core::Currency::Xaf
            ),
            19_600
        );

        Ok(())
    }

    /// Replaying a posting is refused by `ledger_transactions`' primary key,
    /// and a *different* posting reusing a spent id is refused by the same
    /// key — two separate facts, because the second does not follow from the
    /// first.
    ///
    /// The first is the one a settlement job that may run twice needs: the
    /// second attempt does not double the ledger. The second says the id is a
    /// *key* and not a hint — a caller that derived the same id for two
    /// different postings finds out loudly rather than appending a refund's
    /// legs to a capture's transaction.
    ///
    /// Neither is reachable from the two shipping call sites, which mint a
    /// fresh `lt_…` every time (`vpay_core::ids::ledger_transaction_id`).
    /// That is why the ids here are literals: the property belongs to the
    /// writer, and a test that could only reach it through a caller that
    /// cannot produce it would prove nothing.
    #[tokio::test]
    async fn a_replayed_transaction_id_is_refused_and_adds_no_legs() -> anyhow::Result<()> {
        let (_container, pool, _repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_replay", "ch_replay").await?;

        let capture = Transaction::capture("merchant_1", xaf(5_000), Some(xaf(100)))
            .context("a 5 000 XAF capture with a 100 XAF fee")?;
        post(&pool, "lt_replay", "ch_replay", &capture)
            .await
            .context("the first posting commits")?;

        // Same id, same posting: the replay a job that ran twice would make.
        let replayed = post(&pool, "lt_replay", "ch_replay", &capture)
            .await
            .expect_err("the second attempt at the same posting must not commit");
        assert!(
            matches!(
                &replayed,
                DbError::UniqueViolation { constraint, .. }
                    if constraint == "ledger_transactions_pkey"
            ),
            "a replay must be refused by the primary key, which is what makes the caller's id \
             the idempotency key: {replayed:?}"
        );

        // Same id, a *different* posting: a refund, which is two legs rather
        // than three and moves the money the other way. It must not land
        // either, and in particular its legs must not join the capture's
        // transaction.
        let collided = post(
            &pool,
            "lt_replay",
            "ch_replay",
            &Transaction::refund("merchant_1", xaf(2_000)),
        )
        .await
        .expect_err("a different posting reusing a spent id must not commit");
        assert!(
            matches!(&collided, DbError::UniqueViolation { .. }),
            "{collided:?}"
        );

        // Three legs, still, and they are the capture's. A writer that
        // inserted the entries before the parent row, or that swallowed the
        // duplicate, would have five here — and `merchant_payable` would be
        // 2 000 short.
        let legs: Vec<(String, i64)> = sqlx::query_as(
            "SELECT id, amount FROM ledger_entries WHERE transaction_id = 'lt_replay' ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .context("reading the legs back")?;
        assert_eq!(
            legs,
            vec![
                ("lt_replay_0".to_owned(), 5_000),
                ("lt_replay_1".to_owned(), 4_900),
                ("lt_replay_2".to_owned(), 100),
            ]
        );

        Ok(())
    }

    /// Three postings whose ids are prefixes of one another write six
    /// distinct `ledger_entries` rows — [`entry_id`]'s injectivity, against a
    /// real primary key rather than against a `HashSet`.
    ///
    /// The ids are hand-made and deliberately adversarial: `lt_p`, `lt_p_0`
    /// and `lt_p_0_0` are the worst family the derivation can be handed, and
    /// they are the family RFC-0003 § 4's amendment predicted would collide.
    /// They do not, and `ledger_entries_pkey` is what would have said so if
    /// they did — this case would fail on the second `post`, not on the
    /// assertion.
    ///
    /// [`the_entry_id_derivation_is_injective`] is the same property over a
    /// much wider family and with no container; this case is what ties it to
    /// the actual column.
    #[tokio::test]
    async fn entry_ids_do_not_collide_between_ids_that_share_a_prefix() -> anyhow::Result<()> {
        let (_container, pool, _repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_prefix", "ch_prefix").await?;

        for id in ["lt_p", "lt_p_0", "lt_p_0_0"] {
            post(
                &pool,
                id,
                "ch_prefix",
                &Transaction::refund("merchant_1", xaf(1_000)),
            )
            .await
            .with_context(|| format!("posting {id} must commit"))?;
        }

        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM ledger_entries ORDER BY id")
            .fetch_all(&pool)
            .await
            .context("reading every entry id back")?;
        assert_eq!(
            ids,
            vec![
                "lt_p_0".to_owned(),
                "lt_p_0_0".to_owned(),
                "lt_p_0_0_0".to_owned(),
                "lt_p_0_0_1".to_owned(),
                "lt_p_0_1".to_owned(),
                "lt_p_1".to_owned(),
            ],
            "six legs from three two-legged postings, all distinct"
        );

        Ok(())
    }

    /// **The trap in the idempotency, measured.** A caller that treats
    /// [`DbError::UniqueViolation`] as "already posted, carry on" and commits
    /// anyway loses its *whole* transaction — silently, with an `Ok` from
    /// `commit()` in hand.
    ///
    /// Postgres aborts a transaction at the first failed statement and turns
    /// a subsequent `COMMIT` into a `ROLLBACK` without raising anything. That
    /// is the same `sqlx::Transaction::commit` `UnitOfWork::transaction`
    /// calls, so a consumer taking this reading is handed
    /// `TxOutcome::Commit(())` while its charge, its event and its posting
    /// are all gone —
    /// `swallowing_a_duplicate_write_inside_a_transaction_discards_the_whole_transaction`
    /// in `postgres_smoke.rs` measures that half through the real seam.
    ///
    /// This case exists because swallowing the duplicate is the *natural*
    /// reading of "the primary key is the idempotency", and it is the one
    /// reading that must not be taken inside a settlement's own transaction.
    /// `crate::settlement`'s two posting call sites let the error propagate;
    /// `post_capture`'s doc says why, and this is the measurement behind it.
    #[tokio::test]
    async fn swallowing_a_duplicate_posting_inside_a_transaction_discards_the_whole_transaction()
    -> anyhow::Result<()> {
        let (_container, pool, _repositories) = migrated_postgres().await?;
        seed_charge(&pool, "merchant_1", "pi_swallow", "ch_swallow").await?;

        let capture = Transaction::capture("merchant_1", xaf(5_000), Some(xaf(100)))
            .context("a 5 000 XAF capture with a 100 XAF fee")?;

        let mut tx = pool.begin().await.context("beginning a transaction")?;
        post_in_tx(&mut tx, "lt_swallow", "ch_swallow", &capture)
            .await
            .context("the good posting")?;
        // ... and the same id again, with the duplicate swallowed the way an
        // "already posted, that is fine" branch would.
        let duplicate = post_in_tx(&mut tx, "lt_swallow", "ch_swallow", &capture).await;
        assert!(
            matches!(duplicate, Err(DbError::UniqueViolation { .. })),
            "{duplicate:?}"
        );
        tx.commit()
            .await
            .context("committing an aborted transaction does not report an error")?;

        let entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ledger_entries")
            .fetch_one(&pool)
            .await
            .context("counting ledger entries")?;
        assert_eq!(
            entries, 0,
            "... and nothing was written, including the posting that succeeded. A duplicate must \
             not be swallowed inside the transaction that raised it; the caller has to abandon \
             and re-read, or take a SAVEPOINT."
        );

        Ok(())
    }

    /// Every [`AccountKind`] variant maps to a label migration `0005`'s
    /// `CREATE TYPE account_kind` declares, and every [`Direction`] to one of
    /// `CREATE TYPE direction`'s.
    ///
    /// Container-free on purpose — the value of this test is that it fails at
    /// `cargo test` rather than at a database round trip, and adding a
    /// variant to either enum makes the `match` in `account_label` fail to
    /// compile before it can reach here.
    #[test]
    fn every_label_is_one_the_schema_declares() {
        const ACCOUNT_KIND_LABELS: [&str; 3] =
            ["merchant_payable", "payer_clearing", "platform_fee_revenue"];
        const DIRECTION_LABELS: [&str; 2] = ["debit", "credit"];

        for account in [
            AccountKind::merchant_payable("merchant_1"),
            AccountKind::PayerClearing,
            AccountKind::PlatformFeeRevenue,
        ] {
            assert!(
                ACCOUNT_KIND_LABELS.contains(&account_label(&account)),
                "{account:?} maps to a label `CREATE TYPE account_kind` does not declare"
            );
        }
        for direction in [Direction::Debit, Direction::Credit] {
            assert!(
                DIRECTION_LABELS.contains(&direction_label(direction)),
                "{direction:?} maps to a label `CREATE TYPE direction` does not declare"
            );
        }

        // And the mapping is injective, so two accounts cannot collapse onto
        // one column value.
        assert_ne!(
            account_label(&AccountKind::PayerClearing),
            account_label(&AccountKind::PlatformFeeRevenue)
        );
    }

    /// The merchant id travels with the account and only with the account —
    /// the property `ledger_entries_merchant_id_iff_merchant_payable`
    /// mirrors, checked on the Rust side of the bind so that the CHECK is a
    /// backstop rather than the only guard.
    #[test]
    fn only_a_merchant_payable_bind_carries_a_merchant_id() {
        for account in [AccountKind::PayerClearing, AccountKind::PlatformFeeRevenue] {
            assert_eq!(account.merchant_id(), None);
        }
        assert_eq!(
            AccountKind::merchant_payable("merchant_1").merchant_id(),
            Some("merchant_1")
        );
    }

    /// [`entry_id`] is injective over `(transaction_id, index)` — for every
    /// transaction id, not only the ones [`vpay_core::ids`] mints.
    ///
    /// # Why this is asserted and not argued
    ///
    /// RFC-0003 § 4's amendment recorded the opposite as an open gap on a
    /// money table, named `x` and `x_0` as a colliding pair, and asked a
    /// later branch to close it at the writer with a shape check and a new
    /// `DbError` variant. There is nothing to close: the index is decimal,
    /// decimal contains no `_`, so the last `_` of an entry id recovers the
    /// pair that made it. A paragraph saying so is exactly what got the
    /// question wrong the first time, which is why this is a test.
    ///
    /// The family below is adversarial on purpose — every id is built from
    /// `_` and the two digits that appear in small indices, which is the only
    /// alphabet that can produce a collision if one is producible at all.
    /// Ordinary `lt_…` ids would prove nothing here.
    ///
    /// It fails if the separator is changed to something an index can
    /// contain, or if the index stops being rendered in a base that excludes
    /// it — which are the two ways this could actually break, and neither is
    /// visible to `ledger_entries_pkey` until rows collide in production.
    #[test]
    fn the_entry_id_derivation_is_injective() {
        // Every string of length 1..=4 over `{_, 0, 1, p}`, crossed with the
        // indices a posting can reach. `_` and the digits are in there
        // because they are the characters the derivation itself uses.
        let mut level = vec![String::new()];
        let mut transaction_ids: Vec<String> = Vec::new();
        for _ in 0..4 {
            level = level
                .iter()
                .flat_map(|base| ['_', '0', '1', 'p'].map(|c| format!("{base}{c}")))
                .collect();
            transaction_ids.extend(level.iter().cloned());
        }

        let mut seen: std::collections::HashMap<String, (String, usize)> =
            std::collections::HashMap::new();
        for id in &transaction_ids {
            for index in 0..13 {
                let derived = entry_id(id, index);
                if let Some(previous) = seen.insert(derived.clone(), (id.clone(), index)) {
                    panic!(
                        "{derived} is derived by both {previous:?} and {:?}; \
                         `ledger_entries.id` would collide across two transactions",
                        (id, index)
                    );
                }
            }
        }
        assert_eq!(
            seen.len(),
            transaction_ids.len() * 13,
            "every (transaction_id, index) pair must have produced its own entry id"
        );

        // And the pair the amendment named, spelled out, because it is the
        // one a reader will come here to check.
        assert_eq!(entry_id("x", 0), "x_0");
        assert_eq!(entry_id("x_0", 0), "x_0_0");
        assert_ne!(entry_id("x", 0), entry_id("x_0", 0));
    }
}
