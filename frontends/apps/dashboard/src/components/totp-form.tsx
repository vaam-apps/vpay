"use client";

import { useActionState } from "react";

import { Button, Field, FieldLabel, Input, Stack } from "@vpay/ui";

import { NO_ERROR, type FormAction } from "../form-state";
import { FormAlert } from "./form-alert";

/** RFC 6238's digit count, and what the field is constrained to. */
export const CODE_LENGTH = 6;

export interface TotpFormProps {
  /** `submitTotp` from `src/server/actions.ts`. */
  action: FormAction;
  /**
   * Whether this is a first sign-in. Changes only the button and the
   * autocomplete hint — the field, the action and the request are the same,
   * because vpay decides which secret authenticates and this form never
   * carries one.
   */
  enrolling?: boolean;
}

/**
 * Leg two: the six digits.
 *
 * `pending` is load-bearing here in a way it is not on any other form in this
 * app. The code is verified behind a compare-and-swap replay guard
 * (`Staff::record_totp_step` admits only a strictly greater time step), so
 * the **second** submission of one code is refused on purpose — a
 * double-click would show "we could not sign you in" to somebody who typed a
 * perfectly good code. Without this prop a double click called the action
 * twice; that was measured on the recipe this form grew out of
 * (`docs/plans/exp26-notes/lane-d-review.md`, finding 4).
 *
 * `maxLength` and `pattern` constrain the field to six digits and
 * `inputMode="numeric"` gets a phone keypad; `autoComplete="one-time-code"`
 * is what lets a platform offer the code it just saw in a notification.
 */
export function TotpForm({ action, enrolling = false }: TotpFormProps) {
  const [state, submit, pending] = useActionState(action, NO_ERROR);

  return (
    <form action={submit}>
      <Stack direction="column" gap="md">
        <Field invalid={state.error !== null}>
          <FieldLabel htmlFor="dashboard-totp-code">One-time code</FieldLabel>
          <Input
            id="dashboard-totp-code"
            name="code"
            type="text"
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={CODE_LENGTH}
            pattern={`[0-9]{${CODE_LENGTH}}`}
            required
            autoFocus
            disabled={pending}
          />
        </Field>

        <FormAlert error={state.error} requestId={state.requestId} />

        <Button type="submit" block disabled={pending}>
          {pending
            ? "Checking…"
            : enrolling
              ? "Confirm and finish enrolment"
              : "Continue"}
        </Button>
      </Stack>
    </form>
  );
}
