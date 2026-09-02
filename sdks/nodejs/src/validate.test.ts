import { describe, expect, it } from "vitest";
import { assertIntegerAmount } from "./validate.js";

describe("assertIntegerAmount", () => {
  it("accepts an integer", () => {
    expect(() => assertIntegerAmount(5000)).not.toThrow();
  });

  it("throws TypeError on a non-integer amount", () => {
    expect(() => assertIntegerAmount(50.5)).toThrow(TypeError);
    expect(() => assertIntegerAmount(NaN)).toThrow(TypeError);
  });

  it("accepts zero and the largest safe integer", () => {
    expect(() => assertIntegerAmount(0)).not.toThrow();
    expect(() => assertIntegerAmount(Number.MAX_SAFE_INTEGER)).not.toThrow();
  });

  // Regression: `Number.isInteger` alone accepted every one of these.
  it("throws TypeError on a negative amount", () => {
    expect(() => assertIntegerAmount(-1)).toThrow(TypeError);
    expect(() => assertIntegerAmount(-5000)).toThrow(TypeError);
  });

  it("throws TypeError on 1e21, which Number.isInteger accepts", () => {
    expect(Number.isInteger(1e21)).toBe(true);
    expect(() => assertIntegerAmount(1e21)).toThrow(TypeError);
  });

  it("throws TypeError on an integer past MAX_SAFE_INTEGER", () => {
    expect(Number.isInteger(Number.MAX_SAFE_INTEGER + 2)).toBe(true);
    expect(() => assertIntegerAmount(Number.MAX_SAFE_INTEGER + 2)).toThrow(
      TypeError,
    );
  });

  it("throws TypeError on Infinity", () => {
    expect(() => assertIntegerAmount(Infinity)).toThrow(TypeError);
    expect(() => assertIntegerAmount(-Infinity)).toThrow(TypeError);
  });

  it("names the field it was given", () => {
    expect(() => assertIntegerAmount(-1, "refund amount")).toThrow(
      /refund amount/,
    );
  });
});
