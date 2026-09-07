import { defineConfig } from 'vitest/config';

export default defineConfig({
  // The app's `tsconfig.json` sets `jsx: preserve` because Next does its own
  // JSX transform; esbuild reads that and would emit classic
  // `React.createElement` calls into the test bundle, where nothing imports
  // `React`. The runtime transform is stated here rather than by relaxing
  // the tsconfig Next relies on — the same fix `frontends/apps/checkout`
  // and `frontends/packages/ui` already carry.
  esbuild: { jsx: 'automatic', jsxImportSource: 'react' },
  test: {
    environment: 'jsdom',
    globals: false,
    // `app/` as well as `src/`. A `.test.ts` written beside a route matched
    // nothing here and ran nowhere — a test file that is silently not a test,
    // which is the class of thing this repository is careful about
    // everywhere else (exp28 review).
    include: ['{src,app}/**/*.test.{ts,tsx}'],
    setupFiles: ['./vitest.setup.ts'],
  },
});
