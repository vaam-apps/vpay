import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    // The live suite is a separate project (`vitest.live.config.ts`), because
    // it needs a running vpay and this one must stay runnable without one.
    // Excluded here rather than `.skip`ped there: a skipped case reports `ok`
    // and is indistinguishable, in a CI summary, from one that passed.
    exclude: ["src/**/*.live.test.ts"],
  },
});
