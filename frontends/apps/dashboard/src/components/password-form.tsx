'use client';

import { useActionState } from 'react';

import { Button, Field, FieldDescription, FieldLabel, Input, Stack } from '@vpay/ui';

import { NO_ERROR, type FormAction } from '../form-state';
import { FormAlert } from './form-alert';

/**
 * The shortest password vpay accepts (`vpay_api::staff::MIN_PASSWORD_CHARS`).
 *
 * Stated here so the field can say so before a round trip, and it is a
 * *hint*: the server's own bound is the rule, and a `minLength` that drifted
 * from it would refuse something vpay accepts. There is deliberately no
 * composition rule — length is the only one, because "one digit, one symbol"
 * shrinks the space people choose from and is the reason `Password1!` exists.
 */
export const MIN_PASSWORD_CHARS = 12;

export interface PasswordFormProps {
  /** `changePassword` from `src/server/actions.ts`. */
  action: FormAction;
}

/**
 * Replacing the one-time password `vpay-server staff add` printed.
 *
 * Not a nag screen: ADR-0017 decision 1 refuses **every** authenticated route
 * to a session whose staff row still carries `password_change_required`,
 * `/oauth/authorize` included. So this is the only page such a session can
 * reach, and until it is done no `/dash/v1` token exists at all — which is
 * what stops a printed credential becoming a long-lived one by being ignored.
 *
 * There is no "current password" field, and that is vpay's design rather than
 * an omission: the session making the change has already presented both
 * factors, and re-checking a password this endpoint then overwrites would be
 * a second copy of that check in the wrong layer.
 */
export function PasswordForm({ action }: PasswordFormProps) {
  const [state, submit, pending] = useActionState(action, NO_ERROR);

  return (
    <form action={submit}>
      <Stack direction="column" gap="md">
        <Field invalid={state.error !== null}>
          <FieldLabel htmlFor="dashboard-new-password">New password</FieldLabel>
          <Input
            id="dashboard-new-password"
            name="new_password"
            type="password"
            required
            minLength={MIN_PASSWORD_CHARS}
            autoComplete="new-password"
            autoFocus
            disabled={pending}
          />
          <FieldDescription>
            At least {MIN_PASSWORD_CHARS} characters. Length is the only rule.
          </FieldDescription>
        </Field>

        <Field invalid={state.error !== null}>
          <FieldLabel htmlFor="dashboard-confirm-password">Repeat it</FieldLabel>
          <Input
            id="dashboard-confirm-password"
            name="confirm_password"
            type="password"
            required
            minLength={MIN_PASSWORD_CHARS}
            autoComplete="new-password"
            disabled={pending}
          />
        </Field>

        <FormAlert error={state.error} requestId={state.requestId} />

        <Button type="submit" block disabled={pending}>
          {pending ? 'Saving…' : 'Set password'}
        </Button>
      </Stack>
    </form>
  );
}
