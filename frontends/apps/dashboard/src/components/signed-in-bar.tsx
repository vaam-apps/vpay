import { Button, Code } from "@vaam-apps/ui";
import { LogOut } from "lucide-react";

export interface SignedInBarProps {
  /** The address this session signed in with. */
  email: string;
  /** The one tenant this session may read — the dashboard client's binding. */
  merchantId: string;
  /** `signOut` from `src/server/actions.ts`. */
  signOut: () => Promise<void>;
}

/**
 * Who is signed in, which tenant they are reading, and the way out.
 *
 * The merchant is on screen because it is not a preference: `/dash/v1` reads
 * exactly one tenant, fixed by the dashboard client's registration and by
 * nothing in any token (`vpay_api::require_dashboard_token` authorises by
 * registration). An operator looking at an empty payments list needs to be
 * able to tell "this merchant has no payments" from "I am looking at the
 * wrong merchant", and no other element on the page answers that.
 *
 * Sign-out is a `<form>` POST rather than a link, so it works with no
 * JavaScript and so it cannot be triggered by a `GET` — a sign-out link is
 * something a prefetcher, a link scanner or an `<img src>` in a chat message
 * can fire.
 *
 * **Not in the root layout.** The layout is rendered by `/login` too, and a
 * "Sign out" control on a page where nobody is signed in is the same kind of
 * claim as a nav entry for a page nobody wrote.
 */
export function SignedInBar({
  email,
  merchantId,
  signOut,
  layout = "stacked",
}: SignedInBarProps & {
  /**
   * `stacked` (default) — the account block in the sidebar and the More
   * sheet: who, which merchant, then a full-width Sign out. `inline` — the
   * top app bar from 640 to 1279px, where the rail has no room for an
   * address: one row, the merchant left to the bar's own chip.
   */
  readonly layout?: "stacked" | "inline";
}) {
  const initial = email.trim().charAt(0).toUpperCase() || "?";
  const avatar = (
    <span
      aria-hidden="true"
      className="block rounded-full bg-primary-container"
    >
      <span className="grid size-9 place-items-center text-on-primary-container">
        <span className="font-display text-title-medium">{initial}</span>
      </span>
    </span>
  );
  /*
    `secondary`, M3's outlined button: sign-out ends this session and
    revokes its token, but it destroys nothing an operator would need back,
    so it is not painted in the danger hue the status system reserves for
    failures. A filled red button in every account block was the loudest
    thing on every screen, louder than a failed payment.
  */
  const signOutForm = (
    <form action={signOut}>
      <Button type="submit" variant="secondary" size="sm" className="w-full">
        <LogOut size={16} aria-hidden="true" />
        Sign out
      </Button>
    </form>
  );
  if (layout === "inline") {
    return (
      <div className="flex min-w-0 items-center gap-3">
        {avatar}
        <span className="truncate text-label-large text-foreground">
          {email}
        </span>
        {signOutForm}
      </div>
    );
  }
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex min-w-0 items-center gap-3">
        {avatar}
        <span className="flex min-w-0 flex-col">
          <span className="truncate text-label-large text-foreground">
            {email}
          </span>
          <span className="truncate text-body-small text-muted-foreground">
            merchant <Code>{merchantId}</Code>
          </span>
        </span>
      </div>
      {signOutForm}
    </div>
  );
}
