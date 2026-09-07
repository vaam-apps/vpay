import { Alert, Heading, Stack, StatusBadge, Text } from '@vpay/ui';

/**
 * Dashboard landing page.
 *
 * Deliberately shows the scaffold state rather than a mocked payments table
 * — a screenshot of fake rows is how a project convinces itself it is
 * further along than it is (AGENTS.md). The status badges below are not
 * sample data: they are every value `PaymentStatus` can take, rendered once
 * each as a reference for the payments list and detail views slice 1 has
 * not built yet (see docs/flows/dashboard.md).
 */
export default function Home() {
  return (
    <>
      <Alert tone="warning" role="status">
        Scaffold. No data source is connected — <code>/dash/v1</code> is not
        implemented. See <code>docs/status.md</code>.
      </Alert>

      <section>
        <Heading level={2}>Payment statuses</Heading>
        <Text tone="muted" size="sm">
          Every status a payment can carry, not a list of payments — there is
          nothing yet to list.
        </Text>
        <Stack gap="sm" wrap>
          <StatusBadge status="requires_payment_method" />
          <StatusBadge status="requires_action" />
          <StatusBadge status="processing" />
          <StatusBadge status="succeeded" />
          <StatusBadge status="canceled" />
        </Stack>
      </section>
    </>
  );
}
