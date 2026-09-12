import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  stories: [
    "../src/**/*.stories.@(ts|tsx)",
    // The checkout app's screens. They live in `frontends/apps/checkout`
    // because they are that app's own components, not shared primitives —
    // but the a11y addon, the daisyUI theme and CI's
    // `pnpm --filter @vpay/ui build-storybook` step are all configured here,
    // and a second Storybook installation would mean a second set of
    // accessibility settings for the one surface a payer actually sees.
    "../../../apps/checkout/src/**/*.stories.@(ts|tsx)",
  ],
  // `@storybook/addon-essentials` does not exist past 8.6.14 (plan §1, §6.8);
  // `addon-docs` replaces its docs half, and its controls/actions/viewport
  // panels are core Storybook 9+ features rather than addons.
  //
  // `@storybook/addon-vitest` is what lets `vitest.storybook.config.ts`
  // render these stories in a real browser; `addon-a11y`'s `afterEach` then
  // fails the test on a violation. Without this entry the vitest plugin
  // still builds a suite, but the addon annotations it needs are absent and
  // the run asserts nothing.
  addons: [
    "@storybook/addon-a11y",
    "@storybook/addon-docs",
    "@storybook/addon-vitest",
  ],
  framework: { name: "@storybook/react-vite", options: {} },
  docs: { autodocs: "tag" },
};

export default config;
