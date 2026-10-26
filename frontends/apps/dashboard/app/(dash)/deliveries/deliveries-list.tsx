"use client";

import type { WebhookDeliverySummary } from "@vpay/dash-client";

import {
  IdCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import {
  ProcedureListView,
  type ProcedureListBodyProps,
} from "../../../src/components/procedure-list-view";
import { IsoInstant } from "../../../src/components/values";
import {
  asDeliveryState,
  DeliveryStatePill,
} from "../../../src/status-systems";

/**
 * The deliveries list's body: its columns, over the **generated**
 * `WebhookDeliverySummary` — the one TypeScript copy of that wire shape, so a field
 * the schema drops is a compile error here after `just gen-dash-client`.
 * Until 2026-10-26 this was a hand-written `DeliveryRow` in `page.tsx`.
 *
 * A client module because the columns carry render functions, which cannot
 * cross from the Server Component as props.
 */
const COLUMNS: readonly ProcedureColumn<WebhookDeliverySummary>[] = [
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

export function DeliveriesList(props: ProcedureListBodyProps) {
  return (
    <ProcedureListView<WebhookDeliverySummary>
      {...props}
      columns={COLUMNS}
      idOf={(r) => r.id}
    />
  );
}
