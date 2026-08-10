# Examples

Runnable merchant-side integrations.

| Example | What it shows |
|---|---|
| [merchant-curl](merchant-curl/) | The raw HTTP shape: the `client_credentials` + `private_key_jwt` handshake, form-encoded bodies, idempotency keys, both flow shapes |
| [merchant-node](merchant-node/) | Pointing the official Stripe SDK at a vpay host — object model and idempotency semantics only; the SDK cannot perform vpay's own auth handshake, see [ADR-0010](../docs/adr/0010-merchant-auth-private-key-jwt.md) |
| [webhook-receiver](webhook-receiver/) | Verifying the `Vpay-Signature` header correctly |

**Status:** these describe the *intended* API. `/v1/*` is not implemented, so
none of them will succeed against a running vpay today — including the
OAuth2 token endpoint merchant authentication now depends on. See
[../docs/status.md](../docs/status.md).
