"use client";

import { useActionState } from "react";

import { Button } from "@vaam-apps/ui";
import { LogOut } from "lucide-react";

import { NO_ERROR, type FormAction } from "../form-state";
import { FormAlert } from "./form-alert";

export interface SignOutControlProps {
  /** `signOut` from `src/server/actions.ts`. */
  signOut: FormAction;
}

/**
 * The way out: a `<form>` POST, and the sentence that says it did not work.
 *
 * A form rather than a link, so it cannot be triggered by a `GET` — a sign-out
 * link is something a prefetcher, a link scanner or an `<img src>` in a chat
 * message can fire (and the action carries the origin check in
 * `server/csrf.ts`, because a `POST` from another site is the same thing).
 *
 * A client component because the action can **refuse**: when vpay did not
 * confirm the revocation the cookie is kept and the person is told they are
 * still signed in, here, beside the button that will try again. The success
 * path never returns — the action redirects to `/login`.
 *
 * `secondary`, M3's outlined button: sign-out ends this session and revokes
 * its token, but it destroys nothing an operator would need back, so it is
 * not painted in the danger hue the status system reserves for failures.
 */
export function SignOutControl({ signOut }: SignOutControlProps) {
  const [state, dispatch, pending] = useActionState(signOut, NO_ERROR);
  return (
    <form action={dispatch} className="flex flex-col gap-3">
      <FormAlert error={state.error} requestId={state.requestId} />
      <Button
        type="submit"
        variant="secondary"
        size="sm"
        className="w-full"
        disabled={pending}
      >
        <LogOut size={16} aria-hidden="true" />
        Sign out
      </Button>
    </form>
  );
}
