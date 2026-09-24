"use client";

import { PAYMENT_STATUS } from "@vpay/tokens";
import {
  Button,
  DateRangePicker,
  DatePickerClear,
  DatePickerContent,
  DatePickerTrigger,
  DatePickerValue,
  FormField,
  type IsoDateRange,
} from "@vaam-apps/ui";
import { useRouter } from "next/navigation";
import { useState } from "react";

/** What the URL currently asks for. */
export interface PaymentsFilterValues {
  /** One `IntentStatus` wire value, or `''` for every status. */
  status: string;
  /** `YYYY-MM-DD`, inclusive lower bound on `created`, or `''`. */
  createdFrom: string;
  /** `YYYY-MM-DD`, inclusive upper bound, or `''`. */
  createdTo: string;
}

export interface PaymentsFiltersProps {
  values: PaymentsFilterValues;
}

/** The "every status" choice. An empty value, so it drops out of the query. */
const ANY_STATUS = "";

/**
 * `PaymentsFilterValues`' two dates, in `DateRangePicker`'s own shape.
 *
 * Two "unset" spellings meet here: this app's URL uses `''` (what
 * `payments-query.ts`'s `one()` returns for a missing parameter), the picker
 * uses `undefined`. An all-`''` pair becomes `undefined` outright rather than
 * `{ from: undefined, to: undefined }` — the picker's own `hasDate` check
 * (`date-picker.js` in 0.3.0; a `hasValue` flag in 0.2.x) treats those the
 * same for display, but only the `undefined` form is what a freshly-mounted,
 * filter-free screen should hand it.
 */
function rangeFromValues(
  values: PaymentsFilterValues,
): IsoDateRange | undefined {
  if (values.createdFrom === "" && values.createdTo === "") {
    return undefined;
  }
  return {
    from: values.createdFrom === "" ? undefined : values.createdFrom,
    to: values.createdTo === "" ? undefined : values.createdTo,
  };
}

/**
 * The status and date-range filters, as one row above the table.
 *
 * **Was a plain `method="get"` `<form>`** — a screenshot from the e2e run
 * showed it four rows tall, one control per line, and the redesign asked for
 * one row instead. That form shape is also the reason this is no longer it:
 * `@vaam-apps/ui`'s `DateRangePicker` is a controlled
 * component (`value`/`onValueChange`, `date-picker.d.ts`), not a form field
 * with a `name`, so it cannot sit inside a `<form method="get">` the way the
 * two `<input type="date">`s it replaces did. The dashboard already renders
 * this screen through Refine as a client component reading
 * `useSearchParams()` (`payments-screen.tsx`), so the fix is the one that
 * screen already takes: hold the picks in state, and push a built URL on
 * Apply. The URL stays the source of truth either way — a filtered list is
 * still bookmarkable and pasteable into a ticket, and `PaymentsScreen` still
 * reads only `searchParams`, never a prop this component hands it directly.
 *
 * **The status control is still a native `<select id="payments-filter-status">`,
 * not `@vaam-apps/ui`'s `Select`.** That `Select` wraps a Headless UI
 * `Listbox` and forwards no `name` and no ref a plain `onChange` can read
 * without one — the native element is simply the one shape this file already
 * knew how to read reliably, and nothing about moving to client state changed
 * that. The status list is `PAYMENT_STATUS` from `@vpay/tokens` — the same
 * five values `../payment-status.ts` renders as a pill — so a status can
 * never be filterable but unrenderable, or the other way round. vpay answers
 * `400` naming `status` for a value outside its own vocabulary rather than an
 * empty list, which is the right answer and one this form cannot provoke.
 *
 * **No `FormField` around the date range — since `@vaam-apps/ui` 0.3.0 a
 * choice kept, no longer a constraint.** Until 0.3.0 the picker took no `id`,
 * so `FormField`'s `<label htmlFor>` would have pointed at nothing, and the
 * picker named itself by repeating `placeholder` in a `sr-only` span:
 * `"Created between Created between"` empty and `"2026-09-01 → 2026-09-07
 * Created between"` with a range, read off a real render
 * (`docs/status/verification/2026-09-12-dashboard-refine.md`). 0.3.0 made
 * the picker compound parts, and its `DatePickerTrigger` takes an `id`, so a
 * visible `FormField` label is now possible. Adopting one changes what the
 * filter row shows, which is a decision above this file, so the 0.3.0
 * migration kept the row as it was and named the trigger with
 * `aria-label="Created between"` instead — the package's rule for a picker
 * outside a `FormField` (its skill, `primitives-input.md`).
 *
 * The name is now `"Created between"` whether a range is set or not, and the
 * shown range is added as the trigger's description (`aria-describedby` →
 * `DatePickerValue`'s span), so a screen reader hears the label, then the
 * dates. Measured, both in jsdom and in the built Storybook. `placeholder`
 * is what a sighted operator sees while the range is empty and names
 * nothing: removing it alone leaves every case in this app green. Removing
 * the `aria-label` fails `payments-filters.test.tsx`, which finds the
 * trigger by that name with a range set — the one state where nothing else
 * would name it `"Created between"`; removing both makes axe's `button-name`
 * fire in `a11y.test.tsx`.
 *
 * **The cost, stated rather than hidden:** once a range is picked, nothing
 * *visible* labels it as the "created" filter (only the Status field carries
 * a visible label). A sighted operator returning to a bookmarked filtered URL
 * sees two bare dates beside a labelled Status field. Fixing it needs either
 * a `FormField` label pointed at `DatePickerTrigger`'s `id` (possible since
 * 0.3.0) or a visible label this component owns, and both are decisions
 * above this file.
 *
 * **The range is committed on Save, not per click (0.3.0).** Tapping days
 * only stages a pick; `onValueChange` — `setRange` here — fires once, when
 * the operator presses Save, and Cancel, Escape or a press outside throw the
 * pick away. The range rule is M3's: the first tap sets a start with an open
 * end, a tap on or after it sets the end, and a tap on a whole range starts
 * a new one. Two consequences an operator will notice against 0.2.x, which
 * committed every click and used `react-day-picker`'s rule: a single tap
 * then Save is `{ from, to: undefined }`, which `apply()` sends as
 * `created_from` alone — "from that day on", where 0.2.x's first click was a
 * one-day range; and a new start no longer needs Clear first, because a tap
 * on a whole range starts a new one rather than extending it. While the
 * picker is open the rest of the page is inert, so Apply beside an open
 * picker takes two presses: the first only closes it.
 *
 * **One line, and `overflow-x-auto` is what makes that true rather than
 * merely literal.** `flex` with no `flex-wrap` guarantees the controls never
 * go to a second row — but on 0.2.x nothing here shrank, and that was
 * measured rather than reasoned: rendered against this app's own compiled
 * `globals.css` in a real browser, the three controls were 211 px, 262 px
 * and 71 px at *every* container width from 1104 px down to 320 px. The
 * `<select>`'s intrinsic minimum is its widest option
 * (`requires_payment_method`), and the picker's `truncate` never engaged
 * because its trigger sat in a block wrapper the row could not squeeze. So
 * below ~568 px of content width the row simply spilled, and — being a
 * direct child of `ScreenStack`, whose own `min-w-0` protects the column and
 * not its children — it took the whole document into horizontal scroll with
 * it. That is the failure `ScreenStack`'s doc names in as many words, and
 * `PaymentsTable` is already wrapped against it the same way. `min-w-0` lets
 * the row be narrower than its contents; `overflow-x-auto` keeps the spill
 * inside the row's own box. Measured after: at 375 px the document's
 * `scrollWidth` equals its `clientWidth`; at 1280 px nothing changes at all,
 * because there is nothing to scroll.
 *
 * **0.3.0 changed the middle control's width, and not by anything written
 * here.** The block wrapper is gone: `DatePickerTrigger`'s own
 * `inline-flex w-full` box is now the row's flex item, so the trigger takes
 * whatever the row has left and shrinks before the row spills. Measured in
 * the built Storybook `Filters` story (the row alone, 2026-09-24): 932 px in
 * a 1248 px row, 352 px in a 668 px row, and at 375 px it gives way to
 * ~158 px empty before the row spills anyway (`scrollWidth` 473 against
 * `clientWidth` 343) — with the document still at `scrollWidth` 375, so the
 * wrapper above still does its job. Pinning the old width back is a
 * `className` on `DatePickerTrigger`, and a layout decision this migration
 * did not take.
 *
 * Safe with the calendar, checked and not assumed — and for a different
 * reason than before. 0.2.x portalled its panel (`PopoverPanel`'s `anchor`
 * forced `portal`), so it was not a descendant of this scroll container.
 * 0.3.0 renders every picker surface inline, as a descendant, but
 * `position: fixed` (the docked panel placed under the trigger by Floating
 * UI from 640 px up, the full-screen range picker below), and a fixed box is
 * not clipped by an `overflow` ancestor unless something between them has a
 * `transform`, `filter`, `backdrop-filter`, `contain` or `will-change` —
 * nothing here or in `ScreenStack` does. Measured: at 1280 px the docked
 * panel is 714×398 px under a 50 px-tall row, and its Save button, ~370 px
 * below the row's bottom edge, took a real click.
 *
 * Applying always starts from the first page: the built URL never carries
 * `after`/`before`, because `starting_after`/`ending_before` name a row in
 * the *previous* result set, and carrying one across a filter change would
 * ask for "the page after a payment that is no longer in this list".
 */
export function PaymentsFilters({ values }: PaymentsFiltersProps) {
  const router = useRouter();
  // Local state, seeded once from `values` — the same "reads the URL once,
  // on mount" contract the `<input defaultValue>`s this replaces already
  // had. A filter change elsewhere (the pager, a pasted URL) remounts this
  // screen's data, not this component, so re-seeding on every render is not
  // a gap this change introduces.
  const [status, setStatus] = useState(values.status);
  const [range, setRange] = useState<IsoDateRange | undefined>(
    rangeFromValues(values),
  );

  function apply() {
    const search = new URLSearchParams();
    if (status.length > 0) {
      search.set("status", status);
    }
    if (range?.from !== undefined) {
      search.set("created_from", range.from);
    }
    if (range?.to !== undefined) {
      search.set("created_to", range.to);
    }
    const query = search.toString();
    router.push(query.length > 0 ? `/payments?${query}` : "/payments");
  }

  return (
    <div className="flex min-w-0 items-end gap-3 overflow-x-auto">
      <FormField label="Status" htmlFor="payments-filter-status">
        <select
          id="payments-filter-status"
          name="status"
          value={status}
          onChange={(event) => setStatus(event.target.value)}
        >
          <option value={ANY_STATUS}>Any status</option>
          {PAYMENT_STATUS.map((paymentStatus) => (
            <option key={paymentStatus} value={paymentStatus}>
              {paymentStatus}
            </option>
          ))}
        </select>
      </FormField>

      <DateRangePicker value={range} onValueChange={setRange}>
        <DatePickerTrigger aria-label="Created between">
          <DatePickerValue placeholder="Created between" />
          <DatePickerClear />
        </DatePickerTrigger>
        <DatePickerContent />
      </DateRangePicker>

      <Button type="button" variant="secondary" onClick={apply}>
        Apply
      </Button>
    </div>
  );
}
