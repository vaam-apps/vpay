import { fileURLToPath } from "node:url";

import type { StorybookConfig } from "@storybook/react-vite";

/**
 * The dashboard's own Storybook.
 *
 * Modelled on `frontends/apps/checkout/.storybook/main.ts`, which was
 * restored 2026-09-12 after the `@vaam-apps/ui` cutover deleted `@vpay/ui`
 * and the Storybook install it hosted. The dashboard never had one — this
 * closes that gap rather than following one that reopened.
 *
 * `@storybook/addon-vitest` is what lets `vitest.storybook.config.ts` render
 * these stories in a real browser; `addon-a11y`'s own `afterEach` then fails
 * the test on an accessibility violation. `@storybook/addon-essentials` is
 * deliberately absent: it does not exist past 8.6.14, and `addon-docs`
 * replaces its docs half.
 */
const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  addons: [
    "@storybook/addon-a11y",
    "@storybook/addon-docs",
    "@storybook/addon-vitest",
  ],
  framework: { name: "@storybook/react-vite", options: {} },
  docs: { autodocs: "tag" },
  /**
   * **The checkout's `viteFinal` also hand-aliases
   * `@vaam-apps/ui/styles/theme.css` — measured here, and NOT carried over,
   * because this build does not need it.**
   *
   * `app/globals.css` imports the same package specifier the checkout's
   * does, through the same `@tailwindcss/postcss` pipeline
   * (`postcss.config.js` is byte-for-byte the same plugin list in both
   * apps). The checkout's own doc comment records that its build defined
   * `--color-base-100` **0** times without the alias. Reproducing that
   * exact test against THIS app — same method, cache cleared
   * (`rm -rf storybook-static node_modules/.cache`), alias temporarily
   * deleted from a copy of this file, `pnpm run build-storybook` rerun —
   * the built stylesheet still defined `--color-base-100:` **3** times and
   * referenced `var(--color-base-100)` **22** times, identically to the
   * build with the alias present. The theme is not dropped here either way.
   *
   * That result was surprising enough to double-check the checkout's own
   * claim too, with the identical method against its own tree: removing
   * ITS alias and rebuilding also still defined the variable 3 times. So as
   * measured today, on this dependency tree, neither app's build currently
   * reproduces the PR #135 failure with the alias absent — which may mean a
   * `vite`/`@tailwindcss/postcss`/`@vaam-apps/ui` version between then and
   * now already fixed the underlying resolution, or that some other
   * variable in the original repro no longer holds. Fixing or re-verifying
   * the checkout's own comment is outside this app's scope; flagged in this
   * task's report rather than edited there.
   *
   * The alias is left OUT of this file because it is dead code against what
   * was actually measured, not because the theme was assumed to be safe.
   * `src/a11y-gate.test.ts`'s "the BUILT storybook stylesheet actually
   * defines the theme" case is what keeps this honest going forward: if a
   * future dependency bump reintroduces the drop, that test fails against
   * the real built artefact regardless of which explanation above is right.
   */
  viteFinal(viteConfig) {
    /**
     * **The JSX runtime, set HERE so both pipelines share one setting** —
     * same reasoning as the checkout's `main.ts`. This app's `tsconfig.json`
     * also says `"jsx": "preserve"` because Next requires it, and Vite's
     * esbuild reads the tsconfig nearest the file it transforms — so without
     * this, every story that imports no `React` symbol dies at render with
     * `ReferenceError: React is not defined`, and `build-storybook` still
     * exits 0 because compiling is not rendering. `main.ts` is read by
     * `build-storybook` and by `addon-vitest` through `configDir`, so one
     * setting here covers both.
     */
    viteConfig.esbuild = { ...(viteConfig.esbuild || {}), jsx: "automatic" };
    /**
     * **`next/link`, which the checkout's Storybook never had to load.**
     * `PaymentsTable` and `PaymentsPager` render `next/link` anchors (see
     * each component's own doc for why — an href-based pager and a
     * prefetching row link, not `@vaam-apps/ui`'s callback-driven
     * primitives). Measured: with no `define` entries, `pnpm --filter
     * @vpay/dashboard test-storybook` fails the WHOLE suite at import time —
     * `ReferenceError: process is not defined` inside
     * `next/dist/client/has-base-path.js`'s
     * `process.env.__NEXT_ROUTER_BASEPATH`. Next's own webpack build
     * replaces every `process.env.__NEXT_*` flag with a literal via
     * `DefinePlugin`; nothing does that for Vite. `build-storybook` does
     * NOT catch this — it bundles the reference without executing it, and
     * only a real browser loading the module (what `test-storybook` does)
     * throws. Values match this app's own `next.config.ts`, which sets
     * neither a `basePath` nor touch-start suppression.
     */
    viteConfig.define = {
      ...(viteConfig.define || {}),
      "process.env.__NEXT_ROUTER_BASEPATH": JSON.stringify(""),
      "process.env.__NEXT_LINK_NO_TOUCH_START": "undefined",
    };
    viteConfig.resolve ??= {};
    const alias = viteConfig.resolve.alias;
    const entries = [
      /**
       * **The router problem — the main technical risk named in this app's
       * build brief.** `PaymentsFilters` calls `useRouter()` and `AppShell`
       * calls `usePathname()`; outside a mounted Next app router both throw
       * `invariant expected app router to be mounted` the moment they
       * render, and this app is on `@storybook/react-vite`, not
       * `@storybook/nextjs-vite` (which ships a router mock). Measured: a
       * story rendering either component with no alias present throws
       * exactly that invariant in the browser console and axe never runs.
       * `vi.mock`, which `src/a11y.test.tsx` and the components' own
       * `*.test.tsx` files use for this, has no effect on the module graph
       * `addon-vitest`'s browser-mode run or `build-storybook` resolve — so
       * the fix has to live at the resolver, not in a test setup file.
       *
       * **Chosen: a vite alias for the bare specifier `next/navigation` to a
       * hand-written stub** (`./next-navigation-mock.ts`) over an
       * `AppRouterContext.Provider` decorator, because the context that
       * would satisfy these hooks lives at
       * `next/dist/shared/lib/app-router-context.shared-runtime` — an
       * unexported path with no contract to hold it stable across a Next
       * upgrade — for a return of exactly the two hooks this app calls.
       * The stub's own doc comment records exactly what it fixes and does
       * not attempt (no routing simulation, a fixed
       * `usePathname() === "/payments"`).
       */
      {
        find: "next/navigation",
        replacement: fileURLToPath(
          new URL("./next-navigation-mock.ts", import.meta.url),
        ),
      },
    ];
    viteConfig.resolve.alias = Array.isArray(alias)
      ? [...alias, ...entries]
      : {
          ...(alias ?? {}),
          ...Object.fromEntries(entries.map((e) => [e.find, e.replacement])),
        };
    return viteConfig;
  },
};

export default config;
