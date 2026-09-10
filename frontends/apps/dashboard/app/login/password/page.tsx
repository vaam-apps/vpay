import { redirect } from "next/navigation";

import { Alert, Heading, Stack, Text } from "@vpay/ui";

import { dashboardConfig } from "../../../src/config/runtime";
import { PasswordForm } from "../../../src/components/password-form";
import { changePassword } from "../../../src/server/actions";
import { ReadFailure } from "../../../src/components/read-failure";
import { refusalFor } from "../../../src/server/gate";
import {
  HOME_PATH,
  LOGIN_PATH,
  readSession,
  sessionToken,
} from "../../../src/server/session";

/**
 * `/login/password` — replacing the one-time password the operator printed.
 *
 * Not a nag screen and not optional. ADR-0017 decision 1: every authenticated
 * route refuses a session whose staff row still carries
 * `password_change_required`, `/oauth/authorize` included — so a session that
 * reaches this page can reach nothing else, and no `/dash/v1` token exists
 * for it yet. That is what stops a printed password becoming a long-lived
 * credential by being ignored.
 *
 * A session that does **not** carry the flag is sent to the payments list
 * rather than shown a form: this is a step in a sign-in, not a settings page,
 * and this app has no password-change screen for someone who already
 * replaced theirs.
 */
export const dynamic = "force-dynamic";

export default async function PasswordPage() {
  const { config } = dashboardConfig();
  if (config === null) {
    redirect(LOGIN_PATH);
  }

  const token = await sessionToken();
  if (token === null) {
    redirect(LOGIN_PATH);
  }

  const { session, failure } = await readSession(config, token);
  if (session === null) {
    // Only a `401` means the session is over — issue #88 item 2, and
    // `server/gate.ts` carries the argument. A vpay that could not be reached
    // renders here rather than bouncing somebody who is mid-sign-in back to
    // the form with their one-time password already spent.
    if (failure !== null && refusalFor(failure) === "outage") {
      return (
        <Stack direction="column" gap="md">
          <Heading level={2}>Choose a password</Heading>
          <ReadFailure failure={failure} />
        </Stack>
      );
    }
    redirect(LOGIN_PATH);
  }
  if (!session.password_change_required) {
    redirect(HOME_PATH);
  }

  return (
    <Stack direction="column" gap="md">
      <Heading level={2}>Choose a password</Heading>
      <Alert tone="warning" role="status">
        You signed in with the one-time password an operator printed. Replace it
        before going any further — nothing else is reachable until you do.
      </Alert>
      <Text tone="muted" size="sm">
        Signed in as <strong>{session.email}</strong>.
      </Text>
      <PasswordForm action={changePassword} />
    </Stack>
  );
}
