/**
 * The shop's environment, read once and validated loudly.
 *
 * Every value here is server-side. Nothing in this file may be imported from
 * a `'use client'` module: `VPAY_PRIVATE_KEY_FILE` names the merchant's
 * signing key and `VPAY_WEBHOOK_SECRET` is an HMAC secret, and a client
 * import would put the whole record into the browser bundle. The two values
 * the browser legitimately needs (the publishable key and the checkout
 * origin) are passed to client components as props by the server components
 * that render them — see `src/app/orders/[id]/embedded/page.tsx`.
 */

/** A rail code `/v1` accepts in `payment_method_types`. */
export type PaymentMethodType = "mtn_momo" | "orange_money";

const KNOWN_RAILS: readonly PaymentMethodType[] = ["mtn_momo", "orange_money"];

export interface ShopConfig {
  /** vpay's API origin as **this server** reaches it, e.g. `http://vpay-server:8080`. */
  vpayApiUrl: string;
  /**
   * vpay's API origin as a **browser** reaches it, e.g.
   * `http://localhost:8080`. Defaults to {@link vpayApiUrl}, which is right
   * only when the two are the same host — inside compose they are not, and
   * `VPAY_BROWSER_API_URL` is what tells them apart.
   *
   * `initEmbeddedCheckout` does not currently fetch from it (it builds the
   * iframe `src` from `VPAY_CHECKOUT_URL` and listens for messages), so a
   * wrong value is silent today. It would stop being silent the moment this
   * shop called `retrieveCheckoutSession`, which is why it is a named value
   * rather than a copy of the server one.
   */
  vpayBrowserApiUrl: string;
  /**
   * The `aud` this shop signs its `private_key_jwt` client assertion with —
   * vpay's OP as **vpay** names itself, `{deployment.public_base_url}/v1/oauth/token`
   * (or that `/v1/oauth` issuer). Optional; `undefined` leaves the SDK's
   * default, which is the URL the token request is POSTed to.
   *
   * Two different facts, and the reason this is a variable of its own.
   * {@link vpayApiUrl} has to be reachable from *this container*; the `aud`
   * claim has to be a string vpay's `authenticate_client` recognises as its
   * own name, and it compares against nothing else. Inside compose those
   * differ — `http://vpay-server:8080` versus the published
   * `http://localhost:8080` — and with this unset every token request is an
   * `invalid_client` whose response says nothing about audiences.
   */
  vpayOauthAudience: string | undefined;
  /** The shop's merchant `client_id`, as registered in vpay's config. */
  vpayClientId: string;
  /** Path to the PEM holding the merchant's RSA private key. */
  vpayPrivateKeyFile: string;
  /** `pk_…`. Public by name; rendered into the embedded page. */
  vpayPublishableKey: string;
  /** The checkout app's origin — vpay's `checkout.public_base_url`. */
  vpayCheckoutUrl: string;
  /** The signing secret of this shop's webhook endpoint in vpay's config. */
  vpayWebhookSecret: string;
  /** The shop's own externally reachable origin, used to build return URLs. */
  shopPublicUrl: string;
  /**
   * Which rails the shop offers. Configuration, not a code path (ADR-0003):
   * the demo stack's `mtn_momo` profile settles in EUR while this catalogue
   * is priced in XAF, so a deployment that wants only the XAF rail sets this
   * rather than editing anything. See `docs/plans/step9-notes/lane-7.md`.
   */
  paymentMethodTypes: PaymentMethodType[];
}

/**
 * What this module reads an environment as.
 *
 * Not `NodeJS.ProcessEnv`: Next's own type declaration makes `NODE_ENV`
 * **required** on that interface, so a test handing over a literal record of
 * the shop's variables would not compile — measured, in `next build`'s type
 * check, which is stricter here than a bare `tsc --noEmit`.
 */
export type EnvRecord = Record<string, string | undefined>;

class MissingEnvError extends Error {
  constructor(name: string) {
    super(
      `examples/shop: ${name} is not set. Copy .env.example and fill it in, ` +
        `or see docs/plans/step9-notes/lane-7.md for the demo stack's values.`,
    );
    this.name = "MissingEnvError";
  }
}

function required(env: EnvRecord, name: string): string {
  const value = env[name];
  if (value === undefined || value.trim() === "") {
    throw new MissingEnvError(name);
  }
  return value.trim();
}

/** Strips one trailing slash so `${origin}/orders/…` never doubles it. */
function origin(value: string, name: string): string {
  const trimmed = value.replace(/\/+$/, "");
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error(
      `examples/shop: ${name} must be an absolute URL, got ${trimmed}`,
    );
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error(
      `examples/shop: ${name} must be http(s), got ${parsed.protocol}`,
    );
  }
  return trimmed;
}

function parseRails(raw: string | undefined): PaymentMethodType[] {
  const list = (raw ?? "mtn_momo,orange_money")
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  const unknown = list.filter(
    (code) => !KNOWN_RAILS.includes(code as PaymentMethodType),
  );
  if (unknown.length > 0) {
    throw new Error(
      `examples/shop: SHOP_PAYMENT_METHOD_TYPES names rails this shop does not know: ` +
        `${unknown.join(", ")}. Known: ${KNOWN_RAILS.join(", ")}.`,
    );
  }
  if (list.length === 0) {
    throw new Error("examples/shop: SHOP_PAYMENT_METHOD_TYPES is empty");
  }
  return list as PaymentMethodType[];
}

/**
 * Reads and validates the environment. Exported taking `env` so a test can
 * hand it a record rather than mutating `process.env`.
 */
export function loadShopConfig(env: EnvRecord = process.env): ShopConfig {
  return {
    vpayApiUrl: origin(required(env, "VPAY_API_URL"), "VPAY_API_URL"),
    vpayBrowserApiUrl: origin(
      env["VPAY_BROWSER_API_URL"]?.trim() || required(env, "VPAY_API_URL"),
      "VPAY_BROWSER_API_URL",
    ),
    // Optional: unset means "the same URL we POST to", which is right
    // whenever this server reaches vpay the way payers do. `origin()` is not
    // applied — this is a full endpoint URL with a path, not an origin, and
    // it must match vpay's own string byte for byte.
    vpayOauthAudience: env["VPAY_OAUTH_AUDIENCE"]?.trim() || undefined,
    vpayClientId: required(env, "VPAY_CLIENT_ID"),
    vpayPrivateKeyFile: required(env, "VPAY_PRIVATE_KEY_FILE"),
    vpayPublishableKey: required(env, "VPAY_PUBLISHABLE_KEY"),
    vpayCheckoutUrl: origin(
      required(env, "VPAY_CHECKOUT_URL"),
      "VPAY_CHECKOUT_URL",
    ),
    vpayWebhookSecret: required(env, "VPAY_WEBHOOK_SECRET"),
    shopPublicUrl: origin(required(env, "SHOP_PUBLIC_URL"), "SHOP_PUBLIC_URL"),
    paymentMethodTypes: parseRails(env["SHOP_PAYMENT_METHOD_TYPES"]),
  };
}

let cached: ShopConfig | undefined;

/** The process-wide config, parsed on first use. */
export function shopConfig(): ShopConfig {
  cached ??= loadShopConfig();
  return cached;
}
