//! Money as integer minor units. There is no floating point in this crate,
//! and `clippy::float_arithmetic` is denied workspace-wide to keep it that way.
//!
//! See `docs/flows/money.md` for why XAF is zero-decimal and what that means
//! for serialisation to a provider.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An ISO-4217 currency together with its minor-unit exponent.
///
/// The exponent is a property of the *currency*, universally — not of a
/// deployment or an environment. `XAF` is zero-decimal everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// Central African CFA franc. Zero-decimal.
    Xaf,
    /// Euro. Two-decimal. Used only because MTN's sandbox rejects XAF.
    Eur,
}

impl Currency {
    /// Minor units per major unit, as a power of ten.
    #[must_use]
    pub const fn exponent(self) -> u32 {
        match self {
            Self::Xaf => 0,
            Self::Eur => 2,
        }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Xaf => "XAF",
            Self::Eur => "EUR",
        }
    }

    /// Parse an uppercase ISO code. The wire API uses lowercase and normalises
    /// on ingress; the database and adapters use uppercase.
    pub fn from_code(s: &str) -> Result<Self, UnknownCurrency> {
        match s {
            "XAF" => Ok(Self::Xaf),
            "EUR" => Ok(Self::Eur),
            other => Err(UnknownCurrency(other.to_owned())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown currency: {0}")]
pub struct UnknownCurrency(pub String);

/// A non-negative amount in a currency's minor unit.
///
/// `Money::new(5_000, Currency::Xaf)` is 5,000 FCFA — not 50.00, because XAF
/// has no centimes in circulating use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    minor: i64,
    currency: Currency,
}

impl Money {
    /// # Errors
    /// Returns [`MoneyError::Negative`] for a negative amount.
    pub fn new(minor: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor < 0 {
            return Err(MoneyError::Negative(minor));
        }
        Ok(Self { minor, currency })
    }

    #[must_use]
    pub const fn minor(self) -> i64 {
        self.minor
    }

    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }

    /// # Errors
    /// [`MoneyError::CurrencyMismatch`] if the currencies differ,
    /// [`MoneyError::Overflow`] on i64 overflow.
    pub fn checked_add(self, other: Self) -> Result<Self, MoneyError> {
        self.same_currency(other)?;
        let minor = self
            .minor
            .checked_add(other.minor)
            .ok_or(MoneyError::Overflow)?;
        Ok(Self { minor, ..self })
    }

    /// # Errors
    /// [`MoneyError::CurrencyMismatch`], or [`MoneyError::Negative`] if the
    /// result would go below zero — a refund can never exceed what was captured.
    pub fn checked_sub(self, other: Self) -> Result<Self, MoneyError> {
        self.same_currency(other)?;
        let minor = self
            .minor
            .checked_sub(other.minor)
            .ok_or(MoneyError::Overflow)?;
        Self::new(minor, self.currency)
    }

    fn same_currency(self, other: Self) -> Result<(), MoneyError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(MoneyError::CurrencyMismatch {
                left: self.currency,
                right: other.currency,
            })
        }
    }

    /// Render for transmission to a provider, using the currency's exponent.
    ///
    /// This is the single conversion point referenced by `docs/flows/money.md`.
    /// XAF (exponent 0) renders `5000`; EUR (exponent 2) renders `50.00`.
    #[must_use]
    pub fn to_provider_string(self) -> String {
        let exp = self.currency.exponent();
        if exp == 0 {
            return self.minor.to_string();
        }
        let divisor = 10_i64.pow(exp);
        let major = self.minor / divisor;
        let frac = self.minor % divisor;
        format!("{major}.{frac:0width$}", width = exp as usize)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.to_provider_string(), self.currency.code())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("amount must not be negative, got {0}")]
    Negative(i64),
    #[error("currency mismatch: {left:?} vs {right:?}")]
    CurrencyMismatch { left: Currency, right: Currency },
    #[error("arithmetic overflow")]
    Overflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xaf(n: i64) -> Money {
        Money::new(n, Currency::Xaf).expect("non-negative")
    }

    #[test]
    fn xaf_is_zero_decimal_on_the_wire() {
        assert_eq!(xaf(5_000).to_provider_string(), "5000");
    }

    #[test]
    fn eur_renders_two_decimals() {
        let m = Money::new(5_000, Currency::Eur).expect("non-negative");
        assert_eq!(m.to_provider_string(), "50.00");
    }

    #[test]
    fn eur_pads_the_fractional_part() {
        let m = Money::new(5_005, Currency::Eur).expect("non-negative");
        assert_eq!(m.to_provider_string(), "50.05");
    }

    #[test]
    fn negative_amounts_are_rejected() {
        assert_eq!(Money::new(-1, Currency::Xaf), Err(MoneyError::Negative(-1)));
    }

    #[test]
    fn cross_currency_arithmetic_is_rejected() {
        let a = xaf(100);
        let b = Money::new(100, Currency::Eur).expect("non-negative");
        assert!(matches!(
            a.checked_add(b),
            Err(MoneyError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn refunding_more_than_captured_is_rejected() {
        assert!(matches!(
            xaf(100).checked_sub(xaf(101)),
            Err(MoneyError::Negative(_))
        ));
    }
}
