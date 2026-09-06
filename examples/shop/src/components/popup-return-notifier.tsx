"use client";

import { useEffect } from "react";
import { notifyCheckoutOpener } from "@vaam-apps/vpay-stripe-js";

/**
 * The popup half of the popup integration, on the shop's own return page.
 *
 * A popup is not a frame: inside one `window.parent === window`, so vpay's
 * checkout page has no framer to post `vpay:complete` to and deliberately
 * says nothing. What closes the loop is this — `success_url`, running
 * **inside the popup**, telling `window.opener` the payer is finished and
 * closing the window.
 *
 * It renders nothing and it is safe on every other path:
 * `notifyCheckoutOpener` answers `false` and does nothing at all when there
 * is no opener, which is exactly what a payer who came back by an ordinary
 * redirect has. One return page therefore serves the hosted, popup and
 * embedded integrations without branching on a query parameter.
 *
 * **`close: false`, deliberately** (review finding, 2026-09-06). "Has an
 * opener" is not the same as "is a vpay popup": a shop tab that some *other*
 * page opened with `window.open` has one too, and a payer checking out from
 * it by an ordinary redirect reaches this very page. The message is harmless
 * there — it is pinned to `targetOrigin`, so a third-party opener receives
 * nothing — but closing the window is not, and the default would have closed
 * that payer's tab. The popup is still closed on the paths where it should
 * be: `openCheckoutPopup`'s own listener closes it, and only on a message it
 * accepted from the window it opened.
 *
 * The message is a **cue**. The opener navigates to this same page in its
 * own window, where the poll reads the shop's database — which only the
 * signature-verified webhook writes.
 */
export function PopupReturnNotifier({
  sessionId,
  status,
}: {
  sessionId: string | null;
  status: string;
}) {
  useEffect(() => {
    if (sessionId === null) {
      // vpay substitutes `{CHECKOUT_SESSION_ID}` into `success_url`, so a
      // return with no session id is not a completion this page can name.
      // Posting an empty one would be inventing the payload the opener acts
      // on.
      return;
    }
    notifyCheckoutOpener({ session: sessionId, status, close: false });
  }, [sessionId, status]);
  return null;
}
