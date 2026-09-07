'use client';

import { useActionState } from 'react';

import { Button, Field, FieldLabel, Input, Stack } from '@vpay/ui';

import { NO_ERROR, type FormAction } from '../form-state';
import { FormAlert } from './form-alert';

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
 * is refused by the per-email rate limiter after enough of them. `Field`
 * carries the invalid state so the `aria-invalid` a screen reader reads comes
 * from the component's own validation rather than from a hand-rolled
 * `aria-*` prop.
 *
 * The error is rendered in exactly one place — see {@link FormAlert}.
 */
export function SignInForm({ action }: SignInFormProps) {
  const [state, submit, pending] = useActionState(action, NO_ERROR);

  return (
    <form action={submit}>
      <Stack direction="column" gap="md">
        <Field invalid={state.error !== null}>
          <FieldLabel htmlFor="dashboard-signin-email">Work email</FieldLabel>
          <Input
            id="dashboard-signin-email"
            name="email"
            type="email"
            required
            autoComplete="username"
            disabled={pending}
          />
        </Field>

        <Field invalid={state.error !== null}>
          <FieldLabel htmlFor="dashboard-signin-password">Password</FieldLabel>
          <Input
            id="dashboard-signin-password"
            name="password"
            type="password"
            required
            autoComplete="current-password"
            disabled={pending}
          />
        </Field>

        <FormAlert error={state.error} requestId={state.requestId} />

        <Button type="submit" block disabled={pending}>
          {pending ? 'Signing in…' : 'Sign in'}
        </Button>
      </Stack>
    </form>
  );
}
