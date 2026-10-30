import { ScreenStack } from "@vaam-apps/ui";

import type { FormAction } from "../form-state";
import type { ApiFailure } from "../server/api";
import { ReadFailure } from "./read-failure";
import { SignOutControl } from "./sign-out-control";

export interface MintRefusedProps {
  /** vpay's refusal of the token, as `server/api.ts` shaped it. */
  failure: ApiFailure;
  /** `signOut` from `src/server/actions.ts`. */
  signOut: FormAction;
}

/**
 * `/` when the person is signed in and vpay would not mint the token that
 * reads `/dash/v1` for them.
 *
 * It is a page and not a redirect because the session is good: every
 * redirect available leads back here (see `StaffGate`'s `refused`). So the
 * screen says what is true — the sign-in worked, the authorisation did not —
 * quotes the request id, and offers the one move that is the person's, Sign
 * out. It guesses the cause only as far as vpay's own wording lets it: the
 * refusal is one answer for four reasons, and one of them, an account on
 * another merchant than the dashboard is bound to, is the one an operator can
 * act on.
 */
export function MintRefused({ failure, signOut }: MintRefusedProps) {
  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        Signed in, but not authorised
      </h2>
      <p className="text-body text-muted-foreground">
        Your sign-in worked, and vpay then refused to let this dashboard read on
        your behalf. Your password is not the problem. A common cause is an
        account that belongs to a different merchant than the one this dashboard
        is set up for. Ask an operator to check the account, and quote the
        request id below.
      </p>
      <ReadFailure failure={failure} />
      <SignOutControl signOut={signOut} />
    </ScreenStack>
  );
}
