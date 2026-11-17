import { Code } from "@vaam-apps/ui";

import type { FormAction } from "../form-state";
import { SignOutControl } from "./sign-out-control";

export interface SignedInBarProps {
  /** The address this session signed in with. */
  email: string;
  /** The one tenant this session may read — the dashboard client's binding. */
  merchantId: string;
  /** `signOut` from `src/server/actions.ts`. */
  signOut: FormAction;
}

/**
 * Who is signed in, which tenant they are reading, and the way out.
 *
 * The merchant is on screen because it is not a preference: `/dash/v1` reads
 * exactly one tenant, fixed by the dashboard client's registration and by
 * nothing in any token. An operator looking at an empty list needs to be able
 * to tell "this merchant has no payments" from "I am looking at the wrong
 * merchant", and no other element on the page answers that.
 *
 * Sign-out is {@link SignOutControl}: a `<form>` POST rather than a link, and
 * the place its refusal is shown.
 *
 * **Not in the root layout.** The layout is rendered by `/login` too, and a
 * "Sign out" control on a page where nobody is signed in is the same kind of
 * claim as a nav entry for a page nobody wrote.
 */
export function SignedInBar({ email, merchantId, signOut }: SignedInBarProps) {
  const initial = email.trim().charAt(0).toUpperCase() || "?";
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex min-w-0 items-center gap-3">
        <span
          aria-hidden="true"
          className="block rounded-full bg-primary-container"
        >
          <span className="grid size-9 place-items-center text-on-primary-container">
            <span className="font-display text-title-medium">{initial}</span>
          </span>
        </span>
        <span className="flex min-w-0 flex-col">
          <span className="truncate text-label-large text-foreground">
            {email}
          </span>
          <span className="truncate text-body-small text-muted-foreground">
            merchant <Code>{merchantId}</Code>
          </span>
        </span>
      </div>
      <SignOutControl signOut={signOut} />
    </div>
  );
}
