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
import { SDK_VERSION } from "./version.js";

/** RFC 7523's fixed URN for this grant's `client_assertion_type`. */
export const CLIENT_ASSERTION_TYPE_JWT_BEARER =
  "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/** `authkestra-op`'s ceiling on how far in the future `exp` may sit (`MAX_CLIENT_ASSERTION_LIFETIME_SECS`). */
export const MAX_ASSERTION_LIFETIME_SECONDS = 300;

const MIN_ASSERTION_LIFETIME_SECONDS = 1;

/**
 * Default client-assertion lifetime, in seconds. Well inside the OP's
 * `1..=300` bound — see the README's note on why raising it is a trap.
 */
export const DEFAULT_ASSERTION_LIFETIME_SECONDS = 60;

/**
 * Default OAuth2 `audience` request parameter. Server-side the same string is
 * `vpay_config::MERCHANT_AUDIENCE`; this package keeps its own copy, so the
 * two must change together (docs/flows/merchant-auth.md, "The token request").
 */
export const DEFAULT_AUDIENCE = "vpay:v1";

/** Default timeout for the token exchange and every resource call, in milliseconds. */
export const DEFAULT_TIMEOUT_MS = 30_000;

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
  /**
   * The assertion's `aud` claim: the OP's own token endpoint URL, or its
   * issuer identifier — both are accepted by `authenticate_client`
   * (docs/flows/merchant-auth.md). Not necessarily the URL the caller POSTs
   * to; see `MerchantAuthOptions.assertionAudience`.
   */
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
  /**
   * The signing key as a parsed {@link KeyObject}, never PEM text.
   *
   * {@link resolveMerchantAuth} parses it **once**, at construction: a PEM
   * that `createPrivateKey` cannot read is a `VpayConfigError` the merchant
   * sees at startup rather than a `crypto` exception thrown out of the first
   * token exchange, and every later assertion signs against the same already
   * parsed key instead of re-parsing the same text per mint.
   */
  privateKey: KeyObject;
  kid: string | undefined;
  /** Where the token request is POSTed — reachable from *this* process. */
  tokenEndpoint: string;
  /**
   * The client assertion's `aud` claim — what the OP calls itself. Resolved
   * by {@link resolveMerchantAuth}; defaults to {@link tokenEndpoint}, which
   * is right only when this process reaches vpay at the same URL vpay
   * publishes as its own.
   */
  assertionAudience: string;
  /** The OAuth2 `audience` **request parameter** (`vpay:v1`). Not the `aud` claim. */
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
      // `aud` is the **OP's own token endpoint (or issuer)**, never the
      // `audience` request parameter below (`vpay:v1`). RFC 7523 §3 and the
      // OP's own `authenticate_client` both compare it against the endpoint
      // as the OP names itself — `{deployment.public_base_url}/v1/oauth/token`
      // or that issuer — which is not necessarily the URL this process
      // POSTed to. The two are different values with different jobs
      // (docs/flows/merchant-auth.md, "The client assertion").
      audience: this.#options.assertionAudience,
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

/**
 * The subset of an entry point's constructor options that decides how this
 * package authenticates: everything the {@link TokenManager} needs, plus the
 * `baseUrl` the defaults are derived from.
 *
 * Shared by {@link VpayClient} and `createStripeAuthenticator` on purpose.
 * The two entry points mint the *same* assertion against the *same* endpoint
 * with the *same* defaults; resolving those defaults in one place is what
 * stops them from drifting apart as one of them is edited.
 *
 * Every optional property is `?: T | undefined` for the
 * `exactOptionalPropertyTypes` reason spelled out on `VpayClientOptions`.
 */
export interface MerchantAuthOptions {
  /** vpay's base URL, e.g. `https://api.vpay.example`. */
  baseUrl: string;
  /** This merchant's registered OAuth2 `client_id`. */
  clientId: string;
  /** The merchant's RSA private key — PEM text or a `crypto.KeyObject`. */
  privateKey: string | KeyObject;
  /** Required only if the merchant registered more than one JWK. */
  kid?: string | undefined;
  /** Default `${baseUrl}/v1/oauth`. */
  issuer?: string | undefined;
  /**
   * The URL this package POSTs the token request to. Default
   * `${issuer}/token`.
   *
   * This is a *reachability* setting: it must resolve from wherever your
   * server runs — a compose service name, a private DNS name, a mesh
   * address. It is **not** necessarily what the assertion is addressed to;
   * see {@link assertionAudience}.
   */
  tokenEndpoint?: string | undefined;
  /**
   * The OP's own token endpoint (or issuer) as vpay is configured publicly —
   * `{deployment.public_base_url}/v1/oauth/token`, or
   * `{deployment.public_base_url}/v1/oauth`. This is what the client
   * assertion's `aud` claim is signed as. **Set it when your server reaches
   * vpay by a different URL than payers do.**
   *
   * Default: {@link tokenEndpoint}, which is correct only when the two are
   * the same string. When they are not — a merchant server calling
   * `http://vpay-server:8080` inside a compose network, or a private name in
   * production — the OP's `authenticate_client` compares the `aud` claim
   * against its own `{token endpoint, issuer}` pair, neither of which is the
   * internal URL, and every token request answers `invalid_client` /
   * `InvalidAudience`. Nothing else in the handshake is wrong, which is why
   * the symptom carries no hint at the cause.
   */
  assertionAudience?: string | undefined;
  /**
   * The OAuth2 `audience` **request parameter** — a form field naming the
   * resource server (`vpay:v1`), not the assertion's `aud` claim. Default
   * {@link DEFAULT_AUDIENCE}.
   */
  audience?: string | undefined;
  /** Omitted from the token request unless set. */
  scope?: string | undefined;
  /** Default {@link DEFAULT_ASSERTION_LIFETIME_SECONDS}. Must be an integer in `1..=300`. */
  assertionLifetimeSeconds?: number | undefined;
  /** Default {@link DEFAULT_TIMEOUT_MS}. */
  timeoutMs?: number | undefined;
  /** Injection point for tests and proxies. Defaults to the global `fetch`. */
  fetch?: typeof fetch | undefined;
}

/** What {@link resolveMerchantAuth} hands back: a ready {@link TokenManager} and the settings it was built from. */
export interface ResolvedMerchantAuth {
  /** `baseUrl` with any trailing slash removed. */
  baseUrl: string;
  clientId: string;
  timeoutMs: number;
  fetchImpl: typeof fetch;
  userAgent: string;
  tokenManager: TokenManager;
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
 * Turns a configured signing key into a {@link KeyObject}, once.
 *
 * Two things this buys, and both are the reason it is here rather than in
 * {@link mintClientAssertion}'s per-call path:
 *
 * - **A bad key is a startup failure.** `createPrivateKey('')`, or a PEM with
 *   a mangled header, throws an OpenSSL `Error` out of `crypto`. Left to the
 *   first mint, that surfaced as an unrecognisable exception from the middle
 *   of a token exchange — and, through `@vaam-apps/vpay-sdk/stripe`, as stripe-node's
 *   detached `unhandledRejection` (see `stripe-auth.ts`). Parsed here it is a
 *   {@link VpayConfigError} raised by the constructor the merchant wrote.
 * - **One parse, not one per request.** RSA PEM decoding is not free, and
 *   nothing about it changes between mints.
 *
 * The message never quotes the key: the input is the merchant's private key,
 * and a `VpayConfigError` is exactly the sort of thing that reaches a log.
 * `cause` carries OpenSSL's own reason, which names the decoder that failed
 * and no key material.
 */
function parsePrivateKey(privateKey: string | KeyObject): KeyObject {
  if (typeof privateKey !== "string") {
    return privateKey;
  }
  try {
    return createPrivateKey(privateKey);
  } catch (err) {
    throw new VpayConfigError(
      "privateKey could not be read as a private key: expected PEM text (or a crypto.KeyObject)",
      { cause: err },
    );
  }
}

/**
 * Validates {@link MerchantAuthOptions}, applies this package's defaults, and
 * builds the {@link TokenManager} both entry points share.
 *
 * Every failure it can detect is detected here — at construction — rather
 * than on the first request: a merchant with an out-of-range
 * `assertionLifetimeSeconds` finds out at startup, not at 3am under load.
 */
export function resolveMerchantAuth(
  options: MerchantAuthOptions,
): ResolvedMerchantAuth {
  const baseUrl = stripTrailingSlash(
    requireNonEmptyString(options.baseUrl, "baseUrl"),
  );
  const clientId = requireNonEmptyString(options.clientId, "clientId");
  if (options.privateKey === undefined || options.privateKey === null) {
    throw new VpayConfigError("privateKey is required");
  }
  const privateKey = parsePrivateKey(options.privateKey);

  const assertionLifetimeSeconds =
    options.assertionLifetimeSeconds ?? DEFAULT_ASSERTION_LIFETIME_SECONDS;
  validateAssertionLifetimeSeconds(assertionLifetimeSeconds);

  const issuer = options.issuer ?? `${baseUrl}/v1/oauth`;
  const tokenEndpoint = options.tokenEndpoint ?? `${issuer}/token`;
  // Defaults to the URL we POST to — unchanged behaviour for every merchant
  // whose server reaches vpay at the same URL vpay publishes as its own.
  // A merchant reaching vpay by an internal name sets it explicitly.
  const assertionAudience = options.assertionAudience ?? tokenEndpoint;
  const audience = options.audience ?? DEFAULT_AUDIENCE;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const fetchImpl = options.fetch ?? fetch;
  const userAgent = `vpay-sdk-node/${SDK_VERSION}`;

  return {
    baseUrl,
    clientId,
    timeoutMs,
    fetchImpl,
    userAgent,
    tokenManager: new TokenManager({
      clientId,
      privateKey,
      kid: options.kid,
      tokenEndpoint,
      assertionAudience,
      audience,
      scope: options.scope,
      assertionLifetimeSeconds,
      timeoutMs,
      fetchImpl,
      userAgent,
    }),
  };
}
