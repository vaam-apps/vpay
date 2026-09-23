import { Code, InlineBanner } from "@vaam-apps/ui";

/**
 * THE TIMELINE IS NOT THE HISTORY, AND SAYING SO IS THE POINT.
 *
 * `events.type` is constrained to fifteen documented types (migration
 * `0018`, extended through `0039`), and **two of them are written by
 * nothing**: `payment_intent.created` and `payment_intent.processing`. Events
 * are written for terminal transitions only, and those two are progress
 * (`docs/flows/webhooks.md`, "Which of them is written, and by what").
 *
 * _(This said "eight documented types … five of them are written by nothing"
 * until 2026-09-23, naming `payment_intent.canceled`, `charge.refunded` and
 * `charge.refund.updated` among the five. It had been wrong since 2026-09-10,
 * when `payment_intent.canceled` gained a writer; the two refund types gained
 * theirs on 2026-09-16. The test below named all five and checked only that
 * each was present, so it could not notice a type acquiring a writer. It now
 * also refuses the three that have one.)_
 *
 * So a payment's timeline never shows it being created or going to
 * `processing`. An operator
 * reading a section headed "Timeline" with one entry reasonably concludes
 * that is everything that happened to this payment — which is the same
 * failure as an empty table that means "the read was refused": a true
 * rendering of an incomplete source, presented as complete.
 *
 * The sentence is on the SCREEN and not only in the flow document, because
 * the person who needs it is reading the screen. It names the five missing
 * types rather than hedging, so the day one of them is written this line is
 * wrong in a way a grep finds. It names only those two — the types that ARE
 * written appear in the list above when they happen, and repeating them here
 * would be two elements on one screen with the same text.
 *
 * Its own component since 2026-09-11, for no reason but that this comment is
 * four times the length of what it explains and was burying the render tree
 * it sat inside.
 *
 * Wrapped in a plain `<div>` carrying `data-testid`: `InlineBanner` spreads
 * no `...rest`, so the test id has nowhere else to live (§0.2).
 */
export function TimelineGap() {
  return (
    <div data-testid="timeline-gap">
      <InlineBanner variant="plain">
        Two documented event types are written by nothing —{" "}
        <Code>payment_intent.created</Code> and{" "}
        <Code>payment_intent.processing</Code> — so this is not the whole
        history of a payment.
      </InlineBanner>
    </div>
  );
}
