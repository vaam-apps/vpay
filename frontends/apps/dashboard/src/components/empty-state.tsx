import { Heading, Stack, Text } from "@vpay/ui";

export interface EmptyStateProps {
  title: string;
  description: string;
}

/**
 * The "nothing matched" recipe — a filtered payments list with zero rows,
 * for instance. Not the scaffold's "no data source" notice: that stays the
 * `Alert` on `/`, which is a different message (nothing is wired up yet,
 * versus a working query that found nothing).
 *
 * Centred composition only: `Stack`'s own `align`/`justify` variants, no
 * bespoke centering utility written in this file.
 */
export function EmptyState({ title, description }: EmptyStateProps) {
  return (
    <Stack direction="column" align="center" justify="center" gap="sm">
      <Heading level={3}>{title}</Heading>
      <Text tone="muted" size="sm">
        {description}
      </Text>
    </Stack>
  );
}
