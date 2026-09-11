import { Link } from "../link";
import { Stack } from "../stack";
import { Text } from "../text";

export interface PaginationProps {
  /** The element that navigates back, or `null` on the first page. */
  previous: React.ReactElement | null;
  /** The element that navigates on, or `null` when there is no next page. */
  next: React.ReactElement | null;
  /** What to say instead of a "Previous" control on the first page. */
  previousEnd?: string;
  /** What to say instead of a "Next" control on the last page. */
  nextEnd?: string;
  /** Names the `<nav>`. Two pagers on one page need two names. */
  label: string;
}

/**
 * Cursor paging, as two controls.
 *
 * Cursors and not page numbers, because that is what `/dash/v1` serves:
 * `starting_after` and `ending_before` name a row and `has_more` says
 * whether another page exists — there is no count anywhere, so "page 3 of
 * 12" would mean either a `COUNT(*)` this API does not offer or a number
 * this component made up.
 *
 * The two controls are elements the caller supplies, for the same reason
 * {@link Link} takes a `render`: the router is the app's, not this
 * package's. Each is wrapped in a `Link` so both read as links wherever they
 * come from.
 *
 * An absent control is a sentence, never a disabled-looking one that does
 * nothing.
 */
export function Pagination({
  previous,
  next,
  previousEnd = "Newest first",
  nextEnd = "End of results",
  label,
}: PaginationProps) {
  if (previous === null && next === null) {
    return null;
  }
  return (
    <Stack as="nav" justify="between" gap="md" aria-label={label}>
      {previous === null ? (
        <Text as="span" tone="muted" size="sm">
          {previousEnd}
        </Text>
      ) : (
        <Link render={previous} />
      )}
      {next === null ? (
        <Text as="span" tone="muted" size="sm">
          {nextEnd}
        </Text>
      ) : (
        <Link render={next} />
      )}
    </Stack>
  );
}
