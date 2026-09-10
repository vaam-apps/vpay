/**
 * vitest `globalSetup` for the live suite: prove a real vpay is there before
 * a single case runs, and **fail** — never skip — when it is not.
 *
 * Two checks, in order, because they fail for different reasons and the
 * message must say which:
 *
 * 1. `GET /healthz` answers `200` — the server is up and its database is
 *    reachable (that endpoint is a real `SELECT 1`, not a static "ok").
 * 2. one authenticated read completes — the configured `client_id` and
 *    private key are the pair this stack registered.
 */
import { VpayClient } from "../client.js";

import { readLiveEnv } from "./live-env.js";

/** How long the preflight waits for `/healthz`. */
const HEALTHZ_TIMEOUT_MS = 10_000;

export default async function preflight(): Promise<void> {
  const env = readLiveEnv();

  const healthz = `${env.baseUrl}/healthz`;
  let status: number;
  try {
    const response = await fetch(healthz, {
      signal: AbortSignal.timeout(HEALTHZ_TIMEOUT_MS),
    });
    status = response.status;
  } catch (cause) {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: no vpay answered ${healthz}. These cases run against a ` +
        `real stack and must not be run without one — bring it up with \`just sdk-live\`.`,
      { cause },
    );
  }
  if (status !== 200) {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: ${healthz} answered ${status}, not 200. The server is up ` +
        `but not healthy; check \`docker compose logs vpay-server\`.`,
    );
  }

  const client = new VpayClient({
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.privateKey,
  });
  try {
    await client.events.list({ limit: 1 });
  } catch (cause) {
    throw new Error(
      `@vaam-apps/vpay-sdk live suite: the merchant handshake failed for ` +
        `client_id=${env.clientId} using ${env.privateKeyPath}. The stack must be running the ` +
        `\`demo\` profile overlay that registers this key's public half.`,
      { cause },
    );
  }
}
