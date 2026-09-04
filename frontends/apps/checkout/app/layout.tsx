import type { Metadata } from 'next';
import { headers } from 'next/headers';

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
 */
export default async function RootLayout({ children }: { children: React.ReactNode }) {
  const requestHeaders = await headers();
  const locale = pickLocale(requestHeaders.get('accept-language'));
  return (
    <html lang={locale} data-theme="corporate">
      <body className="min-h-screen bg-base-100">{children}</body>
    </html>
  );
}
