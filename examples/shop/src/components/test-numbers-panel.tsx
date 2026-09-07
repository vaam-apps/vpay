import { testNumbersFor, type TestNumber } from "@/lib/test-numbers";

/** One word per order status, so the table cannot render "Failed" for `unpaid`. */
const ORDER_STATUS_LABEL: Readonly<Record<TestNumber["orderStatus"], string>> =
  {
    paid: "Paid",
    failed: "Failed",
    unpaid: "Unpaid",
  };

/** The badge tone per order status the demo numbers can produce. */
const ORDER_STATUS_TONE: Readonly<Record<TestNumber["orderStatus"], string>> = {
  paid: "success",
  failed: "error",
  unpaid: "warning",
};

/**
 * The demo stack's fake numbers, on the screen where a buyer needs them.
 *
 * It is rendered from `src/lib/test-numbers.ts` and only for the rails this
 * deployment actually offers, so a shop configured for one rail does not
 * advertise the other's numbers. `test-numbers.test.ts` checks the same
 * table against `README.md` in both directions.
 *
 * This panel is honest about being a demo affordance. A real shop would ship
 * none of it — which is why it says so, in the panel, rather than only in a
 * document nobody reading the screen has open.
 */
export function TestNumbersPanel({ rails }: { rails: readonly string[] }) {
  const entries = testNumbersFor(rails);
  if (entries.length === 0) {
    return null;
  }
  return (
    <section
      className="mt-12 border-t border-base-300 pt-4"
      data-testid="test-numbers"
    >
      <h2 className="mb-1 text-lg font-semibold">Test numbers</h2>
      <p className="mb-4 text-sm text-base-content/60">
        Nothing here is a phone number and no money moves anywhere. These are
        documentation MSISDNs the demo stack&rsquo;s rail stubs are{" "}
        <em>configured</em> to answer particular things for — there is no branch
        on any of them in vpay, in this shop, or in either adapter, and against
        a real rail they do nothing at all. A real shop ships no panel like this
        one.
      </p>
      {entries.map((entry) => (
        <div key={entry.rail} className="mb-8">
          <h3 className="font-semibold">
            {entry.label} <code>{entry.rail}</code>
          </h3>
          <p className="mb-2 text-sm text-base-content/60">
            Type it {entry.where}.
          </p>
          {entry.caveat === undefined ? null : (
            <p
              role="alert"
              className="alert alert-error mb-2"
              data-testid={`test-numbers-caveat-${entry.rail}`}
            >
              <strong>Read this before you try them.</strong> {entry.caveat}
            </p>
          )}
          <div className="overflow-x-auto">
            <table className="table table-sm">
              <thead>
                <tr>
                  <th>Number</th>
                  <th>What happens</th>
                  <th>Order becomes</th>
                  <th>vpay code</th>
                  <th>The rail said</th>
                </tr>
              </thead>
              <tbody>
                {entry.numbers.map((number) => (
                  <tr
                    key={number.msisdn}
                    data-testid={`test-number-${number.msisdn}`}
                  >
                    <td>
                      <code>{number.msisdn}</code>
                    </td>
                    <td>{number.outcome}</td>
                    <td>
                      <span
                        className={`badge badge-${ORDER_STATUS_TONE[number.orderStatus]}`}
                      >
                        {ORDER_STATUS_LABEL[number.orderStatus]}
                      </span>
                    </td>
                    <td>
                      <code>{number.failureCode ?? "—"}</code>
                    </td>
                    <td>
                      <code>{number.railReason}</code>
                    </td>
                  </tr>
                ))}
                {entry.numbers
                  .filter((number) => number.note !== undefined)
                  .map((number) => (
                    <tr key={`${number.msisdn}-note`}>
                      <td colSpan={5} className="text-sm">
                        <strong>
                          Why <code>{number.msisdn}</code> leaves the order{" "}
                          <code>{number.orderStatus}</code>:
                        </strong>{" "}
                        <span data-testid={`test-number-note-${number.msisdn}`}>
                          {number.note}
                        </span>
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
          {entry.cannotExpress.length === 0 ? null : (
            <ul className="mt-2 list-disc pl-5 text-sm text-base-content/60">
              {entry.cannotExpress.map((gap) => (
                <li key={gap.outcome}>
                  <strong>
                    No number produces <code>{gap.outcome}</code> on this rail.
                  </strong>{" "}
                  {gap.why}
                </li>
              ))}
            </ul>
          )}
        </div>
      ))}
      <p className="text-sm text-base-content/60">
        <strong>Cancelled</strong> is the one outcome no number reaches — and on
        today&rsquo;s vpay nothing else reaches it either. A payer who clicks
        &ldquo;cancel&rdquo; on the rail&rsquo;s page has only navigated: the
        order stays open and the charge may still settle. The order{" "}
        <em>would</em> become <code>cancelled</code> when the shop cancels its
        PaymentIntent — the button on the order page — and vpay delivered{" "}
        <code>payment_intent.canceled</code>. <strong>It does not.</strong>{" "}
        Measured on the demo stack on 2026-09-06: the cancel really does move
        the intent, and vpay writes no event for that transition, so this shop —
        which moves an order only from a signed event — leaves it{" "}
        <code>unpaid</code>. See the README.
      </p>
    </section>
  );
}
