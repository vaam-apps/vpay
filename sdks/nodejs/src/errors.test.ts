import { describe, expect, it } from "vitest";
import { boundedBodyPrefix, VpayUnexpectedResponseError } from "./errors.js";

describe("boundedBodyPrefix", () => {
  it("returns a short body unchanged", () => {
    expect(boundedBodyPrefix("nope")).toBe("nope");
  });

  it("bounds an ASCII body at 500 characters", () => {
    const prefix = boundedBodyPrefix("a".repeat(2000));
    expect(prefix).toHaveLength(500);
  });

  // Regression: the bound was applied to `String.prototype.length`, so a body
  // of two-byte characters produced a 500-character, 1000-byte "bounded"
  // prefix — twice the documented limit.
  it("bounds a multi-byte body at 500 bytes, not 500 code units", () => {
    const prefix = boundedBodyPrefix("é".repeat(2000));
    expect(Buffer.byteLength(prefix, "utf8")).toBeLessThanOrEqual(500);
  });

  it("bounds a four-byte-per-character body at 500 bytes", () => {
    const prefix = boundedBodyPrefix("😀".repeat(1000));
    expect(Buffer.byteLength(prefix, "utf8")).toBeLessThanOrEqual(500);
  });

  it("drops a character straddling the cut rather than emitting U+FFFD", () => {
    // 499 ASCII bytes then a 3-byte character: the cut lands inside it.
    const prefix = boundedBodyPrefix(`${"a".repeat(499)}€${"a".repeat(500)}`);
    expect(prefix).toBe("a".repeat(499));
    expect(prefix).not.toContain("�");
  });

  it("leaves a body of exactly 500 bytes untouched", () => {
    const body = "a".repeat(500);
    expect(boundedBodyPrefix(body)).toBe(body);
  });
});

describe("VpayUnexpectedResponseError", () => {
  it("carries the status and the bounded prefix, and names both in its message", () => {
    const error = new VpayUnexpectedResponseError(502, "<html>Bad Gateway");
    expect(error.status).toBe(502);
    expect(error.bodyPrefix).toBe("<html>Bad Gateway");
    expect(error.message).toContain("502");
    expect(error.message).toContain("Bad Gateway");
    expect(error.name).toBe("VpayUnexpectedResponseError");
  });
});
