import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  react: true,
  // The only package on Tailwind 4 today; the three apps follow in lanes
  // B, C and D. See `tailwind` in @vpay/config/src/eslint.js.
  tailwind: true,
  // `.storybook/` is named in this package's tsconfig `include`, but
  // TypeScript's include-glob expansion skips dot-directories, so `tsc -p
  // tsconfig.json --listFiles` lists neither `main.ts` nor `preview.ts`.
  // They are covered by no typecheck today (recorded in the notes for this
  // pass); ESLint still lints them, minus the rules that need a program.
  outsideTsconfig: [
    "*.config.ts",
    "*.config.js",
    "*.setup.ts",
    ".storybook/**",
  ],
});
