/// Cameroon MSISDN normalisation — the Dart port of
/// `frontends/apps/checkout/src/lib/msisdn.ts`.
///
/// Cameroon mobile numbers are nine digits beginning with `6`; in E.164 that
/// is `+237` followed by those nine. This module accepts the three shapes a
/// payer actually types — `+237 6XX XX XX XX`, `237…` and the bare national
/// `6XX XX XX XX` — and answers **one** canonical string: `2376XXXXXXXX`,
/// twelve digits, no `+`. That is what the rail receives (MTN's
/// `payer.partyId`), so this normalises once rather than letting each
/// caller decide.
///
/// **The server is now authoritative on validation** (`vpay_provider`'s
/// `PayerFieldKind::Phone`, enforced at confirm with `phonenumber` against
/// the adapter's declared region — issue #189, #186). This function is a
/// form affordance for instant feedback only, never the source of truth,
/// and it does not bundle libphonenumber's metadata to duplicate a rule the
/// server owns.
///
/// **This validator is strict about digits.** The demo/e2e steering numbers
/// that encode a WireMock scenario in hex (`237600000ce0` and similar) are
/// not numbers — they carry hex letters — and this form refuses them, same
/// as `msisdn.ts` does. That is a fixture problem, not a validator bug.
library;

/// Cameroon's country calling code.
const String _cmCountryCode = '237';

/// Every Cameroon mobile number begins with this digit.
const String _cmMobilePrefix = '6';

/// Digits in a Cameroon national mobile number, the leading `6` included.
const int _cmNationalDigits = 9;

/// Separators a payer may type — ASCII space, tab, hyphen, dot, parentheses,
/// plus the two spaces a phone keypad or a French locale inserts (U+00A0
/// no-break space, U+202F narrow no-break space). Written as escapes,
/// exactly as `msisdn.ts` writes them, because a literal one is invisible in
/// a diff. Real payers paste from real keyboards, and both of those are
/// exactly what one pastes. Everything else makes the input invalid.
const Set<String> _separators = {
  ' ',
  '\t',
  '-',
  '.',
  '(',
  ')',
  '\u00a0',
  '\u202f',
};

const int _zeroCodeUnit = 0x30;
const int _nineCodeUnit = 0x39;

bool _isDigit(String character) {
  if (character.isEmpty) {
    return false;
  }
  final int codeUnit = character.codeUnitAt(0);
  return codeUnit >= _zeroCodeUnit && codeUnit <= _nineCodeUnit;
}

/// The canonical `2376XXXXXXXX`, or `null` when [input] is not a Cameroon
/// mobile number.
///
/// `null` rather than a thrown error or a best-effort string: the caller is
/// a form field, and the only useful thing to do with an unparseable number
/// is to show the payer the rule and let them retype it.
String? normalizeCameroonMsisdn(String input) {
  String digits = '';
  int index = 0;
  final String trimmed = input.trim();
  if (trimmed.startsWith('+')) {
    index = 1;
  }
  for (; index < trimmed.length; index += 1) {
    final String character = trimmed[index];
    if (_isDigit(character)) {
      digits += character;
      continue;
    }
    if (_separators.contains(character)) {
      continue;
    }
    // A letter, a second `+`, punctuation: not a phone number.
    return null;
  }

  final String national;
  if (digits.length == _cmNationalDigits) {
    national = digits;
  } else if (digits.length == _cmCountryCode.length + _cmNationalDigits &&
      digits.startsWith(_cmCountryCode)) {
    national = digits.substring(_cmCountryCode.length);
  } else {
    return null;
  }

  if (!national.startsWith(_cmMobilePrefix)) {
    return null;
  }
  return '$_cmCountryCode$national';
}

/// `+237 6 71 23 45 67` — the grouping a Cameroonian reads a number in.
///
/// Display only. Never sent to a rail, never compared against anything.
String formatCameroonMsisdn(String canonical) {
  if (canonical.length != _cmCountryCode.length + _cmNationalDigits) {
    return canonical;
  }
  final String national = canonical.substring(_cmCountryCode.length);
  final List<String> pairs = [];
  for (int at = 1; at < national.length; at += 2) {
    final int end = (at + 2 <= national.length) ? at + 2 : national.length;
    pairs.add(national.substring(at, end));
  }
  return '+$_cmCountryCode ${national.substring(0, 1)} ${pairs.join(' ')}';
}
