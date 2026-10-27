/**
 * The merchant SDK client, built from the environment.
 *
 * The signing seed is read from `VPAY_SIGNING_SEED_FILE` and handed to
 * `VpayClient`, which keeps it inside its sealing module and never exposes it
 * again (`sdks/nodejs/src/client.ts` redacts both `util.inspect` and
 * `JSON.stringify`). The seed text itself is not retained here, is never an
 * environment variable of its own, and never reaches a log line or the
 * browser bundle — this module is server-only and is imported only from tRPC
 * procedures and route handlers.
 *
 * The two server keys are public: they are the response keys the shop trusts,
 * pinned so that an answer sealed by anything else is refused.
 */
import { readFileSync } from "node:fs";
import { VpayClient } from "@vaam-apps/vpay-sdk";
import { shopConfig, type ShopConfig } from "./config";

function readKeyFile(path: string, what: string): string {
  try {
    return readFileSync(path, "utf8").trim();
  } catch (cause) {
    throw new Error(
      `examples/shop: cannot read ${what} at ${path}. In the demo stack it is ` +
        `written by \`just gen-demo-keys\`.`,
      { cause },
    );
  }
}

/** Builds a client for an explicit config — the form the tests use. */
export function createVpayClient(config: ShopConfig): VpayClient {
  return new VpayClient({
    baseUrl: config.vpayApiUrl,
    clientId: config.vpayClientId,
    privateKey: readKeyFile(
      config.vpaySigningSeedFile,
      "the merchant signing seed",
    ),
    serverKeys: {
      current: readKeyFile(
        config.vpayServerKeyCurrentFile,
        "vpay's current response key",
      ),
      next: readKeyFile(
        config.vpayServerKeyNextFile,
        "vpay's next response key",
      ),
    },
    // Only when configured: passing `undefined` explicitly is the same as
    // omitting it (the SDK's options are `?: T | undefined`), but spreading
    // keeps the "unset means the SDK default" story literal at the call site.
    ...(config.vpayAudience === undefined
      ? {}
      : { audience: config.vpayAudience }),
  });
}

let cached: VpayClient | undefined;

/** The process-wide client, built on first use. */
export function vpay(): VpayClient {
  cached ??= createVpayClient(shopConfig());
  return cached;
}
