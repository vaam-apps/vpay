import { InlineEmptyState, ScreenStack } from "@vaam-apps/ui";

import { AppShell } from "../../../src/components/app-shell";
import { ReadFailure } from "../../../src/components/read-failure";
import {
  IdCell,
  ProcedureTable,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { REFUNDS } from "../../../src/dash/resource-name";
import {
  formatIsoInstant,
  offsetFrom,
  readProcedurePage,
} from "../../../src/dash/procedure-list";
import { ABSENT, formatAmount } from "../../../src/format";
import { signOut } from "../../../src/server/actions";
import { requireStaff } from "../../../src/server/session";
import { ProcedurePager } from "../../../src/components/procedure-pager";

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
    render: (r) => <IdCell value={r.payment_intent_id} />,
  },
  {
    key: "amount",
    header: "Amount",
    render: (r) => formatAmount(r.amount, r.currency_code),
  },
  { key: "status", header: "Status", render: (r) => r.status },
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
    header: "Created",
    render: (r) => formatIsoInstant(r.created_at),
  },
];

export default async function RefundsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <ScreenStack>
        <h2>Refunds</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const staff = gate.staff;
  const offset = offsetFrom(await searchParams);
  const result = await readProcedurePage<RefundRow>(staff, REFUNDS, offset);

  return (
    <AppShell
      email={staff.session.email}
      merchantId={staff.session.merchant_id}
      signOut={signOut}
    >
      <ScreenStack>
        <h2>Refunds</h2>
        {!result.ok ? (
          <ReadFailure failure={result.failure} />
        ) : result.value.items.length === 0 ? (
          <InlineEmptyState
            variant="standalone"
            message="No refunds. This merchant has no refunds yet."
          />
        ) : (
          <>
            <ProcedureTable
              rows={result.value.items}
              columns={COLUMNS}
              idOf={(r) => r.id}
              caption="Refunds"
            />
            <ProcedurePager
              basePath="/refunds"
              offset={offset}
              returned={result.value.items.length}
              totalCount={result.value.totalCount}
              hasNext={result.value.pageInfo.hasNextPage}
            />
          </>
        )}
      </ScreenStack>
    </AppShell>
  );
}
