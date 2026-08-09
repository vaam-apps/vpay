# Examples

Runnable merchant-side integrations.

| Example | What it shows |
|---|---|
| [merchant-curl](merchant-curl/) | The raw HTTP shape: form-encoded bodies, idempotency keys, both flow shapes |
| [merchant-node](merchant-node/) | Pointing the official Stripe SDK at a vpay host |
| [webhook-receiver](webhook-receiver/) | Verifying the `Vpay-Signature` header correctly |

**Status:** these describe the *intended* API. `/v1/*` is not implemented, so
none of them will succeed against a running vpay today. See
[../docs/STATUS.md](../docs/STATUS.md).
