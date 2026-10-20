# Providers — one matrix, every rail vpay ships or has designed

**Measured 2026-10-20, on `master` at `0a86e83` plus this branch.** One row per
payment provider, one column per thing the provider port asks of an adapter.
The maintainer asked for it on 2026-10-20: every provider implements the same
trait, a gap is an error the trait already has, and this page is where the
state of every cell is read.

**The goal is payments, not cards.** A provider that can take a payment is
worth an adapter whatever its payment methods are; cards are one method among
mobile money, wallets and bank transfers. A provider still has to meet the
four hard rules of
[ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
D1 — the money lands in the merchant's own account, no card number reaches
vpay, the status read is authenticated by a secret, the contract can be
stubbed — and those rules are about trust, not about methods. A provider that
fails one is "gated" below, however many methods it has.

## How a gap is written

The port, `vpay_provider::ProviderAdapter`, already distinguishes the two kinds
of "not supported", and the distinction is what makes this page checkable:

| The provider…               | The adapter answers                              | Checked by                                                                                               |
| --------------------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------- |
| cannot do it, ever          | `ProviderError::Unsupported`, the port's default | the capability flag beside it (`supports_refunds: false`, …); the core refuses before calling            |
| can do it, vpay has not yet | `ProviderError::NotImplemented("rail::op")`      | `cargo xtask verify-status`: every token in code is declared in [docs/status.md](../status.md), and back |
| can do it, vpay does        | a real call                                      | a conformance case against a WireMock stub, then a real sandbox call on a dated verification page        |

No new error type is needed for the matrix, and none should be added: a third
"not supported" variant would be a second way to say one of these two things.

## Legend

- **proven** — a conformance case fails if the behaviour breaks (WireMock).
- **stub-only** — real code whose only witness is a stub.
- **sandbox** — also run against the provider's real sandbox, on the dated
  page named.
- **token** — `NotImplemented`, declared in `docs/status.md`.
- **unsupported** — the provider cannot; the port's `Unsupported`.
- **client** — the HTTP call exists in a client crate, and no adapter uses it.
- **designed** — an RFC describes it; no code.
- **gated** — a hard rule of ADR-0029 D1 is not met; the decision is the
  maintainer's.
- **unknown** — not established; the RFC's "To confirm" list carries it.

## The matrix

| Provider       | Methods (Cameroon)                      | Admission (ADR-0029 D1)                                                               | Collect (`submit`)           | Status read                  | Callbacks                                    | Callback URL                                                   | Refunds                                | Partial refunds   | Account lookup | Design                                                                   |
| -------------- | --------------------------------------- | ------------------------------------------------------------------------------------- | ---------------------------- | ---------------------------- | -------------------------------------------- | -------------------------------------------------------------- | -------------------------------------- | ----------------- | -------------- | ------------------------------------------------------------------------ |
| `mtn_momo`     | MTN MoMo                                | shipping                                                                              | proven; sandbox 2026-09-15   | proven; sandbox 2026-09-15   | stub-only: parsed and routed, never received | per request (`X-Callback-Url`), on a domain registered by hand | stub-only (Disbursements never called) | yes               | proven         | [adapter-mtn-momo.md](../flows/adapter-mtn-momo.md)                      |
| `orange_money` | Orange Money                            | shipping                                                                              | proven; never against Orange | proven; never against Orange | stub-only: parsed and routed, never received | per request (`notif_url`)                                      | token `orange_money::refund`           | no (a decision)   | unsupported    | [adapter-orange-money.md](../flows/adapter-orange-money.md)              |
| `notchpay`     | MTN MoMo, Orange Money (aggregator)     | **gated**: rule 3 — the status read takes the public key; asked in writing 2026-10-20 | client                       | client                       | client parses the event                      | client can list and create (RFC-0013)                          | designed as a token (RFC-0012 § 7)     | rail documents it | unsupported    | [RFC-0012](../rfc/0012-notchpay-adapter.md)                              |
| `stripe`       | cards, on Stripe's hosted page          | passes on its documentation; serves no CEMAC country                                  | designed                     | designed                     | designed                                     | registrable by API (RFC-0013)                                  | designed; the core half is built       | designed          | unsupported    | [RFC-0010](../rfc/0010-stripe-adapter.md)                                |
| `paypal`       | PayPal wallet                           | passes on its documentation; XAF and XOF absent                                       | designed                     | designed                     | designed                                     | registrable by API (RFC-0013)                                  | designed                               | designed          | unsupported    | [RFC-0011](../rfc/0011-paypal-adapter.md)                                |
| `flutterwave`  | cards, MTN MoMo, Orange Money, on paper | **unknown**: two business blockers on 2026-09-23, re-checked 2026-10-20               | —                            | —                            | —                                            | unknown                                                        | —                                      | —                 | —              | [2026-09-23 evaluation](../plans/2026-09-23-card-provider-evaluation.md) |

Notes on cells that a word cannot carry:

- **`stripe` refunds, "the core half is built".** RFC-0010's delivery step 1
  landed on 2026-10-17: the core hands an `Origin` adapter the charge's
  `provider_txn_id`, and two integration cases drive that arm on a test-only
  rail. No Stripe code exists.
- **`notchpay`, "client".** `vpay-notchpay` ([reference](../reference/vpay-notchpay.md))
  implements initialize, process and read a payment, list and create webhooks,
  and parse an event, from Notch Pay's specification 2.1.0, with 34 unit
  tests. Nothing has called Notch Pay, and no binary links it. The adapter
  that would turn its answers into `ChargeStatus` is RFC-0012's step 2, and
  waits on the rule-3 decision.
- **`notchpay` and `flutterwave` "Methods" are the provider's own claims**, read
  from their documentation; neither has been called.
- **`flutterwave` is a blank row on purpose.** No RFC exists. The 2026-09-23
  evaluation found it the strongest candidate technically and blocked on a
  Cameroon XAF payout pause and on hosted XAF cards; both are being re-checked
  on 2026-10-20 and this row changes when that check lands.
- **The "Callback URL" column** is RFC-0013's subject: per request on the two
  shipping rails, registered with the account on the three designed ones.
  Nothing anywhere registers one today.
- **Account lookup** is "unsupported" on every rail but MTN because none of the
  others documents a registered-name lookup for a payer.

## What this page is not

It is not a status of the core. Whether a refund settles, whether a charge
reconciles, whether a payout exists are core questions answered on
[backend.md](backend.md) and [docs/status.md](../status.md), and they are the
same for every row here: nothing settles a `pending` refund on any rail, and
vpay pays nobody out.

It is not generated. Each cell is measured by hand on the date above and is as
current as the last pull request that touched a rail. A row that moves must
move here in the same change, exactly as [CLAUDE.md](../../CLAUDE.md) requires
of every status page. Deriving the capability columns from each adapter's
`Capabilities` with an `xtask` would make half of it checkable; that is not
built.
