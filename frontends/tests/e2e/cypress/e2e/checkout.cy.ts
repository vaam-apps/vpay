/**
 * `examples/checkout-browser` end to end against the real compose stack.
 *
 * Unlike `dashboard.cy.ts`, this spec's page under test is NOT the dashboard
 * container — it is `examples/checkout-browser`, served on its own origin by
 * `cypress.config.ts`'s `before:run` hook (which spawns
 * `examples/checkout-browser/serve.mjs` — see that file and
 * `cypress/tasks/checkoutBrowserServer.ts` for why a child process rather
 * than an import). The PaymentIntent it confirms is minted server-side, in
 * Node, by `cy.task('mintCheckoutPaymentIntent')`
 * (`cypress/tasks/checkoutTasks.ts`) using the `shop-merchant` OAuth keypair
 * `just gen-demo-keys` writes to `.e2e/` — the same credential
 * `examples/shop` pays with, and the tenant the demo dashboard is bound to
 * (`demo_dashboard_merchant`), never present in this spec's browser. It was
 * `demo-merchant` until 2026-09-11 (exp51); which merchant pays makes no
 * difference to anything asserted here, and every difference to whether
 * `dashboard.cy.ts` can find the intent afterwards.
 *
 * `237670000900` keys WireMock scenario `mtn-e2e-poll` (priority 5) to
 * answer `PENDING` on the first `requesttopay` status query and `SUCCESSFUL`
 * on the next, so this spec's wait for `succeeded` exercises the real poll
 * ladder rather than a rail that succeeds on the first try. `vpay-worker` —
 * running in the compose stack, not stubbed — is what drives that poll and
 * settles the charge; nothing in this spec pushes the status forward itself.
 *
 * **Moved off `237600000ce0` on 2026-09-17**, the same migration
 * `cypress/support/shop.ts` already made for the demo numbers ([issue
 * #189]). That number was a hex *steering code*, not a phone number — the
 * `ce0` suffix named the outcome — and since [issue #186] landed real
 * server-side phone validation (`vpay-api`'s `v1/payer_fields.rs`, on
 * libphonenumber), a confirm carrying one gets vpay's own `400` before the
 * rail is ever asked, so the intent never leaves `requires_payment_method`
 * and this spec times out waiting for `processing`.
 *
 * It does not fail as *malformed*, which is worth knowing before writing
 * another one: libphonenumber applies its alphanumeric-keypad mapping, so
 * `…ce0` parses fine as `+237600000230` and is then rejected for being
 * `is_valid() == false` with type `Unknown` rather than `Mobile`. The whole
 * `2376000000xx` block fails that way — prefix `60` is not a Cameroon mobile
 * prefix, so even the digits-only `237600000100` is invalid.
 *
 * `237670000900` validates (`+237670000900`, `Mobile`), it is already in
 * this scenario's own `payer.partyId` matcher, and `shop.ts` documents it as
 * driving the identical walk that `worker_e2e.rs`'s `SETTLING_MSISDN`
 * drives.
 *
 * [issue #186]: https://github.com/vaam-apps/vpay/issues/186
 * [issue #189]: https://github.com/vaam-apps/vpay/issues/189
 */
const MTN_E2E_POLL_MSISDN = "237670000900";

describe("checkout-browser", () => {
  it("confirms an MTN MoMo push through @vaam-apps/vpay-stripe-js and settles to succeeded", () => {
    cy.task("mintCheckoutPaymentIntent").then((minted) => {
      const { id, clientSecret, publishableKey, baseUrl } = minted as {
        id: string;
        status: string;
        clientSecret: string;
        publishableKey: string;
        baseUrl: string;
      };

      cy.log(`minted ${id}`);

      cy.url().then(() => {
        const checkoutBrowserUrl = Cypress.env(
          "CHECKOUT_BROWSER_URL",
        ) as string;
        const url = new URL(checkoutBrowserUrl);
        url.searchParams.set("pk", publishableKey);
        url.searchParams.set("client_secret", clientSecret);
        url.searchParams.set("api", baseUrl);
        // A different origin than `baseUrl` (the dashboard's :3000). Visiting
        // it directly, without `cy.origin()`, is fine because this test
        // never navigates back — see Cypress's cross-origin rules.
        cy.visit(url.toString());
      });

      // The page's first render, before any confirm: retrievePaymentIntent's
      // result, not a placeholder.
      cy.get("#status", { timeout: 10_000 })
        .should("have.attr", "data-status", "requires_payment_method")
        .and("contain.text", "requires_payment_method");
      cy.get("#intent-id").should("contain.text", id);
      cy.get("#error").should("not.be.visible");

      cy.get("#confirm-form").should("be.visible");
      cy.get("#msisdn").type(MTN_E2E_POLL_MSISDN);
      cy.get("#confirm-button").click();

      // confirmMobileMoneyPayment's own result: the rail accepted the push
      // and the intent moved to `processing` before this page starts polling.
      cy.get("#status", { timeout: 10_000 }).should(
        "have.attr",
        "data-status",
        "processing",
      );
      cy.get("#waiting").should("be.visible");

      // waitForPaymentIntent polls every ~2s (jittered) until the intent
      // stops moving; the worker's own poll ladder is what actually drives
      // MTN's stub from PENDING to SUCCESSFUL, so this budget has to clear
      // both. 60s comfortably covers `docs/flows/crash-safety.md`'s ladder
      // for this scenario without the retry-count assertions living here.
      cy.get("#status", { timeout: 60_000 })
        .should("have.attr", "data-status", "succeeded")
        .and("contain.text", "succeeded");
      cy.get("#waiting").should("not.be.visible");
      cy.get("#error").should("not.be.visible");
    });
  });
});
