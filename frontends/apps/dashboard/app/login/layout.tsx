import { Card } from "@vaam-apps/ui";

import { Brand } from "../../src/components/brand";
import { ThemeToggle } from "../../src/components/theme-toggle";

/**
 * The signed-out shell: centred, no rail.
 *
 * The root layout used to wrap every route in this container and render the
 * nav inside it, which put a link to `/payments` on the sign-in form — a
 * page nobody at that point could open. Signed-out routes get their own
 * wrapper instead, and the signed-in rail belongs to `app/(dash)`.
 *
 * # One card on the page ground (2026-10-21)
 *
 * The mark above, the form in a `Card` — the diagnostic register, a hairline
 * on a surface step — and the theme shortcut in the corner, so an operator
 * who prefers the light theme does not have to sign in first to get it.
 * The document's `<h1>` is the brand, `sr-only` as in the signed-in shell;
 * each step's own title stays an `<h2>`, which `dashboard.cy.ts` finds
 * every step by.
 */
export default function LoginLayout({
  children,
}: {
  children: React.ReactNode;
}) {
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
