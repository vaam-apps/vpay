import { CUSTOMERS_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import {
  IdCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { IsoInstant } from "../../../src/components/values";
import { CUSTOMERS } from "../../../src/dash/resource-name";
import { ABSENT } from "../../../src/format";

/** Exactly the fields `type CustomerSummary` declares. */
interface CustomerRow {
  readonly id: string;
  readonly livemode: boolean;
  readonly name: string | null;
  readonly email: string | null;
  readonly phone: string | null;
  readonly last_used_at: string;
  readonly created_at: string;
}

/**
 * `/customers` — this merchant's customers.
 *
 * # The identifiers are UNMASKED, and that is a decision rather than an
 * oversight
 *
 * `name`, `email` and `phone` are rendered in full. They carry `@sensitive`
 * in `schemas/vpay.cstack`, and **that attribute does not protect them**:
 * `cratestack-macros`' `is_sensitive_field` is read in exactly one place, to
 * redact a value from the *audit log*. It excludes nothing from a response
 * and never has. The maintainer's call (2026-09-13) is that an operator who
 * already sees a payer's details on the payment detail screen is not
 * protected by hiding them here. Anyone reading `@sensitive` and inferring
 * masking is reading it wrong, which is why this paragraph exists.
 *
 * # Erased customers do not appear
 *
 * The procedure's `WHERE` carries `anonymized_at IS NULL`, unconditionally.
 * Erasure (issue #111, closed in #143) already answered for those records
 * and this surface does not re-expose them. Its container test removes that
 * clause to prove the exclusion fires.
 *
 * Note this differs from `GET /v1/customers`, the merchant-facing list,
 * which does **not** exclude anonymised rows — an asymmetry worth knowing
 * about rather than discovering.
 *
 * # No address is shown
 *
 * The six `address_*` columns and the two coordinate columns are not on the
 * summary type at all. A list is not where a payer's postal address or GPS
 * point is read.
 */
export const dynamic = "force-dynamic";

const COLUMNS: readonly ProcedureColumn<CustomerRow>[] = [
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

export default async function CustomersPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen<CustomerRow>
      {...CUSTOMERS_SCREEN}
      resource={CUSTOMERS}
      basePath="/customers"
      params={await searchParams}
      columns={COLUMNS}
      idOf={(r) => r.id}
      emptyMessage="No customers. This merchant has no customers yet."
    />
  );
}
