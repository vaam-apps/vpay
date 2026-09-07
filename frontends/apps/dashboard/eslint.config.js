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
  outsideTsconfig: ["*.config.js"],
});
