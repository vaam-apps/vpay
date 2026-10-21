import {
  IdDisplay,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@vaam-apps/ui";
import NextLink from "next/link";
import type { ReactNode } from "react";

/**
 * One column of a {@link ProcedureTable}.
 *
 * `render` returns a `ReactNode` rather than a string so a column can show a
 * `Code` id or a status chip, and takes the whole row rather than one field
 * so a column can combine two — `sent_at` is only meaningful beside `state`.
 */
export interface ProcedureColumn<Row> {
  /** The column heading, as an operator reads it. */
  readonly header: string;
  /** A stable key; the field name where there is one. */
  readonly key: string;
  /** What this cell shows for `row`. */
  readonly render: (row: Row) => React.ReactNode;
  /**
   * Hide this column below `sm` (640px).
   *
   * `Table` already wraps itself in `w-full overflow-x-auto`, so a wide
   * table *can* be swiped sideways on a phone rather than being clipped
   * — measured, not assumed. But a seven-column table on a 375px screen
   * puts five of its columns off-screen with no affordance saying so, and
   * an operator who does not know to swipe reads the first two columns as
   * the whole answer. Marking the columns that are not the answer keeps
   * the phone view to the ones that are, and the rest return at `sm`.
   *
   * `Table`'s own `hideBelow` prop carries it since `@vaam-apps/ui` 0.5.0
   * (`verify-ui` refuses a computed `className` anywhere in an app, which is
   * why this was two literal branches before).
   */
  readonly secondary?: boolean;
  /** `"end"` for a number (an amount), so a column of them aligns. */
  readonly align?: "start" | "end";
}

export interface ProcedureTableProps<Row> {
  /** Exactly the rows the procedure answered with. Never padded. */
  readonly rows: readonly Row[];
  /** The columns, in the order they are read. */
  readonly columns: readonly ProcedureColumn<Row>[];
  /** `row.id`, as a string, for React's key. */
  readonly idOf: (row: Row) => string;
  /** Named for the screen reader, since three tables now share this shell. */
  readonly caption: string;
}

/**
 * The table the three procedure-backed lists share.
 *
 * # Why one component and not three
 *
 * Refunds, deliveries and customers differ only in their columns: none has a
 * filter, none has a detail route yet, and all three are offset-paged the
 * same way. Three copies of this markup would be three places for the table
 * semantics — the caption, the header scope, the empty `<tbody>` — to drift
 * apart, and `a11y.test.tsx` would only ever be pointed at one of them.
 *
 * `payments-table.tsx` is deliberately **not** folded in here. It carries a
 * link to a detail route, a status pill with its own contrast test, and a
 * cursor pager; generalising this component far enough to hold it would make
 * it the union of two screens rather than the shape of one.
 *
 * # Every cell comes from the row
 *
 * There is no placeholder and no "—" invented here: a column that has nothing
 * to show renders what its own `render` returns, and `format.ts`'s `ABSENT`
 * is the one spelling of "this field is null". A table that filled a gap with
 * plausible text would be the failure `CLAUDE.md` names first.
 */
export function ProcedureTable<Row>({
  rows,
  columns,
  idOf,
  caption,
}: ProcedureTableProps<Row>) {
  return (
    <Table>
      <caption className="sr-only">{caption}</caption>
      <TableHeader>
        <TableRow>
          {columns.map((column) => (
            <TableHead
              key={column.key}
              scope="col"
              align={column.align ?? "start"}
              hideBelow={column.secondary === true ? "sm" : undefined}
            >
              {column.header}
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row) => (
          <TableRow key={idOf(row)}>
            {columns.map((column) => (
              <TableCell
                key={column.key}
                align={column.align ?? "start"}
                hideBelow={column.secondary === true ? "sm" : undefined}
              >
                {column.render(row)}
              </TableCell>
            ))}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

/**
 * An id, rendered the way every id in this app is rendered: `IdDisplay`'s
 * table variant — the prefix and the tail, the full value on hover and in
 * its copy button — so a column of `re_…` ids stays one line on a phone.
 */
export function IdCell({ value }: { readonly value: string }) {
  return <IdDisplay value={value} truncateTo={14} />;
}

/**
 * A payment id that opens the payment — the one cross-link every list with a
 * `payment_intent_id` gains (2026-10-21; before, refunds and checkouts
 * printed the id and an operator had to paste it into the URL bar). The id
 * truncates below `sm` with CSS only, so the link's text — and its
 * accessible name — is always the whole id.
 */
export function PaymentLinkCell({
  id,
  compact = false,
}: {
  readonly id: string;
  /**
   * Truncate at every width, not only below `sm` — for a list where the
   * payment is a cross-reference beside the row's own id (refunds,
   * checkouts), so the columns that are the row's subject keep the room.
   */
  readonly compact?: boolean;
}) {
  return (
    <NextLink
      href={`/payments/${id}`}
      className="block font-mono text-primary hover:underline"
    >
      {compact ? (
        <span className="block max-w-[14ch] truncate">{id}</span>
      ) : (
        <span className="block max-w-[12ch] truncate sm:max-w-none">{id}</span>
      )}
    </NextLink>
  );
}

/**
 * A row's amount repeated under its id below `sm`, where the Amount column
 * steps aside so the status still fits a phone's width without a sideways
 * swipe. Hidden from `sm` up, where the column is back.
 */
export function PhoneAmount({ children }: { readonly children: ReactNode }) {
  return (
    <span className="block text-body-small text-muted-foreground sm:hidden">
      {children}
    </span>
  );
}
