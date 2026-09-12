import { ScreenStack } from "@vaam-apps/ui";
import { Suspense } from "react";

import { AppShell } from "../../../src/components/app-shell";
import { ReadFailure } from "../../../src/components/read-failure";
import { signOut } from "../../../src/server/actions";
import { requireStaff } from "../../../src/server/session";
import { PaymentsScreen } from "./payments-screen";

/**
 * `/payments` — the server shell, and the gate.
 *
 * # The gate did not move to the browser, and that is the point
 *
 * This is still a Server Component and it still calls `requireStaff()`
 * before anything renders. The 80 %-of-TTL re-mint, the
 * `X-Vpay-Staff-Session` header and the single-`401` retry all still run
 * here, on the server, where the token is. Refine's hooks below fetch
 * through `/api/dash`, which re-does the same gate per request — so the
 * credential is enforced twice on the server and never once in a browser.
 *
 * # A vpay this page cannot reach is not a sign-out
 *
 * `requireStaff` answers `outage` for anything but a `401`, and this renders
 * the failure with its request id and **keeps the cookie** (issue #88
 * item 2). Before 2026-09-10 a restarting vpay sent every staff member to
 * the sign-in form, indistinguishably from having been signed out on
 * purpose. The client screen applies the same rule to its own reads through
 * `authProvider.onError`.
 *
 * # The `<Suspense>` is required, not decorative
 *
 * `PaymentsScreen` reads `useSearchParams()` — directly, and again inside
 * Refine's hooks — which Next requires a boundary around or the route opts
 * out of static generation with a build error. The same constraint vsms
 * records for `nuqs`.
 */
export const dynamic = "force-dynamic";

export default async function PaymentsPage() {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <ScreenStack>
        <h2>Payments</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const { session } = gate.staff;

  return (
    <AppShell
      email={session.email}
      merchantId={session.merchant_id}
      signOut={signOut}
    >
      <Suspense fallback={null}>
        <PaymentsScreen />
      </Suspense>
    </AppShell>
  );
}
