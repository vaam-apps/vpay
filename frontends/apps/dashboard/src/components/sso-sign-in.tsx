import { Button, Separator } from "@vaam-apps/ui";

import { SSO_START_PATH } from "../server/sso-paths";

/** The one sentence a failed single sign-in is ever described with. */
export const SSO_FAILED_MESSAGE =
  "Single sign-on did not complete. Try again, or sign in with your password.";

/**
 * The secondary way in: a plain `<form method="post">` to
 * `/api/auth/sso/start`.
 *
 * Not a server action and not a client component, on purpose: the start is a
 * route that answers a `303` to the identity provider, which a browser follows
 * from a form post with no JavaScript at all. There is nothing to be pending
 * about — the browser leaves the page — so there is no state here either.
 *
 * Rendered only when vpay says the deployment offers it; the password form
 * above it is never conditional on this.
 */
export function SsoSignIn() {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <Separator className="flex-1" />
        <span className="text-caption text-muted-foreground">or</span>
        <Separator className="flex-1" />
      </div>
      <form method="post" action={SSO_START_PATH}>
        <Button type="submit" variant="secondary" className="w-full">
          Sign in with Keycloak
        </Button>
      </form>
    </div>
  );
}
