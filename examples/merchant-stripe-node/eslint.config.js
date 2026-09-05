import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  // The whole package is one command-line example that prints what it did.
  scripts: ["index.mjs"],
});
