import { CHECKOUTS_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import {
  IdCell,
  PaymentLinkCell,
  type ProcedureColumn,
} from "../../../src/components/procedure-table";
import { IsoInstant } from "../../../src/components/values";
import { CHECKOUT_SESSIONS } from "../../../src/dash/resource-name";
import { ABSENT } from "../../../src/format";
import {
  asCheckoutStatus,
  CheckoutStatusPill,
} from "../../../src/status-systems";

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
  {
    key: "status",
    header: "Status",
    render: (r) => {
      const status = asCheckoutStatus(r.status);
      return status === null ? (
        r.status
      ) : (
        <CheckoutStatusPill state={status} size="sm" />
      );
    },
  },
  { key: "payment_status", header: "Payment", render: (r) => r.payment_status },
  {
    key: "payment_intent_id",
    secondary: true,
    header: "Intent",
    render: (r) => <PaymentLinkCell id={r.payment_intent_id} compact />,
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
    render: (r) =>
      r.customer_id === null ? ABSENT : <IdCell value={r.customer_id} />,
  },
  {
    key: "expires_at",
    secondary: true,
    header: "Expires (UTC)",
    render: (r) => <IsoInstant value={r.expires_at} />,
  },
  {
    key: "created_at",
    secondary: true,
    header: "Created (UTC)",
    render: (r) => <IsoInstant value={r.created_at} />,
  },
];

export default async function CheckoutsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen<CheckoutRow>
      {...CHECKOUTS_SCREEN}
      resource={CHECKOUT_SESSIONS}
      basePath="/checkouts"
      params={await searchParams}
      columns={COLUMNS}
      idOf={(r) => r.id}
      emptyMessage="No checkouts. This merchant has no checkout sessions yet."
    />
  );
}
