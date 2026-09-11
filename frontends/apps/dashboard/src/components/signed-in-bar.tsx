import { Button, Code, Stack, Text } from "@vpay/ui";

export interface SignedInBarProps {
  /** The address this session signed in with. */
  email: string;
  /** The one tenant this session may read — the dashboard client's binding. */
  merchantId: string;
  /** `signOut` from `src/server/actions.ts`. */
  signOut: () => Promise<void>;
}

/**
 * Who is signed in, which tenant they are reading, and the way out.
 *
 * The merchant is on screen because it is not a preference: `/dash/v1` reads
 * exactly one tenant, fixed by the dashboard client's registration and by
 * nothing in any token (`vpay_api::require_dashboard_token` authorises by
 * registration). An operator looking at an empty payments list needs to be
 * able to tell "this merchant has no payments" from "I am looking at the
 * wrong merchant", and no other element on the page answers that.
 *
 * Sign-out is a `<form>` POST rather than a link, so it works with no
 * JavaScript and so it cannot be triggered by a `GET` — a sign-out link is
 * something a prefetcher, a link scanner or an `<img src>` in a chat message
 * can fire.
 *
 * **Not in the root layout.** The layout is rendered by `/login` too, and a
 * "Sign out" control on a page where nobody is signed in is the same kind of
 * claim as a nav entry for a page nobody wrote.
 */
export function SignedInBar({ email, merchantId, signOut }: SignedInBarProps) {
  return (
    <Stack justify="between" align="center" gap="md" wrap>
      <Text as="span" size="sm" tone="muted">
        Signed in as <strong>{email}</strong> · merchant{" "}
        <Code>{merchantId}</Code>
      </Text>
      <form action={signOut}>
        <Button type="submit" variant="ghost" size="sm">
          Sign out
        </Button>
      </form>
    </Stack>
  );
}
