import { ABSENT, formatAmount, formatIsoInstant } from "../format";

/**
 * How an amount and an instant are drawn, everywhere in this app.
 *
 * # Monospace, tabular, and computed exactly
 *
 * An amount is `formatAmount`'s string — exact integer arithmetic on minor
 * units, never a float — in the mono voice with tabular figures, so a column
 * of them lines up on the decimal and a `5,000 XAF` cannot be read as
 * `5,000.00 XAF` from a ragged edge. `@vaam-apps/ui`'s `Money` formats with
 * `Intl` and a locale; the dashboard has always spelled amounts the one way
 * `format.ts` documents, and an operator comparing this screen with a rail's
 * statement should not have to translate between two spellings.
 *
 * # Instants are absolute and UTC, never "3m ago"
 *
 * `TimestampDisplay` would print a relative time after hydration. This app
 * renders its rows on the server, and an operator matching a payment to a
 * log line needs the second it happened, not its age at the moment they
 * opened the page. The `<time>` element carries the machine-readable value.
 */

/** An amount in minor units, as `formatAmount` spells it. */
export function Amount({
  minor,
  currency,
}: {
  readonly minor: number;
  readonly currency: string;
}) {
  return (
    <span className="font-mono whitespace-nowrap tabular-nums">
      {formatAmount(minor, currency)}
    </span>
  );
}

/** A unix-seconds instant (the REST list and detail's wire shape). */
export function Instant({ seconds }: { readonly seconds: number }) {
  if (!Number.isFinite(seconds)) {
    return <span>{ABSENT}</span>;
  }
  return <IsoInstant value={new Date(seconds * 1000).toISOString()} />;
}

/** An RFC 3339 instant (a procedure's wire shape), or {@link ABSENT}. */
export function IsoInstant({
  value,
}: {
  readonly value: string | null | undefined;
}) {
  if (value === null || value === undefined) {
    return <span>{ABSENT}</span>;
  }
  return (
    <time dateTime={value} className="whitespace-nowrap">
      <span className="font-mono text-muted-foreground tabular-nums">
        {formatIsoInstant(value)}
      </span>
    </time>
  );
}
