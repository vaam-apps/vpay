import { Card, InlineEmptyState } from "@vaam-apps/ui";

import {
  offsetFrom,
  readProcedurePage,
  singleParam,
} from "../dash/procedure-list";
import { requireStaff } from "../server/session";
import { FilterChips } from "./filter-chips";
import { PageHeader } from "./layout";
import { ProcedurePager } from "./procedure-pager";
import { ProcedureTable, type ProcedureColumn } from "./procedure-table";
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

export interface ProcedureListScreenProps<Row> {
  readonly title: string;
  readonly description: string;
  /** The `DashResource` this list reads, e.g. `REFUNDS`. */
  readonly resource: string;
  /** This screen's own route, e.g. `/refunds`. */
  readonly basePath: string;
  /** The page's `searchParams`, already awaited. */
  readonly params: Record<string, string | string[] | undefined>;
  readonly columns: readonly ProcedureColumn<Row>[];
  readonly idOf: (row: Row) => string;
  readonly filter?: ProcedureFilter;
  /** What an unfiltered empty list means, e.g. "No refunds. …". */
  readonly emptyMessage: string;
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
 * # A refused read is never an empty list
 *
 * `ReadFailure` with the request id, the session untouched — the same rule
 * `/payments` follows, and the reason the outage branch exists at all.
 */
export async function ProcedureListScreen<Row>({
  title,
  description,
  resource,
  basePath,
  params,
  columns,
  idOf,
  filter,
  emptyMessage,
}: ProcedureListScreenProps<Row>) {
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
  const result = await readProcedurePage<Row>(
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
      {!result.ok ? (
        <ReadFailure failure={result.failure} />
      ) : result.value.items.length === 0 ? (
        <Card>
          <InlineEmptyState
            variant="standalone"
            message={
              chosen === ""
                ? emptyMessage
                : `Nothing matches this filter. Choose "All" to see every row.`
            }
          />
        </Card>
      ) : (
        <Card className="overflow-hidden">
          <ProcedureTable
            rows={result.value.items}
            columns={columns}
            idOf={idOf}
            caption={title}
          />
          <div className="border-t border-edge px-4 py-3">
            <ProcedurePager
              basePath={basePath}
              offset={offset}
              returned={result.value.items.length}
              totalCount={result.value.totalCount}
              hasNext={result.value.pageInfo.hasNextPage}
              query={query}
            />
          </div>
        </Card>
      )}
    </>
  );
}
