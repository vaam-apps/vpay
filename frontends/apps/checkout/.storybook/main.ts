import { createRequire } from "node:module";

import type { StorybookConfig } from "@storybook/react-vite";

const require_ = createRequire(import.meta.url);

/**
 * The checkout's own Storybook.
 *
 * It lived in `frontends/packages/ui` until 2026-09-12, when that package was
 * deleted in the `@vaam-apps/ui` cutover and took this install, the 22 stories
 * and the a11y addon with it — a named, accepted gap with a filed task, which
 * this is. There is no shared package to host it in any more, so it lives in
 * the app whose screens it renders, which is where the subjects were all
 * along.
 *
 * `@storybook/addon-vitest` is what lets `vitest.storybook.config.ts` render
 * these stories in a real browser; `addon-a11y`'s own `afterEach` then fails
 * the test on an accessibility violation. Without this entry the vitest plugin
 * still builds a suite, but the addon annotations it needs are absent and the
 * run asserts nothing.
 *
 * `@storybook/addon-essentials` is deliberately absent: it does not exist past
 * 8.6.14. `addon-docs` replaces its docs half and the controls/actions/viewport
 * panels are core Storybook 9+ features.
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
   * **Resolve `@vaam-apps/ui/styles/theme.css` by hand, because vite drops
   * it silently.** `app/globals.css` imports that specifier, and the package
   * exposes it only through `exports` (`./styles/theme.css` ->
   * `./dist/styles/theme.css`). `@tailwindcss/postcss` honours that map —
   * `src/styling-gate.test.ts` compiles the same file and passes — but
   * Storybook's vite pipeline resolves CSS `@import` itself and emitted a
   * stylesheet with **no theme in it at all**: every `var(--color-base-100)`
   * present and `--color-base-100` defined nowhere.
   *
   * The failure is silent in exactly the way `@vaam-apps/ui`'s README warns
   * about. Every story rendered unstyled on the browser's default white
   * while the shipped page is `#0a0b0d`, axe measured every foreground
   * against the wrong ground, and the suite reported 22 passing stories —
   * with a deliberately unreadable `#3a3a3a` probe passing alongside them.
   *
   * The alias is computed from the package's own `exports` rather than
   * written as a path, so a version that moves the file breaks the build
   * instead of quietly rendering unstyled again.
   * `src/a11y-gate.test.ts` asserts the theme actually lands in the built
   * stylesheet, which is the check that would have caught this on day one.
   */
  viteFinal(config) {
    /**
     * **The JSX runtime, set HERE so both pipelines share one setting.**
     *
     * This app's `tsconfig.json` says `"jsx": "preserve"` because Next
     * requires it, and Vite's esbuild reads the tsconfig nearest the file it
     * transforms — so it emits classic `React.createElement` for stories
     * that import no React, and every one dies at render with
     * `ReferenceError: React is not defined`.
     *
     * It was set in `vitest.storybook.config.ts` alone for one revision, and
     * that is exactly long enough to learn why it belongs here: the vitest
     * suite went green on all 22 stories while `just build-storybook` — the
     * OTHER consumer of this config, and the one a human opens — produced a
     * Storybook in which every single story rendered the red
     * "React is not defined" panel. The build exits 0 either way, because
     * compiling is not rendering. Found by opening the built artefact and
     * looking at it, which is the only thing that could have found it.
     *
     * `main.ts` is read by `build-storybook` and by `addon-vitest` through
     * `configDir`, so one setting here covers both and there is no second
     * place for them to drift apart.
     */
    config.esbuild = { ...(config.esbuild || {}), jsx: "automatic" };
    config.resolve ??= {};
    const alias = config.resolve.alias;
    const entry = {
      find: "@vaam-apps/ui/styles/theme.css",
      replacement: require_.resolve("@vaam-apps/ui/styles/theme.css"),
    };
    config.resolve.alias = Array.isArray(alias)
      ? [...alias, entry]
      : { ...(alias ?? {}), [entry.find]: entry.replacement };
    return config;
  },
};

export default config;
