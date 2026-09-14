import { InlineEmptyState, ScreenStack } from "@vaam-apps/ui";

import { AppShell } from "../../../src/components/app-shell";
import { ReadFailure } from "../../../src/components/read-failure";
import {
  IdCell,
  ProcedureTable,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { ProcedurePager } from "../../../src/components/procedure-pager";
import { CHECKOUT_SESSIONS } from "../../../src/dash/resource-name";
import {
  formatIsoInstant,
  offsetFrom,
  readProcedurePage,
} from "../../../src/dash/procedure-list";
import { ABSENT } from "../../../src/format";
import { signOut } from "../../../src/server/actions";
import { requireStaff } from "../../../src/server/session";

/** Exactly the fields `type CheckoutSessionSummary` declares. */
interface CheckoutRow {
  readonly id: string;
  readonly payment_intent_id: string;
  readonly livemode: boolean;
  readonly ui_mode: string;
  readonly status: string;
  readonly payment_status: string;
  readonly customer_id: string | null;
  readonly expires_at: string;
  readonly created_at: string;
}

/**
 * `/checkouts` — what happened to this merchant's checkouts.
 *
 * # The two statuses are the point of the screen
 *
 * `status` and `payment_status` are separate columns and both are primary,
 * because the interesting row is the one where they disagree: a session
 * `complete` whose payment is `unpaid` is a checkout the payer finished and
 * the rail did not. One combined column would hide exactly the case an
 * operator opens this screen to find.
 *
 * # Two payer credentials are absent, and not by omission
 *
 * `client_secret_suffix` and `return_token` are not on the procedure's
 * summary type at all — the client secret authorises the payer's browser
 * against the checkout and the return token authorises the return leg, so a
 * list carrying either hands them to everyone who can read the list.
 * `search_checkout_sessions.rs` refuses them in the statement, the row
 * struct and the summary, and a container test serialises the answer to
 * prove it.
 *
 * The three URLs are absent too: merchant-supplied, up to 2 048 characters
 * each, and nothing a list is read for.
 */
export const dynamic = "force-dynamic";

const COLUMNS: readonly ProcedureColumn<CheckoutRow>[] = [
  { key: "id", header: "Checkout", render: (r) => <IdCell value={r.id} /> },
  { key: "status", header: "Status", render: (r) => r.status },
  { key: "payment_status", header: "Payment", render: (r) => r.payment_status },
  {
    key: "payment_intent_id",
    secondary: true,
    header: "Intent",
    render: (r) => <IdCell value={r.payment_intent_id} />,
  },
  {
    key: "ui_mode",
    secondary: true,
    header: "Mode",
    render: (r) => r.ui_mode,
  },
  {
    key: "customer_id",
    secondary: true,
    header: "Customer",
    render: (r) => (r.customer_id === null ? ABSENT : r.customer_id),
  },
  {
    key: "expires_at",
    secondary: true,
    header: "Expires",
    render: (r) => formatIsoInstant(r.expires_at),
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created",
    render: (r) => formatIsoInstant(r.created_at),
  },
];

export default async function CheckoutsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <ScreenStack>
        <h2>Checkouts</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const staff = gate.staff;
  const offset = offsetFrom(await searchParams);
  const result = await readProcedurePage<CheckoutRow>(
    staff,
    CHECKOUT_SESSIONS,
    offset,
  );

  return (
    <AppShell
      email={staff.session.email}
      merchantId={staff.session.merchant_id}
      signOut={signOut}
    >
      <ScreenStack>
        <h2>Checkouts</h2>
        {!result.ok ? (
          <ReadFailure failure={result.failure} />
        ) : result.value.items.length === 0 ? (
          <InlineEmptyState
            variant="standalone"
            message="No checkouts. This merchant has no checkout sessions yet."
          />
        ) : (
          <>
            <ProcedureTable
              rows={result.value.items}
              columns={COLUMNS}
              idOf={(r) => r.id}
              caption="Checkout sessions"
            />
            <ProcedurePager
              basePath="/checkouts"
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
