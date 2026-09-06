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

/** The three surfaces a merchant can put vpay's Checkout on. */
export type CheckoutMode = "hosted" | "embedded" | "popup";

const CHECKOUT_MODES: readonly CheckoutMode[] = ["hosted", "embedded", "popup"];

/**
 * Which rails the shop offers, as configuration.
 *
 * Two shapes, because a deployment can legitimately want either. A plain
 * list — `mtn_momo,orange_money` — offers the same rails whatever the order
 * is denominated in. A per-currency map — `xaf:orange_money;eur:mtn_momo` —
 * offers different ones per currency, which is what the demo stack needs and
 * why this shape exists at all: `config/application.yml` gives `mtn_momo` a
 * **EUR** profile (MTN's sandbox refuses XAF) and `orange_money` an XAF one,
 * and `POST /v1/payment_intents/{id}/confirm` refuses a rail whose profile
 * currency is not the intent's — a `400` on `payment_method_data[type]`,
 * `vpay_api::v1::payment_intents::currencies_agree`.
 *
 * Offering a payer a rail that will refuse them at the last step is a
 * failure the shop can prevent without knowing anything about rails: it is
 * one line of its own configuration, not a code path (ADR-0003), and not a
 * table of rail currencies this shop would then have to keep in step with
 * vpay's.
 */
export type RailSelection =
  | { kind: "all"; rails: PaymentMethodType[] }
  | { kind: "by_currency"; byCurrency: Record<string, PaymentMethodType[]> };

/**
 * The rails to offer on an order in `currency`.
 *
 * An empty answer is a real configuration outcome, not an error here: the
 * caller — `placeOrder` — is the one that knows an order is being refused
 * and can say so with the currency in the message.
 */
export function railsForCurrency(
  selection: RailSelection,
  currency: string,
): PaymentMethodType[] {
  if (selection.kind === "all") {
    return [...selection.rails];
  }
  const code = currency.toLowerCase();
  // `Object.hasOwn`, not a bare index: `currency` reaches here from a
  // catalogue row, and `byCurrency["constructor"]` is a function.
  return Object.hasOwn(selection.byCurrency, code)
    ? [...(selection.byCurrency[code] ?? [])]
    : [];
}

/** Every rail named anywhere in a selection, for the test-numbers panel. */
export function allSelectedRails(
  selection: RailSelection,
): PaymentMethodType[] {
  if (selection.kind === "all") {
    return [...selection.rails];
  }
  const seen = new Set<PaymentMethodType>();
  for (const rails of Object.values(selection.byCurrency)) {
    for (const rail of rails) {
      seen.add(rail);
    }
  }
  return [...seen];
}

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
   * Which rails the shop offers, per currency or across the board. See
   * {@link RailSelection}; `SHOP_PAYMENT_METHOD_TYPES` carries either shape.
   */
  rails: RailSelection;
  /**
   * Which surface the shop puts vpay's Checkout on by default —
   * `SHOP_CHECKOUT_MODE`. The checkout page offers all three regardless, so
   * a reader of the demo can see each one; this is the one the page opens
   * on, and it is what a real merchant would set and leave alone.
   */
  checkoutMode: CheckoutMode;
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

function parseRailList(raw: string, where: string): PaymentMethodType[] {
  const list = raw
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  const unknown = list.filter(
    (code) => !KNOWN_RAILS.includes(code as PaymentMethodType),
  );
  if (unknown.length > 0) {
    throw new Error(
      `examples/shop: SHOP_PAYMENT_METHOD_TYPES names rails this shop does ` +
        `not know${where}: ${unknown.join(", ")}. ` +
        `Known: ${KNOWN_RAILS.join(", ")}.`,
    );
  }
  if (list.length === 0) {
    throw new Error(
      `examples/shop: SHOP_PAYMENT_METHOD_TYPES is empty${where}`,
    );
  }
  return list as PaymentMethodType[];
}

/**
 * Reads `SHOP_PAYMENT_METHOD_TYPES` in either of its two shapes.
 *
 * A `:` anywhere in the value selects the per-currency shape, because a rail
 * code never contains one. Groups are separated by `;` so that the rail list
 * inside a group can keep using `,` — `xaf:orange_money;eur:mtn_momo`.
 */
function parseRailSelection(raw: string | undefined): RailSelection {
  const value = (raw ?? "mtn_momo,orange_money").trim();
  if (value.length === 0) {
    throw new Error("examples/shop: SHOP_PAYMENT_METHOD_TYPES is empty");
  }
  if (!value.includes(":")) {
    return { kind: "all", rails: parseRailList(value, "") };
  }
  const byCurrency: Record<string, PaymentMethodType[]> = {};
  for (const group of value.split(";")) {
    const trimmed = group.trim();
    if (trimmed.length === 0) {
      continue;
    }
    const separator = trimmed.indexOf(":");
    const currency = trimmed.slice(0, separator).trim().toLowerCase();
    if (separator === -1 || currency.length === 0) {
      throw new Error(
        `examples/shop: SHOP_PAYMENT_METHOD_TYPES group "${trimmed}" is not ` +
          `"<currency>:<rail>[,<rail>]"`,
      );
    }
    if (Object.hasOwn(byCurrency, currency)) {
      // Two groups for one currency would make the answer depend on which
      // one was written last, which is exactly the kind of thing that is
      // true until someone appends a group.
      throw new Error(
        `examples/shop: SHOP_PAYMENT_METHOD_TYPES names ${currency} twice`,
      );
    }
    byCurrency[currency] = parseRailList(
      trimmed.slice(separator + 1),
      ` for ${currency}`,
    );
  }
  if (Object.keys(byCurrency).length === 0) {
    throw new Error("examples/shop: SHOP_PAYMENT_METHOD_TYPES is empty");
  }
  return { kind: "by_currency", byCurrency };
}

function parseCheckoutMode(raw: string | undefined): CheckoutMode {
  const value = (raw ?? "hosted").trim().toLowerCase();
  if (!CHECKOUT_MODES.includes(value as CheckoutMode)) {
    throw new Error(
      `examples/shop: SHOP_CHECKOUT_MODE must be one of ` +
        `${CHECKOUT_MODES.join(", ")}, got ${value}`,
    );
  }
  return value as CheckoutMode;
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
    rails: parseRailSelection(env["SHOP_PAYMENT_METHOD_TYPES"]),
    checkoutMode: parseCheckoutMode(env["SHOP_CHECKOUT_MODE"]),
  };
}

let cached: ShopConfig | undefined;

/** The process-wide config, parsed on first use. */
export function shopConfig(): ShopConfig {
  cached ??= loadShopConfig();
  return cached;
}
