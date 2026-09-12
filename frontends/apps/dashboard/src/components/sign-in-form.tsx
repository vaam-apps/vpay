"use client";

import { useActionState } from "react";

import { Button, FormField, Input } from "@vaam-apps/ui";

import { NO_ERROR, type FormAction } from "../form-state";
import { FormAlert } from "./form-alert";

export interface SignInFormProps {
  /** `signIn` from `src/server/actions.ts`, passed in so this file imports no server code. */
  action: FormAction;
}

/**
 * Leg one of the sign-in: the work email and the password.
 *
 * **Two legs, not one.** ADR-0017 decision 1 makes both mandatory — argon2id
 * password, then RFC 6238 TOTP — and they are two requests because the second
 * is authorised by the session the first creates. This form ends at
 * `/login/totp`; it never sees a code.
 *
 * `pending` disables every control and is not decoration: without it a
 * double-click submits twice, and the second submission of a credential pair
 * is refused by the per-email rate limiter after enough of them. `FormField`
 * derives no validation state of its own (2026-09-12, `@vaam-apps/ui`
 * cutover — it has no `invalid` prop), so `aria-invalid` is set explicitly
 * on each control from `state.error` instead.
 *
 * The error is rendered in exactly one place — see {@link FormAlert}. Never
 * pass `error` to `FormField` here: it renders its own `role="alert"`
 * paragraph, which would print the refusal a second time and make two
 * elements answer `findByRole("alert")`.
 */
export function SignInForm({ action }: SignInFormProps) {
  const [state, submit, pending] = useActionState(action, NO_ERROR);
  const invalid = state.error !== null;

  return (
    <form action={submit}>
      <div className="flex flex-col gap-4">
        <FormField label="Work email" htmlFor="dashboard-signin-email">
          <Input
            id="dashboard-signin-email"
            name="email"
            type="email"
            required
            autoComplete="username"
            disabled={pending}
            aria-invalid={invalid ? "true" : undefined}
          />
        </FormField>

        <FormField label="Password" htmlFor="dashboard-signin-password">
          <Input
            id="dashboard-signin-password"
            name="password"
            type="password"
            required
            autoComplete="current-password"
            disabled={pending}
            aria-invalid={invalid ? "true" : undefined}
          />
        </FormField>

        <FormAlert error={state.error} requestId={state.requestId} />

        <Button type="submit" className="w-full" disabled={pending}>
          {pending ? "Signing in…" : "Sign in"}
        </Button>
      </div>
    </form>
  );
}
