"use client";

import { PAYMENT_STATUS } from "@vpay/tokens";
import { Button, FormField, Input } from "@vaam-apps/ui";

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
 * The status and date-range filters, as a plain `GET` form.
 *
 * `method="get"` and no `onSubmit`: the filters live in the URL, which is
 * what makes a filtered list a thing an operator can bookmark, reload and
 * paste into a ticket. It also means the whole screen works with JavaScript
 * disabled, and that the server component behind it has one source of truth
 * for what is being asked — `searchParams` — rather than a second copy in
 * client state that can disagree with the address bar.
 *
 * Submitting resets paging, because the cursor is not rendered as a field:
 * `starting_after` names a row in the *previous* result set, and carrying it
 * across a filter change would ask for "the page after a payment that is no
 * longer in this list".
 *
 * **The status control is a native `<select name="status">`, not
 * `@vaam-apps/ui`'s `Select`.** That `Select` wraps a Headless UI `Listbox`
 * and forwards no `name` — there is no hidden input, so it never puts its
 * value into the form at all. Wired into this `method="get"` form it would
 * render, look right, be clickable and silently do nothing: every submit
 * would drop the status and the list would come back unfiltered with no
 * sign anything had gone wrong. A native `<select>` is the only shape that
 * keeps `new FormData(form).get("status")` working, with no client state and
 * no JavaScript. See `payments-filters.test.tsx`, which exists for exactly
 * this failure.
 *
 * The status list is `PAYMENT_STATUS` from `@vpay/tokens` — the same five
 * values `../payment-status.ts` renders as a pill — so a status can never be
 * filterable but unrenderable, or the other way round. vpay answers `400`
 * naming `status` for a value outside its own vocabulary rather than an
 * empty list, which is the right answer and one this form cannot provoke.
 */
export function PaymentsFilters({ values }: PaymentsFiltersProps) {
  return (
    <form method="get">
      <div>
        <FormField label="Status" htmlFor="payments-filter-status">
          <select
            id="payments-filter-status"
            name="status"
            defaultValue={values.status}
          >
            <option value={ANY_STATUS}>Any status</option>
            {PAYMENT_STATUS.map((status) => (
              <option key={status} value={status}>
                {status}
              </option>
            ))}
          </select>
        </FormField>

        <FormField label="Created from" htmlFor="payments-filter-from">
          <Input
            id="payments-filter-from"
            name="created_from"
            type="date"
            defaultValue={values.createdFrom}
          />
        </FormField>

        <FormField label="Created to" htmlFor="payments-filter-to">
          <Input
            id="payments-filter-to"
            name="created_to"
            type="date"
            defaultValue={values.createdTo}
          />
        </FormField>

        <Button type="submit" variant="secondary">
          Apply
        </Button>
      </div>
    </form>
  );
}
