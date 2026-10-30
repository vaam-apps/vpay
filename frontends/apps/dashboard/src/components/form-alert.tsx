import { Code, InlineBanner } from "@vaam-apps/ui";

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
 *
 * The `role="alert"` lives on a plain wrapper `<div>` rather than on
 * `InlineBanner` itself (2026-09-12, `@vaam-apps/ui` cutover): `InlineBanner`
 * accepts no `role` and forwards no attribute it does not declare, so this is
 * the only place left to put it, and `sign-in-form.test.tsx`'s
 * `findByRole("alert")` and `dashboard.cy.ts`'s `[role="alert"]` both depend
 * on it resolving.
 */
export function FormAlert({ error, requestId = null }: FormAlertProps) {
  if (error === null) {
    return null;
  }
  return (
    <div role="alert">
      <InlineBanner variant="danger">
        <span>{error}</span>
        {requestId === null ? null : (
          <span className="block text-caption text-muted-foreground">
            Request <Code>{requestId}</Code>
          </span>
        )}
      </InlineBanner>
    </div>
  );
}
