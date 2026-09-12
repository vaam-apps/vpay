"use client";

import { useList } from "@refinedev/core";
import { InlineEmptyState, RouteSkeleton, ScreenStack } from "@vaam-apps/ui";
import { useSearchParams } from "next/navigation";

import { PaymentsFilters } from "../../../src/components/payments-filters";
import { PaymentsPager } from "../../../src/components/payments-pager";
import { PaymentsTable } from "../../../src/components/payments-table";
import { ReadFailure } from "../../../src/components/read-failure";
import { PAYMENT_INTENTS } from "../../../src/dash/resource-name";
import { failureFromError } from "../../../src/dash/failure";
import { pagerHrefsFromCursors } from "../../../src/payments-query";
import type { PaymentIntentObject } from "../../../src/server/api";

/**
 * The payments list, through Refine.
 *
 * # Why the URL is still the source of truth
 *
 * The filters and the cursor live in the query string, exactly as they did
 * when this was a Server Component, and this screen reads them rather than
 * holding its own state. That is not inertia: a filtered list an operator
 * cannot send to a colleague is a worse tool, and every Cypress leg asserts
 * on a URL.
 *
 * # Paging is cursor-based and stays that way
 *
 * `/dash/v1` is cursor-paged and reports no total, so there is no page count
 * to render and `Pagination` is driven by the two cursors the provider
 * returns. Refine's `useTable` offers offset paging over a `total`; using it
 * here would mean inventing a count, and RD3 (infinite scroll) is a product
 * decision nobody has taken.
 */
export function PaymentsScreen() {
  const params = useSearchParams();

  const status = params.get("status") ?? "";
  const createdFrom = params.get("created_from") ?? "";
  const createdTo = params.get("created_to") ?? "";
  const after = params.get("after") ?? "";
  const before = params.get("before") ?? "";

  const { result, query } = useList<PaymentIntentObject>({
    resource: PAYMENT_INTENTS,
    filters: [
      { field: "status", operator: "eq", value: status },
      { field: "created_from", operator: "gte", value: createdFrom },
      { field: "created_to", operator: "lte", value: createdTo },
    ],
    meta: { after, before },
    pagination: { mode: "off" },
  });

  if (query.isLoading) {
    return <RouteSkeleton rows={8} withFilterBar />;
  }

  if (query.isError) {
    // An outage renders here and the session is untouched — `onError` only
    // signs anyone out on a 401. See `auth-provider.ts`.
    return (
      <ScreenStack>
        <h2>Payments</h2>
        <PaymentsFilters values={{ status, createdFrom, createdTo }} />
        <ReadFailure failure={failureFromError(query.error)} />
      </ScreenStack>
    );
  }

  const rows = result?.data ?? [];
  const cursor = (result as { cursor?: { next?: string; prev?: string } })
    ?.cursor;
  const filtered =
    status.length > 0 || createdFrom.length > 0 || createdTo.length > 0;

  return (
    <ScreenStack>
      {/*
        A plain <h2>, not `ScreenHeader` — that component renders an <h1>,
        and the app's <h1> is the brand in the rail. Thirteen assertions in
        `dashboard.cy.ts` pin `h2` + "Payments", and the existing hierarchy
        (one <h1> naming the app, an <h2> naming the section) is already
        tested and already passes axe. Changing a tested contract because a
        component prefers a different level is not a reason.
      */}
      <h2>Payments</h2>
      <p className="text-body text-muted-foreground">
        Every payment intent this merchant has created, newest first.
      </p>
      <PaymentsFilters values={{ status, createdFrom, createdTo }} />
      {rows.length === 0 ? (
        <InlineEmptyState
          variant="standalone"
          message={
            filtered
              ? "No payments. No payment matched these filters. Widen the range or clear the status."
              : "No payments. This merchant has no payment intents yet."
          }
        />
      ) : (
        <>
          <PaymentsTable rows={rows} />
          {/*
            `PaymentsPager`, not `@vaam-apps/ui`'s `Pagination`: that one
            renders callback-driven <button>s and turns an absent control
            into a DISABLED button rather than omitting it, and two Cypress
            legs assert on `<a href>`. Its own module doc records the
            decision; this screen keeps it.
          */}
          <PaymentsPager
            {...pagerHrefsFromCursors(
              { status, createdFrom, createdTo, after, before },
              { newer: cursor?.prev ?? null, older: cursor?.next ?? null },
            )}
          />
        </>
      )}
    </ScreenStack>
  );
}
