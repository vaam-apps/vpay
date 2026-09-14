import {
  Code,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@vaam-apps/ui";

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
   * This is a static class per column, never a computed one: `verify-ui`
   * refuses a computed `className` anywhere in an app.
   */
  readonly secondary?: boolean;
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
          {columns.map((column) =>
            column.secondary === true ? (
              <TableHead key={column.key} className="hidden sm:table-cell">
                {column.header}
              </TableHead>
            ) : (
              <TableHead key={column.key}>{column.header}</TableHead>
            ),
          )}
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row) => (
          <TableRow key={idOf(row)}>
            {columns.map((column) =>
              column.secondary === true ? (
                <TableCell key={column.key} className="hidden sm:table-cell">
                  {column.render(row)}
                </TableCell>
              ) : (
                <TableCell key={column.key}>{column.render(row)}</TableCell>
              ),
            )}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

/** An id, rendered the way every id in this app is rendered. */
export function IdCell({ value }: { readonly value: string }) {
  return <Code>{value}</Code>;
}
