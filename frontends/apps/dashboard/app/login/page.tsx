import { redirect } from "next/navigation";

import { Code, InlineBanner, ScreenStack } from "@vaam-apps/ui";

import { dashboardConfig } from "../../src/config/runtime";
import { SignInForm } from "../../src/components/sign-in-form";
import { signIn } from "../../src/server/actions";
import { alreadySignedIn, HOME_PATH } from "../../src/server/session";

/**
 * `/login` — leg one of ADR-0017's two-factor sign-in.
 *
 * **The page a dead session lands on.** Every other route in this app clears
 * the cookie and redirects here when vpay answers `401`, and this page never
 * calls the gate — which is what makes "back to login with no loop" true
 * rather than intended.
 *
 * A deployment with no `dashboard_client`, or with `staff_auth` incomplete,
 * mounts no login at all (ADR-0017's Consequences: "in a sandbox deployment
 * they may be absent, and the consequence is stated rather than hidden").
 * This page says which settings are missing instead of rendering a form that
 * could not have worked — an operator reading it is the person who can fix
 * it, and a staff member reading it learns not to keep retyping a password.
 */
export const dynamic = "force-dynamic";

export default async function LoginPage() {
  const { config, problems } = dashboardConfig();

  if (config === null) {
    return (
      <ScreenStack>
        <h2 className="text-title-sm font-medium text-foreground">
          No sign-in is configured
        </h2>
        <div role="status">
          <InlineBanner variant="warning">
            This container has no dashboard client registration, so it can sign
            nobody in. Nothing is wrong with your account.
          </InlineBanner>
        </div>
        <p className="text-body text-muted-foreground">
          The operator of this deployment has to set:
        </p>
        <ul className="flex list-disc flex-col gap-1 pl-5 text-body text-foreground">
          {problems.map((problem) => (
            <li key={problem.variable}>
              <Code>{problem.variable}</Code> — {problem.detail}
            </li>
          ))}
        </ul>
      </ScreenStack>
    );
  }

  if (await alreadySignedIn()) {
    redirect(HOME_PATH);
  }

  return (
    <ScreenStack>
      <h2 className="text-title-sm font-medium text-foreground">Sign in</h2>
      <p className="text-body text-muted-foreground">
        Staff accounts are created with <Code>vpay-server staff add</Code>.
        There is no sign-up.
      </p>
      <SignInForm action={signIn} />
    </ScreenStack>
  );
}
