import type { Metadata } from "next";

import "./globals.css";

export const metadata: Metadata = {
  title: "vpay dashboard",
  description: "Observability for vpay. Administration is YAML.",
};

/**
 * The document, and nothing else.
 *
 * # The nav moved, and that is Lane 4 rather than a tidy-up
 *
 * This layout rendered `PrimaryNav` for every route, signed in or not — so
 * the sign-in form carried a rail linking to a page it could not reach. The
 * rail is `AppShell`'s now, mounted by `app/(dash)/layout.tsx`, which is the
 * only group a signed-in staff member is inside. `src/nav.tsx` and its
 * hand-kept `NAV_LINKS` are deleted: `src/dash/resources.ts` is the one
 * declaration, and Refine routes from the same array the rail renders from.
 *
 * # `data-theme="dark"` stays here
 *
 * `@vaam-apps/ui` registers `dark` as its default and `light` as opt-in
 * since 0.1.2. Pinning it on `<html>` is what the app actually ships, and
 * `layout.test.tsx` asserts it; `ThemeSwitcher` in the shell writes the
 * attribute this element declares.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-theme="dark">
      <body className="min-h-screen bg-base-100">{children}</body>
    </html>
  );
}
