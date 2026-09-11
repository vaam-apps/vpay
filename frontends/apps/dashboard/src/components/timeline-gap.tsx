import { Code, Text } from "@vpay/ui";

/**
 * THE TIMELINE IS NOT THE HISTORY, AND SAYING SO IS THE POINT.
 *
 * `events.type` is constrained to eight documented types (migration `0018`,
 * extended by `0029`), and **five of them are written by nothing at all**
 * (`docs/status.md`, "Events written by the worker"): settlement writes two
 * and the housekeeping sweep writes one, and that is the whole of it.
 *
 * So a succeeded payment's timeline has exactly one line on it. An operator
 * reading a section headed "Timeline" with one entry reasonably concludes
 * that is everything that happened to this payment — which is the same
 * failure as an empty table that means "the read was refused": a true
 * rendering of an incomplete source, presented as complete.
 *
 * The sentence is on the SCREEN and not only in the flow document, because
 * the person who needs it is reading the screen. It names the five missing
 * types rather than hedging, so the day one of them is written this line is
 * wrong in a way a grep finds. It names only those five — the three that ARE
 * written appear in the list above when they happen, and repeating them here
 * would be two elements on one screen with the same text.
 *
 * Its own component since 2026-09-11, for no reason but that this comment is
 * four times the length of what it explains and was burying the render tree
 * it sat inside.
 */
export function TimelineGap() {
  return (
    <Text tone="muted" size="xs" data-testid="timeline-gap">
      Five of the eight documented event types are written by nothing —{" "}
      <Code>payment_intent.created</Code>,{" "}
      <Code>payment_intent.processing</Code>,{" "}
      <Code>payment_intent.canceled</Code>, <Code>charge.refunded</Code> and{" "}
      <Code>charge.refund.updated</Code> — so this is not the whole history of a
      payment.
    </Text>
  );
}
