/**
 * Money rendering. There is no arithmetic here beyond integer multiplication
 * and addition, and there is deliberately no `Number.prototype.toFixed`
 * anywhere in this package: docs/flows/money.md makes an amount an integer
 * count of a currency's minor unit, and XAF's minor unit is its major unit.
 */

/** Minor-unit exponents this shop knows. Mirrors `config/application.yml`'s `currencies`. */
const EXPONENTS: Readonly<Record<string, number>> = { xaf: 0, eur: 2 };

/**
 * Renders an integer minor-unit amount for display.
 *
 * `5000, "xaf"` → `5 000 FCFA`. `5000, "eur"` → `50.00 EUR`. A currency this
 * shop has no exponent for is rendered as its raw minor units with the code,
 * which is ugly on purpose: an unknown exponent must never be *guessed* at,
 * because guessing 2 where the answer is 0 is a 100× error on the price tag.
 */
export function formatMinor(minor: number, currency: string): string {
  const code = currency.toLowerCase();
  // `Object.hasOwn` for the same reason `webhook.ts` uses it: a bare index
  // with `code = "constructor"` yields a function, not `undefined`, and the
  // arithmetic below would then render `NaN` on a price tag.
  const exponent = Object.hasOwn(EXPONENTS, code) ? EXPONENTS[code] : undefined;
  if (exponent === undefined) {
    return `${minor} ${currency.toUpperCase()} (minor units)`;
  }
  const sign = minor < 0 ? "-" : "";
  const absolute = Math.abs(minor);
  if (exponent === 0) {
    const grouped = groupThousands(String(absolute));
    return code === "xaf"
      ? `${sign}${grouped} FCFA`
      : `${sign}${grouped} ${currency.toUpperCase()}`;
  }
  const divisor = 10 ** exponent;
  const major = Math.trunc(absolute / divisor);
  const fraction = absolute % divisor;
  const padded = String(fraction).padStart(exponent, "0");
  return `${sign}${groupThousands(String(major))}.${padded} ${currency.toUpperCase()}`;
}

function groupThousands(digits: string): string {
  // A plain ASCII space, deliberately, and not U+202F (narrow no-break
  // space) however much better that looks in French typography: this string
  // is asserted on by tests here and by Cypress in lane 6, and a separator
  // nobody can type is a separator every assertion gets wrong once.
  return digits.replace(/\B(?=(\d{3})+(?!\d))/g, " ");
}
