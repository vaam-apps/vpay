"use client";

import { useOne } from "@refinedev/core";
import { RouteSkeleton, ScreenStack } from "@vaam-apps/ui";
import NextLink from "next/link";
import { notFound } from "next/navigation";

import { PaymentDetailView } from "../../../../src/components/payment-detail";
import { ReadFailure } from "../../../../src/components/read-failure";
import { failureFromError } from "../../../../src/dash/failure";
import { PAYMENT_INTENTS } from "../../../../src/dash/resource-name";
import type { PaymentDetail } from "../../../../src/server/api";

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
 */
export function PaymentScreen({ id }: { id: string }) {
  const { result, query } = useOne<PaymentDetail>({
    resource: PAYMENT_INTENTS,
    id,
  });

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
