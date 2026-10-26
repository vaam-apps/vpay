"use client";

import type { RefundSummary } from "@vpay/dash-client";

import {
  IdCell,
  PaymentLinkCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import {
  ProcedureListView,
  type ProcedureListBodyProps,
} from "../../../src/components/procedure-list-view";
import { Amount, IsoInstant } from "../../../src/components/values";
import { ABSENT } from "../../../src/format";
import { asRefundStatus, RefundStatusPill } from "../../../src/status-systems";

/**
 * The refunds list's body: its columns, over the **generated**
 * `RefundSummary` — the one TypeScript copy of that wire shape, so a field
 * the schema drops is a compile error here after `just gen-dash-client`.
 * Until 2026-10-26 this was a hand-written `RefundRow` in `page.tsx`.
 *
 * A client module because the columns carry render functions, which cannot
 * cross from the Server Component as props.
 */
const COLUMNS: readonly ProcedureColumn<RefundSummary>[] = [
  { key: "id", header: "Refund", render: (r) => <IdCell value={r.id} /> },
  {
    key: "payment_intent_id",
    secondary: true,
    header: "Payment",
    render: (r) => <PaymentLinkCell id={r.payment_intent_id} compact />,
  },
  {
    key: "amount",
    header: "Amount",
    align: "end",
    render: (r) => <Amount minor={r.amount} currency={r.currency_code} />,
  },
  {
    key: "status",
    header: "Status",
    render: (r) => {
      const status = asRefundStatus(r.status);
      return status === null ? (
        r.status
      ) : (
        <RefundStatusPill state={status} size="sm" />
      );
    },
  },
  {
    key: "failure_code",
    secondary: true,
    header: "Failure",
    render: (r) => r.failure_code ?? ABSENT,
  },
  {
    key: "reason",
    secondary: true,
    header: "Reason",
    render: (r) => r.reason ?? ABSENT,
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created (UTC)",
    render: (r) => <IsoInstant value={r.created_at} />,
  },
];

export function RefundsList(props: ProcedureListBodyProps) {
  return (
    <ProcedureListView<RefundSummary>
      {...props}
      columns={COLUMNS}
      idOf={(r) => r.id}
    />
  );
}
