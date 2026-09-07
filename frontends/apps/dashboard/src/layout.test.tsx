/**
 * The root layout: the theme and the persistent nav, pinned.
 *
 * `docs/plans/2026-09-07-ui-revamp.md` §5 Lane D names this exact
 * regression as a decisive mutation: "the layout's `data-theme` reverts to
 * `corporate`" must fail. Measured directly while writing this test
 * (`docs/plans/exp26-notes/lane-d.md`): `frontends/packages/ui/src/styles.css`
 * configures daisyUI with exactly one theme (`bumblebee`), so a
 * `data-theme` that does not say `bumblebee` compiles no styling at all —
 * the page renders completely unthemed in a real browser. jsdom would not
 * show that (it computes no styles from the built stylesheet), which is
 * why this test pins the attribute on the rendered markup directly rather
 * than trusting a render to "look right".
 */
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import RootLayout from '../app/layout';

function markup(): string {
  return renderToStaticMarkup(RootLayout({ children: <p>content</p> }) as ReactElement);
}

describe('the root layout', () => {
  it('sets the bumblebee theme — the only theme @vpay/ui actually compiles', () => {
    const html = markup();
    expect(html).toContain('data-theme="bumblebee"');
    expect(html).not.toContain('data-theme="corporate"');
  });

  it("renders the brand as the page's one <h1>, inside the <nav>", () => {
    // Real element nesting, on the rendered markup — not the unrendered
    // element tree, which stops at `Heading`/`PageShell` (function
    // components) and never reaches the `<h1>` they render internally.
    expect(markup()).toMatch(/<nav>[\s\S]*<h1[^>]*>vpay dashboard<\/h1>[\s\S]*<\/nav>/);
  });

  it('renders the brand text exactly once', () => {
    const matches = markup().match(/vpay dashboard/g) ?? [];
    expect(matches).toHaveLength(1);
  });
});

const APP_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', 'app');

/**
 * Does `app/` actually have a page for this route?
 *
 * App Router: `/` is `app/page.tsx`, `/payments` is `app/payments/page.tsx`.
 * Route groups and dynamic segments are not handled because this app has
 * none; the day it does, this helper is what has to learn about them, and a
 * false failure here is the correct kind of failure — it is noticed.
 */
function pageExists(route: string): boolean {
  const segments = route.split('/').filter(Boolean);
  return ['tsx', 'ts', 'jsx', 'js'].some((ext) =>
    existsSync(join(APP_DIR, ...segments, `page.${ext}`)),
  );
}

/**
 * The nav-honesty rule, as a gate rather than as a comment.
 *
 * `docs/flows/dashboard.md`: "the navigation is only ever allowed to link to
 * slices that exist. A menu entry for a page nobody wrote is the same lie as
 * an empty table." That rule is stated in this app's README, in its
 * `app/layout.tsx` doc comment and in the flow doc — and until this test it
 * was enforced by nothing at all: adding `<a href="/payments">Payments</a>`
 * to the `<nav>` left the whole suite green, lint green and
 * `dashboard.cy.ts` green (measured — `docs/plans/exp26-notes/lane-d-review.md`,
 * finding 2).
 *
 * External links (`http…`, `mailto:`) are out of scope: this checks that a
 * link into this app goes somewhere this app serves.
 */
describe('the nav links only to pages that exist', () => {
  it('every internal href in the layout has a page under app/', () => {
    const hrefs = [...markup().matchAll(/href="([^"]*)"/g)].map((m) => m[1] ?? '');
    const internal = hrefs.filter((h) => h.startsWith('/'));
    const dangling = internal.filter((h) => !pageExists(h.split(/[?#]/)[0] ?? ''));

    expect(dangling).toEqual([]);
  });

  it('the helper it relies on is not vacuously true', () => {
    // A negative control: if `pageExists` answered `true` for everything the
    // test above would pass whatever the layout linked to.
    expect(pageExists('/')).toBe(true);
    expect(pageExists('/payments')).toBe(false);
  });
});
