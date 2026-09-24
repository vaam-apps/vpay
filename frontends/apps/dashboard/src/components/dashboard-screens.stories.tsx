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

/**
 * The filters with a range set: the date field at its longest value, and
 * the only one of these three stories whose trigger carries a description
 * and a Clear button.
 *
 * The `play` function checks, in a real Chromium and against computed
 * geometry rather than class strings, what `payments-filters.tsx`'s module
 * doc claims and jsdom cannot see:
 *
 * - the trigger is named by its visible label and described by the dates;
 * - the value is not truncated, and leaves at most 8px of its slot unused
 *   (it leaves ~4px: the root's tracking, which `ch` does not see);
 * - "Status" and "Created between" share a top edge, and so do their
 *   controls;
 * - squeezed to a 375px phone's content width, the row stays one line and
 *   scrolls, and the field neither narrows nor truncates;
 * - cleared, the field keeps its width, so picking or clearing a range
 *   does not move the row.
 *
 * The mutations that fail it, each alone, are listed in that module doc.
 */
export const FiltersWithRange: Story = {
  render: () => (
    <PaymentsFilters
      values={{
        status: "",
        createdFrom: "2026-09-01",
        createdTo: "2026-09-07",
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const trigger = canvas.getByRole("button", { name: "Created between" });
    await expect(trigger).toHaveAccessibleDescription(
      "2026-09-01 → 2026-09-07",
    );

    const shown = trigger.querySelector("span");
    await expect(shown).not.toBeNull();
    const value = shown as HTMLSpanElement;
    await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    const text = canvasElement.ownerDocument.createRange();
    text.selectNodeContents(value);
    const unused =
      value.getBoundingClientRect().width - text.getBoundingClientRect().width;
    await expect(unused).toBeGreaterThanOrEqual(0);
    await expect(unused).toBeLessThanOrEqual(8);

    const top = (element: Element) =>
      Math.round(element.getBoundingClientRect().top);
    const status = canvas.getByText("Status", { selector: "label" });
    const created = canvas.getByText("Created between", { selector: "label" });
    await expect(top(status)).toBe(top(created));
    const select = canvas.getByRole("combobox", { name: "Status" });
    await expect(top(select)).toBe(top(trigger));

    const width = trigger.getBoundingClientRect().width;
    const row = canvasElement.firstElementChild as HTMLElement;
    const apply = canvas.getByRole("button", { name: "Apply" });
    canvasElement.style.width = "343px";
    await expect(row.scrollWidth).toBeGreaterThan(row.clientWidth);
    await expect(top(apply)).toBe(top(select));
    await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    await expect(trigger.getBoundingClientRect().width).toBe(width);
    canvasElement.style.width = "";

    await userEvent.click(
      canvas.getByRole("button", { name: "Clear the dates" }),
    );
    await expect(trigger).toHaveAccessibleDescription("Any time");
    await expect(trigger.getBoundingClientRect().width).toBe(width);
  },
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
    await userEvent.click(canvas.getByRole("button", { name: /^menu$/i }));
    const body = within(canvasElement.ownerDocument.body);
    await expect(await body.findByRole("dialog")).toHaveAccessibleName("Menu");
  },
};

// ------------------------------------------------------------------ AppShell

/**
 * **`landmark-unique` alone is turned off here, and nothing else** — a
 * measured defect in a published dependency, narrowed to the single rule it
 * affects so the rest of axe still gates this app's whole chrome.
 *
 * At this suite's own browser viewport — `@vitest/browser-playwright`'s
 * default, measured at **1200x900** (a temporary `play` function asserting
 * `window.innerWidth`/`innerHeight`, since neither vitest nor this config
 * states one) — axe fails `AppShell` with `landmark-unique`: **two** visible
 * elements answer `nav[aria-label="Primary"]` at once.
 *
 * Reproduced directly against the BUILT story with
 * `axe.run(document, { runOnly: ["landmark-unique"] })` at four widths:
 * 1200 and 1279 return this one violation, 1280 and 1440 return none. At
 * 1200 the in-flow sidebar computes `display: block` and the floating
 * vertical rail computes `display: flex` — both visible, both
 * `nav[aria-label="Primary"]`. At 1280 the sidebar is `flex` and the rail is
 * `none`.
 *
 * The markup is upstream, not this app's composition:
 * `@vaam-apps/ui@0.1.2`'s
 * `dist/components/primitives/side-nav.js:456` emits the in-flow sidebar's
 * className as `collapsed ? "hidden" : "xl:flex xl:h-full ... xl:py-4"` —
 * every utility `xl:`-prefixed and no UNPREFIXED `hidden` beside them, so
 * below 1280px the element keeps a `<nav>`'s default `display: block` while
 * the `hidden sm:flex xl:hidden` floating rail is also on. `app-shell.tsx`
 * passes `smallScreen`'s default (`"floating"`) and writes none of those
 * classes itself. It is a real defect that the shipped app has at every
 * width below 1280px, not a Storybook artefact — filing it upstream is the
 * fix; hiding it is not.
 *
 * **Filed 2026-09-13: vaam-apps/ui#16**, with the measurement below and a
 * suggested one-word fix (an unprefixed `hidden` beside the `xl:`
 * utilities). Re-measured across the full range when filing, and the gap
 * is WIDER than first recorded: two visible `nav[aria-label="Primary"]`
 * at **640, 1023, 1200 and 1279**, one at 1280 and 1440 — every width
 * below `xl`, not just the 1200-1279 band. Drop this suppression when a
 * release carrying the fix is picked up; `a11y-gate.test.ts` pins the
 * rule id, so removing it here without removing it there fails the gate.
 *
 * **Re-measured on 0.3.0 (2026-09-24), and still needed.** 0.3.0 rewrote
 * the rails as M3 floating toolbars but not this: the same
 * `axe.run(document, { runOnly: ["landmark-unique"] })` against the built
 * story returns one violation at 375, 640, 1023, 1200 and 1279 and none at
 * 1280 and 1440, and below 1280 the in-flow `<nav>` still computes
 * `display: block`. vaam-apps/ui#16 is still open.
 *
 * **What changed on review (2026-09-13):** these two stories carried
 * `a11y: { test: "todo" }`, which switches the addon off ENTIRELY for the
 * story. Measured: a `#3a3a3a`-on-`#0a0b0d` probe placed inside `Shell`
 * PASSED under `test: "todo"` — so `AppShell`, the chrome wrapped around
 * every screen in the app, was the one composition in this file with no
 * colour-contrast verdict at all. Disabling the single upstream rule keeps
 * `test: "error"` in force: the same probe FAILS at 1.73 against #0a0b0d
 * with the config below. `src/a11y-gate.test.ts` pins this exact set of two
 * stories and this exact rule id, so a third suppression cannot be added
 * without the gate failing.
 */
const SHELL_A11Y_UPSTREAM_LANDMARK = {
  a11y: {
    config: {
      rules: [
        // Re-stated because a story-level `rules` array REPLACES the meta's
        // rather than merging into it.
        { id: "color-contrast", enabled: true },
        { id: "landmark-unique", enabled: false },
      ],
    },
  },
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
  parameters: SHELL_A11Y_UPSTREAM_LANDMARK,
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
  parameters: SHELL_A11Y_UPSTREAM_LANDMARK,
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
