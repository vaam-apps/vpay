import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  react: true,
  next: true,
  forbidTestingImports: true,
  // This app's tsconfig includes `**/*.ts`, so its root-level tooling files
  // are in the program and stay type-aware.
  outsideTsconfig: [],
  // `zen generate`'s output. It is not committed (see the repository's
  // .gitignore), it carries a DO NOT MODIFY banner and a blanket suppression
  // of its own, and linting it only ever produced "unused directive"
  // warnings against a file nobody edits.
  ignores: ["zenstack/schema.ts", "zenstack/models.ts", "zenstack/input.ts"],
});
