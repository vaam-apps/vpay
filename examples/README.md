# Examples

Runnable merchant-side integrations.

| Example | What it shows |
|---|---|
| [merchant-curl](merchant-curl/) | The raw HTTP shape: the `client_credentials` + `private_key_jwt` handshake, form-encoded bodies, idempotency keys, both flow shapes |
| [merchant-node](merchant-node/) | The same flow through vpay's own Node SDK, [`@vpay/sdk`](../sdks/nodejs/), which performs the `private_key_jwt` handshake the Stripe SDK cannot ([ADR-0010](../docs/adr/0010-merchant-auth-private-key-jwt.md)) |
| [`sdks/rust/examples`](../sdks/rust/examples/) | The same flow through the Rust SDK, `vpay-sdk` |
| [webhook-receiver](webhook-receiver/) | Verifying the `Vpay-Signature` header correctly |

**Status:** these describe the *intended* API, as pinned down by
[`docs/flows/merchant-auth.md`](../docs/flows/merchant-auth.md). `/v1/*` is
not implemented, so none of them will succeed against a running vpay today —
including the OAuth2 token endpoint merchant authentication depends on. The
SDKs themselves are tested against HTTP stubs of that contract. See
[../docs/status.md](../docs/status.md).
