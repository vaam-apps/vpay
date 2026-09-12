/**
 * The one rule `@vaam-apps/ui`'s instrument register puts on its callers.
 *
 * `InstrumentPanel` draws an aurora mesh, and a gradient ground breaks the
 * assumption every contrast check rests on — that a foreground sits on one
 * known surface. The library measured it: over `--surface-1` at the shipped
 * 14% stop cap, worst case across the four blobs, `--subtle-foreground`
 * falls to **4.41:1 in dark and 4.45:1 in light**, below the 4.5:1 AA bar,
 * while `--muted-foreground` holds at 5.29:1 and above. The component
 * renders its own caption at muted and its doc says, in as many words:
 * *"Do not put `text-subtle-foreground` on this surface."*
 *
 * That is a rule about **call sites**, and a rule about call sites that
 * nothing checks is a comment. This checks it.
 *
 * **What this does NOT do is re-measure the mesh.** The compositing of four
 * offset blobs over a surface step is the library's measurement and is
 * taken on trust here; nothing in this repository renders a gradient ground
 * and samples it. What it does is make the one derived rule mechanical, so
 * a later edit that reaches for the quieter tier on this surface fails
 * rather than shipping 4.4:1 to an operator.
 *
 * The dashboard has no browser-level a11y gate at all — `just
 * test-storybook` covers the checkout's 22 screens and nothing here — so
 * this file and the jsdom axe suite are the whole of what guards these
 * surfaces. That gap is stated in `docs/status/frontend.md` rather than
 * implied.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const SRC = dirname(fileURLToPath(import.meta.url));

/** Every `.tsx` under `src/`, comments stripped — prose names the banned class. */
function componentSources(): { file: string; code: string }[] {
  const out: { file: string; code: string }[] = [];
  for (const entry of readdirSync(SRC, { recursive: true })) {
    const rel = entry.toString();
    if (!rel.endsWith(".tsx") || rel.includes(".test.")) {
      continue;
    }
    const raw = readFileSync(join(SRC, rel), "utf8");
    out.push({
      file: rel,
      code: raw.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, ""),
    });
  }
  return out;
}

describe("the instrument register's rule on its callers", () => {
  it("finds the sources it is meant to read", () => {
    const files = componentSources();
    expect(files.length).toBeGreaterThan(5);
    expect(
      // The JSX, not the identifier: an import left behind by a partial
      // edit would satisfy `includes("InstrumentPanel")` and this case
      // would go on guarding a surface nothing renders. Measured — that is
      // exactly what the first draft did.
      files.some(({ code }) => /<InstrumentPanel[\s>]/.test(code)),
      "this app renders an InstrumentPanel — if it stopped, delete this file",
    ).toBe(true);
  });

  it("never puts the subtle tier on a mesh ground", () => {
    for (const { file, code } of componentSources()) {
      if (!/<InstrumentPanel[\s>]/.test(code)) {
        continue;
      }
      expect(
        code,
        `${file}: text-subtle-foreground is 4.41:1 on the aurora mesh — the library bans it, use muted`,
      ).not.toMatch(/text-subtle-foreground/);
    }
  });

  it("keeps the mesh free of anything that could read as a status", () => {
    // The aurora is safe in a system where colour means status only because
    // nothing about a state can be expressed through it. `InstrumentPanel`
    // takes title, caption and children and offers no tint — but a caller
    // can still paint a status colour INSIDE it, which would put a hue on a
    // ground that is deliberately meaningless. `verify-ui` bans
    // `text-state-*` app-wide; this says why it matters here specifically.
    for (const { file, code } of componentSources()) {
      if (!/<InstrumentPanel[\s>]/.test(code)) {
        continue;
      }
      expect(
        code,
        `${file}: a status colour inside the mesh — the ground carries no meaning and must not start`,
      ).not.toMatch(/\b(bg|text|border)-state-[a-z]+-(fg|bg|border)\b/);
    }
  });
});
