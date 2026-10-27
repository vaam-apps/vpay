#!/usr/bin/env node
/**
 * One signed `/v1` call from a shell: what `curl` cannot do, because the
 * request is a COSE_Sign1 message and not a header on an ordinary one.
 *
 *   node scripts/vpay-call.mjs GET  /payment_intents --query 'limit=3'
 *   node scripts/vpay-call.mjs POST /payment_intents \
 *       --data 'amount=5000&currency=xaf&payment_method_types[0]=mtn_momo' \
 *       --key  order-42
 *   node scripts/vpay-call.mjs POST /payment_intents/pi_1/confirm --data '…'
 *
 * The path is **under `/v1`**. The answer's JSON is printed to stdout, verified
 * (a body that did not verify is an error, never printed); the HTTP status goes
 * to stderr as `status: <n>`; the exit code is 0 for a 2xx and 1 otherwise.
 * `--data` is the form body exactly as `curl -d` would send it, brackets bare;
 * a write without `--key` gets a fresh UUID, printed to stderr as
 * `idempotency-key: <uuid>` so a retry can reuse it.
 *
 * Reads from the environment (the names `sdks/rust/tests/live_*.rs` and the
 * Node live suite read, so one `just sdk-live` environment serves all three):
 *   VPAY_BASE_URL                  required — e.g. http://localhost:18080
 *   VPAY_MERCHANT_SIGNING_SEED_PATH  required — the merchant's Ed25519 seed file
 *   VPAY_SERVER_KEY_CURRENT_PATH   required — vpay's current response key (public)
 *   VPAY_SERVER_KEY_NEXT_PATH      optional — the next one
 *   VPAY_AUDIENCE                  optional — vpay's public audience, if it differs from the base URL
 *
 * Run after `pnpm --filter @vaam-apps/vpay-sdk build` (imports from ../dist).
 */
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { performSealedCall } from "../dist/sealed-call.js";
import { resolveMerchantAuth } from "../dist/sealer.js";

function fail(message) {
  console.error(`vpay-call: ${message}`);
  process.exit(2);
}

function requireEnv(name) {
  const value = process.env[name];
  if (!value) fail(`missing required environment variable: ${name}`);
  return value;
}

function readKey(path, what) {
  try {
    return readFileSync(path, "utf8").trim();
  } catch (err) {
    return fail(`cannot read ${what} at ${path}: ${err.message}`);
  }
}

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    data: { type: "string", short: "d" },
    query: { type: "string", short: "q" },
    key: { type: "string", short: "k" },
  },
});
const [method, path] = positionals;
if (!method || !path || positionals.length > 2) {
  fail(
    "usage: vpay-call.mjs <GET|POST|DELETE> </path-under-/v1> [--data form] [--query q] [--key idempotency-key]",
  );
}
const verb = method.toUpperCase();
if (!["GET", "POST", "DELETE"].includes(verb))
  fail(`no ${verb} operation under /v1`);
if (!path.startsWith("/"))
  fail("the path is under /v1 and starts with /, e.g. /payment_intents");

const nextPath = process.env.VPAY_SERVER_KEY_NEXT_PATH;
const auth = resolveMerchantAuth({
  baseUrl: requireEnv("VPAY_BASE_URL"),
  clientId: process.env.VPAY_MERCHANT_CLIENT_ID || "vpay-call",
  privateKey: readKey(
    requireEnv("VPAY_MERCHANT_SIGNING_SEED_PATH"),
    "the merchant signing seed",
  ),
  serverKeys: {
    current: readKey(
      requireEnv("VPAY_SERVER_KEY_CURRENT_PATH"),
      "vpay's current response key",
    ),
    ...(nextPath
      ? { next: readKey(nextPath, "vpay's next response key") }
      : {}),
  },
  ...(process.env.VPAY_AUDIENCE ? { audience: process.env.VPAY_AUDIENCE } : {}),
});

const write = verb !== "GET";
const idempotencyKey = write ? (values.key ?? randomUUID()) : undefined;
if (write && values.key === undefined)
  console.error(`idempotency-key: ${idempotencyKey}`);

try {
  const { status, text } = await performSealedCall(auth, {
    method: verb,
    path,
    body: verb === "POST" ? (values.data ?? "") : undefined,
    query: verb === "GET" ? values.query : undefined,
    idempotencyKey,
  });
  console.error(`status: ${status}`);
  console.log(text);
  process.exit(status >= 200 && status < 300 ? 0 : 1);
} catch (err) {
  console.error(`vpay-call: ${err.name}: ${err.message}`);
  process.exit(1);
}
