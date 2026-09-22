# SDKs

| Doc                    | What it covers                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [parity.md](parity.md) | The cross-SDK capability matrix — record: [ADR-0015](../adr/0015-sdk-parity.md). One row per capability, one column per merchant SDK (`sdks/rust`, `sdks/nodejs`), a `✅` naming the proving test(s) or a dated `⛔` gap. Three further tables cover the payer surfaces, each a separate surface with its own rows: `@vaam-apps/vpay-stripe-js`, `sdks/flutter/vpay_checkout_flutter`, and — since 2026-09-22 — `sdks/tauri/tauri-plugin-vpay-checkout`. Machine-checked on every `just verify` by `cargo xtask verify-sdk-parity`. |

For what each SDK is and how it's built, see the SDKs themselves
([`sdks/rust`](../../sdks/rust/), [`sdks/nodejs`](../../sdks/nodejs/),
[`sdks/stripe-js`](../../sdks/stripe-js/)) and
[docs/flows/merchant-auth.md](../flows/merchant-auth.md) for the wire
contract they implement. [`sdks/stripe-compat`](../../sdks/stripe-compat/)
is evidence that drives the real `stripe` package against a live stack, not
an SDK — it gets no rows in the matrix.

**There is a fourth surface in `parity.md` since 2026-09-22**, and it is not
a merchant SDK either:
[`sdks/tauri/tauri-plugin-vpay-checkout`](../../sdks/tauri/tauri-plugin-vpay-checkout/)
— vpay's hosted checkout opened in the payer's own browser from a Tauri v2
app, plus a `window.open` popup fallback outside Tauri
([ADR-0023](../adr/0023-tauri-checkout-plugin.md),
[docs/flows/tauri-checkout.md](../flows/tauri-checkout.md)). Its column's
`✅` cells cover the Rust crate and the guest-JS package only; the Kotlin
and Swift hosts have no test of any kind and are answered by dated `⛔` rows.

See [../status.md](../status.md)'s "Merchant SDKs" section for what has and
has not been proven against a real, deployed vpay — the matrix here answers
a narrower question: do the two SDKs agree with each other.
