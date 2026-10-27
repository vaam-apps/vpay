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
 * 2. one signed read completes — the configured key is the one this stack
 *    registered, the audience is the stack's own, and the pinned response keys
 *    are the ones it signs with. Done here rather than left to the first test
 *    because a refusal surfaces out of stripe-node as a connection error with
 *    the real cause two levels down, and a suite that discovers it inside a
 *    test reports a hundred of them instead of one.
 */
import { createStripeHttpClient } from "@vaam-apps/vpay-sdk/stripe";

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
      `@vaam-apps/vpay-stripe-compat: no vpay answered ${healthz}. This suite runs OUT OF PROCESS ` +
        `against a real stack and must not be run without one — bring it up with ` +
        `\`just stripe-compat\`.`,
      { cause },
    );
  }
  if (status !== 200) {
    throw new Error(
      `@vaam-apps/vpay-stripe-compat: ${healthz} answered ${status}, not 200. The server is up but ` +
        `not healthy; check \`docker compose logs vpay-server\`.`,
    );
  }

  // One signed read, outside stripe-node.
  const client = createStripeHttpClient({
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.signingSeed,
    serverKeys: { current: env.serverKeyCurrent, next: env.serverKeyNext },
  });
  const url = new URL(env.baseUrl);
  try {
    const response = await client.makeRequest(
      url.hostname,
      env.port,
      "/v1/events?limit=1",
      "GET",
      {},
      "",
      env.protocol,
      HEALTHZ_TIMEOUT_MS,
    );
    if (response.getStatusCode() !== 200) {
      throw new Error(
        `a signed GET /v1/events answered ${response.getStatusCode()}`,
      );
    }
  } catch (cause) {
    throw new Error(
      `@vaam-apps/vpay-stripe-compat: a signed read failed for client_id=${env.clientId} ` +
        `using ${env.signingSeedPath}. The stack must be running the \`demo\` profile overlay ` +
        `that registers this key's public half (\`just gen-demo-keys\` + \`-f compose.demo.yml\`) ` +
        `and the envelope keys under .e2e/envelope/.`,
      { cause },
    );
  }
}
