# Runbooks

Operational procedures. Each answers: how do I know this is happening, what do I
do, and how do I know it is fixed.

| Runbook | Trigger |
|---|---|
| [unresolved-charges.md](unresolved-charges.md) | A charge passed 24h with no terminal answer |
| [provider-error-rate.md](provider-error-rate.md) | `provider_error` rate rising |
| [worker-queue.md](worker-queue.md) | A dead-lettered job, a stranded lease, or a rail contradicting a settled charge |

**Status:** written from the design, never exercised against a running system.
[worker-queue.md](worker-queue.md) is the first one whose states a test
actually produces (the worker's integration suite makes dead letters, stranded
leases and `unresolved` escalations on purpose), and its SQL was run against a
database with every migration applied — but no runbook here has been followed
against a deployment. See [../status.md](../status.md).
