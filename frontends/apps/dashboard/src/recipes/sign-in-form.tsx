'use client';

import { Alert, Button, Field, FieldError, FieldLabel, Input } from '@vpay/ui';

export interface SignInFormProps {
  /** A server-side validation error, or `null` once there is none to show. */
  error?: string | null;
  onSubmit: (values: { email: string; code: string }) => void;
}

/**
 * The dashboard's sign-in recipe: an identifier plus a one-time code.
 *
 * Deliberately agnostic to which second factor exp24 lands — password,
 * TOTP, WebAuthn — `docs/flows/dashboard.md` records that as a decision not
 * yet taken. This is the layout (`Field` for the label/error association,
 * `Input` for the control, `Button` for submit), not the auth mechanism:
 * swap the field's `type`/`autoComplete` for whatever the chosen factor
 * needs and the composition stays the same.
 *
 * Uncontrolled inputs + `FormData`, the same pattern
 * `frontends/apps/checkout`'s `MsisdnForm` uses, so this recipe reads the
 * same way the rest of the product's forms do.
 */
export function SignInForm({ error = null, onSubmit }: SignInFormProps) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        const email = data.get('email');
        const code = data.get('code');
        onSubmit({
          email: typeof email === 'string' ? email : '',
          code: typeof code === 'string' ? code : '',
        });
      }}
    >
      <Field invalid={error !== null}>
        <FieldLabel htmlFor="dashboard-signin-email">Work email</FieldLabel>
        <Input
          id="dashboard-signin-email"
          name="email"
          type="email"
          required
          autoComplete="username"
        />
      </Field>
      <Field invalid={error !== null}>
        <FieldLabel htmlFor="dashboard-signin-code">One-time code</FieldLabel>
        <Input
          id="dashboard-signin-code"
          name="code"
          type="text"
          inputMode="numeric"
          autoComplete="one-time-code"
          required
        />
        <FieldError match={error !== null}>{error}</FieldError>
      </Field>
      {error !== null ? <Alert tone="error">{error}</Alert> : null}
      <Button type="submit" block>
        Sign in
      </Button>
    </form>
  );
}
