import type { PaymentStatus } from '@vpay/tokens';

import { StatusBadge, Table } from '@vpay/ui';

export interface PaymentRow {
  id: string;
  amount: string;
  status: PaymentStatus;
}

export interface PaymentsTableProps {
  rows: PaymentRow[];
}

/**
 * Payments list recipe: `Table` for structure, `StatusBadge` for the
 * status column — the same badge the design-system reference on `/`
 * renders, so a status can never read a different colour in the list than
 * in the legend (both compose {@link StatusBadge}, which takes its tone
 * from `@vpay/tokens`, never its own).
 *
 * Takes `rows` as a prop rather than fetching or fabricating them: this
 * file has no opinion about `/dash/v1`, and rendering invented rows here
 * would be exactly the failure mode AGENTS.md names first — "rendering
 * fake rows in the dashboard so a screenshot looks good".
 */
export function PaymentsTable({ rows }: PaymentsTableProps) {
  return (
    <Table zebra>
      <thead>
        <tr>
          <th>Payment</th>
          <th>Amount</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={row.id}>
            <td>{row.id}</td>
            <td>{row.amount}</td>
            <td>
              <StatusBadge status={row.status} />
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
