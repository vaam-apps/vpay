import { redirect } from "next/navigation";

import { Alert, Code, Heading, List, Stack, Text } from "@vpay/ui";

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
      <Stack direction="column" gap="md">
        <Heading level={2}>No sign-in is configured</Heading>
        <Alert tone="warning" role="status">
          This container has no dashboard client registration, so it can sign
          nobody in. Nothing is wrong with your account.
        </Alert>
        <Text tone="muted" size="sm">
          The operator of this deployment has to set:
        </Text>
        <List>
          {problems.map((problem) => (
            <li key={problem.variable}>
              <Code>{problem.variable}</Code> — {problem.detail}
            </li>
          ))}
        </List>
      </Stack>
    );
  }

  if (await alreadySignedIn()) {
    redirect(HOME_PATH);
  }

  return (
    <Stack direction="column" gap="md">
      <Heading level={2}>Sign in</Heading>
      <Text tone="muted" size="sm">
        Staff accounts are created with <Code>vpay-server staff add</Code>.
        There is no sign-up.
      </Text>
      <SignInForm action={signIn} />
    </Stack>
  );
}
