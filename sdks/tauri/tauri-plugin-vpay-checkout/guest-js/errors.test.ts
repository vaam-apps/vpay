import { describe, expect, it } from "vitest";

import {
  embeddedSessionNotSupportedError,
  invalidSessionUrlError,
  platformWindowError,
  unexpectedResponseError,
  VPAY_CLIENT_ERROR_CODES,
} from "./errors.js";

describe("VPAY_CLIENT_ERROR_CODES", () => {
  it("spells the five codes this package originates rather than reads off the wire", () => {
    // Pinned as a whole object rather than key by key: these strings cross
    // into a merchant's `switch (error.code)`, and a rename is a breaking
    // change that should fail here and not in their app.
    expect(VPAY_CLIENT_ERROR_CODES).toEqual({
      pollingTimeout: "polling_timeout",
      unexpectedResponse: "unexpected_response",
      embeddedSessionNotSupported: "embedded_session_not_supported",
      platformWindowFailed: "platform_window_failed",
      invalidRequest: "invalid_request",
    });
  });
});

describe("the fixed-string factories never echo a value they were not handed", () => {
  it("platformWindowError carries a fixed message and no detail of the failure", () => {
    const error = platformWindowError();

    expect(error.type).toBe("api_error");
    expect(error.code).toBe("platform_window_failed");
    expect(error.message).toBe(
      "The checkout window could not be opened, or closed without reporting an outcome.",
    );
    // Two calls, byte-identical: there is nothing variable in it to carry
    // a thrown value.
    expect(platformWindowError()).toEqual(error);
  });

  it("unexpectedResponseError carries the status and nothing else", () => {
    const error = unexpectedResponseError(502);

    expect(error.type).toBe("api_error");
    expect(error.code).toBe("unexpected_response");
    expect(error.message).toBe(
      "The vpay API returned an unexpected response (HTTP 502).",
    );
  });

  it("embeddedSessionNotSupportedError names the session id and the reason", () => {
    const error = embeddedSessionNotSupportedError("cs_123");

    expect(error.type).toBe("invalid_request_error");
    expect(error.code).toBe("embedded_session_not_supported");
    expect(error.message).toContain("cs_123");
    expect(error.message).toContain("embedded");
    // The id is public — the merchant's own server minted it and holds it.
    // The *secret* that shares its prefix is what must never appear, and
    // this factory is never handed one to appear.
    expect(error.message).not.toContain("_secret_");
  });

  it("invalidSessionUrlError names the parameter and never echoes the URL", () => {
    const error = invalidSessionUrlError();

    expect(error.type).toBe("invalid_request_error");
    expect(error.code).toBe("invalid_request");
    expect(error.param).toBe("sessionUrl");
    // It is about a *missing* fragment, and a caller who passed a nearly
    // right URL would have had the secret echoed back had this quoted it.
    expect(error.message).toContain("fragment");
    expect(error.message).not.toContain("https://checkout");
  });
});
