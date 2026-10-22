# Adapter: Notch Pay (mobile money through an aggregator)

`notchpay` is a **push** rail: the payer is prompted on their own handset, the
payment is addressed on Notch Pay by **Notch Pay's own reference** (`trx.…`,
held in `ref_extra` as `notchpay_reference`), and the authenticated read is
what moves money. Notch Pay is an aggregator, so one
merchant account collects MTN MoMo and Orange Money in Cameroon through the
`cm.mobile` channel. Design: [RFC-0012](../rfc/0012-notchpay-adapter.md).
Code: `backends/crates/vpay-adapter-notchpay` on the client
`backends/crates/vpay-notchpay` ([reference](../reference/vpay-notchpay.md)).

> **The adapter has called Notch Pay's real sandbox, and only its sandbox.**
> Its calls were transcribed from Notch Pay's documentation and `openapi.yaml`
> **2.1.0** (read 2026-10-20). On 2026-10-21 Notch Pay's sandbox answered
> RFC-0012 § 10's sandbox items through `vpay-notchpay`'s probe (step 1)
> ([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md)).
> **It contradicted the adapter's addressing:** process and read take Notch
> Pay's `trx.…` reference, and ours is a `404`. Fixed on 2026-10-21: the
> adapter keeps the `trx.…` from the initialize answer and addresses both by
> it. The same day, step 5 took three test-mode payments through the adapter
> itself, from the maintainer's machine: one succeeded, one failed, and one
> read `complete` for less than its amount and was refused, never a success,
> and the `trx.…` addressing worked on all three
> ([verification](../status/verification/2026-10-21-notchpay-step5.md)).
> Production has never been called, no webhook has been received, and
> `refund` is a token. Read § "To confirm with Notch Pay" before trusting
> any row.
>
> _(This banner opened "The adapter has never called Notch Pay" and said
> "That fix has never met Notch Pay either (step 5)" until 2026-10-21, when
> step 5 ran.)_

## Preconditions

- A Notch Pay account per merchant (model A, the merchant's own keys). Until
  [RFC-0009](../rfc/0009-provider-accounts-per-merchant.md) lands, a
  deployment serves one merchant.
- Both of the account's keys: the **public** key and the **private** key.
  Boot refuses a `notchpay` block without either.
- The webhook endpoint `{public_base_url}/provider/notchpay/callback`
  registered by hand in Notch Pay's dashboard (Settings › Webhooks) for the
  five payment events, until [RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md)
  registers it. Optional for correctness: the poll ladder reads every
  charge without callbacks. _(This said "settles every charge" until
  2026-10-22. A charge whose reads the adapter refuses is never settled by
  them: step 5's short collection was still `pending` after fifteen minutes
  ([verification](../status/verification/2026-10-21-notchpay-step5.md)), and by
  the code, not observed, it becomes `unresolved` at the 24-hour horizon.)_

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
   reads `canceled`, and vpay records it `payer_declined` (the mapping below):
   Notch Pay's `canceled` carries no reason and no actor, so vpay cannot tell a
   cancel by the key's holder from the payer declining. A cancel by someone
   else is therefore recorded as the payer's decline.

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
2. `POST /payments/{notchpay_reference}`, where `notchpay_reference` is the
   `trx.…` call 1 returned in `transaction.reference`, with `{ "channel":
"cm.mobile", "data": { "phone": … } }`. This is the call that prompts the
   payer.

| Call 1 answered                                                   | Then                                                                                                                |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `201`, `pending`                                                  | call 2 by `notchpay_reference`. `202` is `Submitted`, with no redirect and `notchpay_reference` as its key material |
| `201`, another documented status                                  | `Submitted` with `notchpay_reference`; the poll resolves it                                                         |
| `201` with no Notch Pay reference, or a `trxref` that is not ours | `Malformed`; nothing is sent to the payer                                                                           |
| `409`, the charge holds `notchpay_reference`                      | the crash-retry case: read it. `pending` → call 2; another status → `Submitted`; none → `Malformed`                 |
| `409`, the charge holds none                                      | `Malformed`: the fate is unknown, never `Submitted`, never a decline (below)                                        |
| `400`, `422`                                                      | `Rejected { provider_error }` with Notch Pay's message and the names of the fields it refused                       |
| `401`, `403`                                                      | `Rejected { provider_account_blocked }`, which pages                                                                |
| `404`                                                             | `Config`                                                                                                            |
| `408`, `429`, `5xx`, no answer                                    | `Transport`: the fate is unknown, never a decline                                                                   |

Call 2's `422` "Payment has already been processed" is not a decline. The
adapter reads the payment and answers `Submitted`, unless the read still says
`pending`, which is `Malformed`.

**The read** is `GET /payments/{notchpay_reference}`, and the answer must
name the charge's own reference as its `trxref`, or it is `Malformed`. A
charge with no `notchpay_reference` is `Config`, before any request: that is
key material vpay lost, not a payment Notch Pay never heard of, so it is
never `NotFound` (the same rule as Orange's missing `pay_token`).

Until 2026-10-21 every call addressed the payment by **our** reference, as
the specification's example does. Notch Pay's sandbox answered `404`
"Payment Not Found" to every process and read by it, and accepted only its
own `trx.…` (§ 10 item 2, contradicted). RFC-0012 § 3 had designed "prefer
Notch Pay's reference when held"; the build dropped it on 2026-10-20 for a
rule that needed no `ref_extra`, and the sandbox overturned the build.

**The crash window this leaves.** Notch Pay's reference exists only in the
initialize answer, and is durable only once `submit` returns. Notch Pay
finds a payment by its own reference alone: `GET /payments?search=` matched
neither our reference nor the description in the sandbox, and list items
carry no `trxref`
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md#the-crash-window-measured-search-does-not-close-it)).
So a crash between call 1 and `submit` returning leaves a payment vpay
cannot read. Before call 2, the payer was never prompted and the payment
expires. After it, the payer may pay, and only Notch Pay's dashboard shows
it. The retry's `409` answers `Malformed`, which leaves the charge's fate
unknown for an operator (the maintainer's decision, 2026-10-21).

| The read answered                           | `ChargeStatus`                                                                                                                                                        |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pending`, `processing`, `incomplete`       | `Pending`. `incomplete` (paid less than asked) is never a success                                                                                                     |
| `complete`, `refunded`, `partialy-refunded` | `Succeeded { provider_txn_id: transaction.id, or transaction.reference }`, **only if** `amount`, `amounts.total` and currency are the charge's; otherwise `Malformed` |
| `canceled`                                  | `Failed { payer_declined }`, the status word as `raw`                                                                                                                 |
| `failed`, `rejected`                        | `Failed { provider_error }`, the status word as `raw`                                                                                                                 |
| `expired`, `abandoned`                      | `Failed { payer_timeout }`                                                                                                                                            |
| any other status, or none                   | `Malformed`, naming it                                                                                                                                                |
| `404` "Payment not found"                   | `NotFound`: never on its own a failure ([crash-safety.md](crash-safety.md))                                                                                           |
| any other `404`                             | `Config`: a wrong base path or an unknown Sync account                                                                                                                |
| `401`, `403`                                | `Rejected { provider_account_blocked }`                                                                                                                               |

**The read names no failure reason.** On 2026-10-21 the sandbox's outcome
numbers read `complete` (MTN and Orange success), `failed` (other failure),
`expired` (timeout) and `canceled` (canceled by user), with no reason field.
`canceled` therefore became `payer_declined`, as RFC-0012 § 4 said it would
on that evidence. `PRODUCED_FAILURE_CODES` is `payer_timeout`,
`payer_declined`, `provider_error`, `provider_account_blocked`.
`insufficient_funds` is not produced.

**The "insufficient funds" number read `complete`, for less than was
asked:** `amount` 107, `amounts.total` 105. On every other answer `total`
equalled `amount`. The adapter compared `amount` only, so once the
addressing is fixed that read would have been `Succeeded`. A success now
needs `amounts.total` to be the charge's too, and a missing `amounts` is
`Malformed`. A short `total` has its own error: money moved, not this
charge's amount, so the charge needs an operator. The question is with
Notch Pay in writing. **Step 5 confirmed it through the adapter on
2026-10-21:** `+237670000001` (230 XAF) read `complete` with a short
`amounts.total` and was refused as `Malformed` on each read after the first,
never `Succeeded`. After 15 minutes the charge was `pending` and the intent
`processing`. That run did not record the `amounts.total` value; step 1 had
105 of 107.

**A payment answer carries no `id`** in the sandbox, so a success records
`reference` (`trx.…`) as `provider_txn_id` when `id` is absent. Step 5 saw
exactly this on 2026-10-21: the success's `provider_txn_id` is the
`trx.test_…` that `provider_ref_extra.notchpay_reference` holds.

## Callbacks

`parse_callback` reads Notch Pay's event `{ id, type, created_at, data }` and
returns `data.trxref` as the charge reference, or `data.reference` when
`trxref` is not one of our UUIDs, with an **empty**
`ref_extra`. It refuses, as `Malformed`, any `type` but the five payment
events (`payment.created`, `.complete`, `.failed`, `.canceled`, `.expired`),
a `data` naming no vpay UUID in either field, and `{}`. `trxref` comes first
because the sandbox's payment answers carried ours there and Notch Pay's
`trx.…` in `reference`; no webhook was received to confirm the event's
`data` has the same shape. The event's own status is
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

RFC-0012 § 10 is the full list, twenty-one since 2026-10-21. The sandbox
answered several on that date
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md));
each is marked below. The ones that decide whether the adapter is right:

1. **(W)** Can the read require the private key, and can the public key's
   payment operations be disabled? A yes retires § "Production risks" items
   1–4 and the boot warning.
2. **(S)** Does the payment path take our reference, Notch Pay's `trx.…`, or
   both? **Contradicted 2026-10-21: `trx.…` only.** Fixed the same day, and
   the fix met Notch Pay through the adapter in step 5 on 2026-10-21: every
   charge held `ref_extra.notchpay_reference` (`trx.test_…`) after submit, and
   every read by it was answered. _(This said "the fix has not met Notch Pay
   (step 5)" until then.)_
3. **(S)** Does `cm.mobile` resolve the network from the number? **Answered
   2026-10-21: yes**, for MTN and Orange outcome numbers.
4. **(S)** Is a repeated reference answered `409`? **Answered 2026-10-21:
   yes**, "Reference already existing".
5. **(S)** What does the read say for each sandbox outcome number (`…000001`
   insufficient funds, `…000002` other failure, `…000003` timeout, `…000004`
   canceled)? Any reason field? **Answered 2026-10-21:** `complete` for
   less than asked, `failed`, `expired`, `canceled`; no reason field. Through
   the adapter (step 5, same day): `…000002` was `failed` and became
   `provider_error`, and `…000001` was refused as `Malformed` on every read
   after the first. `…000003` and `…000004` were not run through vpay.
6. **(W, S)** Is `POST /refunds` real, and how is it made safe to replay?
7. **(S)** Is the webhook signature over the raw body? **Not measured:** no
   public URL to receive one.
8. **(W)** What prefix does a live key carry?
9. **(W)** Does an `incomplete` payment expire as a `pending` one does?
10. **(S)** Are answers under `transaction`, as the specification says, or
    under `data`, as some API reference pages show? **Answered 2026-10-21:
    `transaction`.**
11. **(W)** Can a live payment read `complete` having collected less than
    `amount`, and is `amounts.total` the collected amount? (§ 10 item 18)
12. **(S)** Does `GET /payments?search=` find a payment by our reference?
    It decides whether the addressing fix is crash-safe. (§ 10 item 19)
    **Answered 2026-10-21: no**, so the fix carries a residual crash
    window.

Also open, and not in the adapter's hands: whether Notch Pay accepts a number
without the `+`. The adapter sends the `+` form the documentation shows.

## Status

**Built 2026-10-20. Its calls were first made through the adapter on
2026-10-21, against Notch Pay's real sandbox (RFC-0012 step 5): one
payment succeeded, one failed, and one that read `complete` for less than its
amount was refused and stayed `pending`**
([verification](../status/verification/2026-10-21-notchpay-step5.md)). Step 1
the same day, through the probe, had contradicted the adapter's addressing
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md)).
The addressing was fixed
([verification](../status/verification/2026-10-21-notchpay-trx-reference.md)),
and the fix met Notch Pay in step 5 and worked. Production has never been
called.

_(This paragraph read "Its calls have never been made through the adapter",
and ended "that fix has not met Notch Pay", until 2026-10-21, when step 5
ran.)_

- **Built and tested in process:** the adapter (`vpay-adapter-notchpay`, its
  unit tests in `src/tests.rs` and `src/mapping.rs`); the boot rules
  (`vpay-config`, the `notchpay` tests in `config.rs`); the port's
  `accepted_exposure` and its forward through `Measured`; the boot warning
  (`vpay-api`'s `boot.rs`); the client's per-request deadline.
- **Proven against WireMock:** the shared conformance suite's
  `RailUnderTest::NotchPay` row, against
  `backends/tests/conformance/wiremock/notchpay/mappings/`. Since 2026-10-21
  the stub answers in the shape Notch Pay's sandbox did: paths by
  `trx.test_…`, our reference as `trxref`, no `id`, an `amounts.total`. It
  proves the adapter against that stub, and nothing about Notch Pay.
- **Step 5 done 2026-10-21**, on the maintainer's MacBook (not a cloud
  session, and no `--ca-bundle`), from `master` at `865420b`, which holds
  the `trx.…` addressing fix: `vpay-server`, `vpay-worker` and Postgres 16 on
  the `notchpay-sandbox` profile, `livemode=false`, test keys, on the
  maintainer's approval in that session, which named the numbers, the amounts
  and every call. Three payments on `cm.mobile`, XAF
  ([verification](../status/verification/2026-10-21-notchpay-step5.md)):
  - `+237670000000`, 210 XAF: charge and intent `succeeded` on the worker's
    second read, about 15 s after create. `provider_txn_id` is
    `trx.test_…`, Notch Pay's `reference`, because the answer carries no
    `id`. Event `payment_intent.succeeded`; one balanced ledger transaction,
    debit `payer_clearing` and credit `merchant_payable`, 210 XAF.
  - `+237670000002`, 220 XAF: charge `failed` (`failure_raw` `failed`),
    `failure_code` `provider_error`, intent `requires_payment_method`, event
    `payment_intent.payment_failed`, on the worker's second read.
  - `+237670000001`, 230 XAF ("insufficient funds"): read `complete` with
    `amounts.total` short of the charge's amount. Its first read (about
    0.5 s after submit) answered `pending`; each of the next eleven was
    refused as `Malformed`, with the worker's WARN each time. At 15 minutes the charge was still `pending` and the
    intent `processing`, with no event and no ledger entry. By the code it
    reaches `unresolved` only at the 24 h horizon (`escalate_to_unresolved`);
    that was not observed.
  - Every charge held `provider_ref_extra.notchpay_reference`
    (`trx.test_…`) after submit, and every read by it was answered. vpay sent
    3 submits and 16 reads. The ADR-0031 `WARN` was logged at boot by the
    server and the worker. No key appeared in either log.
- **Two observations from step 5, neither an adapter defect, and no change
  made:** the `provider_requests` rows for the 11 refused reads record "no
  answer" (`status_code` and `responded_at` empty) although Notch Pay
  answered each one, because the worker records every `ProviderError`
  that way; and the adapter's "needs an operator" message wakes nobody for
  24 h, because the worker retries a `Malformed` on the ladder with a `WARN`
  until the horizon (read from the code, not observed past 15 minutes). Both
  belong to the worker and the database, and the second is a design question
  for the maintainer.
- **Step 1 done 2026-10-21** from a cloud session, with the probe's
  cloud-only `--ca-bundle`, on the maintainer's approval in that session:
  items 3, 4, 5 and 15 answered; 2, 13 and 16 contradicted; 7 and 11 not
  measured. Fixed on that evidence: the `amounts.total` check, `canceled` as
  `payer_declined`, `reference` as `provider_txn_id`, and then item 2 (the
  adapter addresses process and read by Notch Pay's `trx.…`). Three webhook
  endpoints the probe created could not be deleted (`DELETE` answered
  `404`); the maintainer deleted them by hand.
- **Before step 5 ran (2026-10-20, kept as it was written).** RFC-0012
  delivery step 5 (the first test-mode call through the adapter) was the
  first time the item 2 fix would meet Notch Pay. No test-mode key was
  available when the adapter landed. A session later on 2026-10-20 had the
  keys and **still made no request**: its permission policy refused the
  calls, and its egress re-terminates TLS, which vpay's vendored-roots client
  rightly rejects. It did measure one thing: the real test keys carry `test.`,
  not `test_`, and the boot rule is corrected for it
  ([status/verification/2026-10-20-notchpay-sandbox.md](../status/verification/2026-10-20-notchpay-sandbox.md)).
  On the maintainer's decision the same day, both steps run on a real
  machine from [runbooks/notchpay-sandbox-test.md](../runbooks/notchpay-sandbox-test.md)
  (a probe, `vpay-notchpay --example sandbox_probe`, and the
  `notchpay-sandbox` profile). Step 1 later ran from a cloud session (above),
  and step 5 on the maintainer's machine (above). Step 1 is evidence about
  Notch Pay, not about the adapter; step 5's dated page is what moved the
  Collect and Status read cells of
  [status/providers.md](../status/providers.md) from "stub-only" to "proven;
  sandbox 2026-10-21". _(This bullet was headed "Not done" and ended "until a
  dated verification page records step 5, the Collect and Status read cells
  of status/providers.md say "stub-only", never "sandbox" or "proven"" until
  2026-10-21.)_
- **Not done**, after step 5: no webhook was received (runbook A4), so
  RFC-0012 § 10 items 7 and 17 stay open, and webhook signature verification
  ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md)) is not built; Notch
  Pay's production was never called and no live key was used, so item 8 (a
  live key's prefix) stays a written question; no refund
  (`notchpay::refund` is still a token); Orange (`+23769…`) was not run
  through vpay, and step 1's probe is its only run; the 24 h escalation of the
  short collection to `unresolved` was not observed, because the stack was
  torn down at 15 minutes; and the `amounts.total` value Notch Pay returned
  for `…000001` was not captured (vpay logs no response body, and no extra
  read was made).
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
