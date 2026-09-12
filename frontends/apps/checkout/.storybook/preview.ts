import type { Preview } from "@storybook/react-vite";

import { THEME } from "../src/config/theme";
import "../app/globals.css";

/**
 * The app's own stylesheet, not a copy of it.
 *
 * `../app/globals.css` is the exact file `app/layout.tsx` imports, so a story
 * is reviewed under the CSS the real page ships. That matters more than it
 * looks: the file carries two traps this repository measured during the
 * `@vaam-apps/ui` cutover and which both fail SILENTLY —
 * `@plugin "daisyui" { themes: false }` (without it daisyUI's built-in `dark`
 * outranks the package theme on specificity) and the `@source` line pointing
 * at `@vaam-apps/ui/dist` (without it Tailwind generates none of the
 * package's utilities and every component renders unstyled, with no error).
 * Importing the app's entry rather than restating it is what keeps a story
 * from being reviewed under a third, wrong stylesheet.
 *
 * **The decorator reproduces `app/layout.tsx`'s document shell, and that is
 * load-bearing rather than cosmetic.** The shipped page is
 * `<html data-theme={THEME}><body class="min-h-screen bg-base-100">`. The
 * `bg-base-100` is what actually PAINTS the background; the theme only
 * defines `--color-base-100` as a variable. Without it a story renders on
 * the browser's default white while the real page is `#0a0b0d` — and axe
 * then measures every foreground against the wrong ground and reports a
 * verdict that is confidently wrong. Measured: the first run of this suite
 * passed 22 stories against white, and a deliberately dark-grey probe
 * (`#3a3a3a`, unreadable on this theme) passed with it.
 *
 * `THEME` is imported from `src/config/theme.ts` rather than spelled here,
 * so this cannot drift from what the layout sets. `src/config/theme.ts` also
 * layers an operator's `branding.yaml` colour on top at runtime; that is a
 * deployment's business and no story asserts it.
 */
const preview: Preview = {
  parameters: {
    controls: { matchers: { color: /(background|color)$/i, date: /Date$/i } },
    backgrounds: { disable: true },
    // `test: "error"` is what makes a violation FAIL `pnpm --filter
    // @vpay/checkout test-storybook` rather than merely colour a panel. It is
    // the addon's own default when a story says nothing, spelled out here so
    // that turning the gate off is an edit somebody has to make on purpose.
    // A single story may override it with `test: "todo"` (report, do not
    // fail) in its own source, where a reviewer sees it — and
    // `src/a11y-gate.test.ts` fails if one does so without a measured reason.
    a11y: {
      test: "error",
      config: { rules: [{ id: "color-contrast", enabled: true }] },
    },
  },
  decorators: [
    (Story) => {
      document.documentElement.setAttribute("data-theme", THEME);
      document.body.classList.add("min-h-screen", "bg-base-100");
      return Story();
    },
  ],
};

export default preview;
