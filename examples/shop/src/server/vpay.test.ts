/**
 * `createVpayClient`, and the one setting a merchant server behind an
 * internal name cannot do without.
 *
 * The shop reaches vpay at `VPAY_API_URL` — inside compose, the service name
 * `http://vpay-server:8080`. The audience every signed request is addressed to
 * is a different fact: it is vpay's configured `deployment.public_base_url`,
 * the signature binds it, and a server that was configured with another name
 * refuses the request with the coarse unsigned `401`. `VPAY_AUDIENCE` is what
 * tells the two apart.
 *
 * These tests drive the real `VpayClient` against a real local HTTP server
 * that verifies the signature the way vpay does (127.0.0.1 on an ephemeral
 * port — an address reachable only from this process, exactly like a compose
 * service name).
 */
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, describe, expect, it } from "vitest";
import { loadShopConfig, type EnvRecord } from "./config";
import { createVpayClient } from "./vpay";
import {
  startSealedServer,
  type SealedTestServer,
} from "../../../../sdks/nodejs/src/testing/test-server";

/** vpay's `deployment.public_base_url` in the demo stack is the host port. */
const PUBLIC_AUDIENCE = "http://localhost:8080";

const keyDir = mkdtempSync(join(tmpdir(), "vpay-shop-audience-"));
const servers: SealedTestServer[] = [];

afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => server.close()));
});

afterAll(() => {
  rmSync(keyDir, { recursive: true, force: true });
});

/** A server that expects `audience` and answers an empty list. */
async function startVpay(audience?: string): Promise<SealedTestServer> {
  const server = await startSealedServer(
    (_request, response) => {
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end(
        JSON.stringify({ object: "list", data: [], has_more: false }),
      );
    },
    audience === undefined ? {} : { audience },
  );
  servers.push(server);
  return server;
}

/** The environment a shop pointed at `server` is started with. */
function envFor(server: SealedTestServer, audience?: string): EnvRecord {
  const seed = join(keyDir, `seed-${server.identity.merchantPublic}`);
  const current = join(keyDir, `current-${server.identity.serverPublic}`);
  writeFileSync(seed, `${server.identity.merchantSeed}\n`, "utf8");
  writeFileSync(current, `${server.identity.serverPublic}\n`, "utf8");
  return {
    VPAY_API_URL: server.url,
    VPAY_CLIENT_ID: "shop-merchant",
    VPAY_SIGNING_SEED_FILE: seed,
    VPAY_SERVER_KEY_CURRENT_FILE: current,
    // Rotation is not this file's subject; the same key twice is a legal pin.
    VPAY_SERVER_KEY_NEXT_FILE: current,
    VPAY_PUBLISHABLE_KEY: "pk_test_shopmerchantsandbox1",
    VPAY_CHECKOUT_URL: "http://localhost:3080",
    VPAY_WEBHOOK_SECRET: "whsec_shop",
    SHOP_PUBLIC_URL: "http://localhost:3001",
    ...(audience === undefined ? {} : { VPAY_AUDIENCE: audience }),
  };
}

describe("loadShopConfig and VPAY_AUDIENCE", () => {
  it("is optional, and absent means the SDK's default", async () => {
    const server = await startVpay();
    expect(loadShopConfig(envFor(server)).vpayAudience).toBeUndefined();
  });

  it("is read verbatim", async () => {
    const server = await startVpay();
    expect(loadShopConfig(envFor(server, PUBLIC_AUDIENCE)).vpayAudience).toBe(
      PUBLIC_AUDIENCE,
    );
  });

  it("treats a blank value as unset rather than addressing an empty audience", async () => {
    const server = await startVpay();
    expect(loadShopConfig(envFor(server, "   ")).vpayAudience).toBeUndefined();
  });
});

describe("createVpayClient", () => {
  it("signs for VPAY_API_URL when no audience is configured, and a server of that name accepts it", async () => {
    const server = await startVpay();

    const client = createVpayClient(loadShopConfig(envFor(server)));

    await expect(client.paymentIntents.list()).resolves.toMatchObject({
      object: "list",
    });
    expect(server.opened).toHaveLength(1);
  });
});

describe("a shop that reaches vpay by an internal name", () => {
  it("is refused with VPAY_AUDIENCE unset", async () => {
    // The server was configured with its public name; the shop reaches it at
    // 127.0.0.1. Everything else about the request is correct — the key is
    // registered and the signature verifies — and the answer is still the
    // coarse unsigned 401, which says nothing about audiences.
    const server = await startVpay(PUBLIC_AUDIENCE);

    const client = createVpayClient(loadShopConfig(envFor(server)));

    await expect(client.paymentIntents.list()).rejects.toThrow(
      /without a signature/u,
    );
    expect(server.opened).toHaveLength(0);
  });

  it("is accepted once VPAY_AUDIENCE names vpay's public name", async () => {
    const server = await startVpay(PUBLIC_AUDIENCE);

    const client = createVpayClient(
      loadShopConfig(envFor(server, PUBLIC_AUDIENCE)),
    );

    await expect(client.paymentIntents.list()).resolves.toMatchObject({
      object: "list",
    });
    expect(server.opened).toHaveLength(1);
  });
});
