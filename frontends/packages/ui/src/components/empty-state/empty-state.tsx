import { Heading } from "../heading";
import { Stack } from "../stack";
import { Text } from "../text";

export interface EmptyStateProps {
  /** What is empty, in three words or so. */
  title: string;
  /**
   * Why it is empty. Required, because "no results" without a reason is the
   * sentence an operator misreads as an outage — and because a zero row
   * count and a refused read must never look the same.
   */
  description: string;
  /** An optional way out — a "clear the filters" control, typically. */
  action?: React.ReactNode;
}

/**
 * A query that ran and found nothing.
 *
 * Deliberately NOT the shape for "this is not built yet" or "the read was
 * refused": those are an `Alert`, because they are conditions rather than
 * results, and an operator who cannot tell the three apart will read an
 * outage as an empty ledger. `docs/flows/dashboard.md`: "an empty table
 * means zero rows and nothing else".
 */
export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <Stack direction="column" align="center" justify="center" gap="sm">
      <Heading level={3}>{title}</Heading>
      <Text tone="muted" size="sm">
        {description}
      </Text>
      {action}
    </Stack>
  );
}
