# Examples

Runnable merchant-side integrations.

| Example | What it shows |
|---|---|
| [merchant-demo](merchant-demo/) | **The only one that runs against a real vpay.** `just demo` boots the stack and drives it: discovery, JWKS, a real token, the 401 boundary, and the honest 404 where payment intents will land |
| [merchant-curl](merchant-curl/) | The raw HTTP shape: the `client_credentials` + `private_key_jwt` handshake, form-encoded bodies, idempotency keys, both flow shapes |
| [merchant-node](merchant-node/) | The same flow through vpay's own Node SDK, [`@vpay/sdk`](../sdks/nodejs/), which performs the `private_key_jwt` handshake the Stripe SDK cannot ([ADR-0010](../docs/adr/0010-merchant-auth-private-key-jwt.md)) |
| [`sdks/rust/examples`](../sdks/rust/examples/) | The same flow through the Rust SDK, `vpay-sdk` |
| [webhook-receiver](webhook-receiver/) | Verifying the `Vpay-Signature` header correctly |

**Status:** `merchant-demo` is the exception — it is written against what
vpay actually serves, and its fourth step exists to show where that stops.
Everything else here describes the *intended* API, as pinned down by
[`docs/flows/merchant-auth.md`](../docs/flows/merchant-auth.md): the merchant
OP (`/v1/oauth/{token,jwks.json,.well-known/openid-configuration}`) and the
`/v1` bearer-token boundary are real as of 2026-09-02, but **no `/v1`
business resource exists**, so every payment-intent, refund, event and
balance call in the other three examples reaches a `404 unknown_route`. See
[../docs/status.md](../docs/status.md).
