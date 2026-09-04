import { defineConfig } from 'vitest/config';

export default defineConfig({
  // The app's `tsconfig.json` sets `jsx: preserve` because Next does its own
  // JSX transform; esbuild reads that and would emit classic
  // `React.createElement` calls into the test bundle, where nothing imports
  // `React`. The runtime transform is stated here rather than by relaxing
  // the tsconfig Next relies on.
  esbuild: { jsx: 'automatic', jsxImportSource: 'react' },
  test: {
    // `node` by default so the tests that speak to the `node:http` stub in
    // `src/testing/browser-stub.ts` use the platform's own `fetch`. The
    // rendering tests opt in with a `// @vitest-environment jsdom` docblock.
    environment: 'node',
    globals: false,
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['./vitest.setup.ts'],
  },
});
