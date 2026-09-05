/**
 * The repository's one ESLint flat configuration.
 *
 * Why it lives here: `@vpay/config` is already the home of the shared
 * tsconfig/tailwind settings, and `package.json` has declared a `./eslint`
 * export pointing at this exact path since the package was created — at a
 * file that did not exist. Every workspace package's `eslint.config.js` is a
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
import js from "@eslint/js";
import nextPlugin from "@next/eslint-plugin-next";
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
 */

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
