import { Check } from "lucide-react";
import NextLink from "next/link";

/** One chip: where it goes and whether it is the filter on screen now. */
export interface FilterChipOption {
  readonly href: string;
  readonly label: string;
  readonly active: boolean;
}

/**
 * A single-choice filter, as a row of Material 3 filter chips that are
 * links.
 *
 * # Links, not buttons
 *
 * Every list in this app keeps its filter in the URL — a filtered list an
 * operator can paste into a ticket is the better tool, and the server
 * component reads the URL to make the read. So a chip is an `<a>` to the
 * filtered URL: it works before hydration, middle-click opens the filtered
 * list in a new tab, and the server does the read the same way it does for a
 * pasted link. `aria-current="true"` marks the one on screen, which is what
 * a screen reader announces for the selected item of a set of links.
 *
 * # Two literal branches, not a computed class
 *
 * `verify-ui` refuses a `className` assembled at runtime, so the selected and
 * unselected chips are two separate elements. The selected chip is
 * `primary-container` with a check, M3's selected filter chip; the seed's
 * primary family is the one hue in this theme that is chrome and never a
 * status (the library's `AGENTS.md`), so a selected filter can never be
 * mistaken for a coloured state.
 */
export function FilterChips({
  label,
  options,
}: {
  /** The accessible name of the set, e.g. "Filter by status". */
  readonly label: string;
  readonly options: readonly FilterChipOption[];
}) {
  return (
    <nav aria-label={label}>
      <ul className="flex flex-wrap gap-2">
        {options.map((option) => (
          <li key={option.href} className="flex">
            {option.active ? (
              <NextLink
                href={option.href}
                aria-current="true"
                className="flex rounded-lg bg-primary-container"
              >
                <span className="flex h-8 items-center pr-3 pl-2 text-on-primary-container">
                  <span className="flex items-center gap-1">
                    <Check size={16} aria-hidden="true" />
                    <span className="text-label-large">{option.label}</span>
                  </span>
                </span>
              </NextLink>
            ) : (
              <NextLink
                href={option.href}
                className="flex rounded-lg hover:bg-on-surface/8"
              >
                <span className="flex h-8 rounded-lg border border-outline-variant">
                  <span className="flex items-center px-3 text-on-surface-variant">
                    <span className="text-label-large">{option.label}</span>
                  </span>
                </span>
              </NextLink>
            )}
          </li>
        ))}
      </ul>
    </nav>
  );
}
