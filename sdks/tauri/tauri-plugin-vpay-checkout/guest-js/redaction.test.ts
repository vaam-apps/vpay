import { describe, expect, it } from "vitest";

import { redacted } from "./redaction.js";

describe("redacted", () => {
  it("renders the exact character count, not the value", () => {
    expect(redacted("cs_123_secret_abcdef")).toBe("[20 chars redacted]");
    expect(redacted("cs_123_secret_abcdef")).not.toContain("secret");
  });

  it("renders the literal null for a null value, not [0 chars redacted]", () => {
    expect(redacted(null)).toBe("null");
  });

  it("renders undefined the same way it renders null", () => {
    expect(redacted(undefined)).toBe("null");
  });

  it("renders a zero-length secret distinctly from null", () => {
    expect(redacted("")).toBe("[0 chars redacted]");
    expect(redacted("")).not.toBe(redacted(null));
  });
});
