# Adapter: Notch Pay (mobile money through an aggregator)

`notchpay` is a **push** rail: the payer is prompted on their own handset, the
charge is addressed on Notch Pay by the reference vpay generated, and the
authenticated read is what moves money. Notch Pay is an aggregator, so one
merchant account collects MTN MoMo and Orange Money in Cameroon through the
`cm.mobile` channel. Design: [RFC-0012](../rfc/0012-notchpay-adapter.md).
Code: `backends/crates/vpay-adapter-notchpay` on the client
`backends/crates/vpay-notchpay` ([reference](../reference/vpay-notchpay.md)).

> **Nothing here has called Notch Pay.** Every call, status and error below
> is transcribed from Notch Pay's documentation and its `openapi.yaml`
> **2.1.0** (read 2026-10-20). The adapter is proven against a WireMock stub
> transcribed from the same pages. Read § "To confirm with Notch Pay" before
> trusting any row.

## Preconditions

- A Notch Pay account per merchant (model A, the merchant's own keys). Until
  [RFC-0009](../rfc/0009-provider-accounts-per-merchant.md) lands, a
  deployment serves one merchant.
- Both of the account's keys: the **public** key and the **private** key.
  Boot refuses a `notchpay` block without either.
- The webhook endpoint `{public_base_url}/provider/notchpay/callback`
  registered by hand in Notch Pay's dashboard (Settings › Webhooks) for the
  five payment events, until [RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md)
  registers it. Optional for correctness: the poll ladder settles every
  charge without callbacks.

## Admission: ADR-0031, not rule 3 as written

Notch Pay's `GET /payments/{reference}` is documented as authenticated by the
key Notch Pay calls **public**. ADR-0029 D1's third rule, as written, refused
an adapter to such a provider.
[ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md)
amended that rule on 2026-10-20, for every provider, after the maintainer
emailed Notch Pay and got no answer. `notchpay` is **admitted under ADR-0031
(rule 3 amended)**. It does not pass rule 3 as ADR-0029 wrote it.

The six conditions ADR-0031 puts on that admission, and where each is met:

| Condition                                          | Here                                                                                                                                                                                                                      |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1. Every secret is sent on every call              | `vpay_notchpay::Client` sends `X-Grant: <private_key>` on every request; the stub's mappings match only requests that carry it                                                                                            |
| 2. Boot refuses a configuration without the secret | `vpay_config::config::REQUIRED_RAIL_KEYS`, `notchpay` row: `public_key` and `private_key`                                                                                                                                 |
| 3. The exposure is written where an operator reads | § "Production risks" below; RFC-0012 § 9; comments beside the keys in `config/application.yml`, `.env.example` and `deploy/helm/vpay/values.yaml`                                                                         |
| 4. The process says it too                         | `ProviderAdapter::accepted_exposure`, logged by `vpay_api::boot::warn_about_accepted_exposures` at every boot of `serve` and `worker`; in livemode, boot also refuses without `settings.accept_public_key_exposure: true` |
| 5. Integrity is checked on the answer              | a success is `Succeeded` only for the charge's own amount and currency; otherwise `Malformed`                                                                                                                             |
| 6. The written question stays open                 | RFC-0012 § 10 item 1, unanswered                                                                                                                                                                                          |

## Production risks

**Read this before taking real payments on this rail.**

Whoever holds the merchant's **public key** can, on that merchant's Notch Pay
account, without vpay ever learning of it:

1. **read** the status, amount and customer of any payment;
2. **initialize** a payment;
3. **process** a pending payment to **any** phone number, which sends that
   number a USSD prompt in the merchant's name;
4. **cancel** any pending payment, including one of vpay's. That charge then
   reads `canceled`, and vpay records it `provider_error`. It never records
   `payer_declined`, because the cancel may not have been the payer's.

vpay keeps the key server-side, sends the private key beside it on every
call, and never hands either to a browser or an app. That is all vpay can do.
The exposure is the merchant's, and it becomes real the moment the public key
leaves the server. Notch Pay's own documentation invites exactly that: it
calls public keys safe to include "in your frontend code". **A merchant who
embeds their Notch Pay public key in any front end has given every operation
above to anyone who loads that page.**

What else an operator takes on, from RFC-0012 § 9:

- **Webhooks are unverified.** Notch Pay signs them (`x-notch-signature`), and
  the port gives `parse_callback` no headers, so vpay cannot check the
  signature. A forged webhook costs one authenticated read and moves no money.
  [RFC-0014](../rfc/0014-verifying-rail-callbacks.md) (a draft) designs the check; it is not built.
- **Licensing is unpublished.** No Notch Pay page names a legal entity, a
  licence, a regulator or a partner bank. That is the operator's due
  diligence (ADR-0029 D1), and § 10 item 10 asks.
- **A company's eligibility is unconfirmed.** The merchant agreement § 3
  reserves the service to natural persons.
- **Holds and reserves** are possible and their periods are unstated
  (agreement §§ 8.4, 20).
- **Notch Pay's fraud system** rejects some payments, for example ones
  concentrated on one number. Expect some `rejected` charges, mapped to
  `provider_error`.

Every boot of a deployment that configures `notchpay` logs this risk at
`WARN`. In livemode the line opens with `LIVEMODE, REAL MONEY`. It never
prints a key. **In livemode, boot refuses the rail until its block says
`settings.accept_public_key_exposure: true`** (ADR-0031 condition 4, as the
maintainer amended it on 2026-10-20: "Livemode acknowledgement: YES, require
it."). Setting it is the operator's written acceptance of every risk above. It
does not silence the warning, and in a sandbox it is optional and changes
nothing.

## Configuration

```yaml
providers:
  - code: notchpay
    host:
      url: https://api.notchpay.co # one host for sandbox and production
      label: notchpay
    currency: XAF # the only one allowed
    settings:
      channel: cm.mobile # the only one allowed today
      description: Payment # optional; the one free-text field Notch Pay receives
      accept_public_key_exposure: true # livemode refuses to boot without it; see Production risks
    credentials:
      public_key: ${NOTCHPAY_PUBLIC_KEY}
      private_key: ${NOTCHPAY_PRIVATE_KEY}
```

Boot refusals, all in `vpay_config::config::REQUIRED_RAIL_KEYS`:

- `credentials.public_key` and `credentials.private_key` present and
  non-blank;
- `settings.channel` exactly `cm.mobile`. `cm.mtn` and `cm.orange` wait for
  the payer's chosen method type to reach the adapter (ADR-0029 D6, RFC-0012
  step 8). Configured for a whole deployment, either would fail every payer on
  the other network;
- `currency` exactly `XAF`;
- **the livemode rule is on the keys**: in livemode a key containing `test.`
  or `test_` refuses to boot, and outside livemode a key with neither does.
  Notch Pay serves its sandbox and its production from one host, so the host
  rule cannot tell them apart. The error names the key and never prints it.
  _(Corrected 2026-10-20: this read "`test_`" alone, as RFC-0012 § 2 did.
  The maintainer's real test-mode keys carry `test` followed by `.` and no
  `test_`, so that rule refused every genuine sandbox key outside livemode
  and admitted one in livemode. `test_` stays because Notch Pay's
  documentation spells it so. See
  [the sandbox page](../status/verification/2026-10-20-notchpay-sandbox.md).)_;
- **in livemode, `settings.accept_public_key_exposure` exactly `true`**
  (ADR-0031 condition 4). Absent or `false`, boot refuses, naming the setting
  and `credentials.public_key` and never a value. Outside livemode it may be
  absent. In either mode a value other than `true` or `false` is refused
  rather than guessed at.

## The calls

Every call carries `Authorization: <public_key>`, `X-Grant: <private_key>`
and `Accept: application/json`, and the configured request deadline. The
client follows no redirect: a `3xx` is `Malformed`.

**Submit** is two calls (RFC-0012 § 3):

1. `POST /payments` with `amount` (XAF has no minor unit), `currency: XAF`,
   `phone` (the payer's number, `+`-prefixed E.164), `reference` (the
   charge's own UUID) and the optional `description`. Nothing else of the
   payer's is sent: no email, no name, no callback, no return URL.
2. `POST /payments/{reference}` with `{ "channel": "cm.mobile", "data": {
"phone": … } }`. This is the call that prompts the payer.

| Call 1 answered                  | Then                                                                                                |
| -------------------------------- | --------------------------------------------------------------------------------------------------- |
| `201`, `pending`                 | call 2. `202` is `Submitted`, with no redirect and **no key material**                              |
| `201`, another documented status | `Submitted`; the poll resolves it                                                                   |
| `409` (reference already exists) | the crash-retry case: read it. `pending` → call 2; another status → `Submitted`; none → `Malformed` |
| `400`, `422`                     | `Rejected { provider_error }` with Notch Pay's message and the names of the fields it refused       |
| `401`, `403`                     | `Rejected { provider_account_blocked }`, which pages                                                |
| `404`                            | `Config`                                                                                            |
| `408`, `429`, `5xx`, no answer   | `Transport`: the fate is unknown, never a decline                                                   |

Call 2's `422` "Payment has already been processed" is not a decline. The
adapter reads the payment and answers `Submitted`, unless the read still says
`pending`, which is `Malformed`.

**The read** is `GET /payments/{reference}`, by **our** reference. That is how
the specification's example addresses it. Notch Pay's direct-charge guide
passes its own `trx.…` reference instead, and which one the live API takes
is § 10 item 2. RFC-0012 § 3 had the adapter prefer Notch Pay's reference
when it held one. That was dropped on 2026-10-20: one rule survives a crash
unchanged, needs no `ref_extra`, and is the one the specification shows.

| The read answered                           | `ChargeStatus`                                                                                                           |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `pending`, `processing`, `incomplete`       | `Pending`. `incomplete` (paid less than asked) is never a success                                                        |
| `complete`, `refunded`, `partialy-refunded` | `Succeeded { provider_txn_id: transaction.id }`, **only if** amount and currency are the charge's; otherwise `Malformed` |
| `failed`, `canceled`, `rejected`            | `Failed { provider_error }`, the status word as `raw`                                                                    |
| `expired`, `abandoned`                      | `Failed { payer_timeout }`                                                                                               |
| any other status, or none                   | `Malformed`, naming it                                                                                                   |
| `404` "Payment not found"                   | `NotFound`: never on its own a failure ([crash-safety.md](crash-safety.md))                                              |
| any other `404`                             | `Config`: a wrong base path or an unknown Sync account                                                                   |
| `401`, `403`                                | `Rejected { provider_account_blocked }`                                                                                  |

**The read names no failure reason** in Notch Pay's specification. A payer
with no funds and a payer who never answered both read `failed`. So
`insufficient_funds` and `payer_declined` are not produced by this rail
(`PRODUCED_FAILURE_CODES` is `payer_timeout`, `provider_error`,
`provider_account_blocked`). The sandbox's outcome numbers are what will show
whether that is true of the live API.

## Callbacks

`parse_callback` reads Notch Pay's event `{ id, type, created_at, data }` and
returns `data.reference` as the charge reference, with an **empty**
`ref_extra`. It refuses, as `Malformed`, any `type` but the five payment
events (`payment.created`, `.complete`, `.failed`, `.canceled`, `.expired`),
a missing or non-UUID `data.reference`, and `{}`. The event's own status is
not read: the client does not model it. The route then pulls the charge's
poll forward, and only the authenticated read moves money.

## Refunds

`supports_refunds` and `supports_partial_refunds` are `true` and
`refund_destination` is `Origin`, because Notch Pay's refunds guide documents
both. `refund` answers the declared token `NotImplemented("notchpay::refund")`
([status.md](../status.md)). The guide's `POST /refunds` is absent from the
specification and carries no reference that would make a replay safe, so a
lost answer re-sent would pay twice. It becomes a call when Notch Pay
confirms a replay-safe refund in writing (§ 10 item 6).

## To confirm with Notch Pay

RFC-0012 § 10 is the full list of seventeen. The ones that decide whether the
adapter is right:

1. **(W)** Can the read require the private key, and can the public key's
   payment operations be disabled? A yes retires § "Production risks" items
   1–4 and the boot warning.
2. **(S)** Does the payment path take our reference, Notch Pay's `trx.…`, or
   both?
3. **(S)** Does `cm.mobile` resolve the network from the number?
4. **(S)** Is a repeated reference answered `409`?
5. **(S)** What does the read say for each sandbox outcome number (`…000001`
   insufficient funds, `…000002` other failure, `…000003` timeout, `…000004`
   canceled)? Any reason field?
6. **(W, S)** Is `POST /refunds` real, and how is it made safe to replay?
7. **(S)** Is the webhook signature over the raw body?
8. **(W)** What prefix does a live key carry?
9. **(W)** Does an `incomplete` payment expire as a `pending` one does?
10. **(S)** Are answers under `transaction`, as the specification says, or
    under `data`, as some API reference pages show? A `data`-shaped answer
    is `Malformed` today.

Also open, and not in the adapter's hands: whether Notch Pay accepts a number
without the `+`. The adapter sends the `+` form the documentation shows.

## Status

**Built 2026-10-20, never called against Notch Pay.**

- **Built and tested in process:** the adapter (`vpay-adapter-notchpay`, its
  unit tests in `src/tests.rs` and `src/mapping.rs`); the boot rules
  (`vpay-config`, the `notchpay` tests in `config.rs`); the port's
  `accepted_exposure` and its forward through `Measured`; the boot warning
  (`vpay-api`'s `boot.rs`); the client's per-request deadline.
- **Proven against WireMock:** the shared conformance suite's
  `RailUnderTest::NotchPay` row, against
  `backends/tests/conformance/wiremock/notchpay/mappings/`. This proves the
  adapter against a stub transcribed from Notch Pay's specification. It
  proves nothing about Notch Pay.
- **Not done:** RFC-0012 delivery step 1 (the first sandbox session) and step
  5 (the first test-mode call through the adapter). No test-mode key was
  available when the adapter landed. A session later on 2026-10-20 had the
  keys and **still made no request**: its permission policy refused the
  calls, and its egress re-terminates TLS, which vpay's vendored-roots client
  rightly rejects. It did measure one thing: the real test keys carry `test.`,
  not `test_`, and the boot rule is corrected for it
  ([status/verification/2026-10-20-notchpay-sandbox.md](../status/verification/2026-10-20-notchpay-sandbox.md)).
  Until a dated verification page records steps 1 and 5, the
  Collect and Status read cells of [status/providers.md](../status/providers.md)
  say "stub-only", never "sandbox".
- **Declared gaps:** `notchpay::refund` (token); model B, Sync, `X-Sync` (not
  offered); `cm.mtn` / `cm.orange` (step 8); webhook signature verification
  ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md), a draft);
  vpay registering its own webhook (RFC-0013).
- **Not wired into any running stack.** `config/application.yml` carries the
  `notchpay` block **commented out**, pointed at the `wiremock-notchpay`
  compose service. Uncommented, even with `enabled: false`, it would make
  two more non-blank secrets mandatory for every deployment of the image,
  because boot checks every configured rail. No e2e or demo walk touches it,
  and `examples/shop` shows no test numbers for it. What boots a Notch Pay
  configuration end to end is `vpay-server`'s
  `a_notchpay_rail_is_warned_about_at_boot_and_its_keys_are_never_printed`,
  which runs the real binary on `tests/fixtures/notchpay-config.yml` and
  reads the warning off its output.
