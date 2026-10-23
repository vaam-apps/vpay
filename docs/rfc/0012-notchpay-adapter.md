# RFC-0012: A Notch Pay adapter — mobile money through an aggregator, behind the port

- **Status:** Draft, **admitted under
  [ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md)
  (rule 3 amended) on 2026-10-20, and built.** Notch Pay still fails the third
  hard rule of [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
  D1 **as ADR-0029 wrote it** (§ "The gate" below). It does not pass that
  rule; the rule was changed, openly, for every provider. Delivery step 0 is
  that ADR. Steps 2–4 are built in the same change as it, with every Notch
  Pay mapping taken from `openapi.yaml` 2.1.0 and marked "To confirm",
  because no test-mode key was available to it. **Steps 1 and 5 were done on
  2026-10-21**: Notch Pay's real sandbox answered the probe (step 1,
  [page][sandbox-run]), and took three test-mode payments through the adapter
  (step 5, [page][step5]): one succeeded, one failed, and one that read
  `complete` for less than its amount was refused and stayed `pending`.
  Notch Pay's production has never been called. `vpay-notchpay`, the client,
  landed earlier on 2026-10-20 (§ 1). _(This said "because no test-mode key
  was available to it: **nothing has called Notch Pay**, and steps 1 and 5
  are not done" until 2026-10-21.)_
- **Author:** vpay maintainers
- **Date:** 2026-10-20
- **Related:** [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
  (D1's four hard rules, D2, D6, D8–D10, D12),
  [RFC-0004](0004-billing-on-top-of-invoices.md) § 7,
  [RFC-0008](0008-payment-method-routing.md),
  [RFC-0009](0009-provider-accounts-per-merchant.md),
  [RFC-0010](0010-stripe-adapter.md) (§§ 2, 6 and 8 are reused here),
  [RFC-0003](0003-refunds-destinations-and-the-first-ledger-postings.md) § 1,
  [the 2026-09-23 card-provider evaluation](../plans/2026-09-23-card-provider-evaluation.md),
  [ADR-0002](../adr/0002-provider-port.md),
  [ADR-0006](../adr/0006-no-mocks-in-main-processes.md),
  [crash-safety.md](../flows/crash-safety.md),
  [adapter-mtn-momo.md](../flows/adapter-mtn-momo.md)
- **Sources, all read on 2026-10-20:** Notch Pay's developer documentation as
  raw Markdown (the index is `developer.notchpay.co/llms.txt`) and its OpenAPI
  specification, `openapi.yaml` **version 2.1.0**; the
  [pricing page](https://notchpay.co/pricing); the
  [merchant agreement](https://notchpay.co/policies/merchant-agreement) (last
  updated 31 December 2023); and the Ministry of Finance's
  [list of approved payment institutions](https://dgtcfm.cm/en/approved-payment-institutions/).
  Nothing in this RFC was learned by calling Notch Pay. Where Notch Pay's own
  pages disagree with each other, both are quoted and the disagreement is a
  "To confirm" item, never resolved by choosing the more convenient one.

## Problem

The maintainer wrote on 2026-10-20: "We'll also need an adapter for
notchpay."

What Notch Pay would add is real. A Cameroon merchant today collects MTN MoMo
through MTN's Collection product and Orange Money through an Orange Web
Payment merchant key: two operator contracts, each with its own onboarding.
Notch Pay is an aggregator. One account, opened online, collects both
networks, plus Express Union and Yoomee wallets that neither direct rail
carries, and XOF wallets in Côte d'Ivoire and Senegal
([mobile money](https://developer.notchpay.co/accept-payments/mobile-money),
[testing](https://developer.notchpay.co/get-started/testing)).

What it would **not** add is cards. The 2026-09-23 evaluation found cards only
as a channel name, `cm.card`, and a phrase in the FAQ. That is still true
today: no card guide, no 3-D Secure, no test card and no card price appear
anywhere in the documentation. This RFC designs a mobile-money rail, and says
so in its title.

### The gate

The same evaluation examined Notch Pay against the rules RFC-0004 § 7 later
set, and Notch Pay is the reason ADR-0029 D1's third rule says
_secret_-authenticated rather than _authenticated_. Re-measured today, every
finding stands.

| ADR-0029 D1 rule                      | Notch Pay, as documented on 2026-10-20                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Verdict                                                                   |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 1. vpay never holds the money         | Payments land in the merchant's own Notch Pay balance, withdrawable to mobile money ("Your money is available at any time", [pricing](https://notchpay.co/pricing)). But settlement funds are held "in accordance with our policies", which are not published (agreement § 8.3), and eligibility is "reserved for natural persons aged 18 and over, residing in French-speaking Africa" (§ 3), against § 1's definition of a merchant as "any natural or legal person"                                                                                            | Passes in shape, on model A (§ 2). A company's eligibility is unconfirmed |
| 2. No card number reaches vpay-server | A push mobile-money rail carries no card                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Passes                                                                    |
| 3. A secret-authenticated status read | **`GET /payments/{reference}` declares `ApiKeyAuth` and `SyncAuth` only** (openapi.yaml 2.1.0). `ApiKeyAuth` is the `Authorization` header, which takes the key the documentation calls public: "Public keys (starting with `pk_`) … can be safely included in your frontend code" ([authentication](https://developer.notchpay.co/get-started/authentication)). The private key, in `X-Grant`, is required on `/balance`, `/transfers`, `/beneficiaries` and `/webhooks` only ([API authentication](https://developer.notchpay.co/api-reference/authentication)) | **Fails** as ADR-0029 wrote it; admitted under ADR-0031                   |
| 4. A stub-able contract               | An OpenAPI 3.1 specification, and a test mode in which a phone number's last digits choose the outcome ([testing](https://developer.notchpay.co/get-started/testing)). The specification omits refunds and the duplicate-reference `409` (§§ 3 and 7)                                                                                                                                                                                                                                                                                                             | Passes, with gaps                                                         |

ADR-0029 D1 is explicit: "a provider … whose status read accepts a
publishable key, gets no adapter." On its documentation today, Notch Pay gets
no adapter.

**Licensing is not established either, and it is not the gate.** No Notch Pay
page names a legal entity, a licence, a regulator, a partner bank or a
registered office; the merchant agreement names "the company operating the
Services" and an email address. The Ministry of Finance's list of approved
payment institutions, read today, names two: Orange Money Cameroun and Mobile
Money Corporation. ADR-0029 D1 makes licensing "a property of the operator's
configuration, not of the adapter", so it is the operator's due diligence
(§ 9), not a reason to refuse code.

### What rule 3 protects, and what Notch Pay's read costs

The read's **integrity** does not come from the key. It comes from TLS to
`api.notchpay.co`: a third party holding the public key still cannot forge the
answer vpay reads, and § 4 checks the amount and currency of every success.

What the public key **does** give its holder, all documented:

- read the status, amount and customer of any payment on the account;
- initialize a payment on the account;
- process a pending payment to **any** phone number — a USSD prompt sent in
  the merchant's name (`POST /payments/{reference}`, `ApiKeyAuth` only);
- cancel any pending payment (`DELETE /payments/{reference}`, `ApiKeyAuth`
  only), whose reference a payer's browser sees on Notch Pay's own redirect
  ([collect](https://developer.notchpay.co/accept-payments/collect)).

vpay keeps the key server-side, so these are the merchant's exposure only if
the key leaks — or if the merchant also embeds it in a front end, which Notch
Pay's documentation invites ("initializing the Notch Pay SDK in client-side
code"). The 2026-09-23 page put it in one sentence: the read "passes the
letter of 'authenticated' and fails its purpose."

**Two ways forward. Choosing between them is the maintainer's decision, and
this RFC does not make it.**

- **A. Ask Notch Pay, in writing (recommended first).** Does
  `GET /payments/{reference}` accept, and can an account make it **require**,
  the private key in `X-Grant`? Can an account disable the public key's
  payment operations? A yes to the first meets rule 3 as written: the adapter
  sends `X-Grant` on every call (§ 3), and boot refuses a configuration
  without the private key (§ 2).
- **B. Amend rule 3.** A new ADR superseding ADR-0029 D1's third rule, for
  example: "secret-authenticated, or authenticated by a key the deployment
  keeps server-side, with the provider's public-key operations recorded as
  the merchant's accepted exposure". That is a policy change for every future
  provider, not an exception for this one, and it would be argued as such.

**Decided 2026-10-20: way A.** The maintainer wrote: "Ask Notch Pay first,
I'll email them", and will raise the same questions in a Telegram group the
maintainer shares with Notch Pay. The questions are § 10's written items (1,
6, 8, 9, 10, 11 and 12) and where merchants' funds are held (§ 9), with the
sandbox items attached for completeness. Rule 3 stays as written until Notch
Pay answers; delivery step 0 waits on that answer now, not on a decision. **The maintainer emailed Notch Pay on 2026-10-20.** No answer is recorded
yet; this paragraph moves when one is.

**Changed 2026-10-20, the same day: way B, taken openly.** Notch Pay had
not answered. The maintainer wrote: "Let's do a full implementation and warn
the developer about the risks in prod. They never answered to me too".
[ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md)
amends rule 3 for every provider, starting from the wording suggested under
"B." above, and keeps everything way A gave for free: the private key goes in
`X-Grant` on every call (§ 3), and boot refuses a configuration without it
(§ 2). The question to Notch Pay stays open (§ 10 item 1); a yes moves Notch
Pay to ADR-0031's first limb. Notch Pay is "admitted under ADR-0031 (rule 3
amended)" — **never** "rule 3 passes". The risk the amendment accepts is
written in § 9 under "Production risks", and the process logs it at every
boot of a deployment that configures the rail.

Building the adapter under the public key and calling rule 3 "satisfied in
practice" is way B done silently. It is the one option this RFC refuses, and
ADR-0031 refuses it too.

The design below was written so that the decision would be about the rule,
not about whether an adapter can be designed. It was written against way A:
every call carries the private key, and nothing was lost when way B was
chosen instead.

## Proposal

### 1. Shape

| Property                         | Value                                                                                                                                                                                                                                        |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rail code                        | `notchpay`                                                                                                                                                                                                                                   |
| Crates                           | **`backends/crates/vpay-notchpay/`**, the client — Notch Pay's bodies and calls, nothing about charges (built 2026-10-20); and `backends/crates/vpay-adapter-notchpay/`, the adapter — `lib.rs` and `mapping.rs` on top of it; no `token.rs` |
| `flow`                           | **`Push`** — Notch Pay's direct charge, which sends the payer a USSD prompt ([charge](https://developer.notchpay.co/accept-payments/charge))                                                                                                 |
| `supports_refunds`               | `true` — the rail documents refunds (§ 7)                                                                                                                                                                                                    |
| `supports_partial_refunds`       | `true` — "Omit amount for full refunds" ([refunds](https://developer.notchpay.co/accept-payments/refunds))                                                                                                                                   |
| `delivers_callbacks`             | `true` — webhooks, registered out of band (§ 6)                                                                                                                                                                                              |
| `requires_ip_allowlist`          | `false` — Notch Pay's IP whitelist is documented "for API transfers", which this adapter never calls. Whether refunds need it is "To confirm" item 12                                                                                        |
| `supports_account_holder_lookup` | `false` — the specification has no lookup                                                                                                                                                                                                    |
| `refund_destination`             | **`Origin`** — "A full refund returns the entire payment amount to the customer", with no payee in the request. The second `Origin` rail after RFC-0010's, and the core half has landed (§ 7)                                                |
| `payer_fields`                   | one phone field, declared exactly as `mtn_momo` declares its own: the payer's mobile-money number                                                                                                                                            |
| `parse_destination`              | the port's default `Unsupported`, which is the permanent, correct answer on an `Origin` rail                                                                                                                                                 |
| Method types (RFC-0008)          | `mtn_wallet` and `orange_wallet` (ADR-0029 D12). Today a merchant sends `payment_method_types[]=notchpay`                                                                                                                                    |

No `token.rs`: Notch Pay authenticates with static keys in headers, so there
is nothing to mint, cache or fingerprint.

**Why the wire is its own crate.** The maintainer wrote on 2026-10-20: "From
their official api-ref we can create a new crate and use it internally."
Notch Pay publishes SDKs for JavaScript, PHP, Python, Laravel and mobile, and
none for Rust. So the `wire.rs` that each existing adapter keeps inside
itself becomes `vpay-notchpay`, an internal crate (`publish = false`)
transcribed from openapi.yaml 2.1.0. The split is the one `wire.rs` and
`mapping.rs` already draw inside `mtn_momo` and `orange_money`, moved to a
crate boundary: the client says what Notch Pay answered, as `Answer::Success`
or `Answer::Refused` with Notch Pay's own words, and returns the port's
`ProviderError` only for a transport failure or an answer that does not
parse. What a `409` or a `failed` means for a charge stays in the adapter's
`mapping.rs`. The client is built and tested; its crate docs list what it
covers and what it deliberately omits — refunds, which the specification
does not have, among them.

**Why `Push` and not Notch Pay's hosted page.** Notch Pay offers both: a
hosted page, Collect, where the payer picks a channel, and a direct charge,
where the merchant names the channel and the payer's number. The port gives
an adapter one `flow`, so the choice is real. `Push` is chosen because mobile
money is all Notch Pay documents, because the push experience is the one the
checkout already gives `mtn_momo` payers, and because the conformance suite's
push cases then run against this rail unchanged. Collect is the alternative
considered below; it becomes the case for an adapter with two flows on the day
Notch Pay documents a card.

**The channel.** Notch Pay's channel table lists `cm.mobile` as "MTN /
Orange, Cameroon, XAF", beside `cm.mtn` and `cm.orange`
([mobile money](https://developer.notchpay.co/accept-payments/mobile-money)).
The adapter sends `cm.mobile`, configurable as `settings.channel`, so it never
carries a number-plan table of its own. Whether `cm.mobile` resolves the
network from the number, and whether the sandbox's outcome numbers behave
under it, is "To confirm" item 3. When ADR-0029 D6 lands and the core can say
which network the payer chose, the adapter sends `cm.mtn` or `cm.orange`
instead.

**The first rail that serves a method a direct rail also serves.** With
`mtn_momo` and `notchpay` both enabled, an MTN payer has two rails. That is
exactly the case ADR-0029 D8–D10 exist for: the cheaper rail first, from the
merchant's configured fees. Until D8–D10 land, the checkout shows the two as
two entries, by rail code (open question 2).

### 2. Configuration, on RFC-0009's two levels

```yaml
# Illustrative. The account model decides which level each key sits on.
providers:
  - code: notchpay
    host:
      url: https://api.notchpay.co # a WireMock host in compose
      label: notchpay
    display_name: { en: Mobile money, fr: Mobile money }
    currency: XAF
    settings:
      channel: cm.mobile
      description: Payment # the one free-text field Notch Pay receives
    credentials: {} # rail-level; see the two models below
```

| Model                          | Rail-level (`providers[]`)                    | Account-level (`provider_accounts[]`, RFC-0009)                              | Who the operator is to Notch Pay                                                                                                                                   |
| ------------------------------ | --------------------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **A. The merchant's own keys** | nothing                                       | `credentials.public_key`, `credentials.private_key` — the merchant's account | Nobody                                                                                                                                                             |
| **B. Sync**                    | the platform's `public_key` and `private_key` | `settings.sync_account` — the connected account, sent as `X-Sync`            | A Notch Pay Sync platform. Sync is enabled by contacting sales ([Sync](https://developer.notchpay.co/sync)), and § "To confirm" 9 decides whether it passes rule 1 |

Sync's own overview offers "Direct Charges: Charge customers directly and
transfer funds to connected accounts". Whether the money lands in the
connected account's balance — pass-through — or in the platform's first —
aggregation, which rule 1 forbids — is not written anywhere. **Model B is not
offered until Notch Pay answers in writing.** Until RFC-0009 lands there is
one level, so a deployment serves one merchant, on model A. That is the limit
MTN and Orange have today.

Boot refusals, in `vpay_config::config::REQUIRED_RAIL_KEYS` per ADR-0012:

- `public_key` and `private_key` are both required. The private key is what
  way A would require on every call and what refunds require in any case.
- **The livemode rule is on the key, because the host cannot carry it.** The
  specification lists one server, `https://api.notchpay.co`, described as
  "Production and Sandbox Environment". vpay's livemode host rule, which
  refuses a host containing a stub marker, cannot tell Notch Pay's sandbox
  from its production. The keys can: test keys are documented as `pk_test_…`
  ([testing](https://developer.notchpay.co/get-started/testing)) and as
  containing "the `test_` prefix"
  ([authentication](https://developer.notchpay.co/get-started/authentication)).
  So in livemode a key containing `test_` refuses to boot, and outside
  livemode a key without it does — the `PublishableKeyLivemodeMismatch` rule
  applied to a rail key, as RFC-0010 § 2 applies it to Stripe's. A live key's
  prefix is not documented ("To confirm" item 8).

  _**Corrected 2026-10-20, on the maintainer's own test-mode keys.** Neither
  of them contains `test_`. Both contain `test` followed by `.`; the public
  key starts `pk` and the private key `sk`. The maintainer confirmed in
  Notch Pay's dashboard that they are test-mode keys. Only those facts were
  read, never a value. So the documented `test_` is not what this account's
  sandbox keys carry, and a rule reading `test_` alone was wrong both ways:
  it refused every genuine sandbox key outside livemode and admitted one in
  livemode. The rule now reads either `test.` or `test_` as a test key.
  `test_` stays because the documentation spells it so, and in livemode
  refusing more test-looking keys costs nothing. A live key's prefix is still
  unknown (item 8). Evidence:
  [2026-10-20-notchpay-sandbox.md](../status/verification/2026-10-20-notchpay-sandbox.md)._

- `settings.channel` must be a `cm.` channel the adapter knows.
- **In livemode, `settings.accept_public_key_exposure` must be `true`.**
  _Added when built, 2026-10-20_, on the maintainer's answer "Livemode
  acknowledgement: YES, require it." It is ADR-0031 condition 4: the operator
  accepts in writing what the public key exposes (§ 9), and boot refuses
  without it, naming the setting and `credentials.public_key`, never a value.
  A sandbox only warns. Any value other than `true` or `false` is refused in
  either mode.
- `currency` must be `XAF`. Notch Pay's XOF channels are other countries and
  another RFC.

There is **no `webhook_hash`** in the configuration, although every Notch Pay
webhook endpoint has one. Nothing could read it until the port passes headers
to `parse_callback` (§ 6), and a secret that nothing reads is a secret held
for no reason.

Notch Pay's amount bounds are 100 and 10 000 000, from two example errors
([errors](https://developer.notchpay.co/api-reference/errors)): "Amount must
be at least 100" and "Amount exceeds maximum allowed (10000000)". They are
`amount_limits` (ADR-0029 D2) when that capability exists; RFC-0010 is its
first consumer. Until then an amount outside them is Notch Pay's `422`.

### 3. The wire — three calls on the charge path

Every call carries `Authorization: <public_key>`, `X-Grant: <private_key>`,
`Content-Type: application/json`, and on model B `X-Sync`. `X-Grant` is sent
even where the specification does not ask for it: it is what way A would
require, and a server that ignores it loses nothing.

**Call 1 — initialize.** `POST /payments`:

```json
{
  "amount": "<Money::to_provider_minor(), XAF has no minor unit>",
  "currency": "XAF",
  "phone": "<the payer's number, E.164>",
  "reference": "<the charge's reference>",
  "description": "<settings.description>"
}
```

The specification requires one of `email`, `phone` or `customer`. `phone` is
the one vpay already holds, so no email or name is invented and no merchant
metadata is forwarded. The payer's number is the one personal datum Notch Pay
receives, and step 4 registers Notch Pay as its recipient in
`schemas/privacy-inventory.yaml`.

| Answer                                               | Outcome                                                                                                                                                                                               |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `201`, `transaction.status = pending`                | call 2. _(Designed as "keep `transaction.id` and Notch Pay's own `transaction.reference` in `ref_extra`, then call 2"; built with an empty `ref_extra`, see below.)_                                  |
| `409` "A payment with this reference already exists" | the crash-retry case: call 3; on `pending`, call 2; on anything else, `Submitted` and the poll resolves it. Documented on the errors page and **absent from the specification** ("To confirm" item 4) |
| `422` validation (amount bounds, currency, phone)    | `Rejected { provider_error }` with Notch Pay's message as the raw reason                                                                                                                              |
| `401`, `403`                                         | `Rejected { provider_account_blocked }` — our own keys, and it pages, as on `mtn_momo`                                                                                                                |
| `429`, `5xx`, timeout                                | `Transport` — the charge's fate is unknown, never a decline                                                                                                                                           |

**Call 2 — process.** `POST /payments/{reference}`:

```json
{ "channel": "cm.mobile", "data": { "phone": "<the payer's number, E.164>" } }
```

`202` with `status = processing` is `Submitted { ref_extra, redirect_url:
None }`. A `422` "Payment has already been processed" is not a decline: the
adapter reads the payment (call 3) and returns `Submitted` unless the read
says it never left `pending`.

Which reference goes in the path is **the first thing to confirm in the
sandbox** (item 2). The API reference passes the merchant's (`order_123`); the
direct-charge guide passes `data.transaction.reference`, which in its own
example response is Notch Pay's (`trx.123456789abcdefghj`), with the
merchant's beside it as `trxref`
([charge](https://developer.notchpay.co/accept-payments/charge)). The adapter
uses Notch Pay's reference from call 1 when it holds one, and the charge's
own only on the crash path, where call 1's answer was lost.

**Changed when built, 2026-10-20: the adapter uses the charge's own
reference on every call, and `ref_extra` is always empty.** That is the
specification's form. One rule behaves the same before and after a crash.
The conformance suite requires a push rail's duplicate submit to carry no key
material, and a stored `trx.…` would be exactly that. If item 2 shows the
path wants Notch Pay's reference, the read changes in one place
(`vpay_adapter_notchpay`'s `query_status`) and `ref_extra` starts carrying
it. Until then the guide's form is not used. The guide also
spells the number as `data.account_number` in one example and `data.phone` in
another; the specification has both, and the adapter sends `phone`.

**Call 3 — read.** `GET /payments/{reference}`, with the same reference rule.
`202` carries `transaction`, mapped by § 4. A `404` whose message is Notch
Pay's "Payment not found" is `ChargeStatus::NotFound`, which
[crash-safety.md](../flows/crash-safety.md) never lets fail a charge on its
own: three `NotFound`s over at least 60 s, and only then a resubmit under the
same reference. Any other `404` is `Config`, as both existing adapters treat a
`404` on submit: an unknown `X-Sync` account answers `404` "Sync account not
found", and a base URL with a wrong path answers `404` on every call. Reading
either as "the payment does not exist" would resubmit a payment that may be
alive.

**A crash between calls 1 and 2 costs one payer a long wait and no money.**
Notch Pay then holds a `pending` payment, and the payer has had no prompt.
Recovery reads it, finds `pending`, and does not resubmit, because the rail
has a record. The charge waits for Notch Pay's own expiry — "Transaction has
expired after 3 hours" ([collect](https://developer.notchpay.co/accept-payments/collect))
— and fails `payer_timeout`. The window is one HTTP round trip wide. Sending
call 2 from inside the read when it finds `pending`, as RFC-0011 sends
PayPal's capture, is refused: a read that sends a USSD prompt minutes later
prompts a payer who has left.

### 4. Status mapping

The Payments API documents six statuses. The Collect page documents eleven,
including the six. The table covers all eleven, because a status the adapter
does not know is a `Malformed`, never a guess.

| Notch Pay `status`  | Notch Pay's description                             | `ChargeStatus`                                                                                                                                                                                                                                                                                       |
| ------------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pending`           | "initialized but not yet processed"                 | `Pending` — on this rail, call 2 has not landed (§ 3)                                                                                                                                                                                                                                                |
| `processing`        | "being processed by the payment provider"           | `Pending`                                                                                                                                                                                                                                                                                            |
| `incomplete`        | "Customer pay less than payment amount"             | `Pending`. **Never** `Succeeded`. Whether it expires as `pending` does is "To confirm" item 11                                                                                                                                                                                                       |
| `complete`          | "successfully completed"                            | `Succeeded { provider_txn_id: transaction.id, else transaction.reference }`, **only if** `amount`, `amounts.total` and `currency` equal the charge's; otherwise `NeedsReconciliation` (`amounts.total` and the fallback added 2026-10-21, item 18; `Malformed` until 2026-10-23, vaam-apps/vpay#313) |
| `refunded`          | "successfully refunded to customer"                 | `Succeeded`, with the same check. The payment succeeded; its refunds are read separately                                                                                                                                                                                                             |
| `partialy-refunded` | Notch Pay's own spelling                            | `Succeeded`, with the same check                                                                                                                                                                                                                                                                     |
| `failed`            | "Transaction has failed"                            | `Failed { provider_error }` with the raw status — see below                                                                                                                                                                                                                                          |
| `canceled`          | "canceled by the merchant or customer"              | `Failed { payer_declined }` since 2026-10-21 — see below                                                                                                                                                                                                                                             |
| `expired`           | "expired after 3 hours"                             | `Failed { payer_timeout }`                                                                                                                                                                                                                                                                           |
| `abandoned`         | "Customer abandoned the payment on our hosted page" | `Failed { payer_timeout }`. A push charge should never read it                                                                                                                                                                                                                                       |
| `rejected`          | "rejected by our system or by operator"             | `Failed { provider_error }`. Notch Pay's fraud system rejects, among other things, payments concentrated on one number ([testing](https://developer.notchpay.co/get-started/testing))                                                                                                                |
| anything else       | —                                                   | `Malformed`, naming the status                                                                                                                                                                                                                                                                       |

**The read names no failure reason.** The payment object in the
specification has `id`, `reference`, `amount`, `currency`, `status`,
`customer`, `created_at`, `completed_at` and `payment_method`, and nothing
saying why a payment failed. MTN's read says `NOT_ENOUGH_FUNDS` or
`APPROVAL_REJECTED`; this one, as documented, says `failed`. So `failed` is
`provider_error` until the sandbox shows otherwise. The sandbox's outcome
numbers make that measurable: a number ending `000001` is "Insufficient
funds", `000002` "Failure (other reason)", `000003` "Timeout (no response)",
`000004` "Canceled by user". The first sandbox session reads each one and
fills this table from what comes back.

**Moved 2026-10-21:** the sandbox read `canceled` for `…000004`, so the row
above is now `payer_declined`
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md)).
The paragraph keeps its original reasoning.

**`canceled` is not mapped to `payer_declined` on a verb.** Notch Pay says a
payment is canceled "by the merchant or customer", vpay never cancels one,
and the public key can (§ "What rule 3 protects"). If the sandbox reads
`canceled` for a `000004` number, that is evidence and the row becomes
`payer_declined`. Until then it is `provider_error`, which is the discipline
[adapter-mtn-momo.md](../flows/adapter-mtn-momo.md) applies to MTN's
`TRANSACTION_CANCELED`.

### 5. The `Pending` window and the poll ladder

Notch Pay's channel table gives `cm.mobile` a processing time of 5 to 30
seconds. The worker's ladder
asks at 10, 20, 30, 45, 60 and 90 s, then every 120 s to the twentieth
attempt, then every 15 minutes, then hourly once unresolved. A payer who never
answers reads `failed` or `expired`; which one Notch Pay reports for a
`000003` number decides whether that charge fails within a few rungs or after
three hours, and is part of the first sandbox session.

**Measured in the sandbox, 2026-10-22, through vpay**
([verification](../status/verification/2026-10-22-notchpay-b3b.md)):
`+237670000003` read `pending` right after submit and `expired` on the next
read, ten seconds later, so the charge failed `payer_timeout` about 12 s
after create. The sandbox's timeout number does not wait out a payer. What a
real unanswered prompt reads, and when, a test number cannot show; § 4's
"expired after 3 hours" is still Notch Pay's word. (Step 1 had seen
`expired` only by its final read, about 180 s after process: its poll loop
read by vpay's reference, which is a `404`, item 2.)

### 6. Webhooks are callbacks, and callbacks are hints

`parse_callback(body)` reads a Notch Pay event
([webhooks API](https://developer.notchpay.co/api-reference/webhooks)):

```json
{ "id": "evt_…", "type": "payment.complete", "created_at": "…",
  "data": { "id": "pay_…", "reference": "<the charge's reference>", "…": "…" } }
```

→ `CallbackRef { reference_id, ref_extra: {} }` — built with an **empty**
`ref_extra`, where this said `{ notchpay_id, event_type }` until 2026-10-20:
the route discards it anyway, and an adapter that hands nothing from an
unauthenticated request onward cannot be the one that leaks it — for
the five payment events the setup page lists: `payment.created`,
`payment.complete`, `payment.failed`, `payment.canceled` and
`payment.expired` ([webhook setup](https://developer.notchpay.co/get-started/webhooks)).
Any other `type`, a missing or non-UUID `data.reference`, or `{}` is
`Malformed`. A search engine's copy of an older page also named
`payment.initialized` and `payment.refunded`; that page now answers `404`,
no current page names either, and the adapter does not accept them on the
strength of a cache. The handler then
does what it does for every rail: pulls the charge's `poll_charge` job forward
and discards `ref_extra`. The authenticated read is the only thing that moves
money.

Three things about this rail's callbacks, none of which changes the design:

- **The URL is registered, not sent per request — and vpay registers it.**
  Call 1 has a `callback` field, but it is a browser redirect, and a push
  charge has no browser. So `{public_base_url}/provider/notchpay/callback`
  has to be registered with the account for the payment events. The
  maintainer decided on 2026-10-20 that vpay does that itself rather than
  depend on a merchant doing it by hand, through `POST /webhooks` with the
  private key; [RFC-0013](0013-vpay-registers-its-own-callbacks.md) designs
  it for every rail that registers its URL, and `vpay-notchpay` already has
  the list and create calls. Two facts from the specification shape that
  design: creating is **not idempotent** (no idempotency header, no refusal
  of a duplicate URL is documented), so vpay lists before it creates; and
  the Webhook schema carries **no signing secret**, although the API
  reference says one is returned "when creating a webhook via the API"
  ("To confirm" item 13). Until RFC-0013 lands, the merchant registers the
  URL in the dashboard (Settings > Webhooks). _(2026-10-22: RFC-0013 landed
  for Notch Pay. vpay registers the endpoint itself, from the worker's
  `ensure_callbacks` job or `vpay-server callbacks ensure`
  ([adapter-notchpay.md](../flows/adapter-notchpay.md) § Callbacks).)_ This is also RFC-0010 § 6's
  situation, and RFC-0010 § 8's change to the conformance suite's
  `callback_url_pattern` covers it; whichever adapter lands first carries
  that change.
- **Notch Pay signs its webhooks, and vpay cannot check the signature
  today.** (2026-10-20: [RFC-0014](0014-verifying-rail-callbacks.md) designs the check; it is not built.) `x-notch-signature` is a hex HMAC-SHA256, keyed by the endpoint's
  "Hash Key" (`hsk_test_…` in the sandbox)
  ([verify](https://developer.notchpay.co/get-started/webhooks/verify)).
  `parse_callback` receives the body and no headers, which is the port
  change RFC-0010's open question 4 names. Until it lands, an unverified
  Notch Pay webhook is worth one authenticated read and no money.
- **What is signed is not settled.** The verification page and the API
  reference sign the raw body; the setup page's Node example signs
  `JSON.stringify(req.body)`, a re-serialisation. The signature carries no
  timestamp, so a captured webhook verifies forever. Both are reasons the
  signature is a quality signal and not a gate, and "To confirm" item 7.

### 7. Refunds: the second `Origin` rail, built as a declared gap

Notch Pay's refunds guide documents `POST /refunds` with `payment` (a payment
id or reference), an optional `amount`, `reason` and `metadata`, authenticated
by both keys; a refund object with `pending`, `processing`, `complete` and
`failed`; reads at `GET /refunds/{id}` and `GET /payments/{id}/refunds`;
`refund.created`, `refund.complete` and `refund.failed` events; a 90-day
window; and no Notch Pay fee on a refund, though "the original
processing fees are typically not refunded"
([refunds](https://developer.notchpay.co/accept-payments/refunds)).

The first build does not call it, for two reasons:

1. **It is in a guide and nowhere else.** The specification has no
   `/refunds` path and the API reference has no refunds page.
2. **It has no duplicate protection.** The request carries no reference of
   vpay's, and there is no idempotency header. vpay addresses every movement
   on a rail by a reference generated before the call
   ([crash-safety.md](../flows/crash-safety.md)), and the conformance case
   `a_duplicate_refund_reference_is_accepted_and_never_paid_twice` demands
   that a replay never pays twice. Without that, a lost response re-sent is a
   second refund. Listing a payment's refunds and matching
   `metadata.vpay_refund` before posting narrows the window and does not close
   it.

So the adapter declares the rail's truth — `supports_refunds: true`,
`supports_partial_refunds: true`, `refund_destination: Origin` — and
overrides `refund` with `ProviderError::NotImplemented("notchpay::refund")`,
declared in [docs/status.md](../status.md), exactly as
`orange_money::refund` was declared on 2026-09-15. It becomes a call when
Notch Pay confirms in writing a refund that is safe to replay, and the
sandbox answers `POST /refunds`. _(On 2026-10-22 the sandbox did not: it
answered `403` "Forbidden" to every request, and `GET /payments/{id}/refunds`
answered `404` for a payment `GET /payments/{id}` had just read
([verification](../status/verification/2026-10-22-notchpay-refunds.md)).)_

The core half is already there. RFC-0010's delivery step 1 landed on
2026-10-17: `refund_at_rail` hands an `Origin` adapter the charge's recorded
`provider_txn_id` under `vpay_api::v1::refunds::REFUND_PROVIDER_TXN_ID_KEY`,
which on this rail is the `pay_…` id § 4 records. `fee` will be `None`: the
guide's refund object has no fee field.

### 8. Proving it

- **Unit tests in the crate**, no container: the mapping table row by row,
  including an unknown status and a `complete` whose amount or currency
  differs, both `Malformed`; the two-call submit and its `409` path; the
  livemode key rule; and a `Debug` canary on every type that holds a key.
- **WireMock mappings** under `backends/tests/conformance/wiremock/notchpay/mappings/`,
  transcribed from openapi.yaml 2.1.0 and steered the way Notch Pay's own
  sandbox is steered: by the payer number's last digits. The `409` and the
  `422` "already processed" come from the errors page and are marked so.
- **The shared conformance suite, unchanged in its cases.** One
  `RailUnderTest::NotchPay` row in each table function. The push cases run as
  they run for `mtn_momo`. The refund cases meet a `NotImplemented` token,
  which `unimplemented_operations_never_fabricate_success` is for.
- **The first sandbox session, before any claim about Notch Pay rather than
  about the stub**: the five outcome numbers, each read and each webhook, and
  "To confirm" items 2 to 5, 7 and 11, on a dated verification page — the
  2026-09-15 MTN precedent.

### 9. What the operator and the merchant take on

#### Production risks

Added 2026-10-20 with
[ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md),
which admitted this rail on the amended rule 3. **Whoever holds the
merchant's public key can, on that merchant's Notch Pay account: read the
status, amount and customer of any payment; initialize a payment; process a
pending payment to any phone number, which sends a USSD prompt in the
merchant's name; and cancel any pending payment, vpay's included.** vpay
keeps the key server-side, sends the private key on every call, refuses to
boot without it, accepts a success only for the charge's own amount and
currency, and logs this exposure at `WARN` on every boot of a deployment
that configures the rail, louder in livemode. It cannot do more: a key that
leaves the server, or that the merchant embeds in a front end as Notch Pay's
documentation invites, is beyond vpay's reach.
[adapter-notchpay.md](../flows/adapter-notchpay.md) § "Production risks" is
the operator's copy, and
[threat-models/notchpay.md](../security/threat-models/notchpay.md) the
STRIDE model.

In livemode boot also refuses the rail until its block carries
`settings.accept_public_key_exposure: true` (§ 2; ADR-0031 condition 4, as
the maintainer amended it on 2026-10-20: "Livemode acknowledgement: YES,
require it."). Setting it admits the rail; it does not silence the warning.

- **An account per merchant.** Opened online, with Notch Pay's KYC. The
  agreement reserves the service to "natural persons aged 18 and over,
  residing in French-speaking Africa"; a company should get Notch Pay's
  written word that it may hold an account ("To confirm" item 10).
- **Fees, the merchant's contract.** 2 % per local payment and 1 % per
  withdrawal to a mobile-money account, from the pricing page, which does not
  name a country. No setup or monthly fee. Under ADR-0029 D8 these are the
  merchant's fee schedule on the provider account.
- **Holds.** "Funds relating to a specific transaction will be held during the
  settlement period" (agreement § 8.4), with no period stated, and "specific
  conditions may apply, such as reserves" (§ 20).
- **Licensing.** Unpublished. It is the operator's due diligence, per ADR-0029
  D1, and the questions are in item 10.
- **The fraud system.** Active in the sandbox and in production; it rejects
  payments concentrated on one number or repeated amounts. A merchant should
  expect some `rejected` charges and read them as `provider_error`.
- **The webhook endpoint**, registered by vpay once RFC-0013 lands, with the
  account's private key; by the merchant in their dashboard until then
  (§ 6). _(2026-10-22: RFC-0013 landed. vpay registers it, with both keys as
  on every call. A merchant who registered one by hand gets a second; see
  [adapter-notchpay.md](../flows/adapter-notchpay.md) § Callbacks.)_
- **The public key stays server-side.** A merchant who also embeds it in a
  front end has exposed every public-key operation listed under "The gate".

### 10. To confirm with Notch Pay

In writing (W) or in the sandbox (S):

1. **(W)** Does `GET /payments/{reference}` accept `X-Grant`, and can an
   account make it required? Can the public key's payment operations be
   disabled? This is the gate.
2. **(S)** Does the payments path take the merchant's reference, Notch Pay's
   `trx.…` reference, or both? _(Contradicted [2026-10-21][sandbox-run] — Notch
   Pay's `trx.…` only. Ours is `404` "Payment Not Found" on process and read; it
   comes back as `trxref` and `merchant_reference`. The adapter was fixed
   the same day: it keeps the `trx.…` as `ref_extra` key material
   ([verification](../status/verification/2026-10-21-notchpay-trx-reference.md)).)_
3. **(S)** Does `cm.mobile` resolve the network from the number, and do the
   outcome numbers work under it? _(Answered [2026-10-21][sandbox-run] — yes, for
   MTN and Orange numbers.)_
4. **(S)** Is a repeated `reference` on `POST /payments` answered `409`, and
   does the read then return the first payment? _(Answered
   [2026-10-21][sandbox-run] — `409` "Reference already existing". The read by
   our reference is `404`, as every read by our reference is: item 2.)_
5. **(S)** What the read returns for each outcome number: the status, and any
   field naming a reason. _(Answered [2026-10-21][sandbox-run] — `complete`,
   `failed`, `expired`, `canceled`; no reason field. **The "insufficient funds"
   number read `complete`** with `amounts.total` 105 of `amount` 107: item 18.
   Through vpay, all six numbers since [2026-10-22][b3b]: `…000003` became
   `payer_timeout` and `…000004` `payer_declined`, and the Orange success
   `succeeded`, each on the worker's second read, about 12 s after create.)_
6. **(W, S)** Does `POST /refunds` exist on the live API, how is it made safe
   to replay, and which events does it fire? _(Not answered
   [2026-10-22][refunds-probe] — in the sandbox, `GET /refunds` answers
   `200` with an empty list, and `POST /refunds` answers `403` "Forbidden",
   with no message, to all four requests sent against a `complete` payment:
   the guide's body twice, and twice with an undocumented `reference`. No
   refund was made, so replay safety is not measured.)_
7. **(S)** Is the webhook signature over the raw body, and is the header
   always lower-case? _(Not measured 2026-10-21: no public URL to receive
   one.)_
8. **(W)** What prefix does a live key carry? _(Its neighbour, the test
   key's shape, was measured on 2026-10-20: `test` followed by `.`, not the
   documented `test_`; § 2 carries the correction. The live prefix is still
   unknown.)_
9. **(W)** On Sync's direct charges, does the money land in the connected
   account's balance or the platform's?
10. **(W)** May a company hold an account, despite agreement § 3? Which legal
    entity operates Notch Pay, under which licence and regulator, with which
    partner bank?
11. **(W)** Does an `incomplete` payment expire as a `pending` one does?
12. **(W)** Do refunds require an allowlisted IP, as transfers do? _(Not
    answered [2026-10-22][refunds-probe] — the sandbox's `403` fits an
    allowlist, and fits refunds being disabled for the account just as well;
    it names neither.)_
13. **(S)** Does `POST /webhooks` return the endpoint's signing secret, as the
    API reference says and the specification's schema does not? _(Contradicted
    [2026-10-21][sandbox-run] — no secret, in the create answer or the list.)_
14. **(S)** Does `POST /webhooks` refuse a URL the account already has, or
    create a second endpoint for it? _(Answered [2026-10-21][sandbox-run] — a
    second endpoint.)_
15. **(S)** Is an answer's payload under the specification's keys
    (`transaction`, `endpoint`, `endpoints`) or under `data`, as the API
    reference pages show it? The client follows the specification, so a
    `data`-shaped answer is `Malformed` until this is settled. _(Answered
    [2026-10-21][sandbox-run] — the specification's keys.)_
16. **(S)** Does a webhook endpoint carry an `active` flag, as the API reference
    page shows and the specification does not? _(Contradicted
    [2026-10-21][sandbox-run] — no `active`; a `status`, `null` on create and
    `"active"` in the list.)_
17. **(S)** Is `payment.processing` a real event? The API reference lists it;
    the specification's `createWebhook` enum does not. _(Half answered
    [2026-10-21][sandbox-run] — `POST /webhooks` accepts it. Delivery not
    measured.)_
18. **(W)** Why does the sandbox's "insufficient funds" number read `complete`
    with `amounts.total` short of `amount`? On the live API, can a payment read
    `complete` having collected less, and is `amounts.total` the collected
    amount? _(Added 2026-10-21. The adapter refuses such a success meanwhile:
    as `Malformed` until 2026-10-23, and since then as `NeedsReconciliation`,
    which the worker escalates to `unresolved` on the first read
    (vaam-apps/vpay#313).)_
19. **(S, W)** Does `GET /payments?search=` find a payment by the merchant's own
    reference? It decides whether carrying Notch Pay's reference in `ref_extra`
    (item 2's fix) can survive a crash between initialize and `ref_extra`
    becoming durable. _(Added and answered 2026-10-21 — no: `search` matches
    Notch Pay's `trx.…` only, not ours and not the description, and list
    items carry no `trxref`. The crash window is a residual risk.)_
20. **(W)** Why does `DELETE /webhooks/{id}` answer `404` "Resource Not Found"
    for an endpoint the list returns? _(Added 2026-10-21.)_
21. **(W)** Where is a webhook endpoint's signing secret, since `POST /webhooks`
    does not return one? _(Added 2026-10-21.)_

[sandbox-run]: ../status/verification/2026-10-21-notchpay-sandbox-run.md
[step5]: ../status/verification/2026-10-21-notchpay-step5.md
[refunds-probe]: ../status/verification/2026-10-22-notchpay-refunds.md
[b3b]: ../status/verification/2026-10-22-notchpay-b3b.md

## Delivery

| Step | Contents                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Depends on                            |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| 0    | **Rule 3 resolved**: Notch Pay's written answer to item 1, or an ADR amending ADR-0029 D1's third rule. **Done 2026-10-20: ADR-0031**                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | the maintainer; Notch Pay             |
| 1    | The first sandbox session: items 2–5, 7 and 11 on a dated verification page. No code. **Attempted 2026-10-20, no request made**; the key shape was measured and contradicted § 2 ([page](../status/verification/2026-10-20-notchpay-sandbox.md)). **Done 2026-10-21** from a cloud session, with the probe's cloud-only `--ca-bundle`: 3, 4, 5 answered, 2 contradicted, 7 and 11 not measured ([page](../status/verification/2026-10-21-notchpay-sandbox-run.md))                                                                                                                                                  | a test-mode account; may run before 0 |
| 1b   | `vpay-notchpay`, the client: initialize, process and read a payment, list and create webhooks, parse an event; unit tests. **Landed 2026-10-20**, ahead of step 0 at the maintainer's direction; linked by nothing                                                                                                                                                                                                                                                                                                                                                                                                  | nothing                               |
| 2    | `vpay-adapter-notchpay` on top of `vpay-notchpay`: `mapping.rs` filled from step 1, `lib.rs`, `refund` as a declared `NotImplemented` token; unit tests. **Built 2026-10-20 without step 1**: no test-mode key was available, so `mapping.rs` is filled from `openapi.yaml` 2.1.0 and every row is "To confirm"                                                                                                                                                                                                                                                                                                     | 0, 1                                  |
| 3    | Mappings, the conformance rows, and RFC-0010 § 8's `callback_url_pattern` change if RFC-0010 has not landed it. **Built 2026-10-20**; this rail carried the `callback_url_pattern` change                                                                                                                                                                                                                                                                                                                                                                                                                           | 2                                     |
| 4    | Wiring: `adapters()`/`adapter_codes()`, `REQUIRED_RAIL_KEYS`, `compose.yml` (`wiremock-notchpay`), `.env.example`, Helm, `config/application.yml`, `docs/api/`, both SDKs' value lists, `sdks/parity.md`, `examples/shop`'s crate map, `docs/flows/adapter-notchpay.md`, `docs/flows/failures.md`, the privacy inventory's recipient, `docs/status.md`'s token, the status pages. **Built 2026-10-20**, except that `config/application.yml` carries the block commented out and `examples/shop` gets a note rather than a test-number table (both in [adapter-notchpay.md](../flows/adapter-notchpay.md) § Status) | 3                                     |
| 5    | The first test-mode call through the adapter, on a dated verification page. **Not done 2026-10-20**: no test-mode key was available when the adapter landed, and the session that had one could not reach Notch Pay. Runs from [the runbook](../runbooks/notchpay-sandbox-test.md), Part B (`--profile notchpay-sandbox`); written, not yet run. **Done 2026-10-21** on the maintainer's machine: a success, a failure, and a short collection refused as `Malformed` and left `pending`; every charge held Notch Pay's `trx.…` reference ([page](../status/verification/2026-10-21-notchpay-step5.md))             | 4; test-mode keys                     |
| 6    | Refunds as a call                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | item 6 answered                       |
| 7    | Model B: `X-Sync`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | RFC-0009; item 9 answered             |
| 8    | `cm.mtn` and `cm.orange` from the payer's chosen method type                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | ADR-0029 D6 landed                    |

Steps 2–5 are one pull request against `master` and one against
`vpay-skills`.

## Alternatives considered

- **Notch Pay's hosted page (Collect), a `Redirect` rail.** It carries every
  channel, Express Union and Yoomee included, and cards on the day they work.
  But the payer leaves the merchant's checkout to choose a network vpay could
  have asked for, Collect adds `abandoned` and a return URL, and nothing on it
  is documented that the push charge lacks for mobile money. Not chosen
  first. When Notch Pay documents a card, a provider with two flows is the
  port's question, and it belongs with ADR-0029 D6.
- **Two adapters, one per flow.** ADR-0029 D1 is "one adapter per provider".
- **Choosing `cm.mtn` or `cm.orange` from the number's prefix.** A
  number-plan table in the adapter is wrong the day a prefix moves.
  `cm.mobile` makes the network Notch Pay's problem.
- **Sending call 2 from inside the read on `pending`.** Refused in § 3.
- **Refunds by listing first and matching metadata.** Narrows the duplicate
  window and does not close it (§ 7).
- **Building now under the public key.** Way B done silently (§ "The gate").

## Open questions

1. **~~Way A or way B for rule 3.~~ Decided 2026-10-20: way A**, asking
   Notch Pay in writing, **then way B the same day**, by
   [ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md),
   because Notch Pay had not answered. Notch Pay's answer is still worth
   having: a yes moves the rail to ADR-0031's first limb and retires the boot
   warning. _(This said "The maintainer's decision, and delivery step 0" until
   the same day, and then "What remains open is Notch Pay's answer, and it is
   delivery step 0" until the ADR.)_
2. **Two entries for one network.** Until ADR-0029 D8–D10 land, a deployment
   with `mtn_momo` and `notchpay` both enabled shows an MTN payer two rails by
   code. Is that acceptable for the interval, or should a deployment enable
   one of them per network until routing exists?
3. **Beyond Cameroon.** Express Union, Yoomee and the XOF countries are
   `settings.channel` and `currency` values, and each needs its own outcome
   numbers in the sandbox. Out of scope here.
4. **~~Registering the webhook at boot.~~ Decided 2026-10-20 by the
   maintainer: vpay registers its own endpoints**, so that no deployment
   depends on a hand-registered webhook. [RFC-0013](0013-vpay-registers-its-own-callbacks.md)
   designs when, by which process, and how a duplicate is avoided. _(This
   said "Recommended against: a boot that writes to a provider is a side
   effect no other rail has" until the same day. The objection is real and
   is answered there rather than dropped: the registration runs in the
   worker, idempotently, and a failure never stops a process from booting.)_

## Impact on existing invariants

- **Rails behind the port** holds with **no core change**: the `Origin`
  refund change this rail needs landed with RFC-0010's step 1.
- **Branch on capability values, never on a provider code** holds.
- **Callbacks are hints** holds, and on this rail it is the only defence
  against an unverified webhook (§ 6).
- **Crash safety** holds. A crash between calls 1 and 2 costs one payer a
  three-hour wait and no money (§ 3).
- **One charge per intent** holds.
- **No mocks in shipping processes** holds: the stub is a WireMock host in
  configuration.
- **Pass-through** holds on model A. Model B is unconfirmed and not offered.
- **ADR-0029 D1's third hard rule, as ADR-0029 wrote it, does not hold** on
  Notch Pay's documentation. The adapter is admitted under the rule as
  [ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md)
  amended it, on that ADR's six conditions. _(This bullet ended "and that is
  why this RFC is not buildable as written" until 2026-10-20.)_
