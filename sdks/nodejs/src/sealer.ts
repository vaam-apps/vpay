/**
 * The merchant's signing identity and the keys it trusts, resolved once.
 *
 * vpay does not issue tokens. Every `/v1` call is a COSE_Sign1 message signed
 * with an **Ed25519** key the operator registered for this merchant
 * (`merchant_clients[].cose_keys`), and every answer is one signed by vpay's
 * response key, which this client pins (`serverKeys`). There is no
 * `client_credentials` grant, no assertion, no access-token cache and no
 * re-authentication retry: `docs/signed-transport.md`, and
 * `docs/flows/merchant-auth.md` for the flow.
 */
import {
  createPrivateKey,
  createPublicKey,
  sign,
  type KeyObject,
} from "node:crypto";
import type { Agent as HttpAgent } from "node:http";
import type { Agent as HttpsAgent } from "node:https";
import { VpayConfigError } from "./errors.js";
import { SDK_VERSION } from "./version.js";
import { isWasmFailure, loadWasm, type WasmSealer } from "./wasm.js";

/** Default timeout for every resource call, in milliseconds. */
export const DEFAULT_TIMEOUT_MS = 30_000;

/** A key given as bytes or as the unpadded base64url text a key file holds. */
export type KeyBytes = string | Uint8Array;

/**
 * A signer whose key this process cannot read: a KMS, an HSM, a Node
 * `KeyObject` held elsewhere. `sign` receives the exact bytes to sign and must
 * return the 64-byte pure Ed25519 signature over them — `crypto.sign(null, tbs,
 * key)`, never a pre-hash. A signature that does not verify under `publicKey`
 * is refused before it is sent, rather than failing at vpay as the coarse 401.
 */
export interface ExternalSigner {
  /** The registered public key: 32 bytes, or unpadded base64url. */
  publicKey: KeyBytes;
  sign(tbs: Uint8Array): Uint8Array | Promise<Uint8Array>;
}

/** The response keys this client trusts. */
export interface ServerKeysOption {
  /** The key vpay seals answers with now: 32 bytes, or unpadded base64url. */
  current: KeyBytes;
  /**
   * The key it will use after the next rotation, so a rotation is not a flag
   * day. Omit it for a first deployment that has not prepared one.
   */
  next?: KeyBytes | undefined;
}

/**
 * The options that decide how this package signs: everything the sealer needs,
 * plus the `baseUrl` the defaults are derived from.
 *
 * Shared by {@link VpayClient} and `createStripeHttpClient` on purpose. The two
 * entry points seal the same calls under the same keys with the same defaults;
 * resolving them in one place is what stops them drifting.
 *
 * Every optional property is `?: T | undefined` for the
 * `exactOptionalPropertyTypes` reason spelled out on `VpayClientOptions`.
 */
export interface MerchantAuthOptions {
  /** vpay's base URL, e.g. `https://api.vpay.example`. */
  baseUrl: string;
  /** This merchant's registered `client_id`. Kept for logs; it is not sent. */
  clientId: string;
  /**
   * The merchant's Ed25519 private key, in any of the forms an operator has
   * it in: a PKCS#8 PEM text or a `crypto.KeyObject` (the key then stays in
   * Node and signs through `crypto.sign`), or the 32-byte RFC 8032 seed as
   * bytes or unpadded base64url (the format `cargo xtask gen-envelope-key`
   * writes). Exactly one of this and {@link signer}.
   */
  privateKey?: string | KeyObject | Uint8Array | undefined;
  /** A signer for a key this process cannot read. Exactly one of this and {@link privateKey}. */
  signer?: ExternalSigner | undefined;
  /** The response keys to trust (`vpay-server` logs them at boot). */
  serverKeys: ServerKeysOption;
  /**
   * What the signature is addressed to: vpay's configured audience, which is
   * its `deployment.public_base_url`. Default {@link baseUrl}. **Set it when
   * your server reaches vpay by a different URL than payers do** (a compose
   * service name, a private DNS name): the signature binds this value, and a
   * mismatch is the coarse 401 with nothing in it to say why.
   */
  audience?: string | undefined;
  /** Default {@link DEFAULT_TIMEOUT_MS}. */
  timeoutMs?: number | undefined;
  /** An `http.Agent`/`https.Agent` for a proxy or a pool. Defaults to the global agent. */
  agent?: HttpAgent | HttpsAgent | undefined;
}

/** What {@link resolveMerchantAuth} hands back. */
export interface ResolvedMerchantAuth {
  /** `baseUrl` with any trailing slash removed. */
  baseUrl: string;
  clientId: string;
  timeoutMs: number;
  userAgent: string;
  agent: HttpAgent | HttpsAgent | undefined;
  sealer: WasmSealer;
}

function requireNonEmptyString(
  value: string | undefined,
  field: string,
): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new VpayConfigError(`${field} is required`);
  }
  return value;
}

function stripTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

/**
 * 32 bytes as unpadded base64url. Anything else is a {@link VpayConfigError}
 * naming the field and never quoting the value — it may be a private seed.
 */
export function toKeyBase64url(
  value: KeyBytes | undefined,
  field: string,
): string {
  if (value === undefined) {
    throw new VpayConfigError(`${field} is required`);
  }
  if (typeof value === "string") {
    const text = value.trim();
    const bytes = Buffer.from(text, "base64url");
    if (
      bytes.length !== 32 ||
      bytes.toString("base64url") !== text // rejects padding and other alphabets
    ) {
      throw new VpayConfigError(
        `${field} must be 32 bytes, as unpadded base64url text`,
      );
    }
    return text;
  }
  if (value.length !== 32) {
    throw new VpayConfigError(`${field} must be exactly 32 bytes`);
  }
  return Buffer.from(value).toString("base64url");
}

function ed25519PublicBase64url(key: KeyObject): string {
  const jwk = createPublicKey(key).export({ format: "jwk" });
  if (typeof jwk.x !== "string") {
    throw new VpayConfigError("privateKey has no Ed25519 public half");
  }
  return jwk.x;
}

/**
 * Turns a configured PEM or `KeyObject` into a signer, once. A bad key is a
 * startup failure (a {@link VpayConfigError} from the constructor the merchant
 * wrote), not an exception out of the first request; the message never quotes
 * the key, and `cause` carries OpenSSL's own reason.
 */
function signerFromKeyObject(privateKey: string | KeyObject): ExternalSigner {
  let key: KeyObject;
  if (typeof privateKey === "string") {
    try {
      key = createPrivateKey(privateKey);
    } catch (err) {
      throw new VpayConfigError(
        "privateKey could not be read as a private key: expected PKCS#8 PEM text, a crypto.KeyObject, or a 32-byte Ed25519 seed",
        { cause: err },
      );
    }
  } else {
    key = privateKey;
  }
  if (key.type !== "private" || key.asymmetricKeyType !== "ed25519") {
    throw new VpayConfigError(
      `privateKey must be an Ed25519 private key (got ${key.asymmetricKeyType ?? key.type}); ` +
        "vpay's signed transport has no RSA or P-256 path for merchants",
    );
  }
  return {
    publicKey: ed25519PublicBase64url(key),
    sign: (tbs) => sign(null, tbs, key),
  };
}

function looksLikePem(text: string): boolean {
  return text.includes("-----BEGIN");
}

/**
 * Validates {@link MerchantAuthOptions}, applies this package's defaults, and
 * builds the sealer both entry points share.
 *
 * Every failure it can detect is detected here — at construction — rather than
 * on the first request.
 */
export function resolveMerchantAuth(
  options: MerchantAuthOptions,
): ResolvedMerchantAuth {
  const baseUrl = stripTrailingSlash(
    requireNonEmptyString(options.baseUrl, "baseUrl"),
  );
  const clientId = requireNonEmptyString(options.clientId, "clientId");
  const hasKey =
    options.privateKey !== undefined && options.privateKey !== null;
  const hasSigner = options.signer !== undefined && options.signer !== null;
  if (hasKey === hasSigner) {
    throw new VpayConfigError(
      hasKey
        ? "give privateKey or signer, not both"
        : "privateKey (or signer) is required",
    );
  }
  if (options.serverKeys === undefined || options.serverKeys === null) {
    throw new VpayConfigError(
      "serverKeys is required: the client refuses to trust an answer it has no key for",
    );
  }
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isInteger(timeoutMs) || timeoutMs <= 0) {
    throw new VpayConfigError(
      `timeoutMs must be a positive integer, got ${timeoutMs}`,
    );
  }
  const audience = stripTrailingSlash(options.audience ?? baseUrl);
  const current = toKeyBase64url(
    options.serverKeys.current,
    "serverKeys.current",
  );
  const next =
    options.serverKeys.next === undefined
      ? undefined
      : toKeyBase64url(options.serverKeys.next, "serverKeys.next");

  const wasm = loadWasm();

  let sealer: WasmSealer;
  try {
    const external = options.signer;
    const privateKey = options.privateKey;
    if (external !== undefined && external !== null) {
      sealer = wasm.Sealer.fromCallback(
        toKeyBase64url(external.publicKey, "signer.publicKey"),
        (tbs) => external.sign(tbs),
        current,
        next,
        audience,
      );
    } else if (
      privateKey instanceof Uint8Array ||
      (typeof privateKey === "string" && !looksLikePem(privateKey))
    ) {
      sealer = wasm.Sealer.fromSeed(
        toKeyBase64url(privateKey, "privateKey"),
        current,
        next,
        audience,
      );
    } else if (privateKey !== undefined && privateKey !== null) {
      const signer = signerFromKeyObject(privateKey);
      sealer = wasm.Sealer.fromCallback(
        signer.publicKey as string,
        (tbs) => signer.sign(tbs),
        current,
        next,
        audience,
      );
    } else {
      throw new VpayConfigError("privateKey (or signer) is required");
    }
  } catch (err) {
    if (err instanceof VpayConfigError) throw err;
    // The wasm module rejects with a plain `{ kind, message }`; its messages
    // describe the key's shape, never its bytes.
    throw new VpayConfigError(
      isWasmFailure(err)
        ? err.message
        : "the signing identity could not be built",
      { cause: err },
    );
  }

  return {
    baseUrl,
    clientId,
    timeoutMs,
    userAgent: `vpay-sdk-node/${SDK_VERSION}`,
    agent: options.agent,
    sealer,
  };
}
