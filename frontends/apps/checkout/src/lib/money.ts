/**
 * Rendering an amount for a payer.
 *
 * `docs/flows/money.md`: money is an integer count of a currency's minor
 * unit, and XAF is zero-decimal — `5000` means 5,000 FCFA, never 50.00. Two
 * rules follow, and this module exists to hold both in one place.
 *
 * **The exponent table is this module's own**, mirroring
 * `Money::to_provider_string`, rather than left to `Intl`: the exponent is
 * the one part of this that must agree with the server, and an ICU version
 * that disagreed about a currency would move a decimal point on a payment
 * page.
 *
 * **No floating point.** The minor-unit integer is turned into a decimal
 * *string* by moving a decimal point through its digits, and that string is
 * what `Intl.NumberFormat` formats (ECMA-402 accepts a string and parses it
 * exactly, which is the whole reason it does). `minor / 100` would be a
 * float operation in the money path, which the Rust half of this repository
 * denies workspace-wide; there is no reason for the TypeScript half to do
 * what the Rust half is linted against.
 *
 * Grouping, the symbol and its placement *are* `Intl`'s, because those are
 * presentation and differ by locale (`5 000 F CFA` in French, `XAF 5,000` in
 * English) — a legibility bug if wrong, not a 100× one.
 *
 * `@vpay/api-client`'s `formatAmount` does the same job for the dashboard.
 * It is not imported: this app must not depend on the dashboard's client
 * (ADR-0008, and this lane's own constraint), and the shared thing is the
 * table in `docs/flows/money.md`, which both mirror.
 */

/** Currencies whose minor unit *is* their major unit. */
const ZERO_DECIMAL = new Set(['XAF', 'XOF', 'JPY', 'KRW', 'CLP', 'VND']);

/** The exponent for a currency code, upper- or lower-case. */
export function currencyExponent(currency: string): number {
  return ZERO_DECIMAL.has(currency.toUpperCase()) ? 0 : 2;
}

/**
 * `5000` at exponent 2 → `"50.00"`; at exponent 0 → `"5000"`.
 *
 * String surgery on the integer's own digits. Exported so the tests can
 * assert the conversion separately from whatever `Intl` then does with it.
 */
export function toDecimalString(minor: number, exponent: number): string {
  const negative = minor < 0;
  const digits = String(Math.abs(minor));
  if (exponent === 0) {
    return `${negative ? '-' : ''}${digits}`;
  }
  const padded = digits.padStart(exponent + 1, '0');
  const whole = padded.slice(0, padded.length - exponent);
  const fraction = padded.slice(padded.length - exponent);
  return `${negative ? '-' : ''}${whole}.${fraction}`;
}

/**
 * Formats an integer minor-unit amount.
 *
 * Throws on a non-integer, the way `@vpay/api-client` does: a fractional
 * count of minor units is a bug upstream, and rounding it here would hide
 * which side produced it. Every caller in this app reads the value straight
 * off a `payment_intent` the server rendered, so this cannot fire for a
 * payer.
 */
export function formatAmount(minor: number, currency: string, locale: string): string {
  if (!Number.isInteger(minor)) {
    throw new TypeError(`amount must be an integer in minor units, got ${minor}`);
  }
  const code = currency.toUpperCase();
  const exponent = currencyExponent(code);
  const decimal = toDecimalString(minor, exponent);
  try {
    return new Intl.NumberFormat(locale, {
      style: 'currency',
      currency: code,
      minimumFractionDigits: exponent,
      maximumFractionDigits: exponent,
      // ECMA-402 `format` takes a string and parses it exactly; the cast
      // is only because TypeScript's bundled `Intl` typings still declare
      // `number | bigint`. Passing the number would reintroduce the float.
    }).format(decimal as unknown as number);
  } catch {
    // An ICU build that does not know the code, or a locale it rejects.
    // The digits and the code are legible and cannot be mistaken for a
    // different amount.
    return `${decimal} ${code}`;
  }
}
