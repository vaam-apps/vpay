# Fapshi against ADR-0029 D1

- **Date:** 2026-10-23
- **Request, verbatim:** "Also, study [fapshi](https://docs.fapshi.com/en/api-reference/getting-started) as another payment provider too"
- **Status:** A desk evaluation from **public documentation only**: Fapshi's
  API reference and its `openapi.json` (3.1.0, `info.version` 1.0.0, 8 paths
  and 1 webhook, fetched 2026-10-23), its terms and conditions, and two of its
  help articles. No account was opened, no key obtained, no API called, and
  Fapshi was not contacted. A "yes" below means "Fapshi's documentation says
  so", never "seen working".

## The verdict

**Fapshi passes all four hard rules on its documentation, including rule 3
as ADR-0029 wrote it, without [ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md).**
Every call, the status read included, carries `apiuser` and `apikey`, and
Fapshi calls the key "a secret key that acts like a password" and forbids it
in client-side code. That makes it the cleaner of the two mobile-money
aggregators: Notch Pay is admitted only under ADR-0031 and logs a production
risk at every boot.

It is a **push rail** like `notchpay` (Direct Pay sends the USSD prompt), with
an optional **redirect** form (Initiate Pay, a Fapshi-hosted page). It serves
MTN MoMo and Orange Money in Cameroon, in XAF, from 100 XAF, and it takes the
network as an explicit `medium` parameter. That means the payer's own choice
of network, the way the maintainer wants the checkout to work, goes straight
into the request.

Four things stand between the documentation and an adapter:

1. **A resent create may prompt the payer twice.** Fapshi documents no
   idempotency key and no uniqueness for `externalId`, so a Direct Pay whose
   answer was lost cannot simply be resent. Fapshi does offer what Notch Pay
   lacks: `GET /transaction/{userId}` finds transactions by an identifier of
   **ours**. Setting `userId` to the charge's reference lets vpay look before
   it resends. Whether that lookup sees a transaction the moment its create
   answers is the first thing to measure.
2. **Direct Pay is off in live mode** and approved only "for absolutely
   necessary use cases (e.g. Mobile app)". A gateway may be granted only
   Initiate Pay, the hosted page. On that page the payer picks the network
   on Fapshi's side.
3. **No refund API at all.** The terms make returns "solely" the merchant's
   responsibility. A refund could only be a payout from a _separate_ payout
   service, which is also off in live mode. `supports_refunds` would be
   `false`, an honest fact about Fapshi, unlike Notch Pay's documented but
   unproven refunds.
4. **The business is unverified.** The terms name "Fapshi Inc" and no
   registration number, licence, regulator or partner bank. Fapshi did not
   appear in a search of the Cameroonian list of approved payment
   institutions. That is a search result, not proof of anything, and it
   remains the operator's due diligence
   ([ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md) D1).

## The four hard rules

| Rule (ADR-0029 D1)                      | Fapshi, on its documentation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1. vpay never holds the money           | **Passes in shape.** "Settlement funds will be held in a deposit account at Fapshi pending Payouts to you", sent "to your designated mobile money or bank account" on a payout schedule set in the dashboard. The balance is the merchant's at Fapshi, as with Notch Pay. Fapshi may "place a Reserve on a portion of your Payouts" and suspend an account "and/or … any funds".                                                                                                                                                                                                                                                       |
| 2. No card number reaches `vpay-server` | **Passes.** The API carries no card at all: `medium` is `mobile money`, `orange money` or, on payouts only, `fapshi`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| 3. A secret-authenticated status read   | **Passes as written.** `GET /payment-status/{transId}` declares `apiuser` and `apikey` headers (`security: apiAuth + apiKey`). The key "acts like a password … Never share them or expose them in client-side code". No ADR-0031 exception and no boot warning.                                                                                                                                                                                                                                                                                                                                                                        |
| 4. A stub-able contract                 | **Passes in shape.** An OpenAPI 3.1 document with 8 paths, and a sandbox (`https://sandbox.fapshi.com`) with documented outcome numbers: MTN success 670000000, 670000002, 650000000 and failure 670000001, 670000003, 650000001; Orange success 690000000, 690000002, 656000000 and failure 690000001, 690000003, 656000001. "Any number not listed above" gets a random outcome. The error contract is thin: every refusal is "4XX" with a `message`. **The sandbox needs an activated account**: "You can no longer test the Fapshi API in test mode without activating your account", and activation is KYC with a Cameroonian ID. |

## How it would sit behind the port

**Flow.** `ProviderFlow::Push` on Direct Pay, one call:
`POST /direct-pay { amount, phone, medium, externalId, userId, message }`
answers `200 { transId, dateInitiated }`. That is one call where Notch Pay
needs two (initialize, then process). Initiate Pay would be a separate
`Redirect` shape: `POST /initiate-pay` answers a `link`, which expires after
24 hours, and `POST /expire-pay` cancels it.

**Addressing.** Fapshi's `transId` is the only key for the status read, so it
goes in `ref_extra`, as Notch Pay's `trx.…` does. The charge's reference goes
in `externalId` and `userId`. Both accept `^[a-zA-Z0-9\-_]{1,100}$`, and a
UUID fits.

**Crash safety.** Using `userId` for the charge reference repurposes a field
Fapshi means for the payer's id in the merchant's system. That costs Fapshi's
dashboard its per-customer grouping and nothing in vpay. In exchange it is
the lookup that makes a lost create recoverable:

| Submit answered                                            | Then                                                                                                                                                                      |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `200` with a `transId`                                     | `Submitted` carrying it                                                                                                                                                   |
| lost (timeout, `5xx`, crash before `ref_extra` is durable) | before any resend, `GET /transaction/{charge reference}`: one transaction → adopt its `transId`; none → resend; more than one → needs reconciliation (vaam-apps/vpay#313) |
| `403`                                                      | credentials or the IP allowlist: our account, `provider_account_blocked`                                                                                                  |
| `400`                                                      | `Rejected` or `Config` by `message`; the catalogue is unpublished                                                                                                         |
| `429`                                                      | `Transport`, asked again later                                                                                                                                            |

The "none → resend" row is safe only if the lookup is consistent the moment
a create has answered. Until the sandbox shows that, it is the open risk of
this design. It is the same crash window that vaam-apps/vpay#313 and a two-step submit
address for Notch Pay, but here the provider gives us the tool.

**Statuses.** `CREATED` and `PENDING` map to pending; `SUCCESSFUL` to
succeeded, only when `amount` is the charge's (`revenue`, the amount net of
Fapshi's fees, is informational); `FAILED` to failed; `EXPIRED` (Initiate Pay
only) to failed with `payer_timeout`. "Direct payment transactions cannot and
do not expire", so their end is `SUCCESSFUL` or `FAILED`. **A `FAILED`
carries no reason** (`reason` "typically appears when a payout via fapshi
fails"), so a decline, insufficient funds and a timeout cannot be told apart
from the documentation.

**Reads are rate-limited:** "at most 6 requests per minute" per `transId`,
`429` beyond. The poll ladder's first rungs (10, 20, 30, 45 s) stay under it.
A burst of callbacks pulling the poll forward could exceed it, which is a
`Transport` and harmless.

**Phone.** The examples are national (`67XXXXXXX`). The adapter would send
the payer's MSISDN without `+237`, unless the sandbox accepts the
international form.

**Network.** `medium: "mobile money" | "orange money"`, or omitted to
auto-detect. That is exactly what the method-type checkout needs:
`mtn_wallet` → `mobile money`, `orange_wallet` → `orange money`.

**Amount limits.** A minimum of 100 XAF on every create. That is ADR-0029
D2's `amount_limits`, which has no consumer yet.

## Callbacks

- **The URL is set per service in Fapshi's dashboard, and no endpoint
  registers it.** vpay cannot keep it registered as it does Notch Pay's
  ([RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md)): this rail is `Registered`, and `ensure_callback_endpoint` stays
  `Unsupported`. But every status read returns the service's `webhook`, so
  vpay can **check** the configured URL on every read and log a mismatch.
  That check comes free, with no extra call.
- **"Fapshi sends only one webhook request per event, regardless of whether
  your server responds or not."** No retry. Callbacks are hints in vpay
  anyway, so a lost one costs a poll's latency.
- **Authentication is a static shared secret**, not a signature. When an
  operator sets a webhook secret in the dashboard, every webhook carries it
  in an `x-wh-secret` header. Checking it needs [RFC-0014](../rfc/0014-verifying-rail-callbacks.md)'s change: the port
  hands `parse_callback` no headers today. It would be a third secret per
  account, compared in constant time. Unlike Notch Pay's, the secret is known
  to the operator.
- **The payload is the status read's body**, so `externalId` names our charge
  and `transId` Fapshi's. Events are sent for `SUCCESSFUL`, `FAILED` and
  `EXPIRED`.

## Refunds and payouts

There is no refund endpoint. "You agree that you are solely responsible for
accepting and processing returns of your products and services." A refund
could only be `POST /payout` from a **payout service**:

- that is a second service, with its own credentials and its own balance;
- "after enabling payouts for a service, that service can no longer collect
  payments";
- payouts are off in live mode until support activates them;
- it carries the same missing idempotency as Direct Pay.

An adapter would declare `supports_refunds: false`. Refund-as-payout would be
a design of its own, beside [RFC-0001](../rfc/0001-settlement-and-payouts.md)'s "vpay pays nobody out".

## Fapshi beside Notch Pay

|                              | `notchpay` (built)                                     | Fapshi (documentation)                                       |
| ---------------------------- | ------------------------------------------------------ | ------------------------------------------------------------ |
| Rule 3                       | admitted under ADR-0031: the read takes the public key | passes as written: `apiuser` + secret `apikey` on every call |
| Push submit                  | two calls: initialize, process                         | one call: `direct-pay`, off in live mode until approved      |
| Network choice               | `cm.mobile` (auto); `cm.mtn` / `cm.orange`             | `medium`, or omitted for auto                                |
| A repeated create            | `409` "Reference already existing"                     | undocumented: may prompt twice                               |
| Find a payment by **our** id | no: `search` matches Notch Pay's `trx.…` only          | yes, by `userId`, if the lookup is consistent (to measure)   |
| Failure reasons              | none: `failed`, `expired`, `canceled`                  | none: `FAILED`                                               |
| Short collection             | `amounts.total` short of `amount`                      | `amount` vs `revenue` (net of fees)                          |
| Webhook registration         | API: vpay registers its own (RFC-0013)                 | dashboard only; every read echoes the URL                    |
| Webhook authentication       | HMAC `x-notch-signature`; the secret is never returned | static secret, `x-wh-secret`                                 |
| Webhook retries              | not documented                                         | none: one attempt per event                                  |
| Refunds                      | a guide; the sandbox answered `403`                    | none                                                         |
| Status-read limit            | none documented                                        | 6 per minute per transaction                                 |
| Other wallets                | Express Union, Yoomee                                  | Fapshi wallet (payouts only)                                 |
| Legal entity                 | not named                                              | "Fapshi Inc"; no licence or regulator named                  |
| Sandbox access               | test keys from the dashboard                           | an activated account: KYC, Cameroonian ID                    |

## What to ask Fapshi, in writing

1. Is `externalId` unique per service? What does a second `direct-pay` with
   the same `externalId` answer? Is there an idempotency key?
2. Does `GET /transaction/{userId}` return a Direct Pay transaction as soon as
   its create has answered?
3. Does a `FAILED` ever say why (declined, insufficient funds, no answer)?
   What ends an unanswered prompt, and after how long?
4. What does `GET /payment-status/{transId}` answer for a `transId` the
   service never issued: status code and `message`?
5. Will Direct Pay be approved for a payment gateway that serves merchants'
   web and mobile checkouts?
6. Where is Fapshi Inc incorporated, which entity holds the CEMAC payment
   licence, under which regulator, with which partner bank? Can a company
   hold an account, and must its owner hold a Cameroonian ID?
7. Is there, or will there be, a refund API? Is a payout from a separate
   service the intended way to refund?
8. Are webhooks ever retried? Is `x-wh-secret` ever an HMAC rather than the
   secret itself?
9. Can a `SUCCESSFUL` collection have collected less than `amount`? Is
   `revenue` always `amount` minus the fee?
10. Is the international form `+2376…` accepted for `phone`?

## What a first sandbox session must measure

It needs an activated account, sandbox keys and the maintainer's approval for
each call, as Notch Pay's runbook did.

- The documented MTN and Orange outcome numbers through `direct-pay`, read to
  a terminal status, with timings.
- A second `direct-pay` with the same `externalId`.
- `GET /transaction/{userId}` immediately after a create answers.
- `payment-status` for a made-up `transId`.
- The `429` at the seventh read in a minute.
- `webhook` in the read, and with a public URL, one delivered webhook with
  its `x-wh-secret`.

## If an adapter is wanted

The order [RFC-0012](../rfc/0012-notchpay-adapter.md) followed, shortened by what Notch Pay already built:

0. The written questions above, sent by the maintainer.
1. An RFC, the design; and a `vpay-fapshi` client from `openapi.json`,
   linked by nothing.
2. `vpay-adapter-fapshi`: Direct Pay as `Push`, `medium` from the payer's
   method type, lookup-before-resend on `userId`, `supports_refunds: false`,
   the `webhook` check on every read.
3. WireMock mappings and the conformance row.
4. Wiring, status pages, skills.
5. The sandbox session above, through vpay, on the maintainer's machine.

Later, and only if needed: Initiate Pay as a `Redirect` shape if Direct Pay
is refused; `x-wh-secret` once RFC-0014 gives `parse_callback` its headers.

## Sources

Fetched 2026-10-23:

- [getting started](https://docs.fapshi.com/en/api-reference/getting-started)
- [API credentials](https://docs.fapshi.com/en/api-reference/preliminary-knowledge/api-credentials)
- [environments and test numbers](https://docs.fapshi.com/en/api-reference/preliminary-knowledge/environment)
- [request status and IP allowlist](https://docs.fapshi.com/en/api-reference/preliminary-knowledge/request-status)
- [initiate-pay](https://docs.fapshi.com/en/api-reference/endpoint/initiate-pay)
- [direct-pay](https://docs.fapshi.com/en/api-reference/endpoint/direct-pay)
- [payment-status](https://docs.fapshi.com/en/api-reference/endpoint/payment-status)
- [expire-pay](https://docs.fapshi.com/en/api-reference/endpoint/expire-pay)
- [payout](https://docs.fapshi.com/en/api-reference/endpoint/payout)
- [search](https://docs.fapshi.com/en/api-reference/endpoint/search-transactions)
- [transactions by user](https://docs.fapshi.com/en/api-reference/endpoint/transaction-by-user)
- [webhooks](https://docs.fapshi.com/en/api-reference/endpoint/webhook)
- [create a service](https://docs.fapshi.com/en/api-reference/create-a-service)
- [activate your account](https://docs.fapshi.com/en/guides/getting-started/activate-account)
- [openapi.json](https://docs.fapshi.com/en/api-reference/openapi.json)
- [terms and conditions](https://fapshi.com/en/terms-and-conditions)
- [activating Direct Pay](https://www.fapshi.com/en/help-and-support/how-to-activate-direct-pay-for-your-fapshi-api)
- [Direct Pay vs Initiate Pay](https://www.fapshi.com/en/help-and-support/direct-pay-vs-initiate-pay-all-you-need-to-know)
- [the DGT list of approved payment institutions](https://dgtcfm.cm/en/approved-payment-institutions/), seen only as a search result
