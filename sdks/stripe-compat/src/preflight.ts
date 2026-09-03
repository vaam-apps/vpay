/**
 * vitest `globalSetup`: prove a real vpay is there before a single case runs,
 * and **fail** — never skip — when it is not.
 *
 * This is the file that makes the suite trustworthy. A conformance suite that
 * skips itself when the stack is down reports `ok` with zero cases and is
 * indistinguishable, in a CI summary, from one that passed. AGENTS.md: "a
 * green run never overstates coverage."
 *
 * Two checks, in order, because they fail for different reasons and the
 * messages must say which:
 *
 * 1. `GET /healthz` answers `200` — the server is up and its database is
 *    reachable (that endpoint is a real `SELECT 1`, not a static "ok").
 * 2. the merchant handshake completes — the configured `client_id` and
 *    private key are the pair this stack registered. Done here rather than
 *    left to the first test because a `invalid_client` surfacing out of
 *    stripe-node is the one failure mode `@vpay/sdk`'s README documents as
 *    *never settling* (a stripe-node defect), so a suite that discovers it
 *    inside a test hangs instead of reporting.
 */
import { createStripeAuthenticator } from "@vpay/sdk/stripe";

import { readCompatEnv } from "./env.js";

/** How long the preflight waits for `/healthz`. */
const HEALTHZ_TIMEOUT_MS = 10_000;

export default async function preflight(): Promise<void> {
  const env = readCompatEnv();

  const healthz = `${env.baseUrl}/healthz`;
  let status: number;
  try {
    const response = await fetch(healthz, {
      signal: AbortSignal.timeout(HEALTHZ_TIMEOUT_MS),
    });
    status = response.status;
  } catch (cause) {
    throw new Error(
      `@vpay/stripe-compat: no vpay answered ${healthz}. This suite runs OUT OF PROCESS ` +
        `against a real stack and must not be run without one — bring it up with ` +
        `\`just stripe-compat\`.`,
      { cause },
    );
  }
  if (status !== 200) {
    throw new Error(
      `@vpay/stripe-compat: ${healthz} answered ${status}, not 200. The server is up but ` +
        `not healthy; check \`docker compose logs vpay-server\`.`,
    );
  }

  // The handshake, once, outside stripe-node — see the module doc.
  const authenticator = createStripeAuthenticator({
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.privateKeyPem,
  });
  const probe: { headers: Record<string, string | number | string[]> } = {
    headers: {},
  };
  try {
    await authenticator(
      // The authenticator reads `headers` and nothing else; the rest of
      // stripe-node's `StripeRequest` is not needed to mint a token, and
      // inventing values for it here would suggest it were.
      probe as Parameters<typeof authenticator>[0],
    );
  } catch (cause) {
    throw new Error(
      `@vpay/stripe-compat: the merchant handshake failed for client_id=${env.clientId} ` +
        `using ${env.privateKeyPath}. The stack must be running the \`demo\` profile overlay ` +
        `that registers this key's public half (\`just gen-demo-keys\` + \`-f compose.demo.yml\`).`,
      { cause },
    );
  }
  if (typeof probe.headers["Authorization"] !== "string") {
    throw new Error(
      "@vpay/stripe-compat: the handshake resolved but set no Authorization header.",
    );
  }
}
