import { headers } from 'next/headers';

import { ReturnClient } from '../../../../src/components/return-client';
import { pickLocale } from '../../../../src/i18n/index';
import { browserApiBaseUrl } from '../../../../src/lib/env';

/**
 * The return page, `/c/{cs_id}/return?t={return_token}`.
 *
 * Top-level in both modes: a rail redirects the whole browsing context, not
 * a frame, and `middleware.ts` sends `frame-ancestors 'none'` for every
 * `/c/` path, this one included.
 *
 * The `return_token` is in the **query**, not the fragment, because a
 * fragment does not survive a rail's redirect. It is not the intent's
 * secret: it authorises reading this session and polling its intent, and
 * the route it authorises renders the intent without a secret, so there is
 * nothing on this page that could confirm a payment.
 */
export const dynamic = 'force-dynamic';

export default async function CheckoutReturnPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const requestHeaders = await headers();
  return (
    <ReturnClient
      sessionId={id}
      apiBaseUrl={browserApiBaseUrl()}
      initialLocale={pickLocale(requestHeaders.get('accept-language'))}
    />
  );
}
