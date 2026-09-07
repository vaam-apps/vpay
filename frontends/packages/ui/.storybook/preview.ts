import type { Preview } from '@storybook/react-vite';

import '../src/styles.css';

/**
 * `bumblebee` is the only theme that ships (plan §4.2, §1) — the checkout
 * page and the dashboard both moved onto it in the same revamp that removed
 * `corporate`/`business`. There is nothing left to switch between; the
 * global remains only so a future second theme has one place to add a
 * toolbar entry, rather than reinventing the decorator.
 */
const THEME = 'bumblebee';

const preview: Preview = {
  parameters: {
    controls: { matchers: { color: /(background|color)$/i, date: /Date$/i } },
    backgrounds: { disable: true },
    a11y: { config: { rules: [{ id: 'color-contrast', enabled: true }] } },
  },
  decorators: [
    (Story) => {
      document.documentElement.setAttribute('data-theme', THEME);
      return Story();
    },
  ],
};

export default preview;
