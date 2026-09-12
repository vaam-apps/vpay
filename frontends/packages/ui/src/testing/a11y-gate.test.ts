/**
 * Asserts that the browser a11y suite is still a gate.
 *
 * `pnpm --filter @vpay/ui test-storybook` needs a Chromium binary, so it
 * runs in CI's `web` job and not in `just ci` — which means the standard
 * gate everyone runs locally would never notice the suite being switched
 * off. This file is the lock on that: it is a plain jsdom test in the suite
 * `just test-web` (and so `just ci`) does run, and it reads the same three
 * files a reviewer would.
 *
 * It is written the way `frontends/packages/config/src/eslint.test.js` is
 * written, and for the same reason that file gives: a gate reports its own
 * absence as success. Every assertion below was checked against the
 * mutation that would otherwise pass silently — deleting `test: "error"`
 * from `preview.ts`, adding a fifth undocumented `test: "todo"`, and
 * dropping `@storybook/addon-vitest` from `main.ts`.
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * Walk up for the workspace marker rather than deriving paths from
 * `import.meta.url` — vitest rewrites that to a virtual specifier
 * (`/src/index.ts`) and every `readFileSync` below then reads from `/`.
 */
function repoRoot(): string {
  let dir = process.cwd();
  while (!existsSync(join(dir, "pnpm-workspace.yaml"))) {
    const parent = dirname(dir);
    if (parent === dir) throw new Error("no pnpm-workspace.yaml above cwd");
    dir = parent;
  }
  return dir;
}

const REPO_ROOT = repoRoot();
const UI_ROOT = join(REPO_ROOT, "frontends", "packages", "ui");
const CHECKOUT_SRC = join(REPO_ROOT, "frontends", "apps", "checkout", "src");

/**
 * Strip comments before matching. `preview.ts`'s own note explains the
 * `test: "todo"` escape hatch in prose, and the first version of this file
 * failed on its own documentation — the same distinction `verify-status`
 * draws when it lexes rather than greps: prose declares nothing.
 */
function code(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");
}

function storyFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      out.push(...storyFiles(full));
    } else if (entry.endsWith(".stories.tsx")) {
      out.push(full);
    }
  }
  return out;
}

/**
 * The four stories that are allowed to report instead of fail, each with
 * the measured ratio that earned it. Adding a row here is a deliberate,
 * reviewable act; adding a `test: "todo"` without one fails this file.
 */
const DECLARED_TODOS: Record<string, string> = {
  "field.stories.tsx": "2.92:1 — text-error #ff6266 on #ffffff",
  "text.stories.tsx": "2.92:1 — text-error #ff6266 on #ffffff",
  "timeline.stories.tsx": "2.71:1 — opacity-60 #9d9d9d on #ffffff",
  "checkout-screens.stories.tsx": "2.92:1 — text-error #ff6266 on #ffffff",
};

describe("the browser a11y suite is still a gate", () => {
  it("preview.ts still asks the addon to FAIL on a violation", () => {
    const preview = code(
      readFileSync(join(UI_ROOT, ".storybook/preview.ts"), "utf8"),
    );
    // `test: "todo"` or `"off"` here would turn every story into a warning
    // at once, and nothing else in the repository would notice.
    expect(preview).toMatch(/test:\s*"error"/);
    expect(preview).not.toMatch(/test:\s*"(off|todo)"/);
  });

  it("main.ts still loads the addon that runs the stories in a browser", () => {
    const main = code(
      readFileSync(join(UI_ROOT, ".storybook/main.ts"), "utf8"),
    );
    expect(main).toContain("@storybook/addon-vitest");
    expect(main).toContain("@storybook/addon-a11y");
  });

  it("no story opts out of the gate without a declared, measured reason", () => {
    const files = [
      ...storyFiles(join(UI_ROOT, "src")),
      ...storyFiles(CHECKOUT_SRC),
    ];
    // Every story file is reachable, so a renamed directory fails here
    // rather than quietly shrinking the set this test walks.
    expect(files.length).toBeGreaterThanOrEqual(30);

    const optedOut = files
      .filter((f) =>
        /a11y:\s*\{\s*test:\s*"todo"\s*\}/.test(code(readFileSync(f, "utf8"))),
      )
      .map((f) => f.split("/").pop() as string)
      .sort();

    expect(optedOut).toEqual(Object.keys(DECLARED_TODOS).sort());
  });

  it("each opted-out story states the ratio it was measured at", () => {
    for (const [file, evidence] of Object.entries(DECLARED_TODOS)) {
      const match = storyFiles(join(UI_ROOT, "src"))
        .concat(storyFiles(CHECKOUT_SRC))
        .find((f) => f.endsWith(file));
      expect(match, `${file} not found`).toBeDefined();
      const source = readFileSync(match as string, "utf8");
      const ratio = evidence.split(" ")[0] as string;
      // The number in the comment, not a vague "known issue" — so a fix
      // that moves the colour makes the stale claim visible.
      expect(source, `${file} must state ${ratio}`).toContain(ratio);
    }
  });
});
