import { AppShell } from "../../src/components/app-shell";
import { DashProviders } from "../../src/dash/providers";
import { signOut } from "../../src/server/actions";
import { requireStaff } from "../../src/server/session";

/**
 * The signed-in layout: the gate, then the shell, then the page.
 *
 * # Why the shell lives here and not in every page
 *
 * Until 2026-10-21 each page rendered its own `AppShell`. A layout is the
 * only place Next keeps a subtree mounted across navigations, so with the
 * shell inside the page every navigation unmounted and remounted the rail,
 * the app bar and the account block — the floating rails, being portalled
 * after hydration, visibly disappeared and came back — and nothing on screen
 * said a navigation was in progress until the next page had fully arrived.
 * Here the shell stays put, and each route's `loading.tsx` fills `<main>` the
 * instant a link is followed.
 *
 * # The gate still runs on every page
 *
 * `requireStaff` is memoised per request (`src/server/session.ts`), so a
 * full render costs one session read for this layout and its page together.
 * On a client-side navigation Next re-renders the page but not this layout,
 * and every page still calls `requireStaff()` itself before it reads
 * anything — so the gate runs on every navigation, exactly as before, and
 * this layout's call only decides whether there is a shell to draw.
 *
 * A vpay this layout cannot reach (`outage`) is rendered by the page, which
 * meets the same memoised answer and renders the failure with the cookie
 * kept (issue #88 item 2) — without a shell, because there is no session to
 * say who is signed in.
 */
export const dynamic = "force-dynamic";

export default async function DashLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const gate = await requireStaff();
  return (
    <DashProviders>
      {gate.kind === "ready" ? (
        <AppShell
          email={gate.staff.session.email}
          merchantId={gate.staff.session.merchant_id}
          signOut={signOut}
        >
          {children}
        </AppShell>
      ) : (
        children
      )}
    </DashProviders>
  );
}
