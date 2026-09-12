"use client";

import { useList } from "@refinedev/core";
import { InlineEmptyState, RouteSkeleton, ScreenStack } from "@vaam-apps/ui";

import { PaymentsFilters } from "../../../src/components/payments-filters";
import { PaymentsPager } from "../../../src/components/payments-pager";
import { PaymentsTable } from "../../../src/components/payments-table";
import { ReadFailure } from "../../../src/components/read-failure";
import { PAYMENT_INTENTS } from "../../../src/dash/resource-name";
import { failureFromError } from "../../../src/dash/failure";
import {
  initialFailure,
  initialQueryOptions,
  type InitialList,
} from "../../../src/dash/initial";
import {
  pagerHrefsFromCursors,
  type PaymentsQuery,
} from "../../../src/payments-query";
import type { PaymentIntentObject } from "../../../src/server/api";

export interface PaymentsScreenProps {
  /**
   * The filters and cursor this page was read with — `queryFrom(searchParams)`
   * as the Server Component parsed them.
   *
   * **Not `useSearchParams()`, and that is load-bearing rather than a
   * preference.** Next updates the URL *optimistically* during a `<Link>`
   * navigation, so a hook reading it can see the next page's cursor one
   * render before the next page's rows arrive. The query key would move
   * first, and this screen would hand Refine the PREVIOUS page's rows as the
   * new key's `initialData` — rows that, with no re-fetch to correct them,
   * would then stand as the answer for that cursor. The repair is that the
   * query and the rows arrive together, out of the one render that produced
   * both. The URL is still where the query comes from; the server is simply
   * the one thing reading it.
   */
  readonly query: PaymentsQuery;
  /**
   * The page the Server Component read on this request, or the refusal it
   * met. Optional so the component tests can mount this screen against a
   * stub provider with nothing handed in — which is the only way to reach
   * the loading and client-error branches below.
   */
  readonly initial?: InitialList;
}

/**
 * The payments list, through Refine.
 *
 * # Why the URL is still the source of truth
 *
 * The filters and the cursor live in the query string, exactly as they did
 * when this was a Server Component, and this screen renders them rather than
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
 *
 * # The page is the SERVER's read, not this hook's
 *
 * `initial` is the page `app/(dash)/payments/page.tsx` read on this request,
 * with the token that never leaves that process. It is handed to the hook as
 * `initialData` rather than fetched again from the browser, which is what
 * puts the rows in the very first paint and what stops this screen issuing a
 * `/api/dash` request for data it was already given. `src/dash/initial.ts`
 * carries the two measured defects that shape is the repair for. Paging and
 * filtering are ordinary navigations, and the server reads those too.
 */
export function PaymentsScreen({ query, initial }: PaymentsScreenProps) {
  const { status, createdFrom, createdTo, after, before } = query;

  const serverFailure = initialFailure(initial);
  const { result, query: listQuery } = useList<PaymentIntentObject>({
    resource: PAYMENT_INTENTS,
    filters: [
      { field: "status", operator: "eq", value: status },
      { field: "created_from", operator: "gte", value: createdFrom },
      { field: "created_to", operator: "lte", value: createdTo },
    ],
    meta: { after, before },
    pagination: { mode: "off" },
    queryOptions: initialQueryOptions(initial),
  });

  if (serverFailure !== null) {
    // The server met this refusal on this request and the cookie is
    // untouched. It is rendered rather than re-read from the browser — see
    // `initial.ts`: a `401` re-read through `/api/dash` reaches
    // `authProvider.onError`, which signs the person out of a session the
    // server had just decided to keep.
    return (
      <ScreenStack>
        <h2>Payments</h2>
        <PaymentsFilters values={{ status, createdFrom, createdTo }} />
        <ReadFailure failure={serverFailure} />
      </ScreenStack>
    );
  }

  if (listQuery.isLoading) {
    return <RouteSkeleton rows={8} withFilterBar />;
  }

  if (listQuery.isError) {
    // An outage renders here and the session is untouched — `onError` only
    // signs anyone out on a 401. See `auth-provider.ts`.
    return (
      <ScreenStack>
        <h2>Payments</h2>
        <PaymentsFilters values={{ status, createdFrom, createdTo }} />
        <ReadFailure failure={failureFromError(listQuery.error)} />
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
            {...pagerHrefsFromCursors(query, {
              newer: cursor?.prev ?? null,
              older: cursor?.next ?? null,
            })}
          />
        </>
      )}
    </ScreenStack>
  );
}
