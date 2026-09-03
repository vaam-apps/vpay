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

    /// The amount as an integer count of minor units, for a rail whose API
    /// takes a JSON *number* rather than a decimal string.
    ///
    /// The same conversion as [`Money::to_provider_string`] in a different
    /// encoding, not a second conversion: both read the exponent from the
    /// currency and neither scales anything. Orange Money's `webpayment`
    /// body sends `"amount": 5000` (`docs/flows/adapter-orange-money.md`),
    /// while MTN's sends `"amount": "5000"` — one exponent lookup, two
    /// renderings, which is what `docs/flows/money.md`'s "single conversion
    /// point" rule means now that a rail needs the other encoding.
    ///
    /// # The mistake this can be used to make
    ///
    /// It returns *minor* units. Handing `5000` to a rail that expects major
    /// units is 100× on a two-decimal currency and nothing can detect it
    /// downstream — the number is valid, the currency is right, and the
    /// charge succeeds. An adapter that reaches for this must have read its
    /// rail's documentation and found the word "minor" (or a zero-decimal
    /// currency, where the question does not arise).
    ///
    /// ```
    /// use vpay_core::{Currency, Money};
    /// // EUR is two-decimal: 50.00 EUR is five thousand *cents*.
    /// let fifty_euro = Money::new(5_000, Currency::Eur).expect("non-negative");
    /// assert_eq!(fifty_euro.to_provider_string(), "50.00");
    /// assert_eq!(fifty_euro.to_provider_minor(), 5_000);
    /// // Not 50. A rail expecting major units must be sent the string form
    /// // or a number this function does not produce.
    /// assert_ne!(fifty_euro.to_provider_minor(), 50);
    /// ```
    #[must_use]
    pub const fn to_provider_minor(self) -> i64 {
        self.minor
    }

    /// Render for transmission to a provider, using the currency's exponent.
    ///
    /// This is the single conversion point referenced by `docs/flows/money.md`.
    /// XAF (exponent 0) renders `5000`; EUR (exponent 2) renders `50.00`.
    /// [`Money::to_provider_minor`] is the same conversion for a rail whose
    /// API takes a JSON number.
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

impl crate::error::Classify for MoneyError {
    fn category(&self) -> crate::error::Category {
        use crate::error::Category;
        match self {
            // An amount or a currency pair came from a caller's request.
            Self::Negative(_) | Self::CurrencyMismatch { .. } => Category::InvalidRequest,
            // `i64` minor units overflowing is not a request a merchant can
            // make; it is arithmetic this crate performed on values it had
            // already accepted — a bug, and one in the money path.
            Self::Overflow => Category::Internal,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Negative(_) => "amount_negative",
            Self::CurrencyMismatch { .. } => "currency_mismatch",
            Self::Overflow => "internal_error",
        }
    }

    fn public_message(&self) -> String {
        match self {
            // Caller-category errors say exactly what to fix; the value is
            // the caller's own, so echoing it leaks nothing.
            Self::Negative(n) => format!("amount must not be negative, got {n}"),
            Self::CurrencyMismatch { left, right } => {
                format!("currency mismatch: {} vs {}", left.code(), right.code())
            }
            Self::Overflow => self.category().generic_message().to_owned(),
        }
    }
}

/// How many characters of a rejected currency code may be echoed back.
///
/// An ISO-4217 code is three characters; eight is generous enough that a
/// merchant recognises what they sent (`"xaf "`, `"XAFF"`, `"xaf\n"`) and
/// short enough that the echo cannot become a reflection channel. Without a
/// bound, [`Currency::from_code`] would happily build an `UnknownCurrency`
/// around a megabyte of caller-supplied bytes and
/// [`Classify::public_message`](crate::error::Classify::public_message)
/// would put all of it in a response body.
const CURRENCY_ECHO_CHARS: usize = 8;

/// The first [`CURRENCY_ECHO_CHARS`] characters of `code`, with an ellipsis
/// iff anything was dropped.
///
/// Character-wise, not byte-wise: the code is caller-supplied text and
/// slicing at byte 8 would panic on a multi-byte boundary — ADR-0007 denies
/// panics in production code, and this runs on a request path. Nothing is
/// appended when the input already fits, so the overwhelmingly common case
/// (a three-letter typo) reads back exactly as the caller sent it.
fn echoed_currency(code: &str) -> String {
    let mut out: String = code.chars().take(CURRENCY_ECHO_CHARS).collect();
    if code.chars().nth(CURRENCY_ECHO_CHARS).is_some() {
        out.push('\u{2026}');
    }
    out
}

impl crate::error::Classify for UnknownCurrency {
    fn category(&self) -> crate::error::Category {
        crate::error::Category::InvalidRequest
    }

    fn code(&self) -> &'static str {
        "currency_unknown"
    }

    /// Echoes the offending code, bounded. The value is the caller's own so
    /// echoing it leaks nothing — but it is also *unbounded*: `from_code`
    /// accepts whatever arrived on the wire, so the public message truncates
    /// rather than trusting an upstream length check that does not exist
    /// yet. `Display` keeps the whole string, because that half goes to an
    /// operator's log, not into a response body.
    fn public_message(&self) -> String {
        format!("unknown currency: {}", echoed_currency(&self.0))
    }
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

    /// Zero-decimal: the number a rail is sent and the string a rail is sent
    /// are the same digits, because XAF's minor unit *is* its major unit.
    #[test]
    fn xaf_renders_the_same_digits_in_both_encodings() {
        assert_eq!(xaf(5_000).to_provider_minor(), 5_000);
        assert_eq!(
            xaf(5_000).to_provider_string(),
            xaf(5_000).to_provider_minor().to_string()
        );
    }

    /// Two-decimal: they are deliberately *not* the same value. This is the
    /// test that fails if someone "simplifies" `to_provider_minor` into
    /// something major-unit-shaped, which would silently divide every EUR
    /// charge by 100 on the rail that takes a number.
    #[test]
    fn eur_minor_units_are_not_the_major_amount() {
        let m = Money::new(5_000, Currency::Eur).expect("non-negative");
        assert_eq!(m.to_provider_minor(), 5_000);
        assert_eq!(m.to_provider_string(), "50.00");
        assert_ne!(m.to_provider_minor(), 50);
    }

    /// The invariant behind both renderings: neither scales anything, so the
    /// number is always exactly what `Money::minor` holds, for every
    /// currency in the table.
    #[test]
    fn to_provider_minor_is_the_stored_minor_for_every_currency() {
        for currency in [Currency::Xaf, Currency::Eur] {
            for amount in [0_i64, 1, 5_005, 9_999_999] {
                let m = Money::new(amount, currency).expect("non-negative");
                assert_eq!(m.to_provider_minor(), amount, "{currency:?} {amount}");
                assert_eq!(m.to_provider_minor(), m.minor(), "{currency:?} {amount}");
            }
        }
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

    #[test]
    fn a_short_currency_code_is_echoed_verbatim() {
        use crate::error::Classify as _;
        let e = Currency::from_code("xaf").expect_err("lowercase is not a known code");
        assert_eq!(e.public_message(), "unknown currency: xaf");
    }

    /// A megabyte of caller-supplied bytes must not become a megabyte of
    /// response body. `from_code` is the real ingress — it wraps whatever
    /// arrived — so the bound has to live in `public_message`, not in a
    /// caller that may forget.
    #[test]
    fn a_megabyte_of_currency_code_is_bounded_in_the_public_message() {
        use crate::error::Classify as _;
        let huge = "A".repeat(1024 * 1024);
        let e = Currency::from_code(&huge).expect_err("that is not a currency");
        let message = e.public_message();
        assert_eq!(message, "unknown currency: AAAAAAAA\u{2026}");
        assert!(
            message.len() < 64,
            "the public message must stay small, got {} bytes",
            message.len()
        );
        // The operator-facing half is deliberately unbounded: `Display` goes
        // to a log, never to a response body.
        assert!(e.to_string().len() > 1024 * 1024);
    }

    /// Byte-wise truncation would panic here; ADR-0007 denies panics on a
    /// request path and this runs on one.
    #[test]
    fn a_multibyte_currency_code_is_truncated_on_a_character_boundary() {
        use crate::error::Classify as _;
        let e = Currency::from_code("€€€€€€€€€€").expect_err("that is not a currency");
        assert_eq!(
            e.public_message(),
            "unknown currency: \u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{20ac}\u{2026}"
        );
    }
}
