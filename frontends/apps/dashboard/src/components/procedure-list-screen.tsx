import type { ReactNode } from "react";

import {
  offsetFrom,
  readProcedureList,
  singleParam,
} from "../dash/procedure-list";
import type { ProcedureResource } from "../dash/resource-name";
import { requireStaff } from "../server/session";
import { FilterChips } from "./filter-chips";
import { PageHeader } from "./layout";
import type { ProcedureListBodyProps } from "./procedure-list-view";
import { ReadFailure } from "./read-failure";

/**
 * One status-like filter a procedure accepts, offered as chips.
 *
 * `param` is the URL's name for it and `field` the procedure's — the same
 * word for every list today (`status`, or `state` for deliveries), kept
 * separate so a rename on one side cannot silently stop the other reading.
 */
export interface ProcedureFilter {
  readonly param: string;
  readonly field: string;
  /** The accessible name of the chip set, e.g. "Filter by status". */
  readonly label: string;
  /** The vocabulary, in reading order, with the words an operator reads. */
  readonly options: readonly {
    readonly value: string;
    readonly label: string;
  }[];
}

export interface ProcedureListScreenProps {
  readonly title: string;
  readonly description: string;
  /** The `ProcedureResource` this list reads, e.g. `REFUNDS`. */
  readonly resource: ProcedureResource;
  /** This screen's own route, e.g. `/refunds`. */
  readonly basePath: string;
  /** The page's `searchParams`, already awaited. */
  readonly params: Record<string, string | string[] | undefined>;
  readonly filter?: ProcedureFilter;
  /** What an unfiltered empty list means, e.g. "No refunds. …". */
  readonly emptyMessage: string;
  /**
   * The list's client body — `app/(dash)/<list>/<list>-list.tsx`, which owns
   * the columns. A function between two Server Components, so nothing with
   * a render function in it ever crosses to the browser as a prop.
   */
  readonly List: (props: ProcedureListBodyProps) => ReactNode;
}

/**
 * The four procedure-backed lists — refunds, deliveries, checkouts,
 * customers — as one server component.
 *
 * # Why one screen and not four pages of markup
 *
 * The four pages were the same eighty lines with a different column list:
 * gate, read, failure-or-empty-or-table, pager. They drifted the moment one
 * gained something — only refunds had a description, none carried a filter
 * — so the shape lives here and each page is its columns and its words.
 *
 * # The filter is the procedure's own, and the URL is its source
 *
 * Each procedure declares a named filter type (`RefundListFilter`,
 * `WebhookDeliveryListFilter`, `CheckoutSessionListFilter` in
 * `schemas/vpay.cstack`). This passes the URL's value through unchanged; a
 * value vpay does not know is a `400` naming the field, which renders as a
 * refusal — never as an empty page that reads "none in this state". The
 * Overview's counts link here with these parameters, so a number on the
 * Overview always opens the rows it counted.
 *
 * # The read goes through `@cratestack/refine`, the body through Refine
 *
 * Since 2026-10-26 the page is read by `readProcedureList` — the generated
 * client's procedure, behind `@cratestack/refine`'s data provider — and the
 * table, empty state and pager render in `ProcedureListView`, a client
 * component that hands that read to Refine's `useList` as `initialData`.
 * The header and the filter chips stay here: they are links, and work
 * before hydration.
 *
 * # A refused read is never an empty list
 *
 * `ReadFailure` with the request id, the session untouched — the same rule
 * `/payments` follows, and the reason the outage branch exists at all.
 */
export async function ProcedureListScreen({
  title,
  description,
  resource,
  basePath,
  params,
  filter,
  emptyMessage,
  List,
}: ProcedureListScreenProps) {
  const header = <PageHeader title={title} description={description} />;
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <>
        {header}
        <ReadFailure failure={gate.failure} />
      </>
    );
  }

  const offset = offsetFrom(params);
  const chosen = filter === undefined ? "" : singleParam(params, filter.param);
  const initial = await readProcedureList(
    gate.staff,
    resource,
    offset,
    filter === undefined || chosen === "" ? {} : { [filter.field]: chosen },
  );
  const query =
    filter === undefined || chosen === ""
      ? ""
      : new URLSearchParams({ [filter.param]: chosen }).toString();

  return (
    <>
      {header}
      {filter === undefined ? null : (
        <FilterChips
          label={filter.label}
          options={[
            { href: basePath, label: "All", active: chosen === "" },
            ...filter.options.map((option) => ({
              href: `${basePath}?${new URLSearchParams({ [filter.param]: option.value }).toString()}`,
              label: option.label,
              active: chosen === option.value,
            })),
          ]}
        />
      )}
      <List
        resource={resource}
        title={title}
        basePath={basePath}
        filterField={filter?.field ?? null}
        chosen={chosen}
        query={query}
        offset={offset}
        emptyMessage={emptyMessage}
        initial={initial}
      />
    </>
  );
}
