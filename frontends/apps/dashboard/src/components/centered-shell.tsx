import { Card } from "@vaam-apps/ui";

import { Brand } from "./brand";
import { ThemeToggle } from "./theme-toggle";

/**
 * The shell every page of this app has until the read pages arrive: the brand,
 * one card, and the theme shortcut in the corner. Centred, no rail.
 *
 * # One card on the page ground
 *
 * The mark above, the content in a `Card` — the diagnostic register, a hairline
 * on a surface step — and the theme shortcut in the corner, so an operator who
 * prefers the light theme does not have to sign in first to get it. The
 * document's `<h1>` is the brand, `sr-only`; each page's own title is an
 * `<h2>`, which the end-to-end suite finds every step by.
 *
 * The `Card` is flat: nothing in this app asks `@vaam-apps/ui` for an
 * `elevated` variant or a `shadow-*` utility (`no-shadows.test.ts`).
 *
 * Shared by the login pages' layout, the account page and the not-found page,
 * so the three cannot drift into three shells.
 */
export function CenteredShell({ children }: { children: React.ReactNode }) {
  return (
    <main className="relative grid min-h-dvh place-items-center p-4">
      <span className="absolute top-4 right-4">
        <ThemeToggle />
      </span>
      <div className="flex w-full max-w-lg flex-col gap-6 py-12">
        <header className="flex justify-center">
          <h1 className="sr-only">vpay dashboard</h1>
          <Brand />
        </header>
        <Card>
          <div className="flex flex-col gap-6 p-6 sm:p-8">{children}</div>
        </Card>
      </div>
    </main>
  );
}
