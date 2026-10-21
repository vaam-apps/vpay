"use client";

import { PAYMENT_STATUS, statusLabel } from "@vpay/tokens";
import {
  DateRangePicker,
  DatePickerClear,
  DatePickerContent,
  DatePickerTrigger,
  DatePickerValue,
  FormField,
  type IsoDateRange,
  Skeleton,
} from "@vaam-apps/ui";
import { useRouter } from "next/navigation";

import { FilterChips } from "./filter-chips";
import { FilterChipsSkeleton } from "./list-skeleton";

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
 * `/payments` with these filters and **no cursor**: a cursor names a row in
 * the previous result set, and carrying it across a filter change asks for
 * "the page after a payment that is no longer in this list" — an answer
 * that looks like a working page.
 */
export function paymentsHref(values: PaymentsFilterValues): string {
  const search = new URLSearchParams();
  if (values.status.length > 0) {
    search.set("status", values.status);
  }
  if (values.createdFrom.length > 0) {
    search.set("created_from", values.createdFrom);
  }
  if (values.createdTo.length > 0) {
    search.set("created_to", values.createdTo);
  }
  const query = search.toString();
  return query.length > 0 ? `/payments?${query}` : "/payments";
}

/** The two dates in `DateRangePicker`'s shape; all-empty is `undefined`. */
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
 * The status and date-range filters above the payments table.
 *
 * # One tap, no Apply (2026-10-21, the Material 3 Expressive revamp)
 *
 * The status was a native `<select>` plus an Apply button, so changing a
 * filter took three actions and a filter half-chosen sat on screen looking
 * applied. Status is now a row of M3 filter chips — `FilterChips`, the same
 * component every list in this app filters with — one per `PAYMENT_STATUS`
 * value from `@vpay/tokens`, labelled with the words the status pill uses,
 * so a status can never be filterable but unrenderable. A chip is a link to
 * the filtered URL. The date range navigates when the picker's Save commits
 * it (`onValueChange` fires there and nowhere else, since `@vaam-apps/ui`
 * 0.3.0) or when Clear empties it.
 *
 * The URL stays the only source of truth: nothing here holds filter state,
 * so a filtered list is still bookmarkable and pasteable into a ticket, and
 * the page is still read by its Server Component. Every href drops the
 * cursor ({@link paymentsHref}).
 *
 * # The date field's name and description
 *
 * `FormField` labels it "Created between": the label's `htmlFor` is the
 * trigger's `id` (`payments-filter-created`), so the `<label for>` names it,
 * and `DatePickerValue`'s text is its description — "Any time" while empty,
 * the dates once picked. Clicking the label focuses the trigger and opens
 * nothing (Headless UI's `Label` cancels the native click); Enter then
 * opens it.
 */
export function PaymentsFilters({ values }: PaymentsFiltersProps) {
  const router = useRouter();
  return (
    <div className="flex min-w-0 flex-col gap-4 lg:flex-row lg:items-end">
      <div className="min-w-0 flex-1">
        <FilterChips
          label="Filter payments by status"
          options={[
            {
              href: paymentsHref({ ...values, status: ANY_STATUS }),
              label: "All",
              active: values.status === ANY_STATUS,
            },
            ...PAYMENT_STATUS.map((status) => ({
              href: paymentsHref({ ...values, status }),
              label: statusLabel[status],
              active: values.status === status,
            })),
          ]}
        />
      </div>
      <FormField
        label="Created between"
        htmlFor="payments-filter-created"
        className="max-w-full"
      >
        <DateRangePicker
          value={rangeFromValues(values)}
          onValueChange={(range) =>
            router.push(
              paymentsHref({
                ...values,
                createdFrom: range?.from ?? "",
                createdTo: range?.to ?? "",
              }),
            )
          }
        >
          <DatePickerTrigger
            id="payments-filter-created"
            className="w-[calc(25ch+16px+3.5rem)] max-w-full font-mono"
          >
            <DatePickerValue placeholder="Any time" />
            <DatePickerClear />
          </DatePickerTrigger>
          <DatePickerContent />
        </DateRangePicker>
      </FormField>
    </div>
  );
}

/**
 * The filter row's placeholder, the same footprint as the real one so
 * nothing below it moves when the screen arrives: the chips are
 * `FilterChipsSkeleton` over the same labels, and the date field's label is
 * an invisible copy of its text. Inert and `aria-hidden`; the `role="status"`
 * that announces the wait is the screen skeleton's.
 */
export function PaymentsFiltersSkeleton() {
  return (
    <div
      aria-hidden="true"
      inert
      className="flex min-w-0 flex-col gap-4 lg:flex-row lg:items-end"
    >
      <div className="min-w-0 flex-1">
        <FilterChipsSkeleton
          labels={PAYMENT_STATUS.map((status) => statusLabel[status])}
        />
      </div>
      <div className="flex max-w-full flex-col gap-1.5">
        <Skeleton className="w-fit text-body font-medium">
          <span className="invisible">Created between</span>
        </Skeleton>
        <Skeleton className="h-10 w-[calc(25ch+16px+3.5rem)] max-w-full font-mono" />
      </div>
    </div>
  );
}
