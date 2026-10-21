import { DELIVERIES_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import {
  IdCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { IsoInstant } from "../../../src/components/values";
import { WEBHOOK_DELIVERIES } from "../../../src/dash/resource-name";
import {
  asDeliveryState,
  DeliveryStatePill,
} from "../../../src/status-systems";

/** Exactly the fields `type WebhookDeliverySummary` declares. */
interface DeliveryRow {
  readonly id: string;
  readonly event_id: string;
  readonly event_type: string;
  readonly url: string;
  readonly state: string;
  readonly created_at: string;
  readonly sent_at: string | null;
  readonly responded_at: string | null;
  readonly next_attempt_at: string | null;
}

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

const COLUMNS: readonly ProcedureColumn<DeliveryRow>[] = [
  {
    key: "event_type",
    header: "Event",
    render: (r) => (
      <span className="font-mono text-body-small">{r.event_type}</span>
    ),
  },
  {
    key: "state",
    header: "State",
    render: (r) => {
      const state = asDeliveryState(r.state);
      return state === null ? (
        r.state
      ) : (
        <DeliveryStatePill state={state} size="sm" />
      );
    },
  },
  {
    key: "event_id",
    secondary: true,
    header: "Event id",
    render: (r) => <IdCell value={r.event_id} />,
  },
  {
    key: "url",
    secondary: true,
    header: "Endpoint",
    render: (r) => <span className="break-all">{r.url}</span>,
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created (UTC)",
    render: (r) => <IsoInstant value={r.created_at} />,
  },
  {
    key: "sent_at",
    secondary: true,
    header: "Sent",
    render: (r) => <IsoInstant value={r.sent_at} />,
  },
  {
    key: "next_attempt_at",
    secondary: true,
    header: "Next attempt",
    render: (r) => <IsoInstant value={r.next_attempt_at} />,
  },
];

export default async function DeliveriesPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen<DeliveryRow>
      {...DELIVERIES_SCREEN}
      resource={WEBHOOK_DELIVERIES}
      basePath="/deliveries"
      params={await searchParams}
      columns={COLUMNS}
      idOf={(r) => r.id}
      emptyMessage="No deliveries. Nothing has been delivered for this merchant yet."
    />
  );
}
