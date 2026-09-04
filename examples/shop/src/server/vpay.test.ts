/**
 * `createVpayClient`, and the one setting a merchant server behind an
 * internal name cannot do without.
 *
 * The shop reaches vpay at `VPAY_API_URL` — inside compose, the service name
 * `http://vpay-server:8080`. The client assertion's `aud` claim is a
 * different fact: vpay's OP compares it against its **own** two names for
 * itself, `{deployment.public_base_url}/v1/oauth/token` and that
 * `/v1/oauth` issuer (`vpay_api::op::issuer_for`), and against nothing else.
 * `VPAY_OAUTH_AUDIENCE` is what tells them apart.
 *
 * These tests drive the real `VpayClient` against a real local HTTP server
 * (127.0.0.1 on an ephemeral port — an address reachable only from this
 * process, exactly like a compose service name) and read the assertion off
 * the wire.
 */
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createPublicKey, createPrivateKey, verify } from "node:crypto";
import { afterAll, afterEach, describe, expect, it } from "vitest";
import { loadShopConfig, type EnvRecord } from "./config";
import { createVpayClient } from "./vpay";
import { testPrivateKeyPem } from "../testing/keys";
import {
  reply,
  startVpayTestServer,
  type VpayTestServer,
} from "../testing/vpay-test-server";

/** vpay's `deployment.public_base_url` in the demo stack is the host port. */
const PUBLIC_TOKEN_ENDPOINT = "http://localhost:8080/v1/oauth/token";
const PUBLIC_ISSUER = "http://localhost:8080/v1/oauth";
/** What `authenticate_client` passes as `expected_audiences`. */
const OP_AUDIENCES = [PUBLIC_TOKEN_ENDPOINT, PUBLIC_ISSUER];

const keyDir = mkdtempSync(join(tmpdir(), "vpay-shop-audience-"));
const keyFile = join(keyDir, "shop-merchant.pem");
writeFileSync(keyFile, testPrivateKeyPem(), "utf8");

const servers: VpayTestServer[] = [];

afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => server.close()));
});

afterAll(() => {
  rmSync(keyDir, { recursive: true, force: true });
});

function envFor(apiUrl: string, audience?: string): EnvRecord {
  return {
    VPAY_API_URL: apiUrl,
    VPAY_CLIENT_ID: "shop-merchant",
    VPAY_PRIVATE_KEY_FILE: keyFile,
    VPAY_PUBLISHABLE_KEY: "pk_test_shopmerchantsandbox1",
    VPAY_CHECKOUT_URL: "http://localhost:3080",
    VPAY_WEBHOOK_SECRET: "whsec_shop",
    SHOP_PUBLIC_URL: "http://localhost:3001",
    ...(audience === undefined ? {} : { VPAY_OAUTH_AUDIENCE: audience }),
  };
}

/**
 * Drives one token exchange through the real SDK and returns the
 * `client_assertion` the shop actually put on the wire.
 */
async function assertionSentBy(audience?: string): Promise<string> {
  const server = await startVpayTestServer({
    routes: {
      "GET /v1/balance": reply(200, {
        object: "balance",
        available: [],
        pending: [],
      }),
    },
  });
  servers.push(server);

  const client = createVpayClient(loadShopConfig(envFor(server.url, audience)));
  await client.balance.retrieve();

  const tokenRequest = server.requestsTo("POST", "/v1/oauth/token")[0];
  expect(tokenRequest).toBeDefined();
  const assertion = tokenRequest!.form.get("client_assertion");
  expect(assertion).toBeTruthy();
  // The address this went to is loopback, not the OP's published name — the
  // "internal URL" this whole file is about.
  expect(server.url.startsWith("http://127.0.0.1:")).toBe(true);
  return assertion!;
}

interface AssertionPayload {
  iss: string;
  sub: string;
  aud: string;
  exp: number;
  iat: number;
}

/**
 * The audience check vpay's OP actually performs, reproduced.
 *
 * `authkestra_op::client_assertion::verify_client_assertion` is handed
 * `expected_audiences` by `authenticate_client`, and that list holds the OP's
 * own two names. The URL the merchant happened to POST to is never in it
 * unless it coincides with one of them. A shaped stand-in, not the real
 * verifier — `sdks/rust/tests/op_conformance.rs` runs the same case through
 * the pinned crate itself.
 */
function verifyAsTheOpWould(
  jwt: string,
  expectedClientId: string,
): { ok: true } | { ok: false; reason: string } {
  const [headerPart, payloadPart, signaturePart] = jwt.split(".");
  if (!headerPart || !payloadPart || !signaturePart) {
    return { ok: false, reason: "MalformedAssertion" };
  }
  const publicKey = createPublicKey(createPrivateKey(testPrivateKeyPem()));
  const signingInput = Buffer.from(`${headerPart}.${payloadPart}`, "utf8");
  if (
    !verify(
      "sha256",
      signingInput,
      publicKey,
      Buffer.from(signaturePart, "base64url"),
    )
  ) {
    return { ok: false, reason: "InvalidSignature" };
  }
  const payload = JSON.parse(
    Buffer.from(payloadPart, "base64url").toString("utf8"),
  ) as AssertionPayload;
  if (payload.iss !== expectedClientId || payload.sub !== expectedClientId) {
    return { ok: false, reason: "InvalidIssuer" };
  }
  const now = Math.floor(Date.now() / 1000);
  if (payload.exp <= now || payload.exp - now > 300) {
    return { ok: false, reason: "InvalidLifetime" };
  }
  if (!OP_AUDIENCES.includes(payload.aud)) {
    return { ok: false, reason: "InvalidAudience" };
  }
  return { ok: true };
}

describe("loadShopConfig and VPAY_OAUTH_AUDIENCE", () => {
  it("is optional, and absent means the SDK's default", () => {
    expect(
      loadShopConfig(envFor("http://vpay-server:8080")).vpayOauthAudience,
    ).toBeUndefined();
  });

  it("is read verbatim, path and all — it is not an origin", () => {
    expect(
      loadShopConfig(envFor("http://vpay-server:8080", PUBLIC_TOKEN_ENDPOINT))
        .vpayOauthAudience,
    ).toBe(PUBLIC_TOKEN_ENDPOINT);
  });

  it("treats a blank value as unset rather than signing an empty audience", () => {
    expect(
      loadShopConfig(envFor("http://vpay-server:8080", "   "))
        .vpayOauthAudience,
    ).toBeUndefined();
  });
});

describe("createVpayClient", () => {
  it("reaches vpay at VPAY_API_URL and signs that URL when no audience is configured", async () => {
    const assertion = await assertionSentBy();
    const payload = JSON.parse(
      Buffer.from(assertion.split(".")[1]!, "base64url").toString("utf8"),
    ) as AssertionPayload;
    // The default is the URL we POST to — right only when the shop reaches
    // vpay the way payers do.
    expect(payload.aud).toMatch(
      /^http:\/\/127\.0\.0\.1:\d+\/v1\/oauth\/token$/,
    );
  });

  it("passes a configured VPAY_OAUTH_AUDIENCE through to the assertion", async () => {
    const assertion = await assertionSentBy(PUBLIC_TOKEN_ENDPOINT);
    const payload = JSON.parse(
      Buffer.from(assertion.split(".")[1]!, "base64url").toString("utf8"),
    ) as AssertionPayload;
    expect(payload.aud).toBe(PUBLIC_TOKEN_ENDPOINT);
  });
});

describe("a shop that reaches vpay by an internal name", () => {
  it("is refused by the OP audience check with VPAY_OAUTH_AUDIENCE unset", async () => {
    const verdict = verifyAsTheOpWould(
      await assertionSentBy(),
      "shop-merchant",
    );
    // Everything else about the assertion is correct — the signature
    // verifies, `iss`/`sub` are the client id, the lifetime is in range. The
    // OP still answers `invalid_client`, and nothing in that response says
    // why.
    expect(verdict).toEqual({ ok: false, reason: "InvalidAudience" });
  });

  it("authenticates once VPAY_OAUTH_AUDIENCE names vpay's own token endpoint", async () => {
    expect(
      verifyAsTheOpWould(
        await assertionSentBy(PUBLIC_TOKEN_ENDPOINT),
        "shop-merchant",
      ),
    ).toEqual({ ok: true });
  });

  it("authenticates on the issuer form too, which the OP also accepts", async () => {
    expect(
      verifyAsTheOpWould(await assertionSentBy(PUBLIC_ISSUER), "shop-merchant"),
    ).toEqual({ ok: true });
  });
});
