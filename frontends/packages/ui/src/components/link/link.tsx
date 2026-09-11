import { useRender } from "@base-ui/react/use-render";
import { type VariantProps } from "class-variance-authority";

import { cn } from "../../cn";
import { link } from "./link.variants";

export interface LinkProps
  extends useRender.ComponentProps<"a">, VariantProps<typeof link> {}

/**
 * Navigation to another page.
 *
 * This package cannot depend on `next/link` — it is consumed by two Next
 * apps and by Storybook, and only two of those three have a router — so the
 * element is supplied by the caller through Base UI's `render` prop:
 * `<Link render={<NextLink href="/payments" />}>Payments</Link>`. Base UI's
 * `useRender` merges the class names rather than replacing them, so the
 * caller's element keeps its own props and gains this one's look.
 *
 * It exists because five links across the dashboard rendered as bare,
 * unstyled anchors: a link in a table cell was indistinguishable from the
 * text beside it except by hovering, which is not a thing a keyboard or a
 * touch screen does.
 */
export function Link({ tone, className, render, ...rest }: LinkProps) {
  return useRender({
    render,
    defaultTagName: "a",
    props: { ...rest, className: cn(link({ tone }), className) },
  });
}
