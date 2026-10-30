import { Code, ScreenStack } from "@vaam-apps/ui";

import { SignedInBar, type SignedInBarProps } from "./signed-in-bar";

export interface AccountPageProps extends SignedInBarProps {
  /** What to greet them by (`display_name` on the session). */
  readonly displayName: string;
}

/**
 * `/` — who is signed in, which tenant they read, and the way out.
 *
 * **An account page and nothing more, and it says so.** The pages that read
 * `/dash/v1` are a later ticket (VP-6b), so a link to `/payments` would be a
 * menu entry for a page nobody wrote. What a staff member can honestly be
 * shown after signing in is that they are signed in, as whom, reading which
 * merchant — the one fact an operator needs before trusting any list that
 * follows — and a sign-out that revokes the token.
 *
 * The notice is a status and plain text, not a banner: nothing is broken, and
 * a banner variant is a colour pair `payment-status-contrast.test.ts` would
 * have to measure.
 */
export function AccountPage({
  displayName,
  email,
  merchantId,
  signOut,
}: AccountPageProps) {
  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        Signed in as {displayName}
      </h2>
      <SignedInBar email={email} merchantId={merchantId} signOut={signOut} />
      <p role="status" className="text-body text-muted-foreground">
        The payments, refunds and deliveries screens are not built in this
        dashboard yet. Until they are, <Code>/dash/v1</Code> is read by a client
        that signs its requests.
      </p>
    </ScreenStack>
  );
}
