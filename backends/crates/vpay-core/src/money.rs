//! Money as integer minor units. There is no floating point in this crate,
//! and `clippy::float_arithmetic` is denied workspace-wide to keep it that
//! way.
//!
//! `docs/flows/money.md` is the flow: why XAF is zero-decimal and what the
//! single conversion point means. The two renderings a rail may ask for, and
//! the bound on what a rejected currency code may echo back, are in
//! [docs/reference/vpay-core.md § money](../../../../docs/reference/vpay-core.md#money).

use std::fmt;

use serde::{Deserialize, Serialize};

/// An ISO-4217 currency together with its minor-unit exponent.
///
/// The exponent is a property of the *currency*, universally — not of a
/// deployment or an environment. `XAF` is zero-decimal everywhere.
///
/// ```
/// use vpay_core::Currency;
///
/// assert_eq!(Currency::Xaf.exponent(), 0);
/// assert_eq!(Currency::Eur.exponent(), 2);
/// assert_eq!(Currency::Xaf.code(), "XAF");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
// `UPPERCASE`, not the workspace's `snake_case` convention, and exempt from it
// on purpose: these are ISO-4217 codes rather than vpay field names, and
// `"XAF"` is the spelling the database, both adapters and [`Currency::code`]
// already agree on.
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// Central African CFA franc. Zero-decimal.
    Xaf,
    /// Euro. Two-decimal. Used only because MTN's sandbox rejects XAF.
    Eur,
}

impl Currency {
    /// Every currency, so [`Currency::from_code`] and any exhaustive test can
    /// be written once over the list rather than twice over the table.
    pub const ALL: [Self; 2] = [Self::Xaf, Self::Eur];

    /// Minor units per major unit, as a power of ten.
    #[must_use]
    pub const fn exponent(self) -> u32 {
        match self {
            Self::Xaf => 0,
            Self::Eur => 2,
        }
    }

    /// The uppercase ISO-4217 code, as the database and the adapters spell it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Xaf => "XAF",
            Self::Eur => "EUR",
        }
    }

    /// Parses an uppercase ISO code. The wire API uses lowercase and
    /// normalises on ingress; the database and adapters use uppercase.
    ///
    /// Reads [`Currency::code`] back rather than repeating the table, so the
    /// two directions cannot disagree about a code's spelling.
    ///
    /// # Errors
    /// [`UnknownCurrency`] for anything outside [`Currency::ALL`], carrying
    /// the caller's own string.
    ///
    /// ```
    /// use vpay_core::Currency;
    ///
    /// assert_eq!(Currency::from_code("EUR").expect("a known code"), Currency::Eur);
    /// // Uppercase only: the wire API normalises before it gets here.
    /// assert!(Currency::from_code("eur").is_err());
    /// ```
    pub fn from_code(s: &str) -> Result<Self, UnknownCurrency> {
        Self::ALL
            .into_iter()
            .find(|currency| currency.code() == s)
            .ok_or_else(|| UnknownCurrency(s.to_owned()))
    }
}

/// A currency code that is not one of [`Currency::ALL`], carrying the
/// caller's own string.
///
/// `Display` keeps the whole of it (it goes to an operator's log);
/// [`Classify::public_message`](crate::error::Classify::public_message)
/// truncates, because that half crosses the wire.
#[derive(Debug, thiserror::Error)]
#[error("unknown currency: {0}")]
pub struct UnknownCurrency(pub String);

/// A non-negative amount in a currency's minor unit.
///
/// `Money::new(5_000, Currency::Xaf)` is 5,000 FCFA — not 50.00, because XAF
/// has no centimes in circulating use.
///
/// ```
/// use vpay_core::{Currency, Money};
///
/// let five_thousand_francs = Money::new(5_000, Currency::Xaf).expect("non-negative");
/// assert_eq!(five_thousand_francs.minor(), 5_000);
/// assert_eq!(five_thousand_francs.currency(), Currency::Xaf);
/// assert_eq!(five_thousand_francs.to_string(), "5000 XAF");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// vpay's own shape, so it carries the workspace convention even though both
// field names are already snake_case — the attribute is what keeps it true
// when someone adds a field.
#[serde(rename_all = "snake_case")]
pub struct Money {
    minor: i64,
    currency: Currency,
}

impl Money {
    /// An amount in `currency`'s minor unit.
    ///
    /// # Errors
    /// Returns [`MoneyError::Negative`] for a negative amount.
    ///
    /// ```
    /// use vpay_core::{Currency, Money, MoneyError};
    ///
    /// assert!(Money::new(0, Currency::Xaf).is_ok());
    /// assert_eq!(
    ///     Money::new(-1, Currency::Xaf),
    ///     Err(MoneyError::Negative(-1))
    /// );
    /// ```
    pub fn new(minor: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor < 0 {
            return Err(MoneyError::Negative(minor));
        }
        Ok(Self { minor, currency })
    }

    /// The amount, in the currency's minor unit.
    #[must_use]
    pub const fn minor(self) -> i64 {
        self.minor
    }

    /// The currency this amount is denominated in.
    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }

    /// # Errors
    /// [`MoneyError::CurrencyMismatch`] if the currencies differ,
    /// [`MoneyError::Overflow`] on i64 overflow.
    ///
    /// ```
    /// use vpay_core::{Currency, Money, MoneyError};
    ///
    /// let xaf = |n| Money::new(n, Currency::Xaf).expect("non-negative");
    /// assert_eq!(xaf(100).checked_add(xaf(23)), Ok(xaf(123)));
    ///
    /// let eur = Money::new(100, Currency::Eur).expect("non-negative");
    /// assert!(matches!(
    ///     xaf(100).checked_add(eur),
    ///     Err(MoneyError::CurrencyMismatch { .. })
    /// ));
    /// ```
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
    /// result would go below zero — a refund can never exceed what was
    /// captured.
    ///
    /// ```
    /// use vpay_core::{Currency, Money, MoneyError};
    ///
    /// let xaf = |n| Money::new(n, Currency::Xaf).expect("non-negative");
    /// assert_eq!(xaf(100).checked_sub(xaf(40)), Ok(xaf(60)));
    /// assert!(matches!(
    ///     xaf(100).checked_sub(xaf(101)),
    ///     Err(MoneyError::Negative(_))
    /// ));
    /// ```
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
    /// encoding, not a second conversion. **It returns minor units**, and
    /// handing them to a rail that expects major units is 100× on a
    /// two-decimal currency with nothing downstream able to detect it — see
    /// [docs/reference/vpay-core.md § the two provider encodings](../../../../docs/reference/vpay-core.md#the-two-provider-encodings).
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
    /// This is the single conversion point referenced by
    /// `docs/flows/money.md`. [`Money::to_provider_minor`] is the same
    /// conversion for a rail whose API takes a JSON number.
    ///
    /// ```
    /// use vpay_core::{Currency, Money};
    ///
    /// let xaf = Money::new(5_000, Currency::Xaf).expect("non-negative");
    /// let eur = Money::new(5_005, Currency::Eur).expect("non-negative");
    /// assert_eq!(xaf.to_provider_string(), "5000"); // exponent 0
    /// assert_eq!(eur.to_provider_string(), "50.05"); // exponent 2, padded
    /// ```
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

/// What can go wrong constructing or combining a [`Money`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    /// An amount below zero. vpay has no negative money: a refund is its own
    /// object, not a negative charge.
    #[error("amount must not be negative, got {0}")]
    Negative(i64),
    /// Two amounts in different currencies were combined.
    #[error("currency mismatch: {left:?} vs {right:?}")]
    CurrencyMismatch {
        /// The left operand's currency.
        left: Currency,
        /// The right operand's currency.
        right: Currency,
    },
    /// `i64` minor units overflowed.
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
/// Eight: generous enough that a merchant recognises what they sent, short
/// enough that the echo cannot become a reflection channel. See
/// [docs/reference/vpay-core.md § the bounded currency echo](../../../../docs/reference/vpay-core.md#the-bounded-currency-echo).
const CURRENCY_ECHO_CHARS: usize = 8;

/// The first [`CURRENCY_ECHO_CHARS`] characters of `code`, with an ellipsis
/// iff anything was dropped.
///
/// Character-wise, not byte-wise: the code is caller-supplied text and
/// slicing at byte 8 would panic on a multi-byte boundary (ADR-0007, on a
/// request path).
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

    /// Echoes the offending code, bounded — `from_code` wraps whatever
    /// arrived on the wire, so the bound lives here rather than in a caller
    /// that may forget.
    ///
    /// ```
    /// use vpay_core::{Classify, Currency};
    ///
    /// let short = Currency::from_code("xaf").expect_err("lowercase is not a code");
    /// assert_eq!(short.public_message(), "unknown currency: xaf");
    ///
    /// let huge = Currency::from_code(&"A".repeat(1024 * 1024)).expect_err("not a code");
    /// assert_eq!(huge.public_message(), "unknown currency: AAAAAAAA\u{2026}");
    /// // The operator-facing half is deliberately unbounded: it goes to a
    /// // log, never to a response body.
    /// assert!(huge.to_string().len() > 1024 * 1024);
    /// ```
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
