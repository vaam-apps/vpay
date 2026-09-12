import type { Metadata } from "next";
import { headers } from "next/headers";

import { runtimeConfig } from "../src/config/runtime";
import { THEME, themeStyleSheet } from "../src/config/theme";
import { pickLocale } from "../src/i18n/index";

import "./globals.css";

/**
 * React 19 dedupes a hoisted `<style>` by this key and never renders two.
 * It is not a URL and nothing fetches it.
 */
export const THEME_OVERRIDE_HREF = "vpay-checkout-theme-override";

/**
 * React orders hoisted styles by the order their precedences were first seen,
 * and Next's own stylesheet link is `next` and comes first — so this one lands
 * after it. Specificity would win anyway (`:root[data-theme=…]` beats
 * daisyUI's `[data-theme=…]`); the order is belt as well as braces.
 */
export const THEME_OVERRIDE_PRECEDENCE = "vpay-theme";

export const metadata: Metadata = {
  title: "vpay checkout",
  description: "Pay by mobile money.",
};

/**
 * `lang` is chosen here, server-side, from `Accept-Language` — so the very
 * first byte of HTML is tagged with a language a screen reader can act on,
 * rather than being corrected by JavaScript after the page has been
 * announced in the wrong one. The switch inside the page updates
 * `document.documentElement.lang` when a payer changes it.
 *
 * The theme is `@vaam-apps/ui`'s one registered theme, daisyUI's built-in
 * name `"dark"` (2026-09-12, the `@vaam-apps/ui` cutover — it was daisyUI's
 * own `bumblebee` before that, and `corporate` before 2026-09-05), and an
 * operator's own primary colour is applied from `branding.yaml` as a
 * `<style>` React hoists into `<head>`. That placement is the whole point
 * of doing it at runtime: the override arrives in the same response as the
 * markup, so there is no frame
 * in which a payer sees the default colour and then watches it change.
 * `themeStyleSheet` emits digits and punctuation only, from a `#rrggbb` it
 * re-validated — no value from a mounted file reaches the document as markup.
 *
 * **`href` + `precedence` rather than an explicit `<head>` element, and that
 * is a fix rather than a style.** This layout rendered `<head>{style}</head>`
 * for one revision, and a real browser threw React #418 — *hydration failed
 * because the server rendered HTML did not match the client* — on the hosted
 * payment page, uncaught, taking all three of `shop-hosted.cy.ts`'s tests
 * with it. An explicitly rendered `<head>` is an ordinary host element whose
 * children React hydrates exactly, and a Next App Router document's head is
 * full of nodes React did not render there (Next's metadata, its stylesheet
 * links, its bootstrap scripts) plus whatever a proxy, an extension or a
 * test runner has added. `href`/`precedence` is React 19's own hoisting: the
 * element is written into `<head>` on the server and reconciled as a
 * resource rather than as a child of a host element, so nothing else in the
 * head has to match. `precedence` also puts it after Next's own stylesheet.
 */
export default async function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const requestHeaders = await headers();
  const locale = pickLocale(requestHeaders.get("accept-language"));
  const { branding } = runtimeConfig();
  const themeOverride = themeStyleSheet(branding.primaryColor);
  return (
    <html lang={locale} data-theme={THEME}>
      <body className="min-h-screen bg-base-100">
        {themeOverride === null ? null : (
          // No `data-testid`: React strips every prop but its own from a
          // hoisted style, so one here would be a selector nothing could ever
          // match. `layout.test.tsx` asserts the rendered element instead.
          <style
            href={THEME_OVERRIDE_HREF}
            precedence={THEME_OVERRIDE_PRECEDENCE}
          >
            {themeOverride}
          </style>
        )}
        {children}
      </body>
    </html>
  );
}
