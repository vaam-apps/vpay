import { Alert, Heading, Stack, Text } from '@vpay/ui';

import { EmptyState } from '../../src/components/empty-state';
import { Pager } from '../../src/components/pager';
import { PaymentsFilters } from '../../src/components/payments-filters';
import { PaymentsTable } from '../../src/components/payments-table';
import { SignedInBar } from '../../src/components/signed-in-bar';
import { apiQueryString, pagerHrefs, queryFrom } from '../../src/payments-query';
import { getJson, type PaymentIntentList } from '../../src/server/api';
import { signOut } from '../../src/server/actions';
import { requireStaff } from '../../src/server/session';

/**
 * `/payments` — the merchant's payment intents, newest first.
 *
 * Server-rendered: the `/dash/v1` bearer token is read out of the
 * `staff_sessions` row on this request and used on this request, and no
 * browser ever holds it (ADR-0008, ADR-0017 decision 4).
 *
 * **The token is re-read on every render**, which is the whole mechanism
 * behind sign-out: deleting the session row removes the only place this
 * server can obtain it from. A token cached in this process would keep
 * rendering payments for a signed-out session until it expired.
 *
 * # An empty table means zero rows and nothing else
 *
 * A failed read renders the failure and the request id, never an empty list:
 * "this merchant has no payments" and "the read was refused" are different
 * answers, and the empty list is the one an operator would believe.
 */
export const dynamic = 'force-dynamic';

export default async function PaymentsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { session, accessToken, config } = await requireStaff();
  const query = queryFrom(await searchParams);

  const result = await getJson<PaymentIntentList>(
    config.apiBaseUrl,
    `/dash/v1/payment_intents?${apiQueryString(query)}`,
    { bearer: accessToken },
  );

  return (
    <Stack direction="column" gap="lg">
      <SignedInBar email={session.email} merchantId={session.merchant_id} signOut={signOut} />

      <Heading level={2}>Payments</Heading>

      <PaymentsFilters
        values={{
          status: query.status,
          createdFrom: query.createdFrom,
          createdTo: query.createdTo,
        }}
      />

      {!result.ok ? (
        <Alert tone="error">
          <Text as="span">{result.failure.message}</Text>
          {result.failure.requestId === null ? null : (
            <Text as="span" size="xs" tone="muted">
              Request <code>{result.failure.requestId}</code>
            </Text>
          )}
        </Alert>
      ) : result.value.data.length === 0 ? (
        <EmptyState
          title="No payments"
          description={
            query.status.length > 0 || query.createdFrom.length > 0 || query.createdTo.length > 0
              ? 'No payment matched these filters. Widen the range or clear the status.'
              : 'This merchant has no payment intents yet.'
          }
        />
      ) : (
        <>
          <PaymentsTable rows={result.value.data} />
          <Pager {...pagerHrefs(query, result.value.data, result.value.has_more)} />
        </>
      )}
    </Stack>
  );
}
