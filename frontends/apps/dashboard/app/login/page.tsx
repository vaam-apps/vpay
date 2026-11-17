import { redirect } from "next/navigation";

import { Code, ScreenStack } from "@vaam-apps/ui";

import { dashboardConfig } from "../../src/config/runtime";
import {
  NotConfigured,
  PairedKeyOnly,
  SignInUnreachable,
} from "../../src/components/login-notices";
import { SignInForm } from "../../src/components/sign-in-form";
import {
  SsoFailedNotice,
  SsoNotConfigured,
  SsoSignIn,
  SsoUnchecked,
} from "../../src/components/sso-sign-in";
import { SSO_ISSUER_VAR } from "../../src/config/settings";
import { signIn } from "../../src/server/actions";
import { getSsoStatus } from "../../src/server/api";
import { readDiscovery } from "../../src/server/discovery";
import { alreadySignedIn, HOME_PATH } from "../../src/server/session";

/**
 * `/login` — leg one of ADR-0017's two-factor sign-in, **when vpay serves it**.
 *
 * **The page a dead session lands on.** Every other route in this app clears
 * the cookie and redirects here when vpay answers `401`, and this page never
 * calls the gate — which is what makes "back to login with no loop" true
 * rather than intended.
 *
 * # It asks vpay before it asks for a password
 *
 * `GET /.well-known/vpay-auth` is read on the server, on every render
 * (`server/discovery.ts`), and decides which of four pages this is:
 *
 * | vpay says                          | this page                         |
 * | ---------------------------------- | --------------------------------- |
 * | it could not be asked              | the outage, with the request id    |
 * | `human_login` is not served        | "signs in with a paired key"       |
 * | served, container not configured   | the variables to set, no form      |
 * | served and configured              | the email question                 |
 *
 * # Single sign-on is a button beside the email question (VPA-5b)
 *
 * On the last row only, `GET /dash/v1/staff/sso` is read as well, and the
 * button is drawn exactly when it says `enabled: true` **and** this container
 * knows the identity provider's origin (`VPAY_DASHBOARD_SSO_ISSUER`). The
 * read failing is not "off": the page says SSO could not be checked, with the
 * request id, and keeps the password question. `?sso=failed` is where a
 * refused flow lands (`server/sso.ts`); it carries no reason.
 *
 * The first row is never the second: a vpay that is restarting does not lack a
 * password sign-in. And the third is only reachable when vpay *does* serve one,
 * so a deployment that has no `dashboard_client` is not told to write three
 * variables it has nowhere to point.
 */
export const dynamic = "force-dynamic";

export default async function LoginPage({
  searchParams,
}: {
  searchParams?: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { config, problems, apiBaseUrl, ssoIssuerOrigin } = dashboardConfig();

  if (apiBaseUrl === null) {
    // Nothing can be asked without vpay's address. `problems` names it (and the
    // three that follow it, which are reported together so an operator fixes
    // them in one restart).
    return <NotConfigured problems={problems} />;
  }

  const discovery = await readDiscovery(apiBaseUrl);
  if (discovery.kind === "unreadable") {
    return <SignInUnreachable failure={discovery.failure} />;
  }
  if (discovery.kind === "paired-key-only") {
    return <PairedKeyOnly />;
  }
  if (config === null) {
    return <NotConfigured problems={problems} />;
  }

  if (await alreadySignedIn()) {
    redirect(HOME_PATH);
  }

  const sso = await getSsoStatus(apiBaseUrl);
  const ssoFailed = (await searchParams)?.sso === "failed";

  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        Sign in
      </h2>
      {ssoFailed ? <SsoFailedNotice /> : null}
      <p className="text-body text-muted-foreground">
        Staff accounts are created with <Code>vpay-server staff add</Code>.
        There is no sign-up.
      </p>
      {sso.kind === "unreadable" ? (
        <SsoUnchecked requestId={sso.failure.requestId} />
      ) : null}
      {sso.kind === "enabled" && ssoIssuerOrigin === null ? (
        <SsoNotConfigured variable={SSO_ISSUER_VAR} />
      ) : null}
      <SignInForm
        action={signIn}
        {...(sso.kind === "enabled" && ssoIssuerOrigin !== null
          ? { sso: <SsoSignIn /> }
          : {})}
      />
    </ScreenStack>
  );
}
