import type { StorybookConfig } from '@storybook/react-vite';

const config: StorybookConfig = {
  stories: [
    '../src/**/*.stories.@(ts|tsx)',
    // The checkout app's screens. They live in `frontends/apps/checkout`
    // because they are that app's own components, not shared primitives —
    // but the a11y addon, the daisyUI theme and CI's
    // `pnpm --filter @vpay/ui build-storybook` step are all configured here,
    // and a second Storybook installation would mean a second set of
    // accessibility settings for the one surface a payer actually sees.
    '../../../apps/checkout/src/**/*.stories.@(ts|tsx)',
  ],
  // `@storybook/addon-essentials` does not exist past 8.6.14 (plan §1, §6.8);
  // `addon-docs` replaces its docs half, and its controls/actions/viewport
  // panels are core Storybook 9+ features rather than addons.
  addons: ['@storybook/addon-a11y', '@storybook/addon-docs'],
  framework: { name: '@storybook/react-vite', options: {} },
  docs: { autodocs: 'tag' },
};

export default config;
