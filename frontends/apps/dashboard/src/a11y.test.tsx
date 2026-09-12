/**
 * This app's own structural accessibility gate.
 *
 * This app composed `@vpay/ui`, which had an axe suite of its own
 * (`frontends/packages/ui/src/axe.test.tsx`), and Lane D's notes argued that
 * made a second one here redundant, since this app was "composition only".
 * That argument was wrong, and the review measured why
 * (`docs/plans/exp26-notes/lane-d-review.md`, finding 1): the scaffold at
 * `08d9b8e` wrapped its page in `<main>`, the rewritten composition did not,
 * and axe-core's `region` rule went from **0 violations to 1** — "All page
 * content should be contained by landmarks". No component's own suite can see
 * that, because the landmark is a decision `app/layout.tsx` makes and no
 * component holds. That reasoning is unaffected by which package supplies the
 * components — this app's own gate is what actually renders `app/layout.tsx`
 * and checks the assembled page.
 *
 * Structural rules only. jsdom computes no real layout or paint, so
 * `color-contrast` here would be evidence of nothing (plan §7 row 6 — that
 * check is `cypress-axe` against a real browser, and is still built by
 * nobody).
 *
 * **Updated 2026-09-12:** `@vpay/ui` was deleted; the dashboard now composes
 * `@vaam-apps/ui`, which ships no axe suite of its own that this repo builds
 * — its structural coverage lives upstream, in that package's own Storybook
 * a11y panel, and is not re-run here. `DetailTimeline` and `EmptyState` had
 * cases here until 2026-09-11 for exactly the reason above (they were
 * `@vpay/ui` primitives rendered by that package's own suite); the timeline
 * is now this app's own plain `<ul>` and the empty state is
 * `InlineEmptyState`, so both are covered again below, on this app's own
 * markup, rather than left to a suite this repo does not build.
 *
 * Every screen is covered through the **component** it is made of rather than
 * through its `page.tsx`: every page in this app is an async server component
 * that reads cookies and calls vpay, which jsdom cannot run. What each page
 * adds on top of these components is composition and a redirect, and the
 * landmark case below renders the real layout around real content.
 */
import { render } from "@testing-library/react";
import axe from "axe-core";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import RootLayout from "../app/layout";
import LoginLayout from "../app/login/layout";
import { EnrolmentPanel } from "./components/enrolment-panel";
import { PasswordForm } from "./components/password-form";
import { PaymentDetailView } from "./components/payment-detail";
import { PaymentsFilters } from "./components/payments-filters";
import { PaymentsPager } from "./components/payments-pager";
import { PaymentsTable } from "./components/payments-table";
import { SignInForm } from "./components/sign-in-form";
import { SignedInBar } from "./components/signed-in-bar";
import { TotpForm } from "./components/totp-form";
import { DETAIL, INTENT } from "./testing/fixtures";

// `PaymentsFilters` calls `useRouter()` to apply (see its own module doc);
// outside a mock, Next throws "invariant expected app router to be mounted"
// the moment it renders, which is every rendered-component case below since
// none of them wrap a real `<AppRouterContext.Provider>`.
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));

/**
 * Exactly plan §7 row 5's list, as `frontends/packages/ui/src/testing/axe.ts`
 * spells it — **plus `select-name`, 2026-09-12**.
 *
 * That addition is a hole being closed, not a rule being collected.
 * axe-core 4.13's `label` rule has the selector `input, textarea`: a
 * `<select>` is not in it, and is covered by the separate `select-name`
 * rule instead (read back off the installed build:
 * `axe._audit.rules.find(r => r.id === "label").selector`). This list
 * carried `label` and not `select-name`, so the one `<select>` in this app
 * — `PaymentsFilters`' status filter, the control the payments screen is
 * filtered by — had its accessible name checked by nothing here. Measured
 * before adding it: pointing that field's `<label htmlFor>` at an id no
 * element carries left every case in this file green.
 */
const STRUCTURAL_RULES = [
  "label",
  "select-name",
  "button-name",
  "link-name",
  "aria-required-attr",
  "aria-required-children",
  "aria-required-parent",
  "aria-roles",
  "aria-valid-attr",
  "aria-valid-attr-value",
  "aria-command-name",
  "region",
  "list",
  "listitem",
  "duplicate-id",
  "duplicate-id-aria",
];

async function violations(container: Element): Promise<string[]> {
  const results = await axe.run(container, {
    runOnly: { type: "rule", values: STRUCTURAL_RULES },
  });
  // The rule id and the offending markup, so a failure says what and where.
  return results.violations.map(
    (v) => `${v.id}: ${v.nodes.map((n) => n.html).join(" | ")}`,
  );
}

const NO_STATE = { error: null, requestId: null };
const idle = () => vi.fn(() => Promise.resolve(NO_STATE));

describe("the rendered app", () => {
  it("puts every byte of page content inside a landmark", async () => {
    // The real `<body>` the layouts render around real page content, not a
    // fragment — `region` is a document-level rule and a fragment would pass
    // it vacuously.
    //
    // **Two layouts, because that is what a route actually gets.** The root
    // layout is the document and provides no landmark of its own since the
    // nav moved out of it; the `<main>` comes from the group a route is in —
    // `LoginLayout` for the signed-out routes, `AppShell` for `(dash)`.
    // Rendering the root alone would assert against a composition no URL
    // resolves to, and it would fail exactly as it did when the nav left.
    const html = renderToStaticMarkup(
      RootLayout({
        children: LoginLayout({
          children: <PaymentsTable rows={[INTENT]} />,
        }) as ReactElement,
      }) as ReactElement,
    );
    const body = html
      .replace(/^[\s\S]*?<body[^>]*>/, "")
      .replace(/<\/body>[\s\S]*$/, "");
    document.body.innerHTML = body;

    expect(await violations(document.body)).toEqual([]);
  });
});

describe("every screen this app renders", () => {
  it("the password form, clean, refused and pending", async () => {
    const clean = render(<SignInForm action={idle()} />);
    expect(await violations(clean.container)).toEqual([]);
    clean.unmount();

    const refused = render(
      <SignInForm
        action={vi.fn(() =>
          Promise.resolve({ error: "Refused.", requestId: "req_01J8" }),
        )}
      />,
    );
    expect(await violations(refused.container)).toEqual([]);
    refused.unmount();
  });

  it("the code form and the enrolment panel", async () => {
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

  it("the password-change form", async () => {
    const { container, unmount } = render(<PasswordForm action={idle()} />);
    expect(await violations(container)).toEqual([]);
    unmount();
  });

  it("the payments list, its filters and its signed-in bar", async () => {
    const table = render(<PaymentsTable rows={[INTENT]} />);
    expect(await violations(table.container)).toEqual([]);
    table.unmount();

    const filters = render(
      <PaymentsFilters
        values={{ status: "", createdFrom: "", createdTo: "" }}
      />,
    );
    expect(await violations(filters.container)).toEqual([]);
    filters.unmount();

    const bar = render(
      <SignedInBar
        email="ada@example.test"
        merchantId="demo-merchant-tenant"
        signOut={vi.fn()}
      />,
    );
    expect(await violations(bar.container)).toEqual([]);
    bar.unmount();
  });

  it("the payment detail, with and without a charge", async () => {
    const full = render(<PaymentDetailView detail={DETAIL} />);
    expect(await violations(full.container)).toEqual([]);
    full.unmount();

    const bare = render(
      <PaymentDetailView detail={{ ...DETAIL, charge: null, events: [] }} />,
    );
    expect(await violations(bare.container)).toEqual([]);
    bare.unmount();
  });

  it("the pager, on a middle page and on the first one", async () => {
    // The pager is entirely this app's own (`payments-pager.tsx`) — neither
    // `@vpay/ui` (deleted 2026-09-12) nor `@vaam-apps/ui`'s callback-driven
    // `Pagination` fits an href-based `next/link` pager, so there is no
    // upstream suite to defer to here. What is checked is a `<nav>` whose
    // links are `next/link` elements. `link-name` and `region` both bite on
    // a nav.
    const middle = render(
      <PaymentsPager
        previousHref="/payments?ending_before=pi_2"
        nextHref="/payments?starting_after=pi_9"
      />,
    );
    expect(await violations(middle.container)).toEqual([]);
    middle.unmount();

    const first = render(
      <PaymentsPager
        previousHref={null}
        nextHref="/payments?starting_after=pi_9"
      />,
    );
    expect(await violations(first.container)).toEqual([]);
    first.unmount();
  });
});
