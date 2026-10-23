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
> On 2026-10-22 runbook B3b took the other three outcome numbers through it:
> the MTN timeout number failed `payer_timeout`, the MTN canceled number
> `payer_declined`, and the Orange success number succeeded
> ([verification](../status/verification/2026-10-22-notchpay-b3b.md)).
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
  deployment serves one merchant. _(2026-10-22: one legal entity, not one
  `merchant_id`. Several projects of one company may share the company's
  account, as RFC-0009's revision (phase A, unbuilt) describes. Whether a
  company may hold a Notch Pay account at all is RFC-0012 § 10 item 10.)_
- Both of the account's keys: the **public** key and the **private** key.
  Boot refuses a `notchpay` block without either.
- The webhook endpoint `{public_base_url}/provider/notchpay/callback`,
  registered with the account for the five payment events. **vpay registers
  it** ([RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md), built
  2026-10-22): the worker's `ensure_callbacks` job, at every worker start and
  hourly, or `vpay-server callbacks ensure` by hand. See § Callbacks. _(This
  said "registered by hand in Notch Pay's dashboard (Settings › Webhooks) …
  until RFC-0013 registers it" until 2026-10-22.)_ Optional for
  correctness: the poll ladder reads every
  charge without callbacks. _(This said "settles every charge" until
  2026-10-22. A charge whose reads the adapter refuses is never settled by
  them: step 5's short collection was still `pending` after fifteen minutes
  ([verification](../status/verification/2026-10-21-notchpay-step5.md)), and by
  the code, not observed, it became `unresolved` at the 24-hour horizon.
  Since 2026-10-23 it becomes `unresolved` on its first such read, which is
  proven against WireMock only.)_

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
| 5. Integrity is checked on the answer              | a success is `Succeeded` only for the charge's own amount and currency; otherwise `NeedsReconciliation` (`Malformed` until 2026-10-23)                                                                                    |
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
    callbacks: # optional; absent means `register: ensure`
      register: ensure # or `check` (list, never create) or `off` (never ask)
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

**A charge is two port calls, with a commit between them** (RFC-0012 § 3,
[ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)).
The adapter declares `SubmitSteps::PrepareThenSubmit`:

1. **`prepare`** is `POST /payments` with `amount` (XAF has no minor unit),
   `currency: XAF`, `phone` (the payer's number, `+`-prefixed E.164),
   `reference` (the charge's own UUID) and the optional `description`.
   Nothing else of the payer's is sent: no email, no name, no callback, no
   return URL. It creates the payment and **prompts nobody**, and returns
   the `trx.…` from `transaction.reference` as `notchpay_reference`. The core
   commits it on the charge, then writes the `submit` row.
2. **`submit`** is `POST /payments/{notchpay_reference}` with `{ "channel":
"cm.mobile", "data": { "phone": … } }`, by the reference the charge now
   holds. This is the call that prompts the payer.

| `prepare` answered                                                | Then                                                                                                                                          |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `201`, a documented status                                        | `notchpay_reference`, for the core to commit                                                                                                  |
| `201` with no Notch Pay reference, or a `trxref` that is not ours | `Malformed`; nothing exists that vpay could address, and nobody was prompted                                                                  |
| `409`, and no `submit` row exists for the charge                  | `Rejected { provider_unavailable }`: an earlier initialize lost its answer and nobody was prompted, so the charge fails and the payer retries |
| `409`, and a `submit` row exists                                  | `NeedsReconciliation`: a one-call build's process may have prompted, and the payment cannot be found (below)                                  |
| `400`, `422`                                                      | `Rejected { provider_error }` with Notch Pay's message and the names of the fields it refused                                                 |
| `401`, `403`                                                      | `Rejected { provider_account_blocked }`, which pages                                                                                          |
| `404`                                                             | `Config`                                                                                                                                      |
| `408`, `429`, `5xx`, no answer                                    | `Transport`: the fate is unknown, never a decline                                                                                             |

`prepare` on a charge that already holds `notchpay_reference` answers it
back with no request; the core never asks. `submit` on a charge that holds
none is `NeedsReconciliation`, before any request: the core never sends one,
and one that arrives has lost the only address Notch Pay answers for.
`submit`'s `202` is `Submitted`, with no redirect and `notchpay_reference` as
its key material. Its `422` "Payment has already been processed" is not a
decline: the adapter reads the payment and answers `Submitted`, unless the
read still says `pending`, which is `Malformed`.

_(Until 2026-10-23 `submit` was both calls, and a `201` with a status other
than `pending` was `Submitted` without call 2. A `409` read the payment when
the charge held `notchpay_reference` and was `NeedsReconciliation` when it
did not, whatever had been sent.)_

**The read** is `GET /payments/{notchpay_reference}`, and the answer must
name the charge's own reference as its `trxref`, or it is `Malformed`. A
charge with no `notchpay_reference` is `NeedsReconciliation`, before any
request: that is key material vpay lost, not a payment Notch Pay never heard
of, so it is never `NotFound` (the same rule as Orange's missing
`pay_token`). It was `Config` until 2026-10-23, which dead-lettered the poll
job.

Until 2026-10-21 every call addressed the payment by **our** reference, as
the specification's example does. Notch Pay's sandbox answered `404`
"Payment Not Found" to every process and read by it, and accepted only its
own `trx.…` (§ 10 item 2, contradicted). RFC-0012 § 3 had designed "prefer
Notch Pay's reference when held"; the build dropped it on 2026-10-20 for a
rule that needed no `ref_extra`, and the sandbox overturned the build.

**The crash window, and how ADR-0032 closed it.** Notch Pay's reference
exists only in the initialize answer, and Notch Pay finds a payment by its
own reference alone: `GET /payments?search=` matched neither our reference
nor the description in the sandbox, and list items carry no `trxref`
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md#the-crash-window-measured-search-does-not-close-it)).
Under the one-call `submit`, a crash after process and before `submit`
returned left a prompted payment vpay could not read, and the retry's `409`
could only be `NeedsReconciliation` for an operator (the maintainer's
decision, 2026-10-21; escalated to `unresolved` at once since
vaam-apps/vpay#313).

Since 2026-10-23 the `trx.…` is committed before anything prompts, and the
`submit` row is written only after it, so a crash leaves one of three
states ([crash-safety.md](crash-safety.md) § "Two-step push rails"):

| After the crash                              | What recovery does                                                                                    |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| a `prepare` row, no `trx.…`, no `submit` row | the resubmit's `prepare` meets `409`; nobody was prompted, so the charge fails `provider_unavailable` |
| the `trx.…` held, no `submit` row            | the resubmit skips `prepare` and prompts by the held reference: one initialize, one process           |
| the `trx.…` held, an unanswered `submit` row | the poll reads by the held reference, as on every rail; nothing is sent again                         |

**Charges in flight across that deploy** — an unanswered `submit` row and no
`trx.…`, left by the one-call build — may have been prompted, and stay
`NeedsReconciliation` whichever job reaches them first: the poll cannot read
them, and a queued resubmit's `prepare` is told a submit was recorded. A
one-call charge with no attempt row at all was never sent and is prepared
like any other.

**The orphan.** A lost initialize leaves a payment created on Notch Pay and
never processed. vpay fails its charge and never touches it again, and
Notch Pay expires it. Until then, anyone holding the merchant's public key
could process it to a number of their choosing — the exposure
[ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md)
accepted for every payment on the account, and nothing this step adds.

**Not run against Notch Pay.** Every case above runs against the WireMock
stub (`backends/tests/integration/tests/worker_recovery.rs`,
`confirm_rails.rs`). No sandbox run has exercised the split.

| The read answered                           | `ChargeStatus`                                                                                                                                                                  |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pending`, `processing`, `incomplete`       | `Pending`. `incomplete` (paid less than asked) is never a success                                                                                                               |
| `complete`, `refunded`, `partialy-refunded` | `Succeeded { provider_txn_id: transaction.id, or transaction.reference }`, **only if** `amount`, `amounts.total` and currency are the charge's; otherwise `NeedsReconciliation` |
| `canceled`                                  | `Failed { payer_declined }`, the status word as `raw`                                                                                                                           |
| `failed`, `rejected`                        | `Failed { provider_error }`, the status word as `raw`                                                                                                                           |
| `expired`, `abandoned`                      | `Failed { payer_timeout }`                                                                                                                                                      |
| any other status, or none                   | `Malformed`, naming it                                                                                                                                                          |
| `404` "Payment not found"                   | `NotFound`: never on its own a failure ([crash-safety.md](crash-safety.md))                                                                                                     |
| any other `404`                             | `Config`: a wrong base path or an unknown Sync account                                                                                                                          |
| `401`, `403`                                | `Rejected { provider_account_blocked }`                                                                                                                                         |

**The read names no failure reason.** On 2026-10-21 the sandbox's outcome
numbers read `complete` (MTN and Orange success), `failed` (other failure),
`expired` (timeout) and `canceled` (canceled by user), with no reason field.
`canceled` therefore became `payer_declined`, as RFC-0012 § 4 said it would
on that evidence. `PRODUCED_FAILURE_CODES` is `payer_timeout`,
`payer_declined`, `provider_error`, `provider_account_blocked` and, since
ADR-0032, `provider_unavailable`: `prepare`'s `409` with no `submit` row.
`insufficient_funds` is not produced. Through vpay on 2026-10-22 (runbook
B3b), `expired` and `canceled` became `payer_timeout` and `payer_declined`,
each on the worker's second read, about 12 s after create
([verification](../status/verification/2026-10-22-notchpay-b3b.md)).

**The "insufficient funds" number read `complete`, for less than was
asked:** `amount` 107, `amounts.total` 105. On every other answer `total`
equalled `amount`. The adapter compared `amount` only, so once the
addressing is fixed that read would have been `Succeeded`. A success now
needs `amounts.total` to be the charge's too, and a missing `amounts` is
refused as well. A short `total` has its own message: money moved, not this
charge's amount, so the charge needs an operator. The question is with
Notch Pay in writing. **All three refusals are `NeedsReconciliation` since
2026-10-23** (vaam-apps/vpay#313; `Malformed` until then): the worker
escalates the charge to `unresolved`, hourly and alerting, on the first read
that says so, instead of riding the ladder at `WARN` until the 24-hour
horizon. Including the wrong `amount` or currency and the missing total, not
only the short total, was the implementer's call, recorded in the pull
request for the maintainer to reverse. **Step 5 confirmed it through the adapter on
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

### Registering the endpoint (RFC-0013, built 2026-10-22)

`notchpay` declares `CallbackUrlMode::Registered`: Notch Pay is not told the
callback URL on a payment, so the account must hold a webhook endpoint at it.
The worker's `ensure_callbacks` job keeps one there, under the registration
lock, at every worker start and hourly; `vpay-server callbacks ensure`
runs the same pass by hand and prints one line per rail.

One pass lists **every page** of `GET /webhooks`, then:

- **Recognises vpay's endpoint by its description**, never by its URL. Notch
  Pay answered `"url": null` for every endpoint in the sandbox, in the create
  answers and the list (2026-10-21). The description is
  `vpay:{deployment.name}:notchpay:default url:{16 hex digits of SHA-256 of
the callback URL}`: who created it, and for which URL. An endpoint with
  this marker and this tag is `present`; one with this marker and another
  tag (a `public_base_url` that changed) is superseded; a second current one
  is a duplicate.
- **Creates at most once**, with `url`, that description and the five
  payment events, only when no current one exists and the setting is
  `ensure`. A create whose answer is lost is not repeated in that pass; the
  next pass lists first.
- **Never modifies or deletes anything.** The client has no `PUT`, and
  `DELETE /webhooks/{id}` answered `404` in the sandbox. A current endpoint
  missing an event is reported `drifted`; superseded and duplicate ones are
  logged at `warn`, by id, for an operator to delete in the dashboard. That
  is the design and not a gap: the maintainer decided on 2026-10-23 that
  retiring an endpoint is "an operator's task only, not code".
- **Cannot see a hand-registered endpoint at vpay's URL**, because the URL is
  not echoed. A deployment that registered one by hand before 2026-10-22
  gets a second, vpay's own: each callback arrives twice, which costs a
  second pull-forward and nothing else (RFC-0013 § 3). Delete the hand-made
  one in the dashboard, or set `register: off`.

A refused key (`401`/`403`) is logged at `error` with `alert = true`; the job
reschedules either way and never dead-letters.

## Refunds

`supports_refunds` and `supports_partial_refunds` are `true` and
`refund_destination` is `Origin`, because Notch Pay's refunds guide documents
both. `refund` answers the declared token `NotImplemented("notchpay::refund")`
([status.md](../status.md)). The guide's `POST /refunds` is absent from the
specification and carries no reference that would make a replay safe, so a
lost answer re-sent would pay twice. It becomes a call when Notch Pay
confirms a replay-safe refund in writing (§ 10 item 6).

On 2026-10-22 the probe asked the sandbox. `GET /refunds` answered `200` with
an empty list. `POST /refunds` answered `403` "Forbidden", with no message,
to all four requests against a `complete` payment, and refunded nothing
([verification](../status/verification/2026-10-22-notchpay-refunds.md)).
Whether that is an IP allowlist (§ 10 item 12) or refunds not being enabled
for the account is not known, and replay safety is still unmeasured.

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
   after the first. Through vpay on 2026-10-22 (runbook B3b): `…000003` was
   `expired` and became `payer_timeout`, and `…000004` was `canceled` and
   became `payer_declined`, each on the worker's second read
   ([verification](../status/verification/2026-10-22-notchpay-b3b.md)).
   _(This said "`…000003` and `…000004` were not run through vpay" until
   2026-10-22.)_
6. **(W, S)** Is `POST /refunds` real, and how is it made safe to replay?
   **Not answered 2026-10-22:** the sandbox answered `403` to every
   `POST /refunds`, so nothing was refunded and no replay was measured.
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
    window. **Closed 2026-10-23** by
    [ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md),
    without needing a search: the `trx.…` is committed before the prompt.

Also open, and not in the adapter's hands: whether Notch Pay accepts a number
without the `+`. The adapter sends the `+` form the documentation shows.

## Status

**Changed 2026-10-23: the `trx.…` is committed before the prompt**
([ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)).
Initialize is the port's `prepare` and process is `submit`, with a commit
and the `submit` row between them, on the confirm path and on the
resubmit. A crash before the prompt fails the charge cleanly, and the
`409` stays `NeedsReconciliation` only for a charge a one-call build may
have prompted. Proven against WireMock and a real Postgres in
`worker_recovery.rs` (six cases) and `confirm_rails.rs` (two), each guard
mutation-checked; not run against Notch Pay
([verification](../status/verification/2026-10-23-two-step-submit.md)).

**Changed 2026-10-23: an answer only a person can act on escalates at once**
([#313](https://github.com/vaam-apps/vpay/issues/313)). Three answers are
`ProviderError::NeedsReconciliation` (they were `Malformed` or `Config`):

- the short collection, and any `complete` for a sum that is not the
  charge's;
- the `409` for a charge that lost its `trx.…` (since ADR-0032, the same
  day, only when a `submit` row exists: with none, the `409` fails the
  charge as never prompted);
- a charge with no `trx.…` to read by.

The worker makes the charge `unresolved` on the first one, polled hourly and
alerting. Proven against WireMock and a real Postgres in `worker_recovery.rs`;
not run against Notch Pay
([verification](../status/verification/2026-10-23-needs-reconciliation.md)).

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

- **Callback registration, built 2026-10-22 (RFC-0013):**
  `src/callbacks.rs`, with its unit tests; WireMock mappings
  `mappings/webhooks.json`, transcribed from the 2026-10-21 sandbox answers;
  and `backends/tests/integration/tests/callback_registration.rs` against a
  real Postgres and that stub (one create, then found; two racing passes, one
  create; `check` never writes). **It has never registered an endpoint with
  a real Notch Pay account** (RFC-0013 delivery step 5).
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
    intent `processing`, with no event and no ledger entry. By the code at
    the time it reached `unresolved` only at the 24 h horizon
    (`escalate_to_unresolved`); that was not observed. Since 2026-10-23 the
    same read escalates it on its first occurrence (#313).
  - Every charge held `provider_ref_extra.notchpay_reference`
    (`trx.test_…`) after submit, and every read by it was answered. vpay sent
    3 submits and 16 reads. The ADR-0031 `WARN` was logged at boot by the
    server and the worker. No key appeared in either log.
- **Runbook B3b done 2026-10-22**, on the maintainer's MacBook, the same
  way as step 5 and on its approval rule. The binary was built from `master`
  at `e7406dd`, whose code is `master` at `760fbb9`'s but for the version
  string. It ran the three outcome numbers step 5 had not
  ([verification](../status/verification/2026-10-22-notchpay-b3b.md)):
  - `+237670000003` (MTN timeout), 240 XAF: read `expired`, charge `failed`,
    `failure_code` `payer_timeout`, intent `requires_payment_method`, event
    `payment_intent.payment_failed`.
  - `+237670000004` (MTN canceled by user), 250 XAF: read `canceled`, charge
    `failed`, `failure_code` `payer_declined`, intent
    `requires_payment_method`, event `payment_intent.payment_failed`.
  - `+237690000000` (Orange success), 260 XAF: charge and intent
    `succeeded`, `provider_txn_id` the `trx.test_…` reference, event
    `payment_intent.succeeded`, one balanced 260 XAF ledger transaction.
  - Each answered `pending` to the read right after submit and its terminal
    status to the next, ten seconds later: 11 to 12 s from create. vpay sent
    3 submits and 6 reads, and no read after a charge was terminal. Every
    charge held `notchpay_reference` `trx.test_…` after submit. No key
    appeared in either log. Nothing differed from the mapping, and no code
    changed.
- **Two observations from step 5, neither an adapter defect.** The first is
  fixed. In step 5 the `provider_requests` rows for the 11 refused reads
  recorded "no answer" (`status_code` and `responded_at` empty) although
  Notch Pay answered each one, because the worker recorded every
  `ProviderError` that way. Since
  [#314](https://github.com/vaam-apps/vpay/pull/314) (`6d9dfbf`, 2026-10-21) a
  refused status read is recorded as answered, `status_code` 0 with its
  `error_kind`; a refused submit still stays unresolved
  ([verification](../status/verification/2026-10-21-provider-requests-refused-answers.md)).
  The second is fixed too, since 2026-10-23
  ([#313](https://github.com/vaam-apps/vpay/issues/313), the maintainer's
  "Go with option 1 on #313, build it"): the short collection is
  `NeedsReconciliation`, and the worker escalates the charge to `unresolved`,
  hourly and alerting, on the first read that says so, against a WireMock
  Notch Pay and a real Postgres
  ([verification](../status/verification/2026-10-23-needs-reconciliation.md)).
  Before it, the adapter's "needs an operator" message woke nobody for 24 h,
  because the worker retried a `Malformed` on the ladder with a `WARN` until
  the horizon. _(This bullet was headed "neither an adapter defect, and no
  change made", and described the first observation in the present tense,
  until 2026-10-22; #314 had already changed it. It called the second
  "unchanged … a design question for the maintainer" until 2026-10-23.)_
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
- **Not done**, after step 5 and B3b: no webhook was received (runbook A4), so
  RFC-0012 § 10 items 7 and 17 stay open, and webhook signature verification
  ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md)) is not built; Notch
  Pay's production was never called and no live key was used, so item 8 (a
  live key's prefix) stays a written question; no refund
  (`notchpay::refund` is still a token; the sandbox answered `403` to the
  probe's `POST /refunds` on 2026-10-22); Orange's failure path, for which
  Notch Pay documents no test number; how a real unanswered prompt ends, and
  how long it takes, which a test number cannot show; a real short collection
  reaching `unresolved`, which since 2026-10-23 happens on its first read and
  has been proven against WireMock only (the 24 h escalation step 5 would have
  waited for was never observed either: the stack was torn down at 15
  minutes); and the `amounts.total` value Notch Pay returned
  for `…000001` was not captured (vpay logs no response body, and no extra
  read was made). _(This bullet listed "Orange (`+23769…`), timeout
  (`…000003`) and canceled (`…000004`) were not run through vpay, and step
  1's probe is their only run (runbook B3b is written for them)" until
  2026-10-22, when B3b ran them.)_
- **Declared gaps:** `notchpay::refund` (token); model B, Sync, `X-Sync` (not
  offered); `cm.mtn` / `cm.orange` (step 8); webhook signature verification
  ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md), a draft).
  _(This list ended "retiring a superseded or duplicate webhook endpoint (no
  working `DELETE`)" until 2026-10-23, when the maintainer decided it is "an
  operator's task only, not code": vpay logs each such endpoint by id at
  `warn`, and an operator deletes it in Notch Pay's dashboard (§ "Registering
  the endpoint"). Before that the last item was "vpay registering its own
  webhook (RFC-0013)", until 2026-10-22.)_
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
