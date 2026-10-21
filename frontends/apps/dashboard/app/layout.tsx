import "@fontsource-variable/roboto-flex/standard.css";
import "@fontsource-variable/jetbrains-mono";
import { themeInitScript } from "@vaam-apps/ui";
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
 *
 * # The theme is applied before the first paint (2026-10-21)
 *
 * `themeInitScript` — a fixed string the package exports, nothing
 * interpolated — reads the operator's stored choice (System, Light, Dark)
 * and sets `data-theme` before the body paints. Without it a light-theme
 * operator saw every document load paint dark and then flip, and that flash
 * read as the page reloading. `suppressHydrationWarning` is for exactly
 * that one attribute, which the script may change before React hydrates.
 *
 * # The fonts are self-hosted
 *
 * `theme.css` names Roboto Flex and JetBrains Mono and ships neither; until
 * 2026-10-21 every glyph fell back to the system UI font and to the
 * system monospace. `@fontsource-variable` bundles both into this app's
 * own assets, so no third-party font host is contacted.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-theme="dark" suppressHydrationWarning>
      <head>
        <script
          // A fixed literal exported by the package, nothing interpolated.
          dangerouslySetInnerHTML={{ __html: themeInitScript }}
        />
      </head>
      <body className="min-h-dvh bg-base-100 text-foreground antialiased">
        {children}
      </body>
    </html>
  );
}
