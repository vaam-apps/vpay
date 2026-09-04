/**
 * The TypeScript half of AGENTS.md's first rule, for the checkout app: no
 * test double is reachable from the page a payer loads.
 *
 * `cargo xtask verify-no-mocks` enforces it for the Rust workspace and
 * `examples/shop/src/testing/no-runtime-imports.test.ts` for the shop.
 * Nothing enforced it here, so the sentence at the top of `fixtures.ts`
 * ("nothing under `src/testing` is imported from `app/` or from any
 * component") was prose, and it was false as written — `src/components/
 * screen-states.ts` imported `../testing/fixtures`. That file now lives
 * under `src/testing/` and this is the check that keeps the claim honest.
 *
 * The decisive check: add
 * `import { makeSession } from '../testing/fixtures'` to
 * `src/lib/controller.ts` and this test fails.
 *
 * **What counts as shipping.** `app/`, `middleware.ts` and everything under
 * `src/components`, `src/lib` and `src/i18n` — `next build` reaches all of
 * them. Two file *kinds* are excluded, not two paths: `*.test.ts(x)`, which
 * vitest runs and Next never bundles, and `*.stories.tsx`, which only
 * `pnpm --filter @vpay/ui build-storybook` reads (its glob is
 * `frontends/packages/ui/.storybook/main.ts`). Both are excluded by suffix
 * so that a new one is covered by the same rule rather than by an
 * allowlist entry somebody has to remember to add.
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/** `frontends/apps/checkout/` — `src/testing/` is two levels down. */
const APP = fileURLToPath(new URL('../..', import.meta.url));

/**
 * Matches an import specifier that names `src/testing`, by either spelling
 * the app uses: a relative hop (`../testing/…`, `./testing/…`) or the `@/`
 * alias. Kept in one place so the guard below and its self-check cannot
 * drift apart.
 */
const NAMES_TESTING = /["'](?:@\/testing\/|(?:\.\.?\/)+testing\/)/;

function walk(dir: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      found.push(...walk(full));
    } else if (/\.tsx?$/.test(entry) && !/\.(?:test|stories)\.tsx?$/.test(entry)) {
      found.push(full);
    }
  }
  return found;
}

describe('the shipping module graph', () => {
  it('names nothing under src/testing', () => {
    const middleware = join(APP, 'middleware.ts');
    // Named explicitly rather than found by the walk: it sits at the package
    // root beside `next.config.ts`, it is what sets the checkout page's
    // `Content-Security-Policy`, and it ships.
    expect(existsSync(middleware)).toBe(true);

    const shipping = [
      ...walk(join(APP, 'app')),
      ...walk(join(APP, 'src', 'components')),
      ...walk(join(APP, 'src', 'lib')),
      ...walk(join(APP, 'src', 'i18n')),
      middleware,
    ];
    // A guard that scanned an empty list would pass forever. 30 files today.
    expect(shipping.length).toBeGreaterThan(25);

    const offenders = shipping.filter((file) => NAMES_TESTING.test(readFileSync(file, 'utf8')));
    expect(offenders).toEqual([]);
  });

  it('would catch an import if one were added', () => {
    // The regex above, applied to the lines it exists to reject. If this
    // stops matching, the guard above has quietly stopped guarding — which
    // is exactly how the claim in `fixtures.ts` went stale unnoticed.
    expect(NAMES_TESTING.test("import { makeSession } from '../testing/fixtures';")).toBe(true);
    expect(NAMES_TESTING.test("import { CHECKOUT_SCREENS } from './testing/screen-states';")).toBe(
      true,
    );
    expect(NAMES_TESTING.test("import { post } from '@/testing/browser-stub';")).toBe(true);
    expect(NAMES_TESTING.test("import { renderScreen } from '../lib/machine';")).toBe(false);
  });

  it('excludes tests and stories by suffix, not by path', () => {
    const excluded = /\.(?:test|stories)\.tsx?$/;
    expect(excluded.test('checkout-view.test.tsx')).toBe(true);
    expect(excluded.test('dictionary.test.ts')).toBe(true);
    expect(excluded.test('checkout-screens.stories.tsx')).toBe(true);
    expect(excluded.test('checkout-view.tsx')).toBe(false);
    expect(excluded.test('middleware.ts')).toBe(false);
  });
});
