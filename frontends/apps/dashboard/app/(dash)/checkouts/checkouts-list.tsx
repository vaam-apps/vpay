"use client";

import type { CheckoutSessionSummary } from "@vpay/dash-client";

import {
  IdCell,
  PaymentLinkCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import {
  ProcedureListView,
  type ProcedureListBodyProps,
} from "../../../src/components/procedure-list-view";
import { IsoInstant } from "../../../src/components/values";
import { ABSENT } from "../../../src/format";
import {
  asCheckoutStatus,
  CheckoutStatusPill,
} from "../../../src/status-systems";

/**
 * The checkouts list's body: its columns, over the **generated**
 * `CheckoutSessionSummary` — the one TypeScript copy of that wire shape, so a field
 * the schema drops is a compile error here after `just gen-dash-client`.
 * Until 2026-10-26 this was a hand-written `CheckoutRow` in `page.tsx`.
 *
 * A client module because the columns carry render functions, which cannot
 * cross from the Server Component as props.
 */
const COLUMNS: readonly ProcedureColumn<CheckoutSessionSummary>[] = [
  { key: "id", header: "Checkout", render: (r) => <IdCell value={r.id} /> },
  {
    key: "status",
    header: "Status",
    render: (r) => {
      const status = asCheckoutStatus(r.status);
      return status === null ? (
        r.status
      ) : (
        <CheckoutStatusPill state={status} size="sm" />
      );
    },
  },
  { key: "payment_status", header: "Payment", render: (r) => r.payment_status },
  {
    key: "payment_intent_id",
    secondary: true,
    header: "Intent",
    render: (r) => <PaymentLinkCell id={r.payment_intent_id} compact />,
  },
  {
    key: "ui_mode",
    secondary: true,
    header: "Mode",
    render: (r) => r.ui_mode,
  },
  {
    key: "customer_id",
    secondary: true,
    header: "Customer",
    render: (r) =>
      r.customer_id == null ? ABSENT : <IdCell value={r.customer_id} />,
  },
  {
    key: "expires_at",
    secondary: true,
    header: "Expires (UTC)",
    render: (r) => <IsoInstant value={r.expires_at} />,
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created (UTC)",
    render: (r) => <IsoInstant value={r.created_at} />,
  },
];

export function CheckoutsList(props: ProcedureListBodyProps) {
  return (
    <ProcedureListView<CheckoutSessionSummary>
      {...props}
      columns={COLUMNS}
      idOf={(r) => r.id}
    />
  );
}
