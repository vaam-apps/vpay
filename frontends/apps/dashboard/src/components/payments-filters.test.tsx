import { PAYMENT_STATUS, statusLabel } from "@vpay/tokens";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The status chips are links (`FilterChips`), but the date range applies by
// calling `useRouter().push(...)` when the picker's Save commits — a
// controlled `DateRangePicker` has no form submission to ride on — so the
// date cases below need a router to push into. `vi.hoisted` — not a plain top-level `const` — because
// `vi.mock` is hoisted above imports, and a factory that closes over an
// un-hoisted variable throws "Cannot access 'push' before initialization".
const { push } = vi.hoisted(() => ({ push: vi.fn() }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push }) }));

const { PaymentsFilters } = await import("./payments-filters");

const EMPTY = { status: "", createdFrom: "", createdTo: "" };

/**
 * A September 2026 range, so the calendar the picker opens is a **known**
 * month rather than whatever month the machine running the suite is in.
 *
 * `DateRangePicker` passes `value.from` to `react-day-picker` as
 * `defaultMonth` (`date-picker.js`), so seeding `from` is the only thing
 * that makes a day cell nameable — "Thursday, September 10th, 2026" is a
 * button that exists on any day of any year with this seed, and is not on
 * screen at all without it. Still true on 0.3.0 for the docked picker, the
 * one these cases click (see `dockedSeptember`); the full-screen one stacks
 * the 12 months either side of the same `from`.
 */
const SEPTEMBER = {
  status: "",
  createdFrom: "2026-09-01",
  createdTo: "2026-09-07",
};

/** The status chips' links, in order, by their visible label. */
function chipLinks(): HTMLAnchorElement[] {
  const nav = screen.getByRole("navigation", {
    name: "Filter payments by status",
  });
  return within(nav).getAllByRole("link");
}

/** One chip's href, resolved so its query can be read. */
function chipUrl(label: string): URL {
  const link = chipLinks().find((a) => a.textContent === label);
  expect(link, `a chip labelled ${label}`).toBeDefined();
  const url = new URL(link!.getAttribute("href")!, "https://dashboard.test");
  expect(url.pathname).toBe("/payments");
  return url;
}

/**
 * The date field — `DatePickerTrigger`'s button — found by the name its
 * visible label gives it.
 *
 * Since 2026-09-24 the range sits in a `FormField` labelled "Created
 * between", whose `<label for>` points at the trigger's `id`; the trigger
 * has no `aria-label`, and its own text is the dates, or "Any time" while
 * empty. So the label is the only thing on the page that can name it
 * "Created between", in every state, and every case that goes through here
 * also fails when the label stops pointing at the trigger (the component's
 * module doc has the mutation). Exact match, so "Clear the dates" beside it
 * is no competitor.
 *
 * _History:_ on `@vaam-apps/ui` 0.2.x this took the current range, because
 * the name was the shown text followed by an `sr-only` copy of the
 * placeholder (`"2026-09-01 → 2026-09-07 Created between"`) and changed with
 * the range; on 0.3.0, before the label, it was `dateTrigger()` and found
 * the name an `aria-label` gave.
 */
function dateFieldByLabel(): HTMLElement {
  return screen.getByRole("button", { name: "Created between" });
}

/**
 * The docked picker's September 2026 grid, once the picker is open.
 *
 * **Why not `screen.getByRole("button", { name: /September 10th/ })`:**
 * `DatePickerContent` renders two trees — the docked picker (from 640 px
 * up) and the full-screen range picker (below) — and hides one by CSS media
 * query. jsdom applies no CSS, so both are in the accessibility tree here:
 * 27 grids (the docked picker's September and October, then the full-screen
 * picker's 25 stacked months, September 2025 to September 2027), two of each
 * September day, and two Save buttons. Both trees render the same staged
 * pick, so clicking either is the same pick; the docked tree comes first in
 * the DOM, and scoping to its grid both makes the choice explicit and keeps
 * the day query off the ~800 other buttons (measured: 3 ms scoped against
 * ~250 ms unscoped, per query, on this suite's jsdom).
 */
function dockedSeptember(): HTMLElement {
  const [docked] = within(screen.getByRole("dialog")).getAllByRole("grid", {
    name: "September 2026",
  });
  // `getAllByRole` throws when nothing matches, so this `!` cannot hide a
  // missing grid — only `noUncheckedIndexedAccess`'s `| undefined`.
  return docked!;
}

/** Press a day in the docked picker's September. */
function pickSeptember(day: RegExp): void {
  fireEvent.click(within(dockedSeptember()).getByRole("button", { name: day }));
}

/**
 * Press Save — the one press that commits a staged range on 0.3.0.
 *
 * The first of the two (docked before full-screen in the DOM, as above);
 * `getAllByRole` throws when there are none, so the `!` hides nothing.
 */
function clickSave(): void {
  fireEvent.click(screen.getAllByRole("button", { name: "Save" })[0]!);
}

/**
 * The URL `router.push` was called with, resolved against a fake origin so
 * `new URL` accepts the relative path `push` actually receives.
 *
 * Asserts exactly one call rather than reading `push.mock.calls[0]`
 * unconditionally — a case that presses Save without the component having
 * wired the handler up at all would otherwise read `undefined` and fail on
 * `new URL(undefined, ...)` with a message that says nothing about what's
 * actually wrong.
 *
 * **The `pathname` assertion lives here, not in one case.** It was in the
 * no-filters case alone, which asserts the branch that pushes the bare
 * string `"/payments"`; the branch that pushes `` `/payments?${query}` ``
 * — the one an operator actually reaches, since they came here to filter —
 * had its route asserted by nothing. Mutating that template to
 * `` `/wrong?${query}` `` left all five cases green. Every case goes
 * through this helper, so every case now pins the route.
 */
function pushedUrl(): URL {
  expect(push).toHaveBeenCalledTimes(1);
  const url = new URL(
    String(push.mock.calls[0]?.[0]),
    "https://dashboard.test",
  );
  expect(url.pathname).toBe("/payments");
  return url;
}

describe("the payments filters", () => {
  beforeEach(() => {
    push.mockClear();
  });

  it("links the All chip to the bare list, so the filters live in the URL", () => {
    // A chip is a link to the filtered URL rather than state held here: a
    // filtered list stays bookmarkable, reloadable and pasteable into a
    // ticket, and `PaymentsScreen` has one source of truth in `searchParams`.
    render(<PaymentsFilters values={{ ...EMPTY, status: "succeeded" }} />);
    expect(chipUrl("All").search).toBe("");
  });

  it("carries the status as `status`, which is the parameter vpay reads", () => {
    render(<PaymentsFilters values={EMPTY} />);
    expect(chipUrl(statusLabel.succeeded).searchParams.get("status")).toBe(
      "succeeded",
    );
    expect(chipUrl(statusLabel.canceled).searchParams.get("status")).toBe(
      "canceled",
    );
  });

  it("marks the chip on screen, and only that one", () => {
    render(<PaymentsFilters values={{ ...EMPTY, status: "processing" }} />);
    const current = chipLinks().filter((a) => a.hasAttribute("aria-current"));
    expect(current.map((a) => a.textContent)).toEqual([statusLabel.processing]);
  });

  it("keeps the dates when the status changes", () => {
    render(<PaymentsFilters values={SEPTEMBER} />);
    const url = chipUrl(statusLabel.succeeded);
    expect(url.searchParams.get("created_from")).toBe("2026-09-01");
    expect(url.searchParams.get("created_to")).toBe("2026-09-07");
  });

  it("carries the two dates under the names the page reads", () => {
    // `created_from`/`created_to`, this app's own vocabulary — `payments-query.ts`
    // translates them into the API's `created_gte`/`created_lte` instants.
    render(<PaymentsFilters values={{ ...SEPTEMBER, status: "succeeded" }} />);
    fireEvent.click(dateFieldByLabel());
    clickSave();
    const url = pushedUrl();
    expect(url.searchParams.get("created_from")).toBe("2026-09-01");
    expect(url.searchParams.get("created_to")).toBe("2026-09-07");
    expect(url.searchParams.get("status")).toBe("succeeded");
  });

  it("names the date field by its visible label, and reads the dates as its description", () => {
    // The label is a real `<label for>` on the trigger, not an `aria-label`
    // that happens to say the same words: both satisfy `getByRole`'s name
    // and `getByLabelText`, and only one of them is visible to a sighted
    // operator returning to a bookmarked, filtered URL — which is what the
    // label was added for.
    render(<PaymentsFilters values={SEPTEMBER} />);
    const trigger = dateFieldByLabel();
    expect(trigger).not.toHaveAttribute("aria-label");
    const label = screen.getByText("Created between", { selector: "label" });
    expect((label as HTMLLabelElement).control).toBe(trigger);
    // `DatePickerTrigger` moves the shown value into its description once
    // it finds a `<label>` pointing at it (`labels`, read after mount), so a
    // screen reader hears the label, then the dates.
    expect(trigger).toHaveAccessibleDescription("2026-09-01 → 2026-09-07");
    // Clicking the label focuses the field and opens nothing. `FormField`'s
    // `Label` is
    // Headless UI's, which cancels the native label click (on a button that
    // would be a click, and would open the picker) and focuses instead.
    fireEvent.click(label);
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it('describes the empty date field as "Any time", not as its label again', () => {
    // The placeholder is the field's "Any status": what an unset filter
    // means. It was "Created between", the label's own words. Because the
    // shown text is the trigger's description, that already made the empty
    // trigger "Created between, button, Created between" under the
    // `aria-label` this replaced (measured on that revision, `c562564b`, in
    // jsdom), and beside a visible label it would draw them twice as well.
    render(<PaymentsFilters values={EMPTY} />);
    expect(dateFieldByLabel()).toHaveAccessibleDescription("Any time");
  });

  it("carries the range an operator PICKS in the calendar", () => {
    // Same hole as the status case above, and the more dangerous half of
    // it: `DateRangePicker` is a *controlled* component, so `onValueChange`
    // is the whole of its wiring — replace it with `() => {}` and the
    // calendar opens, highlights and changes nothing, while the seeded
    // dates keep coming back out of every other case in this file.
    // Measured: that mutation left all five of the original cases green.
    //
    // **`created_from` stayed at the seed on 0.2.x, and that was
    // `react-day-picker`'s measured behaviour, not a requirement of ours.**
    // Clicking a later day on a range that was already complete EXTENDED it
    // — the end moved, the start did not — so these two clicks on the
    // 1–7 September seed gave 2026-09-01 → 2026-09-18, and an operator who
    // wanted a different start had to Clear first. It was recorded here so
    // that a package bump changing it would say so out loud rather than
    // quietly re-point the filter at a different range.
    //
    // **0.3.0 is that bump.** Its range rule is M3's: a tap on a whole range
    // starts a new one, and the next tap on or after it sets the end — so
    // the same two taps now give 2026-09-10 → 2026-09-18, and no Clear is
    // needed. And a pick is only staged until Save: `onValueChange` fires
    // there and nowhere else, so without `clickSave()` the seed would come
    // back out exactly as it does with `onValueChange` deleted.
    render(<PaymentsFilters values={SEPTEMBER} />);
    fireEvent.click(dateFieldByLabel());
    pickSeptember(/Thursday, September 10th, 2026/);
    pickSeptember(/Friday, September 18th, 2026/);
    clickSave();
    const url = pushedUrl();
    expect(url.searchParams.get("created_from")).toBe("2026-09-10");
    expect(url.searchParams.get("created_to")).toBe("2026-09-18");
  });

  it("carries a start with no end as `created_from` alone", () => {
    // New on 0.3.0, and reachable in two presses: the first tap of a range
    // is a start with an OPEN end (0.2.x made it a one-day range), and Save
    // commits it as `{ from, to: undefined }` — which the package documents
    // as a value, not a validation error. So "from 10 September on" is now a
    // filter an operator can set by accident as easily as on purpose, and
    // `apply()` has to carry the half it has rather than drop the pair: a
    // version that only sent dates when both were set would pass every
    // other case in this file.
    render(<PaymentsFilters values={SEPTEMBER} />);
    fireEvent.click(dateFieldByLabel());
    pickSeptember(/Thursday, September 10th, 2026/);
    clickSave();
    const url = pushedUrl();
    expect(url.searchParams.get("created_from")).toBe("2026-09-10");
    expect(url.searchParams.has("created_to")).toBe(false);
  });

  it("drops both dates when the range is cleared", () => {
    // The other half of the controlled contract: clearing has to reach the
    // URL too, or an operator who removes a date filter gets the list they
    // were already looking at and no sign that the control did nothing.
    render(<PaymentsFilters values={SEPTEMBER} />);
    fireEvent.click(screen.getByRole("button", { name: "Clear the dates" }));
    const url = pushedUrl();
    expect(url.searchParams.has("created_from")).toBe(false);
    expect(url.searchParams.has("created_to")).toBe(false);
  });

  it("carries no cursor, so changing a filter starts from the first page", () => {
    // `after`/`before` name a row in the PREVIOUS result set; carrying one
    // across a filter change asks for "the page after a payment that is no
    // longer in this list", and the answer looks like a working page.
    render(<PaymentsFilters values={{ ...EMPTY, status: "succeeded" }} />);
    for (const link of chipLinks()) {
      const url = new URL(link.getAttribute("href")!, "https://dashboard.test");
      expect(url.searchParams.has("after"), link.textContent ?? "").toBe(false);
      expect(url.searchParams.has("before"), link.textContent ?? "").toBe(
        false,
      );
    }
  });

  it("offers every status the pill can render, and no others", () => {
    // One vocabulary: a status cannot be filterable but unrenderable, or the
    // other way round. Compared in order against the token list itself,
    // not a copy of it — deleting four of the five chips must fail here.
    render(<PaymentsFilters values={EMPTY} />);
    expect(
      chipLinks().map(
        (link) =>
          new URL(
            link.getAttribute("href")!,
            "https://dashboard.test",
          ).searchParams.get("status") ?? "",
      ),
    ).toEqual(["", ...PAYMENT_STATUS]);
  });
});
