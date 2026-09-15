//! The `ledger_transactions` / `ledger_entries` repository — the first code
//! in this repository's history to write either table (RFC-0003 § 4).
//!
//! # What exists here, and what does not
//!
//! One write, [`post_in_tx`], and one read,
//! [`Ledger::merchant_payable_balance`]. The write is `pub(crate)` and is
//! reached from outside this crate only through
//! [`crate::TxRepositories::post_ledger_transaction_in_tx`] — the shape
//! [`crate::refunds::settle_in_tx`] has, and for the same reason: a ledger
//! posting has to commit with the settlement that caused it, so the only
//! spelling available to a consumer is one that already holds a transaction.
//! There is deliberately **no pooled variant**; a `post` that opened its own
//! transaction would make "the ledger agrees with the charge" a property of
//! whoever remembered to call it in the right place.
//!
//! **No shipping code path calls the write yet, and that is not an
//! oversight.** `Settlement::apply_succeeded` and
//! `Settlement::apply_refund_succeeded` do not post; wiring them is separate
//! work, so every deployment's ledger is empty today. `docs/status.md` and
//! `docs/flows/ledger.md` § Status say so. What this module removes is the
//! reason they could not: the machinery, the schema and the merchant
//! dimension.
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
        // Not a statement — `crate::sql_audit` scans only a `format!` whose
        // result is bound to a `sql` variable, and this one builds a value
        // that is passed as `$1`. It is spelled with `format!` for the same
        // reason every other id in this crate is a `&str`: the database is
        // never handed a fragment, only a parameter.
        let entry_id = format!("{transaction_id}_{index}");
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
/// is **not** on this trait, for [`crate::refunds::Refunds`]' reason: it is
/// `pub(crate)` and belongs to the caller's transaction, so a consumer cannot
/// post to the ledger without the settlement that justifies the posting.
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
    use super::*;

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
}
