"use client";

import { useOne } from "@refinedev/core";
import { RouteSkeleton, ScreenStack } from "@vaam-apps/ui";
import NextLink from "next/link";
import { notFound } from "next/navigation";

import { PaymentDetailView } from "../../../../src/components/payment-detail";
import { ReadFailure } from "../../../../src/components/read-failure";
import { failureFromError } from "../../../../src/dash/failure";
import {
  initialFailure,
  initialQueryOptions,
  type InitialOne,
} from "../../../../src/dash/initial";
import { PAYMENT_INTENTS } from "../../../../src/dash/resource-name";
import type { PaymentDetail } from "../../../../src/server/api";

export interface PaymentScreenProps {
  /** The `pi_…` id, unencoded. */
  readonly id: string;
  /**
   * The payment the Server Component read on this request, or the refusal it
   * met. Optional for the reason `PaymentsScreen`'s is.
   */
  readonly initial?: InitialOne;
}

/**
 * One payment, through Refine.
 *
 * # A `404` is still `notFound()`, and still says nothing
 *
 * `/dash/v1` answers a byte-identical `404` for an id belonging to another
 * merchant and for an id that does not exist — that is the cross-tenant
 * guarantee `dashboard_read_surface.rs` pins, and it is only a guarantee if
 * **this** side keeps it too. So the 404 branch renders Next's own
 * `notFound()` and the error is never echoed back with the id in it: an
 * error mapper that spelled the two cases differently would leak
 * existence through the dashboard after the API refused to.
 *
 * Every other status is an outage and renders `ReadFailure` with the
 * session intact — `authProvider.onError` signs nobody out below `401`.
 *
 * # The payment is the SERVER's read, not this hook's
 *
 * `initial` is what `page.tsx` read on this request. It is handed to the
 * hook as `initialData` rather than fetched again from the browser, so the
 * detail is in the document at the `load` event rather than a skeleton that
 * fills in later — which is the difference between
 * `dashboard.cy.ts`'s masked-payer leg finding a charge and reporting that
 * this merchant has none. See `src/dash/initial.ts`.
 */
export function PaymentScreen({ id, initial }: PaymentScreenProps) {
  const serverFailure = initialFailure(initial);
  const { result, query } = useOne<PaymentDetail>({
    resource: PAYMENT_INTENTS,
    id,
    queryOptions: initialQueryOptions(initial),
  });

  if (serverFailure !== null) {
    // A `404` never reaches here: `page.tsx` answers it with `notFound()` on
    // the server, which is what keeps the two 404 bodies indistinguishable.
    return (
      <ScreenStack>
        <h2>Payment</h2>
        <ReadFailure failure={serverFailure} />
      </ScreenStack>
    );
  }

  if (query.isLoading) {
    return <RouteSkeleton rows={10} />;
  }

  if (query.isError) {
    const failure = failureFromError(query.error);
    if (failure.status === 404) {
      notFound();
    }
    return (
      <ScreenStack>
        <h2>Payment</h2>
        <ReadFailure failure={failure} />
      </ScreenStack>
    );
  }

  const detail = result;
  if (detail === undefined) {
    notFound();
  }

  return (
    <ScreenStack>
      {/* A plain <h2> — see `payments-screen.tsx` for why, and the 13
          Cypress assertions that pin it. */}
      <header>
        <h2>Payment</h2>
        <NextLink href="/payments">Back to payments</NextLink>
      </header>
      <PaymentDetailView detail={detail} />
    </ScreenStack>
  );
}
