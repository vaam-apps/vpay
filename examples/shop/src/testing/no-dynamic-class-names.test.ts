/**
 * Tailwind 4 generates a class only if it can **read it in the source**, so a
 * `className` assembled at runtime is a rule that never reaches the
 * stylesheet.
 *
 * This is the silent failure `docs/plans/2026-09-07-ui-revamp.md` §6.3/§6.4
 * is about, and it is silent in every direction: the markup is valid, the
 * component renders, jsdom asserts the class is on the element, `typecheck`,
 * `lint` and `next build` are all green — and the badge is colourless in a
 * browser. It shipped here once already. `OrderStatusBadge` and
 * `TestNumbersPanel` rendered `` `badge badge-${tone}` ``, and the three
 * `badge-*` rules were present in `.next`'s stylesheet **only because
 * `order-summary.test.tsx` happens to contain the strings as expected
 * values**: deleting that one test file removed the colour from every order
 * page. Measured on 2026-09-07 by doing exactly that.
 *
 * So the rule is enforced rather than commented: no `className` in this
 * package's shipping source is a template literal with an interpolation in
 * it. Where a class depends on state, the whole class string is written out
 * per case (`order-summary.tsx`'s `BADGE_CLASS`), which is what Tailwind's
 * scanner can see.
 *
 * The decisive check: restore `` className={`badge badge-${TONE[status]}`} ``
 * in `src/components/order-summary.tsx` and this test fails.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = fileURLToPath(new URL("..", import.meta.url));

/**
 * A `className=` whose value is a template literal carrying at least one
 * `${…}`. Deliberately narrow: `className="…"`, `className={CONST[key]}` and
 * a template literal with no interpolation are all fine, because all three
 * leave a complete class string in the file for the scanner to find.
 */
const DYNAMIC_CLASS_NAME = /className=\{`[^`]*\$\{/;

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

describe("the shop's class strings", () => {
  it("are never assembled at runtime", () => {
    const shipping = [
      ...walk(join(SRC, "app")),
      ...walk(join(SRC, "components")),
    ];
    // A guard that scanned an empty list would pass forever.
    expect(shipping.length).toBeGreaterThan(10);

    const offenders = shipping.filter((file) =>
      DYNAMIC_CLASS_NAME.test(readFileSync(file, "utf8")),
    );
    expect(offenders).toEqual([]);
  });

  it("would catch an assembled one if it came back", () => {
    // The regex above, applied to the shape it exists to reject and to the
    // shapes it must not reject. If the first stops matching, the guard has
    // quietly stopped guarding.
    //
    // The tone words here are deliberately nonsense (`tonehere`, `xyzzy`):
    // Tailwind scans this file too, and a real daisyUI class written out in
    // a test is exactly how the defect above hid for a day.
    expect(
      DYNAMIC_CLASS_NAME.test("className={`chip chip-${TONE[status]}`}"),
    ).toBe(true);
    expect(DYNAMIC_CLASS_NAME.test('className="chip chip-tonehere"')).toBe(
      false,
    );
    expect(DYNAMIC_CLASS_NAME.test("className={CHIP_CLASS[status]}")).toBe(
      false,
    );
    expect(DYNAMIC_CLASS_NAME.test("data-testid={`chip-${id}`}")).toBe(false);
  });
});
