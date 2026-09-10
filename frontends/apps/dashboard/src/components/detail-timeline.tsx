import { List, Stack, Text } from "@vpay/ui";

export interface TimelineEvent {
  id: string;
  /** Pre-formatted by the caller — this recipe has no locale opinion. */
  at: string;
  label: string;
}

export interface DetailTimelineProps {
  events: TimelineEvent[];
}

/**
 * Payment detail recipe: the event history
 * `GET /dash/v1/payment_intents/{id}` will return (`docs/flows/dashboard.md`),
 * rendered as one line per event — no icon set or connector graphic
 * invented for it, because none exists in the component set yet and this
 * recipe has no consumer that has asked for one.
 *
 * `List` supplies the vertical rhythm; `Stack` lines up the label against
 * the timestamp without a raw `flex` in this file. An empty list reads as
 * "no events yet", not as a blank section — the same "state it plainly"
 * rule the scaffold notice on `/` already follows.
 */
export function DetailTimeline({ events }: DetailTimelineProps) {
  if (events.length === 0) {
    return <Text tone="muted">No events yet.</Text>;
  }
  return (
    <List>
      {events.map((event) => (
        <li key={event.id}>
          <Stack justify="between" gap="md">
            <Text as="span" weight="medium">
              {event.label}
            </Text>
            <Text as="span" tone="muted" size="xs">
              {event.at}
            </Text>
          </Stack>
        </li>
      ))}
    </List>
  );
}
