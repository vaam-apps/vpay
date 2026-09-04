import { headers } from 'next/headers';

import { CheckoutClient } from '../../../src/components/checkout-client';
import { pickLocale } from '../../../src/i18n/index';
import { browserApiBaseUrl } from '../../../src/lib/env';

/**
 * The hosted page, `/c/{cs_id}#{client_secret}`.
 *
 * `force-dynamic` because there is nothing here to cache and a cached
 * payment page is a payment page served to the wrong payer;
 * `middleware.ts` sends `frame-ancestors 'none'` for this path.
 *
 * The session id comes from the path; the credential comes from the
 * fragment, which never reaches this server component at all — a fragment
 * is not sent in a request. That is D6, and it is why this file passes no
 * secret to the client: there is none here to pass.
 */
export const dynamic = 'force-dynamic';

export default async function HostedCheckoutPage({
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
      mode="hosted"
      allowedOrigins={[]}
      initialLocale={pickLocale(requestHeaders.get('accept-language'))}
    />
  );
}
