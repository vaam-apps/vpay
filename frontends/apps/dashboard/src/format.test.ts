import { describe, expect, it } from "vitest";

import {
  ABSENT,
  asPaymentStatus,
  endOfDayUtc,
  formatAmount,
  formatInstant,
  formatMethods,
  startOfDayUtc,
} from "./format";

describe("an amount", () => {
  it("renders a zero-decimal currency without inventing a decimal point", () => {
    // docs/flows/money.md: XAF is zero-decimal, so 5000 minor units is
    // 5,000 FCFA. Dividing by a hundred would show fifty.
    expect(formatAmount(5000, "xaf")).toBe("5,000 XAF");
  });

  it("places the point where a two-decimal currency puts it", () => {
    expect(formatAmount(125_000, "eur")).toBe("1,250.00 EUR");
    expect(formatAmount(5, "usd")).toBe("0.05 USD");
  });

  it("renders zero as zero, never as a dash", () => {
    expect(formatAmount(0, "xaf")).toBe("0 XAF");
    expect(formatAmount(0, "xaf")).not.toBe(ABSENT);
  });

  it("keeps a negative sign", () => {
    expect(formatAmount(-5000, "xaf")).toBe("-5,000 XAF");
  });

  it("shows an unknown currency unformatted rather than guessing an exponent", () => {
    // Better an operator sees a raw number and asks than a confidently
    // placed decimal point in the wrong place.
    expect(formatAmount(1234, "zzz")).toBe("ZZZ 1234");
  });
});

describe("an instant", () => {
  it("is UTC, to the second, with the Z on screen", () => {
    // Two operators comparing a payment against a rail support ticket must
    // be quoting the same instant; toLocaleString would give each a
    // different string for the same row.
    expect(formatInstant(1_757_000_000)).toBe("2025-09-04T15:33:20Z");
  });

  it("reads seconds, not milliseconds", () => {
    // `created` is seconds everywhere in this API. Reading it as
    // milliseconds puts every payment in 1970.
    expect(formatInstant(1_757_000_000)).toContain("2025-");
  });

  it("is the dash for a value that is not a number", () => {
    expect(formatInstant(Number.NaN)).toBe(ABSENT);
  });
});

describe("a status from the wire", () => {
  it("is typed when the vocabulary knows it", () => {
    expect(asPaymentStatus("succeeded")).toBe("succeeded");
  });

  it("is null for one this build cannot name — never defaulted", () => {
    // A green pill on an unknown status is a claim. The caller renders the
    // raw string instead.
    expect(asPaymentStatus("disputed")).toBeNull();
  });
});

describe("the methods cell", () => {
  it("lists them", () => {
    expect(formatMethods(["mtn_momo", "orange_money"])).toBe(
      "mtn_momo, orange_money",
    );
  });

  it("is the dash when there are none", () => {
    expect(formatMethods([])).toBe(ABSENT);
  });
});

describe("a date filter", () => {
  it("becomes the start and the inclusive end of that UTC day", () => {
    // 23:59:59 and not the next midnight: IntentFilter bounds are inclusive
    // at both ends, so rolling over would include the following day first
    // second.
    expect(startOfDayUtc("2026-09-07")).toBe("2026-09-07T00:00:00Z");
    expect(endOfDayUtc("2026-09-07")).toBe("2026-09-07T23:59:59.999999Z");
    // `.999999` and not `:59Z`: `created_at` is a TIMESTAMPTZ and the
    // predicate is `<=`, so a payment created at 23:59:59.4 is greater than
    // 23:59:59.0 and a whole second of the range an operator asked for
    // vanishes. Microseconds because that is the column resolution.
    expect(endOfDayUtc("2026-09-07")).not.toBe("2026-09-07T23:59:59Z");
    expect(endOfDayUtc("2026-09-07") ?? "").toMatch(/\.999999Z$/);
  });

  it("is null for anything that is not a real calendar date", () => {
    for (const value of [
      "yesterday",
      "2026-13-01",
      "2026-02-30",
      "2026-9-7",
      "",
    ]) {
      expect(startOfDayUtc(value)).toBeNull();
    }
  });
});
