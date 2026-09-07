/**
 * This app's own structural accessibility gate.
 *
 * `@vpay/ui` has an axe suite of its own (`frontends/packages/ui/src/axe.test.tsx`)
 * and Lane D's notes argued that made a second one here redundant, since this
 * app is "composition only". That argument is wrong, and the review measured
 * why (`docs/plans/exp26-notes/lane-d-review.md`, finding 1): the scaffold at
 * `08d9b8e` wrapped its page in `<main>`, the rewritten composition did not,
 * and axe-core's `region` rule went from **0 violations to 1** — "All page
 * content should be contained by landmarks". No component's own suite can see
 * that, because the landmark is a decision `app/layout.tsx` makes and no
 * component holds.
 *
 * Structural rules only. jsdom computes no real layout or paint, so
 * `color-contrast` here would be evidence of nothing (plan §7 row 6 — that
 * check is `cypress-axe` against a real browser, and is still built by
 * nobody).
 *
 * Every screen is covered through the **component** it is made of rather than
 * through its `page.tsx`: every page in this app is an async server component
 * that reads cookies and calls vpay, which jsdom cannot run. What each page
 * adds on top of these components is composition and a redirect, and the
 * landmark case below renders the real layout around real content.
 */
import { render } from '@testing-library/react';
import axe from 'axe-core';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

import RootLayout from '../app/layout';
import { DetailTimeline } from './components/detail-timeline';
import { EmptyState } from './components/empty-state';
import { EnrolmentPanel } from './components/enrolment-panel';
import { PasswordForm } from './components/password-form';
import { PaymentDetailView } from './components/payment-detail';
import { PaymentsFilters } from './components/payments-filters';
import { PaymentsTable } from './components/payments-table';
import { SignInForm } from './components/sign-in-form';
import { SignedInBar } from './components/signed-in-bar';
import { TotpForm } from './components/totp-form';
import { DETAIL, INTENT } from './testing/fixtures';

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
  return results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.html).join(' | ')}`);
}

const NO_STATE = { error: null, requestId: null };
const idle = () => vi.fn(() => Promise.resolve(NO_STATE));

describe('the rendered app', () => {
  it('puts every byte of page content inside a landmark', async () => {
    // The real `<body>` the layout renders around real page content, not a
    // fragment — `region` is a document-level rule and a fragment would pass
    // it vacuously.
    const html = renderToStaticMarkup(
      RootLayout({ children: <PaymentsTable rows={[INTENT]} /> }) as ReactElement,
    );
    const body = html.replace(/^[\s\S]*?<body[^>]*>/, '').replace(/<\/body>[\s\S]*$/, '');
    document.body.innerHTML = body;

    expect(await violations(document.body)).toEqual([]);
  });
});

describe('every screen this app renders', () => {
  it('the password form, clean, refused and pending', async () => {
    const clean = render(<SignInForm action={idle()} />);
    expect(await violations(clean.container)).toEqual([]);
    clean.unmount();

    const refused = render(
      <SignInForm action={vi.fn(() => Promise.resolve({ error: 'Refused.', requestId: 'req_01J8' }))} />,
    );
    expect(await violations(refused.container)).toEqual([]);
    refused.unmount();
  });

  it('the code form and the enrolment panel', async () => {
    const form = render(<TotpForm action={idle()} enrolling />);
    expect(await violations(form.container)).toEqual([]);
    form.unmount();

    const panel = render(
      <EnrolmentPanel
        // A one-pixel PNG: this test is about structure, and generating a
        // real QR here would put `qrcode` in the assertion path.
        qrDataUrl="data:image/png;base64,iVBORw0KGgo="
        secret="GEZDGNBVGY3TQOJQ"
      />,
    );
    expect(await violations(panel.container)).toEqual([]);
    panel.unmount();
  });

  it('the password-change form', async () => {
    const { container, unmount } = render(<PasswordForm action={idle()} />);
    expect(await violations(container)).toEqual([]);
    unmount();
  });

  it('the payments list, its filters and its signed-in bar', async () => {
    const table = render(<PaymentsTable rows={[INTENT]} />);
    expect(await violations(table.container)).toEqual([]);
    table.unmount();

    const filters = render(
      <PaymentsFilters values={{ status: '', createdFrom: '', createdTo: '' }} />,
    );
    expect(await violations(filters.container)).toEqual([]);
    filters.unmount();

    const bar = render(
      <SignedInBar email="ada@example.test" merchantId="demo-merchant-tenant" signOut={vi.fn()} />,
    );
    expect(await violations(bar.container)).toEqual([]);
    bar.unmount();
  });

  it('the payment detail, with and without a charge', async () => {
    const full = render(<PaymentDetailView detail={DETAIL} />);
    expect(await violations(full.container)).toEqual([]);
    full.unmount();

    const bare = render(<PaymentDetailView detail={{ ...DETAIL, charge: null, events: [] }} />);
    expect(await violations(bare.container)).toEqual([]);
    bare.unmount();
  });

  it('the timeline, populated and empty', async () => {
    const populated = render(
      <DetailTimeline events={[{ id: 'evt_1', at: '2026-09-07T09:00:00Z', label: 'created' }]} />,
    );
    expect(await violations(populated.container)).toEqual([]);
    populated.unmount();

    const empty = render(<DetailTimeline events={[]} />);
    expect(await violations(empty.container)).toEqual([]);
    empty.unmount();
  });

  it('the empty state', async () => {
    const { container, unmount } = render(
      <EmptyState title="No payments" description="This merchant has no payment intents yet." />,
    );
    expect(await violations(container)).toEqual([]);
    unmount();
  });
});
