import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { code } from "./code.variants";

export interface CodeProps
  extends React.ComponentPropsWithoutRef<"code">, VariantProps<typeof code> {}

/**
 * An identifier a person is expected to copy into a ticket or a log query.
 *
 * No Base UI primitive — a `<code>`, which is the element that already means
 * "this is a machine token, read it literally". It is a component rather
 * than a bare tag because the dashboard renders one in eighteen places and
 * every one of them was an unstyled default before: the same id looked like
 * body copy in a table cell and like body copy again in an alert, with
 * nothing marking it as something to select whole.
 */
export function Code({ wrap, className, ...rest }: CodeProps) {
  return <code className={cn(code({ wrap }), className)} {...rest} />;
}
