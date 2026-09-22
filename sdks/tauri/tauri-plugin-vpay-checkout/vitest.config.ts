import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Browser code, tested on Node — the same choice `sdks/stripe-js` made.
    // Every browser thing this package touches is injectable and is
    // injected by the suite rather than emulated: `fetch` through
    // `VpayCheckoutOptions.fetch`, the Tauri `invoke`/`Channel` pair
    // through `tauriHost({ … })`, and the popup `Window` through
    // `webHost(win)`. No jsdom, because no test here is about a real DOM —
    // `host-web.test.ts` drives a hand-rolled window whose `open`,
    // `addEventListener` and `location.origin` are the whole surface the
    // host uses, which makes the assertions about the host's rules rather
    // than about jsdom's fidelity.
    environment: "node",
    include: ["guest-js/**/*.test.ts"],
  },
});
