"use client";

import { PAYMENT_STATUS } from "@vpay/tokens";
import { Button, Field, FieldLabel, Input, Select, Stack } from "@vpay/ui";

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
 * The status list is `PAYMENT_STATUS` from `@vpay/tokens` — the same five
 * values `StatusBadge` colours — so a status can never be filterable but
 * unrenderable, or the other way round. vpay answers `400` naming `status`
 * for a value outside its own vocabulary rather than an empty list, which is
 * the right answer and one this form cannot provoke.
 */
export function PaymentsFilters({ values }: PaymentsFiltersProps) {
  return (
    <form method="get">
      <Stack gap="md" align="end" wrap>
        <Field>
          <FieldLabel htmlFor="payments-filter-status">Status</FieldLabel>
          <Select
            id="payments-filter-status"
            name="status"
            defaultValue={values.status}
            placeholder="Any status"
            items={[
              { value: ANY_STATUS, label: "Any status" },
              ...PAYMENT_STATUS.map((status) => ({
                value: status,
                label: status,
              })),
            ]}
          />
        </Field>

        <Field>
          <FieldLabel htmlFor="payments-filter-from">Created from</FieldLabel>
          <Input
            id="payments-filter-from"
            name="created_from"
            type="date"
            defaultValue={values.createdFrom}
          />
        </Field>

        <Field>
          <FieldLabel htmlFor="payments-filter-to">Created to</FieldLabel>
          <Input
            id="payments-filter-to"
            name="created_to"
            type="date"
            defaultValue={values.createdTo}
          />
        </Field>

        <Button type="submit" variant="outline">
          Apply
        </Button>
      </Stack>
    </form>
  );
}
