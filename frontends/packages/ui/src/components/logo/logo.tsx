import { cn } from "../../cn";

export interface LogoProps extends Omit<
  React.ComponentPropsWithoutRef<"img">,
  "src" | "alt"
> {
  src: string;
  /** Required: the operator's mark is often the only place their name appears. */
  alt: string;
}

/**
 * An operator's mark, at the one height this product renders it.
 *
 * `h-8 w-auto` is load-bearing rather than decorative: Tailwind's own
 * preflight resets `img { height: auto }`, so without an explicit height an
 * operator's logo renders at its intrinsic size, whatever that is. Owning it
 * here means the app does not write a class to get a predictable header.
 *
 * `next/image` is deliberately not used by the consumer: the checkout app is
 * `output: 'standalone'` with no image-optimisation loader, and the URL is an
 * operator's own absolute one.
 */
export function Logo({ className, ...rest }: LogoProps) {
  // `alt` is a REQUIRED prop on `LogoProps`, so a caller cannot omit it; it
  // arrives through the spread rather than being written out here.
  return <img className={cn("h-8 w-auto", className)} {...rest} />;
}
