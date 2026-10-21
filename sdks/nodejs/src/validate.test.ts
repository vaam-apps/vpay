import { describe, expect, it } from "vitest";
import { VpayError, VpayInvalidParamsError } from "./errors.js";
import { assertIntegerAmount } from "./validate.js";

describe("assertIntegerAmount", () => {
  it("accepts an integer", () => {
    expect(() => assertIntegerAmount(5000)).not.toThrow();
  });

  it("throws VpayInvalidParamsError on a non-integer amount", () => {
    expect(() => assertIntegerAmount(50.5)).toThrow(VpayInvalidParamsError);
    expect(() => assertIntegerAmount(NaN)).toThrow(VpayInvalidParamsError);
  });

  it("accepts zero and the largest safe integer", () => {
    expect(() => assertIntegerAmount(0)).not.toThrow();
    expect(() => assertIntegerAmount(Number.MAX_SAFE_INTEGER)).not.toThrow();
  });

  // Regression: `Number.isInteger` alone accepted every one of these.
  it("throws VpayInvalidParamsError on a negative amount", () => {
    expect(() => assertIntegerAmount(-1)).toThrow(VpayInvalidParamsError);
    expect(() => assertIntegerAmount(-5000)).toThrow(VpayInvalidParamsError);
  });

  it("throws VpayInvalidParamsError on 1e21, which Number.isInteger accepts", () => {
    expect(Number.isInteger(1e21)).toBe(true);
    expect(() => assertIntegerAmount(1e21)).toThrow(VpayInvalidParamsError);
  });

  it("throws VpayInvalidParamsError on an integer past MAX_SAFE_INTEGER", () => {
    expect(Number.isInteger(Number.MAX_SAFE_INTEGER + 2)).toBe(true);
    expect(() => assertIntegerAmount(Number.MAX_SAFE_INTEGER + 2)).toThrow(
      VpayInvalidParamsError,
    );
  });

  it("throws VpayInvalidParamsError on Infinity", () => {
    expect(() => assertIntegerAmount(Infinity)).toThrow(VpayInvalidParamsError);
    expect(() => assertIntegerAmount(-Infinity)).toThrow(
      VpayInvalidParamsError,
    );
  });

  it("refuses with a VpayInvalidParamsError carrying the wire name, not a TypeError", () => {
    let caught: unknown;
    try {
      assertIntegerAmount(-1, "unit_amount");
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(VpayError);
    expect(caught).toBeInstanceOf(VpayInvalidParamsError);
    expect(caught).not.toBeInstanceOf(TypeError);
    expect((caught as VpayInvalidParamsError).param).toBe("unit_amount");
    expect((caught as VpayInvalidParamsError).message).toBe(
      "invalid request parameter unit_amount: must be a non-negative safe integer in minor units, got -1",
    );
  });

  it("names the field it was given", () => {
    expect(() => assertIntegerAmount(-1, "refund amount")).toThrow(
      /refund amount/,
    );
  });
});
