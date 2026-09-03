# Runbooks

Operational procedures. Each answers: how do I know this is happening, what do I
do, and how do I know it is fixed.

| Runbook | Trigger |
|---|---|
| [unresolved-charges.md](unresolved-charges.md) | A charge passed 24h with no terminal answer |
| [provider-error-rate.md](provider-error-rate.md) | `provider_error` rate rising |
| [worker-queue.md](worker-queue.md) | A dead-lettered job, a stranded lease, or a rail contradicting a settled charge |
| [webhook-delivery-failures.md](webhook-delivery-failures.md) | A delivery in `exhausted`, an endpoint with no signing secret, or a secret rotation |

**Status:** written from the design, never exercised against a running system.
[worker-queue.md](worker-queue.md) and
[webhook-delivery-failures.md](webhook-delivery-failures.md) are the two whose
states a test actually produces (the worker's integration suite makes dead
letters, stranded leases, `unresolved` escalations and exhausted deliveries on
purpose), and both had their SQL run against a database with every migration
applied — the webhook one including its replay transaction, run twice to
confirm the second run is a no-op. **No runbook here has been followed against
a deployment**, and no replayed delivery has been observed reaching a receiver.
See [../status.md](../status.md).
