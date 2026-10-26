"use client";

import type { CustomerSummary } from "@vpay/dash-client";

import {
  IdCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import {
  ProcedureListView,
  type ProcedureListBodyProps,
} from "../../../src/components/procedure-list-view";
import { IsoInstant } from "../../../src/components/values";
import { ABSENT } from "../../../src/format";

/**
 * The customers list's body: its columns, over the **generated**
 * `CustomerSummary` — the one TypeScript copy of that wire shape, so a field
 * the schema drops is a compile error here after `just gen-dash-client`.
 * Until 2026-10-26 this was a hand-written `CustomerRow` in `page.tsx`.
 *
 * A client module because the columns carry render functions, which cannot
 * cross from the Server Component as props.
 */
const COLUMNS: readonly ProcedureColumn<CustomerSummary>[] = [
  {
    key: "id",
    secondary: true,
    header: "Customer",
    render: (r) => <IdCell value={r.id} />,
  },
  {
    key: "name",
    header: "Name",
    render: (r) => (
      <span className="font-medium text-foreground">{r.name ?? ABSENT}</span>
    ),
  },
  { key: "email", header: "Email", render: (r) => r.email ?? ABSENT },
  {
    key: "phone",
    secondary: true,
    header: "Phone",
    render: (r) => <span className="font-mono">{r.phone ?? ABSENT}</span>,
  },
  {
    key: "livemode",
    secondary: true,
    header: "Mode",
    render: (r) => (r.livemode ? "live" : "test"),
  },
  {
    key: "last_used_at",
    secondary: true,
    header: "Last used (UTC)",
    render: (r) => <IsoInstant value={r.last_used_at} />,
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created (UTC)",
    render: (r) => <IsoInstant value={r.created_at} />,
  },
];

export function CustomersList(props: ProcedureListBodyProps) {
  return (
    <ProcedureListView<CustomerSummary>
      {...props}
      columns={COLUMNS}
      idOf={(r) => r.id}
    />
  );
}
