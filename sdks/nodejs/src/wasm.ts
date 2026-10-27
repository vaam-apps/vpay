/**
 * The seam to `sdks/rust/wasm`: cratestack's own envelope, compiled to
 * WebAssembly (`just wasm-build`, or `pnpm run build:wasm` in this package).
 *
 * The signature binds a dozen things byte for byte (audience, method, route
 * template, path parameters, canonical query, contract digest, payload type,
 * `Idempotency-Key`), so this SDK does not reimplement the envelope in
 * TypeScript beside the Rust one: `vpay-sdk` seals through the same crate, and
 * this package calls it. The module is CommonJS (what `wasm-bindgen --target
 * nodejs` emits, in a directory whose `package.json` says so), loaded lazily
 * so that `import "@vaam-apps/vpay-sdk"` costs nothing until a client is built.
 *
 * The shapes below are written out by hand rather than read from the generated
 * `.d.ts`: that file does not exist in a checkout until the wasm is built, and
 * `tsc` must not depend on it.
 */
import { createRequire } from "node:module";
import { VpayConfigError } from "./errors.js";

/** `[name, value]` header pairs, as the wasm module takes and returns them. */
export type HeaderPairs = [string, string][];

/** A sealed call: the bytes and headers to send, and what the answer is opened with. */
export interface WasmSealedCall {
  readonly body: Uint8Array;
  readonly headers: HeaderPairs;
  /** Verifies the answer and returns its JSON text. Rejects with a {@link WasmFailure}. */
  openResponse(
    status: number,
    headers: HeaderPairs,
    body: Uint8Array,
  ): Promise<string>;
}

export interface WasmSealer {
  readonly publicKey: string;
  sealCall(
    method: string,
    path: string,
    body: string | undefined,
    query: string | undefined,
    idempotencyKey: string | undefined,
  ): Promise<WasmSealedCall>;
}

/** What the wasm module rejects with: a plain object, never an `Error`. */
export interface WasmFailure {
  kind: "unbound" | "seal" | "unsigned" | "unverified" | "invalid";
  message: string;
  status?: number;
}

export interface WasmModule {
  Sealer: {
    fromSeed(
      seedBase64url: string,
      serverCurrent: string,
      serverNext: string | undefined,
      audience: string,
    ): WasmSealer;
    fromCallback(
      publicKeyBase64url: string,
      sign: (tbs: Uint8Array) => Uint8Array | Promise<Uint8Array>,
      serverCurrent: string,
      serverNext: string | undefined,
      audience: string,
    ): WasmSealer;
  };
}

const require = createRequire(import.meta.url);

let loaded: WasmModule | undefined;

/** Loads the sealing module (once). */
export function loadWasm(): WasmModule {
  if (loaded === undefined) {
    try {
      loaded = require("../wasm/vpay_sdk_wasm.js") as WasmModule;
    } catch (err) {
      throw new VpayConfigError(
        "the sealing module (wasm/vpay_sdk_wasm.js) could not be loaded: " +
          "it is built from sdks/rust/wasm — run `pnpm run build:wasm` in sdks/nodejs, " +
          "or install the published package, which ships it",
        { cause: err },
      );
    }
  }
  return loaded;
}

/** Whether a rejection is one of the wasm module's own failure objects. */
export function isWasmFailure(value: unknown): value is WasmFailure {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Record<string, unknown>)["kind"] === "string" &&
    typeof (value as Record<string, unknown>)["message"] === "string"
  );
}
