import type { Preview } from "@storybook/react-vite";

import "../src/styles.css";

/**
 * `bumblebee` is the only theme that ships (plan §4.2, §1) — the checkout
 * page and the dashboard both moved onto it in the same revamp that removed
 * `corporate`/`business`. There is nothing left to switch between; the
 * global remains only so a future second theme has one place to add a
 * toolbar entry, rather than reinventing the decorator.
 */
const THEME = "bumblebee";

const preview: Preview = {
  parameters: {
    controls: { matchers: { color: /(background|color)$/i, date: /Date$/i } },
    backgrounds: { disable: true },
    // `test: "error"` is what makes a violation fail `pnpm --filter
    // @vpay/ui test-storybook` rather than merely colour a panel. It is the
    // addon's own default when a story says nothing, and it is spelled out
    // here so that turning the gate off is an edit somebody has to make on
    // purpose. A single story may override it with `test: "todo"` (report,
    // do not fail) in its own source, where a reviewer sees it.
    a11y: {
      test: "error",
      config: { rules: [{ id: "color-contrast", enabled: true }] },
    },
  },
  decorators: [
    (Story) => {
      document.documentElement.setAttribute("data-theme", THEME);
      return Story();
    },
  ],
};

export default preview;
