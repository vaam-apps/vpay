# Flows

One document per process, each answering: what happens, in what order, what can
go wrong, and what invariant holds throughout.

This is one of three documentation tiers, and the distinction is worth keeping
straight: an [ADR](../adr/) records a _decision_ that has been taken (immutable
— superseded, never edited); a flow here describes a _process_; and a
[reference page](../reference/) explains why the _code_ that implements it is
shaped the way it is. A change to a process belongs here. A change to how the
code expresses it belongs in `../reference/`.

| Flow                                                 | What it covers                                                                                                                                                                                          |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [payment-lifecycle.md](payment-lifecycle.md)         | PaymentIntent states, both flow shapes, what each transition means                                                                                                                                      |
| [crash-safety.md](crash-safety.md)                   | Why a payer must never act on a transaction we cannot name                                                                                                                                              |
| [reconciler.md](reconciler.md)                       | The poll ladder, prompt expiry, the 24h escalation                                                                                                                                                      |
| [money.md](money.md)                                 | Integer minor units, zero-decimal XAF, the single conversion point                                                                                                                                      |
| [failures.md](failures.md)                           | The canonical taxonomy and how adapters map into it                                                                                                                                                     |
| [provider-port.md](provider-port.md)                 | The adapter interface and the checklist for adding a rail                                                                                                                                               |
| [configuration.md](configuration.md)                 | Boot sequence, validation rules, what refuses to start                                                                                                                                                  |
| [merchant-auth.md](merchant-auth.md)                 | The `client_credentials` + `private_key_jwt` handshake and the `/v1` wire contract the Rust and Node SDKs implement                                                                                     |
| [../sdks/parity.md](../sdks/parity.md)               | The cross-SDK capability matrix — does `sdks/rust` agree with `sdks/nodejs`, capability by capability, each cell naming its proving test or a dated gap (ADR-0015), machine-checked in `just verify`    |
| [stripe-sdk-compat.md](stripe-sdk-compat.md)         | Driving the official Stripe SDKs against vpay: the `config.authenticator` seam, what carries over, and every divergence                                                                                 |
| [browser-checkout.md](browser-checkout.md)           | `/v1/browser` and `@vaam-apps/vpay-stripe-js`: publishable keys, per-intent `client_secret`, the uniform-404 confidentiality property, the redirect gap and its closure                                 |
| [hosted-checkout.md](hosted-checkout.md)             | The page vpay serves — `checkout.session`, hosted and embedded, the two payer credentials, the iframe protocol, `frame-ancestors`, and what is not proven                                               |
| [dashboard.md](dashboard.md)                         | `/dash/v1`'s read surface — the two routes, the boundary that keeps a merchant token off them, and why the app that would use them has no pages                                                         |
| [dashboard-auth.md](dashboard-auth.md)               | Staff login: vpay as its own OpenID Provider for `/dash/v1`                                                                                                                                             |
| [customers.md](customers.md)                         | The `Customer` object: phone-first identity, the address, the erasure that covers every copy, and the twelve-month retention sweep                                                                      |
| [webhooks.md](webhooks.md)                           | The two-step outbox and the signature scheme                                                                                                                                                            |
| [ledger.md](ledger.md)                               | Double-entry postings and the four invariants                                                                                                                                                           |
| [errors.md](errors.md)                               | How an error travels from where it happens to where it is acted on: leaf/composite/boundary tiers, the `Classify` policy table, `anyhow` at the edge only                                               |
| [account-holder-lookup.md](account-holder-lookup.md) | `GET /v1/account_holders`: the three-way answer a name lookup has, the privacy rules that apply only here, and the three controls issue #47 asked for that are **reserved decisions rather than built** |
| [adapter-mtn-momo.md](adapter-mtn-momo.md)           | MTN specifics — push flow                                                                                                                                                                               |
| [adapter-orange-money.md](adapter-orange-money.md)   | Orange specifics — redirect flow                                                                                                                                                                        |
| [deployment.md](deployment.md)                       | What ships, what must exist before a pod starts, the boot order, the `subPath` overlay rule, and what guards a bad deployment                                                                           |

Every flow here is _designed_. See [../status.md](../status.md) for which parts
are actually built.

**Six of these are an overview plus a directory**, since 2026-09-11: each of
[hosted-checkout.md](hosted-checkout.md), [customers.md](customers.md),
[dashboard.md](dashboard.md), [webhooks.md](webhooks.md),
[merchant-auth.md](merchant-auth.md) and [dashboard-auth.md](dashboard-auth.md)
had passed 700 lines. The page you open is still the one linked above; its dated
Status detail is on pages it indexes, moved verbatim. See
[../README.md](../README.md) for the before-and-after counts and for what the
split did and did not change.
