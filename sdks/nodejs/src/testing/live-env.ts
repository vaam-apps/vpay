/**
 * How the live suite is told which vpay to talk to, and as whom.
 *
 * Five variables, no defaults, and a throw rather than an `undefined` for
 * each — `sdks/stripe-compat/src/env.ts`'s reasoning, applied to this
 * package: a suite that quietly degrades to "no stack, nothing to check" is
 * the failure mode AGENTS.md rule 2 exists to prevent. The names are the ones
 * `sdks/rust/tests/live_*.rs` reads, so one `just sdk-live` environment serves
 * both SDKs.
 */
import { readFileSync } from "node:fs";

/** Everything the live suite needs to build a {@link VpayClient}. */
export interface LiveEnv {
  /** e.g. `http://localhost:18080`. */
  readonly baseUrl: string;
  /** The registered merchant `client_id` (`just gen-demo-keys` writes `demo-merchant`). */
  readonly clientId: string;
  /** The merchant's Ed25519 seed, unpadded base64url. Read once, never logged. */
  readonly privateKey: string;
  /** Where it was read from — for error messages only. */
  readonly privateKeyPath: string;
  /** vpay's current response key (public), unpadded base64url. */
  readonly serverKeyCurrent: string;
  /** The key it will use after the next rotation. */
  readonly serverKeyNext: string;
}

function required(name: string): string {
  const value = process.env[name];
  if (value === undefined || value.trim() === "") {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: ${name} is not set. These cases run against a REAL vpay ` +
        `stack; \`just sdk-live\` brings one up and sets VPAY_BASE_URL, VPAY_MERCHANT_CLIENT_ID, ` +
        `VPAY_MERCHANT_SIGNING_SEED_PATH, VPAY_SERVER_KEY_CURRENT_PATH and ` +
        `VPAY_SERVER_KEY_NEXT_PATH for you.`,
    );
  }
  return value;
}

function readKeyFile(path: string, what: string): string {
  try {
    return readFileSync(path, "utf8").trim();
  } catch (cause) {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: cannot read ${what} at ${path}. ` +
        `\`just gen-demo-keys\` writes them under .e2e/demo-merchant/.`,
      { cause },
    );
  }
}

/** Reads the environment, or throws naming the variable that is missing. */
export function readLiveEnv(): LiveEnv {
  const baseUrl = new URL(required("VPAY_BASE_URL")).origin;
  const clientId = required("VPAY_MERCHANT_CLIENT_ID");
  const privateKeyPath = required("VPAY_MERCHANT_SIGNING_SEED_PATH");
  return {
    baseUrl,
    clientId,
    privateKey: readKeyFile(privateKeyPath, "the merchant signing seed"),
    privateKeyPath,
    serverKeyCurrent: readKeyFile(
      required("VPAY_SERVER_KEY_CURRENT_PATH"),
      "vpay's current response key",
    ),
    serverKeyNext: readKeyFile(
      required("VPAY_SERVER_KEY_NEXT_PATH"),
      "vpay's next response key",
    ),
  };
}

/** The options a {@link VpayClient} is built from. */
export function liveClientOptions(env: LiveEnv): {
  baseUrl: string;
  clientId: string;
  privateKey: string;
  serverKeys: { current: string; next: string };
} {
  return {
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.privateKey,
    serverKeys: { current: env.serverKeyCurrent, next: env.serverKeyNext },
  };
}
