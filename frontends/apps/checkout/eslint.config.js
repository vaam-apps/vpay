import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  react: true,
  next: true,
  forbidTestingImports: true,
  // This app's tsconfig includes `**/*.ts`, so its root-level tooling files
  // are in the program and stay type-aware.
  outsideTsconfig: ["*.config.js"],
});
