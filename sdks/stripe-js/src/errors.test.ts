/**
 * The error envelope reader, and the invariant that no message this package
 * builds can carry a credential.
 */
import { describe, expect, it } from "vitest";
import {
  connectionError,
  parseErrorEnvelope,
  stripeError,
  unexpectedResponseError,
} from "./errors.js";

describe("parseErrorEnvelope", () => {
  it("reads the four keys vpay_api::error_envelope_with_param writes", () => {
    expect(
      parseErrorEnvelope({
        error: {
          type: "invalid_request_error",
          code: "invalid_request",
          message: "amount must be a positive integer",
          param: "amount",
        },
      }),
    ).toEqual({
      type: "invalid_request_error",
      code: "invalid_request",
      message: "amount must be a positive integer",
      param: "amount",
    });
  });

  it("leaves param absent when the server omitted it, as `'param' in error` callers expect", () => {
    const error = parseErrorEnvelope({
      error: {
        type: "invalid_request_error",
        code: "resource_missing",
        message: "No such payment intent: pi_1",
      },
    });
    expect(error).toBeDefined();
    expect("param" in (error as object)).toBe(false);
  });

  it.each([
    ["a non-object", "boom"],
    ["null", null],
    ["no error key", { data: {} }],
    ["a non-object error", { error: "boom" }],
    ["an error with no type", { error: { code: "x", message: "y" } }],
    ["an error whose type is not a string", { error: { type: 7 } }],
  ])("returns undefined for %s", (_name, body) => {
    expect(parseErrorEnvelope(body)).toBeUndefined();
  });
});

describe("client-originated errors", () => {
  it("reports a connection failure with no code and a fixed message", () => {
    // No `code`: vpay's server never sends `api_connection_error`, so the
    // absence of a code is itself the signal that the request never landed.
    expect(connectionError()).toEqual({
      type: "api_connection_error",
      message: "Could not reach the vpay API.",
    });
  });

  it("reports an unrecognisable response with the status and nothing from the body", () => {
    const error = unexpectedResponseError(502);
    expect(error.type).toBe("api_error");
    expect(error.code).toBe("unexpected_response");
    expect(error.message).toBe(
      "The vpay API returned an unexpected response (HTTP 502).",
    );
  });

  it("omits code and param rather than setting them undefined", () => {
    const error = stripeError("api_error", undefined, "x");
    expect("code" in error).toBe(false);
    expect("param" in error).toBe(false);
  });
});
