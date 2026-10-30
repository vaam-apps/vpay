"use client";

import type { FormAction } from "../form-state";
import { OneQuestionSteps, type Question } from "./one-question-steps";

/** RFC 6238's digit count, and what the field is constrained to. */
export const CODE_LENGTH = 6;

export interface TotpFormProps {
  /** `submitTotp` from `src/server/actions.ts`. */
  action: FormAction;
  /**
   * Whether this is a first sign-in. Changes only the button — the question,
   * the action and the request are the same, because vpay decides which secret
   * authenticates and this form never carries one.
   */
  enrolling?: boolean;
}

/** Six digits and nothing else. */
const CODE_SHAPE = new RegExp(`^[0-9]{${CODE_LENGTH}}$`);

/** The one question: the code the authenticator app shows. */
export const TOTP_QUESTIONS: readonly Question[] = [
  {
    name: "code",
    label: "One-time code",
    type: "text",
    // Lets a platform offer the code it just saw in a notification.
    autoComplete: "one-time-code",
    inputMode: "numeric",
    maxLength: CODE_LENGTH,
    isValid: (value) => CODE_SHAPE.test(value.trim()),
  },
];

/**
 * Leg two: the six digits.
 *
 * Continue stays disabled until there are six, and while the action runs: the
 * code is verified behind a compare-and-swap replay guard that admits only a
 * strictly greater time step, so the **second** submission of one code is
 * refused on purpose — a double-click would show "we could not sign you in" to
 * somebody who typed a perfectly good code.
 */
export function TotpForm({ action, enrolling = false }: TotpFormProps) {
  return (
    <OneQuestionSteps
      action={action}
      questions={TOTP_QUESTIONS}
      submitLabel={enrolling ? "Confirm and finish enrolment" : "Continue"}
      pendingLabel="Checking…"
    />
  );
}
