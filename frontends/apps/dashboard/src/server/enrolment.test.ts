import { describe, expect, it } from "vitest";

import {
  decodePendingEnrolment,
  encodePendingEnrolment,
  secretFrom,
} from "./enrolment";

const PENDING = {
  sealed: "c2VhbGVkLWJsb2I",
  otpauth:
    "otpauth://totp/vpay%20sandbox:ada%40example.test?secret=GEZDGNBVGY3TQOJQ&issuer=vpay%20sandbox",
};

describe("the pending enrolment cookie", () => {
  it("round-trips both halves", () => {
    expect(decodePendingEnrolment(encodePendingEnrolment(PENDING))).toEqual(
      PENDING,
    );
  });

  it("survives the query separators an otpauth URI is full of", () => {
    const encoded = encodePendingEnrolment(PENDING);
    expect(encoded).not.toContain("?");
    expect(encoded).not.toContain("&");
    expect(encoded).not.toContain(";");
  });

  it.each<[string, string | undefined]>([
    ["absent", undefined],
    ["empty", ""],
    ["not JSON", "nonsense"],
    ["JSON but not an object", encodeURIComponent('"a string"')],
    [
      "missing the sealed half",
      encodeURIComponent(JSON.stringify({ otpauth: PENDING.otpauth })),
    ],
    [
      "missing the URI",
      encodeURIComponent(JSON.stringify({ sealed: PENDING.sealed })),
    ],
    [
      "a URI that is not otpauth",
      encodeURIComponent(
        JSON.stringify({
          sealed: "x",
          otpauth: "https://example.test/?secret=x",
        }),
      ),
    ],
  ])("is null when it is %s", (_case, value) => {
    // Null and not a throw: the page's answer to all of these is the same
    // redirect back to /login.
    expect(decodePendingEnrolment(value)).toBeNull();
  });
});

describe("the typed-secret fallback", () => {
  it("reads the base32 secret out of the URI the QR encodes", () => {
    // Out of the URI, so the QR and the typed key cannot disagree about
    // which secret is being enrolled.
    expect(secretFrom(PENDING.otpauth)).toBe("GEZDGNBVGY3TQOJQ");
  });

  it("is null for a URI with no secret parameter", () => {
    expect(secretFrom("otpauth://totp/vpay:ada")).toBeNull();
  });
});
