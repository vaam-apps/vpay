import { cn } from "../../cn";

export interface HeadingProps extends React.ComponentPropsWithoutRef<"h1"> {
  level?: 1 | 2 | 3;
}

const HEADING_TAGS = { 1: "h1", 2: "h2", 3: "h3" } as const;

/**
 * A screen or section heading.
 *
 * Styling only — `data-screen`, `tabIndex` and the focus-on-mount behaviour
 * a screen transition needs stay in the app (plan §4.1, `ScreenHeading`),
 * because they are screen navigation logic, not a look.
 */
export function Heading({ level = 1, className, ...rest }: HeadingProps) {
  const Tag = HEADING_TAGS[level];
  return (
    <Tag
      className={cn("text-xl font-semibold outline-none", className)}
      {...rest}
    />
  );
}
