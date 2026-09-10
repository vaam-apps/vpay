import Link from 'next/link';
import { notFound } from 'next/navigation';

import { Heading, Stack } from '@vpay/ui';

import { PaymentDetailView } from '../../../src/components/payment-detail';
import { ReadFailure } from '../../../src/components/read-failure';
import { SignedInBar } from '../../../src/components/signed-in-bar';
import { type PaymentDetail } from '../../../src/server/api';
import { readDash } from '../../../src/server/dash-read';
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
  const gate = await requireStaff();
  if (gate.kind === 'outage') {
    // A vpay this page cannot reach is not a sign-out — see `payments/page.tsx`.
    return (
      <Stack direction="column" gap="lg">
        <Heading level={2}>Payment</Heading>
        <ReadFailure failure={gate.failure} />
      </Stack>
    );
  }
  const staff = gate.staff;
  const { session, config } = staff;
  const { id } = await params;

  const result = await readDash<PaymentDetail>(
    config,
    `/dash/v1/payment_intents/${encodeURIComponent(id)}`,
    staff.accessToken,
    staff.sessionToken,
  );

  if (!result.ok && result.failure.status === 404) {
    notFound();
  }

  return (
    <Stack direction="column" gap="lg">
      <SignedInBar email={session.email} merchantId={session.merchant_id} signOut={signOut} />

      <Stack as="header" justify="between" align="center" gap="md" wrap>
        <Heading level={2}>Payment</Heading>
        <Link href="/payments">Back to payments</Link>
      </Stack>

      {!result.ok ? (
        <ReadFailure failure={result.failure} />
      ) : (
        <PaymentDetailView detail={result.value} />
      )}
    </Stack>
  );
}
