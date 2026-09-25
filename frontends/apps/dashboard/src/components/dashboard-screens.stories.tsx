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
 * `PaymentsFilters` and `AppShell` call `next/navigation` hooks. `.storybook/main.ts` aliases that specifier to
 * `./next-navigation-mock.ts` — see that file's doc comment for what it
 * fixes and does not attempt.
 */
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ScreenStack } from "@vaam-apps/ui";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";

import { AppShell } from "./app-shell";
import { EnrolmentPanel } from "./enrolment-panel";
import { FormAlert } from "./form-alert";
import { PasswordForm } from "./password-form";
import { PaymentDetailHeader, PaymentDetailView } from "./payment-detail";
import { PaymentSkeleton } from "./payment-skeleton";
import { PaymentsFilters } from "./payments-filters";
import { PaymentsPager } from "./payments-pager";
import { PAYMENTS_DESCRIPTION, PaymentsSkeleton } from "./payments-skeleton";
import { PaymentsTable } from "./payments-table";
import { ReadFailure } from "./read-failure";
import { SignedInBar } from "./signed-in-bar";
import { SignInForm } from "./sign-in-form";
import { TimelineGap } from "./timeline-gap";
import { TotpForm } from "./totp-form";
import { NAV_ENTRIES } from "../dash/resources";
import { DETAIL, INTENT, OTHER_INTENT } from "../testing/fixtures";
import type { FormAction } from "../form-state";

const NOOP_ASYNC = () => Promise.resolve();

/**
 * A sign-out that records its calls, for the `Shell*` viewport stories to
 * prove a sign-out submits rather than only that it is there.
 * Module-level because a story's `render` and `play` have to share it.
 * Storybook's own loader restores every `fn()` spy before each story
 * renders (`parameters.test.restoreMocks`, on unless a story turns it
 * off), so each story's count starts at zero.
 */
const SIGN_OUT = fn(NOOP_ASYNC);

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

/**
 * The viewports stories here are tested at. The filter row and the loading
 * skeletons use two: a phone, where the row stacks, and a desktop, where it
 * is one line. The `Shell` stories use all four, one per shape of
 * `SideNav`'s: the phone bar (375), the vertical rail either side of `lg`
 * (1024px) in its 640–1279px band (700 and 1100), and the sidebar (1280).
 * `@storybook/addon-vitest` resizes the test browser to a story's
 * `globals.viewport` before running it; every story that sets none keeps
 * its 1200×900 default.
 *
 * Declared on `meta` rather than on the stories that use them, because
 * `a11y-gate.test.ts` pins which stories may carry a `parameters` override
 * at all (none, since the `Shell` stories' `landmark-unique` suppression
 * went with `@vaam-apps/ui` 0.4.0) and a viewport list is not a reason to
 * widen that. Declaring them here also makes them this file's viewport
 * menu in the Storybook toolbar.
 */
const VIEWPORTS = {
  phone: {
    name: "Phone, 375 × 812",
    styles: { width: "375px", height: "812px" },
    type: "mobile",
  },
  tablet: {
    name: "Tablet, 700 × 1024",
    styles: { width: "700px", height: "1024px" },
    type: "tablet",
  },
  laptop: {
    name: "Laptop, 1100 × 800",
    styles: { width: "1100px", height: "800px" },
    type: "desktop",
  },
  desktop: {
    name: "Desktop, 1280 × 800",
    styles: { width: "1280px", height: "800px" },
    type: "desktop",
  },
} as const;

const meta = {
  title: "Dashboard/Screens",
  parameters: {
    layout: "padded",
    a11y: { config: { rules: [{ id: "color-contrast", enabled: true }] } },
    viewport: { options: VIEWPORTS },
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

/** The one range both `FiltersWithRange` stories render. */
const FILTERED = {
  status: "",
  createdFrom: "2026-09-01",
  createdTo: "2026-09-07",
};

/** Round a box's top edge, so "on the same line" is an equality. */
function top(element: Element): number {
  return Math.round(element.getBoundingClientRect().top);
}

/**
 * The date field's trigger, found by the name its visible label gives it,
 * and the span that shows its value.
 */
async function dateField(canvasElement: HTMLElement) {
  const trigger = within(canvasElement).getByRole("button", {
    name: "Created between",
  });
  const value = trigger.querySelector("span");
  await expect(value).not.toBeNull();
  return { trigger, value: value as HTMLSpanElement };
}

/**
 * The filter row's three controls end inside its right edge, and neither
 * the row nor the page scrolls sideways — the row wraps rather than spill.
 */
async function nothingOffScreen(
  canvasElement: HTMLElement,
  controls: readonly HTMLElement[],
) {
  const row = canvasElement.firstElementChild as HTMLElement;
  const page = canvasElement.ownerDocument.documentElement;
  await expect(row.scrollWidth).toBeLessThanOrEqual(row.clientWidth);
  await expect(page.scrollWidth).toBeLessThanOrEqual(page.clientWidth);
  const edge = row.getBoundingClientRect().right;
  for (const control of controls) {
    await expect(control.getBoundingClientRect().right).toBeLessThanOrEqual(
      edge,
    );
  }
}

/**
 * The filters with a range set, at 1280×800: the date field at its longest
 * value, and the only desktop `Filters` story whose trigger carries a
 * description and a Clear button.
 *
 * The `play` function checks, in a real Chromium and against computed
 * geometry rather than class strings, what `payments-filters.tsx`'s module
 * doc claims and jsdom cannot see:
 *
 * - the trigger is named by its visible label and described by the dates;
 * - at 1280px the three controls are one line and nothing scrolls;
 * - the value is not truncated, and leaves at most 8px of its slot unused
 *   (it leaves ~4px: the root's tracking, which `ch` does not see);
 * - with the root at 20px (Chrome's "Large" default font size), which grows
 *   the trigger's rem-sized chrome and not its 14px value, the value is
 *   still not truncated;
 * - "Status" and "Created between" share a top edge, and so do their
 *   controls;
 * - squeezed to 496px, too narrow for one line (the app shell's content
 *   column at a 640px window), the row wraps — Apply goes down; whether the
 *   date field goes with it depends on the sans face, so that is not
 *   asserted — nothing ends past its edge, nothing scrolls sideways, and
 *   the field neither narrows nor truncates;
 * - cleared, the field keeps its width, so picking or clearing a range
 *   does not move the row.
 *
 * The mutations that fail it, each alone, are listed in that module doc.
 */
export const FiltersWithRange: Story = {
  render: () => <PaymentsFilters values={FILTERED} />,
  globals: { viewport: { value: "desktop", isRotated: false } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const { trigger, value } = await dateField(canvasElement);
    await expect(trigger).toHaveAccessibleDescription(
      "2026-09-01 → 2026-09-07",
    );
    const select = canvas.getByRole("combobox", { name: "Status" });
    const apply = canvas.getByRole("button", { name: "Apply" });
    const controls = [select, trigger, apply];

    await expect(top(trigger)).toBe(top(select));
    await expect(top(apply)).toBe(top(select));
    await nothingOffScreen(canvasElement, controls);

    await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    const text = canvasElement.ownerDocument.createRange();
    text.selectNodeContents(value);
    const unused =
      value.getBoundingClientRect().width - text.getBoundingClientRect().width;
    await expect(unused).toBeGreaterThanOrEqual(0);
    await expect(unused).toBeLessThanOrEqual(8);

    const status = canvas.getByText("Status", { selector: "label" });
    const created = canvas.getByText("Created between", { selector: "label" });
    await expect(top(status)).toBe(top(created));

    // Restored in `finally`: every later story in this page shares the root.
    const root = canvasElement.ownerDocument.documentElement;
    root.style.fontSize = "20px";
    try {
      await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    } finally {
      root.style.fontSize = "";
    }

    const width = trigger.getBoundingClientRect().width;
    canvasElement.style.width = "496px";
    await expect(top(apply)).toBeGreaterThan(top(select));
    await nothingOffScreen(canvasElement, controls);
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

/**
 * The same filters at 375×812, where the row stacks: a filter an operator
 * has to scroll sideways to discover is worse on a phone than a short
 * stack.
 *
 * The `play` function checks it at the story's own width and squeezed to
 * the app shell's content column at a 375px and a 320px window (327px and
 * 272px): nothing ends past the row's right edge, neither the row nor the
 * page scrolls sideways, the date field sits on a line below Status, and
 * the range is not truncated. The row going back to one line that scrolls
 * (`flex-nowrap` with `overflow-x-auto`) fails it.
 *
 * Then at the column of a 280px window, 232px, narrower than the date
 * field's own 265.9px: the field shrinks to the column and truncates its
 * value rather than push the page sideways, so the same "nothing ends past
 * the edge, nothing scrolls" holds, and the dates stay whole in the
 * trigger's description. Dropping `max-w-full` from the trigger or from its
 * `FormField` fails it.
 *
 * Last, the same 232px column with the root at 20px (Chrome's "Large"
 * default font size), where the Status select's widest option outgrows the
 * column too: capped by its `FormField`'s `max-w-full`, nothing ends past
 * the edge. Dropping that `max-w-full` fails it.
 */
export const FiltersWithRangePhone: Story = {
  render: () => <PaymentsFilters values={FILTERED} />,
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const { trigger, value } = await dateField(canvasElement);
    const select = canvas.getByRole("combobox", { name: "Status" });
    const apply = canvas.getByRole("button", { name: "Apply" });

    for (const width of ["", "327px", "272px"]) {
      canvasElement.style.width = width;
      await nothingOffScreen(canvasElement, [select, trigger, apply]);
      await expect(top(trigger)).toBeGreaterThan(top(select));
      await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    }

    canvasElement.style.width = "232px";
    await nothingOffScreen(canvasElement, [select, trigger, apply]);
    await expect(trigger).toHaveAccessibleDescription(
      "2026-09-01 → 2026-09-07",
    );

    // Restored in `finally`: every later story in this page shares the root.
    const root = canvasElement.ownerDocument.documentElement;
    root.style.fontSize = "20px";
    try {
      await nothingOffScreen(canvasElement, [select, trigger, apply]);
    } finally {
      root.style.fontSize = "";
      canvasElement.style.width = "";
    }
  },
};

// ------------------------------------------------------------ PaymentsSkeleton

/**
 * The payments screen's loading state above the part of its loaded state
 * the skeleton stands in for — the heading, the description and the filter
 * row, composed as `payments-screen.tsx`'s loaded branch composes them.
 */
function SkeletonAboveScreen() {
  return (
    <div className="flex flex-col gap-12">
      <PaymentsSkeleton />
      <ScreenStack>
        <h2>Payments</h2>
        <p className="text-body text-muted-foreground">
          {PAYMENTS_DESCRIPTION}
        </p>
        <PaymentsFilters
          values={{ status: "", createdFrom: "", createdTo: "" }}
        />
      </ScreenStack>
    </div>
  );
}

/**
 * Where a filter row's boxes sit in their own screen stack: the row and
 * each of its three children, as offsets from the stack's top-left corner
 * and sizes, and how many lines the row wrapped onto (its children are
 * `items-end`, so one line is one shared bottom edge).
 */
function rowLayout(stack: Element, row: Element) {
  const origin = stack.getBoundingClientRect();
  const boxes = [row, ...row.children].map((element) => {
    const box = element.getBoundingClientRect();
    return [
      box.left - origin.left,
      box.top - origin.top,
      box.width,
      box.height,
    ];
  });
  const bottoms = [...row.children].map((child) =>
    Math.round(child.getBoundingClientRect().bottom),
  );
  return { lines: new Set(bottoms).size, boxes };
}

/**
 * The skeleton's filter row lands on the real one: the same number of
 * lines, and every box within a pixel, at each column width given and
 * then again at a 20px root. The canvas width and the root are restored
 * in `finally`, since every later story in this page shares them.
 */
async function skeletonLandsOnTheRow(
  canvasElement: HTMLElement,
  widths: readonly string[],
) {
  // Found by what they are, not by the layout classes under test.
  const canvas = within(canvasElement);
  const loading = canvasElement.querySelector('[role="status"]');
  const skeleton = loading?.querySelector("[inert]");
  const select = canvas.getByRole("combobox", { name: "Status" });
  const stack = canvas.getByRole("heading", { name: "Payments" }).parentElement;
  const row = [...(stack?.children ?? [])].find((child) =>
    child.contains(select),
  );
  await expect(loading).not.toBeNull();
  await expect(skeleton).not.toBeNull();
  await expect(row).toBeDefined();

  // The select and the button inside the skeleton are there to be measured,
  // never seen or reached: unpainted, and focus does not land on them.
  const copies = [...(skeleton?.querySelectorAll("select, button") ?? [])];
  await expect(copies).toHaveLength(2);
  for (const copy of copies) {
    await expect(getComputedStyle(copy).visibility).toBe("hidden");
    (copy as HTMLElement).focus();
    await expect(copy.ownerDocument.activeElement).not.toBe(copy);
  }
  const root = canvasElement.ownerDocument.documentElement;
  try {
    for (const rootSize of ["", "20px"]) {
      root.style.fontSize = rootSize;
      for (const width of widths) {
        canvasElement.style.width = width;
        const expected = rowLayout(stack as Element, row as Element);
        const actual = rowLayout(loading as Element, skeleton as Element);
        const where = `root ${rootSize || "16px"}, column ${width || "full"}`;
        await expect(actual.lines, where).toBe(expected.lines);
        const off = actual.boxes.flatMap((box, i) =>
          box.map((value, j) =>
            Math.abs(value - (expected.boxes[i]?.[j] ?? 0)),
          ),
        );
        await expect(
          Math.max(...off),
          `${where}: skeleton ${JSON.stringify(actual.boxes)}, row ${JSON.stringify(expected.boxes)}`,
        ).toBeLessThanOrEqual(1);
      }
    }
  } finally {
    root.style.fontSize = "";
    canvasElement.style.width = "";
  }
}

/**
 * The payments screen's loading state, at 1280×800, above the loaded
 * heading, description and filter row it stands in for.
 *
 * The `play` function is the guard on `PaymentsSkeleton` and
 * `PaymentsFiltersSkeleton`: at the story's own width and squeezed to the
 * app shell's content column at 726, 700 and 640px windows (582, 556 and
 * 496px: one line, then two with Apply wrapped, then two however the sans
 * face breaks them), and again at a 20px root, the skeleton's filter row
 * has as many lines as the real row and every box within a pixel of it.
 * The mutations that fail it are in `payments-skeleton.tsx`'s module doc.
 */
export const PaymentsLoading: Story = {
  render: () => <SkeletonAboveScreen />,
  globals: { viewport: { value: "desktop", isRotated: false } },
  play: async ({ canvasElement }) => {
    await skeletonLandsOnTheRow(canvasElement, ["", "582px", "556px", "496px"]);
  },
};

/**
 * The same at 375×812, where the real row is three lines and the
 * description two: at the story's own width and squeezed to the shell's
 * column at 375, 320 and 280px windows (327, 272 and 232px, the last where
 * the date field truncates), at both roots.
 */
export const PaymentsLoadingPhone: Story = {
  render: () => <SkeletonAboveScreen />,
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    await skeletonLandsOnTheRow(canvasElement, ["", "327px", "272px", "232px"]);
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

// ------------------------------------------------------------- PaymentSkeleton

/**
 * One payment's loading state above the loaded page it stands in for —
 * `PaymentDetailHeader` and `PaymentDetailView` in a `ScreenStack`, as
 * `payment-screen.tsx`'s loaded branch composes them.
 */
function PaymentSkeletonAbovePage() {
  return (
    <div className="flex flex-col gap-12">
      <PaymentSkeleton />
      <ScreenStack>
        <PaymentDetailHeader />
        <PaymentDetailView detail={DETAIL} />
      </ScreenStack>
    </div>
  );
}

/**
 * The boxes a payment's loading state claims, each as an offset from its
 * own screen stack's top-left corner and a size: the header, the panel,
 * the panel's title-and-caption block, its figure row, each figure, the
 * Summary section's position and width (not its height: the rows under
 * the heading are data), and the Summary heading.
 *
 * Both stacks are read by position, the same position in each: the page
 * is `[header, view]` and the skeleton `[Loading…, header, view]`, where
 * `view` is `[panel, Summary, …]`, `panel` is `[title and caption,
 * figures]` and `Summary` is `[heading, rows]`.
 */
function pageBoxes(stack: Element, header: Element) {
  const origin = stack.getBoundingClientRect();
  const at = (element: Element | undefined, withHeight = true) => {
    const box = (element as Element).getBoundingClientRect();
    const edges = [
      box.left - origin.left,
      box.top - origin.top,
      box.width,
      box.height,
    ];
    return withHeight ? edges : edges.slice(0, 3);
  };
  const view = header.nextElementSibling as Element;
  const [panel, summary] = [...view.children];
  const [titled, figures] = [...(panel as Element).children];
  return [
    at(header),
    at(panel),
    at(titled),
    at(figures),
    ...[...(figures as Element).children].map((figure) => at(figure)),
    at(summary, false),
    at((summary as Element).firstElementChild as Element),
  ];
}

/**
 * The payment skeleton lands on the page: every claimed box within a
 * pixel of the loaded one, at each column width given and then again at a
 * 20px root. The canvas width and the root are restored in `finally`,
 * since every later story in this page shares them.
 */
async function paymentSkeletonLandsOnThePage(
  canvasElement: HTMLElement,
  widths: readonly string[],
) {
  // Found by what they are, not by the layout classes under test.
  const canvas = within(canvasElement);
  const loading = canvasElement.querySelector(
    '[role="status"][aria-busy="true"]',
  );
  const heading = canvas.getByRole("heading", { name: "Payment" });
  const header = heading.parentElement as Element;
  const page = header.parentElement as Element;
  await expect(loading).not.toBeNull();
  await expect(header.tagName).toBe("HEADER");
  const skeletonHeader = (loading as Element).children[1] as Element;

  const root = canvasElement.ownerDocument.documentElement;
  try {
    for (const rootSize of ["", "20px"]) {
      root.style.fontSize = rootSize;
      for (const width of widths) {
        canvasElement.style.width = width;
        const expected = pageBoxes(page, header);
        const actual = pageBoxes(loading as Element, skeletonHeader);
        const where = `root ${rootSize || "16px"}, column ${width || "full"}`;
        await expect(actual.length, where).toBe(expected.length);
        const off = actual.flatMap((box, i) =>
          box.map((value, j) =>
            Math.abs(value - (expected[i]?.[j] ?? Number.NaN)),
          ),
        );
        await expect(
          Math.max(...off),
          `${where}: skeleton ${JSON.stringify(actual)}, page ${JSON.stringify(expected)}`,
        ).toBeLessThanOrEqual(1);
      }
    }
  } finally {
    root.style.fontSize = "";
    canvasElement.style.width = "";
  }
}

/**
 * One payment's loading state, at 1280×800, above the loaded header and
 * view it stands in for.
 *
 * The `play` function is the guard on `PaymentSkeleton`: at the story's
 * own width and squeezed to the app shell's content column at 1100 and
 * 640px windows (956 and 496px), and again at a 20px root, the header,
 * the panel and each of its parts, and the Summary heading are within a
 * pixel of the loaded page's. The mutations that fail it are in
 * `payment-skeleton.tsx`'s module doc.
 */
export const PaymentLoading: Story = {
  render: () => <PaymentSkeletonAbovePage />,
  globals: { viewport: { value: "desktop", isRotated: false } },
  play: async ({ canvasElement }) => {
    await paymentSkeletonLandsOnThePage(canvasElement, ["", "956px", "496px"]);
  },
};

/**
 * The same at 375×812, where the panel's caption is two lines and its
 * three figures wrap: at the story's own width and squeezed to the
 * shell's column at 375 and 320px windows (327 and 272px — two figure
 * lines, then three), at both roots.
 */
export const PaymentLoadingPhone: Story = {
  render: () => <PaymentSkeletonAbovePage />,
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    await paymentSkeletonLandsOnThePage(canvasElement, ["", "327px", "272px"]);
  },
};

// ------------------------------------------------------------------ AppShell

/**
 * Twenty rows: taller than every viewport below, so the phone story's
 * scroll to the end is a real scroll and the last row's clearance is a
 * measurement rather than a page too short to reach the bar.
 */
const MANY_INTENTS = Array.from({ length: 20 }, (_, row) => ({
  ...INTENT,
  id: `pi_example_${row + 1}`,
}));

/** The shell as each viewport story renders it. */
function shellStory() {
  return (
    <AppShell
      email="ops@example.test"
      merchantId="acct_test"
      signOut={SIGN_OUT}
    >
      <PaymentsTable rows={MANY_INTENTS} />
    </AppShell>
  );
}

/**
 * `AppShell` at the suite's own 1200×900, with nothing suppressed.
 *
 * **These two stories disabled axe's `landmark-unique` from 2026-09-13 to
 * 2026-09-24**, narrowed to that one rule: below 1280px `@vaam-apps/ui`'s
 * in-flow sidebar `<nav aria-label="Primary">` kept a `display: block` box
 * beside whichever floating rail was showing, so axe saw two `Primary`
 * landmarks at every width under `xl` (vaam-apps/ui#16, filed from this
 * file with a four-width reproduction; still reproducing on 0.3.0 at 375,
 * 640, 1023, 1200 and 1279). 0.4.0 fixed it upstream (vaam-apps/ui#39)
 * and the suppression is gone: axe runs every rule here, and over the
 * four `Shell*` viewport stories below, and `a11y-gate.test.ts` now pins
 * the set of suppressions as empty.
 *
 * Before that, these carried `a11y: { test: "todo" }`, which switches the
 * addon off entirely for the story; a `#3a3a3a`-on-`#0a0b0d` probe inside
 * `Shell` passed under it and failed at 1.73 with the addon on (review,
 * 2026-09-13). That is why a narrowed rule list replaced it, and why no
 * story here may carry a `parameters` override at all now.
 */
export const Shell: Story = {
  render: () => shellStory(),
};

export const ShellLight: Story = {
  render: () => shellStory(),
  globals: { theme: "light" },
};

/**
 * What every width must hold: exactly one exposed `Primary` landmark, the
 * given number of exposed sign-outs, and a page that does not scroll
 * sideways. `queryAllByRole` leaves out what `display: none` or an
 * `aria-hidden` ancestor hides, which is the point — jsdom cannot answer
 * either count, because it applies no CSS and sees every copy.
 *
 * The sign-outs: none on a phone until the sheet opens (`<main>`'s copy is
 * `hidden sm:block`), one from `sm` up (`<main>`'s, then the sidebar's
 * from `xl`), and one with a sheet open, which hides the rest of the page.
 * Never two.
 */
async function shellHolds(body: HTMLElement, signOuts: 0 | 1) {
  const page = within(body);
  await expect(
    page.queryAllByRole("navigation", { name: "Primary" }),
  ).toHaveLength(1);
  await expect(
    page.queryAllByRole("button", { name: /sign out/i }),
  ).toHaveLength(signOuts);
  const root = body.ownerDocument.documentElement;
  await expect(root.scrollWidth).toBeLessThanOrEqual(root.clientWidth);
}

/**
 * Close the open More sheet with Escape and wait for it to go: vaul keeps
 * it mounted, `data-state="closed"`, through its exit animation, and the
 * page stays hidden from the accessibility tree until it unmounts. Then
 * focus is back on the control that opened it.
 */
async function closeSheet(body: HTMLElement, more: HTMLElement) {
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(within(body).queryByRole("dialog", { name: "More" })).toBeNull(),
  );
  await expect(body.ownerDocument.activeElement).toBe(more);
}

/**
 * Open the toolbar's More sheet with the pointer, check it holds the
 * account block, switch the theme and sign out from inside it, then close
 * it with Escape and check focus went back to More.
 */
async function moreSheetWorks(body: HTMLElement) {
  const page = within(body);
  const more = page.getByRole("button", { name: "More" });
  await userEvent.click(more);
  const sheet = await page.findByRole("dialog", { name: "More" });
  const inSheet = within(sheet);
  await expect(inSheet.getByText("ops@example.test")).toBeVisible();
  // Modal: the rest of the page, rails and `<main>` included, has left the
  // accessibility tree, so the sheet's sign-out is the only one exposed.
  await expect(
    page.queryAllByRole("button", { name: /sign out/i }),
  ).toHaveLength(1);

  // The theme, by pointer, and back — `html[data-theme]` is what the
  // stylesheet keys on.
  const root = body.ownerDocument.documentElement;
  await userEvent.click(inSheet.getByRole("radio", { name: /light/i }));
  await expect(root.dataset["theme"]).toBe("light");
  await userEvent.click(inSheet.getByRole("radio", { name: /dark/i }));
  await expect(root.dataset["theme"]).toBe("dark");

  // Sign out, by pointer: the form's action runs.
  const signOutButton = inSheet.getByRole("button", { name: /sign out/i });
  await userEvent.click(signOutButton);
  await expect(SIGN_OUT).toHaveBeenCalledTimes(1);

  await closeSheet(body, more);
}

/**
 * Open the sheet again from the keyboard (focus is already on More), reach
 * sign-out with Tab and press it with Enter, and leave the sheet OPEN so
 * the a11y addon's axe run, which follows `play`, covers its markup.
 */
async function moreSheetByKeyboard(body: HTMLElement) {
  await userEvent.keyboard("{Enter}");
  const sheet = await within(body).findByRole("dialog", { name: "More" });
  const signOutButton = within(sheet).getByRole("button", {
    name: /sign out/i,
  });
  for (let presses = 0; presses < 20; presses += 1) {
    if (body.ownerDocument.activeElement === signOutButton) break;
    await userEvent.tab();
  }
  await expect(body.ownerDocument.activeElement).toBe(signOutButton);
  await userEvent.keyboard("{Enter}");
  await expect(SIGN_OUT).toHaveBeenCalledTimes(2);
}

/**
 * The phone bar, at 375×812. Below 640px the account block is behind the
 * bar's "More" control, in a bottom sheet (`@vaam-apps/ui` 0.4.0), and
 * this app no longer floats a Menu pill of its own above the bar.
 *
 * The `play` function scrolls to the end of twenty rows and checks the
 * last one ends at least 16px above the bar's top edge (it ends 24px
 * above: `<main>`'s `pb-20` clears the bar and the column's `p-6` is the
 * spare). In the Storybook UI the story's `padded` layout adds 16px of
 * body padding the app does not have, so the body's padding is zeroed for
 * the measurement and restored after. Then
 * it opens the sheet by pointer and by keyboard (`moreSheetWorks`,
 * `moreSheetByKeyboard`), and axe runs over the open sheet.
 */
export const ShellPhone: Story = {
  render: () => shellStory(),
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    const body = canvasElement.ownerDocument.body;
    await shellHolds(body, 0);

    const bar = body.querySelector('[data-floating-rail-axis="horizontal"]');
    const rows = canvasElement.querySelectorAll("tbody tr");
    const last = rows[rows.length - 1];
    await expect(bar).not.toBeNull();
    await expect(rows).toHaveLength(MANY_INTENTS.length);
    const view = canvasElement.ownerDocument.defaultView as Window;
    const padding = body.style.padding;
    try {
      body.style.padding = "0px";
      view.scrollTo(0, body.ownerDocument.documentElement.scrollHeight);
      const clearance =
        (bar as Element).getBoundingClientRect().top -
        (last as Element).getBoundingClientRect().bottom;
      await expect(clearance).toBeGreaterThanOrEqual(16);
    } finally {
      body.style.padding = padding;
      view.scrollTo(0, 0);
    }

    await moreSheetWorks(body);
    await moreSheetByKeyboard(body);
  },
};

/**
 * The vertical rail at 700×1024, the narrow end of its 640–1279px band.
 * Its "More" control opens a navigation drawer from the left edge holding
 * every destination, labelled, then the account block.
 *
 * The `play` function checks `<main>`'s left padding ends at least 16px
 * past the rail's right edge — `sm:pl-24` (96px) against a rail ending at
 * x=80, the gutter 0.3.0's release notes asked for — then that the drawer
 * lists every `NAV_ENTRIES` destination, in order, and works by pointer
 * and keyboard.
 */
export const ShellRail: Story = {
  render: () => shellStory(),
  globals: { viewport: { value: "tablet", isRotated: false } },
  play: async ({ canvasElement }) => {
    const body = canvasElement.ownerDocument.body;
    await shellHolds(body, 1);
    await railGutterHolds(canvasElement);

    const more = within(body).getByRole("button", { name: "More" });
    await userEvent.click(more);
    const sheet = await within(body).findByRole("dialog", { name: "More" });
    await expect(
      within(sheet)
        .getAllByRole("link")
        .map((link) => link.getAttribute("href")),
    ).toEqual(NAV_ENTRIES.map((entry) => entry.href));
    await closeSheet(body, more);

    await moreSheetWorks(body);
    await moreSheetByKeyboard(body);
  },
};

/**
 * `<main>`'s content box starts at least 16px past the vertical rail's
 * right edge. Measured with the body's padding zeroed, as `ShellPhone`'s
 * clearance is: the rail is `fixed` to the viewport and does not move with
 * it, so the Storybook UI's `padded` layout would hand `<main>` 16px the
 * app does not have.
 */
async function railGutterHolds(canvasElement: HTMLElement) {
  const body = canvasElement.ownerDocument.body;
  const rail = body.querySelector(
    '[data-floating-rail]:not([data-floating-rail-axis="horizontal"])',
  );
  const main = canvasElement.querySelector("main");
  await expect(rail).not.toBeNull();
  await expect(main).not.toBeNull();
  const padding = body.style.padding;
  try {
    body.style.padding = "0px";
    const start =
      (main as HTMLElement).getBoundingClientRect().left +
      Number.parseFloat(getComputedStyle(main as HTMLElement).paddingLeft);
    const railBox = (rail as Element).getBoundingClientRect();
    // A hidden element measures 0 wide at x=0, and the gutter below would
    // then pass without measuring anything, e.g. if the selector ever
    // matched the phone bar (hidden from 640px) instead of the rail.
    await expect(railBox.width).toBeGreaterThan(0);
    await expect(start - railBox.right).toBeGreaterThanOrEqual(16);
  } finally {
    body.style.padding = padding;
  }
}

/**
 * The vertical rail at 1100×800, above `lg` in its band. On 0.3.0 this
 * width exposed vaam-apps/ui#16's second `Primary` landmark, as every width
 * from 375 to 1279px did. Same checks as `ShellRail`, and the identity is on
 * screen in `<main>` without a tap.
 */
export const ShellLaptop: Story = {
  render: () => shellStory(),
  globals: { viewport: { value: "laptop", isRotated: false } },
  play: async ({ canvasElement }) => {
    const body = canvasElement.ownerDocument.body;
    await shellHolds(body, 1);
    await railGutterHolds(canvasElement);
    const main = within(canvasElement.querySelector("main") as HTMLElement);
    await expect(main.getByText("ops@example.test")).toBeVisible();

    await moreSheetWorks(body);
    await moreSheetByKeyboard(body);
  },
};

/**
 * The sidebar at 1280×800: the account block is in flow under the nav,
 * no More control is exposed, and sign-out there submits.
 */
export const ShellDesktop: Story = {
  render: () => shellStory(),
  globals: { viewport: { value: "desktop", isRotated: false } },
  play: async ({ canvasElement }) => {
    const body = canvasElement.ownerDocument.body;
    await shellHolds(body, 1);
    const page = within(body);
    await expect(page.queryByRole("button", { name: "More" })).toBeNull();
    const nav = page.getByRole("navigation", { name: "Primary" });
    await expect(within(nav).getByText("ops@example.test")).toBeVisible();
    await userEvent.click(
      within(nav).getByRole("button", { name: /sign out/i }),
    );
    await expect(SIGN_OUT).toHaveBeenCalledTimes(1);
  },
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
