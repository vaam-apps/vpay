# SDKs

| Doc | What it covers |
|---|---|
| [parity.md](parity.md) | The cross-SDK capability matrix — record: [ADR-0015](../adr/0015-sdk-parity.md). One row per capability, one column per merchant SDK (`sdks/rust`, `sdks/nodejs`), a `✅` naming the proving test(s) or a dated `⛔` gap. A second table covers `@vpay/stripe-js`, a separate surface with its own rows. Machine-checked on every `just verify` by `cargo xtask verify-sdk-parity`. |

For what each SDK is and how it's built, see the SDKs themselves
([`sdks/rust`](../../sdks/rust/), [`sdks/nodejs`](../../sdks/nodejs/),
[`sdks/stripe-js`](../../sdks/stripe-js/)) and
[docs/flows/merchant-auth.md](../flows/merchant-auth.md) for the wire
contract they implement. [`sdks/stripe-compat`](../../sdks/stripe-compat/)
is evidence that drives the real `stripe` package against a live stack, not
an SDK — it gets no rows in the matrix.

See [../status.md](../status.md)'s "Merchant SDKs" section for what has and
has not been proven against a real, deployed vpay — the matrix here answers
a narrower question: do the two SDKs agree with each other.
