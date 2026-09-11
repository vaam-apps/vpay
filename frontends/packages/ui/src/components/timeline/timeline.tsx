import { List } from "../list";
import { Stack } from "../stack";
import { Text } from "../text";

export interface TimelineItem {
  /** Stable key. The event's own id, not its index. */
  id: string;
  /** Pre-formatted by the caller — this primitive has no locale opinion. */
  at: string;
  /** What happened. */
  label: string;
}

export interface TimelineProps {
  items: readonly TimelineItem[];
  /** Shown when there is nothing. Required: a blank section is not an answer. */
  emptyMessage: string;
}

/**
 * What happened, in order, one line per event.
 *
 * No icon set and no connector graphic: neither exists in this component set
 * and no consumer has asked for one, and a decorative rail down the left
 * would be a thing to maintain in exchange for nothing a reader gains.
 *
 * `emptyMessage` is required rather than defaulted. A timeline with nothing
 * in it reads as "nothing happened", and whether that is true depends
 * entirely on what the caller knows about its source — the dashboard's
 * charge timeline, for instance, is missing five of eight documented event
 * types because nothing writes them, and its own copy has to say so.
 */
export function Timeline({ items, emptyMessage }: TimelineProps) {
  if (items.length === 0) {
    return <Text tone="muted">{emptyMessage}</Text>;
  }
  return (
    <List>
      {items.map((item) => (
        <li key={item.id}>
          <Stack justify="between" gap="md">
            <Text as="span" weight="medium">
              {item.label}
            </Text>
            <Text as="span" tone="muted" size="xs">
              {item.at}
            </Text>
          </Stack>
        </li>
      ))}
    </List>
  );
}
