import { Table } from "../table";

export type DataListProps = Omit<
  React.ComponentPropsWithoutRef<typeof Table>,
  "zebra" | "children"
> & { children: React.ReactNode };

/**
 * A list of labelled facts about one object — the payment-detail summary,
 * the charge, anything shaped "field: value".
 *
 * A `<table>` with a row header per row, not a `<dl>`: the rows are
 * two-column data, `<th scope="row">` is what tells a screen reader which
 * label owns which cell, and daisyUI 5 styles `table` and nothing about
 * `dl`. `payment-detail.tsx` wrote this markup by hand twice, twenty rows
 * between them, and every row repeated `scope="row"` — one wrong `scope`
 * there is an unreadable table with every gate green.
 *
 * `zebra` is deliberately not exposed: banding is for scanning many rows of
 * the same shape, and this is one object.
 */
export function DataList({ children, ...rest }: DataListProps) {
  return (
    <Table {...rest}>
      <tbody>{children}</tbody>
    </Table>
  );
}

export interface DataListRowProps extends Omit<
  React.ComponentPropsWithoutRef<"tr">,
  "children"
> {
  /** The field name. Rendered as the row's header cell. */
  label: React.ReactNode;
  /** The value. Rendered in the row's one data cell. */
  children: React.ReactNode;
}

/** One labelled fact inside a {@link DataList}. */
export function DataListRow({ label, children, ...rest }: DataListRowProps) {
  return (
    <tr {...rest}>
      <th scope="row">{label}</th>
      <td>{children}</td>
    </tr>
  );
}
