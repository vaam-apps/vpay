import { Button, InlineBanner } from "@vaam-apps/ui";

import { SSO_START_PATH } from "../server/sso-paths";

/** The one sentence a failed single sign-on is ever described with. */
export const SSO_FAILED_MESSAGE =
  "Single sign-on did not complete. Try again, or sign in with your password.";

/** The sentence when vpay could not be asked whether SSO is offered. */
export const SSO_UNCHECKED_MESSAGE =
  "Single sign-on could not be checked. You can still sign in with your password, or reload to try again.";

/**
 * The secondary way in: a button that posts to `/api/auth/sso/start`.
 *
 * **A button beside the email question, not a form**: it asks nothing, so
 * there is no field, no stepper and no state. It is a single-button
 * `<form method="post">` only because the start is a route that answers a
 * `303` to the identity provider, which a browser follows from a post with no
 * JavaScript at all — not a server action and not a client component, on
 * purpose. There is nothing to be pending about: the browser leaves the page.
 *
 * Rendered only when vpay says the deployment offers it
 * ({@link import('../server/api').getSsoStatus}); the password question above
 * it is never conditional on this.
 */
export function SsoSignIn() {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-center text-caption text-muted-foreground">or</p>
      <form method="post" action={SSO_START_PATH}>
        <Button type="submit" variant="secondary" className="w-full">
          Sign in with single sign-on
        </Button>
      </form>
    </div>
  );
}

/** `/login?sso=failed`: the generic message, with no reason attached. */
export function SsoFailedNotice() {
  return (
    <div role="alert">
      <InlineBanner variant="danger">{SSO_FAILED_MESSAGE}</InlineBanner>
    </div>
  );
}

/**
 * vpay could not be asked whether SSO is offered. The password question is
 * still there; the button is not hidden *silently* — this says why it is not
 * drawn, with the request id when there is one.
 */
export function SsoUnchecked({
  requestId,
}: {
  readonly requestId: string | null;
}) {
  return (
    <div role="status">
      <InlineBanner variant="warning">
        <span>{SSO_UNCHECKED_MESSAGE}</span>
        {requestId === null ? null : (
          <span className="block text-caption text-muted-foreground">
            Request {requestId}
          </span>
        )}
      </InlineBanner>
    </div>
  );
}

/**
 * SSO is on at vpay, and this dashboard does not know which identity provider
 * to trust a redirect to, so it does not offer the button.
 */
export function SsoNotConfigured({ variable }: { readonly variable: string }) {
  return (
    <div role="status">
      <InlineBanner variant="warning">
        Single sign-on is on at vpay but this dashboard cannot check where it
        sends you, so it is not offered. The operator has to set {variable} to
        the identity provider&apos;s issuer URL.
      </InlineBanner>
    </div>
  );
}
