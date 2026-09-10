import { Alert, Text } from "@vpay/ui";

/** What {@link FormAlert} renders, or nothing. */
export interface FormAlertProps {
  /** The one sentence, or `null` for no alert at all. */
  error: string | null;
  /** `request-id` from the refusal, if the response carried one. */
  requestId?: string | null;
}

/**
 * The **one** place a form failure is shown.
 *
 * One place, and that is the whole reason this is a component: the first
 * draft of the sign-in recipe rendered the same sentence through `FieldError`
 * *and* through `Alert`, so every failed sign-in printed it twice
 * (`docs/plans/exp26-notes/lane-d-review.md`). A genuinely field-level error
 * is what `FieldError match={…}` is for; every refusal on this path is
 * form-level, because vpay answers one sentence for the whole credential
 * (`docs/flows/dashboard-auth.md`, "Every refusal is one answer").
 *
 * The request id is rendered beside it because it is the only thing that
 * distinguishes one refusal from another: vpay deliberately answers "no such
 * address", "wrong password", "disabled account", "wrong code" and "replayed
 * code" identically, so a staff member who cannot sign in has nothing else to
 * quote to an operator who can read the log.
 */
export function FormAlert({ error, requestId = null }: FormAlertProps) {
  if (error === null) {
    return null;
  }
  return (
    <Alert tone="error">
      <Text as="span">{error}</Text>
      {requestId === null ? null : (
        <Text as="span" size="xs" tone="muted">
          Request <code>{requestId}</code>
        </Text>
      )}
    </Alert>
  );
}
