# Flows

One document per process, each answering: what happens, in what order, what can
go wrong, and what invariant holds throughout.

| Flow | What it covers |
|---|---|
| [payment-lifecycle.md](payment-lifecycle.md) | PaymentIntent states, both flow shapes, what each transition means |
| [crash-safety.md](crash-safety.md) | Why a payer must never act on a transaction we cannot name |
| [reconciler.md](reconciler.md) | The poll ladder, prompt expiry, the 24h escalation |
| [money.md](money.md) | Integer minor units, zero-decimal XAF, the single conversion point |
| [failures.md](failures.md) | The canonical taxonomy and how adapters map into it |
| [provider-port.md](provider-port.md) | The adapter interface and the checklist for adding a rail |
| [configuration.md](configuration.md) | Boot sequence, validation rules, what refuses to start |
| [webhooks.md](webhooks.md) | The two-step outbox and the signature scheme |
| [ledger.md](ledger.md) | Double-entry postings and the four invariants |
| [adapter-mtn-momo.md](adapter-mtn-momo.md) | MTN specifics — push flow |
| [adapter-orange-money.md](adapter-orange-money.md) | Orange specifics — redirect flow |

Every flow here is *designed*. See [../status.md](../status.md) for which parts
are actually built.
