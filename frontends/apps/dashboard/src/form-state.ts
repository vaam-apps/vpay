/**
 * What a form action answers, shared by the actions and the forms.
 *
 * Its own module because `src/server/actions.ts` is a `'use server'` file,
 * and such a file may export **only async functions** — a shared constant or
 * a route path exported from there is a build error, not a lint warning.
 * Putting the type here also means the client components import no server
 * module at all to know its shape.
 */

/** One sentence to show, and the id to quote — or neither, before a submit. */
export interface FormState {
  /** The one sentence, or `null`. */
  readonly error: string | null;
  /**
   * `request-id` from the refusal.
   *
   * Every refusal on the sign-in path is the same sentence by design
   * (`docs/flows/dashboard-auth.md`, "Every refusal is one answer"), so this
   * is the only thing that distinguishes one from another for an operator
   * reading the log.
   */
  readonly requestId: string | null;
}

/** The state a form starts in: no submit has happened. */
export const NO_ERROR: FormState = { error: null, requestId: null };

/** A `useActionState` action: previous state plus the submitted form. */
export type FormAction = (
  previous: FormState,
  form: FormData,
) => Promise<FormState>;
