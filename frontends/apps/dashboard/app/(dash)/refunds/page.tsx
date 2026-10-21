import { REFUNDS_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import {
  IdCell,
  PaymentLinkCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { Amount, IsoInstant } from "../../../src/components/values";
import { REFUNDS } from "../../../src/dash/resource-name";
import { ABSENT } from "../../../src/format";
import { asRefundStatus, RefundStatusPill } from "../../../src/status-systems";

/** Exactly the fields `type RefundSummary` declares. */
interface RefundRow {
  readonly id: string;
  readonly payment_intent_id: string;
  readonly amount: number;
  readonly currency_code: string;
  readonly status: string;
  readonly reason: string | null;
  readonly failure_code: string | null;
  readonly created_at: string;
}

/**
 * `/refunds` — one page of this merchant's refunds.
 *
 * A Server Component, so the read happens where the token is, exactly as
 * `/payments` does. The rows come from `procedure searchRefunds`, whose
 * tenancy predicate is a **join** onto `payment_intents` because
 * `model Refund` carries no `merchant_id` of its own — the property its own
 * container test proves by replacing that predicate with a tautology.
 *
 * **`failure_raw` is not shown and is not available**: the procedure's
 * summary type does not carry it, deliberately. It is up to 2 000 characters
 * of a rail's own prose and belongs to a detail read, which does not exist
 * yet. `failure_code` is the part an operator reads from a list.
 */
export const dynamic = "force-dynamic";

const COLUMNS: readonly ProcedureColumn<RefundRow>[] = [
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

export default async function RefundsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen<RefundRow>
      {...REFUNDS_SCREEN}
      resource={REFUNDS}
      basePath="/refunds"
      params={await searchParams}
      columns={COLUMNS}
      idOf={(r) => r.id}
      emptyMessage="No refunds. This merchant has no refunds yet."
    />
  );
}
