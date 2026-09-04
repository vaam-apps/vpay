/**
 * The TypeScript half of AGENTS.md's first rule: no test double is reachable
 * from a shipping process.
 *
 * `cargo xtask verify-no-mocks` enforces it for the Rust workspace. Nothing
 * enforced it here, so this does: every `.ts`/`.tsx` file under `src/app` and
 * `src/server` that is not itself a test must not name `src/testing`.
 *
 * The decisive check: add `import { MemoryShopStore } from "../testing/memory-store"`
 * to `src/server/context.ts` and this test fails.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = fileURLToPath(new URL("..", import.meta.url));

function walk(dir: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      found.push(...walk(full));
    } else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
      found.push(full);
    }
  }
  return found;
}

describe("the shipping module graph", () => {
  it("names nothing under src/testing", () => {
    const shipping = [
      ...walk(join(SRC, "app")),
      ...walk(join(SRC, "server")),
      ...walk(join(SRC, "components")),
      ...walk(join(SRC, "lib")),
    ];
    // A guard that scanned an empty list would pass forever.
    expect(shipping.length).toBeGreaterThan(15);

    const offenders = shipping.filter((file) => {
      const source = readFileSync(file, "utf8");
      return /["'](?:@\/testing\/|(?:\.\.?\/)+testing\/)/.test(source);
    });
    expect(offenders).toEqual([]);
  });

  it("would catch an import if one were added", () => {
    // The regex above, applied to the line it exists to reject. If this
    // stops matching, the guard above has quietly stopped guarding.
    const line = 'import { MemoryShopStore } from "../testing/memory-store";';
    expect(/["'](?:@\/testing\/|(?:\.\.?\/)+testing\/)/.test(line)).toBe(true);
    expect(
      /["'](?:@\/testing\/|(?:\.\.?\/)+testing\/)/.test(
        'import { x } from "@/testing/memory-store";',
      ),
    ).toBe(true);
    expect(
      /["'](?:@\/testing\/|(?:\.\.?\/)+testing\/)/.test(
        'import { db } from "./db";',
      ),
    ).toBe(false);
  });
});
