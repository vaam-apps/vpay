import { ScreenStack } from "@vaam-apps/ui";
import NextLink from "next/link";

/**
 * Every path this app has no page for.
 *
 * Says what happened and offers the one way on. It used to render the
 * paired-key notice for every unknown path, which was right while no page
 * existed; with a sign-in and an account page a mistyped address is a mistyped
 * address.
 */
export function NotFoundNotice() {
  return (
    <ScreenStack>
      <h2 className="font-display text-headline-small text-foreground">
        There is no such page
      </h2>
      <p className="text-body text-muted-foreground">
        Check the address, or go back to{" "}
        <NextLink href="/" className="text-primary underline">
          the start
        </NextLink>
        .
      </p>
    </ScreenStack>
  );
}
