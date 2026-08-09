import { StatusBadge } from '@vpay/ui';

/**
 * Dashboard landing page.
 *
 * Deliberately shows the scaffold state rather than a mocked payments table.
 * A screenshot of fake data is how a project convinces itself it is further
 * along than it is.
 */
export default function Home() {
  return (
    <main className="mx-auto max-w-3xl p-8">
      <h1 className="text-2xl font-semibold">vpay dashboard</h1>

      <div className="alert alert-warning mt-6" role="status">
        <span>
          Scaffold. No data source is connected — <code>/dash/v1</code> is not
          implemented. See <code>docs/STATUS.md</code>.
        </span>
      </div>

      <section className="mt-8">
        <h2 className="mb-3 text-lg font-medium">Design system smoke test</h2>
        <div className="flex flex-wrap gap-2">
          <StatusBadge status="requires_payment_method" />
          <StatusBadge status="requires_action" />
          <StatusBadge status="processing" />
          <StatusBadge status="succeeded" />
          <StatusBadge status="canceled" />
        </div>
      </section>
    </main>
  );
}
