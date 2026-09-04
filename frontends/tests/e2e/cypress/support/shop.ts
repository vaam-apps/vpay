/**
 * What both shop specs need: where the stack is, and how to read an order.
 *
 * A module rather than two copies, and it is also what makes the two spec
 * files *modules* — a Cypress spec with no `import`/`export` is a script in
 * the global scope, and two of them declaring the same `const` is a
 * `tsc --noEmit` error the runner would never have told you about.
 *
 * Nothing here may be called inside a `cy.origin()` callback: that callback
 * runs in another Cypress instance with its own module graph. Pass values in
 * through `args` instead.
 */

/** The demo shop, as a browser reaches it. */
export const shopUrl = (): string => Cypress.expose("SHOP_URL") as string;
/** vpay's own checkout page (`checkout.public_base_url`). */
export const checkoutOrigin = (): string =>
  Cypress.expose("CHECKOUT_URL") as string;
/** The Orange stub — the RAIL's hosted page, not a vpay page (D7). */
export const orangeOrigin = (): string =>
  Cypress.expose("ORANGE_STUB_URL") as string;
/** The fixture page's origin, which is in nobody's `checkout_origins`. */
export const frameFixtureUrl = (): string =>
  Cypress.expose("FRAME_FIXTURE_URL") as string;
/** `shop-merchant`'s publishable key. Public by name; authorises nothing. */
export const shopPublishableKey = (): string =>
  Cypress.expose("SHOP_PUBLISHABLE_KEY") as string;

/**
 * The DIGITS-ONLY steering MSISDNs, added in Step 9 lane 2b
 * (`backends/tests/conformance/wiremock/mtn/mappings/`).
 *
 * vpay's page validates Cameroon E.164 and refuses the older hex-suffixed
 * twins (`237600000ce0`, `…f01`, `…f02`) — correctly: a form that accepted
 * letters as a phone number would accept them for every payer. See
 * `docs/plans/step9-notes/lane-3.md` §4c.
 */
export const MTN = {
  /** `PENDING` on the first status query, `SUCCESSFUL` on the next. */
  succeeds: "237600000100",
  /** Arms `FAILED / NOT_ENOUGH_FUNDS` on the next status query. */
  insufficientFunds: "237600000101",
  /** Arms `FAILED / COULD_NOT_PERFORM_TRANSACTION` — the payer let it expire. */
  payerTimeout: "237600000102",
} as const;

/** Products from the seeded catalogue (`prisma/migrations/…_seed_catalogue`). */
export const PRODUCT = {
  tote: "njangi-tote",
  coffee: "mbanga-coffee-1kg",
} as const;

/** The order id out of `/orders/{id}/…`, whichever shop page we landed on. */
export function orderIdFromUrl(): Cypress.Chainable<string> {
  return cy.url().then((url) => {
    const match = /\/orders\/([^/?#]+)\//.exec(url);
    expect(match, `an order id in ${url}`).to.not.equal(null);
    return (match as RegExpExecArray)[1] as string;
  });
}

/** What `orders.get` answers with — the shop's own database, and nothing else. */
export interface ShopOrder {
  status: "unpaid" | "paid" | "failed" | "cancelled" | string;
  paymentIntentId: string | null;
  checkoutSessionId: string | null;
}

/**
 * The shop's own `orders.get`, over HTTP, exactly as its return page polls
 * it — one tRPC query, no batching, no transformer.
 *
 * This is the only authority any of these specs consults about whether money
 * moved. It reads the shop's database, which is written by the shop's webhook
 * handler after it has verified vpay's signature, and by nothing else.
 */
export function readOrder(orderId: string): Cypress.Chainable<ShopOrder> {
  const input = encodeURIComponent(JSON.stringify({ id: orderId }));
  return cy
    .request(`${shopUrl()}/api/trpc/orders.get?input=${input}`)
    .then((response) => {
      expect(response.status).to.equal(200);
      return (response.body as { result: { data: ShopOrder } }).result.data;
    });
}

/**
 * Polls {@link readOrder} until the order reaches `expected`, failing loudly
 * if it ever becomes `paid` when that is not what was asked for.
 *
 * Used only where the page itself cannot be the assertion — the shop's
 * cancelled page is server-rendered once and does not poll.
 */
export function waitForOrderStatus(
  orderId: string,
  expected: ShopOrder["status"],
  timeoutMs = 120_000,
): void {
  const deadline = Date.now() + timeoutMs;
  const poll = (): void => {
    readOrder(orderId).then((order) => {
      if (expected !== "paid") {
        expect(order.status, "the shop's own orders.get").to.not.equal("paid");
      }
      if (order.status === expected) {
        return;
      }
      if (Date.now() > deadline) {
        throw new Error(
          `order ${orderId} stayed '${order.status}' for ${timeoutMs}ms; ` +
            `expected '${expected}'`,
        );
      }
      cy.wait(2000);
      poll();
    });
  };
  poll();
}
