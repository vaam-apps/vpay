"use client";

import type { ReactNode } from "react";

import type { FormAction } from "../form-state";
import { OneQuestionSteps, type Question } from "./one-question-steps";

export interface SignInFormProps {
  /** `signIn` from `src/server/actions.ts`, passed in so this file imports no server code. */
  action: FormAction;
  /** The single sign-on button, or nothing; shown under the email question only. */
  sso?: ReactNode;
}

/**
 * An address has an `@` with something on both sides of it and no space.
 *
 * Deliberately no more than that: whether it is *an account* is vpay's to
 * answer, and the same sentence answers "no such address" and "wrong password"
 * so nothing here may hint at which. A stricter pattern would only refuse an
 * address vpay accepts.
 */
const EMAIL_SHAPE = /^[^\s@]+@[^\s@]+$/;

/** The sign-in's two questions, in the order they are asked. */
export const SIGN_IN_QUESTIONS: readonly Question[] = [
  {
    name: "email",
    label: "Work email",
    type: "email",
    autoComplete: "username",
    isValid: (value) => EMAIL_SHAPE.test(value.trim()),
  },
  {
    name: "password",
    label: "Password",
    type: "password",
    autoComplete: "current-password",
    // Not trimmed: a password's leading or trailing space is part of it.
    isValid: (value) => value.length > 0,
  },
];

/**
 * Leg one of the sign-in: the work email, then the password — two screens.
 *
 * **Two legs, not one.** ADR-0017 decision 1 makes both factors mandatory —
 * argon2id password, then RFC 6238 TOTP — and they are two requests because the
 * second is authorised by the session the first creates. This ends at
 * `/login/totp`; it never sees a code. Inside the first leg the email and the
 * password are separate screens (`OneQuestionSteps`), which is the rule that
 * one screen collects one piece of information.
 */
export function SignInForm({ action, sso }: SignInFormProps) {
  return (
    <OneQuestionSteps
      action={action}
      questions={SIGN_IN_QUESTIONS}
      submitLabel="Sign in"
      pendingLabel="Signing in…"
      usernameFrom="email"
      {...(sso === undefined ? {} : { alternative: sso })}
    />
  );
}
