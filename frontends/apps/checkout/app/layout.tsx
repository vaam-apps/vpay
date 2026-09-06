import type { Metadata } from 'next';
import { headers } from 'next/headers';

import { runtimeConfig } from '../src/config/runtime';
import { THEME, themeStyleSheet } from '../src/config/theme';
import { pickLocale } from '../src/i18n/index';

import './globals.css';

export const metadata: Metadata = {
  title: 'vpay checkout',
  description: 'Pay by mobile money.',
};

/**
 * `lang` is chosen here, server-side, from `Accept-Language` — so the very
 * first byte of HTML is tagged with a language a screen reader can act on,
 * rather than being corrected by JavaScript after the page has been
 * announced in the wrong one. The switch inside the page updates
 * `document.documentElement.lang` when a payer changes it.
 *
 * The theme is daisyUI's **bumblebee** (the maintainer's requirement,
 * 2026-09-05; it was `corporate` until then), and an operator's own primary
 * colour is applied **here**, in `<head>`, from `branding.yaml`. That
 * placement is the whole point of doing it at runtime: the override arrives
 * in the same response as the markup, so there is no frame in which a payer
 * sees the default colour and then watches it change. `themeStyleSheet`
 * emits digits and punctuation only, from a `#rrggbb` it re-validated — no
 * value from a mounted file reaches the document as markup.
 */
export default async function RootLayout({ children }: { children: React.ReactNode }) {
  const requestHeaders = await headers();
  const locale = pickLocale(requestHeaders.get('accept-language'));
  const { branding } = runtimeConfig();
  const themeOverride = themeStyleSheet(branding.primaryColor);
  return (
    <html lang={locale} data-theme={THEME}>
      <head>
        {themeOverride === null ? null : (
          <style data-testid="theme-override">{themeOverride}</style>
        )}
      </head>
      <body className="min-h-screen bg-base-100">{children}</body>
    </html>
  );
}
