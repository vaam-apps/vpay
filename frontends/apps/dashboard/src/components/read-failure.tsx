import { Code, InlineBanner } from "@vaam-apps/ui";

import type { ApiFailure } from "../server/api";

/** What {@link ReadFailure} renders. */
export interface ReadFailureProps {
  /** The refusal, as `server/api.ts` shaped it. */
  failure: ApiFailure;
}

/**
 * The **one** place a failed read of vpay is shown on a page.
 *
 * `FormAlert`'s argument, for the other half of this app: a sentence and the
 * request id beside it, because vpay answers one message for a whole class of
 * refusal and the id is the only thing that distinguishes two of them to an
 * operator reading the log. It is a separate component from `FormAlert`
 * because that one takes a `string | null` out of a `FormState` and this one
 * takes an {@link ApiFailure}; collapsing them would mean every page
 * destructuring a failure to hand over two of its three fields.
 *
 * A **server** component: it renders inside a page's own render, and the
 * failures it shows are ones that happened server-side.
 *
 * # What it deliberately does not do
 *
 * It does not decide anything. Whether a refusal ends the session is
 * `server/gate.ts`'s `refusalFor`, and it is a pure function with a unit test
 * precisely so that "a `503` signs everybody out" is a red test rather than a
 * thing somebody notices during an incident.
 *
 * The `role="alert"` lives on a plain wrapper `<div>` rather than on
 * `InlineBanner` itself (2026-09-12, `@vaam-apps/ui` cutover): `InlineBanner`
 * accepts no `role` and forwards no attribute it does not declare, so this is
 * the only place left to put it, and both `dashboard.cy.ts`'s `[role="alert"]`
 * assertions and the axe suite depend on it resolving.
 */
export function ReadFailure({ failure }: ReadFailureProps) {
  return (
    <div role="alert">
      <InlineBanner variant="danger">
        <span>{failure.message}</span>
        {failure.requestId === null ? null : (
          <span className="block text-caption text-muted-foreground">
            Request <Code>{failure.requestId}</Code>
          </span>
        )}
      </InlineBanner>
    </div>
  );
}
