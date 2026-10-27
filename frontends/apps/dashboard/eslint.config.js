import { vpayEslintConfig } from "@vpay/config/eslint";

export default [
  ...vpayEslintConfig({
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
  }),
  // `@vpay/dash-client` is generated from `schemas/vpay.cstack` and carries a
  // class for every MODEL as well as the procedures. vpay mounts none of the
  // model routes, and `PaymentIntentApi.list()` would reach the hand-written
  // `GET /dash/v1/payment_intents` and cast its envelope to an array. No
  // module builds the client now (the signed reader is VP-6b's), and when one
  // does it is the only one to be exempted here; until then everything may
  // import the generated TYPES and nothing that runs.
  //
  // ESLint's own `no-restricted-imports`, NOT `@typescript-eslint/`'s: the
  // shared config already sets that one (the `testing/**` ban), and a flat
  // config entry setting the same rule REPLACES its options rather than
  // adding to them — so reusing it here would quietly lift that ban.
  {
    files: ["**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "@vpay/dash-client",
              allowTypeImports: true,
              message:
                "Only the generated client's types may be imported: no module runs it until VP-6b builds the signed reader.",
            },
          ],
        },
      ],
    },
  },
];
