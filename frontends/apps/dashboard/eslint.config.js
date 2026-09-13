import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  react: true,
  next: true,
  // This app moved to Tailwind 4 in exp26 Lane D. `tailwind` is not folded
  // into `react` because the plugin compiles the linted package's own
  // `tailwindcss` to learn the class universe and aborts outright on
  // Tailwind 3 — see `tailwind` in @vpay/config/src/eslint.js. Enables
  // `enforce-consistent-line-wrapping` (the maintainer's "no `className`
  // longer than one line"), `no-unknown-classes` (catches a daisyUI 4 class
  // daisyUI 5 removed, independently of `just verify-ui`'s grep),
  // `enforce-consistent-class-order`, `enforce-consistent-variable-syntax`,
  // `no-conflicting-classes` and `no-duplicate-classes`.
  tailwind: true,
  // This app's tsconfig includes `**/*.ts`, so its root-level tooling files
  // are in the program and stay type-aware.
  // `.storybook/` is inside this app's tsconfig `include` by the letter of
  // `**/*.ts`, but TypeScript's include-glob expansion skips dot-directories
  // — measured, not assumed, the same way the checkout's identical comment
  // was: `tsc -p tsconfig.json --listFiles | grep -F '/.storybook/'` returns
  // 0 lines against a program of 1713 files. So `main.ts` and `preview.ts`
  // are linted for syntax and style but have no program behind them, and the
  // type-aware rules are absent by design here too, exactly the gap
  // `frontends/apps/checkout/eslint.config.js` already names.
  outsideTsconfig: ["*.config.js", ".storybook/**"],
});
