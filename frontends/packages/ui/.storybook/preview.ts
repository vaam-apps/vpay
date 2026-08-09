import type { Preview } from '@storybook/react';

import '../src/styles.css';

const preview: Preview = {
  parameters: {
    controls: { matchers: { color: /(background|color)$/i, date: /Date$/i } },
    // Both daisyUI themes are exercised, so contrast is checked in both.
    backgrounds: { disable: true },
    a11y: { config: { rules: [{ id: 'color-contrast', enabled: true }] } },
  },
  globalTypes: {
    theme: {
      description: 'daisyUI theme',
      defaultValue: 'corporate',
      toolbar: { items: ['corporate', 'business'], dynamicTitle: true },
    },
  },
  decorators: [
    (Story, ctx) => {
      document.documentElement.setAttribute('data-theme', ctx.globals.theme as string);
      return Story();
    },
  ],
};

export default preview;
