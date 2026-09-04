import type { StorybookConfig } from '@storybook/react-vite';

const config: StorybookConfig = {
  stories: [
    '../src/**/*.stories.@(ts|tsx)',
    // The checkout app's screens. They live in `frontends/apps/checkout`
    // because they are that app's own components, not shared primitives —
    // but the a11y addon, the two daisyUI themes and CI's
    // `pnpm --filter @vpay/ui build-storybook` step are all configured here,
    // and a second Storybook installation would mean a second set of
    // accessibility settings for the one surface a payer actually sees.
    '../../../apps/checkout/src/**/*.stories.@(ts|tsx)',
  ],
  addons: ['@storybook/addon-essentials', '@storybook/addon-a11y'],
  framework: { name: '@storybook/react-vite', options: {} },
  docs: { autodocs: 'tag' },
};

export default config;
