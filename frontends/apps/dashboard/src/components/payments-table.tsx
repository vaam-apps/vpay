import NextLink from "next/link";

import {
  Code,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@vaam-apps/ui";

import { PaymentStatusPill } from "../payment-status";
import {
  ABSENT,
  asPaymentStatus,
  formatAmount,
  formatInstant,
  formatMethods,
} from "../format";
import type { PaymentIntentObject } from "../server/api";

export interface PaymentsTableProps {
  /** Exactly the rows `/dash/v1/payment_intents` answered with. */
  rows: readonly PaymentIntentObject[];
}

/**
 * The payments list.
 *
 * `Table` for structure, `PaymentStatusPill` for the status column — the one
 * place a `PaymentIntent` status is drawn (`../payment-status.ts`), so a
 * status can never read a different presentation in the list than it does on
 * the detail page.
 *
 * # The two columns this table does not have, and why absence is the honest
 * shape
 *
 * **The rail.** `charges.provider_code` is which rail actually took a
 * payment, and `GET /dash/v1/payment_intents` does not return a charge at all
 * — the list renders `PaymentIntentObject`, whose `payment_method_types` is
 * the set of rails an intent *may* be confirmed against. That set is here,
 * under the heading **Methods**, because that is what it is. A column headed
 * "Rail" filled from it would be wrong for every intent that offers two and
 * was taken by one.
 *
 * **The payer.** `charges.payer_ref_masked` is `NULL` on every row this
 * deployment has ever written — the confirm path stores `None` in it and in
 * the unmasked `payer_ref` (`docs/status.md`) — and, like the rail, it is not
 * in this response in the first place. The detail page renders it, from the
 * column, as {@link ABSENT}, and says so. Deriving a mask here would mean
 * reading the unmasked number into a staff surface to make a column look
 * populated.
 *
 * `rows` is a prop. Nothing in this file fetches, and nothing in it invents a
 * row — "rendering fake rows in the dashboard so a screenshot looks good" is
 * the failure mode `CLAUDE.md` names third.
 */
export function PaymentsTable({ rows }: PaymentsTableProps) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead scope="col">Payment</TableHead>
          <TableHead scope="col">Created (UTC)</TableHead>
          <TableHead scope="col">Amount</TableHead>
          <TableHead scope="col">Status</TableHead>
          <TableHead scope="col">Methods</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row) => {
          const status = asPaymentStatus(row.status);
          return (
            <TableRow key={row.id} data-payment-id={row.id}>
              <TableCell>
                <NextLink href={`/payments/${row.id}`}>
                  <Code>{row.id}</Code>
                </NextLink>
              </TableCell>
              <TableCell>{formatInstant(row.created)}</TableCell>
              <TableCell>{formatAmount(row.amount, row.currency)}</TableCell>
              <TableCell>
                {/*
                  A status this build cannot name is rendered as the raw
                  string rather than coloured as something it is not: the
                  deployment holds a value newer than this bundle, and a
                  glyph on an unknown status is a claim.
                */}
                {status === null ? (
                  <span>{row.status}</span>
                ) : (
                  <PaymentStatusPill state={status} />
                )}
              </TableCell>
              <TableCell>{formatMethods(row.payment_method_types)}</TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

/** Re-exported so a caller can render the same dash this table would. */
export { ABSENT };
