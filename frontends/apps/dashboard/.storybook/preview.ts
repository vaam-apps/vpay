import type { Preview } from "@storybook/react-vite";

import "../app/globals.css";

/**
 * The app's own stylesheet, not a copy of it.
 *
 * `../app/globals.css` is the exact file `app/layout.tsx` imports, so a
 * story is reviewed under the CSS the real page ships — see that file's own
 * doc comment for the two silent-failure traps it already survived
 * (`themes: false`, the `@source` line, and `@import` ordering). Importing
 * the app's entry rather than restating it is what keeps a story from being
 * reviewed under a third, wrong stylesheet.
 *
 * # Two themes, which is a real difference from the checkout
 *
 * The checkout ships one theme. `@vaam-apps/ui` 0.1.2 registers `dark` as
 * its default and `light` as opt-in, `app/layout.tsx` pins
 * `data-theme="dark"`, and this app has a real `ThemeSwitcher`
 * (`src/components/more-menu.tsx`, `src/components/app-shell.tsx`). So this
 * file adds a toolbar **globalType** (`theme`, `dark`/`light`, defaulting to
 * `dark` — the value the shipped layout pins) and a decorator that writes
 * `data-theme` from it, rather than hard-coding one value the way the
 * checkout's `preview.ts` does from `src/config/theme.ts` (which this app
 * has no equivalent of — `app/layout.tsx` pins the attribute directly, with
 * no `branding.yaml` layer in front of it here).
 *
 * **`bg-base-100` is what actually PAINTS the background**; the theme only
 * defines `--color-base-100` as a variable. Without reproducing
 * `app/layout.tsx`'s `<body class="min-h-screen bg-base-100">` here, a story
 * renders on the browser's default white while the real page is dark, and
 * axe then measures every foreground against the wrong ground. This is the
 * exact failure `frontends/apps/checkout/src/a11y-gate.test.ts` names as
 * PR #135's finding, reproduced and checked again on this app's own build —
 * see `.storybook/main.ts`'s doc comment for the counts measured here.
 *
 * # Both themes and the a11y run — the answer is stated plainly, not implied
 *
 * `addon-a11y`'s `afterEach` runs axe over whatever `data-theme` the
 * decorator wrote for THAT render. The toolbar control lets a human flip
 * themes in the Storybook UI, but `vitest.storybook.config.ts` runs each
 * story exactly once, under whatever `globals.theme` resolves to at story
 * load — which is this file's default, `dark`, for every story that does
 * not set its own `globals.theme`. **So only `dark` is genuinely
 * axe-checked by `just test-storybook` today**; `light` is reviewable by a
 * human in the Storybook UI but is not covered by the automated gate, unless
 * a story explicitly sets `parameters.globals = { theme: "light" }` (see
 * `dashboard-screens.stories.tsx` for the small number that do, one per
 * screen family, to get at least one `light` data point into the automated
 * run rather than leaving the whole theme unchecked).
 */
const preview: Preview = {
  parameters: {
    controls: { matchers: { color: /(background|color)$/i, date: /Date$/i } },
    backgrounds: { disable: true },
    // `test: "error"` is what makes a violation FAIL
    // `pnpm --filter @vpay/dashboard test-storybook` rather than merely
    // colour a panel. It is the addon's own default when a story says
    // nothing, spelled out here so that turning the gate off is an edit
    // somebody has to make on purpose.
    a11y: {
      test: "error",
      config: { rules: [{ id: "color-contrast", enabled: true }] },
    },
  },
  globalTypes: {
    theme: {
      description: "the @vaam-apps/ui theme this story is reviewed under",
      toolbar: {
        title: "Theme",
        icon: "mirror",
        items: [
          { value: "dark", title: "Dark (shipped default)" },
          { value: "light", title: "Light (opt-in)" },
        ],
        dynamicTitle: true,
      },
    },
  },
  // `app/layout.tsx` pins `data-theme="dark"` with no fallback of its own,
  // so `dark` is this file's default too — a story reviewed with no
  // `globals.theme` set sees exactly what the shipped page ships.
  initialGlobals: { theme: "dark" },
  decorators: [
    (Story, context) => {
      const theme = context.globals.theme === "light" ? "light" : "dark";
      document.documentElement.setAttribute("data-theme", theme);
      document.body.classList.add("min-h-screen", "bg-base-100");
      return Story();
    },
  ],
};

export default preview;
