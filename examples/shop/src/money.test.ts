import { describe, expect, it } from "vitest";
import { formatMinor } from "./money";

describe("formatMinor", () => {
  it("renders XAF as zero-decimal, per docs/flows/money.md", () => {
    expect(formatMinor(5000, "xaf")).toBe("5 000 FCFA");
    expect(formatMinor(12000, "XAF")).toBe("12 000 FCFA");
    expect(formatMinor(0, "xaf")).toBe("0 FCFA");
    expect(formatMinor(250000, "xaf")).toBe("250 000 FCFA");
  });

  it("renders EUR with two decimals from the same integer", () => {
    expect(formatMinor(5000, "eur")).toBe("50.00 EUR");
    expect(formatMinor(5, "eur")).toBe("0.05 EUR");
    expect(formatMinor(123456, "eur")).toBe("1 234.56 EUR");
  });

  it("refuses to guess an exponent it does not know", () => {
    // Not "12.00 XYZ": guessing 2 where the answer is 0 is a 100x error on a
    // price tag, so an unknown currency is rendered as what it actually is.
    expect(formatMinor(1200, "xyz")).toBe("1200 XYZ (minor units)");
    // And does not read an inherited property for a currency code that
    // happens to name one.
    expect(formatMinor(1200, "constructor")).toBe(
      "1200 CONSTRUCTOR (minor units)",
    );
  });
});
