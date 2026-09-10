import { defineConfig } from "vitest/config";

/**
 * The live suite: `src/**\/*.live.test.ts`, against a **real** vpay.
 *
 * A second project rather than a second describe block, for
 * `sdks/stripe-compat`'s reason: those cases need a stack, `pnpm test` must
 * stay runnable without one, and the answer to "no stack" must be a failure
 * rather than a skip. `vitest.config.ts` excludes this glob and this file is
 * the only thing that includes it.
 */
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.live.test.ts"],
    // Fails the run rather than reporting a green nothing if the glob above
    // ever stops matching — the same rule `sdks/stripe-compat` applies.
    passWithNoTests: false,
    // Every case talks to one shared vpay, and the lifecycle case depends on
    // its own ordering.
    fileParallelism: false,
    globalSetup: ["./src/testing/live-preflight.ts"],
    testTimeout: 30_000,
    hookTimeout: 30_000,
  },
});
