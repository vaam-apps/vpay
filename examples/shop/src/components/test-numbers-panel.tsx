import { testNumbersFor } from "@/lib/test-numbers";

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
    <section className="test-numbers" data-testid="test-numbers">
      <h2>Test numbers</h2>
      <p style={{ color: "var(--muted)" }}>
        Nothing here is a phone number and no money moves anywhere. These are
        documentation MSISDNs the demo stack&rsquo;s rail stubs are{" "}
        <em>configured</em> to answer particular things for — there is no branch
        on any of them in vpay, in this shop, or in either adapter, and against
        a real rail they do nothing at all. A real shop ships no panel like this
        one.
      </p>
      {entries.map((entry) => (
        <div key={entry.rail}>
          <h3>
            {entry.label} <code>{entry.rail}</code>
          </h3>
          <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
            Type it {entry.where}.
          </p>
          {entry.caveat === undefined ? null : (
            <p
              className="error"
              role="alert"
              data-testid={`test-numbers-caveat-${entry.rail}`}
            >
              <strong>Read this before you try them.</strong> {entry.caveat}
            </p>
          )}
          <table>
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
                    <span className={`status status-${number.orderStatus}`}>
                      {number.orderStatus === "paid" ? "Paid" : "Failed"}
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
            </tbody>
          </table>
          {entry.cannotExpress.length === 0 ? null : (
            <ul style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
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
      <p style={{ color: "var(--muted)", fontSize: "0.9rem" }}>
        <strong>Cancelled</strong> is the one outcome no number reaches. A payer
        who clicks &ldquo;cancel&rdquo; on the rail&rsquo;s page has only
        navigated: the order stays open and the charge may still settle. The
        order becomes <code>cancelled</code> when the shop cancels its
        PaymentIntent — the button on the order page — and vpay delivers{" "}
        <code>payment_intent.canceled</code>.
      </p>
    </section>
  );
}
