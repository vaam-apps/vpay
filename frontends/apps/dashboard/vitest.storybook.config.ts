import { playwright } from "@vitest/browser-playwright";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import { defineConfig } from "vitest/config";

/**
 * Renders every dashboard story in a real browser and fails on an
 * accessibility violation.
 *
 * Modelled on `frontends/apps/checkout/vitest.storybook.config.ts` — see
 * that file's own doc comment for why this suite exists at all (the jsdom
 * axe suite next door, `src/a11y.test.tsx`, computes no colour and answers
 * `color-contrast` "incomplete" rather than pass or fail) and why it is a
 * separate config file from `vitest.config.ts` (this one needs a Chromium
 * binary and `just ci` is expected to pass offline).
 *
 * There is deliberately no `setupFiles` calling `setProjectAnnotations`:
 * `@storybook/addon-vitest` has applied `.storybook/preview.ts` itself since
 * Storybook 10.3 and warns when a setup file does it too, because the
 * manual call SUPPRESSES the automatic one.
 */
export default defineConfig({
  /**
   * **Every dependency the stories reach, declared up front — measured
   * against THIS app's own component tree, not copied from the checkout's
   * list.** Checked by grepping every component this suite stories for its
   * runtime imports (`import`, not `import type`): only `react`,
   * `react-dom`, `lucide-react` (`AppShell`, `MoreMenu`), `@vaam-apps/ui`
   * and `@vpay/tokens` (`PaymentsFilters`' `PAYMENT_STATUS`) are reached.
   *
   * `@refinedev/core`, `@tanstack/react-query` and `@vpay/api-client` —
   * named as "at minimum" in this task's brief — are NOT reached by this
   * story set: `src/dash/resources.ts` imports `ResourceProps` from
   * `@refinedev/core` with `import type` only (erased at build time, so it
   * pulls no runtime module), and nothing this suite stories imports
   * `@tanstack/react-query` or `@vpay/api-client` at all — those three are
   * reached only by `app/(dash)/**` page components and `src/dash/*`
   * providers, none of which this suite renders (see the router-mock note
   * in `.storybook/main.ts` for why the app's actual pages are not storied).
   * Listed here anyway is a guess this file does not make; if a future story
   * renders a page-level component that does reach them, add the entry then
   * and re-measure — that is what this comment is for.
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
      "lucide-react",
    ],
  },
  test: {
    name: "storybook",
    /**
     * **`css: true` is the difference between measuring the product and
     * measuring nothing.** Vitest stubs a CSS import by default, so without
     * this `preview.ts`'s `import "../app/globals.css"` is a no-op and every
     * story renders unstyled — see `.storybook/main.ts`'s doc comment for
     * the measured counts on this app's own build.
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
      // The stories are the subject; `.mdx` docs pages are not.
      disableAddonDocs: true,
    }),
  ],
});
