/**
 * Turning what `/dash/v1` answers into what a screen shows.
 *
 * Pure, and separate from every component, because these are the decisions a
 * reader of a payments dashboard is entitled to have got right — an amount
 * off by a factor of a hundred is not a rendering bug, it is a wrong number
 * on an operator's screen.
 */
import { PAYMENT_STATUS, type PaymentStatus } from "@vpay/tokens";

/**
 * The em dash this app shows where a value is genuinely absent.
 *
 * One constant, so "not written yet" reads the same everywhere and so a test
 * can assert on it. It is never a substitute for a zero: a zero amount is
 * `0`, and a `null` payer reference is this.
 */
export const ABSENT = "—";

/**
 * An amount in **integer minor units** plus its currency, as text.
 *
 * `docs/flows/money.md`: XAF is zero-decimal, so `5000` on an `xaf` intent is
 * 5,000 FCFA and dividing by a hundred would show fifty. The exponent comes
 * from `Intl.NumberFormat`, which knows each ISO-4217 code's own — so this
 * function never hard-codes a divisor and never does floating-point
 * arithmetic on a money value: the minor units go in as an integer and
 * `minimumFractionDigits`/`maximumFractionDigits` place the point.
 *
 * A currency `Intl` does not know falls back to `<code> <units>`, which is
 * wrong-looking on purpose: it is better for an operator to see an
 * unformatted number and ask than to see a confidently placed decimal point
 * in the wrong place.
 */
export function formatAmount(minorUnits: number, currency: string): string {
  const code = currency.toUpperCase();
  const digits = exponentOf(code);
  if (digits === null) {
    return `${code} ${minorUnits}`;
  }
  // Integer arithmetic only, then a string with the point inserted — the
  // major/minor split is done on the digits, not by dividing.
  const negative = minorUnits < 0;
  const units = Math.abs(minorUnits)
    .toString()
    .padStart(digits + 1, "0");
  const major = units.slice(0, units.length - digits);
  const minor = digits === 0 ? "" : `.${units.slice(units.length - digits)}`;
  return `${negative ? "-" : ""}${group(major)}${minor} ${code}`;
}

/**
 * Thousands separators, inserted into the digit **string**.
 *
 * Not `Number(major).toLocaleString()`, and the difference is not cosmetic:
 * amounts are `i64` on the wire, and an `i64` above 2^53 does not survive a
 * round trip through a JavaScript number. `9007199254740993` would render as
 * `9,007,199,254,740,992`. No amount that large is realistic, and rendering a
 * money value through a lossy type on the argument that it is unrealistic is
 * the reasoning this repository denies float arithmetic workspace-wide to
 * avoid. The digits never stop being digits here.
 */
function group(digits: string): string {
  return digits.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/**
 * How many minor units make one major unit of `code`, or `null` if this
 * runtime has no data for it.
 *
 * `Intl.NumberFormat` alone cannot answer the second half: a well-formed
 * three-letter code it has never heard of does **not** throw — it resolves to
 * two fraction digits, which is a guess wearing the same face as a fact.
 * `ZZZ 1234` would render as `12.34`, and on a zero-decimal currency that is
 * a hundredfold error on an operator's screen. `Intl.supportedValuesOf` is
 * what distinguishes "this currency has two decimals" from "I do not know
 * this currency"; a runtime without it (it is ES2022) falls back to trusting
 * the formatter, which is the same behaviour this function had before the
 * distinction was drawn and is no worse than it.
 */
function exponentOf(code: string): number | null {
  try {
    if (
      typeof Intl.supportedValuesOf === "function" &&
      !Intl.supportedValuesOf("currency").includes(code)
    ) {
      return null;
    }
    return (
      new Intl.NumberFormat("en", {
        style: "currency",
        currency: code,
      }).resolvedOptions().maximumFractionDigits ?? 0
    );
  } catch {
    // A code that is not even well-formed — `Intl` throws a RangeError.
    return null;
  }
}

/**
 * A Unix **seconds** timestamp as an ISO-8601 UTC instant, to the second.
 *
 * UTC and not the reader's locale, deliberately: two operators comparing a
 * payment against a rail's support ticket must be quoting the same instant,
 * and `toLocaleString` would give each of them a different string for the
 * same row. The `Z` is on the screen so nobody has to assume.
 *
 * `created` is seconds everywhere in this API, never milliseconds — see
 * `PaymentIntentObject::created`.
 */
export function formatInstant(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds)) {
    return ABSENT;
  }
  return `${new Date(unixSeconds * 1000).toISOString().slice(0, 19)}Z`;
}

/**
 * A status string from the wire, as a {@link PaymentStatus}, or `null`.
 *
 * `null` rather than a default, and the caller renders the raw string
 * instead: a status this build cannot name means the deployment holds
 * something newer than this bundle, and quietly showing it as
 * `requires_payment_method` would be a colour that means something specific
 * on a value that is not it.
 */
export function asPaymentStatus(value: string): PaymentStatus | null {
  return (PAYMENT_STATUS as readonly string[]).includes(value)
    ? (value as PaymentStatus)
    : null;
}

/**
 * The `payment_method_types` list as one cell.
 *
 * These are the rails an intent **may** be confirmed against, not the rail
 * that took it — that is `charge.provider_code`, which the list endpoint does
 * not return at all. The column is labelled accordingly; see
 * `payments-table.tsx`.
 */
export function formatMethods(types: readonly string[]): string {
  return types.length === 0 ? ABSENT : types.join(", ");
}

/**
 * A date a browser's `<input type="date">` produced (`YYYY-MM-DD`), as the
 * RFC 3339 instant `/dash/v1` takes for `created_gte`.
 *
 * Start of that day, UTC. `null` for anything that is not a plain date, so a
 * hand-edited query string cannot put arbitrary text into an upstream query
 * parameter — vpay would answer `400` naming the parameter, which is correct
 * and is still a worse screen than the filter simply not applying.
 */
export function startOfDayUtc(date: string): string | null {
  return isPlainDate(date) ? `${date}T00:00:00Z` : null;
}

/**
 * The same, as the **inclusive** upper bound `created_lte` takes.
 *
 * Not the next midnight: the bound is inclusive at both ends
 * (`IntentFilter`), so rolling over to the following day would include
 * payments created in that day's first instant.
 *
 * # `.999999`, and the second this used to lose
 *
 * It read `23:59:59Z` until the exp28 review. `payment_intents.created_at` is
 * a `TIMESTAMPTZ` — Postgres keeps microseconds — and the predicate is
 * `created_at <= $7`, so a payment created at `23:59:59.4` is **greater** than
 * `23:59:59.0` and was silently dropped from a filter whose whole meaning is
 * "up to and including this day". One second of every filtered range, at the
 * end an operator is most likely to be asking about, answering as though
 * nothing had happened there — and an empty list reads as data having been
 * lost, which is the failure `payments-query.ts` exists to avoid.
 *
 * `999999` and not `999`: microseconds are the resolution the column stores,
 * and a millisecond bound would lose the last 999 microseconds for the same
 * reason. `vpay_api::dash::payment_intents::parse_timestamp` is
 * `OffsetDateTime::parse(_, Rfc3339)`, which takes a fractional second.
 */
export function endOfDayUtc(date: string): string | null {
  return isPlainDate(date) ? `${date}T23:59:59.999999Z` : null;
}

/** `YYYY-MM-DD`, and a date that exists. */
function isPlainDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    return false;
  }
  const parsed = new Date(`${value}T00:00:00Z`);
  return (
    !Number.isNaN(parsed.getTime()) &&
    parsed.toISOString().slice(0, 10) === value
  );
}
