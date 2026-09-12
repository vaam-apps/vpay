import { ScreenStack } from "@vaam-apps/ui";
import { Suspense } from "react";

import { AppShell } from "../../../../src/components/app-shell";
import { ReadFailure } from "../../../../src/components/read-failure";
import { signOut } from "../../../../src/server/actions";
import { requireStaff } from "../../../../src/server/session";
import { PaymentScreen } from "./payment-screen";

/**
 * `/payments/{id}` — the server shell, and the gate.
 *
 * Same shape as the list: `requireStaff()` runs here, on the server, before
 * anything renders, and the screen below reads through `/api/dash`, which
 * re-does the gate per request. The token is enforced twice server-side and
 * never once in a browser.
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
  const { session } = gate.staff;
  const { id } = await params;

  return (
    <AppShell
      email={session.email}
      merchantId={session.merchant_id}
      signOut={signOut}
    >
      <Suspense fallback={null}>
        <PaymentScreen id={id} />
      </Suspense>
    </AppShell>
  );
}
