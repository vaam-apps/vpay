import { defineConfig } from "vitest/config";

export default defineConfig({
  // The app's `tsconfig.json` sets `jsx: preserve` because Next does its own
  // JSX transform; esbuild reads that and would emit classic
  // `React.createElement` calls into the test bundle, where nothing imports
  // `React`. The runtime transform is stated here rather than by relaxing
  // the tsconfig Next relies on — the same fix `frontends/apps/checkout`
  // and `frontends/packages/ui` already carry.
  esbuild: { jsx: "automatic", jsxImportSource: "react" },
  test: {
    environment: "jsdom",
    globals: false,
    // `app/` as well as `src/`. A `.test.ts` written beside a route matched
    // nothing here and ran nowhere — a test file that is silently not a test,
    // which is the class of thing this repository is careful about
    // everywhere else (exp28 review).
    //
    // And `middleware.test.ts`, which is at the project root because the file
    // it tests has to be: Next reads a middleware only from there, and its
    // `config.matcher` must be a literal in that file (`getPageStaticInfo`
    // extracts it with SWC). Without this entry it would have been the exp28
    // review's finding again, one directory up.
    include: ["{src,app}/**/*.test.{ts,tsx}", "middleware.test.ts"],
    setupFiles: ["./vitest.setup.ts"],
  },
});
