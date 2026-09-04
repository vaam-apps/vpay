/**
 * The merchant SDK client, built from the environment.
 *
 * The private key is read from `VPAY_PRIVATE_KEY_FILE` and handed to
 * `VpayClient`, which parses it once into a `crypto.KeyObject` and never
 * exposes it again (`sdks/nodejs/src/client.ts` redacts both `util.inspect`
 * and `JSON.stringify`). The PEM text itself is not retained here, is never
 * an environment variable of its own, and never reaches a log line or the
 * browser bundle — this module is server-only and is imported only from tRPC
 * procedures and route handlers.
 */
import { readFileSync } from "node:fs";
import { VpayClient } from "@vpay/sdk";
import { shopConfig, type ShopConfig } from "./config";

/** Builds a client for an explicit config — the form the tests use. */
export function createVpayClient(config: ShopConfig): VpayClient {
  let privateKey: string;
  try {
    privateKey = readFileSync(config.vpayPrivateKeyFile, "utf8");
  } catch (cause) {
    throw new Error(
      `examples/shop: cannot read the merchant private key at ` +
        `${config.vpayPrivateKeyFile}. In the demo stack it is written by ` +
        `\`just gen-demo-keys\`.`,
      { cause },
    );
  }
  return new VpayClient({
    baseUrl: config.vpayApiUrl,
    clientId: config.vpayClientId,
    privateKey,
    // Only when configured: passing `undefined` explicitly is the same as
    // omitting it (the SDK's options are `?: T | undefined`), but spreading
    // keeps the "unset means the SDK default" story literal at the call site.
    ...(config.vpayOauthAudience === undefined
      ? {}
      : { assertionAudience: config.vpayOauthAudience }),
  });
}

let cached: VpayClient | undefined;

/** The process-wide client, built on first use. */
export function vpay(): VpayClient {
  cached ??= createVpayClient(shopConfig());
  return cached;
}
