"use client";

import { useId } from "react";

import { Heading } from "../heading";
import { Stack } from "../stack";

export interface SectionProps extends Omit<
  React.ComponentPropsWithoutRef<"section">,
  "title"
> {
  /** The section's heading text. Rendered as the `level` asked for. */
  title: React.ReactNode;
  /** Heading level. Default 2 — a section under the page's own `<h1>`. */
  level?: 1 | 2 | 3;
}

/**
 * A titled region of a page.
 *
 * A `<section>` and its heading, together, and wired to each other with
 * `aria-labelledby`, because that wiring is the whole difference between a
 * landmark and a `<div>`: a `<section>` with no accessible NAME is not a
 * `region` in the accessibility tree at all, and a heading that merely sits
 * inside it does not give it one. The dashboard rendered eleven bare
 * `<section>` elements, every one of them nameless.
 *
 * The id comes from `useId`, so two sections on a page cannot collide and
 * no caller has to invent one.
 */
export function Section({ title, level = 2, children, ...rest }: SectionProps) {
  const headingId = useId();
  return (
    <Stack
      as="section"
      direction="column"
      align="stretch"
      gap="sm"
      aria-labelledby={headingId}
      {...rest}
    >
      <Heading id={headingId} level={level}>
        {title}
      </Heading>
      {children}
    </Stack>
  );
}
