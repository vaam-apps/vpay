//! Double-entry ledger primitives.
//!
//! Convention: `balance(account) = SUM(credit) - SUM(debit)`.
//! `merchant_payable` is credit-normal — a positive balance is money the
//! merchant received. See `docs/flows/ledger.md`.
//!
//! STATUS: types and the balancing invariant are implemented and tested.
//! Persistence is NOT implemented — see `docs/status.md`.
//!
//! ```
//! use vpay_core::{Currency, Money};
//! use vpay_ledger::{AccountKind, Direction, Entry, Transaction};
//!
//! let xaf = |n| Money::new(n, Currency::Xaf).expect("non-negative");
//! let entry = |account, direction, n| Entry {
//!     account,
//!     direction,
//!     amount: xaf(n),
//! };
//!
//! // A 5,000 FCFA capture with a 100 FCFA platform fee.
//! let capture = Transaction {
//!     entries: vec![
//!         entry(AccountKind::PayerClearing, Direction::Debit, 5_000),
//!         entry(AccountKind::MerchantPayable, Direction::Credit, 4_900),
//!         entry(AccountKind::PlatformFeeRevenue, Direction::Credit, 100),
//!     ],
//! };
//! assert!(capture.validate().is_ok());
//! ```

use vpay_core::{Money, MoneyError};

/// Which side of an account an entry lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Decreases a credit-normal account's balance.
    Debit,
    /// Increases a credit-normal account's balance.
    Credit,
}

/// The accounts vpay keeps. Not merchant-configurable: the chart of accounts
/// is part of the settlement model, not of a deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    /// Money owed to the merchant. Credit-normal.
    MerchantPayable,
    /// Money received from payers and not yet allocated.
    PayerClearing,
    /// vpay's own fee income.
    PlatformFeeRevenue,
}

/// One leg of a [`Transaction`].
#[derive(Debug, Clone)]
pub struct Entry {
    /// The account this leg moves.
    pub account: AccountKind,
    /// Which way it moves.
    pub direction: Direction,
    /// How much, in integer minor units.
    pub amount: Money,
}

/// A set of entries that must balance before it may be recorded.
#[derive(Debug, Clone)]
pub struct Transaction {
    /// The legs, at least two of them, debits summing to credits.
    pub entries: Vec<Entry>,
}

/// What can go wrong building a ledger transaction.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// Debits and credits do not sum to the same number.
    #[error("transaction does not balance: debits {debits}, credits {credits}")]
    Unbalanced {
        /// The sum of every debit leg, in minor units.
        debits: i64,
        /// The sum of every credit leg, in minor units.
        credits: i64,
    },
    /// Fewer than two legs — a single-legged transaction cannot balance and
    /// is not a double entry.
    #[error("a ledger transaction needs at least two entries")]
    TooFewEntries,
    /// An amount was not constructible.
    #[error(transparent)]
    Money(#[from] MoneyError),
}

impl vpay_core::Classify for LedgerError {
    fn category(&self) -> vpay_core::Category {
        match self {
            // No caller builds a ledger transaction; the core does, from
            // amounts it already validated. An unbalanced or degenerate one
            // is therefore this code's own invariant failing — the most
            // expensive kind of bug this system can have (ADR-0007).
            Self::Unbalanced { .. } | Self::TooFewEntries => vpay_core::Category::Internal,
            Self::Money(inner) => inner.category(),
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unbalanced { .. } => "ledger_unbalanced",
            Self::TooFewEntries => "ledger_degenerate",
            Self::Money(inner) => inner.code(),
        }
    }

    // The three methods below are exhaustive rather than
    // `Self::Money(..) => .., _ => ..`: a wildcard would silently give a new
    // variant the *invariant-violation* policy (never retry, page, say
    // nothing), which is right for the two that exist and would be a lie for,
    // say, a future `AccountNotFound`. Adding a variant should not compile
    // until someone has decided.
    fn retry(&self) -> vpay_core::Retry {
        match self {
            Self::Money(inner) => inner.retry(),
            Self::Unbalanced { .. } | Self::TooFewEntries => vpay_core::Retry::Never,
        }
    }

    fn severity(&self) -> vpay_core::Severity {
        match self {
            Self::Money(inner) => inner.severity(),
            Self::Unbalanced { .. } | Self::TooFewEntries => vpay_core::Severity::Page,
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Money(inner) => inner.public_message(),
            // Internal: the merchant learns nothing about the ledger.
            Self::Unbalanced { .. } | Self::TooFewEntries => {
                self.category().generic_message().to_owned()
            }
        }
    }
}

impl Transaction {
    /// Checks the double-entry invariant: at least two legs, debits equal to
    /// credits.
    ///
    /// # Errors
    /// [`LedgerError::TooFewEntries`] for fewer than two legs,
    /// [`LedgerError::Unbalanced`] unless debits equal credits.
    ///
    /// ```
    /// use vpay_core::{Classify, Currency, Money, Severity};
    /// use vpay_ledger::{AccountKind, Direction, Entry, LedgerError, Transaction};
    ///
    /// let leg = |account, direction, n| Entry {
    ///     account,
    ///     direction,
    ///     amount: Money::new(n, Currency::Xaf).expect("non-negative"),
    /// };
    ///
    /// // 100 francs unaccounted for.
    /// let lopsided = Transaction {
    ///     entries: vec![
    ///         leg(AccountKind::PayerClearing, Direction::Debit, 5_000),
    ///         leg(AccountKind::MerchantPayable, Direction::Credit, 4_900),
    ///     ],
    /// };
    /// let error = lopsided.validate().expect_err("that does not balance");
    /// assert!(matches!(
    ///     error,
    ///     LedgerError::Unbalanced {
    ///         debits: 5_000,
    ///         credits: 4_900
    ///     }
    /// ));
    /// // Nobody outside this code can cause it, so it pages and tells the
    /// // merchant nothing about the ledger.
    /// assert_eq!(error.severity(), Severity::Page);
    ///
    /// // One leg is not a double entry.
    /// let single = Transaction {
    ///     entries: vec![leg(AccountKind::PayerClearing, Direction::Debit, 1)],
    /// };
    /// assert!(matches!(
    ///     single.validate(),
    ///     Err(LedgerError::TooFewEntries)
    /// ));
    /// ```
    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.entries.len() < 2 {
            return Err(LedgerError::TooFewEntries);
        }
        let mut debits: i64 = 0;
        let mut credits: i64 = 0;
        for e in &self.entries {
            match e.direction {
                Direction::Debit => debits = debits.saturating_add(e.amount.minor()),
                Direction::Credit => credits = credits.saturating_add(e.amount.minor()),
            }
        }
        if debits == credits {
            Ok(())
        } else {
            Err(LedgerError::Unbalanced { debits, credits })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpay_core::Currency;

    fn xaf(n: i64) -> Money {
        Money::new(n, Currency::Xaf).expect("non-negative")
    }

    fn entry(account: AccountKind, direction: Direction, n: i64) -> Entry {
        Entry {
            account,
            direction,
            amount: xaf(n),
        }
    }

    #[test]
    fn a_capture_with_a_fee_balances() {
        let tx = Transaction {
            entries: vec![
                entry(AccountKind::PayerClearing, Direction::Debit, 5_000),
                entry(AccountKind::MerchantPayable, Direction::Credit, 4_900),
                entry(AccountKind::PlatformFeeRevenue, Direction::Credit, 100),
            ],
        };
        assert!(tx.validate().is_ok());
    }

    #[test]
    fn an_unbalanced_transaction_is_rejected() {
        let tx = Transaction {
            entries: vec![
                entry(AccountKind::PayerClearing, Direction::Debit, 5_000),
                entry(AccountKind::MerchantPayable, Direction::Credit, 4_900),
            ],
        };
        assert!(matches!(tx.validate(), Err(LedgerError::Unbalanced { .. })));
    }

    #[test]
    fn a_single_legged_transaction_is_rejected() {
        let tx = Transaction {
            entries: vec![entry(AccountKind::PayerClearing, Direction::Debit, 1)],
        };
        assert!(matches!(tx.validate(), Err(LedgerError::TooFewEntries)));
    }
}
