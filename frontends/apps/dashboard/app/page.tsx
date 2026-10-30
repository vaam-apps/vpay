import { AccountPage } from "../src/components/account-page";
import { CenteredShell } from "../src/components/centered-shell";
import { ReadFailure } from "../src/components/read-failure";
import { signOut } from "../src/server/actions";
import { requireStaff } from "../src/server/session";

/**
 * `/` — the account page, and the first page behind the gate.
 *
 * `requireStaff` redirects a browser with no session to `/login`, one mid
 * sign-in to its code form, one that must replace the printed password to
 * `/login/password`, and one whose session vpay refused to `/signed-out`. What
 * is left is a signed-in staff member, or a vpay that could not be asked — which
 * renders here and keeps the cookie, because the session may be perfectly good
 * (issue #88 item 2).
 */
export const dynamic = "force-dynamic";

export default async function Home() {
  const gate = await requireStaff();
  if (gate.kind === "outage") {
    return (
      <CenteredShell>
        <ReadFailure failure={gate.failure} />
      </CenteredShell>
    );
  }
  const { session } = gate.staff;
  return (
    <CenteredShell>
      <AccountPage
        displayName={session.display_name}
        email={session.email}
        merchantId={session.merchant_id}
        signOut={signOut}
      />
    </CenteredShell>
  );
}
