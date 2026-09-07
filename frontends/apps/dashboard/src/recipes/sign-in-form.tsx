'use client';

import { Alert, Button, Field, FieldLabel, Input, Text } from '@vpay/ui';

/** RFC 6238's default, and what `claude/exp24-staff-auth`'s ADR-0017 mints. */
const DEFAULT_CODE_LENGTH = 6;

export interface SignInFormProps {
  /** A server-side validation error, or `null` once there is none to show. */
  error?: string | null;
  /**
   * The `request-id` from the response that produced {@link error}.
   *
   * vpay emits `request-id` **and** `x-request-id` with one value on every
   * response (`docs/flows/stripe-sdk-compat.md`), and `vpay-api`'s error
   * envelope deliberately carries no `request_id` field because the header
   * and the tracing span already do (`backends/crates/vpay-api/src/error.rs`).
   * A staff member who cannot sign in has nothing else to quote to an
   * operator, so the alert renders it when there is one.
   */
  requestId?: string | null;
  /**
   * True while the caller's submit is in flight.
   *
   * Not cosmetic. The second factor this form feeds is verified behind a
   * compare-and-swap replay guard, so the *second* of two submissions of
   * one code is refused on purpose — a double-click would surface that
   * refusal as "invalid code" to someone who typed a perfectly good one.
   * Every control is disabled and the submit is a no-op while this is set.
   */
  pending?: boolean;
  /** Digits in the one-time code. Six unless a deployment says otherwise. */
  codeLength?: number;
  onSubmit: (values: { email: string; code: string }) => void;
}

/**
 * The dashboard's sign-in recipe: an identifier plus a one-time code.
 *
 * Deliberately agnostic to which second factor lands — `docs/flows/dashboard.md`
 * records that as a decision not yet taken on this branch. This is the
 * layout (`Field` for the label/error association, `Input` for the control,
 * `Button` for submit), not the auth mechanism: swap the field's
 * `type`/`autoComplete` for whatever the chosen factor needs and the
 * composition stays the same.
 *
 * **What this recipe is NOT**, so nobody builds a real login on it by
 * assumption (`docs/plans/exp26-notes/lane-d-review.md`, finding 4): it is
 * **one leg**. `claude/exp24-staff-auth`'s ADR-0017 chose a two-leg flow —
 * `POST /dash/v1/staff/login` with an argon2id password, then
 * `POST /dash/v1/staff/totp` with the code — and this form has no password
 * control, because adding one here would import a decision this branch's own
 * `docs/flows/dashboard.md` still records as open. A second leg is another
 * `Field` + `FieldLabel` + `Input type="password"` in this same shape; what
 * the review added instead are the controls that leg needs whichever factor
 * wins: `pending`, `requestId` and `codeLength`.
 *
 * Uncontrolled inputs + `FormData`, the same pattern
 * `frontends/apps/checkout`'s `MsisdnForm` uses, so this recipe reads the
 * same way the rest of the product's forms do.
 */
export function SignInForm({
  error = null,
  requestId = null,
  pending = false,
  codeLength = DEFAULT_CODE_LENGTH,
  onSubmit,
}: SignInFormProps) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        // Belt as well as braces: `disabled` stops the click, this stops a
        // submit the browser raises some other way (Enter in a field that a
        // caller re-enabled, a programmatic requestSubmit).
        if (pending) {
          return;
        }
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
          disabled={pending}
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
          maxLength={codeLength}
          pattern={`[0-9]{${codeLength}}`}
          required
          disabled={pending}
        />
      </Field>
      {error === null ? null : (
        // Once, here, and nowhere else. An earlier draft also rendered the
        // same string through `FieldError`, so every failed sign-in printed
        // it twice (review finding 3, measured).
        <Alert tone="error">
          {error}
          {requestId === null ? null : (
            <Text as="span" size="xs" tone="muted">
              Request <code>{requestId}</code>
            </Text>
          )}
        </Alert>
      )}
      <Button type="submit" block disabled={pending}>
        {pending ? 'Signing in…' : 'Sign in'}
      </Button>
    </form>
  );
}
