import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

export default defineConfig({
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  // The app's `tsconfig.json` sets `jsx: preserve` because Next does its own
  // JSX transform; esbuild reads that and would emit classic
  // `React.createElement` calls into the test bundle, where nothing imports
  // `React`. The runtime transform is stated here rather than by relaxing
  // the tsconfig Next relies on. Mirrors frontends/apps/checkout's
  // vitest.config.ts, which hit the same thing first.
  esbuild: { jsx: "automatic", jsxImportSource: "react" },
  test: {
    // `node` by default — every existing suite here is server/business
    // logic. The two component-rendering suites this lane adds
    // (order-summary.test.tsx, test-numbers-panel.test.tsx — the daisyUI
    // tone/a11y decisive mutations, plan §5 Lane C) opt into a DOM with a
    // `// @vitest-environment jsdom` docblock, same convention as checkout.
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    setupFiles: ["./vitest.setup.ts"],
  },
});
