"use client";

import { PAYMENT_STATUS } from "@vpay/tokens";
import {
  Button,
  DateRangePicker,
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
 * `{ from: undefined, to: undefined }` — the picker's own `hasValue` check
 * (`date-picker.d.ts`) treats those the same for display, but only the
 * `undefined` form is what a freshly-mounted, filter-free screen should hand
 * it.
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
 * **No `FormField` around the date range.** `FormField`'s `htmlFor` mode
 * clones its child with `aria-describedby`/`aria-invalid` and renders a
 * `<label htmlFor>` expecting the child to carry that same `id` — but
 * `DateRangePickerProps` (`date-picker.d.ts`) has no `id` to give it, so the
 * label would point at nothing. The picker supplies its own accessible name
 * instead: its trigger always renders `placeholder` in a `sr-only` span
 * (`PickerTrigger` in the package's own source), the mechanism
 * `DatePickerProps.placeholder`'s doc comment states outright ("used as the
 * control's accessible name") — note that is `DatePicker`'s prop; the
 * range variant's identical `placeholder` carries no doc comment of its own
 * and shares only the implementation.
 *
 * So the claim was read off a real render rather than off the `.d.ts`: the
 * trigger's computed name is `"Created between Created between"` empty and
 * `"2026-09-01 → 2026-09-07 Created between"` with a range — doubled while
 * empty because the visible label falls back to the placeholder too, which
 * is the package's business and not a defect here. `a11y.test.tsx` holds
 * it: emptying this `placeholder` makes `button-name` fire there.
 *
 * **The cost, stated rather than hidden:** once a range is picked, nothing
 * *visible* says what those two dates mean — the only remaining "Created
 * between" is the `sr-only` one. A sighted operator returning to a
 * bookmarked filtered URL sees two bare dates beside a labelled Status
 * field. Fixing it needs either an `id` on `DateRangePickerProps` or a
 * visible label this component owns, and both are decisions above this
 * file.
 *
 * **One line, and `overflow-x-auto` is what makes that true rather than
 * merely literal.** `flex` with no `flex-wrap` guarantees the controls never
 * go to a second row — but nothing here shrinks, and that was measured
 * rather than reasoned: rendered against this app's own compiled
 * `globals.css` in a real browser, the three controls are 211 px, 262 px and
 * 71 px at *every* container width from 1104 px down to 320 px. The
 * `<select>`'s intrinsic minimum is its widest option
 * (`requires_payment_method`), and the picker's `truncate` never engages
 * because its trigger sits in a block wrapper the row cannot squeeze. So
 * below ~568 px of content width the row simply spilled, and — being a
 * direct child of `ScreenStack`, whose own `min-w-0` protects the column and
 * not its children — it took the whole document into horizontal scroll with
 * it. That is the failure `ScreenStack`'s doc names in as many words, and
 * `PaymentsTable` is already wrapped against it the same way. `min-w-0` lets
 * the row be narrower than its contents; `overflow-x-auto` keeps the spill
 * inside the row's own box. Measured after: at 375 px the document's
 * `scrollWidth` equals its `clientWidth`; at 1280 px nothing changes at all,
 * because there is nothing to scroll. Safe with the calendar, checked and
 * not assumed: `PopoverPanel`'s `anchor` prop forces `portal`
 * (`@headlessui/react`'s `popover.js` — `m&&(b=!0)`), so the panel is not a
 * descendant of this scroll container and cannot be clipped by it.
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

      <DateRangePicker
        value={range}
        onValueChange={setRange}
        placeholder="Created between"
      />

      <Button type="button" variant="secondary" onClick={apply}>
        Apply
      </Button>
    </div>
  );
}
