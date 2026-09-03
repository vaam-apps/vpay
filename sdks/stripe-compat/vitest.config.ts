import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.compat.test.ts"],
    // Fails the run rather than reporting a green nothing if the glob above
    // ever stops matching. See `src/preflight.ts` for the same rule applied
    // to a missing stack.
    passWithNoTests: false,
    // Every case talks to one shared vpay over the network, and a few of
    // them measure how long stripe-node waited. Parallel files would make
    // those measurements a property of the machine.
    fileParallelism: false,
    globalSetup: ["./src/preflight.ts"],
    // A confirm reaches a rail over HTTP; 30s is comfortably above that and
    // well below anything that would hide a hang. The one case that waits for
    // the worker to settle a charge sets its own, longer timeout inline, so
    // that a settlement failure reports as "still processing after N polls"
    // rather than as an anonymous vitest timeout.
    testTimeout: 30_000,
    hookTimeout: 30_000,
  },
});
