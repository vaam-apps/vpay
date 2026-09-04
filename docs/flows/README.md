# Flows

One document per process, each answering: what happens, in what order, what can
go wrong, and what invariant holds throughout.

This is one of three documentation tiers, and the distinction is worth keeping
straight: an [ADR](../adr/) records a *decision* that has been taken (immutable
— superseded, never edited); a flow here describes a *process*; and a
[reference page](../reference/) explains why the *code* that implements it is
shaped the way it is. A change to a process belongs here. A change to how the
code expresses it belongs in `../reference/`.

| Flow | What it covers |
|---|---|
| [payment-lifecycle.md](payment-lifecycle.md) | PaymentIntent states, both flow shapes, what each transition means |
| [crash-safety.md](crash-safety.md) | Why a payer must never act on a transaction we cannot name |
| [reconciler.md](reconciler.md) | The poll ladder, prompt expiry, the 24h escalation |
| [money.md](money.md) | Integer minor units, zero-decimal XAF, the single conversion point |
| [failures.md](failures.md) | The canonical taxonomy and how adapters map into it |
| [provider-port.md](provider-port.md) | The adapter interface and the checklist for adding a rail |
| [configuration.md](configuration.md) | Boot sequence, validation rules, what refuses to start |
| [merchant-auth.md](merchant-auth.md) | The `client_credentials` + `private_key_jwt` handshake and the `/v1` wire contract the Rust and Node SDKs implement |
| [../sdks/parity.md](../sdks/parity.md) | The cross-SDK capability matrix — does `sdks/rust` agree with `sdks/nodejs`, capability by capability, each cell naming its proving test or a dated gap (ADR-0015), machine-checked in `just verify` |
| [stripe-sdk-compat.md](stripe-sdk-compat.md) | Driving the official Stripe SDKs against vpay: the `config.authenticator` seam, what carries over, and every divergence |
| [browser-checkout.md](browser-checkout.md) | `/v1/browser` and `@vpay/stripe-js`: publishable keys, per-intent `client_secret`, the uniform-404 confidentiality property, the redirect gap |
| [dashboard-auth.md](dashboard-auth.md) | Staff login: vpay as its own OpenID Provider for `/dash/v1` |
| [webhooks.md](webhooks.md) | The two-step outbox and the signature scheme |
| [ledger.md](ledger.md) | Double-entry postings and the four invariants |
| [errors.md](errors.md) | How an error travels from where it happens to where it is acted on: leaf/composite/boundary tiers, the `Classify` policy table, `anyhow` at the edge only |
| [adapter-mtn-momo.md](adapter-mtn-momo.md) | MTN specifics — push flow |
| [adapter-orange-money.md](adapter-orange-money.md) | Orange specifics — redirect flow |
| [deployment.md](deployment.md) | What ships, what must exist before a pod starts, the boot order, the `subPath` overlay rule, and what guards a bad deployment |

Every flow here is *designed*. See [../status.md](../status.md) for which parts
are actually built.
