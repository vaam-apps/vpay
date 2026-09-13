/**
 * Every dashboard screen this app renders, with the a11y addon on.
 *
 * The states come from `../testing/fixtures.ts` — the same objects
 * `a11y.test.tsx` renders — so a screen a designer reviews here is a screen
 * a test covers, and a screen added to one without the other is a gap this
 * file's own comment names rather than one nobody noticed.
 *
 * `pnpm --filter @vpay/dashboard test-storybook` renders each of these in a
 * real Chromium and runs axe over it — the only thing in this repository
 * that can answer `color-contrast` for these screens; `a11y.test.tsx` runs
 * in jsdom, which computes no colour.
 *
 * # What is NOT here, and why
 *
 * Every page in this app (`app/**\/page.tsx`) is an async server component
 * that reads cookies and calls vpay — jsdom cannot run it and neither can a
 * browser-mode vitest story without a real backend, so `a11y.test.tsx`
 * covers each screen through the **component** it is built from rather than
 * its page, and this file mirrors that: `PaymentsScreen` and
 * `PaymentScreen` (the Refine-connected wrappers under `app/(dash)/`) are
 * intentionally absent, for the same reason.
 *
 * # The router problem
 *
 * `PaymentsFilters` and `AppShell` (and therefore `MoreMenu`) call
 * `next/navigation` hooks. `.storybook/main.ts` aliases that specifier to
 * `./next-navigation-mock.ts` — see that file's doc comment for what it
 * fixes and does not attempt.
 */
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";

import { AppShell } from "./app-shell";
import { EnrolmentPanel } from "./enrolment-panel";
import { FormAlert } from "./form-alert";
import { MoreMenu } from "./more-menu";
import { PasswordForm } from "./password-form";
import { PaymentDetailView } from "./payment-detail";
import { PaymentsFilters } from "./payments-filters";
import { PaymentsPager } from "./payments-pager";
import { PaymentsTable } from "./payments-table";
import { ReadFailure } from "./read-failure";
import { SignedInBar } from "./signed-in-bar";
import { SignInForm } from "./sign-in-form";
import { TimelineGap } from "./timeline-gap";
import { TotpForm } from "./totp-form";
import { DETAIL, INTENT, OTHER_INTENT } from "../testing/fixtures";
import type { FormAction } from "../form-state";

const NOOP_ASYNC = () => Promise.resolve();

/** A `FormAction` that never resolves — see the `Pending` stories below. */
function hangingAction(): FormAction {
  return () => new Promise(() => undefined);
}

/** A `FormAction` that resolves with no error, immediately. */
function cleanAction(): FormAction {
  return () => Promise.resolve({ error: null, requestId: null });
}

/** A `FormAction` that resolves refused, immediately. */
function refusedAction(message: string, requestId: string): FormAction {
  return () => Promise.resolve({ error: message, requestId });
}

const meta = {
  title: "Dashboard/Screens",
  parameters: {
    layout: "padded",
    a11y: { config: { rules: [{ id: "color-contrast", enabled: true }] } },
  },
  tags: ["autodocs"],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

// ---------------------------------------------------------------- SignInForm

export const SignInClean: Story = {
  render: () => <SignInForm action={cleanAction()} />,
};

export const SignInRefused: Story = {
  render: () => (
    <SignInForm action={refusedAction("Refused.", "req_01J8EXAMPLE")} />
  ),
};

/**
 * The pending state: `useActionState`'s `pending` disables every control and
 * changes the button's label while a submit is in flight
 * (`sign-in-form.tsx`'s own doc — a double-click must not submit twice). The
 * action here never resolves on purpose, and the `play` function submits the
 * form so the story actually reaches that state rather than describing it —
 * axe then runs against the real disabled-and-pending markup, not the idle
 * one. `a11y.test.tsx` does not cover this state at all (its "clean, refused
 * and pending" case only exercises the first two, despite its title); this
 * story is this app's first coverage of it anywhere.
 */
export const SignInPending: Story = {
  render: () => <SignInForm action={hangingAction()} />,
  play: async ({ canvasElement }) => {
    // Both fields carry `required`, and — unlike jsdom's `fireEvent.click`,
    // which does not enforce constraint validation — a real browser refuses
    // to fire `submit` on an empty required field. Measured: without these
    // two lines the click never submits at all and the story stays on
    // "Sign in", never reaching pending.
    const canvas = within(canvasElement);
    await userEvent.type(
      canvas.getByLabelText("Work email"),
      "ops@example.test",
    );
    await userEvent.type(canvas.getByLabelText("Password"), "correct-horse");
    await userEvent.click(canvas.getByRole("button", { name: /sign in/i }));
    await expect(
      canvas.getByRole("button", { name: /signing in/i }),
    ).toBeDisabled();
  },
};

// ------------------------------------------------------------------ TotpForm

export const TotpSignIn: Story = {
  render: () => <TotpForm action={cleanAction()} />,
};

export const TotpEnrolling: Story = {
  render: () => <TotpForm action={cleanAction()} enrolling />,
};

// ------------------------------------------------------------ EnrolmentPanel

export const Enrolment: Story = {
  render: () => (
    <EnrolmentPanel
      // A one-pixel PNG, exactly what `a11y.test.tsx` uses: this story is
      // about structure and colour contrast, and generating a real QR here
      // would put the `qrcode` package in the render path for nothing this
      // suite asserts.
      qrDataUrl="data:image/png;base64,iVBORw0KGgo="
      secret="GEZDGNBVGY3TQOJQ"
    />
  ),
};

// -------------------------------------------------------------- PasswordForm

export const PasswordChange: Story = {
  render: () => <PasswordForm action={cleanAction()} />,
};

// -------------------------------------------------------------- PaymentsTable

export const PaymentsRows: Story = {
  render: () => <PaymentsTable rows={[INTENT, OTHER_INTENT]} />,
};

export const PaymentsEmpty: Story = {
  render: () => <PaymentsTable rows={[]} />,
};

// ------------------------------------------------------------ PaymentsFilters

export const Filters: Story = {
  render: () => (
    <PaymentsFilters values={{ status: "", createdFrom: "", createdTo: "" }} />
  ),
};

/**
 * The `light` variant of one screen family, so `just test-storybook` gets at
 * least one automated data point on `@vaam-apps/ui`'s opt-in theme rather
 * than leaving it checked only by a human flipping the toolbar control —
 * see `.storybook/preview.ts`'s own doc comment for why only the stories
 * that set this are covered.
 */
export const FiltersLight: Story = {
  render: () => (
    <PaymentsFilters values={{ status: "", createdFrom: "", createdTo: "" }} />
  ),
  globals: { theme: "light" },
};

// -------------------------------------------------------------- PaymentsPager

export const PagerMiddle: Story = {
  render: () => (
    <PaymentsPager
      previousHref="/payments?ending_before=pi_2"
      nextHref="/payments?starting_after=pi_9"
    />
  ),
};

export const PagerFirst: Story = {
  render: () => (
    <PaymentsPager
      previousHref={null}
      nextHref="/payments?starting_after=pi_9"
    />
  ),
};

// -------------------------------------------------------------- SignedInBar

export const SignedIn: Story = {
  render: () => (
    <SignedInBar
      email="ada@example.test"
      merchantId="demo-merchant-tenant"
      signOut={NOOP_ASYNC}
    />
  ),
};

export const SignedInLight: Story = {
  render: () => (
    <SignedInBar
      email="ada@example.test"
      merchantId="demo-merchant-tenant"
      signOut={NOOP_ASYNC}
    />
  ),
  globals: { theme: "light" },
};

// --------------------------------------------------------- PaymentDetailView

export const DetailWithCharge: Story = {
  render: () => <PaymentDetailView detail={DETAIL} />,
};

export const DetailWithoutCharge: Story = {
  render: () => (
    <PaymentDetailView detail={{ ...DETAIL, charge: null, events: [] }} />
  ),
};

export const DetailWithChargeLight: Story = {
  render: () => <PaymentDetailView detail={DETAIL} />,
  globals: { theme: "light" },
};

// ---------------------------------------------------------------- MoreMenu

export const MoreMenuClosed: Story = {
  render: () => (
    <MoreMenu
      email="ops@example.test"
      merchantId="acct_test"
      signOut={NOOP_ASYNC}
      currentPath="/payments"
    />
  ),
};

/**
 * The drawer OPEN — `a11y.test.tsx` covers this deliberately, because a
 * closed vaul drawer renders none of its children (not hidden, absent), so
 * a story that only mounted `MoreMenu` idle would assert against an empty
 * portal. `MoreDetailDrawer` portals to `document.body`, not to this
 * story's own root, so the `play` function queries `document.body` directly
 * — the same reason `a11y.test.tsx` asserts against `document.body` rather
 * than a render container.
 */
export const MoreMenuOpen: Story = {
  render: () => (
    <MoreMenu
      email="ops@example.test"
      merchantId="acct_test"
      signOut={NOOP_ASYNC}
      currentPath="/payments"
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /more/i }));
    const body = within(canvasElement.ownerDocument.body);
    await expect(await body.findByRole("dialog")).toHaveAccessibleName("More");
  },
};

// ------------------------------------------------------------------ AppShell

/**
 * **`test: "todo"` here is a measured upstream defect, not a hidden gap** —
 * the same escape hatch `frontends/apps/checkout/.storybook/preview.ts`
 * documents ("A single story may override it... where a reviewer sees it").
 *
 * At this suite's own browser viewport (`@vitest/browser-playwright`'s
 * default, measured at 1200×900 by a temporary `console.log` in this play
 * function, since neither vitest nor this config states one), axe fails
 * `AppShell` with `landmark-unique`: **two** elements answer
 * `nav[aria-label="Primary"]` at once. `@vaam-apps/ui@0.1.2`'s own
 * `side-nav.js` doc comment asserts "Exactly one `<nav aria-label="Primary">`
 * exists at any width" and reasons through why — but the in-flow sidebar's
 * own className is `xl:flex xl:h-full … xl:w-64` with no UNPREFIXED `hidden`
 * alongside it, so below the `xl` breakpoint (1280px) none of those
 * `xl:`-prefixed utilities apply and the element falls back to a `<nav>`'s
 * browser-default `display: block` — visible, not absent — at the same time
 * the 1024–1279px floating vertical rail (`hidden sm:flex xl:hidden`) is
 * ALSO visible in that same band. Reproduced directly, not inferred from the
 * failure message: `axe.run(document, { runOnly: ["landmark-unique"] })`
 * against this exact built story at 1280×800 (above `xl`) returns zero
 * violations; at 1200×900 (below it) it returns this one, with both
 * `nav[aria-label="Primary"]` elements present in the accessibility tree.
 *
 * This is a defect in `@vaam-apps/ui`, a published dependency this repo does
 * not own the source of — not a gap in this app's markup, and not something
 * a Storybook config setting can honestly paper over. `parameters.a11y` is
 * scoped to just these two stories, not `preview.ts`'s default, so every
 * OTHER screen in this file still fails on a real violation.
 */
const SHELL_A11Y_TODO = {
  a11y: { test: "todo" as const },
};

export const Shell: Story = {
  render: () => (
    <AppShell
      email="ops@example.test"
      merchantId="acct_test"
      signOut={NOOP_ASYNC}
    >
      <PaymentsTable rows={[INTENT]} />
    </AppShell>
  ),
  parameters: SHELL_A11Y_TODO,
};

export const ShellLight: Story = {
  render: () => (
    <AppShell
      email="ops@example.test"
      merchantId="acct_test"
      signOut={NOOP_ASYNC}
    >
      <PaymentsTable rows={[INTENT]} />
    </AppShell>
  ),
  globals: { theme: "light" },
  parameters: SHELL_A11Y_TODO,
};

// ------------------------------------------------------------------ FormAlert

export const AlertVisible: Story = {
  render: () => <FormAlert error="Refused." requestId="req_01J8EXAMPLE" />,
};

// ---------------------------------------------------------------- ReadFailure

export const ReadFailed: Story = {
  render: () => (
    <ReadFailure
      failure={{
        status: 503,
        message: "vpay could not be reached.",
        requestId: "req_01J8",
      }}
    />
  ),
};

// --------------------------------------------------------------- TimelineGap

export const Gap: Story = {
  render: () => <TimelineGap />,
};
