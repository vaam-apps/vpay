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
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['./vitest.setup.ts'],
  },
});
