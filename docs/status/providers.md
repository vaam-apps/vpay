# Providers — one matrix, every rail vpay ships or has designed

**Measured 2026-10-20, on `master` at `0a86e83` plus this branch; the
`notchpay` row re-measured the same day on `master` at `b8be107` plus the
adapter's branch.** The `notchpay` row's Collect and Status read cells were
re-measured on 2026-10-21 by RFC-0012 delivery step 5
([verification](verification/2026-10-21-notchpay-step5.md)). One row per
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
fails one is "gated" below, however many methods it has. **The third rule was
amended on 2026-10-20** by
[ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md):
a read authenticated by a key the provider calls public is admissible when
vpay keeps it server-side and the exposure is documented and logged at boot.
A provider admitted that way says so in its Admission cell, and never "rule 3
passes".

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
- **admitted under ADR-0031** — rule 3 is met only as ADR-0031 amended it:
  the read takes a key the provider calls public, held server-side, and the
  process logs the exposure at every boot.
- **unknown** — not established; the RFC's "To confirm" list carries it.

## The matrix

| Provider       | Methods (Cameroon)                                                                                                 | Admission (ADR-0029 D1)                                                                                                                                                                                                              | Collect (`submit`)           | Status read                  | Callbacks                                       | Callback URL                                                        | Refunds                                                                          | Partial refunds          | Account lookup | Design                                                                                           |
| -------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------- | ---------------------------- | ----------------------------------------------- | ------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------ | -------------- | ------------------------------------------------------------------------------------------------ |
| `mtn_momo`     | MTN MoMo                                                                                                           | shipping                                                                                                                                                                                                                             | proven; sandbox 2026-09-15   | proven; sandbox 2026-09-15   | stub-only: parsed and routed, never received    | per request (`X-Callback-Url`), on a domain registered by hand      | stub-only (Disbursements never called)                                           | yes                      | proven         | [adapter-mtn-momo.md](../flows/adapter-mtn-momo.md)                                              |
| `orange_money` | Orange Money                                                                                                       | shipping                                                                                                                                                                                                                             | proven; never against Orange | proven; never against Orange | stub-only: parsed and routed, never received    | per request (`notif_url`)                                           | token `orange_money::refund`                                                     | no (a decision)          | unsupported    | [adapter-orange-money.md](../flows/adapter-orange-money.md)                                      |
| `notchpay`     | MTN MoMo, Orange Money (aggregator)                                                                                | **admitted under [ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md) (rule 3 amended)**, 2026-10-20. It fails rule 3 as ADR-0029 wrote it; Notch Pay was asked in writing on 2026-10-20 and has not answered | proven; sandbox 2026-10-21   | proven; sandbox 2026-10-21   | stub-only: parsed (conformance), never received | registered with the account by hand; RFC-0013 would register it     | token `notchpay::refund`                                                         | rail documents it; token | unsupported    | [adapter-notchpay.md](../flows/adapter-notchpay.md), [RFC-0012](../rfc/0012-notchpay-adapter.md) |
| `stripe`       | cards, on Stripe's hosted page                                                                                     | passes on its documentation; serves no CEMAC country                                                                                                                                                                                 | designed                     | designed                     | designed                                        | registrable by API (RFC-0013)                                       | designed; the core half is built                                                 | designed                 | unsupported    | [RFC-0010](../rfc/0010-stripe-adapter.md)                                                        |
| `paypal`       | PayPal wallet                                                                                                      | passes on its documentation; XAF and XOF absent                                                                                                                                                                                      | designed                     | designed                     | designed                                        | registrable by API (RFC-0013)                                       | designed                                                                         | designed                 | unsupported    | [RFC-0011](../rfc/0011-paypal-adapter.md)                                                        |
| `flutterwave`  | MTN MoMo, Orange Money (v3 `mobile_money_franco`, XAF 100 to 2 800 000); cards listed for XAF, 3-D Secure unstated | **unknown**: rules 3 and 4 pass on its documentation; rule 1 passes in shape, but XAF withdrawals are unknown because the Cameroon payout pause was never retracted; rule 2 passes on v3's hosted page only                          | —                            | —                            | —                                               | dashboard only: no registration API, so RFC-0013 cannot register it | designed nowhere; v3 refunds have no idempotency key, v4 has `X-Idempotency-Key` | rail documents it        | unknown        | [2026-09-23 evaluation](../plans/2026-09-23-card-provider-evaluation.md), re-measured 2026-10-20 |

Notes on cells that a word cannot carry:

- **`stripe` refunds, "the core half is built".** RFC-0010's delivery step 1
  landed on 2026-10-17: the core hands an `Origin` adapter the charge's
  `provider_txn_id`, and two integration cases drive that arm on a test-only
  rail. No Stripe code exists.
- **`notchpay`, "proven; sandbox 2026-10-21"** (2026-10-20, moved 2026-10-21).
  `vpay-adapter-notchpay` sits on the client `vpay-notchpay`
  ([reference](../reference/vpay-notchpay.md)), `vpay-server` links it, and the
  shared conformance suite's `notchpay` row passes against a WireMock stub
  transcribed from Notch Pay's specification 2.1.0. **Until 2026-10-21 the
  adapter had never called Notch Pay.** When it landed no test-mode key was
  available. A later session the same day had keys and still made no request
  ([verification/2026-10-20-notchpay-sandbox.md](verification/2026-10-20-notchpay-sandbox.md)).
  It found that the real test keys carry `test.` rather than `test_`, and
  corrected the boot rule. **RFC-0012 step 1 ran on 2026-10-21**, through
  `vpay-notchpay`'s probe and not the adapter
  ([verification/2026-10-21-notchpay-sandbox-run.md](verification/2026-10-21-notchpay-sandbox-run.md)).
  Notch Pay's sandbox **contradicted the adapter's addressing**: process and
  read take Notch Pay's `trx.…` reference, and ours is a `404`. The adapter
  was fixed the same day to keep that reference as key material and address
  both by it
  ([verification/2026-10-21-notchpay-trx-reference.md](verification/2026-10-21-notchpay-trx-reference.md)),
  with a crash window Notch Pay offers no way to close.
  **RFC-0012 step 5 ran on 2026-10-21**, on the maintainer's machine: vpay
  (server, worker and Postgres, `livemode=false`) took three test-mode
  payments through the adapter against Notch Pay's real sandbox
  ([verification/2026-10-21-notchpay-step5.md](verification/2026-10-21-notchpay-step5.md)).
  `+237670000000` (210 XAF) reached `succeeded` through the worker's read,
  with `provider_txn_id` `trx.test_…` and a ledger capture of 210 XAF.
  `+237670000002` (220 XAF) reached charge `failed` and intent
  `requires_payment_method`, `failure_code` `provider_error`.
  `+237670000001` (230 XAF, "insufficient funds") read `complete` with a short
  `amounts.total`: its first read answered `pending`, and each of the next
  eleven was refused as `Malformed`. 15 minutes later the charge was still
  `pending` and the intent `processing`, with no success and no failure. Every charge held `notchpay_reference` `trx.test_…` after submit,
  so the fix above met Notch Pay and worked. Both conditions for moving the
  cells held: a dated page records the run, and two of the three charges
  reached a terminal status through the worker's read. The Collect and Status
  read cells therefore say "proven; sandbox 2026-10-21", for MTN's outcome
  numbers on `cm.mobile`. That does not cover Orange through vpay
  (step 1's probe ran it; step 5 did not), a webhook (none was received), a
  refund (`notchpay::refund` is still a token), a live key or Notch Pay's
  production. The short collection's escalation to `unresolved` at the 24 h
  horizon is read from the code and was not observed.
  [verification/2026-10-20-notchpay-adapter.md](verification/2026-10-20-notchpay-adapter.md)
  has the evidence for the build. _(This note read "client … no binary links
  it … waits on the rule-3 decision" until the same day. It was headed
  "stub-only; never called", said "The adapter has never called Notch Pay" and
  ended "The fix has not met Notch Pay. The cells below therefore stay
  'stub-only; never called': step 1 is evidence about Notch Pay, not proof of
  the adapter, and only step 5 can move them" until 2026-10-21, when step 5
  ran.)_
- **`notchpay` carries a production risk no other row does**: whoever holds
  the merchant's public key can read, initialize, process to any number and
  cancel payments on that account. Every boot logs it at `WARN`, and a
  livemode boot refuses the rail unless its block says
  `settings.accept_public_key_exposure: true` (ADR-0031 condition 4). See
  [adapter-notchpay.md](../flows/adapter-notchpay.md) § "Production risks".
- **`notchpay` and `flutterwave` "Methods" are the provider's own claims**, read
  from their documentation. Notch Pay's sandbox has been called: the MTN
  outcome numbers through vpay on 2026-10-21 (step 5), and the MTN and Orange
  outcome numbers through the probe the same day (step 1). `flutterwave` has
  not been called. _(This said "neither has been called" until 2026-10-21.)_
- **`flutterwave` has no RFC and no code.** Re-measured on 2026-10-20 from
  its documentation alone (no account, no call). Its status reads are
  secret-authenticated — a Bearer secret key on v3, OAuth2 client credentials
  on v4 — so it passes the rule Notch Pay fails. One business blocker from
  2026-09-23 still stands: the "payout pause in Cameroon" help page now
  answers "Oops! We can't seem to find the page", with no replacement and no
  retraction, while Flutterwave's payout-time table lists Cameroon XAF bank
  and mobile-money payouts as available. The second has softened, not
  cleared: v3's payment-methods table lists `card` for XAF, and the hosted
  page's own documentation still says nothing of XAF or 3-D Secure. Mobile
  money is the payment this row could carry first. The evaluation page has
  the sources.
- **The "Callback URL" column** is RFC-0013's subject: per request on the two
  shipping rails, registered with the account on the three designed ones.
  Nothing anywhere registers one today.
- **Callback verification** is [RFC-0014](../rfc/0014-verifying-rail-callbacks.md)'s
  subject (Draft, 2026-10-20). Today vpay checks no rail's callback: MTN
  signs nothing; Orange's `notif_token` is stored at submit and never
  compared; Stripe, PayPal and Notch Pay sign, and the port cannot read a
  header. A callback is a hint on every row either way: it moves a poll,
  never money.
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
