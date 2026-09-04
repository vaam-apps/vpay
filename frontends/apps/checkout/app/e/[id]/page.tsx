import { headers } from 'next/headers';

import { CheckoutClient } from '../../../src/components/checkout-client';
import { pickLocale } from '../../../src/i18n/index';
import { EMBED_ORIGINS_HEADER, decodeOriginsHeader } from '../../../src/lib/csp';
import { browserApiBaseUrl } from '../../../src/lib/env';

/**
 * The embedded page, `/e/{cs_id}?key={pk}#{client_secret}`.
 *
 * The origin list is **not** looked up here. `middleware.ts` already did it
 * to build the CSP, and it forwards the result on
 * {@link EMBED_ORIGINS_HEADER}; looking it up a second time could produce a
 * different answer from the one the browser is enforcing, and the page's
 * `postMessage` target must be a member of the list the browser was given.
 *
 * An empty list reaches the client as an empty list, and
 * `decideEntry` refuses. That is the fail-closed path for an unknown key, a
 * lookup that failed and a merchant with no registered origins alike.
 */
export const dynamic = 'force-dynamic';

export default async function EmbeddedCheckoutPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const requestHeaders = await headers();
  return (
    <CheckoutClient
      sessionId={id}
      apiBaseUrl={browserApiBaseUrl()}
      mode="embedded"
      allowedOrigins={decodeOriginsHeader(requestHeaders.get(EMBED_ORIGINS_HEADER))}
      initialLocale={pickLocale(requestHeaders.get('accept-language'))}
    />
  );
}
