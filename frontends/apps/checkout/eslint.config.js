import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  react: true,
  next: true,
  // This app moved onto Tailwind 4 (exp26 Lane B, 2026-09-07) in the same
  // commit as this flag: `eslint-plugin-better-tailwindcss` aborts outright
  // against a Tailwind 3 package, so the two must land together.
  tailwind: true,
  forbidTestingImports: true,
  // This app's tsconfig includes `**/*.ts`, so its root-level tooling files
  // are in the program and stay type-aware.
  outsideTsconfig: ["*.config.js"],
});
