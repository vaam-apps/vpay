//! Double-entry ledger primitives.
//!
//! Convention: `balance(account) = SUM(credit) - SUM(debit)`.
//! `merchant_payable` is credit-normal — a positive balance is money the
//! merchant received. See `docs/flows/ledger.md`.
//!
//! STATUS: types and the balancing invariant are implemented and tested.
//! Persistence is NOT implemented — see `docs/status.md`.

use vpay_core::{Money, MoneyError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Debit,
    Credit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    MerchantPayable,
    PayerClearing,
    PlatformFeeRevenue,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub account: AccountKind,
    pub direction: Direction,
    pub amount: Money,
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub entries: Vec<Entry>,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("transaction does not balance: debits {debits}, credits {credits}")]
    Unbalanced { debits: i64, credits: i64 },
    #[error("a ledger transaction needs at least two entries")]
    TooFewEntries,
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

    fn retry(&self) -> vpay_core::Retry {
        match self {
            Self::Money(inner) => inner.retry(),
            _ => vpay_core::Retry::Never,
        }
    }

    fn severity(&self) -> vpay_core::Severity {
        match self {
            Self::Money(inner) => inner.severity(),
            _ => vpay_core::Severity::Page,
        }
    }

    fn public_message(&self) -> String {
        match self {
            // Internal: the merchant learns nothing about the ledger.
            Self::Money(inner) => inner.public_message(),
            _ => self.category().generic_message().to_owned(),
        }
    }
}

impl Transaction {
    /// # Errors
    /// [`LedgerError::Unbalanced`] unless debits equal credits.
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
