/**
 * `private_key_jwt` client assertion minting and access-token management,
 * per docs/flows/merchant-auth.md.
 */
import {
  randomUUID,
  createPrivateKey,
  sign,
  type KeyObject,
} from "node:crypto";
import {
  VpayAuthError,
  VpayConfigError,
  VpayTransportError,
  VpayUnexpectedResponseError,
  boundedBodyPrefix,
} from "./errors.js";
import { encodeForm } from "./form.js";

/** RFC 7523's fixed URN for this grant's `client_assertion_type`. */
export const CLIENT_ASSERTION_TYPE_JWT_BEARER =
  "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/** `authkestra-op`'s ceiling on how far in the future `exp` may sit (`MAX_CLIENT_ASSERTION_LIFETIME_SECS`). */
export const MAX_ASSERTION_LIFETIME_SECONDS = 300;

const MIN_ASSERTION_LIFETIME_SECONDS = 1;
const DEFAULT_ASSERTION_LIFETIME_SECONDS = 60;

/** Validates a configured assertion lifetime against the OP's `1..=300` bound. */
export function validateAssertionLifetimeSeconds(
  lifetimeSeconds: number,
): void {
  if (
    !Number.isInteger(lifetimeSeconds) ||
    lifetimeSeconds < MIN_ASSERTION_LIFETIME_SECONDS ||
    lifetimeSeconds > MAX_ASSERTION_LIFETIME_SECONDS
  ) {
    throw new VpayConfigError(
      `assertionLifetimeSeconds must be an integer between ${MIN_ASSERTION_LIFETIME_SECONDS} and ` +
        `${MAX_ASSERTION_LIFETIME_SECONDS}, got ${lifetimeSeconds}`,
    );
  }
}

function base64url(input: Buffer): string {
  return input.toString("base64url");
}

export interface MintClientAssertionOptions {
  clientId: string;
  privateKey: string | KeyObject;
  /** The token endpoint URL — the assertion's `aud` (docs/flows/merchant-auth.md). */
  audience: string;
  kid?: string | undefined;
  /** Default 60. Must be an integer in `1..=300`. */
  lifetimeSeconds?: number | undefined;
  /** Unix seconds. Defaults to `Date.now() / 1000`, injectable for tests. */
  now?: number | undefined;
}

/**
 * Mints an RS256 `private_key_jwt` client assertion
 * (docs/flows/merchant-auth.md, "The client assertion" table).
 *
 * Signed with `crypto.sign('sha256', ..., rsaKey)`, which for an RSA key
 * without an explicit padding option produces PKCS#1 v1.5 — exactly RS256.
 */
export function mintClientAssertion(
  options: MintClientAssertionOptions,
): string {
  const lifetimeSeconds =
    options.lifetimeSeconds ?? DEFAULT_ASSERTION_LIFETIME_SECONDS;
  validateAssertionLifetimeSeconds(lifetimeSeconds);

  const now = options.now ?? Math.floor(Date.now() / 1000);

  const header: { alg: "RS256"; typ: "JWT"; kid?: string } = {
    alg: "RS256",
    typ: "JWT",
  };
  if (options.kid !== undefined) {
    header.kid = options.kid;
  }

  const payload = {
    iss: options.clientId,
    sub: options.clientId,
    aud: options.audience,
    jti: randomUUID(),
    exp: now + lifetimeSeconds,
    iat: now,
  };

  const encodedHeader = base64url(Buffer.from(JSON.stringify(header), "utf8"));
  const encodedPayload = base64url(
    Buffer.from(JSON.stringify(payload), "utf8"),
  );
  const signingInput = `${encodedHeader}.${encodedPayload}`;

  const keyObject =
    typeof options.privateKey === "string"
      ? createPrivateKey(options.privateKey)
      : options.privateKey;
  const signature = sign(
    "sha256",
    Buffer.from(signingInput, "utf8"),
    keyObject,
  );

  return `${signingInput}.${base64url(signature)}`;
}

interface TokenResponseBody {
  access_token: string;
  token_type: string;
  expires_in: number;
  scope?: string;
}

/**
 * Recognises `authkestra_op::handlers::token::TokenResponse`.
 *
 * `token_type` must be `Bearer`, compared case-insensitively per RFC 6749
 * §7.1 (the value is a case-insensitive token, and OPs differ on `Bearer` vs
 * `bearer`). Anything else — a `MAC` or `DPoP` token, say — is a token this
 * SDK does not know how to present: it only ever sends
 * `Authorization: Bearer …`. Such a response is reported as a
 * {@link VpayUnexpectedResponseError}, the same as any other 200 whose body
 * is not the documented shape; it is not an authentication *failure*, so it
 * is deliberately not a {@link VpayAuthError}.
 */
function isTokenResponseBody(value: unknown): value is TokenResponseBody {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  const tokenType = record["token_type"];
  return (
    typeof record["access_token"] === "string" &&
    typeof tokenType === "string" &&
    tokenType.toLowerCase() === "bearer" &&
    typeof record["expires_in"] === "number"
  );
}

function isTokenErrorBody(
  value: unknown,
): value is { error: string; error_description?: string } {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Record<string, unknown>)["error"] === "string"
  );
}

export interface TokenManagerOptions {
  clientId: string;
  privateKey: string | KeyObject;
  kid: string | undefined;
  tokenEndpoint: string;
  audience: string;
  scope: string | undefined;
  assertionLifetimeSeconds: number;
  timeoutMs: number;
  fetchImpl: typeof fetch;
  userAgent: string;
}

/**
 * Caches the merchant's `/v1` access token and mints a fresh one only when
 * needed, per docs/flows/merchant-auth.md's "Using the token" section:
 *
 * - Cached until `expires_in − margin` has elapsed (margin: 30s, or half of
 *   `expires_in` for short TTLs — integer arithmetic only).
 * - Concurrent callers share one in-flight token request (a single `jti`
 *   spent per refresh, not one per caller).
 * - {@link TokenManager.invalidate} lets the caller force a re-auth on a 401.
 */
export class TokenManager {
  readonly #options: TokenManagerOptions;
  #cached: { accessToken: string; expiresAtMs: number } | undefined;
  #inflight: Promise<string> | undefined;

  constructor(options: TokenManagerOptions) {
    this.#options = options;
  }

  async getToken(): Promise<string> {
    const now = Date.now();
    if (this.#cached && this.#cached.expiresAtMs > now) {
      return this.#cached.accessToken;
    }
    if (!this.#inflight) {
      this.#inflight = this.#fetchToken().finally(() => {
        this.#inflight = undefined;
      });
    }
    return this.#inflight;
  }

  /** Discards the cached token, forcing the next {@link getToken} call to re-authenticate. */
  invalidate(): void {
    this.#cached = undefined;
  }

  async #fetchToken(): Promise<string> {
    const assertion = mintClientAssertion({
      clientId: this.#options.clientId,
      privateKey: this.#options.privateKey,
      // `aud` is the **token endpoint URL**, never the `audience` request
      // parameter below (`vpay:v1`). RFC 7523 §3 and the OP's own
      // `authenticate_client` both compare it against the endpoint that
      // received the request; the two are different values with different
      // jobs (docs/flows/merchant-auth.md, "The client assertion").
      audience: this.#options.tokenEndpoint,
      kid: this.#options.kid,
      lifetimeSeconds: this.#options.assertionLifetimeSeconds,
    });

    const formBody: Record<string, string> = {
      grant_type: "client_credentials",
      client_id: this.#options.clientId,
      client_assertion_type: CLIENT_ASSERTION_TYPE_JWT_BEARER,
      client_assertion: assertion,
      audience: this.#options.audience,
    };
    if (this.#options.scope !== undefined) {
      formBody["scope"] = this.#options.scope;
    }

    let status: number;
    let ok: boolean;
    let text: string;
    try {
      const response = await this.#options.fetchImpl(
        this.#options.tokenEndpoint,
        {
          method: "POST",
          headers: {
            "Content-Type": "application/x-www-form-urlencoded",
            Accept: "application/json",
            "User-Agent": this.#options.userAgent,
          },
          body: encodeForm(formBody),
          signal: AbortSignal.timeout(this.#options.timeoutMs),
        },
      );
      status = response.status;
      ok = response.ok;
      // Read the body inside the same `try`: `fetch` resolves on headers, so
      // a body that stalls (or the timeout firing mid-stream) rejects here.
      // Outside this block it escapes as a raw `DOMException: TimeoutError`
      // rather than a `VpayTransportError` carrying it as `cause`.
      text = await response.text();
    } catch (err) {
      throw new VpayTransportError("token request failed", { cause: err });
    }

    let parsed: unknown;
    try {
      parsed = text.length > 0 ? JSON.parse(text) : undefined;
    } catch {
      parsed = undefined;
    }

    if (!ok) {
      if (isTokenErrorBody(parsed)) {
        throw new VpayAuthError(parsed.error, parsed.error_description);
      }
      throw new VpayUnexpectedResponseError(status, boundedBodyPrefix(text));
    }

    if (!isTokenResponseBody(parsed)) {
      throw new VpayUnexpectedResponseError(status, boundedBodyPrefix(text));
    }

    const margin = Math.min(30, Math.floor(parsed.expires_in / 2));
    const ttlSeconds = Math.max(parsed.expires_in - margin, 0);
    this.#cached = {
      accessToken: parsed.access_token,
      expiresAtMs: Date.now() + ttlSeconds * 1000,
    };
    return parsed.access_token;
  }
}
