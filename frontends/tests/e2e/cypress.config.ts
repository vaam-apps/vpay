import { defineConfig } from "cypress";
import {
  CHECKOUT_BROWSER_URL,
  startCheckoutBrowserServer,
  stopCheckoutBrowserServer,
} from "./cypress/tasks/checkoutBrowserServer.js";
import { mintCheckoutPaymentIntent } from "./cypress/tasks/checkoutTasks.js";

export default defineConfig({
  e2e: {
    baseUrl: process.env["VPAY_DASHBOARD_URL"] ?? "http://localhost:3000",
    supportFile: "cypress/support/e2e.ts",
    specPattern: "cypress/e2e/**/*.cy.ts",
    video: false,
    retries: { runMode: 2, openMode: 0 },
    setupNodeEvents(on, config) {
      // `examples/checkout-browser` is served on its own origin for the
      // whole run, not per-spec: `checkout.cy.ts` is the only spec that
      // visits it, so starting it once here (rather than inside the spec)
      // keeps the "how is it served" answer in one place —
      // docs/flows/browser-checkout.md points here.
      on("before:run", async () => {
        await startCheckoutBrowserServer();
      });
      on("after:run", () => {
        stopCheckoutBrowserServer();
      });

      // Runs in Node, not the browser — see checkoutTasks.ts's header for
      // why the merchant credential has to be minted here rather than in
      // the page under test.
      on("task", {
        mintCheckoutPaymentIntent,
      });

      config.env["CHECKOUT_BROWSER_URL"] = CHECKOUT_BROWSER_URL;
      return config;
    },
  },
});
