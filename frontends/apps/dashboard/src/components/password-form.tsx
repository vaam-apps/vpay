"use client";

import type { FormAction } from "../form-state";
import { OneQuestionSteps, type Question } from "./one-question-steps";

/**
 * The shortest password vpay accepts (`vpay_api::staff::MIN_PASSWORD_CHARS`).
 *
 * Stated here so the screen can say so before a round trip, and it is a
 * *hint*: the server's own bound is the rule, and a bound that drifted from it
 * would refuse something vpay accepts. There is deliberately no composition
 * rule — length is the only one, because "one digit, one symbol" shrinks the
 * space people choose from and is the reason `Password1!` exists.
 */
export const MIN_PASSWORD_CHARS = 12;

export interface PasswordFormProps {
  /** `changePassword` from `src/server/actions.ts`. */
  action: FormAction;
}

/** The password change's three questions, in the order they are asked. */
export const PASSWORD_QUESTIONS: readonly Question[] = [
  {
    name: "current_password",
    label: "Current password",
    hint: "On your first sign-in this is the one-time password you were given.",
    type: "password",
    // So a password manager offers the stored one rather than generating a
    // new one into it, which is what `new-password` here would do.
    autoComplete: "current-password",
    isValid: (value) => value.length > 0,
  },
  {
    name: "new_password",
    label: "New password",
    hint: `At least ${MIN_PASSWORD_CHARS} characters. Length is the only rule.`,
    type: "password",
    autoComplete: "new-password",
    isValid: (value) => value.length >= MIN_PASSWORD_CHARS,
  },
  {
    name: "confirm_password",
    label: "Repeat it",
    type: "password",
    autoComplete: "new-password",
    // The one rule vpay cannot check: it sees one password, and a mistyped
    // pair would be a password nobody can reproduce.
    isValid: (value, answers) =>
      value.length > 0 && value === answers["new_password"],
  },
];

/**
 * Replacing the one-time password `vpay-server staff add` printed — three
 * screens: the one in force, the new one, the new one again.
 *
 * Not a nag screen: `/oauth/authorize` refuses a session whose staff row still
 * carries `password_change_required`, so until this is done no `/dash/v1`
 * token exists at all — which is what stops a printed credential becoming a
 * long-lived one by being ignored.
 *
 * # The current password, and why a session is not enough
 *
 * The session making the change presented both factors once, up to twelve
 * hours earlier; so without the password in force the credential protecting an
 * account takeover is the session cookie alone (issue #79 item 3). On a first
 * sign-in the password in force is the one `staff add` printed, so that is what
 * the first screen's hint says.
 */
export function PasswordForm({ action }: PasswordFormProps) {
  return (
    <OneQuestionSteps
      action={action}
      questions={PASSWORD_QUESTIONS}
      submitLabel="Set password"
      pendingLabel="Saving…"
    />
  );
}
