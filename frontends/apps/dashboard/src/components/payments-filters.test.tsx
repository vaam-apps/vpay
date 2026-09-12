import { PAYMENT_STATUS } from "@vpay/tokens";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// `PaymentsFilters` now applies by calling `useRouter().push(...)` (see its
// own module doc for why a controlled `DateRangePicker` forced that) rather
// than submitting a `method="get"` form, so every case below needs a router
// to push into. `vi.hoisted` — not a plain top-level `const` — because
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
 * screen at all without it.
 */
const SEPTEMBER = {
  status: "",
  createdFrom: "2026-09-01",
  createdTo: "2026-09-07",
};

function clickApply(): void {
  fireEvent.click(screen.getByRole("button", { name: "Apply" }));
}

/**
 * The date range's trigger button.
 *
 * Queried by its *current* accessible name rather than a substring, because
 * a substring match also hits the "Clear Created between" button the picker
 * renders beside it once a range is set, and `getByRole` throws on two
 * matches. The name is the visible label followed by the `sr-only`
 * placeholder — see the component's module doc on where that name comes
 * from — so it changes as the range changes and must be re-queried.
 */
function dateTrigger(label: string): HTMLElement {
  return screen.getByRole("button", { name: `${label} Created between` });
}

/**
 * The URL `router.push` was called with, resolved against a fake origin so
 * `new URL` accepts the relative path `push` actually receives.
 *
 * Asserts exactly one call rather than reading `push.mock.calls[0]`
 * unconditionally — a case that clicks Apply without the component having
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

  it("pushes a URL on Apply, so the filters live in the URL", () => {
    // Not a `method="get"` form's own navigation any more (see the module
    // doc), but the property that mattered — a filtered list is
    // bookmarkable, reloadable and pasteable into a ticket, and
    // `PaymentsScreen` has one source of truth in `searchParams` rather than
    // a second copy of these picks in a prop — survives: `router.push`
    // still lands on `/payments` with no filters when none are set.
    render(<PaymentsFilters values={EMPTY} />);
    clickApply();
    expect(pushedUrl().search).toBe("");
  });

  it("carries the status as `status`, which is the parameter vpay reads", () => {
    // THE case this file exists for. `Select` is a Headless UI listbox and
    // not a native `<select>`; if this component read its value the way
    // that one is read, the status filter would render, look right, be
    // clickable, and silently do nothing.
    render(<PaymentsFilters values={{ ...EMPTY, status: "succeeded" }} />);
    clickApply();
    expect(pushedUrl().searchParams.get("status")).toBe("succeeded");
  });

  it("carries the status an operator PICKS, not the one the URL arrived with", () => {
    // The case above seeds `values.status` and asserts it comes back out,
    // which the component would pass with its `<select>`'s `onChange`
    // deleted entirely — measured: replacing it with `() => {}` left every
    // other case in this file green. Under the `method="get"` form this
    // replaced there was no handler to delete (the browser read the DOM at
    // submit time); there is one now, and this is what holds it.
    render(<PaymentsFilters values={{ ...EMPTY, status: "succeeded" }} />);
    fireEvent.change(screen.getByLabelText("Status"), {
      target: { value: "canceled" },
    });
    clickApply();
    expect(pushedUrl().searchParams.get("status")).toBe("canceled");
  });

  it("carries the two dates under the names the page reads", () => {
    render(<PaymentsFilters values={SEPTEMBER} />);
    clickApply();
    const url = pushedUrl();
    // `created_from`/`created_to`, this app's own vocabulary — `payments-query.ts`
    // translates them into the API's `created_gte`/`created_lte` instants.
    expect(url.searchParams.get("created_from")).toBe("2026-09-01");
    expect(url.searchParams.get("created_to")).toBe("2026-09-07");
  });

  it("carries the range an operator PICKS in the calendar", () => {
    // Same hole as the status case above, and the more dangerous half of
    // it: `DateRangePicker` is a *controlled* component, so `onValueChange`
    // is the whole of its wiring — replace it with `() => {}` and the
    // calendar opens, highlights and changes nothing, while the seeded
    // dates keep coming back out of every other case in this file.
    // Measured: that mutation left all five of the original cases green.
    //
    // **`created_from` staying at the seed is `react-day-picker`'s measured
    // behaviour, not a requirement of ours.** Clicking a later day on a
    // range that is already complete EXTENDS it — the end moves, the start
    // does not, whatever you click — so an operator who wants a different
    // start has to Clear first (the case below). Recorded here because a
    // package bump that changes it should say so out loud rather than
    // quietly re-point the filter at a different month.
    render(<PaymentsFilters values={SEPTEMBER} />);
    fireEvent.click(dateTrigger("2026-09-01 → 2026-09-07"));
    fireEvent.click(
      screen.getByRole("button", { name: /Thursday, September 10th, 2026/ }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /Friday, September 18th, 2026/ }),
    );
    clickApply();
    const url = pushedUrl();
    expect(url.searchParams.get("created_from")).toBe("2026-09-01");
    expect(url.searchParams.get("created_to")).toBe("2026-09-18");
  });

  it("drops both dates when the range is cleared", () => {
    // The other half of the controlled contract: clearing has to reach the
    // URL too, or an operator who removes a date filter gets the list they
    // were already looking at and no sign that the control did nothing.
    render(<PaymentsFilters values={SEPTEMBER} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Clear Created between" }),
    );
    clickApply();
    const url = pushedUrl();
    expect(url.searchParams.has("created_from")).toBe(false);
    expect(url.searchParams.has("created_to")).toBe(false);
  });

  it("carries no cursor, so applying a filter starts from the first page", () => {
    // `starting_after` names a row in the PREVIOUS result set; carrying it
    // across a filter change asks for "the page after a payment that is no
    // longer in this list", and the answer looks like a working page.
    render(<PaymentsFilters values={{ ...EMPTY, status: "succeeded" }} />);
    clickApply();
    const url = pushedUrl();
    expect(url.searchParams.has("after")).toBe(false);
    expect(url.searchParams.has("before")).toBe(false);
  });

  it("offers every status the badge can render, and no others", () => {
    // One vocabulary: a status cannot be filterable but unrenderable, or the
    // other way round. vpay answers 400 naming `status` for a value outside
    // its own vocabulary, which is right of vpay and a refusal this form
    // cannot provoke.
    //
    // **This asserted only that a control labelled "Status" existed**, which
    // is not what its name claims and not what the claim is worth: deleting
    // four of the five options (`PAYMENT_STATUS.slice(0, 1)`) left it green.
    // The option VALUES are the vocabulary, so those are what is compared,
    // in order, against the token list itself rather than a copy of it.
    render(<PaymentsFilters values={EMPTY} />);
    const options = within(screen.getByLabelText("Status")).getAllByRole(
      "option",
    );
    expect(
      options.map((option) => (option as HTMLOptionElement).value),
    ).toEqual(["", ...PAYMENT_STATUS]);
  });
});
