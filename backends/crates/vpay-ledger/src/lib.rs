//! Double-entry ledger primitives.
//!
//! Convention: `balance(account) = SUM(credit) - SUM(debit)`.
//! `merchant_payable` is credit-normal — a positive balance is money the
//! merchant received. See `docs/flows/ledger.md`.
//!
//! STATUS: types, the balancing invariant and the per-merchant dimension are
//! implemented and tested here. **Nothing in a shipping binary posts a
//! transaction yet**: `vpay_db::TxRepositories::post_ledger_transaction_in_tx`
//! is the writer (RFC-0003 § 4), and wiring it into
//! `Settlement::apply_succeeded` / `apply_refund_succeeded` is a separate
//! piece of work — see `docs/status.md` and `docs/flows/ledger.md` § Status.
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
//!         entry(
//!             AccountKind::merchant_payable("merchant_1"),
//!             Direction::Credit,
//!             4_900,
//!         ),
//!         entry(AccountKind::PlatformFeeRevenue, Direction::Credit, 100),
//!     ],
//! };
//! assert!(capture.validate().is_ok());
//! ```

use vpay_core::{Currency, Money, MoneyError};

/// Which side of an account an entry lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Decreases a credit-normal account's balance.
    Debit,
    /// Increases a credit-normal account's balance.
    Credit,
}

/// The accounts vpay keeps. Not merchant-configurable: the chart of accounts
/// is part of the settlement model, not of a deployment.
///
/// # Why `MerchantPayable` carries its merchant, and the other two do not
///
/// `docs/flows/ledger.md` invariant 2 is "per merchant:
/// `balance(merchant_payable) = Σ captures − Σ fees − Σ refunds`". Until
/// RFC-0003 § 4 this type had three payload-free variants, so that sentence
/// was **uncomputable** — nothing said which merchant a `merchant_payable`
/// posting belonged to. Both `docs/flows/ledger.md` and migration `0005`'s
/// own `GAP` comment recorded it as a gap in *this type*, to be closed here
/// rather than by adding a `merchant_id` column the Rust side did not have.
///
/// It is a variant payload rather than a field on [`Entry`] because the two
/// are not the same claim. A field would admit a `payer_clearing` posting
/// carrying a merchant (meaningless — the clearing account is vpay's, pooled
/// across tenants) and a `merchant_payable` posting carrying none (the gap,
/// reintroduced). The sum type makes exactly one of the three variants
/// tenant-scoped and the other two not, which is what `ledger_entries`'
/// `ledger_entries_merchant_id_iff_merchant_payable` CHECK mirrors in SQL
/// (migration `0045`).
///
/// **Not `Copy` since that change**, unlike [`Direction`]: the merchant id is
/// an owned `String`, because an id is borrowed at every boundary in this
/// workspace and owned only where it is stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AccountKind {
    /// Money owed to one merchant. Credit-normal.
    MerchantPayable {
        /// Whose money it is — `payment_intents.merchant_id`, verbatim.
        merchant_id: String,
    },
    /// Money received from payers and not yet allocated. vpay's own, pooled
    /// across every tenant, so it carries no merchant.
    PayerClearing,
    /// vpay's own fee income. Pooled for [`AccountKind::PayerClearing`]'s
    /// reason.
    PlatformFeeRevenue,
}

impl AccountKind {
    /// [`AccountKind::MerchantPayable`] for `merchant_id`.
    ///
    /// A constructor rather than a struct literal at every call site because
    /// the variant is written far more often than it is matched, and
    /// `AccountKind::merchant_payable("acme")` reads as the account it names.
    #[must_use]
    pub fn merchant_payable(merchant_id: impl Into<String>) -> Self {
        Self::MerchantPayable {
            merchant_id: merchant_id.into(),
        }
    }

    /// The merchant this account belongs to, or `None` for the two vpay
    /// keeps pooled.
    ///
    /// ```
    /// use vpay_ledger::AccountKind;
    ///
    /// assert_eq!(
    ///     AccountKind::merchant_payable("acme").merchant_id(),
    ///     Some("acme")
    /// );
    /// assert_eq!(AccountKind::PayerClearing.merchant_id(), None);
    /// ```
    #[must_use]
    pub fn merchant_id(&self) -> Option<&str> {
        match self {
            Self::MerchantPayable { merchant_id } => Some(merchant_id),
            Self::PayerClearing | Self::PlatformFeeRevenue => None,
        }
    }
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
    /// The capture posting of `docs/flows/ledger.md` § Postings: the payer's
    /// clearing account is debited the gross, the merchant is credited what
    /// is left after the platform fee, and the fee — when there is one — is
    /// credited to vpay.
    ///
    /// `fee` of `None` **and** a fee of zero both produce the two-leg form.
    /// That is deliberate: a zero-amount leg would post a row that moves no
    /// money, and would make the number of entries a transaction has depend
    /// on whether a merchant happens to be on a zero-rate plan.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Money`] wrapping [`MoneyError::CurrencyMismatch`] if
    /// `fee` is in a different currency from `gross`, or
    /// [`MoneyError::Negative`] if the fee exceeds the gross — vpay never
    /// posts a capture that leaves the merchant owing money.
    ///
    /// ```
    /// use vpay_core::{Currency, Money};
    /// use vpay_ledger::{AccountKind, Transaction};
    ///
    /// let xaf = |n| Money::new(n, Currency::Xaf).expect("non-negative");
    /// let with_fee = Transaction::capture("acme", xaf(5_000), Some(xaf(100)))
    ///     .expect("a fee below the gross");
    /// assert_eq!(with_fee.entries.len(), 3);
    /// assert!(with_fee.validate().is_ok());
    ///
    /// // A zero fee is the two-leg form, not a three-leg one with a nil leg.
    /// let free = Transaction::capture("acme", xaf(5_000), Some(xaf(0)))
    ///     .expect("a zero fee");
    /// assert_eq!(free.entries.len(), 2);
    ///
    /// // The merchant's leg names the merchant.
    /// assert_eq!(
    ///     free.entries[1].account,
    ///     AccountKind::merchant_payable("acme")
    /// );
    /// ```
    pub fn capture(
        merchant_id: &str,
        gross: Money,
        fee: Option<Money>,
    ) -> Result<Self, LedgerError> {
        let fee = fee.filter(|fee| fee.minor() > 0);
        let net = match fee {
            Some(fee) => gross.checked_sub(fee)?,
            None => gross,
        };

        let mut entries = vec![
            Entry {
                account: AccountKind::PayerClearing,
                direction: Direction::Debit,
                amount: gross,
            },
            Entry {
                account: AccountKind::merchant_payable(merchant_id),
                direction: Direction::Credit,
                amount: net,
            },
        ];
        if let Some(fee) = fee {
            entries.push(Entry {
                account: AccountKind::PlatformFeeRevenue,
                direction: Direction::Credit,
                amount: fee,
            });
        }

        Ok(Self { entries })
    }

    /// The refund posting of `docs/flows/ledger.md` § Postings: the merchant
    /// is debited, the payer's clearing account is credited.
    ///
    /// **Two legs, never three.** The rail's refund fee is reported on the
    /// `refund` object and posted to no account — issue #46's decision, which
    /// RFC-0003 § 4 leaves standing — so there is no `fee` parameter here for
    /// a caller to be tempted to pass. Posting it would need a rule about who
    /// bears it and would add a second kind of term to invariant 2; both are
    /// spelled out in `docs/flows/ledger.md` § "A rail-charged refund fee is
    /// reported, not posted".
    ///
    /// ```
    /// use vpay_core::{Currency, Money};
    /// use vpay_ledger::Transaction;
    ///
    /// let amount = Money::new(2_000, Currency::Xaf).expect("non-negative");
    /// let refund = Transaction::refund("acme", amount);
    /// assert_eq!(refund.entries.len(), 2);
    /// assert!(refund.validate().is_ok());
    /// ```
    #[must_use]
    pub fn refund(merchant_id: &str, amount: Money) -> Self {
        Self {
            entries: vec![
                Entry {
                    account: AccountKind::merchant_payable(merchant_id),
                    direction: Direction::Debit,
                    amount,
                },
                Entry {
                    account: AccountKind::PayerClearing,
                    direction: Direction::Credit,
                    amount,
                },
            ],
        }
    }

    /// Checks the double-entry invariant: at least two legs, debits equal to
    /// credits.
    ///
    /// **Not per currency, which `docs/flows/ledger.md` invariant 1 is** —
    /// this sums minor units across every leg whatever currency it is in, so
    /// a hand-built transaction debiting 100 XAF and crediting 100 EUR passes
    /// it. That is a pre-existing gap, unchanged by RFC-0003 § 4 and recorded
    /// here rather than quietly fixed: neither [`Transaction::capture`] nor
    /// [`Transaction::refund`] can produce such a transaction (both build
    /// every leg from one [`Money`], and `capture` rejects a fee in another
    /// currency), so nothing that reaches the database today can trip it.
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
    ///         leg(
    ///             AccountKind::merchant_payable("acme"),
    ///             Direction::Credit,
    ///             4_900,
    ///         ),
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

/// `balance(account) = Σ credit − Σ debit` over `entries`, in `currency`.
///
/// The in-memory half of `docs/flows/ledger.md`'s invariant 2. It exists so
/// that "merchant A's postings do not move merchant B's balance" is something
/// a test can assert rather than a property of a `WHERE` clause nobody has
/// written yet; the database half of the same question is
/// `vpay_db::Ledger::merchant_payable_balance`, and the two agree because
/// both key on the whole of [`AccountKind`] — including the merchant id the
/// `MerchantPayable` variant carries.
///
/// `currency` is a parameter rather than inferred from the entries because
/// the convention in `docs/flows/ledger.md` is explicitly *per currency*: an
/// account holding both XAF and EUR postings has two balances, not one, and
/// summing minor units across them would produce a number in no currency at
/// all.
///
/// Returns minor units rather than [`Money`], because a balance may legally
/// be negative — `payer_clearing` always is, and a merchant's is while a
/// refund outruns a capture — and [`Money`] is non-negative by construction.
///
/// ```
/// use vpay_core::{Currency, Money};
/// use vpay_ledger::{AccountKind, Transaction, balance};
///
/// let xaf = |n| Money::new(n, Currency::Xaf).expect("non-negative");
/// let capture = Transaction::capture("acme", xaf(5_000), Some(xaf(100)))
///     .expect("a fee below the gross");
/// let refund = Transaction::refund("acme", xaf(2_000));
///
/// let entries: Vec<_> = capture
///     .entries
///     .iter()
///     .chain(refund.entries.iter())
///     .collect();
///
/// // 4 900 credited, 2 000 debited.
/// assert_eq!(
///     balance(
///         entries.iter().copied(),
///         &AccountKind::merchant_payable("acme"),
///         Currency::Xaf
///     ),
///     2_900
/// );
/// // Another merchant's balance is untouched by both.
/// assert_eq!(
///     balance(
///         entries.iter().copied(),
///         &AccountKind::merchant_payable("globex"),
///         Currency::Xaf
///     ),
///     0
/// );
/// ```
#[must_use]
pub fn balance<'a, I>(entries: I, account: &AccountKind, currency: Currency) -> i64
where
    I: IntoIterator<Item = &'a Entry>,
{
    entries
        .into_iter()
        .filter(|entry| entry.account == *account && entry.amount.currency() == currency)
        .fold(0_i64, |running, entry| match entry.direction {
            Direction::Credit => running.saturating_add(entry.amount.minor()),
            Direction::Debit => running.saturating_sub(entry.amount.minor()),
        })
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
                entry(
                    AccountKind::merchant_payable("merchant_1"),
                    Direction::Credit,
                    4_900,
                ),
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
                entry(
                    AccountKind::merchant_payable("merchant_1"),
                    Direction::Credit,
                    4_900,
                ),
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

    #[test]
    fn only_the_merchant_payable_account_carries_a_merchant() {
        assert_eq!(
            AccountKind::merchant_payable("merchant_1").merchant_id(),
            Some("merchant_1")
        );
        assert_eq!(AccountKind::PayerClearing.merchant_id(), None);
        assert_eq!(AccountKind::PlatformFeeRevenue.merchant_id(), None);
    }

    #[test]
    fn two_merchants_payable_accounts_are_different_accounts() {
        assert_ne!(
            AccountKind::merchant_payable("merchant_1"),
            AccountKind::merchant_payable("merchant_2")
        );
        // And the same merchant twice is the same account, so the inequality
        // above is about the id and not about `String` identity.
        assert_eq!(
            AccountKind::merchant_payable("merchant_1"),
            AccountKind::merchant_payable("merchant_1")
        );
    }

    /// `docs/flows/ledger.md` invariant 2, asserted for two merchants at
    /// once — the question the three-variant `AccountKind` could not be
    /// asked.
    ///
    /// **The decisive property is the cross-merchant one.** Merchant 2's
    /// capture is four times merchant 1's and merchant 1 alone is refunded,
    /// so a `balance` that ignored the merchant id would answer 22 500 for
    /// both and fail both assertions rather than neither.
    #[test]
    fn two_merchants_payable_balances_do_not_mix() {
        let entries: Vec<Entry> = [
            Transaction::capture("merchant_1", xaf(5_000), Some(xaf(100)))
                .expect("a fee below the gross"),
            Transaction::capture("merchant_2", xaf(20_000), Some(xaf(400)))
                .expect("a fee below the gross"),
            Transaction::refund("merchant_1", xaf(2_000)),
        ]
        .into_iter()
        .flat_map(|tx| tx.entries)
        .collect();

        // 4 900 captured net of fee, 2 000 refunded.
        assert_eq!(
            balance(
                &entries,
                &AccountKind::merchant_payable("merchant_1"),
                Currency::Xaf
            ),
            2_900
        );
        // 19 600 captured net of fee, nothing refunded — merchant 1's refund
        // is not merchant 2's.
        assert_eq!(
            balance(
                &entries,
                &AccountKind::merchant_payable("merchant_2"),
                Currency::Xaf
            ),
            19_600
        );
        // A merchant that has never traded has a zero balance, not the sum of
        // everyone else's.
        assert_eq!(
            balance(
                &entries,
                &AccountKind::merchant_payable("merchant_3"),
                Currency::Xaf
            ),
            0
        );

        // The two pooled accounts carry both merchants' flow and are not
        // split by merchant, which is the other half of the modelling claim.
        assert_eq!(
            balance(&entries, &AccountKind::PayerClearing, Currency::Xaf),
            -23_000
        );
        assert_eq!(
            balance(&entries, &AccountKind::PlatformFeeRevenue, Currency::Xaf),
            500
        );
    }

    /// A balance is per currency. Two captures of the same minor amount in
    /// XAF and EUR are two balances, not one of 10 000.
    #[test]
    fn a_balance_is_per_currency() {
        let eur = Money::new(5_000, Currency::Eur).expect("non-negative");
        let entries: Vec<Entry> = [
            Transaction::capture("merchant_1", xaf(5_000), None).expect("no fee"),
            Transaction::capture("merchant_1", eur, None).expect("no fee"),
        ]
        .into_iter()
        .flat_map(|tx| tx.entries)
        .collect();

        let account = AccountKind::merchant_payable("merchant_1");
        assert_eq!(balance(&entries, &account, Currency::Xaf), 5_000);
        assert_eq!(balance(&entries, &account, Currency::Eur), 5_000);
    }

    #[test]
    fn a_capture_whose_fee_exceeds_the_gross_is_refused() {
        let error = Transaction::capture("merchant_1", xaf(100), Some(xaf(101)))
            .expect_err("a fee above the gross leaves the merchant owing money");
        assert!(matches!(
            error,
            LedgerError::Money(MoneyError::Negative(-1))
        ));
    }

    #[test]
    fn a_capture_fee_in_another_currency_is_refused() {
        let eur = Money::new(100, Currency::Eur).expect("non-negative");
        let error = Transaction::capture("merchant_1", xaf(5_000), Some(eur))
            .expect_err("a fee in another currency");
        assert!(matches!(
            error,
            LedgerError::Money(MoneyError::CurrencyMismatch { .. })
        ));
    }

    /// Both postings `docs/flows/ledger.md` § Postings tabulates, built by
    /// the constructors rather than by hand, balance.
    #[test]
    fn the_documented_postings_balance() {
        assert!(
            Transaction::capture("merchant_1", xaf(5_000), None)
                .expect("no fee")
                .validate()
                .is_ok()
        );
        assert!(
            Transaction::capture("merchant_1", xaf(5_000), Some(xaf(100)))
                .expect("a fee below the gross")
                .validate()
                .is_ok()
        );
        assert!(
            Transaction::refund("merchant_1", xaf(2_000))
                .validate()
                .is_ok()
        );
    }
}
