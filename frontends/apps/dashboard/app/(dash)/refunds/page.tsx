import { REFUNDS_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import { REFUNDS } from "../../../src/dash/resource-name";
import { RefundsList } from "./refunds-list";

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

export default async function RefundsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen
      {...REFUNDS_SCREEN}
      resource={REFUNDS}
      basePath="/refunds"
      params={await searchParams}
      List={RefundsList}
      emptyMessage="No refunds. This merchant has no refunds yet."
    />
  );
}
