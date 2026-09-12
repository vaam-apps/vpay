import { playwright } from "@vitest/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import { defineConfig } from "vitest/config";

/**
 * Runs every story in a real browser and fails on an accessibility
 * violation.
 *
 * **Why this is a separate config file and not a project in
 * `vitest.config.ts`.** That one is the jsdom unit suite, and it is what
 * `pnpm --filter @vpay/ui test` — and therefore `just test-web`, and
 * therefore `just ci` — runs. This suite needs a Chromium binary
 * (~115 MB, fetched from Playwright's CDN), and `just ci` is expected to
 * pass on a machine with no network; the justfile says so where it explains
 * why `helm-check` is not in `just ci` either. Folding the two together
 * would turn "offline" into "failing" for everyone running the standard
 * gate. It runs in CI's `web` job instead, next to `build-storybook`, which
 * is also not in `just ci`.
 *
 * **What it checks that nothing else did.** `@storybook/addon-a11y` has
 * been installed since 2026-09-04 and reported only in its own panel, which
 * nothing in CI opens; `build-storybook` compiles the stories and renders
 * none of them. The checkout's `screens.axe.test.tsx` does run axe, but in
 * jsdom — which implements no layout and no cascade, so it computes no
 * colour and answers `color-contrast` "incomplete" rather than pass or
 * fail. This suite is the first thing in the repository that runs axe
 * against pixels.
 *
 * There is no second axe configuration to keep in step: the addon's own
 * `afterEach` is what runs here, reading the same `parameters.a11y` the
 * panel reads, using the addon's own `axe-core` — which resolves to
 * `4.13.0`, the version this package already pins for the jsdom suite.
 *
 * There is deliberately no `setupFiles` entry calling
 * `setProjectAnnotations`. `@storybook/addon-vitest` has applied
 * `.storybook/preview.ts` itself since Storybook 10.3 and prints a warning
 * when a setup file does it too, because the manual call SUPPRESSES the
 * automatic one — so the redundant-looking file is the version that can
 * silently drift from the Storybook a reviewer opens.
 */
export default defineConfig({
  /**
   * The checkout's stories live in `frontends/apps/checkout`, whose
   * `tsconfig.json` sets `"jsx": "preserve"` because Next requires it —
   * and Vite's esbuild reads the tsconfig nearest the file it is
   * transforming. Left alone it therefore emits `React.createElement` for
   * those 22 stories, none of which imports React, and every one of them
   * dies at render with `ReferenceError: React is not defined` — measured,
   * 36 errors on the first run of this suite. `build-storybook` does not
   * hit this: `@storybook/react-vite` runs `@vitejs/plugin-react`'s babel
   * pass over the JSX before esbuild ever sees it.
   *
   * This is not a Next build, so the Next constraint does not apply here;
   * saying `automatic` once is better than adding a React import to 22
   * stories to satisfy a pipeline they are not compiled by.
   */
  esbuild: { jsx: "automatic" },
  resolve: { dedupe: ["react", "react-dom"] },
  /**
   * **Every dependency the stories reach, declared up front.** Without this
   * the suite is a false green: vite discovers `@base-ui/react`'s subpath
   * entries while the run is already underway, re-bundles, and the modules
   * loaded before the re-bundle keep a `null` React — so `SelectRoot`,
   * `Button` and `RadioGroup` die with `Cannot read properties of null
   * (reading 'useContext')` and vitest reports **"93 passed" with 34
   * unhandled errors**, because a story that throws while rendering still
   * counts as a passing test. axe never ran on those stories at all.
   *
   * It reproduces only from a cold dep cache, which is why it survived
   * several local runs and was caught by CI — 24 errors on the `web` job of
   * the first push of this branch (the count varies with load; 34 here, 7
   * with a different `optimizeDeps` setting). `just test-storybook` now
   * clears that cache before every run for exactly this reason: a local
   * green that a warm cache produced is not a green.
   *
   * `resolve.dedupe` alone does not fix it (measured: 36 errors), and
   * `optimizeDeps.exclude` makes it worse rather than better (8 test files
   * failed outright, React `null` at `useRef`) — React is CJS, so excluding
   * it from pre-bundling is what breaks the import. The fix is to pre-bundle
   * everything, together, before the first story renders.
   */
  optimizeDeps: {
    include: [
      "react",
      "react-dom",
      "react-dom/client",
      "react/jsx-runtime",
      "react/jsx-dev-runtime",
      "@base-ui/react/button",
      "@base-ui/react/checkbox",
      "@base-ui/react/dialog",
      "@base-ui/react/drawer",
      "@base-ui/react/field",
      "@base-ui/react/input",
      "@base-ui/react/radio",
      "@base-ui/react/radio-group",
      "@base-ui/react/select",
      "@base-ui/react/use-render",
    ],
  },
  test: {
    name: "storybook",
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: "chromium" }],
    },
  },
  plugins: [
    storybookTest({
      configDir: ".storybook",
      // The stories are the subject; `.mdx` docs pages are not, and parsing
      // them here costs time for nothing this suite asserts.
      disableAddonDocs: true,
    }),
  ],
});
