/**
 * The server half of the transport, for tests: `sdks/rust/wasm` built with its
 * `responder` feature (`pnpm run build:wasm-test`) into `wasm-test/`, which the
 * npm package does not ship.
 *
 * It opens a sealed request the way a vpay server does — finds the `/v1`
 * operation, rebuilds the binding the client signed, verifies, refuses a replay
 * — and seals an answer under the server's key, using cratestack's own server
 * envelope. That is the point: the SDK's suites run against a verifier that is
 * not a copy of the SDK's sealing code. The real `EnvelopeLayer` is exercised
 * against the same sealing path by `sdks/rust/tests`, the integration suites and
 * the live suites.
 */
import { randomBytes } from "node:crypto";
import { createRequire } from "node:module";
import type { HeaderPairs, WasmModule } from "../wasm.js";

export interface OpenedRequest {
  readonly payload: string;
  readonly idempotencyKey: string | undefined;
  sealResponse(
    status: number,
    body: string,
  ): Promise<{ headers: HeaderPairs; body: Uint8Array }>;
}

export interface WasmResponder {
  openRequest(
    method: string,
    path: string,
    query: string | undefined,
    headers: HeaderPairs,
    body: Uint8Array,
  ): Promise<OpenedRequest>;
}

interface ResponderModule extends WasmModule {
  Responder: {
    new (
      serverSeed: string,
      merchantKeys: string[],
      audience: string,
    ): WasmResponder;
    serverPublicKey(serverSeed: string): string;
  };
}

const require = createRequire(import.meta.url);

let loaded: ResponderModule | undefined;

export function loadResponderModule(): ResponderModule {
  loaded ??= require("../../wasm-test/vpay_sdk_wasm.js") as ResponderModule;
  return loaded;
}

/** A merchant key pair and a server key pair, drawn fresh. Never a canned fixture. */
export interface TestIdentity {
  /** The merchant's Ed25519 seed, unpadded base64url. */
  merchantSeed: string;
  /** The registered public half. */
  merchantPublic: string;
  /** The server's Ed25519 seed. */
  serverSeed: string;
  /** The server's public key: what a client pins as `serverKeys.current`. */
  serverPublic: string;
}

export function newTestIdentity(): TestIdentity {
  const wasm = loadResponderModule();
  const merchantSeed = randomBytes(32).toString("base64url");
  const serverSeed = randomBytes(32).toString("base64url");
  const merchantPublic = wasm.Sealer.fromSeed(
    merchantSeed,
    wasm.Responder.serverPublicKey(serverSeed),
    undefined,
    "http://placeholder.invalid",
  ).publicKey;
  return {
    merchantSeed,
    merchantPublic,
    serverSeed,
    serverPublic: wasm.Responder.serverPublicKey(serverSeed),
  };
}
