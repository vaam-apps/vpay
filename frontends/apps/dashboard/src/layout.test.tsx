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
