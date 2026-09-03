# Examples

Runnable merchant-side integrations.

| Example | What it shows |
|---|---|
| [merchant-demo](merchant-demo/) | **The only one that runs against a real vpay.** `just demo` boots the stack and drives it: discovery, JWKS, a real token, the 401 boundary, and the honest 404 where payment intents will land |
| [merchant-curl](merchant-curl/) | The raw HTTP shape: the `client_credentials` + `private_key_jwt` handshake, form-encoded bodies, idempotency keys, both flow shapes |
| [merchant-node](merchant-node/) | The same flow through vpay's own Node SDK, [`@vpay/sdk`](../sdks/nodejs/), which performs the `private_key_jwt` handshake ([ADR-0010](../docs/adr/0010-merchant-auth-private-key-jwt.md)) |
| [merchant-stripe-node](merchant-stripe-node/) | **The second one that runs against a real vpay.** The same payment through the *official* `stripe` package, authenticated by `@vpay/sdk/stripe` — create, confirm, poll to `succeeded` ([stripe-sdk-compat.md](../docs/flows/stripe-sdk-compat.md)) |
| [`sdks/rust/examples`](../sdks/rust/examples/) | The same flow through the Rust SDK, `vpay-sdk` |
| [webhook-receiver](webhook-receiver/) | Verifying the `Vpay-Signature` header correctly |

**Status, corrected 2026-09-03.** This section used to say "**no `/v1`
business resource exists**", which was true when it was written on
2026-09-02 and false the next day. `/v1/payment_intents` — create, retrieve,
list, confirm, cancel — is served, and a confirm reaches a rail.

`merchant-demo` and `merchant-stripe-node` are the two examples written
against what vpay actually serves, and both are run end to end against the
compose stack. `merchant-curl` and `merchant-node` still describe the
*intended* API as pinned down by
[`docs/flows/merchant-auth.md`](../docs/flows/merchant-auth.md), and their
refund, event and balance calls still reach a `404 unknown_route`, because
those routes are deliberately not mounted — `/v1/events`, though, **is**
served as of Step 5. `webhook-receiver` describes a delivery vpay now really
sends: the worker signs and POSTs it, and a delivered one has been verified
both with `@vpay/sdk` and with the official `stripe` package's
`constructEvent`. An intent reaches `succeeded` too — `vpay-worker` polls the
rail and settles the charge — but every rail and every receiver involved so
far has been a WireMock host on a compose network, and no money has moved. See
[../docs/status.md](../docs/status.md).
