#!/usr/bin/env node
/**
 * Mints one `private_key_jwt` client assertion and prints it, alongside the
 * public JWK derived from the same private key, as JSON on stdout:
 *
 *   { "assertion": "<jwt>", "jwks": { "keys": [<public JWK>] } }
 *
 * Intended for a Rust example to feed straight into the real
 * `authkestra_op::client_assertion::verify_client_assertion` — see
 * docs/flows/merchant-auth.md and docs/status.md's "Node SDK" row.
 *
 * Reads from the environment:
 *   VPAY_CLIENT_ID          required
 *   VPAY_PRIVATE_KEY_FILE   required — path to a PEM private key
 *   VPAY_AUDIENCE           required — the assertion's `aud` (token endpoint URL)
 *   VPAY_KID                optional
 *
 * Run after `pnpm --filter @vaam-apps/vpay-sdk build` (imports from ../dist).
 */
import { readFileSync } from "node:fs";
import { createPrivateKey, createPublicKey } from "node:crypto";
import { mintClientAssertion } from "../dist/index.js";

function requireEnv(name) {
  const value = process.env[name];
  if (!value) {
    console.error(`missing required environment variable: ${name}`);
    process.exit(1);
  }
  return value;
}

const clientId = requireEnv("VPAY_CLIENT_ID");
const privateKeyFile = requireEnv("VPAY_PRIVATE_KEY_FILE");
const audience = requireEnv("VPAY_AUDIENCE");
const kid = process.env.VPAY_KID || undefined;

let privateKeyPem;
try {
  privateKeyPem = readFileSync(privateKeyFile, "utf8");
} catch (err) {
  console.error(
    `cannot read VPAY_PRIVATE_KEY_FILE (${privateKeyFile}): ${err.message}`,
  );
  process.exit(1);
}

let privateKey;
try {
  privateKey = createPrivateKey(privateKeyPem);
} catch (err) {
  console.error(
    `VPAY_PRIVATE_KEY_FILE (${privateKeyFile}) is not a readable PEM private key: ${err.message}`,
  );
  process.exit(1);
}
const publicKey = createPublicKey(privateKey);

const assertion = mintClientAssertion({
  clientId,
  privateKey,
  audience,
  ...(kid ? { kid } : {}),
});

const jwk = publicKey.export({ format: "jwk" });
if (kid) {
  jwk.kid = kid;
}

process.stdout.write(
  `${JSON.stringify({ assertion, jwks: { keys: [jwk] } })}\n`,
);
