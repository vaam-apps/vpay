import { playwright } from "@vitest/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import { defineConfig } from "vitest/config";

/**
 * Renders every checkout story in a real browser and fails on an
 * accessibility violation.
 *
 * **Why this exists at all.** `just ci` builds neither web app, and the two
 * axe suites next door (`screens.axe.test.tsx`, `outcome-contrast.test.ts`)
 * run in jsdom — which implements no layout and no cascade, so it computes
 * no colour and answers `color-contrast` "incomplete" rather than pass or
 * fail. `outcome-contrast.test.ts`'s own header records that issue #73's
 * Cypress attempt could not get a verdict out of the real page either:
 * daisyUI's `:root` scroll-lock rule carries an unconditional
 * `background-image` and axe-core abandons the rule under any such
 * ancestor. A story renders inside `#storybook-root` with no such ancestor,
 * which is why this route returns a verdict where Cypress could not.
 *
 * **Why a separate config file.** `vitest.config.ts` is the jsdom suite, and
 * it is what `pnpm --filter @vpay/checkout test` — and therefore
 * `just test-web`, and therefore `just ci` — runs. This one needs a Chromium
 * binary (~115 MB from Playwright's CDN) and `just ci` is expected to pass
 * offline, the same reason `helm-check` is excluded from it. Folding them
 * together would turn "offline" into "failing" for the standard gate.
 *
 * There is no second axe configuration to keep in step: `addon-a11y`'s own
 * `afterEach` is what runs, reading the same `parameters.a11y` its panel
 * reads, with the addon's own axe-core.
 *
 * There is deliberately no `setupFiles` calling `setProjectAnnotations`.
 * `@storybook/addon-vitest` has applied `.storybook/preview.ts` itself since
 * Storybook 10.3 and warns when a setup file does it too, because the manual
 * call SUPPRESSES the automatic one — measured on the predecessor of this
 * file, where it reported 93 passing stories while checking none of them.
 */
export default defineConfig({
  /*
    No `esbuild.jsx` here on purpose. It lives in `.storybook/main.ts`'s
    `viteFinal`, which `build-storybook` and this plugin both read through
    `configDir` — so the setting cannot differ between the artefact a human
    opens and the one axe measures. It was duplicated here for one revision
    and that is how the divergence was found: this suite passed all 22
    stories while the built Storybook rendered "React is not defined" on
    every one of them. One setting, one place, and THIS suite failing is now
    the signal that the build's JSX runtime is wrong.
  */
  /**
   * **Every dependency the stories reach, declared up front.** Without this
   * the suite is a false green rather than a failure: vite discovers a
   * package's entries while the run is already underway, re-bundles, and the
   * modules loaded before the re-bundle keep a `null` React. Components then
   * die with `Cannot read properties of null (reading 'useContext')` and
   * vitest reports every story PASSING alongside a separate count of
   * unhandled errors — because a story that throws while rendering still
   * counts as a passing test, and axe never ran on it.
   *
   * Measured on the predecessor of this file (PR #133): 24 such errors on a
   * CI runner against 0 locally, because it reproduces from a COLD dep cache
   * only. `resolve.dedupe` does not fix it and `optimizeDeps.exclude` makes
   * it worse — React is CJS, so excluding it from pre-bundling is what
   * breaks the import. `just test-storybook` also clears the cache before
   * every run, so a local green means what a runner's green means.
   */
  optimizeDeps: {
    include: [
      "react",
      "react-dom",
      "react-dom/client",
      "react/jsx-runtime",
      "react/jsx-dev-runtime",
      "@vaam-apps/ui",
      "@vpay/tokens",
    ],
  },
  test: {
    name: "storybook",
    /**
     * **`css: true` is the difference between measuring the product and
     * measuring nothing.** Vitest does not process CSS imports by default —
     * it stubs them — so `preview.ts`'s `import "../app/globals.css"` was a
     * no-op and every story rendered completely unstyled: measured,
     * `--color-base-100` resolved to the empty string and `document.body`'s
     * background came back `rgba(0, 0, 0, 0)` while the shipped page is
     * `#0a0b0d`. The suite passed 22 stories that way, and a deliberately
     * unreadable `#3a3a3a` probe passed with them, because axe was
     * measuring dark text against the browser's default white.
     *
     * `build-storybook` does NOT share this failure — its output carries the
     * full 136 KB stylesheet — so a green build says nothing about whether
     * this run is styled. That is why `a11y-gate.test.ts` asserts the
     * setting rather than trusting it.
     */
    css: true,
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
      // them costs time for nothing this suite asserts.
      disableAddonDocs: true,
    }),
  ],
});
