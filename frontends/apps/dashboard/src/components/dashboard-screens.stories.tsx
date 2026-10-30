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
import { expect, within } from "storybook/test";

import { AccountPage } from "./account-page";
import { CenteredShell } from "./centered-shell";
import { EnrolmentPanel } from "./enrolment-panel";
import { FilterChips } from "./filter-chips";
import { PageHeader } from "./layout";
import { MintRefused } from "./mint-refused";
import {
  NotConfigured,
  PairedKeyOnly,
  SignInUnreachable,
} from "./login-notices";
import { PasswordForm } from "./password-form";
import { SignInForm } from "./sign-in-form";
import { TotpForm } from "./totp-form";
import { PaymentDetailHeader, PaymentDetailView } from "./payment-detail";
import { PaymentSkeleton } from "./payment-skeleton";
import { PaymentsFilters } from "./payments-filters";
import { PaymentsPager } from "./payments-pager";
import { PAYMENTS_DESCRIPTION, PaymentsSkeleton } from "./payments-skeleton";
import { PaymentsTable } from "./payments-table";
import { ProcedurePager } from "./procedure-pager";
import { ReadFailure } from "./read-failure";
import { TimelineGap } from "./timeline-gap";
import { NO_ERROR } from "../form-state";
import { DETAIL, INTENT, OTHER_INTENT } from "../testing/fixtures";

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

/** The status chip row, by its accessible name. */
function statusChips(canvasElement: HTMLElement): HTMLElement {
  return within(canvasElement).getByRole("navigation", {
    name: "Filter payments by status",
  });
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
 * - nothing ends past the row's edge and nothing scrolls sideways;
 * - the value is not truncated, and leaves at most 8px of its slot unused
 *   (it leaves ~4px: the root's tracking, which `ch` does not see);
 * - with the root at 20px (Chrome's "Large" default font size), which grows
 *   the trigger's rem-sized chrome and not its 14px value, the value is
 *   still not truncated;
 * - squeezed to 496px (the app shell's content column at a 640px window),
 *   the chips wrap rather than spill, nothing scrolls sideways, and the
 *   field neither narrows nor truncates;
 *
 * The Status select and Apply button this story used to measure are gone
 * (2026-10-21): status is a row of filter chips that are links, and a
 * range applies on the picker's Save.
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
    const chips = statusChips(canvasElement);
    const controls = [chips, trigger];
    await nothingOffScreen(canvasElement, controls);

    await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    const text = canvasElement.ownerDocument.createRange();
    text.selectNodeContents(value);
    const unused =
      value.getBoundingClientRect().width - text.getBoundingClientRect().width;
    await expect(unused).toBeGreaterThanOrEqual(0);
    await expect(unused).toBeLessThanOrEqual(8);

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
    await nothingOffScreen(canvasElement, controls);
    await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    await expect(trigger.getBoundingClientRect().width).toBe(width);
    canvasElement.style.width = "";
    // Clearing is no longer measured here: it navigates (the URL is the
    // filter's only state), and this story's router is a no-op, so the
    // field keeps the dates it was rendered with. `payments-filters.test.tsx`
    // holds what Clear pushes.
    await expect(
      canvas.getByRole("button", { name: "Clear the dates" }),
    ).toBeVisible();
  },
};

/**
 * The same filters at 375×812, where the row stacks: a filter an operator
 * has to scroll sideways to discover is worse on a phone than a short
 * stack.
 *
 * The `play` function checks it at the story's own width and squeezed to
 * the app shell's content column at a 375px window (343px): nothing ends
 * past the row's right edge, neither the row nor the page scrolls
 * sideways, the date field sits below the chips, and the range is not
 * truncated. Then at a 232px column, narrower than the date field's own
 * width: the field shrinks to the column rather than push the page
 * sideways, and the dates stay whole in the trigger's description.
 *
 * At a 320px window (288px column) the field is wider than the column and
 * truncates its value since 2026-10-21: the self-hosted JetBrains Mono's
 * dates need 25ch where the system monospace fitted in 23, and the field
 * was widened to fit them. The full range is still the trigger's
 * description.
 * Dropping `max-w-full` from the trigger or from its `FormField` fails it.
 */
export const FiltersWithRangePhone: Story = {
  render: () => <PaymentsFilters values={FILTERED} />,
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    const { trigger, value } = await dateField(canvasElement);
    const chips = statusChips(canvasElement);

    for (const width of ["", "343px"]) {
      canvasElement.style.width = width;
      await nothingOffScreen(canvasElement, [chips, trigger]);
      await expect(top(trigger)).toBeGreaterThan(top(chips));
      await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
    }

    canvasElement.style.width = "232px";
    try {
      await nothingOffScreen(canvasElement, [chips, trigger]);
      await expect(trigger).toHaveAccessibleDescription(
        "2026-09-01 → 2026-09-07",
      );
    } finally {
      canvasElement.style.width = "";
    }
  },
};

// ------------------------------------------------------------ PaymentsSkeleton

/**
 * The payments screen's loading state above the part of its loaded state
 * the skeleton stands in for — the header and the filter row, composed as
 * `payments-screen.tsx`'s loaded branch composes them, in the same
 * `flex-col gap-6` column `AppShell` gives every screen.
 */
function SkeletonAboveScreen() {
  return (
    <div className="flex flex-col gap-12">
      <PaymentsSkeleton />
      <div className="flex flex-col gap-6" data-loaded="">
        <PageHeader title="Payments" description={PAYMENTS_DESCRIPTION} />
        <PaymentsFilters
          values={{ status: "", createdFrom: "", createdTo: "" }}
        />
      </div>
    </div>
  );
}

/**
 * Each element's box, as an offset from `origin`'s top-left, and its size.
 *
 * Width only where `withWidth` says so: the loaded title, description and
 * field label are block elements as wide as their column, while their
 * placeholders are as wide as their text — the same line, the same height,
 * a different box width, and the width of a block says nothing about where
 * the eye lands.
 */
function boxesFrom(
  origin: Element,
  elements: readonly (readonly [Element, boolean])[],
) {
  const o = origin.getBoundingClientRect();
  return elements.map(([element, withWidth]) => {
    const box = element.getBoundingClientRect();
    return withWidth
      ? [box.left - o.left, box.top - o.top, box.width, box.height]
      : [box.left - o.left, box.top - o.top, box.height];
  });
}

/**
 * The boxes a reader's eye lands on, in reading order: the title, the
 * description, every status chip, the date field's label and the field.
 * `loaded` finds them by role; the skeleton by position, since it is all
 * `aria-hidden` blocks — the order `list-skeleton.tsx` and
 * `PaymentsFiltersSkeleton` build them in.
 */
function loadedBoxes(column: HTMLElement) {
  const canvas = within(column);
  return boxesFrom(column, [
    [canvas.getByRole("heading", { name: "Payments" }), false],
    [canvas.getByText(PAYMENTS_DESCRIPTION), false],
    ...within(
      canvas.getByRole("navigation", { name: "Filter payments by status" }),
    )
      .getAllByRole("listitem")
      .map((chip) => [chip, true] as const),
    [canvas.getByText("Created between", { selector: "label" }), false],
    [canvas.getByRole("button", { name: "Created between" }), true],
  ]);
}

function skeletonBoxes(loading: Element) {
  const [header, filters] = [...loading.children].filter(
    (child) => getComputedStyle(child).position !== "absolute",
  );
  const titleAndDescription = [...(header?.firstElementChild?.children ?? [])];
  const [chipHalf, dateHalf] = [...(filters?.children ?? [])];
  const chips = [...(chipHalf?.firstElementChild?.children ?? [])];
  const date = [...(dateHalf?.children ?? [])];
  const [label, field] = date;
  return boxesFrom(loading, [
    ...titleAndDescription.map((block) => [block, false] as const),
    ...chips.map((chip) => [chip, true] as const),
    [label as Element, false],
    [field as Element, true],
  ]);
}

/**
 * The skeleton's header and filter row land on the real ones: every box
 * within a pixel, at each column width given and then again at a 20px
 * root. The canvas width and the root are restored in `finally`, since
 * every later story in this page shares them.
 */
async function skeletonLandsOnTheRow(
  canvasElement: HTMLElement,
  widths: readonly string[],
) {
  const loading = canvasElement.querySelector('[role="status"]');
  const column = canvasElement.querySelector<HTMLElement>("[data-loaded]");
  await expect(loading).not.toBeNull();
  await expect(column).not.toBeNull();
  const root = canvasElement.ownerDocument.documentElement;
  try {
    for (const rootSize of ["", "20px"]) {
      root.style.fontSize = rootSize;
      for (const width of widths) {
        canvasElement.style.width = width;
        const expected = loadedBoxes(column as HTMLElement);
        const actual = skeletonBoxes(loading as Element);
        const where = `root ${rootSize || "16px"}, column ${width || "full"}`;
        await expect(actual.length, where).toBe(expected.length);
        const off = actual.flatMap((box, i) =>
          box.map((value, j) => Math.abs(value - (expected[i]?.[j] ?? 0))),
        );
        await expect(
          Math.max(...off),
          `${where}: skeleton ${JSON.stringify(actual)}, loaded ${JSON.stringify(expected)}`,
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
 * header and filter row it stands in for.
 *
 * The `play` function is the guard on `PaymentsSkeleton`: at the story's
 * own width and squeezed to the app shell's content column at a 1024px
 * window (880px; narrower columns are below `lg`, where the row stacks,
 * and are `PaymentsLoadingPhone`'s), and again at a 20px root, every placeholder — title,
 * description, each chip, the date label and field — is within a pixel of
 * the box it stands for. A guessed width anywhere (a chip block of fixed
 * width, the title's placeholder in another type size) fails it.
 */
export const PaymentsLoading: Story = {
  render: () => <SkeletonAboveScreen />,
  globals: { viewport: { value: "desktop", isRotated: false } },
  play: async ({ canvasElement }) => {
    await skeletonLandsOnTheRow(canvasElement, ["", "880px"]);
  },
};

/**
 * The same at 375×812, where the chips wrap and the description is two
 * lines: at the story's own width and squeezed to the shell's column at
 * 375 and 320px windows (343 and 288px), at both roots.
 */
export const PaymentsLoadingPhone: Story = {
  render: () => <SkeletonAboveScreen />,
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    await skeletonLandsOnTheRow(canvasElement, ["", "343px", "288px"]);
  },
};

// --------------------------------------------------------------- FilterChips

/**
 * The filter chips every list filters with, one selected — in both themes,
 * so axe measures the selected chip's `on-primary-container` text and the
 * unselected chips' `on-surface-variant` text against their grounds.
 */
const CHIPS = [
  { href: "/refunds", label: "All", active: false },
  { href: "/refunds?status=pending", label: "Pending", active: false },
  { href: "/refunds?status=failed", label: "Failed", active: true },
  { href: "/refunds?status=succeeded", label: "Refunded", active: false },
];

export const Chips: Story = {
  render: () => (
    <FilterChips label="Filter refunds by status" options={CHIPS} />
  ),
  play: async ({ canvasElement }) => {
    const links = within(canvasElement).getAllByRole("link");
    await expect(
      links.filter((link) => link.getAttribute("aria-current") === "true"),
    ).toHaveLength(1);
  },
};

export const ChipsLight: Story = {
  render: () => (
    <FilterChips label="Filter refunds by status" options={CHIPS} />
  ),
  globals: { theme: "light" },
};

// ------------------------------------------------------------- ProcedurePager

export const ProcedurePagerMiddle: Story = {
  render: () => (
    <ProcedurePager
      basePath="/refunds"
      offset={20}
      returned={20}
      totalCount={57}
      hasNext
      query="status=failed"
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // Paging a filtered list keeps the filter, on both links.
    await expect(
      canvas.getByRole("link", { name: /previous/i }),
    ).toHaveAttribute("href", "/refunds?status=failed");
    await expect(canvas.getByRole("link", { name: /next/i })).toHaveAttribute(
      "href",
      "/refunds?status=failed&offset=40",
    );
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
 * `PaymentDetailHeader` and `PaymentDetailView` in the shell's
 * `flex-col gap-6` column, as `payment-screen.tsx`'s loaded branch composes
 * them.
 */
function PaymentSkeletonAbovePage() {
  return (
    <div className="flex flex-col gap-12">
      <PaymentSkeleton />
      <div className="flex flex-col gap-6" data-loaded="">
        <PaymentDetailHeader />
        <PaymentDetailView detail={DETAIL} />
      </div>
    </div>
  );
}

/**
 * The boxes a payment's loading state claims, each as an offset from its
 * column's top-left corner and a size: the back link, the title, the
 * panel's position and width (not its height: the tiles are data), the
 * panel's title-and-caption block, and the tile grid's position and width.
 *
 * Read by position, the same position in each: the column is `[header,
 * panel, …]` (the skeleton's first child is its `sr-only` text, which is
 * absolutely positioned and skipped), the header is `[[back, title]]`, and
 * the panel is `[title and caption, tiles]`.
 */
function pageBoxes(column: Element) {
  const origin = column.getBoundingClientRect();
  const at = (element: Element | undefined | null, withHeight = true) => {
    const box = (element as Element).getBoundingClientRect();
    const edges = [
      box.left - origin.left,
      box.top - origin.top,
      box.width,
      box.height,
    ];
    return withHeight ? edges : edges.slice(0, 3);
  };
  const [header, panel] = [...column.children].filter(
    (child) => getComputedStyle(child).position !== "absolute",
  );
  const [back, title] = [...(header?.firstElementChild?.children ?? [])];
  const [titled, tiles] = [...(panel?.children ?? [])];
  // The title's width is left out: the loaded `<h2>` is a block as wide as
  // its column, the placeholder as wide as the word.
  const [titleLeft = 0, titleTop = 0, , titleHeight = 0] = at(title);
  return [
    at(back),
    [titleLeft, titleTop, titleHeight],
    at(panel, false),
    at(titled),
    at(tiles, false),
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
  const loading = canvasElement.querySelector(
    '[role="status"][aria-busy="true"]',
  );
  const column = canvasElement.querySelector("[data-loaded]");
  await expect(loading).not.toBeNull();
  await expect(column).not.toBeNull();
  await expect(
    within(column as HTMLElement).getByRole("heading", { name: "Payment" }),
  ).toBeVisible();

  const root = canvasElement.ownerDocument.documentElement;
  try {
    for (const rootSize of ["", "20px"]) {
      root.style.fontSize = rootSize;
      for (const width of widths) {
        canvasElement.style.width = width;
        const expected = pageBoxes(column as Element);
        const actual = pageBoxes(loading as Element);
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
 * 640px windows (956 and 496px), and again at a 20px root, the back link,
 * the title, the panel, its title-and-caption block and its tile grid are
 * within a pixel of the loaded page's.
 */
export const PaymentLoading: Story = {
  render: () => <PaymentSkeletonAbovePage />,
  globals: { viewport: { value: "desktop", isRotated: false } },
  play: async ({ canvasElement }) => {
    await paymentSkeletonLandsOnThePage(canvasElement, ["", "956px", "496px"]);
  },
};

/**
 * The same at 375×812, where the panel's caption is two lines: at the
 * story's own width and squeezed to the shell's column at 375 and 320px
 * windows (343 and 288px), at both roots.
 */
export const PaymentLoadingPhone: Story = {
  render: () => <PaymentSkeletonAbovePage />,
  globals: { viewport: { value: "phone", isRotated: false } },
  play: async ({ canvasElement }) => {
    await paymentSkeletonLandsOnThePage(canvasElement, ["", "343px", "288px"]);
  },
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

// ------------------------------------------------------------------- Sign-in
//
// The sign-in's screens inside `CenteredShell`, in both themes, because
// `a11y.test.tsx` runs in jsdom and answers no `color-contrast`: this is the
// only place the muted text on the card, the hint under a password field and
// the warning banners get a contrast verdict. The actions are no-ops — a
// page's server actions cannot run in a story.

const NOOP = () => Promise.resolve({ error: null, requestId: null });

export const SignInEmail: Story = {
  render: () => (
    <CenteredShell>
      <SignInForm action={NOOP} />
    </CenteredShell>
  ),
};

export const SignInEmailLight: Story = {
  render: () => (
    <CenteredShell>
      <SignInForm action={NOOP} />
    </CenteredShell>
  ),
  globals: { theme: "light" },
};

export const PasswordChange: Story = {
  render: () => (
    <CenteredShell>
      <PasswordForm action={NOOP} />
    </CenteredShell>
  ),
};

export const Enrolment: Story = {
  render: () => (
    <CenteredShell>
      <EnrolmentPanel
        qrDataUrl="data:image/gif;base64,R0lGODlhAQABAAAAACw="
        secret="JBSWY3DPEHPK3PXP"
      />
      <TotpForm action={NOOP} enrolling />
    </CenteredShell>
  ),
};

export const PairedKeyNotice: Story = {
  render: () => (
    <CenteredShell>
      <PairedKeyOnly />
    </CenteredShell>
  ),
};

export const NotConfiguredNotice: Story = {
  render: () => (
    <CenteredShell>
      <NotConfigured
        problems={[
          {
            variable: "VPAY_DASHBOARD_CLIENT_ID",
            detail: "this deployment's registered dashboard_client.client_id.",
          },
        ]}
      />
    </CenteredShell>
  ),
};

export const UnreachableNotice: Story = {
  render: () => (
    <CenteredShell>
      <SignInUnreachable
        failure={{
          status: 503,
          message: "Starting up.",
          requestId: "req_01J8",
        }}
      />
    </CenteredShell>
  ),
};

export const Account: Story = {
  render: () => (
    <CenteredShell>
      <AccountPage
        displayName="Ada Lovelace"
        email="ada@acme.test"
        merchantId="acme"
        signOut={() => Promise.resolve(NO_ERROR)}
      />
    </CenteredShell>
  ),
};

export const MintRefusedScreen: Story = {
  render: () => (
    <CenteredShell>
      <MintRefused
        failure={{
          status: 401,
          message: "We could not sign you in.",
          requestId: "req_01J8",
        }}
        signOut={() => Promise.resolve(NO_ERROR)}
      />
    </CenteredShell>
  ),
};

export const MintRefusedLight: Story = {
  ...MintRefusedScreen,
  globals: { theme: "light" },
};

export const AccountLight: Story = {
  render: () => (
    <CenteredShell>
      <AccountPage
        displayName="Ada Lovelace"
        email="ada@acme.test"
        merchantId="acme"
        signOut={() => Promise.resolve(NO_ERROR)}
      />
    </CenteredShell>
  ),
  globals: { theme: "light" },
};
