import { InlineEmptyState, ScreenStack } from "@vaam-apps/ui";

import { AppShell } from "../../../src/components/app-shell";
import { ReadFailure } from "../../../src/components/read-failure";
import {
  IdCell,
  ProcedureTable,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { ProcedurePager } from "../../../src/components/procedure-pager";
import { WEBHOOK_DELIVERIES } from "../../../src/dash/resource-name";
import {
  formatIsoInstant,
  offsetFrom,
  readProcedurePage,
} from "../../../src/dash/procedure-list";
import { signOut } from "../../../src/server/actions";
import { requireStaff } from "../../../src/server/session";

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
    render: (r) => r.event_type,
  },
  {
    key: "event_id",
    secondary: true,
    header: "Event id",
    render: (r) => <IdCell value={r.event_id} />,
  },
  { key: "state", header: "State", render: (r) => r.state },
  { key: "url", secondary: true, header: "Endpoint", render: (r) => r.url },
  {
    key: "created_at",
    secondary: true,
    header: "Created",
    render: (r) => formatIsoInstant(r.created_at),
  },
  {
    key: "sent_at",
    secondary: true,
    header: "Sent",
    render: (r) => formatIsoInstant(r.sent_at),
  },
  {
    key: "next_attempt_at",
    secondary: true,
    header: "Next attempt",
    render: (r) => formatIsoInstant(r.next_attempt_at),
  },
];

export default async function DeliveriesPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <ScreenStack>
        <h2>Deliveries</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const staff = gate.staff;
  const offset = offsetFrom(await searchParams);
  const result = await readProcedurePage<DeliveryRow>(
    staff,
    WEBHOOK_DELIVERIES,
    offset,
  );

  return (
    <AppShell
      email={staff.session.email}
      merchantId={staff.session.merchant_id}
      signOut={signOut}
    >
      <ScreenStack>
        <h2>Deliveries</h2>
        {!result.ok ? (
          <ReadFailure failure={result.failure} />
        ) : result.value.items.length === 0 ? (
          <InlineEmptyState
            variant="standalone"
            message="No deliveries. Nothing has been delivered for this merchant yet."
          />
        ) : (
          <>
            <ProcedureTable
              rows={result.value.items}
              columns={COLUMNS}
              idOf={(r) => r.id}
              caption="Webhook deliveries"
            />
            <ProcedurePager
              basePath="/deliveries"
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
