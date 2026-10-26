import { DELIVERIES_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import { WEBHOOK_DELIVERIES } from "../../../src/dash/resource-name";
import { DeliveriesList } from "./deliveries-list";

/**
 * `/deliveries` — did this merchant's webhooks actually arrive?
 *
 * The rows come from `procedure searchWebhookDeliveries`, whose tenancy
 * predicate is a **join** onto `events`, because `model WebhookDelivery`
 * carries no `merchant_id`. Without that join the list would carry every
 * tenant's deliveries — including the URLs their endpoints are registered at
 * — and nothing would fail; its container test replaces the predicate with a
 * tautology to prove it fires.
 *
 * **`response_excerpt` is not shown and is not available**: up to 2 000
 * characters of a merchant endpoint's own response body, which the procedure
 * deliberately does not carry. It belongs to a detail read, which does not
 * exist yet.
 */
export const dynamic = "force-dynamic";

export default async function DeliveriesPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen
      {...DELIVERIES_SCREEN}
      resource={WEBHOOK_DELIVERIES}
      basePath="/deliveries"
      params={await searchParams}
      List={DeliveriesList}
      emptyMessage="No deliveries. Nothing has been delivered for this merchant yet."
    />
  );
}
