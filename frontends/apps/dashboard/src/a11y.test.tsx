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
 * Every screen is covered through the **component** it is made of. Since
 * VPA-5a the app's pages are the sign-in's steps and the account page, all of
 * them inside `CenteredShell`'s landmark; the pages that read `/dash/v1` are a
 * later ticket, and the presentational components they were made of are covered
 * here and in their own suites.
 */
import { fireEvent, render, screen } from "@testing-library/react";
import axe from "axe-core";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import RootLayout from "../app/layout";
import { AccountPage } from "./components/account-page";
import { CenteredShell } from "./components/centered-shell";
import { EnrolmentPanel } from "./components/enrolment-panel";
import {
  NotConfigured,
  PairedKeyOnly,
  SignInUnreachable,
} from "./components/login-notices";
import { NotFoundNotice } from "./components/not-found-notice";
import { PasswordForm } from "./components/password-form";
import { SignInForm } from "./components/sign-in-form";
import { TotpForm } from "./components/totp-form";
import { PaymentDetailView } from "./components/payment-detail";
import { PaymentsFilters } from "./components/payments-filters";
import { PaymentsPager } from "./components/payments-pager";
import { PaymentsTable } from "./components/payments-table";
import { DETAIL, INTENT } from "./testing/fixtures";

// `PaymentsFilters` calls `useRouter()` to apply (see its own module doc);
// outside a mock, Next throws "invariant expected app router to be mounted"
// the moment it renders, which is every rendered-component case below since
// none of them wrap a real `<AppRouterContext.Provider>`.
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
  usePathname: () => "/payments",
}));

/**
 * Exactly plan §7 row 5's list, as `frontends/packages/ui/src/testing/axe.ts`
 * spells it — **plus `aria-dialog-name`, 2026-10-30 (VPA-5a)**: the key sheet
 * is a dialog, and with an empty title the rules above passed it; the
 * accessible name of a dialog is its own rule — **and `select-name`, 2026-09-12**.
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
  "aria-dialog-name",
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

const NOOP = () => Promise.resolve({ error: null, requestId: null });

describe("the rendered app", () => {
  it("puts every byte of page content inside a landmark", async () => {
    // The real `<body>` the root layout renders around a real page, not a
    // fragment — `region` is a document-level rule and a fragment would pass
    // it vacuously. The root layout is the document and provides no landmark
    // of its own; the `<main>` comes from `CenteredShell`.
    const html = renderToStaticMarkup(
      RootLayout({
        children: (
          <CenteredShell>
            <PairedKeyOnly />
          </CenteredShell>
        ),
      }) as ReactElement,
    );
    const body = html
      .replace(/^[\s\S]*?<body[^>]*>/, "")
      .replace(/<\/body>[\s\S]*$/, "");
    document.body.innerHTML = body;

    expect(await violations(document.body)).toEqual([]);
  });
});

describe("every screen of the sign-in", () => {
  /** Renders `ui` inside the shell every page has, and checks the whole thing. */
  async function clean(ui: ReactElement) {
    const { container, unmount } = render(<CenteredShell>{ui}</CenteredShell>);
    const found = await violations(container);
    unmount();
    return found;
  }

  it("the email question", async () => {
    expect(await clean(<SignInForm action={NOOP} />)).toEqual([]);
  });

  it("the password question, with the account name carried for a password manager", async () => {
    const { container, unmount } = render(
      <CenteredShell>
        <SignInForm action={NOOP} />
      </CenteredShell>,
    );
    fireEvent.change(screen.getByLabelText("Work email"), {
      target: { value: "ada@acme.test" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await violations(container)).toEqual([]);
    unmount();
  });

  it("the code question", async () => {
    expect(await clean(<TotpForm action={NOOP} />)).toEqual([]);
  });

  it("the enrolment screen, and the key sheet open over it", async () => {
    const { container, unmount } = render(
      <CenteredShell>
        <EnrolmentPanel
          qrDataUrl="data:image/png;base64,AAAA"
          secret="JBSWY3DPEHPK3PXP"
        />
        <TotpForm action={NOOP} enrolling />
      </CenteredShell>,
    );
    expect(await violations(container)).toEqual([]);

    fireEvent.click(
      screen.getByRole("button", {
        name: "Cannot scan? Enter the key instead",
      }),
    );
    await screen.findByRole("dialog");
    // The sheet is portalled out of `container`, so the document is the root.
    expect(await violations(document.body)).toEqual([]);
    unmount();
  });

  it("the password change", async () => {
    expect(await clean(<PasswordForm action={NOOP} />)).toEqual([]);
  });

  it("the account page", async () => {
    expect(
      await clean(
        <AccountPage
          displayName="Ada Lovelace"
          email="ada@acme.test"
          merchantId="acme"
          signOut={() => Promise.resolve()}
        />,
      ),
    ).toEqual([]);
  });

  it("the three notices and the not-found page", async () => {
    expect(await clean(<PairedKeyOnly />)).toEqual([]);
    expect(
      await clean(
        <NotConfigured
          problems={[
            { variable: "VPAY_DASHBOARD_CLIENT_ID", detail: "the id." },
          ]}
        />,
      ),
    ).toEqual([]);
    expect(
      await clean(
        <SignInUnreachable
          failure={{ status: 503, message: "Starting up.", requestId: "req_1" }}
        />,
      ),
    ).toEqual([]);
    expect(await clean(<NotFoundNotice />)).toEqual([]);
  });
});

describe("every screen this app renders", () => {
  it("the payments list and its filters", async () => {
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

    // And with a range set, which the empty render above never reaches: the
    // date field's trigger then points `aria-describedby` at the shown dates
    // (`aria-valid-attr-value` checks the reference resolves), and the Clear
    // button renders beside it (`button-name`). The empty render is the one
    // that holds the trigger's own name: since 2026-09-24 that is the
    // visible "Created between" label (`button-name` accepts a `<label for>`
    // through axe's `explicit-label` check), with the "Any time" placeholder
    // as visible text besides — so axe fires only when both are gone.
    const filtered = render(
      <PaymentsFilters
        values={{
          status: "",
          createdFrom: "2026-09-01",
          createdTo: "2026-09-07",
        }}
      />,
    );
    expect(await violations(filtered.container)).toEqual([]);
    filtered.unmount();
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
