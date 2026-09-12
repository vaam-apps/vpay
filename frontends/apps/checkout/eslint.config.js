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
  // `.storybook/` is inside this app's tsconfig `include` by the letter of
  // `**/*.ts`, but TypeScript's include-glob expansion skips dot-directories,
  // so `tsc --listFiles` lists neither `main.ts` nor `preview.ts` and the
  // type-aware rules have no program for them. Same gap `@vpay/ui` carried
  // and recorded before it was deleted; stated here rather than rediscovered.
  outsideTsconfig: ["*.config.js", ".storybook/**"],
});
