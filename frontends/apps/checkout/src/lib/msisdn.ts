/**
 * Cameroon MSISDN normalisation.
 *
 * Cameroon mobile numbers are nine digits beginning with `6`; in E.164 that
 * is `+237` followed by those nine. This module accepts the three shapes a
 * payer actually types — `+237 6XX XX XX XX`, `237…` and the bare national
 * `6XX XX XX XX` — and answers **one** canonical string:
 * `2376XXXXXXXX`, twelve digits, no `+`.
 *
 * Twelve digits and no `+` because that is what the rail receives:
 * MTN's `payer.partyId` is `237600000000` in
 * `vpay-adapter-mtn-momo/src/wire.rs` and in every conformance mapping. The
 * page normalises once, here, rather than letting each caller decide — the
 * same reasoning as `Money::to_provider_string` having exactly one home.
 *
 * **This validator is strict about digits, and that has a consequence worth
 * stating.** The demo and e2e steering numbers `237600000ce0`,
 * `237600000f01` and `237600000f02` are not numbers — they carry hex letters
 * so that a WireMock mapping can key on them — and this form refuses them,
 * as it refuses any other non-number. See
 * `docs/plans/step9-notes/lane-3.md`; the fix belongs in the stub mappings,
 * not in a phone-number validator that accepts letters.
 */

/** Cameroon's country calling code. */
const CM_COUNTRY_CODE = '237';
/** Every Cameroon mobile number begins with this digit. */
const CM_MOBILE_PREFIX = '6';
/** Digits in a Cameroon national mobile number, the leading `6` included. */
const CM_NATIONAL_DIGITS = 9;

/**
 * Separators a payer may type — ASCII space, tab, hyphen, dot, parentheses,
 * plus the two spaces a phone keypad or a French locale inserts (U+00A0
 * no-break space, U+202F narrow no-break space). Written as escapes because
 * a literal one is invisible in a diff. Everything else makes the input
 * invalid.
 */
const SEPARATORS = new Set([' ', '\t', '-', '.', '(', ')', '\u00a0', '\u202f']);

function isDigit(character: string): boolean {
  return character >= '0' && character <= '9';
}

/**
 * The canonical `2376XXXXXXXX`, or `null` when the input is not a Cameroon
 * mobile number.
 *
 * `null` rather than a thrown error or a best-effort string: the caller is a
 * form, and the only useful thing to do with an unparseable number is to
 * show the payer the rule and let them retype it.
 */
export function normalizeCameroonMsisdn(input: string): string | null {
  if (typeof input !== 'string') {
    return null;
  }
  let digits = '';
  let index = 0;
  const trimmed = input.trim();
  if (trimmed.startsWith('+')) {
    index = 1;
  }
  for (; index < trimmed.length; index += 1) {
    const character = trimmed[index] ?? '';
    if (isDigit(character)) {
      digits += character;
      continue;
    }
    if (SEPARATORS.has(character)) {
      continue;
    }
    // A letter, a second `+`, punctuation: not a phone number.
    return null;
  }

  let national: string;
  if (digits.length === CM_NATIONAL_DIGITS) {
    national = digits;
  } else if (
    digits.length === CM_COUNTRY_CODE.length + CM_NATIONAL_DIGITS &&
    digits.startsWith(CM_COUNTRY_CODE)
  ) {
    national = digits.slice(CM_COUNTRY_CODE.length);
  } else {
    return null;
  }

  if (!national.startsWith(CM_MOBILE_PREFIX)) {
    return null;
  }
  return `${CM_COUNTRY_CODE}${national}`;
}

/**
 * `237 6 71 23 45 67` — the grouping a Cameroonian reads a number in.
 *
 * Display only. Never sent to a rail, never compared against anything.
 */
export function formatCameroonMsisdn(canonical: string): string {
  if (canonical.length !== CM_COUNTRY_CODE.length + CM_NATIONAL_DIGITS) {
    return canonical;
  }
  const national = canonical.slice(CM_COUNTRY_CODE.length);
  const pairs: string[] = [];
  for (let at = 1; at < national.length; at += 2) {
    pairs.push(national.slice(at, at + 2));
  }
  return `+${CM_COUNTRY_CODE} ${national.slice(0, 1)} ${pairs.join(' ')}`;
}
