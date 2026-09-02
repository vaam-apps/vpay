import { verify } from "node:crypto";
import { describe, expect, it } from "vitest";
import { mintClientAssertion, MAX_ASSERTION_LIFETIME_SECONDS } from "./auth.js";
import { VpayConfigError } from "./errors.js";
import { generateTestRsaKeyPair } from "./testing/keys.js";

const UUID_V4_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function decodePart<T>(part: string): T {
  return JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as T;
}

interface AssertionHeader {
  alg: string;
  typ: string;
  kid?: string;
}

interface AssertionPayload {
  iss: string;
  sub: string;
  aud: string;
  jti: string;
  exp: number;
  iat: number;
}

describe("mintClientAssertion", () => {
  const { privateKey, publicKey } = generateTestRsaKeyPair();
  const clientId = "merchant_a";
  const audience = "https://api.vpay.example/v1/oauth/token";

  it("produces a header with alg RS256, typ JWT, and no kid when none is configured", () => {
    const jwt = mintClientAssertion({ clientId, privateKey, audience });
    const [headerPart] = jwt.split(".");
    const header = decodePart<AssertionHeader>(headerPart!);
    expect(header.alg).toBe("RS256");
    expect(header.typ).toBe("JWT");
    expect(header.kid).toBeUndefined();
  });

  it("includes kid in the header exactly when configured", () => {
    const jwt = mintClientAssertion({
      clientId,
      privateKey,
      audience,
      kid: "key-1",
    });
    const [headerPart] = jwt.split(".");
    const header = decodePart<AssertionHeader>(headerPart!);
    expect(header.kid).toBe("key-1");
  });

  it("sets iss and sub to the client_id, and aud to the token endpoint", () => {
    const jwt = mintClientAssertion({ clientId, privateKey, audience });
    const [, payloadPart] = jwt.split(".");
    const payload = decodePart<AssertionPayload>(payloadPart!);
    expect(payload.iss).toBe(clientId);
    expect(payload.sub).toBe(clientId);
    expect(payload.aud).toBe(audience);
  });

  it("carries exactly the six claims the OP verifier expects and no nbf", () => {
    // docs/flows/merchant-auth.md: `nbf` is not emitted — the OP sets
    // `validate_nbf = true`, so a not-yet-valid `nbf` would be refused, and
    // an extra claim is an extra way to drift from the Rust SDK's set.
    const jwt = mintClientAssertion({ clientId, privateKey, audience });
    const payload = decodePart<Record<string, unknown>>(jwt.split(".")[1]!);
    expect(Object.keys(payload).sort()).toEqual([
      "aud",
      "exp",
      "iat",
      "iss",
      "jti",
      "sub",
    ]);
    expect(payload).not.toHaveProperty("nbf");
  });

  it("mints a UUIDv4 jti that differs across two mints", () => {
    const jwtA = mintClientAssertion({ clientId, privateKey, audience });
    const jwtB = mintClientAssertion({ clientId, privateKey, audience });
    const payloadA = decodePart<AssertionPayload>(jwtA.split(".")[1]!);
    const payloadB = decodePart<AssertionPayload>(jwtB.split(".")[1]!);
    expect(payloadA.jti).toMatch(UUID_V4_PATTERN);
    expect(payloadB.jti).toMatch(UUID_V4_PATTERN);
    expect(payloadA.jti).not.toBe(payloadB.jti);
  });

  it("sets exp exactly lifetimeSeconds after iat, and within 300s of now", () => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = mintClientAssertion({
      clientId,
      privateKey,
      audience,
      lifetimeSeconds: 120,
      now,
    });
    const payload = decodePart<AssertionPayload>(jwt.split(".")[1]!);
    expect(payload.iat).toBe(now);
    expect(payload.exp - payload.iat).toBe(120);
    expect(payload.exp - now).toBeLessThanOrEqual(
      MAX_ASSERTION_LIFETIME_SECONDS,
    );
  });

  it("defaults lifetimeSeconds to 60", () => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = mintClientAssertion({ clientId, privateKey, audience, now });
    const payload = decodePart<AssertionPayload>(jwt.split(".")[1]!);
    expect(payload.exp - payload.iat).toBe(60);
  });

  it("produces a signature verifiable with the matching public key", () => {
    const jwt = mintClientAssertion({ clientId, privateKey, audience });
    const [headerPart, payloadPart, signaturePart] = jwt.split(".");
    const signingInput = Buffer.from(`${headerPart}.${payloadPart}`, "utf8");
    const signature = Buffer.from(signaturePart!, "base64url");
    expect(verify("sha256", signingInput, publicKey, signature)).toBe(true);
  });

  it("fails verification against a different public key", () => {
    const { publicKey: otherPublicKey } = generateTestRsaKeyPair();
    const jwt = mintClientAssertion({ clientId, privateKey, audience });
    const [headerPart, payloadPart, signaturePart] = jwt.split(".");
    const signingInput = Buffer.from(`${headerPart}.${payloadPart}`, "utf8");
    const signature = Buffer.from(signaturePart!, "base64url");
    expect(verify("sha256", signingInput, otherPublicKey, signature)).toBe(
      false,
    );
  });

  it("accepts a PEM string in place of a KeyObject", () => {
    const { privateKeyPem } = generateTestRsaKeyPair();
    expect(() =>
      mintClientAssertion({ clientId, privateKey: privateKeyPem, audience }),
    ).not.toThrow();
  });

  it("throws VpayConfigError when lifetimeSeconds exceeds 300", () => {
    expect(() =>
      mintClientAssertion({
        clientId,
        privateKey,
        audience,
        lifetimeSeconds: 301,
      }),
    ).toThrow(VpayConfigError);
  });

  it("throws VpayConfigError when lifetimeSeconds is 0", () => {
    expect(() =>
      mintClientAssertion({
        clientId,
        privateKey,
        audience,
        lifetimeSeconds: 0,
      }),
    ).toThrow(VpayConfigError);
  });

  it("throws VpayConfigError when lifetimeSeconds is not an integer", () => {
    expect(() =>
      mintClientAssertion({
        clientId,
        privateKey,
        audience,
        lifetimeSeconds: 1.5,
      }),
    ).toThrow(VpayConfigError);
  });

  it("accepts the boundary values 1 and 300", () => {
    expect(() =>
      mintClientAssertion({
        clientId,
        privateKey,
        audience,
        lifetimeSeconds: 1,
      }),
    ).not.toThrow();
    expect(() =>
      mintClientAssertion({
        clientId,
        privateKey,
        audience,
        lifetimeSeconds: 300,
      }),
    ).not.toThrow();
  });
});
