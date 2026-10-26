import { CHECKOUTS_SCREEN } from "../../../src/dash/list-screens";
import { ProcedureListScreen } from "../../../src/components/procedure-list-screen";
import { CHECKOUT_SESSIONS } from "../../../src/dash/resource-name";
import { CheckoutsList } from "./checkouts-list";

/**
 * `/checkouts` — what happened to this merchant's checkouts.
 *
 * # The two statuses are the point of the screen
 *
 * `status` and `payment_status` are separate columns and both are primary,
 * because the interesting row is the one where they disagree: a session
 * `complete` whose payment is `unpaid` is a checkout the payer finished and
 * the rail did not. One combined column would hide exactly the case an
 * operator opens this screen to find.
 *
 * # Two payer credentials are absent, and not by omission
 *
 * `client_secret_suffix` and `return_token` are not on the procedure's
 * summary type at all — the client secret authorises the payer's browser
 * against the checkout and the return token authorises the return leg, so a
 * list carrying either hands them to everyone who can read the list.
 * `search_checkout_sessions.rs` refuses them in the statement, the row
 * struct and the summary, and a container test serialises the answer to
 * prove it.
 *
 * The three URLs are absent too: merchant-supplied, up to 2 048 characters
 * each, and nothing a list is read for.
 */
export const dynamic = "force-dynamic";

export default async function CheckoutsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  return (
    <ProcedureListScreen
      {...CHECKOUTS_SCREEN}
      resource={CHECKOUT_SESSIONS}
      basePath="/checkouts"
      params={await searchParams}
      List={CheckoutsList}
      emptyMessage="No checkouts. This merchant has no checkout sessions yet."
    />
  );
}
