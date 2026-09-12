import { ScreenStack } from "@vaam-apps/ui";
import { notFound } from "next/navigation";
import { Suspense } from "react";

import { AppShell } from "../../../../src/components/app-shell";
import { ReadFailure } from "../../../../src/components/read-failure";
import type { InitialOne } from "../../../../src/dash/initial";
import { dashProvider, PAYMENT_INTENTS } from "../../../../src/dash/provider";
import { signOut } from "../../../../src/server/actions";
import { requireStaff } from "../../../../src/server/session";
import { PaymentScreen } from "./payment-screen";

/**
 * `/payments/{id}` — the server shell, the gate, and the read.
 *
 * Same shape as the list: `requireStaff()` runs here, on the server, before
 * anything renders, and the payment itself is read here too and handed to
 * `PaymentScreen` as Refine's `initialData`. The token is enforced and used
 * on the server and never once in a browser.
 *
 * A `404` from vpay is this app's `notFound()`, and that mapping is exact
 * rather than convenient: the detail read is tenant-scoped, so **another
 * merchant's id answers the same `404` a nonexistent one does**
 * (`vpay_api::dash::payment_intents::retrieve`). Rendering "you may not see
 * this" for one and "no such payment" for the other would turn this page
 * into an oracle for which ids exist in other tenants. It is answered here,
 * before anything is handed to the client screen, so the two bodies stay
 * indistinguishable whatever the browser does next.
 */
export const dynamic = "force-dynamic";

export default async function PaymentDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    // A vpay this page cannot reach is not a sign-out — see `payments/page.tsx`.
    return (
      <ScreenStack>
        <h2>Payment</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const staff = gate.staff;
  const { session } = staff;
  const { id } = await params;

  const result = await dashProvider(staff).getOne({
    resource: PAYMENT_INTENTS,
    id,
  });
  if (!result.ok && result.failure.status === 404) {
    notFound();
  }
  const initial: InitialOne = result.ok
    ? { ok: true, value: { data: result.value } }
    : { ok: false, failure: result.failure };

  return (
    <AppShell
      email={session.email}
      merchantId={session.merchant_id}
      signOut={signOut}
    >
      <Suspense fallback={null}>
        <PaymentScreen id={id} initial={initial} />
      </Suspense>
    </AppShell>
  );
}
