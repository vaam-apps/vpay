import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // The package is browser code, but its unit tests drive it against a
    // real `node:http` server (see src/testing/browser-stub.ts) rather than
    // a mocked `fetch`, so they run on Node. The two browser globals the
    // package touches — `fetch` and `window` — are supplied per test:
    // `fetch` through the `VpayStripeOptions.fetch` option, `window` by
    // assigning to `globalThis` in the redirect tests only.
    //
    // The one exception is `src/embedded.test.ts`, whose contract is a real
    // `<iframe>` and a real `message` event rather than bytes: it opts into
    // jsdom with a `// @vitest-environment jsdom` docblock of its own, so
    // the DOM is present exactly where it is the subject and nowhere else.
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
