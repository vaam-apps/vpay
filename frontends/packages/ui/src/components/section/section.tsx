import { Heading } from "../heading";
import { Stack } from "../stack";

export interface SectionProps extends Omit<
  React.ComponentPropsWithoutRef<"section">,
  "title"
> {
  /**
   * The section's heading. A `string` rather than a `ReactNode`, because it
   * is used twice — as the heading you can see and as the region's
   * accessible name — and the two must be the same words.
   */
  title: string;
  /** Heading level. Default 2 — a section under the page's own `<h1>`. */
  level?: 1 | 2 | 3;
}

/**
 * A titled region of a page.
 *
 * A `<section>` and its heading, together, and the section carries the
 * heading's own words as its accessible name — because that name is the
 * whole difference between a landmark and a `<div>`: a `<section>` with no
 * accessible NAME is not a `region` in the accessibility tree at all, and a
 * heading merely sitting inside it does not give it one. The dashboard
 * rendered eleven bare `<section>` elements, every one of them nameless.
 *
 * `aria-label` rather than `aria-labelledby` off a generated id, and the
 * reason is that `useId` would make this a client component: it is rendered
 * by `payment-detail.tsx`, a Server Component, and a `"use client"` here
 * would put a boundary around a presentational wrapper and ship its
 * JavaScript to the browser. The usual objection to `aria-label` — invisible
 * text that drifts from the visible words — cannot happen here, because both
 * come from this one prop.
 */
export function Section({ title, level = 2, children, ...rest }: SectionProps) {
  return (
    <Stack
      as="section"
      direction="column"
      align="stretch"
      gap="sm"
      aria-label={title}
      {...rest}
    >
      <Heading level={level}>{title}</Heading>
      {children}
    </Stack>
  );
}
