/**
 * The gate on the gate.
 *
 * A lint gate is the one check that reports its own absence as success:
 * delete a rule from the shared config and every package still exits 0;
 * delete a package's `lint` script and `pnpm -r` skips it without a word;
 * drop `--max-warnings 0` and a `warn`-level finding exits 0; open a file
 * with a whole-file disable directive and nothing anywhere reports it.
 * CLAUDE.md's failure mode for this repository is a check that looks like it
 * is working, and `pnpm -r lint` was exactly that until 2026-09-05.
 *
 * Every assertion here was written against a measured mutation — each was
 * applied to the tree, watched to pass `pnpm -r lint` with no complaint, and
 * only then turned into a test. They are recorded in
 * `docs/plans/exp4-notes/opus-review.md`.
 *
 * @module
 */
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

import { ESLint } from "eslint";
import { describe, expect, it } from "vitest";

const REPO_ROOT = path.resolve(import.meta.dirname, "../../../..");

/** Every workspace package directory, from git rather than from a glob. */
const PACKAGE_DIRS = execFileSync(
  "git",
  ["ls-files", "--full-name", "*/package.json", "package.json"],
  { cwd: REPO_ROOT, encoding: "utf8" },
)
  .split("\n")
  .filter((line) => line.length > 0 && !line.includes("node_modules/"))
  .map((line) => path.dirname(line))
  .filter((dir) => dir !== ".")
  .sort();

/** Every authored source file, likewise from git. */
const SOURCE_FILES = execFileSync(
  "git",
  [
    "ls-files",
    "--full-name",
    "*.ts",
    "*.tsx",
    "*.js",
    "*.jsx",
    "*.mjs",
    "*.cjs",
  ],
  { cwd: REPO_ROOT, encoding: "utf8" },
)
  .split("\n")
  .filter((line) => line.length > 0 && !line.includes("node_modules/"));

/**
 * Resolve one file's effective rule set, as ESLint itself would compute it
 * when the package's own `lint` script runs.
 *
 * @param {string} relativePath repo-relative path of the file
 * @returns {Promise<Record<string, unknown>>} rule name -> severity/options
 */
async function rulesFor(relativePath) {
  const absolute = path.join(REPO_ROOT, relativePath);
  const packageDir = PACKAGE_DIRS.filter((dir) =>
    relativePath.startsWith(`${dir}/`),
  ).sort((a, b) => b.length - a.length)[0];
  const eslint = new ESLint({ cwd: path.join(REPO_ROOT, packageDir) });
  const config = await eslint.calculateConfigForFile(absolute);
  return config.rules ?? {};
}

/**
 * @param {Record<string, unknown>} rules
 * @param {string} name
 * @returns {boolean} whether the rule is on at any severity above "off"
 */
function isOn(rules, name) {
  const entry = rules[name];
  if (entry === undefined) {
    return false;
  }
  const severity = Array.isArray(entry) ? entry[0] : entry;
  return severity !== "off" && severity !== 0;
}

/**
 * `calculateConfigForFile` normalises severities to numbers, so "error" comes
 * back as 2.
 *
 * @param {Record<string, unknown>} rules
 * @param {string} name
 * @returns {boolean} whether the rule is on at error severity
 */
function isError(rules, name) {
  const entry = rules[name];
  const severity = Array.isArray(entry) ? entry[0] : entry;
  return severity === 2 || severity === "error";
}

describe("every workspace package is actually linted", () => {
  // Mutation that motivated this: delete `@vpay/tokens`'s `lint` script.
  // `pnpm -r lint` prints the same "Scope: 15 of 16 workspace projects" line
  // and exits 0 — pnpm skips a missing script without a word, which is the
  // mechanism by which ten of these packages went unlinted before this pass.
  it.each(PACKAGE_DIRS)("%s declares a lint script", (dir) => {
    const manifest = JSON.parse(
      readFileSync(path.join(REPO_ROOT, dir, "package.json"), "utf8"),
    );
    expect(manifest.scripts?.lint, `${dir} has no lint script`).toBeTypeOf(
      "string",
    );
  });

  // Mutation: rewrite `frontends/apps/dashboard`'s script as plain `eslint .`.
  // `@next/eslint-plugin-next`'s recommended set puts several rules at `warn`
  // (`no-img-element` among them), so an `<img>` in the dashboard's page then
  // exits 0 instead of 1 — measured both ways.
  it.each(PACKAGE_DIRS)("%s runs eslint with --max-warnings 0", (dir) => {
    const manifest = JSON.parse(
      readFileSync(path.join(REPO_ROOT, dir, "package.json"), "utf8"),
    );
    expect(
      manifest.scripts.lint,
      `${dir}: a warn-level rule would pass this gate`,
    ).toContain("eslint . --max-warnings 0");
  });

  // Mutation: replace `sdks/nodejs/eslint.config.js` with `export default [];`.
  // That package then lints zero rules and both `pnpm --filter @vaam-apps/vpay-sdk lint`
  // and `pnpm -r lint` exit 0.
  it.each(PACKAGE_DIRS)("%s uses the shared config", (dir) => {
    const configPath = path.join(REPO_ROOT, dir, "eslint.config.js");
    expect(existsSync(configPath), `${dir} has no eslint.config.js`).toBe(true);
    // `@vpay/config` is the one package that cannot name itself by its own
    // specifier; it reaches the factory by relative path.
    const specifier =
      dir === "frontends/packages/config"
        ? "./src/eslint.js"
        : "@vpay/config/eslint";
    expect(readFileSync(configPath, "utf8"), dir).toContain(specifier);
  });
});

describe("the shared config still carries the rules the gate claims", () => {
  // Mutation: scope `js.configs.recommended` to `**/*.js` only, as the first
  // version of this file did. 42 base rules — `no-fallthrough`, `no-debugger`,
  // `no-unsafe-optional-chaining`, `no-constant-binary-expression`,
  // `no-async-promise-executor`, `use-isnan` … — then report nothing over the
  // 207 TypeScript files that are almost all of this repository's source, and
  // `pnpm -r lint` still exits 0. A sample is asserted rather than all 42:
  // these are the ones the TypeScript compiler does NOT also catch.
  const BASE_RULES = [
    "no-async-promise-executor",
    "no-constant-binary-expression",
    "no-debugger",
    "no-fallthrough",
    "no-prototype-builtins",
    "no-sparse-arrays",
    "no-unsafe-finally",
    "no-unsafe-optional-chaining",
    "no-useless-escape",
    "use-isnan",
  ];

  it("applies the base ESLint rules to TypeScript, not only to .js", async () => {
    const rules = await rulesFor("sdks/nodejs/src/client.ts");
    for (const rule of BASE_RULES) {
      expect(isOn(rules, rule), `${rule} is off on a .ts file`).toBe(true);
    }
  });

  // Mutation: delete the `no-console` block. The three planted `console.log`
  // calls — a Next page, an SDK source file, the shop's server code — stop
  // being reported and `pnpm -r lint` exits 0.
  it.each([
    "sdks/nodejs/src/client.ts",
    "frontends/apps/dashboard/app/page.tsx",
    "examples/shop/src/server/context.ts",
    "frontends/apps/checkout/src/lib/money.ts",
    "examples/checkout-browser/checkout.js",
  ])("no-console is an error in %s", async (file) => {
    const rules = await rulesFor(file);
    expect(isError(rules, "no-console"), file).toBe(true);
  });

  it.each([
    "sdks/nodejs/src/client.test.ts",
    "examples/merchant-node/index.mjs",
    "frontends/tests/e2e/cypress/e2e/checkout.cy.ts",
  ])("no-console is off in %s, which prints on purpose", async (file) => {
    const rules = await rulesFor(file);
    expect(isOn(rules, "no-console")).toBe(false);
  });

  // Mutation: set `@typescript-eslint/no-restricted-imports` to "off". Both
  // `import "@/testing/memory-store"` in the shop's server context and
  // `import "../testing/fixtures"` in the checkout app's `entry.ts` stop being
  // reported. (The hand-written vitest guards in both packages still catch it
  // — that is the point of having two locks, and both are asserted here and
  // there.)
  it.each([
    "frontends/apps/checkout/src/lib/entry.ts",
    "examples/shop/src/server/context.ts",
  ])("the testing/** import ban is on in %s", async (file) => {
    const rules = await rulesFor(file);
    expect(isOn(rules, "@typescript-eslint/no-restricted-imports")).toBe(true);
  });

  // The rule that cannot fire without a real type checker: if `projectService`
  // ever stops resolving a package's tsconfig, this is the rule that goes
  // quiet, and with it every other type-aware rule in the set.
  it.each([
    "sdks/nodejs/src/client.ts",
    "frontends/apps/checkout/src/lib/api.ts",
    "examples/shop/src/server/orders.ts",
  ])("the type-aware rules are on in %s", async (file) => {
    const rules = await rulesFor(file);
    expect(isOn(rules, "@typescript-eslint/no-floating-promises")).toBe(true);
  });

  it("react-hooks is on in the React packages", async () => {
    for (const file of [
      "frontends/apps/checkout/src/components/checkout-client.tsx",
      "frontends/packages/ui/src/components/status-badge.tsx",
      "examples/shop/src/components/cart-table.tsx",
      "frontends/apps/dashboard/app/page.tsx",
    ]) {
      const rules = await rulesFor(file);
      expect(isOn(rules, "react-hooks/rules-of-hooks"), file).toBe(true);
    }
  });

  it("the Next plugin is on in the Next apps", async () => {
    for (const file of [
      "frontends/apps/checkout/app/layout.tsx",
      "frontends/apps/dashboard/app/page.tsx",
      "examples/shop/src/app/layout.tsx",
    ]) {
      const rules = await rulesFor(file);
      expect(isOn(rules, "@next/next/no-img-element"), file).toBe(true);
    }
  });
});

describe("suppressions stay visible", () => {
  const DISABLE_LINE = /(?:\/\/|\/\*|\*)\s*eslint-disable(-next-line|-line)?\b/;

  // This file quotes the directives in prose. Everything else in the tree is
  // scanned.
  const SELF = "frontends/packages/config/src/eslint.test.js";
  const SCANNED = SOURCE_FILES.filter((file) => file !== SELF);

  // The task brief forbids a blanket disable, and a reviewer of this pass
  // added `/* eslint-disable no-console */` to the top of the dashboard's
  // page with a `console.log` under it: `pnpm -r lint` exits 0 and nothing
  // anywhere reports it. ESLint's own `reportUnusedDisableDirectives` catches
  // a suppression that has become stale, but never one that is doing work.
  it("no file-scope eslint-disable exists", () => {
    const offenders = [];
    for (const file of SCANNED) {
      const text = readFileSync(path.join(REPO_ROOT, file), "utf8");
      if (/\/\*\s*eslint-disable(?![-a-z])/.test(text)) {
        offenders.push(file);
      }
    }
    expect(offenders, "disable the rule on the line, not on the file").toEqual(
      [],
    );
  });

  // Every one of the 21 suppressions this pass's implementation added carries
  // a prose reason on the line above it. Nothing enforced that, so the
  // twenty-second would not have needed one.
  it("every eslint-disable-next-line carries a reason above it", () => {
    const offenders = [];
    for (const file of SCANNED) {
      const lines = readFileSync(path.join(REPO_ROOT, file), "utf8").split(
        "\n",
      );
      lines.forEach((line, index) => {
        if (!DISABLE_LINE.test(line)) {
          return;
        }
        const above = (lines[index - 1] ?? "").trim();
        const isPlainComment =
          (above.startsWith("//") || above.startsWith("*")) &&
          !DISABLE_LINE.test(above);
        if (!isPlainComment && !line.includes("--")) {
          offenders.push(`${file}:${index + 1}`);
        }
      });
    }
    expect(offenders, "a suppression with no stated reason").toEqual([]);
  });
});
