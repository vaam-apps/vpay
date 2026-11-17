/**
 * The app's own source text, for the tests that gate what the source may say.
 *
 * `no-dropdowns.test.ts` and `no-shadows.test.ts` scan the same tree the same
 * way, so the walk and the comment stripper live once, here. Tests import
 * this; nothing under `app/` may (the shared ESLint config bans it).
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

/** The dashboard package's root. */
export const APP_ROOT = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

/** A source file, as a path from {@link APP_ROOT} and its text. */
export interface Source {
  readonly path: string;
  readonly text: string;
}

function walk(dir: string, extensions: RegExp): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.name === "node_modules" || entry.name === ".next") {
      return [];
    }
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      return walk(full, extensions);
    }
    return extensions.test(entry.name) ? [full] : [];
  });
}

/** Every file under `app/` and `src/` whose name matches `extensions`. */
export function appSources(extensions: RegExp): Source[] {
  return ["app", "src"]
    .flatMap((root) => walk(join(APP_ROOT, root), extensions))
    .map((full) => ({
      path: relative(APP_ROOT, full),
      text: readFileSync(full, "utf8"),
    }))
    .sort((a, b) => a.path.localeCompare(b.path));
}

/**
 * The text with comments removed, line structure kept.
 *
 * A `//` counts as a comment only at the start of a line or after whitespace,
 * so the `//` in `https://` is left alone. That is a lexer's cheap
 * approximation, and it errs toward *keeping* text — a scan over the result
 * can over-report, never hide a use. Block comments, including the ones in
 * JSX braces, are replaced by their newlines.
 */
export function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, ""))
    .replace(/(^|\s)\/\/.*$/gm, "$1");
}
