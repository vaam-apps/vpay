/// Locale-aware presentation of an amount — the half of `money.ts`'s
/// `formatAmount` Lane 1's `money.dart` deliberately did not port (its own
/// doc comment: "a locale-aware formatter belongs with the screen that
/// renders it" — this is that screen).
///
/// **No floating point here either.** [formatAmountForDisplay] calls
/// straight into [minorUnitsToDecimalString] for the digit surgery and only
/// ever touches the resulting *string* — grouping a string's digits is not
/// arithmetic.
///
/// `money.ts` hands its decimal string to `Intl.NumberFormat`, which knows
/// each currency's own symbol and placement. Dart's standard library has no
/// such table, and this package does not add the `intl` package for one
/// formatter — so this is deliberately simpler than the web page: digits are
/// grouped by locale (a narrow no-break space every three digits for French,
/// a comma for English — `5 000`/`5,000`), and the ISO 4217 code is shown
/// next to it (`5 000 XAF`/`XAF 5,000`), the same shape `money.ts`'s own
/// catch branch falls back to when `Intl` cannot format a currency it does
/// not recognise. Legible and unambiguous, at the cost of not rendering a
/// currency symbol (`F CFA`, `$`) — recorded here rather than silently
/// matched to the web page's exact output.
library;

import 'money.dart';

/// Groups [decimal]'s whole-number part into runs of three, from the right,
/// joined by [separator] — never touching the fractional part after
/// [decimalSeparator], which is inserted verbatim.
String _grouped(String decimal, String separator, String decimalSeparator) {
  final bool negative = decimal.startsWith('-');
  final String unsigned = negative ? decimal.substring(1) : decimal;
  final int dot = unsigned.indexOf('.');
  final String whole = dot == -1 ? unsigned : unsigned.substring(0, dot);
  final String fraction = dot == -1 ? '' : unsigned.substring(dot + 1);

  final StringBuffer buffer = StringBuffer();
  for (int i = 0; i < whole.length; i++) {
    final int fromEnd = whole.length - i;
    if (i > 0 && fromEnd % 3 == 0) {
      buffer.write(separator);
    }
    buffer.write(whole[i]);
  }
  final String groupedWhole = buffer.toString();
  final String withFraction = fraction.isEmpty
      ? groupedWhole
      : '$groupedWhole$decimalSeparator$fraction';
  return negative ? '-$withFraction' : withFraction;
}

/// `money.ts`'s `formatAmount`, minus `Intl` — see this file's own doc
/// comment for exactly what is and is not equivalent.
String formatAmountForDisplay(
  int minorUnits,
  String currency,
  bool frenchLocale,
) {
  final String code = currency.toUpperCase();
  final String decimal = minorUnitsToDecimalString(minorUnits, code);
  final String grouped = frenchLocale
      ? _grouped(decimal, ' ', ',')
      : _grouped(decimal, ',', '.');
  return frenchLocale ? '$grouped $code' : '$code $grouped';
}
