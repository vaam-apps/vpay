/**
 * `examples/shop`'s `/orders/{id}/embedded` page: vpay's checkout page framed
 * on the merchant's own site, driven in a real browser against the real
 * compose stack (Step 9, lane 6, D8).
 *
 * ## Why this spec runs on its own, with `chromeWebSecurity: false`
 *
 * The iframe is `http://localhost:{checkout}` inside `http://localhost:{shop}`
 * — a genuine cross-origin frame, and a test runner cannot reach into one
 * while the browser's same-origin policy is on. `chromeWebSecurity` is a
 * browser launch flag, not a per-test option, so `package.json`'s `e2e`
 * script runs this file in a SECOND `cypress run` with the flag off and
 * leaves every other spec's run with web security intact. `checkout.cy.ts`'s
 * evidence about `/v1/browser`'s CORS layer is therefore unchanged.
 *
 * ## What is proven about `frame-ancestors`, and what is not
 *
 * The header is asserted as the SERVER SENDS IT, with `cy.request` — which
 * goes out from Cypress's Node process and reads the real response headers.
 * Browser ENFORCEMENT of `frame-ancestors` is not observable here, and that
 * is a property of the runner rather than of vpay: Cypress strips the
 * `Content-Security-Policy` header from every document it proxies
 * (`experimentalCspAllowList` defaults to `false`), and turning that off is
 * not an option — the hosted page sends `frame-ancestors 'none'`, and
 * Cypress renders the application under test inside an iframe of its own, so
 * an enforced policy would stop `shop-hosted.cy.ts` from loading vpay's page
 * at all. Said plainly rather than papered over: no test in this repository
 * has seen a browser refuse a frame because of vpay's CSP.
 *
 * What IS proven in a browser is vpay's own second lock, which does not
 * depend on the header at all: the page resolves its framer from
 * `document.referrer` against the merchant's registered origins and refuses
 * before it looks at the credential (`src/lib/entry.ts`,
 * `src/lib/origins.ts`). The last test below frames the byte-identical
 * `src` from an origin nobody registered and watches it refuse.
 */

import {
  MTN,
  PRODUCT,
  checkoutOrigin,
  frameFixtureUrl,
  orangeOrigin,
  orderIdFromUrl,
  readOrder,
  shopPublishableKey,
  shopUrl,
} from "../support/shop";

/** Drives the shop's UI to `/orders/{id}/embedded`. */
function buyWithoutLeavingTheShop(): void {
  cy.visit(shopUrl());
  cy.get(`[data-testid="add-${PRODUCT.coffee}"]`).click();
  cy.visit(`${shopUrl()}/checkout`);
  cy.get('[data-testid="email"]').type("framed@example.test");
  // One button and a surface selector since 2026-09-06 (exp22), where there
  // used to be one button per surface. `check()` rather than `click()`: the
  // input is the thing with the state, and a click on the label around it
  // would pass whether or not the radio moved.
  cy.get('[data-testid="mode-embedded"]').check();
  cy.get('[data-testid="pay"]').click();
  cy.url({ timeout: 60_000 }).should("include", "/embedded");
}

/** The shop's own embedded panel, and the fixture page's frame. */
const SHOP_FRAME = "#vpay-embedded-checkout iframe";
const FIXTURE_FRAME = "#framed";

/**
 * One element inside the framed document, retried for as long as the caller
 * says. Reaching into the frame at all is possible only because this file's
 * run has web security off — see the header.
 *
 * The timeout is on the `find`, not only on the `iframe`: the frame's `<body>`
 * exists (and is non-empty) while vpay's page is still on its `loading`
 * screen, so a default 4 s `find` fails on a page that is working perfectly.
 * Measured, on the first run of this spec.
 */
function inFrame(
  selector: string,
  options: { frame?: string; timeout?: number } = {},
): Cypress.Chainable<JQuery<HTMLElement>> {
  const frame = options.frame ?? SHOP_FRAME;
  const timeout = options.timeout ?? 60_000;
  return cy
    .get(frame, { timeout })
    .its("0.contentDocument.body", { timeout })
    .should("not.be.empty")
    .then((body) =>
      cy
        .wrap(body as unknown as JQuery<HTMLElement>)
        .find(selector, { timeout }),
    );
}

describe("the shop, paid inside an iframe on its own page", () => {
  it("frames vpay's page with the exact src, and vpay names the shop's origin in frame-ancestors", () => {
    buyWithoutLeavingTheShop();

    orderIdFromUrl().then((orderId) => {
      cy.get("#vpay-embedded-checkout iframe", { timeout: 60_000 })
        .should("have.attr", "src")
        .then((value) => {
          // `.should('have.attr', ...)` yields the attribute string at runtime;
          // Cypress's types leave the subject typed as the element.
          // eslint-disable-next-line @typescript-eslint/no-base-to-string
          const src = String(value);
          readOrder(orderId).then((order) => {
            const sessionId = order.checkoutSessionId;
            expect(sessionId, "the order's checkout session").to.match(/^cs_/);
            // D6, exactly: the id in the path, the PUBLISHABLE key in the
            // query (it is public by name and the page's own server needs it
            // before any script runs), and the session's secret in the
            // FRAGMENT, which is never sent to a server.
            const [beforeHash, fragment] = src.split("#");
            expect(beforeHash).to.equal(
              `${checkoutOrigin()}/e/${sessionId}?key=${shopPublishableKey()}`,
            );
            expect(fragment ?? "").to.match(
              new RegExp(`^${sessionId}_secret_[A-Za-z0-9]+$`),
            );
            expect(src).to.not.include("client_secret=");

            // The header as the server sends it. `cy.request` is issued from
            // Cypress's Node process, so this is the real response and not
            // one the runner's proxy has rewritten.
            cy.request(beforeHash as string).then((response) => {
              expect(
                response.headers["content-security-policy"],
                "frame-ancestors on the embedded page",
              ).to.equal(`frame-ancestors ${shopUrl()}`);
              expect(response.headers["referrer-policy"]).to.equal(
                "no-referrer",
              );
              expect(response.headers["cache-control"]).to.include("no-store");
            });
          });
        });
    });

    // And it is not merely served: the page inside the frame reached its
    // first payable screen, which means the browser read succeeded with the
    // credential out of that fragment.
    inFrame('[data-screen="select_rail"]').should("be.visible");
  });

  it("MTN completes inside the frame, the shop is told, and the order reaches `paid` through the webhook", () => {
    buyWithoutLeavingTheShop();

    inFrame('[data-screen="select_rail"]').should("be.visible");
    inFrame('button[data-rail="mtn_momo"]').click();
    inFrame("#vpay-msisdn").type(MTN.succeeds);
    inFrame('button[type="submit"]').click();

    // Settled by `vpay-worker` polling MTN's WireMock stub, exactly as in the
    // hosted spec. Nothing in the browser moves the intent.
    inFrame('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
      "be.visible",
    );

    // `vpay:complete` — the shop's own `onComplete` treats it as a cue and
    // sends the payer to the return page, which reads the DATABASE. The
    // message is not what marks the order paid.
    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );
    cy.get('[data-testid="order-status"]').should("have.text", "Paid");

    orderIdFromUrl().then((orderId) => {
      readOrder(orderId).then((order) => {
        expect(order.status).to.equal("paid");
      });
    });
  });

  it("Orange asks the shop to break out of the frame, and the trip ends on the shop's return_url", () => {
    buyWithoutLeavingTheShop();

    // WHY THE PARENT'S NAVIGATION IS INTERCEPTED HERE, AND WHAT THAT COSTS.
    //
    // An iframe may not navigate its parent — `FRAME_SANDBOX` withholds
    // `allow-top-navigation` — so vpay's page posts `{type:'vpay:redirect',
    // url}` and `@vaam-apps/vpay-stripe-js` performs `win.top.location.assign(url)`.
    // In a payer's browser `win.top` is the merchant's own tab. Under Cypress
    // it is the RUNNER's window, and this run has the frame-busting rewrite
    // off (see `cypress.config.ts`), so the SDK would navigate Cypress itself
    // away from the test. Measured: the run wedges there and never returns.
    //
    // So a capture-phase listener records the message and stops it reaching
    // the SDK's handler, and the spec then performs the navigation the SDK
    // would have performed, to the byte-identical URL it was given.
    //
    // What that does NOT prove, stated rather than implied: that
    // `@vaam-apps/vpay-stripe-js` calls `window.top.location.assign` with that URL.
    // `sdks/stripe-js`'s vitest covers exactly that. What IS proven here, in
    // a real browser, is everything either side of it — the framed page
    // confirms with the rail, asks its parent to move, names the rail's own
    // page, and the trip that follows ends on the shop's `return_url` with
    // the order paid.
    cy.window().then((win) => {
      const store: { origin: string; url: unknown }[] = [];
      (win as unknown as { __vpayRedirects: typeof store }).__vpayRedirects =
        store;
      win.addEventListener(
        "message",
        (event: MessageEvent) => {
          const data: unknown = event.data;
          if (
            typeof data === "object" &&
            data !== null &&
            (data as { type?: unknown }).type === "vpay:redirect"
          ) {
            store.push({
              origin: event.origin,
              url: (data as { url?: unknown }).url,
            });
            event.stopImmediatePropagation();
          }
        },
        true,
      );
    });

    inFrame('[data-screen="select_rail"]').should("be.visible");
    inFrame('button[data-rail="orange_money"]').click();
    inFrame('[data-screen="ready_redirect"]').should("be.visible");
    inFrame("button.btn-primary").click();

    cy.window()
      .its("__vpayRedirects", { timeout: 60_000 })
      .should("have.length", 1)
      .then((redirects) => {
        const message = (
          redirects as unknown as { origin: string; url: string }[]
        )[0];
        expect(message).to.not.equal(undefined);
        const { origin, url } = message as { origin: string; url: string };
        // The origin check is the whole security boundary of `embedded.ts`.
        expect(origin, "the message's origin").to.equal(checkoutOrigin());
        // The rail's own page, minted by the stub from the submit vpay made.
        expect(url).to.match(
          new RegExp(`^${orangeOrigin()}/stub-hosted-page/`),
        );

        cy.origin(orangeOrigin(), { args: { url } }, ({ url: railUrl }) => {
          cy.visit(railUrl);
          cy.get("#pay", { timeout: 60_000 }).should("be.visible");
          cy.get("#pay").click();
        });
      });

    cy.origin(checkoutOrigin(), () => {
      cy.url({ timeout: 60_000 }).should("include", "/return");
      cy.get('[data-outcome="succeeded"]', { timeout: 120_000 }).should(
        "be.visible",
      );
      cy.get('[data-outcome="succeeded"] button').click();
    });

    // An embedded session has one `return_url` for every outcome, and this is
    // it — the shop's own return page.
    cy.url({ timeout: 60_000 }).should("include", "/return");
    cy.get('[data-testid="paid-message"]', { timeout: 120_000 }).should(
      "be.visible",
    );
    orderIdFromUrl().then((orderId) => {
      readOrder(orderId).then((order) => {
        expect(order.status).to.equal("paid");
      });
    });
  });

  it("refuses to be framed by an origin the merchant never registered", () => {
    // The credential is real and the URL is built by the same rule
    // `embeddedFrameSrc` uses (asserted byte for byte against the shop's own
    // iframe in the first test above). The only thing that differs from the
    // working case is the origin of the page doing the framing.
    cy.request("POST", `${shopUrl()}/api/trpc/orders.create`, {
      email: "stranger@example.test",
      lines: [{ productId: PRODUCT.coffee, quantity: 1 }],
      mode: "embedded",
    }).then((created) => {
      const orderId = (
        created.body as { result: { data: { orderId: string } } }
      ).result.data.orderId;
      cy.request("POST", `${shopUrl()}/api/trpc/orders.embeddedSecret`, {
        orderId,
      }).then((minted) => {
        const { clientSecret, sessionId } = (
          minted.body as {
            result: { data: { clientSecret: string; sessionId: string } };
          }
        ).result.data;
        const src = `${checkoutOrigin()}/e/${sessionId}?key=${shopPublishableKey()}#${clientSecret}`;

        // The header the merchant's registered origin gets — and it does not
        // name the fixture's.
        cy.request(src.split("#")[0] as string).then((response) => {
          const csp = response.headers["content-security-policy"];
          expect(csp).to.equal(`frame-ancestors ${shopUrl()}`);
          expect(csp).to.not.include(frameFixtureUrl());
        });

        cy.visit(`${frameFixtureUrl()}/frame?src=${encodeURIComponent(src)}`);
        cy.get("#fixture-heading").should("be.visible");

        // vpay's own second lock, in a real browser: the page resolved its
        // framer from `document.referrer`, did not find it on the merchant's
        // list, and refused — WITHOUT reading the session, so a hostile
        // framer learns nothing about the credential it was handed.
        inFrame('[data-screen="refused_embed"]', {
          frame: FIXTURE_FRAME,
        }).should("be.visible");
        inFrame('[data-testid="notice-body"]', {
          frame: FIXTURE_FRAME,
        }).should("not.be.empty");
        // No payable screen, and no summary — the refusal happens before the
        // read, so the page never learned what this session is worth.
        inFrame('[data-screen="select_rail"]', {
          frame: FIXTURE_FRAME,
          timeout: 4_000,
        }).should("not.exist");
        inFrame('[data-testid="amount"]', {
          frame: FIXTURE_FRAME,
          timeout: 4_000,
        }).should("not.exist");

        // And it told the framer nothing: no `vpay:resize`, no
        // `vpay:complete`, no `vpay:redirect`.
        cy.window()
          .its("__vpayFrameMessages")
          .should("be.an", "array")
          .and("have.length", 0);

        // The order is untouched by any of it.
        readOrder(orderId).then((order) => {
          expect(order.status).to.equal("unpaid");
        });
      });
    });
  });
});
