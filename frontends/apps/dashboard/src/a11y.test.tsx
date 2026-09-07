/**
 * This app's own structural accessibility gate.
 *
 * `@vpay/ui` has an axe suite of its own (`frontends/packages/ui/src/axe.test.tsx`)
 * and Lane D's notes argued that made a second one here redundant, since
 * this app is "composition only". That argument is wrong, and the review
 * measured why (`docs/plans/exp26-notes/lane-d-review.md`, finding 1): the
 * scaffold at `08d9b8e` wrapped its page in `<main>`, the rewritten
 * composition did not, and axe-core's `region` rule went from **0
 * violations to 1** — "All page content should be contained by landmarks".
 * No component's own suite can see that, because the landmark is a decision
 * this app's `app/layout.tsx` makes and no component holds.
 *
 * Same rule set and same reasoning as `@vpay/ui`'s helper: structural rules
 * only. jsdom computes no real layout or paint, so `color-contrast` here
 * would be evidence of nothing (plan §7 row 6 — that check is `cypress-axe`
 * against a real browser, and is still built by nobody).
 */
import { render } from '@testing-library/react';
import axe from 'axe-core';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import RootLayout from '../app/layout';
import Home from '../app/page';
import { DetailTimeline } from './recipes/detail-timeline';
import { EmptyState } from './recipes/empty-state';
import { PaymentsTable } from './recipes/payments-table';
import { SignInForm } from './recipes/sign-in-form';

/** Exactly plan §7 row 5's list, as `frontends/packages/ui/src/testing/axe.ts` spells it. */
const STRUCTURAL_RULES = [
  'label',
  'button-name',
  'link-name',
  'aria-required-attr',
  'aria-required-children',
  'aria-required-parent',
  'aria-roles',
  'aria-valid-attr',
  'aria-valid-attr-value',
  'aria-command-name',
  'region',
  'list',
  'listitem',
  'duplicate-id',
  'duplicate-id-aria',
];

async function violations(container: Element): Promise<string[]> {
  const results = await axe.run(container, {
    runOnly: { type: 'rule', values: STRUCTURAL_RULES },
  });
  // The rule id and the offending markup, so a failure says what and where.
  return results.violations.map(
    (v) => `${v.id}: ${v.nodes.map((n) => n.html).join(' | ')}`,
  );
}

describe('the rendered app', () => {
  it('puts every byte of page content inside a landmark', async () => {
    // The real `<body>` the layout renders around the real page, not a
    // fragment — `region` is a document-level rule and a fragment would
    // pass it vacuously.
    const html = renderToStaticMarkup(RootLayout({ children: Home() }) as ReactElement);
    const body = html.replace(/^[\s\S]*?<body[^>]*>/, '').replace(/<\/body>[\s\S]*$/, '');
    document.body.innerHTML = body;

    expect(await violations(document.body)).toEqual([]);
  });
});

describe('each recipe', () => {
  it('SignInForm, including its error state and its pending state', async () => {
    const { container, unmount } = render(
      <SignInForm error="That code has expired." requestId="req_01J8ABCDEF" onSubmit={() => {}} />,
    );
    expect(await violations(container)).toEqual([]);
    unmount();

    const disabled = render(<SignInForm pending onSubmit={() => {}} />);
    expect(await violations(disabled.container)).toEqual([]);
    disabled.unmount();
  });

  it('PaymentsTable', async () => {
    const { container, unmount } = render(
      <PaymentsTable rows={[{ id: 'pi_example_1', amount: '$12.00', status: 'succeeded' }]} />,
    );
    expect(await violations(container)).toEqual([]);
    unmount();
  });

  it('DetailTimeline, populated and empty', async () => {
    const populated = render(
      <DetailTimeline events={[{ id: 'evt_1', at: '2026-09-07T09:00:00Z', label: 'created' }]} />,
    );
    expect(await violations(populated.container)).toEqual([]);
    populated.unmount();

    const empty = render(<DetailTimeline events={[]} />);
    expect(await violations(empty.container)).toEqual([]);
    empty.unmount();
  });

  it('EmptyState', async () => {
    const { container, unmount } = render(
      <EmptyState title="No payments match these filters" description="Try widening the date range." />,
    );
    expect(await violations(container)).toEqual([]);
    unmount();
  });
});
