import { describe, expect, it } from "vitest";

import { envelopeMessage, requestIdOf, unreachable } from "./api";

describe("a refusal", () => {
  it("takes its sentence from vpay error envelope", () => {
    const body = {
      error: {
        type: "invalid_request_error",
        code: "authentication_error",
        message: "  We could not read this.  ",
      },
    };
    expect(envelopeMessage(401, body)).toBe("We could not read this.");
  });

  it("never renders an unparseable body — a proxy error page is not a message", () => {
    // `null` is what a body that would not parse becomes at the call site.
    for (const body of [
      null,
      "<html><body>502 from the ingress</body></html>",
      { error: {} },
      { error: { message: "   " } },
      { error: { message: 7 } },
    ]) {
      const message = envelopeMessage(502, body);
      expect(message).not.toContain("<html>");
      expect(message).toBe("vpay answered 502.");
    }
  });

  it("carries the request id, from either spelling vpay emits", () => {
    // vpay sends both `request-id` (Stripe's spelling) and `x-request-id`
    // with one value; a deployment behind something that strips one must
    // still show an id, because it is the only thing that tells two
    // refusals apart.
    for (const header of ["request-id", "x-request-id"]) {
      const response = new Response("{}", {
        status: 500,
        headers: { [header]: " req_01J8 " },
      });
      expect(requestIdOf(response)).toBe("req_01J8");
    }
    expect(requestIdOf(new Response("{}", { status: 500 }))).toBeNull();
    expect(
      requestIdOf(new Response("{}", { headers: { "request-id": "  " } })),
    ).toBeNull();
  });

  it("turns an unreachable vpay into a failure rather than a thrown error", () => {
    // A rejected fetch inside a server component renders Next own error
    // page, which tells a person nothing and an operator less.
    const failure = unreachable(new Error("ECONNREFUSED"));
    expect(failure.status).toBe(0);
    expect(failure.message).toContain("ECONNREFUSED");
    expect(failure.requestId).toBeNull();
    expect(unreachable("boom").message).toContain("boom");
  });
});
