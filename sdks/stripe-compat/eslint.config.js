import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  // `vitest.config.ts` is in this package's tsconfig `include`.
  outsideTsconfig: [],
});
