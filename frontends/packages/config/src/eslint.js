/**
 * The repository's one ESLint flat configuration.
 *
 * Why it lives here: `package.json` has declared a `./eslint` export
 * pointing at this exact path since the package was created — at a file
 * that did not exist. `@vpay/config`'s own header used to also claim to be
 * "the home of the shared tsconfig/tailwind settings"; that sentence was
 * never true (there was no shared tsconfig or Tailwind config here, only
 * this file), and under Tailwind 4 — which has no `tailwind.config.ts` at
 * all — the natural home for shared Tailwind/daisyUI settings is the CSS
 * entry point in `@vpay/ui` (`frontends/packages/ui/src/styles.css`), not a
 * config package. Corrected 2026-09-07 (exp26 UI revamp) rather than
 * inventing a package to make the old sentence true. Every workspace
 * package's `eslint.config.js` is a
 * three-line call into `vpayEslintConfig` below, so a rule is added in one
 * place or not at all.
 *
 * Why a factory rather than a flat array: ESLint's `projectService` needs the
 * *consuming* package's directory (`tsconfigRootDir`) to find its tsconfig,
 * and that is not knowable from here. The remaining options exist because
 * three facts genuinely differ per package — whether it renders React,
 * whether it is a Next app, and which of its files are scripts rather than
 * shipping source.
 *
 * The rule set is deliberately small and every rule in it is one this
 * repository would actually want to fail a build on. Nothing here is enabled
 * to make a number look bigger.
 *
 * @module
 */
import { fileURLToPath } from "node:url";

import js from "@eslint/js";
import nextPlugin from "@next/eslint-plugin-next";
import betterTailwind from "eslint-plugin-better-tailwindcss";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

/**
 * Build output and generated files. None of it is authored, so linting it
 * reports on a code generator rather than on this repository.
 *
 * `next-env.d.ts` is written by `next build` and is regenerated on every run.
 */
const GENERATED = [
  "**/node_modules/**",
  "**/dist/**",
  "**/.next/**",
  "**/storybook-static/**",
  "**/coverage/**",
  "**/next-env.d.ts",
];

/**
 * Files that are not shipping source: the test suites, the Storybook stories,
 * the `testing/` helpers those two import, the Cypress specs, and the build
 * and test tooling at a package root.
 *
 * This is the list `no-console` and the `testing/` import ban are switched
 * *off* for. It is not a list of files that go unlinted — every one of them
 * is still linted by everything else.
 */
const NOT_SHIPPING_SOURCE = [
  "**/*.test.ts",
  "**/*.test.tsx",
  // `.test.js` too: `@vpay/config`'s own suite is plain JavaScript, because
  // the config it tests is (nothing builds this package).
  "**/*.test.js",
  "**/*.test.mjs",
  "**/*.stories.ts",
  "**/*.stories.tsx",
  "**/*.cy.ts",
  "**/testing/**",
  "cypress/**",
  "scripts/**",
  ".storybook/**",
  "**/*.config.ts",
  "**/*.config.js",
  "**/*.config.mjs",
  "**/*.config.cjs",
  "**/*.setup.ts",
];

/**
 * Tooling files at a package root that the package's own tsconfig does not
 * `include`. `projectService` reports a hard error on a file it cannot place
 * in a program, so type-aware rules are turned off for exactly these rather
 * than for a whole directory. Packages whose tsconfig *does* cover them pass
 * their own (usually empty) list.
 */
const DEFAULT_OUTSIDE_TSCONFIG = [
  "*.config.ts",
  "*.config.js",
  "*.config.mjs",
  "*.config.cjs",
  "*.setup.ts",
];

/**
 * `src/testing/**` is where a package keeps the stubs its own unit tests run
 * against. AGENTS.md's first rule is that no test double is reachable from a
 * shipping process; `frontends/apps/checkout` and `examples/shop` each carry
 * a hand-written vitest guard that reads their shipping files and asserts the
 * string never appears. This is the second lock on the same door, and it
 * fails at lint time rather than at test time.
 */
const TESTING_IMPORT_PATTERNS = [
  "**/testing",
  "**/testing/*",
  "**/testing/**",
  "@/testing",
  "@/testing/*",
  "@/testing/**",
];

/**
 * @typedef {object} VpayEslintOptions
 * @property {string} tsconfigRootDir Absolute path of the package being
 *   linted — pass `import.meta.dirname`. `projectService` resolves the
 *   package's `tsconfig.json` from it.
 * @property {boolean} [react] Enable the react-hooks rules. On for anything
 *   that renders components.
 * @property {boolean} [next] Enable `@next/eslint-plugin-next`'s recommended
 *   and core-web-vitals rules. On for the Next apps only.
 * @property {boolean} [forbidTestingImports] Refuse an import of
 *   `testing/**` from a shipping file. On where a hand-written guard already
 *   asserts the same thing.
 * @property {string[]} [scripts] Globs, relative to the package, that are
 *   command-line scripts rather than shipping source: they may print.
 * @property {string[]} [browser] Globs of plain `.js`/`.mjs` files that run
 *   in a browser rather than in Node, so `no-undef` knows which globals exist.
 * @property {string[]} [outsideTsconfig] Globs the package's tsconfig does
 *   not `include`; type-aware rules are switched off for them. Defaults to
 *   the root-level tooling files most packages leave out.
 * @property {string[]} [ignores] Extra paths to skip entirely.
 * @property {boolean} [tailwind] Enable the class-string rules
 *   (`eslint-plugin-better-tailwindcss`). Requires the package to resolve
 *   `tailwindcss` **4** — the plugin loads the Tailwind entry point below to
 *   learn which classes exist, and throws outright in a package that only has
 *   Tailwind 3. Off by default for exactly that reason; each app turns it on
 *   in the commit that migrates it.
 */

/**
 * The one Tailwind 4 entry point, `@vpay/ui`'s `styles.css`.
 *
 * `eslint-plugin-better-tailwindcss` compiles it to learn the class universe
 * — which is what lets `no-unknown-classes` know that `btn-primary` exists
 * and that the classes daisyUI 5 removed do not. Resolved from this file's
 * own URL rather
 * than from the linted package, because the path from here is fixed while the
 * path from a consumer is not, and `@vpay/config` cannot depend on `@vpay/ui`
 * without a cycle.
 */
const TAILWIND_ENTRY_POINT = fileURLToPath(
  new URL("../../ui/src/styles.css", import.meta.url),
);

/**
 * Build the flat config for one workspace package.
 *
 * @param {VpayEslintOptions} options
 * @returns {import("eslint").Linter.Config[]}
 */
export function vpayEslintConfig(options) {
  const {
    tsconfigRootDir,
    react = false,
    next = false,
    forbidTestingImports = false,
    tailwind = false,
    scripts = [],
    browser = [],
    outsideTsconfig = DEFAULT_OUTSIDE_TSCONFIG,
    ignores = [],
  } = options;

  if (typeof tsconfigRootDir !== "string" || tsconfigRootDir.length === 0) {
    // A silently-wrong root would leave every type-aware rule unable to find
    // a program, which ESLint reports as a parse error per file rather than
    // as the configuration mistake it is.
    throw new TypeError(
      "vpayEslintConfig: tsconfigRootDir is required — pass import.meta.dirname",
    );
  }

  /** Everything `no-console` and the import ban do not apply to. */
  const exempt = [...NOT_SHIPPING_SOURCE, ...scripts];

  return [
    { ignores: [...GENERATED, ...ignores] },

    // ---- The base ESLint rules, on **every** file, TypeScript included.
    //
    // `js.configs.recommended` must be spread before the typescript-eslint
    // blocks below, because `typescript-eslint/eslint-recommended` — which
    // those blocks carry — switches off the 23 base rules the compiler
    // already covers (`no-undef`, `no-redeclare`, `no-dupe-class-members`
    // …) on `.ts`/`.tsx`. Scoping this block to `.js` alone, as an earlier
    // version of this file did, left the other 42 (`no-fallthrough`,
    // `no-debugger`, `no-unsafe-optional-chaining`,
    // `no-constant-binary-expression`, `no-async-promise-executor`,
    // `no-sparse-arrays`, `no-useless-escape`, `use-isnan` …) reporting
    // nothing over the 207 TypeScript files that are almost all of this
    // repository's source — 48 active rules on a `.ts` file rather than 90,
    // measured with `eslint --print-config`.
    {
      files: [
        "**/*.js",
        "**/*.jsx",
        "**/*.mjs",
        "**/*.cjs",
        "**/*.ts",
        "**/*.tsx",
        "**/*.mts",
        "**/*.cts",
      ],
      ...js.configs.recommended,
    },

    // ---- JavaScript: the examples' `.mjs` entry points and `checkout.js`.
    {
      files: ["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"],
      languageOptions: {
        ecmaVersion: 2023,
        sourceType: "module",
        globals: globals.node,
      },
    },
    ...(browser.length > 0
      ? [
          {
            files: browser,
            languageOptions: { globals: globals.browser },
          },
        ]
      : []),

    // ---- TypeScript, type-aware. `projectService` reads the package's own
    // tsconfig, so `strict`, `noUncheckedIndexedAccess` and
    // `exactOptionalPropertyTypes` from `tsconfig.base.json` are the types
    // these rules reason about.
    ...tseslint.configs.recommendedTypeChecked.map((config) => ({
      ...config,
      files: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"],
    })),
    {
      files: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"],
      languageOptions: {
        parserOptions: { projectService: true, tsconfigRootDir },
      },
    },

    // ---- The tooling files no tsconfig claims. Still linted; just not by
    // the rules that need a type checker.
    ...(outsideTsconfig.length > 0
      ? [
          {
            files: outsideTsconfig,
            ...tseslint.configs.disableTypeChecked,
          },
        ]
      : []),

    // ---- React.
    ...(react
      ? [
          {
            files: ["**/*.ts", "**/*.tsx"],
            // `configs.flat[...]`, not `configs[...]`: in v7 the top-level
            // entries are still eslintrc-shaped (`plugins` is an array of
            // strings) and ESLint 9 refuses them outright.
            ...reactHooks.configs.flat["recommended-latest"],
          },
        ]
      : []),

    // ---- The class-string rules (exp26 UI revamp, plan §7 "The class-string
    // rules, concretely"; §3's "what elegant means here, as rules a gate can
    // read"). The plugin was added to this package's dependencies by the
    // implementing pass and never wired into a config, so none of these rules
    // ran and the plan's own mutation — "a two-line `className` string is
    // added → fail `just lint-web`" — passed. Measured before this block
    // landed: `eslint --print-config` on a component reported 0 rules whose
    // name contains "tailwind", and a deliberately six-line `className` in
    // `badge.tsx` left `pnpm --filter @vpay/ui lint` at exit 0.
    //
    // On `tailwind`, not on `react`: the plugin compiles TAILWIND_ENTRY_POINT
    // through the linted package's own `tailwindcss`, so it aborts ESLint
    // entirely in `@vpay/checkout` and `@vpay/dashboard`, which are still on
    // Tailwind 3 until lanes B and D migrate them. Measured, not assumed —
    // enabling it for every React package fails both apps' `lint` at
    // "@import 'tailwindcss'". Each app flips this flag in the same commit
    // that moves it to Tailwind 4.
    //
    // `enforce-consistent-line-wrapping` is the maintainer's "no `className`
    // longer than one line", expressed in the two directions the rule has:
    // `preferSingleLine` makes an unnecessarily wrapped string an error, and
    // `printWidth` makes a string that does not fit one line an error. The
    // remedy for the second is to shorten the string — to reach for a `cva`
    // variant or a layout primitive — not to accept the autofix's wrap.
    ...(tailwind
      ? [
          {
            files: ["**/*.tsx", "**/*.jsx"],
            plugins: { "better-tailwindcss": betterTailwind },
            settings: {
              "better-tailwindcss": { entryPoint: TAILWIND_ENTRY_POINT },
            },
            rules: {
              "better-tailwindcss/enforce-consistent-line-wrapping": [
                "error",
                { group: "never", preferSingleLine: true, printWidth: 100 },
              ],
              // A deterministic order, so a diff shows a changed class rather
              // than a reshuffle (plan §7).
              "better-tailwindcss/enforce-consistent-class-order": "error",
              // Plan §7 names this `no-unregistered-classes`; that is the
              // rule's name in an earlier major. Under the pinned 4.7.0 it is
              // `no-unknown-classes`, and the plan is wrong on the name only.
              // This is the rule plan §6.3 says would have caught the daisyUI
              // 4 classes daisyUI 5 removed — `just verify-ui`'s check 2
              // names them; this comment deliberately does not, because that
              // grep reads comments too.
              "better-tailwindcss/no-unknown-classes": "error",
              // Caught a real defect on its first run: `w-[--anchor-width]`
              // in `select.tsx`, Tailwind 3 syntax that Tailwind 4 compiles
              // to an invalid declaration rather than rejecting. See the
              // commit before this one.
              "better-tailwindcss/enforce-consistent-variable-syntax": "error",
              "better-tailwindcss/no-conflicting-classes": "error",
              "better-tailwindcss/no-duplicate-classes": "error",
            },
          },
        ]
      : []),

    // ---- Next.
    ...(next
      ? [
          {
            files: ["**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx"],
            plugins: { "@next/next": nextPlugin },
            rules: {
              ...nextPlugin.configs.recommended.rules,
              ...nextPlugin.configs["core-web-vitals"].rules,
            },
          },
        ]
      : []),

    // ---- `_name` is this repository's mark for a binding that exists
    // because a signature or a destructuring demands it and is deliberately
    // not read — the same convention Rust uses, and what
    // `frontends/apps/checkout/src/lib/machine.ts` and
    // `sdks/nodejs/src/stripe-auth.test.ts` already spell. Configuring the
    // rule to honour it, rather than switching the rule off.
    {
      files: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"],
      rules: {
        "@typescript-eslint/no-unused-vars": [
          "error",
          {
            argsIgnorePattern: "^_",
            varsIgnorePattern: "^_",
            caughtErrorsIgnorePattern: "^_",
            destructuredArrayIgnorePattern: "^_",
          },
        ],
      },
    },

    // ---- The two rules this repository adds on its own account.
    //
    // `no-console` in shipping source: a payment page that prints is a
    // payment page that can print a `client_secret`. `frontends/apps/checkout`
    // already has a vitest credential trace asserting no `console.*` call
    // carries a secret; this stops the call being written at all. Tests,
    // stories, Cypress specs and command-line scripts print on purpose and
    // are exempt.
    {
      files: [
        "**/*.ts",
        "**/*.tsx",
        "**/*.mts",
        "**/*.cts",
        "**/*.js",
        "**/*.jsx",
        "**/*.mjs",
        "**/*.cjs",
      ],
      ignores: exempt,
      rules: { "no-console": "error" },
    },
    ...(forbidTestingImports
      ? [
          {
            files: ["**/*.ts", "**/*.tsx"],
            ignores: exempt,
            rules: {
              "@typescript-eslint/no-restricted-imports": [
                "error",
                {
                  patterns: [
                    {
                      group: TESTING_IMPORT_PATTERNS,
                      message:
                        "testing/** holds test doubles. AGENTS.md: no test double may be reachable from a shipping process.",
                    },
                  ],
                },
              ],
            },
          },
        ]
      : []),
  ];
}

export default vpayEslintConfig;
