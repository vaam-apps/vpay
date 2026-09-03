/**
 * `@vpay/sdk/stripe` — a `config.authenticator` for the **official Stripe
 * Node SDK**, so `stripe-node` can talk to vpay's `/v1` API.
 *
 * vpay never accepts an API key ([ADR-0010](../../../docs/adr/0010-merchant-auth-private-key-jwt.md)):
 * every `/v1` call carries a short-lived bearer token minted from a
 * `private_key_jwt` client assertion (docs/flows/merchant-auth.md). stripe-node
 * has no notion of that handshake, but it does accept an arbitrary async
 * `authenticator` invoked once per request attempt with the whole outbound
 * request — which is exactly the seam this module fills:
 *
 * ```js
 * import Stripe from "stripe";
 * import { createStripeAuthenticator } from "@vpay/sdk/stripe";
 *
 * const stripe = new Stripe("", {
 *   authenticator: createStripeAuthenticator({
 *     baseUrl: "http://localhost:8080",
 *     clientId: "acme-cameroon",
 *     privateKey: readFileSync("./merchant-key.pem", "utf8"),
 *   }),
 *   host: "localhost", port: "8080", protocol: "http",
 * });
 * ```
 *
 * The `host`/`port`/`protocol` in that snippet are **not optional garnish** —
 * see {@link createStripeAuthenticator}'s "Where the token may be sent".
 *
 * This module deliberately imports **nothing** from `stripe`. `stripe` is an
 * optional peer dependency, and the request type below is written out
 * structurally rather than imported, so `@vpay/sdk` type-checks and builds
 * with `stripe` absent. `src/stripe-auth.test.ts` pins the structural type
 * against the real `Stripe.StripeConfig["authenticator"]`, so a divergence in
 * a future `stripe` release fails the build rather than the merchant's
 * deployment.
 *
 * See the README's "Using the official Stripe SDK" section for what does and
 * does not carry over from real Stripe.
 */
import { resolveMerchantAuth, type MerchantAuthOptions } from "./auth.js";
import { VpayConfigError } from "./errors.js";

/**
 * Configuration for {@link createStripeAuthenticator}.
 *
 * **The same options `VpayClient` takes** ({@link MerchantAuthOptions}, which
 * `VpayClientOptions` also extends): both entry points run the same handshake
 * through the same {@link resolveMerchantAuth}, so a knob one of them offers
 * and the other does not would be a bug rather than a feature. Only the
 * *meaning* of `timeoutMs` narrows, and it is redeclared below to say so —
 * `stripe-node` owns the resource requests, their timeout and their retries,
 * so this module configures the **token exchange** alone.
 *
 * Every optional property is declared `?: T | undefined` rather than `?: T`
 * so a consumer compiling with `exactOptionalPropertyTypes` can pass a value
 * read from configuration (`string | undefined`) without a conditional
 * spread — the same rule the core entry point follows.
 */
export interface StripeAuthenticatorOptions extends MerchantAuthOptions {
  /**
   * Default 30000. Applies to the **token exchange only** — stripe-node's own
   * `timeout` and `httpClient` govern every resource call.
   */
  timeoutMs?: number | undefined;
}

/**
 * `stripe`'s `StripeRequest` (`Types.d.ts`: `{host, port, path, method,
 * headers, body, protocol}`), written out structurally so this module needs
 * no `stripe` import.
 */
export interface StripeRequestShape {
  host: string;
  port: string;
  path: string;
  method: string;
  headers: Record<string, string | number | string[]>;
  body: string;
  protocol: string;
}

/**
 * What an authenticator is actually called with.
 *
 * Wider than {@link StripeRequestShape} on purpose, in the one direction that
 * is safe: `headers` is required (it is the only field this module writes),
 * everything else is optional. Two reasons, and neither is convenience:
 *
 * - the README's startup probe is `await authenticator({ headers: {} })` —
 *   mint a token at boot so a wrong key fails deployment rather than the
 *   first payment — and that call has no host, path or body to give;
 * - a parameter type *wider* than `StripeRequest` keeps the returned function
 *   assignable to `Stripe.StripeConfig["authenticator"]` (parameters are
 *   contravariant), which `stripe-auth.test.ts` pins.
 *
 * A narrower parameter would have made the probe a type error and the
 * assignment unsound.
 */
export type StripeAuthenticatorRequest = Pick<StripeRequestShape, "headers"> &
  Partial<Omit<StripeRequestShape, "headers">>;

/**
 * What {@link createStripeAuthenticator} returns: a function assignable to
 * stripe-node's `StripeConfig["authenticator"]`, carrying one extra method.
 */
export interface VpayStripeAuthenticator {
  (request: StripeAuthenticatorRequest): Promise<void>;

  /**
   * Discards the cached access token so the next request mints a fresh one.
   *
   * Needed because the authenticator **cannot see the response**: it runs
   * before the request and its promise settles before any status code
   * exists, so a `401` cannot trigger a re-auth from in here the way
   * `VpayClient` does it. Normal expiry is handled by the cache's own safety
   * margin; this is the escape hatch for the rest:
   *
   * ```js
   * try { await stripe.paymentIntents.retrieve(id); }
   * catch (err) {
   *   if (err instanceof Stripe.errors.StripeAuthenticationError) {
   *     authenticator.invalidate();
   *   }
   *   throw err;
   * }
   * ```
   */
  invalidate(): void;
}

/** The origin an authenticator is bound to, normalised for comparison. */
interface BoundOrigin {
  /** Lower-cased, without the brackets an IPv6 literal wears in a URL. */
  host: string;
  /** Always explicit: the URL's port, or the scheme's default. */
  port: string;
  /** Lower-cased, without the trailing colon `URL.protocol` carries. */
  protocol: string;
}

function defaultPortFor(protocol: string): string {
  return protocol === "http" ? "80" : "443";
}

/**
 * `api.vpay.example`, `API.Vpay.Example.` and `[::1]` all have to compare
 * equal to the same configured host, because a merchant writes `baseUrl` and
 * stripe-node's `host` by hand and nothing makes them agree on case or on
 * bracket notation. Everything else about the value is left alone: this is a
 * comparison, not a validator.
 */
function normaliseHost(host: string): string {
  const lowered = host.trim().toLowerCase();
  const unbracketed =
    lowered.startsWith("[") && lowered.endsWith("]")
      ? lowered.slice(1, -1)
      : lowered;
  return unbracketed.endsWith(".") ? unbracketed.slice(0, -1) : unbracketed;
}

function normaliseProtocol(protocol: string): string {
  const lowered = protocol.trim().toLowerCase();
  return lowered.endsWith(":") ? lowered.slice(0, -1) : lowered;
}

function parseBoundOrigin(baseUrl: string): BoundOrigin {
  let url: URL;
  try {
    url = new URL(baseUrl);
  } catch (err) {
    throw new VpayConfigError(
      `baseUrl is not an absolute URL: ${baseUrl}`,
      { cause: err },
    );
  }
  const protocol = normaliseProtocol(url.protocol);
  return {
    host: normaliseHost(url.hostname),
    port: url.port === "" ? defaultPortFor(protocol) : url.port,
    protocol,
  };
}

function describeOrigin(origin: BoundOrigin): string {
  return `${origin.protocol}://${origin.host}:${origin.port}`;
}

/**
 * Refuses to mint a token for a request bound somewhere other than
 * {@link BoundOrigin}.
 *
 * See {@link createStripeAuthenticator}'s "Where the token may be sent" for
 * why this is a refusal and not a warning.
 *
 * A request with **no `host`** is allowed: that is the README's startup probe
 * (`authenticator({ headers: {} })`), which has no destination to check
 * against and never reaches a socket. Every field stripe-node does supply is
 * checked; a `protocol` it somehow omitted is read as the configured one
 * rather than as a mismatch, because a missing field is not evidence of a
 * different destination and this check exists to catch a *different* one.
 */
function assertRequestIsBound(
  request: StripeAuthenticatorRequest,
  bound: BoundOrigin,
): void {
  if (request.host === undefined) {
    return;
  }
  const protocol =
    request.protocol === undefined
      ? bound.protocol
      : normaliseProtocol(request.protocol);
  const host = normaliseHost(request.host);
  const port =
    request.port === undefined || request.port === ""
      ? defaultPortFor(protocol)
      : String(request.port);

  if (
    host === bound.host &&
    port === bound.port &&
    protocol === bound.protocol
  ) {
    return;
  }
  throw new VpayConfigError(
    `refusing to authenticate a request to ${describeOrigin({ host, port, protocol })}: ` +
      `this authenticator mints tokens for ${describeOrigin(bound)} (its baseUrl) and will not ` +
      `send one anywhere else. Set stripe-node's host, port and protocol to match baseUrl — ` +
      `with them omitted stripe-node addresses api.stripe.com:443, and your vpay access token ` +
      `would be sent to Stripe.`,
  );
}

/**
 * Builds a stripe-node `authenticator` that performs vpay's
 * `client_credentials` + `private_key_jwt` handshake and puts the resulting
 * bearer token on the outbound request.
 *
 * Token caching, single-flight refresh under concurrency, and the
 * `expires_in` safety margin are the *same* `TokenManager` `VpayClient` uses
 * — not a second implementation of the handshake.
 *
 * # Where the token may be sent
 *
 * **The authenticator is bound to `baseUrl`'s origin and refuses every other
 * one.** stripe-node calls it with the whole outbound request, for *any*
 * request the client makes; `host`/`port`/`protocol` are configured on the
 * `Stripe` instance, separately from this module, and when they are omitted
 * stripe-node addresses `api.stripe.com:443`. An authenticator that wrote
 * `Authorization` unconditionally would therefore hand a merchant's vpay
 * bearer token to Stripe the moment someone forgot one line of config — a
 * live credential for someone else's API, sent over TLS to a party that has
 * no reason to want it and every reason to log it.
 *
 * So a request whose host, effective port or protocol is not `baseUrl`'s is
 * a {@link VpayConfigError} naming both origins, thrown **before** a token is
 * minted (a refused request costs no assertion and spends no `jti`). A
 * request with no `host` at all — the README's startup probe — is allowed.
 *
 * # What it writes
 *
 * The returned function writes exactly one thing: `headers.Authorization`.
 * Mutating anything else is possible (stripe-node re-reads
 * `host`/`port`/`path`/`method`/`headers`/`body`/`protocol` after the
 * authenticator resolves) but must not be done: `Content-Length` is computed
 * from the body *before* this runs, so rewriting the body desynchronises the
 * two and the request is truncated or hangs.
 *
 * A failed token exchange rejects, and that rejection is the caller's to
 * handle when the authenticator is called directly.
 *
 * **Through stripe-node it is not.** Measured against `stripe@22.6.1`: it
 * builds the right error (`StripeError: Unable to authenticate the request`,
 * with the `VpayAuthError` at `err.raw.exception`) but throws it inside a
 * detached promise chain that never calls its own callback — so the error
 * arrives as a process-level `unhandledRejection` and the promise the
 * merchant awaited never settles. See the README's "A stripe-node defect you
 * will hit if your key is wrong", and the test that pins it. It is also why
 * both of this function's construction-time checks are construction-time:
 * a `VpayConfigError` a merchant sees when they build the client is worth
 * more than the same fact delivered as an unhandled rejection at 3am.
 *
 * @throws `VpayConfigError` at construction if `baseUrl`, `clientId` or
 * `privateKey` is missing, if `baseUrl` is not an absolute URL, if
 * `privateKey` is not a readable private key, or if
 * `assertionLifetimeSeconds` is out of range.
 */
export function createStripeAuthenticator(
  options: StripeAuthenticatorOptions,
): VpayStripeAuthenticator {
  // Validation, defaulting, the one-time private-key parse and the
  // `TokenManager` itself all live in `resolveMerchantAuth`, shared with
  // `VpayClient`: both entry points must mint the same assertion against the
  // same endpoint with the same defaults, and one copy is how that stays
  // true.
  const { baseUrl, tokenManager } = resolveMerchantAuth(options);
  // Parsed once, here, rather than per request: the answer cannot change
  // between calls, and a `baseUrl` that is not a URL is a configuration
  // mistake the merchant should be told about at construction.
  const bound = parseBoundOrigin(baseUrl);

  const authenticator: VpayStripeAuthenticator = Object.assign(
    async (request: StripeAuthenticatorRequest): Promise<void> => {
      assertRequestIsBound(request, bound);
      const token = await tokenManager.getToken();
      request.headers["Authorization"] = `Bearer ${token}`;
    },
    {
      invalidate: (): void => tokenManager.invalidate(),
    },
  );

  return authenticator;
}
