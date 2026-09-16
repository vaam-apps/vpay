import { InlineEmptyState, ScreenStack } from "@vaam-apps/ui";

import { AppShell } from "../../../src/components/app-shell";
import { ReadFailure } from "../../../src/components/read-failure";
import {
  IdCell,
  ProcedureTable,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { ProcedurePager } from "../../../src/components/procedure-pager";
import { CUSTOMERS } from "../../../src/dash/resource-name";
import {
  formatIsoInstant,
  offsetFrom,
  readProcedurePage,
} from "../../../src/dash/procedure-list";
import { ABSENT } from "../../../src/format";
import { signOut } from "../../../src/server/actions";
import { requireStaff } from "../../../src/server/session";

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
  { key: "name", header: "Name", render: (r) => r.name ?? ABSENT },
  { key: "email", header: "Email", render: (r) => r.email ?? ABSENT },
  {
    key: "phone",
    secondary: true,
    header: "Phone",
    render: (r) => r.phone ?? ABSENT,
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
    header: "Last used",
    render: (r) => formatIsoInstant(r.last_used_at),
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created",
    render: (r) => formatIsoInstant(r.created_at),
  },
];

export default async function CustomersPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <ScreenStack>
        <h2>Customers</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const staff = gate.staff;
  const offset = offsetFrom(await searchParams);
  const result = await readProcedurePage<CustomerRow>(staff, CUSTOMERS, offset);

  return (
    <AppShell
      email={staff.session.email}
      merchantId={staff.session.merchant_id}
      signOut={signOut}
    >
      <ScreenStack>
        <h2>Customers</h2>
        {!result.ok ? (
          <ReadFailure failure={result.failure} />
        ) : result.value.items.length === 0 ? (
          <InlineEmptyState
            variant="standalone"
            message="No customers. This merchant has no customers yet."
          />
        ) : (
          <>
            <ProcedureTable
              rows={result.value.items}
              columns={COLUMNS}
              idOf={(r) => r.id}
              caption="Customers"
            />
            <ProcedurePager
              basePath="/customers"
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
