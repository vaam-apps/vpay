/// Rendering an amount for a payer — the Dart port of
/// `frontends/apps/checkout/src/lib/money.ts`'s no-float half.
///
/// `docs/flows/money.md`: money is an integer count of a currency's minor
/// unit, and XAF is zero-decimal — `5000` means 5,000 FCFA, never 50.00.
///
/// **The exponent table is this module's own**, mirroring
/// `Money::to_provider_string` and `money.ts`'s `ZERO_DECIMAL`, rather than
/// left to a locale-formatting library: the exponent is the one part of this
/// that must agree with the server, and a formatting library that disagreed
/// about a currency would move a decimal point on a payment screen.
///
/// **No floating point, anywhere in this file.** [toDecimalString] turns the
/// minor-unit integer into a decimal *string* by moving a decimal point
/// through its own digits — `padded.slice`/`padStart` in the TypeScript,
/// `substring`/`padLeft` here. `minor / 100` would be a float operation in
/// the money path, which the Rust half of this repository denies
/// workspace-wide (`docs/flows/money.md`); there is no reason for this half
/// to do what that half is linted against.
///
/// **Locale-aware presentation (`Intl.NumberFormat` in `money.ts`'s
/// `formatAmount`) is deliberately not ported here.** Grouping, the currency
/// symbol and its placement are presentation, and this lane is pure-Dart
/// core with no i18n catalogue and no widgets (issue #189, "out of scope for
/// this lane") — a locale-aware formatter belongs with the screen that
/// renders it. What crosses here is the part that must never regress
/// regardless of which lane formats the string for display: the digit
/// surgery, and the currency's exponent.
library;

/// Currencies whose minor unit *is* their major unit — `money.ts`'s
/// `ZERO_DECIMAL`, restated. Compared upper-case, so a lower-case wire value
/// (`docs/flows/money.md`: vpay's own `currency` fields are lower-case) is
/// still recognised.
const Set<String> _zeroDecimalCurrencies = {
  'XAF',
  'XOF',
  'JPY',
  'KRW',
  'CLP',
  'VND',
};

/// The exponent for a currency code, upper- or lower-case.
int currencyExponent(String currency) =>
    _zeroDecimalCurrencies.contains(currency.toUpperCase()) ? 0 : 2;

/// `5000` at exponent 2 -> `"50.00"`; at exponent 0 -> `"5000"`.
///
/// String surgery on the integer's own digits, mirroring `money.ts:46-56`
/// line for line: pad the absolute value's digits out to at least
/// `exponent + 1` characters, then split whole from fraction by position —
/// never by division. [minor] and [exponent] are both taken as integers on
/// purpose: an integer division (`minor ~/ pow`) is exactly the "move a
/// decimal point through a float" bug this function exists to make
/// impossible, so this signature gives it nothing to divide.
String toDecimalString(int minor, int exponent) {
  final bool negative = minor < 0;
  final String digits = minor.abs().toString();
  if (exponent == 0) {
    return negative ? '-$digits' : digits;
  }
  final String padded = digits.padLeft(exponent + 1, '0');
  final String whole = padded.substring(0, padded.length - exponent);
  final String fraction = padded.substring(padded.length - exponent);
  return '${negative ? '-' : ''}$whole.$fraction';
}

/// [toDecimalString] for a [currency] code, deriving the exponent from
/// [currencyExponent] rather than making every caller pass one — the pairing
/// `money.ts`'s `formatAmount` makes before handing its result to `Intl`.
String minorUnitsToDecimalString(int minor, String currency) =>
    toDecimalString(minor, currencyExponent(currency));
