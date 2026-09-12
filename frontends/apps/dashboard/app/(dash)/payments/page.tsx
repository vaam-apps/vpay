import { ScreenStack } from "@vaam-apps/ui";
import { Suspense } from "react";

import { AppShell } from "../../../src/components/app-shell";
import { ReadFailure } from "../../../src/components/read-failure";
import { refineCursor, type InitialList } from "../../../src/dash/initial";
import { dashProvider, PAYMENT_INTENTS } from "../../../src/dash/provider";
import { queryFrom } from "../../../src/payments-query";
import { signOut } from "../../../src/server/actions";
import { requireStaff } from "../../../src/server/session";
import { PaymentsScreen } from "./payments-screen";

/**
 * `/payments` — the server shell, the gate, **and the read**.
 *
 * # The gate did not move to the browser, and neither did the read
 *
 * This is still a Server Component and it still calls `requireStaff()`
 * before anything renders. The 80 %-of-TTL re-mint, the
 * `X-Vpay-Staff-Session` header and the single-`401` retry all still run
 * here, on the server, where the token is.
 *
 * The page of payments is read here too, through the same
 * `src/dash/provider.ts` seam this page has always used, and handed to
 * `PaymentsScreen` as Refine's `initialData`. Lane 3 briefly had the browser
 * make that read through `/api/dash`; `src/dash/initial.ts` carries the two
 * things `dashboard.cy.ts` measured about it. Refine still owns the data
 * layer — the cache, the paging, the hooks — it is simply handed the answer
 * instead of being sent to fetch one, which is §2.6's SSR shape.
 *
 * `/api/dash` is untouched and still serves the browser: the detail screen's
 * and this screen's *subsequent* reads go through it, and it remains the
 * surface `bff.test.ts` and the Cypress BFF block gate.
 *
 * # A vpay this page cannot reach is not a sign-out
 *
 * `requireStaff` answers `outage` for anything but a `401`, and this renders
 * the failure with its request id and **keeps the cookie** (issue #88
 * item 2). Before 2026-09-10 a restarting vpay sent every staff member to
 * the sign-in form, indistinguishably from having been signed out on
 * purpose. A refused *list* read is the same rule one layer in: it is passed
 * to the screen as a refusal to render, never as an empty list and never as
 * a browser retry.
 *
 * # The `<Suspense>` is required, not decorative
 *
 * Refine's hooks read `useSearchParams()` below this point, which Next
 * requires a boundary around or the route opts out of static generation with
 * a build error. The same constraint vsms records for `nuqs`. The screen
 * itself no longer reads it — `payments-screen.tsx`'s `query` prop says why
 * that had to stop — but the boundary is still the hooks'.
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
      <ScreenStack>
        <h2>Payments</h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }
  const staff = gate.staff;
  const { session } = staff;

  const query = queryFrom(await searchParams);
  const result = await dashProvider(staff).getList({
    resource: PAYMENT_INTENTS,
    query,
  });
  // Shaped exactly as `dashDataProvider.getList` resolves, because that is
  // what the hook's cache holds for this key — a second shape here would be
  // a first render that disagreed with every render after it.
  const initial: InitialList = result.ok
    ? {
        ok: true,
        value: {
          // Copied because `GetListResponse.data` is mutable and the seam
          // answers a `readonly` array; the copy is the cache's own.
          data: [...result.value.data],
          total: 0,
          cursor: refineCursor(result.value.cursor),
        },
      }
    : { ok: false, failure: result.failure };

  return (
    <AppShell
      email={session.email}
      merchantId={session.merchant_id}
      signOut={signOut}
    >
      <Suspense fallback={null}>
        <PaymentsScreen query={query} initial={initial} />
      </Suspense>
    </AppShell>
  );
}
