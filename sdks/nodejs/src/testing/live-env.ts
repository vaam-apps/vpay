/**
 * How the live suite is told which vpay to talk to, and as whom.
 *
 * Three variables, no defaults, and a throw rather than an `undefined` for
 * each — `sdks/stripe-compat/src/env.ts`'s reasoning, applied to this
 * package: a suite that quietly degrades to "no stack, nothing to check" is
 * the failure mode AGENTS.md rule 2 exists to prevent.
 */
import { readFileSync } from "node:fs";

/** Everything the live suite needs to build a {@link VpayClient}. */
export interface LiveEnv {
  /** e.g. `http://localhost:18080`. */
  readonly baseUrl: string;
  /** The registered merchant `client_id` (`just gen-demo-keys` writes `demo-merchant`). */
  readonly clientId: string;
  /** PEM text of that merchant's private key. Read once, never logged. */
  readonly privateKey: string;
  /** Where it was read from — for error messages only. */
  readonly privateKeyPath: string;
}

function required(name: string): string {
  const value = process.env[name];
  if (value === undefined || value.trim() === "") {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: ${name} is not set. These cases run against a REAL vpay ` +
        `stack; \`just sdk-live\` brings one up and sets VPAY_BASE_URL, VPAY_MERCHANT_CLIENT_ID ` +
        `and VPAY_MERCHANT_PRIVATE_KEY_PATH for you.`,
    );
  }
  return value;
}

/** Reads the environment, or throws naming the variable that is missing. */
export function readLiveEnv(): LiveEnv {
  const baseUrl = new URL(required("VPAY_BASE_URL")).origin;
  const clientId = required("VPAY_MERCHANT_CLIENT_ID");
  const privateKeyPath = required("VPAY_MERCHANT_PRIVATE_KEY_PATH");
  let privateKey: string;
  try {
    privateKey = readFileSync(privateKeyPath, "utf8");
  } catch (cause) {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: cannot read the merchant private key at ${privateKeyPath}. ` +
        `\`just gen-demo-keys\` writes one to .e2e/demo-merchant/oauth-signing-key.pem.`,
      { cause },
    );
  }
  return { baseUrl, clientId, privateKey, privateKeyPath };
}
