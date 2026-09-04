import { defineConfig } from "cypress";
import {
  CHECKOUT_BROWSER_URL,
  startCheckoutBrowserServer,
  stopCheckoutBrowserServer,
} from "./cypress/tasks/checkoutBrowserServer.js";
import {
  FRAME_FIXTURE_URL,
  startFrameFixtureServer,
  stopFrameFixtureServer,
} from "./cypress/tasks/frameFixtureServer.js";
import { mintCheckoutPaymentIntent } from "./cypress/tasks/checkoutTasks.js";

/**
 * The compose stack's published ports, as a BROWSER reaches them.
 *
 * Defaults are `compose.e2e.yml`'s literal publications, which are also what
 * `compose.demo.yml` publishes when `VPAY_DEMO_*` is unset — the arrangement
 * CI's `e2e` job relies on. Each is overridable so a second stack on
 * `just demo_port=… demo_shop_port=…` can be driven from the same specs.
 */
const shopUrl = process.env["VPAY_SHOP_URL"] ?? "http://localhost:3001";
const checkoutUrl = process.env["VPAY_CHECKOUT_URL"] ?? "http://localhost:3080";
const orangeStubUrl =
  process.env["VPAY_ORANGE_STUB_URL"] ?? "http://localhost:8082";
/**
 * `shop-merchant`'s publishable key. A fixed literal in
 * `just gen-demo-keys`'s generated overlay and in `compose.e2e.yml`'s
 * `vpay-shop` service, for the reason that recipe states about
 * `demo-merchant`'s: a generated one would have to be threaded into three
 * more files.
 */
const shopPublishableKey =
  process.env["VPAY_SHOP_PUBLISHABLE_KEY"] ?? "pk_test_shopmerchantsandbox1";

/**
 * `shop-embedded.cy.ts` needs the browser's same-origin policy off to reach
 * into a cross-origin iframe, and `chromeWebSecurity` is a browser LAUNCH
 * flag — Cypress cannot override it per test. So that spec runs in its own
 * `cypress run` (`package.json`'s `e2e:framed`), selected by this variable,
 * and every other spec keeps web security on. `checkout.cy.ts` in particular
 * still exercises `/v1/browser`'s CORS layer for real.
 */
const framedRun = process.env["VPAY_E2E_FRAMED"] === "1";
const FRAMED_SPEC = "cypress/e2e/shop-embedded.cy.ts";

export default defineConfig({
  e2e: {
    baseUrl: process.env["VPAY_DASHBOARD_URL"] ?? "http://localhost:3000",
    supportFile: "cypress/support/e2e.ts",
    specPattern: framedRun ? FRAMED_SPEC : "cypress/e2e/**/*.cy.ts",
    excludeSpecPattern: framedRun ? [] : [FRAMED_SPEC],
    chromeWebSecurity: !framedRun,
    // FRAME-BUSTING REWRITES, AND WHY THE TWO RUNS SET THEM DIFFERENTLY.
    //
    // Cypress renders the application under test inside an IFRAME of its own,
    // and by default (`modifyObstructiveCode`) it rewrites `window.top` and
    // `window.parent` reads in the JavaScript it proxies so that an app which
    // busts out of frames does not bust out of the runner. That rewrite hands
    // the reading window ITSELF back.
    //
    // vpay's checkout page reads exactly that property, and reaches opposite
    // conclusions from it in its two modes (`src/lib/entry.ts`):
    //
    //   /c/{id}  hosted   — refuses if it IS framed (the second lock behind
    //                       `frame-ancestors 'none'`)
    //   /e/{id}  embedded — refuses if it is NOT framed (it would have nobody
    //                       to report the outcome to)
    //
    // So one run cannot serve both, and each is given what it needs:
    //
    //   the ordinary run (`shop-hosted.cy.ts`, `checkout.cy.ts`, the
    //   dashboard) keeps the rewrite AND extends it to third-party code,
    //   because vpay's page is never the primary origin here — the shop is —
    //   and the default only rewrites the primary origin's JavaScript. Without
    //   `experimentalModifyObstructiveThirdPartyCode` every hosted test failed
    //   on the refusal screen "This page will not load here". MEASURED.
    //
    //   the framed run (`shop-embedded.cy.ts`) turns BOTH off, so the page in
    //   the iframe sees its real parent. With the default left on, every
    //   nested frame reports `window.parent === window` to its own code and
    //   the embedded page refuses — including the one framed from an
    //   unregistered origin, which would then have "passed" the negative test
    //   for the wrong reason entirely. MEASURED, by instrumenting
    //   `decideEntry`'s inputs in a throwaway build: `framed:false` with the
    //   rewrite on, `framed:true` and `decision:"ready"` with it off, same
    //   session, same shop, same referrer.
    //
    // What the ordinary run therefore does NOT prove: that vpay's HOSTED page
    // refuses to render inside a frame. Nothing here can — the runner is a
    // frame. `frontends/apps/checkout/src/lib/entry.test.ts` covers it.
    experimentalModifyObstructiveThirdPartyCode: !framedRun,
    modifyObstructiveCode: !framedRun,
    video: false,
    retries: { runMode: 2, openMode: 0 },
    setupNodeEvents(on, config) {
      // Two servers, both on their own origin, both for the whole run rather
      // than per-spec: starting them here keeps the "how is it served"
      // answer in one place — docs/flows/browser-checkout.md points here.
      //
      // `examples/checkout-browser` is `checkout.cy.ts`'s page under test.
      // The frame fixture is `shop-embedded.cy.ts`'s NEGATIVE case: a page on
      // an origin that is in nobody's `checkout_origins`, which is a thing
      // only a separate origin can be.
      on("before:run", async () => {
        await startCheckoutBrowserServer();
        await startFrameFixtureServer();
      });
      on("after:run", () => {
        stopCheckoutBrowserServer();
        stopFrameFixtureServer();
      });

      // Runs in Node, not the browser — see checkoutTasks.ts's header for
      // why the merchant credential has to be minted here rather than in
      // the page under test.
      on("task", {
        mintCheckoutPaymentIntent,
        dump(payload: { what: string; value: string }) {
          console.log("PROBE " + payload.what + ": " + payload.value);
          return null;
        },
      });

      // `checkout.cy.ts` reads `CHECKOUT_BROWSER_URL` through `Cypress.env`,
      // which Cypress 15 deprecates in favour of `Cypress.expose` — that
      // spec is unchanged here, so this value stays where it is looking.
      config.env["CHECKOUT_BROWSER_URL"] = CHECKOUT_BROWSER_URL;
      // Everything new goes through `expose`, the supported API: these are
      // origins and a publishable key, all public by name, and none of them
      // is a credential.
      config.expose = {
        ...config.expose,
        FRAME_FIXTURE_URL,
        SHOP_URL: shopUrl,
        CHECKOUT_URL: checkoutUrl,
        ORANGE_STUB_URL: orangeStubUrl,
        SHOP_PUBLISHABLE_KEY: shopPublishableKey,
      };
      return config;
    },
  },
});
