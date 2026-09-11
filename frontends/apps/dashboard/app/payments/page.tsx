import { EmptyState, Heading, Stack } from "@vpay/ui";

import { PaymentsPager } from "../../src/components/payments-pager";
import { ReadFailure } from "../../src/components/read-failure";
import { PaymentsFilters } from "../../src/components/payments-filters";
import { PaymentsTable } from "../../src/components/payments-table";
import { SignedInBar } from "../../src/components/signed-in-bar";
import { dashProvider, PAYMENT_INTENTS } from "../../src/dash/provider";
import { pagerHrefs, queryFrom } from "../../src/payments-query";
import { signOut } from "../../src/server/actions";
import { requireStaff } from "../../src/server/session";

/**
 * `/payments` — the merchant's payment intents, newest first.
 *
 * Server-rendered: the `/dash/v1` bearer token is read out of the
 * `staff_sessions` row on this request and used on this request, and no
 * browser ever holds it (ADR-0008, ADR-0017 decision 4).
 *
 * **The token is re-read on every render**, which is the whole mechanism
 * behind sign-out: deleting the session row removes the only place this
 * server can obtain it from. A token cached in this process would keep
 * rendering payments for a signed-out session until it expired.
 *
 * The read goes through `src/dash/provider.ts` — the seam, which is where the
 * path, the query parameters and the meaning of a page of results live now
 * that more than this page wants them. It calls `readDash` with the same two
 * tokens this page used to pass itself; nothing about when a token is minted
 * or where it is read from has moved.
 *
 * `readDash` mints a fresh access token once if
 * the one on the row has expired — the token's TTL is fifteen minutes and a
 * session's is up to twelve hours, so without that this page answers "the
 * bearer token is invalid, expired" for the rest of every sign-in past the
 * first quarter of an hour. See that module.
 *
 * # An empty table means zero rows and nothing else
 *
 * A failed read renders the failure and the request id, never an empty list:
 * "this merchant has no payments" and "the read was refused" are different
 * answers, and the empty list is the one an operator would believe.
 *
 * # A vpay this page cannot reach is not a sign-out
 *
 * `requireStaff` answers `outage` for anything but a `401` on the session
 * read, and this page renders the failure with its request id and **keeps the
 * cookie** (issue #88 item 2). Before 2026-09-10 a restarting vpay sent every
 * staff member to the sign-in form, indistinguishably from having been signed
 * out on purpose.
 */
export const dynamic = "force-dynamic";

export default async function PaymentsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <Stack direction="column" gap="lg">
        <Heading level={2}>Payments</Heading>
        <ReadFailure failure={gate.failure} />
      </Stack>
    );
  }
  const staff = gate.staff;
  const { session } = staff;
  const query = queryFrom(await searchParams);

  const result = await dashProvider(staff).getList({
    resource: PAYMENT_INTENTS,
    query,
  });

  return (
    <Stack direction="column" gap="lg">
      <SignedInBar
        email={session.email}
        merchantId={session.merchant_id}
        signOut={signOut}
      />

      <Heading level={2}>Payments</Heading>

      <PaymentsFilters
        values={{
          status: query.status,
          createdFrom: query.createdFrom,
          createdTo: query.createdTo,
        }}
      />

      {!result.ok ? (
        <ReadFailure failure={result.failure} />
      ) : result.value.data.length === 0 ? (
        <EmptyState
          title="No payments"
          description={
            query.status.length > 0 ||
            query.createdFrom.length > 0 ||
            query.createdTo.length > 0
              ? "No payment matched these filters. Widen the range or clear the status."
              : "This merchant has no payment intents yet."
          }
        />
      ) : (
        <>
          <PaymentsTable rows={result.value.data} />
          <PaymentsPager
            {...pagerHrefs(query, result.value.data, result.value.hasMore)}
          />
        </>
      )}
    </Stack>
  );
}
