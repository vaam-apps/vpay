import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  react: true,
  // The class-string rules (`eslint-plugin-better-tailwindcss`): one line per
  // class attribute, a deterministic class order, no unknown class, no
  // conflicting or duplicate class. Turned on in the commit that puts this
  // package on Tailwind 4, which is what the rules need to compile a class
  // universe from (`@vpay/config` aborts ESLint outright in a Tailwind 3
  // package).
  //
  // It reads `@vpay/ui`'s `styles.css` as that entry point, and that does not
  // reopen D1: D1 is about what the shop SHIPS — a merchant copying this
  // example must be able to reproduce it from public npm packages, and
  // nothing in `src/` imports `@vpay/ui` or `@vpay/tokens`. This is a
  // dev-only lint preset in a file a merchant would replace with their own,
  // in the same way `@vpay/config` already supplies this package's ESLint
  // rules and has since before the revamp.
  tailwind: true,
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
