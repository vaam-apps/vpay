import Link from 'next/link';
import { notFound } from 'next/navigation';

import { Alert, Heading, Stack, Text } from '@vpay/ui';

import { PaymentDetailView } from '../../../src/components/payment-detail';
import { SignedInBar } from '../../../src/components/signed-in-bar';
import { getJson, type PaymentDetail } from '../../../src/server/api';
import { signOut } from '../../../src/server/actions';
import { requireStaff } from '../../../src/server/session';

/**
 * `/payments/{id}` — everything `/dash/v1` knows about one payment.
 *
 * A `404` from vpay is this app's `notFound()`, and that mapping is exact
 * rather than convenient: the detail read is tenant-scoped, so **another
 * merchant's id answers the same `404` a nonexistent one does**
 * (`vpay_api::dash::payment_intents::retrieve`). Rendering "you may not see
 * this" for one and "no such payment" for the other would turn this page into
 * an oracle for which ids exist in other tenants.
 */
export const dynamic = 'force-dynamic';

export default async function PaymentDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { session, accessToken, config } = await requireStaff();
  const { id } = await params;

  const result = await getJson<PaymentDetail>(
    config.apiBaseUrl,
    `/dash/v1/payment_intents/${encodeURIComponent(id)}`,
    { bearer: accessToken },
  );

  if (!result.ok && result.failure.status === 404) {
    notFound();
  }

  return (
    <Stack direction="column" gap="lg">
      <SignedInBar email={session.email} merchantId={session.merchant_id} signOut={signOut} />

      <Stack justify="between" align="center" gap="md" wrap>
        <Heading level={2}>Payment</Heading>
        <Link href="/payments">Back to payments</Link>
      </Stack>

      {!result.ok ? (
        <Alert tone="error">
          <Text as="span">{result.failure.message}</Text>
          {result.failure.requestId === null ? null : (
            <Text as="span" size="xs" tone="muted">
              Request <code>{result.failure.requestId}</code>
            </Text>
          )}
        </Alert>
      ) : (
        <PaymentDetailView detail={result.value} />
      )}
    </Stack>
  );
}
