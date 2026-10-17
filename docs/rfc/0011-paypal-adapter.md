# RFC-0011: A PayPal adapter — Orders v2 behind the port, and the capture that approval is not

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-10-17
- **Related:** [RFC-0010](0010-stripe-adapter.md) (the Stripe adapter; the two
  core changes it names are shared), [RFC-0004](0004-billing-on-top-of-invoices.md)
  § 7 (one adapter per provider), [RFC-0008](0008-payment-method-routing.md)
  (method types), [RFC-0009](0009-provider-accounts-per-merchant.md) (provider
  accounts per merchant), [RFC-0003](0003-refunds-destinations-and-the-first-ledger-postings.md)
  (refunds), [ADR-0002](../adr/0002-provider-port.md),
  [ADR-0006](../adr/0006-no-mocks-in-main-processes.md),
  [ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md),
  [flows/adapter-orange-money.md](../flows/adapter-orange-money.md),
  [flows/reconciler.md](../flows/reconciler.md) (callbacks are hints),
  [flows/money.md](../flows/money.md).

Nothing in this document is built. It is the design of one adapter crate,
`vpay-adapter-paypal`, rail code `paypal`. Every PayPal fact below was read on
developer.paypal.com on 2026-10-17 and is cited; nothing has been called, in
sandbox or anywhere else.

## Problem

PayPal is the second provider RFC-0004 § 7 admits on its public
documentation, and the one a payer without a card is most likely to hold:

| Hard rule (RFC-0004 § 7)               | How PayPal Orders v2 meets it                                                                                                                                                                                                         |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| vpay never holds the money             | A capture lands in the account named by `purchase_units[].payee` — the merchant's own app, or a merchant a partner acts for with `PayPal-Auth-Assertion` ([make-api-requests](https://developer.paypal.com/api/make-api-requests.md)) |
| `vpay-server` never sees a card number | The payer approves on PayPal's page; a card typed there is PayPal's guest checkout, on PayPal's page                                                                                                                                  |
| A secret-authenticated status query    | `GET /v2/checkout/orders/{id}` with an OAuth bearer minted from the client secret ([orders-get](https://developer.paypal.com/api/orders/v2/orders-get))                                                                               |
| A stub-able contract                   | The Orders v2 and Payments v2 references publish every request, response and error issue, plus a sandbox with a documented negative-testing header                                                                                    |

Two facts from the 2026-10-16 research bound what it is for. PayPal marks
Cameroon, Côte d'Ivoire and Gabon **"send only"** — "Sellers in send-only
countries cannot receive payments" ([seller onboarding](https://developer.paypal.com/platforms/seller-onboarding#country-eligibility)) —
and neither XAF nor XOF is a PayPal currency
([currency codes](https://developer.paypal.com/api/rest/reference/currency-codes/)).
So this adapter serves merchants in PayPal's receiving countries, Senegal
among them, in PayPal's currencies.

**And one fact decides the shape of this RFC.** On Stripe, the hosted page
takes the money. On PayPal, the payer's approval takes nothing: "the buyer
must first approve the order", and then the merchant's server **captures** it
([orders-capture](https://developer.paypal.com/api/orders/v2/orders-capture)).
PayPal's `processing_instruction=ORDER_COMPLETE_ON_PAYMENT_APPROVAL` would
make PayPal capture on approval, and it "is currently only available for
the following payment_source: Alipay, BANCOMAT Pay, Bancontact, BLIK, …,
WeChat Pay" — a list that does not include the PayPal wallet
([order_request](https://developer.paypal.com/api/orders/v2/definitions/order_request)).
vpay has to make a second, money-moving call after the redirect, and the port
has no method for it. § 3 is where that call goes.

## Proposal

### 1. Shape

| Property                         | Value                                                                                                                     |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Rail code                        | `paypal`                                                                                                                  |
| Crate                            | `backends/crates/vpay-adapter-paypal/`, the `lib.rs` / `wire.rs` / `mapping.rs` / `token.rs` split of the Orange crate    |
| `flow`                           | `Redirect`                                                                                                                |
| `supports_refunds`               | `true`                                                                                                                    |
| `supports_partial_refunds`       | `true` — `amount` on the capture refund ([captures-refund](https://developer.paypal.com/api/payments/v2/captures-refund)) |
| `delivers_callbacks`             | `true` — webhooks, registered out of band (§ 6)                                                                           |
| `requires_ip_allowlist`          | `false`                                                                                                                   |
| `supports_account_holder_lookup` | `false`                                                                                                                   |
| `refund_destination`             | `Origin` — the second such rail; RFC-0010 § 7 pays the port's debts, this rail inherits them                              |
| `payer_fields`                   | the default `&[]`                                                                                                         |
| `parse_destination`              | the port's default `Unsupported`                                                                                          |
| Method type (RFC-0008)           | `paypal`. Today a merchant sends `payment_method_types[]=paypal`. Not `card`: § 4 says why                                |

### 2. Configuration, on RFC-0009's two levels

```yaml
# Illustrative.
providers:
  - code: paypal
    host:
      url: https://api-m.paypal.com # api-m.sandbox.paypal.com in sandbox; a WireMock host in compose
      label: paypal-live
    display_name: { en: PayPal, fr: PayPal }
    currency: EUR
    settings:
      order_validity_seconds: 10800 # PayPal's documented 3 hours; up to 259200 if PayPal extended the account's
      landing_page: NO_PREFERENCE # LOGIN | GUEST_CHECKOUT | NO_PREFERENCE
      payment_method_preference: IMMEDIATE_PAYMENT_REQUIRED
      brand_name: Acme Cameroun
      locale: fr-FR
    credentials: {} # rail-level; see the two models
```

| Model                         | Rail-level (`providers[]`)                                                                   | Account-level (`provider_accounts[]`, RFC-0009)      | Who the operator is to PayPal                                                                                                                                                                                                          |
| ----------------------------- | -------------------------------------------------------------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A. The merchant's own app** | nothing                                                                                      | `credentials.client_id`, `credentials.client_secret` | Nobody. A first-party integration on the merchant's REST app                                                                                                                                                                           |
| **B. Partner, third-party**   | `credentials.client_id`, `credentials.client_secret` (the partner's app), `settings.bn_code` | `settings.merchant_id` (13 characters)               | An **approved** PayPal partner, sending `PayPal-Auth-Assertion` and `payee.merchant_id` on every order, with each merchant's explicit consent ([multiparty](https://developer.paypal.com/docs/multiparty/checkout/immediate-capture/)) |

The assertion is PayPal's own shape: a header `{"alg":"none"}`, a payload
`{"iss": <client_id>, "payer_id": <merchant_id>}`, an empty signature, the
three base64-encoded and joined with periods
([make-api-requests](https://developer.paypal.com/api/make-api-requests.md)).
It is not a secret and not a signature; it is a statement of whom the
partner acts for, which PayPal checks against the consent the merchant gave.
Onboarding a merchant to a partner (Partner Referrals) is out of scope: the
`merchant_id` arrives by PR, as a merchant's JWK does (ADR-0010).
`payment_instruction.platform_fees` is **not sent in v1**; it is RFC-0010's
open question 5 under another name.

**Until RFC-0009 lands, there is one level**, so a deployment serves one
merchant on model A, or one merchant of one partner on model B.

Boot refusals, in `vpay_config::config::REQUIRED_RAIL_KEYS` per ADR-0012:

- `client_id` and `client_secret` are required. **Neither encodes the
  environment** — PayPal's docs separate sandbox and live credentials only by
  which Dashboard tab they came from — so the host is the only signal, and
  `sandbox` joins `vpay_config`'s `STUB_MARKERS`: a `livemode: true`
  deployment whose `host.url` names a sandbox refuses to boot, on every rail
  (MTN's sandbox host carries the word too, and refusing it under livemode
  is equally right).
- `merchant_id`, when present, is exactly 13 characters
  ([purchase_unit_request](https://developer.paypal.com/api/orders/v2/definitions/purchase_unit_request)).
- `order_validity_seconds` is within 10800 and 259200 (§ 5).
- `landing_page` and `payment_method_preference` are members of their enums
  ([experience_context](https://developer.paypal.com/api/orders/v2/definitions/paypal_wallet_experience_context)).

### 3. The wire — four calls, and where the capture lives

Every call carries `Authorization: Bearer <token>` and, on model B,
`PayPal-Auth-Assertion` and `PayPal-Partner-Attribution-Id: <bn_code>`.
Bodies are JSON. Every mutating call carries a `PayPal-Request-Id`, which
PayPal "stores for up to 45 days" and answers a repeat of with "the result
of the original call" ([make-api-requests](https://developer.paypal.com/api/make-api-requests.md)).
The ids are **derived, never reused across calls**: the order create uses
the charge reference; the capture uses a UUIDv5 of the reference under a
crate-constant namespace; the refund uses the refund's own reference. Whether
PayPal scopes a request id per endpoint is undocumented, and an id reused
from the create on the capture could be answered with the create's result;
deriving makes the question moot. All are 36 characters, under the
idempotency guide's 38 ([idempotency](https://developer.paypal.com/api/rest/reference/idempotency.md)).

#### 3.0 The token — `POST {base_url}/v1/oauth2/token`

`Basic <client_id:client_secret>`, `grant_type=client_credentials` →
`access_token`, `expires_in` ([authentication](https://developer.paypal.com/api/rest/authentication/)).
Cached through `vpay_provider::token` exactly as Orange's is: one
`CachedToken` per adapter, fingerprinted over `client_id` and
`client_secret` so a second account's configuration mints its own, a
per-rail margin before `expires_in`, and a missing `expires_in` meaning "use
once, do not cache". A 401 re-mints exactly once and then reports
`provider_account_blocked`, the rule both shipping adapters follow. No
`expires_in` is nominal in PayPal's docs (the sample shows 31668 s), so the
margin is a crate constant and not a guess at the lifetime.

#### 3.1 `submit` — `POST {base_url}/v2/checkout/orders`

```http
POST /v2/checkout/orders
PayPal-Request-Id: <ChargeRef::reference_id>
Prefer: return=representation

{ "intent": "CAPTURE",
  "purchase_units": [{
    "reference_id": "<reference_id>",
    "custom_id": "<reference_id>",
    "amount": { "currency_code": "EUR", "value": "50.00" },
    "payee": { "merchant_id": "<merchant_id>" }            ← model B only
  }],
  "payment_source": { "paypal": { "experience_context": {
    "return_url": "<ChargeRef::return_url>",
    "cancel_url": "<ChargeRef::return_url>",
    "user_action": "PAY_NOW",
    "shipping_preference": "NO_SHIPPING",
    "payment_method_preference": "IMMEDIATE_PAYMENT_REQUIRED",
    "landing_page": "NO_PREFERENCE",
    "brand_name": "…", "locale": "fr-FR" } } } }
→ 201 { "id": "5O190127TN364715T", "status": "PAYER_ACTION_REQUIRED",
        "create_time": "…", "links": [ { "rel": "payer-action", "href": "…" }, … ] }
```

- **`value` is `Money::to_provider_string()`** — `"50.00"` for EUR,
  `"5000"` for a zero-decimal currency — because PayPal's `value` is "integer
  for currencies like JPY … decimal fraction for currencies like TND" and
  `DECIMAL_PRECISION` refuses more than two places
  ([purchase_unit_request](https://developer.paypal.com/api/orders/v2/definitions/purchase_unit_request),
  [errors](https://developer.paypal.com/api/rest/reference/orders/v2/errors/)).
  This is the string encoding [money.md](../flows/money.md) already has;
  Orange takes the integer one. The adapter refuses, before any call, a
  currency outside PayPal's published 25 as `ProviderError::Config`, so a
  misconfigured deployment fails at the first charge with a sentence rather
  than at PayPal with `INVALID_CURRENCY_CODE`. Today that list meets vpay's
  `Currency` enum at exactly one member, EUR; growing the enum is open
  question 8.
- **`custom_id` carries the reference and `invoice_id` is not sent.**
  `custom_id` "appears in transaction and settlement reports" and is not
  shown to the payer; `invoice_id` is shown to the payer and "must be unique
  within each merchant account by default", which turns a merchant's own
  numbering into a `DUPLICATE_INVOICE_ID` 422 ([purchase_unit_request](https://developer.paypal.com/api/orders/v2/definitions/purchase_unit_request)).
  Webhooks are parsed by `custom_id` (§ 6).
- **`payment_source.paypal` is what makes the response
  `PAYER_ACTION_REQUIRED` with a `payer-action` link**; the legacy form
  without it answers `CREATED` with an `approve` link
  ([standard integration](https://developer.paypal.com/api/rest/integration/orders-api/api-use-cases/standard.md)).
  The adapter follows `payer-action` and treats a response with neither as
  `Malformed`.
- **`return_url` and `cancel_url` carry the same value, verbatim**, as on
  Orange and Stripe: the outcome comes from the status read.
- **`IMMEDIATE_PAYMENT_REQUIRED`** keeps eCheck and other delayed funding
  off the page, so a capture is `COMPLETED` or refused and never `PENDING`
  for days (open question 3). **`NO_SHIPPING`** because a charge is not a
  parcel. **`PAY_NOW`** puts "Pay Now" rather than "Continue" on PayPal's
  button, which is what a payer who will be captured on return should read.

Out of the response the adapter keeps `id`, the `payer-action` href and
`create_time`: `Submitted { redirect_url, ref_extra: { order_id, expires_at:
create_time + order_validity_seconds } }`. The order id persisted before the
redirect is what the redirect-flow precondition requires.

#### 3.2 `query_status` — read, then complete what the payer approved

```http
GET /v2/checkout/orders/{order_id}
→ 200 { "status": "…", "purchase_units": [{ "payments": { "captures": [ … ] } }] }
```

**When the order is `APPROVED`, `query_status` captures it:**

```http
POST /v2/checkout/orders/{order_id}/capture
PayPal-Request-Id: uuid5(<reference_id>, "capture")
Prefer: return=representation
→ 201 { "status": "COMPLETED", "purchase_units": [{ "payments": { "captures": [
     { "id": "3C679366HH908993F", "status": "COMPLETED", "custom_id": "<reference_id>",
       "seller_receivable_breakdown": { "paypal_fee": { "value": "…" } } } ] } }] }
```

This is the decision this RFC exists to record. The port says the
authenticated status query is "the only thing that moves money", and on
every rail so far that sentence was about vpay's state: the rail had already
moved the money, and the status read was what let vpay say so. On PayPal the
sentence is literal. An approved order is a payer who has said yes and a
rail waiting to be told to proceed; the worker's status read is the one
place in vpay that asks a rail about a charge on a schedule, with the
reconciler's recovery semantics behind it, and it is where the capture
belongs:

- **It is idempotent.** The derived `PayPal-Request-Id` makes a capture
  retried after a crash a replay, and a capture that raced an earlier one
  answers `ORDER_ALREADY_CAPTURED` or `ORDER_COMPLETION_IN_PROGRESS`, both of
  which are "read the order again", not failures.
- **It is crash-safe by the existing ordering.** A capture whose result was
  never persisted is found on the next rung as a `COMPLETED` order with a
  capture in it, and reported `Succeeded` then. Nothing new has to be durable
  before the call.
- **It keeps one charge per intent.** `INSTRUMENT_DECLINED` on capture is
  PayPal's "restart the payment so the payer can select a different payment
  option" ([funding failures](https://developer.paypal.com/docs/checkout/standard/customize/handle-funding-failures)),
  and in vpay a restart is a new PaymentIntent, as RFC-0008 § 8 says. The
  charge is `Failed(payer_declined)`; the adapter never re-redirects.
- **The alternative is a port change for one rail**: a `complete` method
  and a `ChargeStatus::Approved` the worker would branch on. Open
  question 1 keeps it open; this RFC does not take it.

The outcome is a pure function of the two responses:

| Order `status`                                                                          | Capture                                                                                          | → `ChargeStatus`                                                                                                                  |
| --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| `CREATED`, `SAVED`, `PAYER_ACTION_REQUIRED`                                             | —                                                                                                | `Pending`; **`Failed(payer_timeout)` once `now > expires_at`** (§ 5)                                                              |
| `APPROVED`                                                                              | `201`, capture `COMPLETED`                                                                       | `Succeeded { provider_txn_id: <capture id> }`                                                                                     |
| `APPROVED`                                                                              | `201`, capture `PENDING`                                                                         | `Pending` — `status_details.reason` (`ECHECK`, `PENDING_REVIEW`, `UNILATERAL`, …) kept in the log line                            |
| `APPROVED`                                                                              | `201`, capture `DECLINED` or `FAILED`                                                            | `Failed(map(status_details.reason))`                                                                                              |
| `APPROVED`                                                                              | `422` with an `issue`                                                                            | `Failed(map(issue))` — or `Pending` for `ORDER_NOT_APPROVED`, `PAYER_ACTION_REQUIRED`                                             |
| `APPROVED`                                                                              | `422 ORDER_ALREADY_CAPTURED`, `ORDER_COMPLETION_IN_PROGRESS`, `409 PREVIOUS_REQUEST_IN_PROGRESS` | `Pending` — the next rung reads the capture                                                                                       |
| `COMPLETED`                                                                             | `captures[0].status` `COMPLETED`, `REFUNDED`, `PARTIALLY_REFUNDED`                               | `Succeeded { provider_txn_id: <capture id> }` — a refund made in PayPal's Dashboard is not vpay's, and the charge still succeeded |
| `COMPLETED`                                                                             | `captures[0].status` `PENDING`                                                                   | `Pending`                                                                                                                         |
| `COMPLETED`                                                                             | `captures[0].status` `DECLINED`, `FAILED`                                                        | `Failed(map(status_details.reason))`                                                                                              |
| `VOIDED`                                                                                | —                                                                                                | `Failed(provider_error)` — cancelled at PayPal, by nobody in vpay                                                                 |
| 404 `RESOURCE_NOT_FOUND`                                                                |                                                                                                  | `NotFound`                                                                                                                        |
| 401 after one re-mint, 403 `PERMISSION_DENIED`, `NOT_AUTHORIZED`, `PAYEE_NOT_CONSENTED` |                                                                                                  | `Err(Rejected(provider_account_blocked))`                                                                                         |
| 429 `RATE_LIMIT_REACHED`, 5xx, 409                                                      |                                                                                                  | `Err(Transport)` — `Retry::AfterBackoff`                                                                                          |
| 3xx                                                                                     |                                                                                                  | `Err(Malformed)` — never followed                                                                                                 |

The enums are PayPal's: order status `CREATED`, `SAVED`, `APPROVED`,
`VOIDED`, `COMPLETED`, `PAYER_ACTION_REQUIRED`
([order_status](https://developer.paypal.com/api/orders/v2/definitions/order_status));
capture status `COMPLETED`, `DECLINED`, `PARTIALLY_REFUNDED`, `PENDING`,
`REFUNDED`, `FAILED` and the `status_details.reason` list
([capture](https://developer.paypal.com/api/orders/v2/definitions/capture)).
A charge whose `ref_extra` has no `order_id` is `ProviderError::Config`,
never `NotFound`, for Orange's `pay_token` reason.

#### 3.3 `refund` — `POST {base_url}/v2/payments/captures/{capture_id}/refund`

```http
PayPal-Request-Id: <the refund's own reference>
{ "amount": { "currency_code": "EUR", "value": "20.00" }, "custom_id": "<refund reference>" }
→ 201 { "id": "…", "status": "COMPLETED", "seller_payable_breakdown": { "paypal_fee": { "value": "0.00" } } }
```

- The capture id is the charge's `provider_txn_id`, which reaches the
  adapter through RFC-0010 § 7's first core change. A charge with none is
  not refundable, and that is the handler's refusal before any call.
- `status` `COMPLETED` or `PENDING` (reason `ECHECK`) →
  `Ok(Refunded { ref_extra: { refund_id, status, paypal_fee }, fee: None })`;
  `FAILED` or `CANCELLED` → `Err(Rejected(provider_error))`
  ([refund](https://developer.paypal.com/api/orders/v2/definitions/refund)).
- **`fee` is `None` in v1, and `paypal_fee` is kept raw in `ref_extra`.**
  PayPal's refund carries `seller_payable_breakdown.paypal_fee`, which would
  make this the first rail to answer the question issue #46 asked. But the
  field's documentation is the generic money description, the sample shows
  `0.00`, and PayPal's policy page says "there are no fees to make the
  refund, but the fees you originally paid to receive the payment are not
  returned to you" ([merchant fees](https://www.paypal.com/us/webapps/mpp/merchant-fees)).
  Whether `paypal_fee` on a refund is the refund's own cost, the original
  fee returned, or always zero is item 3 of § 10, and a merchant's `refund`
  object will carry `Some(…)` only once a real response has said which.
  Promoting it needs `Money::from_provider_string`, the parse that
  `to_provider_string` lacks — exact, no floats — and nothing else.
- The 422 issues `REFUND_AMOUNT_EXCEEDED`, `CAPTURE_FULLY_REFUNDED`,
  `MAX_NUMBER_OF_REFUNDS_EXCEEDED`, `REFUND_NOT_ALLOWED` and
  `REFUND_TIME_LIMIT_EXCEEDED` (180 days from the capture —
  [refund-payment](https://developer.paypal.com/checkout/refund-payment.md))
  are each `Err(Rejected(provider_error))` carrying the issue: a decline,
  never an accepted refund. A duplicate is the request id replaying the
  original.
- As on Stripe, `Ok` is an instruction: `COMPLETED` is what PayPal usually
  answers, and vpay's refund row stays `pending` (RFC-0003 open question 8).

#### 3.4 Issues → `FailureCode`

`mapping.rs` holds one table over PayPal's **issue names** — the capture's
`status_details.reason` and the 422 `details[].issue` of the create, capture
and refund calls — case-sensitive as PayPal spells them, an unmapped name
falling through to `provider_error` carrying the raw issue. The create and
capture issue lists are snapshotted from
[the Orders v2 error reference](https://developer.paypal.com/api/rest/reference/orders/v2/errors/)
as a `#[cfg(test)]` const, so every published issue is either mapped or
deliberately unmapped.

| `FailureCode`              | PayPal issue or reason                                                                                                                                                                                                                                                                    |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `payer_declined`           | `INSTRUMENT_DECLINED`, `PAYER_CANNOT_PAY`, `TRANSACTION_REFUSED`, `PAYMENT_DENIED`, `DECLINED_DUE_TO_RELATED_TXN`, `REDIRECT_PAYER_FOR_ALTERNATE_FUNDING`, `CONTINGENCY_NOT_SUCCESSFUL`, `COMPLIANCE_VIOLATION`, `DOMESTIC_TRANSACTION_REQUIRED`; reason `DECLINED_BY_RISK_FRAUD_FILTERS` |
| `payer_account_blocked`    | `PAYER_ACCOUNT_LOCKED_OR_CLOSED`, `PAYER_ACCOUNT_RESTRICTED`                                                                                                                                                                                                                              |
| `payer_limit_reached`      | `TRANSACTION_LIMIT_EXCEEDED`, `MAX_NUMBER_OF_PAYMENT_ATTEMPTS_EXCEEDED`                                                                                                                                                                                                                   |
| `invalid_payer`            | `CARD_BRAND_NOT_SUPPORTED`, `CARD_COUNTRY_NOT_SUPPORTED`, `REFERENCED_CARD_EXPIRED`, `PAYMENT_SOURCE_NOT_SUPPORTED`, `COUNTRY_NOT_SUPPORTED_BY_PAYMENT_SOURCE`                                                                                                                            |
| `payee_account_blocked`    | `PAYEE_ACCOUNT_RESTRICTED`, `PAYEE_ACCOUNT_LOCKED_OR_CLOSED`, `PAYEE_BLOCKED_TRANSACTION`, `TRANSACTION_BLOCKED_BY_PAYEE`, `TRANSACTION_RECEIVING_LIMIT_EXCEEDED`                                                                                                                         |
| `invalid_payee`            | `PAYEE_ACCOUNT_INVALID`, `PAYEE_NOT_ENABLED_FOR_CARD_PROCESSING`, `PAYEE_NOT_ENABLED_FOR_BANK_PROCESSING`, `NOT_ENABLED_FOR_CARD_PROCESSING`, `PAYMENT_ORIGIN_NOT_ENABLED`, `AUTH_CAPTURE_NOT_ENABLED`, `NOT_ENABLED_FOR_PAYMENT_SOURCE`                                                  |
| `payer_timeout`            | the local deadline on an order no payer approved (§ 5)                                                                                                                                                                                                                                    |
| `provider_account_blocked` | HTTP 401 after one re-mint; 403 `PERMISSION_DENIED`, `NOT_AUTHORIZED`, `PAYEE_NOT_CONSENTED`, `CONSENT_NEEDED`                                                                                                                                                                            |
| `provider_error`           | anything unmapped; `VOIDED`; a lapsed approval window (§ 5); every refund issue                                                                                                                                                                                                           |

So `PRODUCED_FAILURE_CODES` is **nine of the eleven**, and a different nine
from Stripe's: `insufficient_funds` is unreachable because the wallet
reports a payer who cannot fund a payment as `INSTRUMENT_DECLINED` or
`PAYER_CANNOT_PAY` and never says why, and `provider_unavailable` is
unreachable because PayPal's "come back later" answers (`409`,
`ORDER_COMPLETION_IN_PROGRESS`, `503`) are transport errors the worker
retries, not declines. Where Stripe cannot say anything about a payee,
PayPal says five things, because a payee there is an account PayPal can
restrict. [failures.md](../flows/failures.md) gains a column.

### 4. What the payer sees, and why this rail is not `card`

The existing checkout: `paypal` is one more `Redirect` rail with the
deployment's `display_name`, the "Redirecting to the rail" screen sends the
payer to PayPal, and vpay's return page reads the intent's status before
forwarding them. On that page a payer logs in and approves, or — with
`landing_page=GUEST_CHECKOUT` — types a card into PayPal's own form. That
form is "presented based on several risk factors" and needs account
settings a merchant controls ([help307](https://www.paypal.com/cshelp/article/help307));
vpay cannot know whether a given payer will be offered it. So under
RFC-0008 this rail declares `method_types: [paypal]` and not `card`: a rail
may not promise a method it cannot see. `landing_page` stays a setting, so a
merchant who has guest checkout enabled can lead with it.

### 5. Two clocks: the order's, and the approval's

**An unapproved order does not expire on PayPal's side in any way vpay can
read.** "After you create an order, you have 3 hours to capture or modify
that order. You can extend it up to 72 hours based on your use case"
([standard integration](https://developer.paypal.com/api/rest/integration/orders-api/api-use-cases/standard.md)),
and the status enum has no expired value; no page says what `GET` returns
afterwards. Orange reports `EXPIRED`; Stripe reports `expired`; PayPal, on
its documentation, reports nothing. So the adapter times out locally: the
submit records `expires_at = create_time + order_validity_seconds`, and a
status read that finds the order still unapproved past it answers
`Failed(payer_timeout)`. The default is PayPal's documented three hours;
the ceiling is the 72 the document names as the extension PayPal grants.
This is the one place this adapter decides an outcome PayPal did not state,
it is bounded by PayPal's own figures, and it is item 1 of § 10.

**An approved order must be captured within a window** that "is controlled
by the merchant, but the default is 3 hours"; when it lapses PayPal sends
`CHECKOUT.PAYMENT-APPROVAL.REVERSED`, cancels the order and refunds the
buyer ([uncaptured payments](https://developer.paypal.com/payment-methods/uncaptured-payments.md)).
The worker's ladder (`vpay_worker::poll_delay`: 10, 20, 30, 45, 60, 90 s,
then 120 s to the twentieth attempt, then 15 min) captures an approval
within one rung, which is under 15 minutes for the whole of those three
hours. The case this does not cover is a worker that was down for three
hours while a payer approved, and that case is an alert
(`provider_error`), not a payer's fault.

**The payer's return is the strongest hint there is, and today nothing
reads it.** `GET /v1/browser/checkout/sessions/{id}/return` renders the
intent's status and does not pull the charge's `poll_charge` job forward;
the callback handler does. A payer who approves and returns within 30 s of
the redirect is therefore captured at the ladder's next rung — up to two
minutes later, watching vpay's "waiting" state. Two things close that gap,
and both are the hint design applied where it was not yet:

1. PayPal's `CHECKOUT.ORDER.APPROVED` webhook on
   `/provider/paypal/callback` (§ 6), which pulls the poll forward within the
   handler's ten-second floor.
2. **The return page pulls the poll forward too** — one call to the same
   `pull_forward_in_tx` the callback handler uses, with the same floor. It is
   a core change, it benefits Orange exactly as much, and it is the third
   core change this RFC proposes (§ 7).

### 6. Webhooks are callbacks, and callbacks are hints

`parse_callback(body)` reads a PayPal event
([webhooks](https://developer.paypal.com/api/rest/webhooks/)):

```json
{ "event_type": "PAYMENT.CAPTURE.COMPLETED",
  "resource": { "id": "<capture id>", "custom_id": "<reference>",
                "supplementary_data": { "related_ids": { "order_id": "…" } } } }
```

→ `CallbackRef { reference_id, ref_extra: { event_type, order_id } }`. The
reference is `resource.custom_id` on capture events and
`resource.purchase_units[0].custom_id` on order events; a body with neither,
a non-UUID, or an `event_type` outside the subscribed set is `Malformed`.
The subscribed set is `CHECKOUT.ORDER.APPROVED`,
`CHECKOUT.PAYMENT-APPROVAL.REVERSED`, `PAYMENT.CAPTURE.COMPLETED`,
`PAYMENT.CAPTURE.PENDING`, `PAYMENT.CAPTURE.DECLINED`,
`PAYMENT.CAPTURE.DENIED`, `PAYMENT.CAPTURE.REFUNDED` and
`PAYMENT.CAPTURE.REVERSED` ([event names](https://developer.paypal.com/api/rest/webhooks/event-names/)
— `DENIED` is listed against Payments v1 and `DECLINED` against v2, so both
are subscribed). The handler does what it does for every rail: pulls the
poll forward and discards `ref_extra`.

- **The URL is registered with PayPal**, in the Dashboard or with
  `POST /v1/notifications/webhooks` ([webhooks-post](https://developer.paypal.com/api/webhooks/v1/webhooks-post)),
  as `{public_base_url}/provider/paypal/callback`. The conformance suite's
  `callback_url_pattern` is `None` for this rail, under the suite change
  RFC-0010 § 8 makes.
- **PayPal signs its webhooks, and vpay cannot check the signature today.**
  The scheme is RSA over `transmission_id|timestamp|webhook_id|crc32(body)`
  with the headers `PAYPAL-TRANSMISSION-ID`, `-TIME`, `-SIG`, `PAYPAL-CERT-URL`
  and `PAYPAL-AUTH-ALGO`, or a call to
  `POST /v1/notifications/verify-webhook-signature`
  ([webhooks/rest](https://developer.paypal.com/api/rest/webhooks/rest/)).
  Both need headers `parse_callback` does not receive, and the second needs
  the `webhook_id` as configuration. RFC-0010's open question 4 covers both
  rails; until it is answered, an unverified PayPal webhook is worth one
  authenticated status read and no money.

### 7. Core changes

Two are RFC-0010 § 7's, and this rail is their second consumer:

1. `refund_at_rail` passes the charge's `provider_txn_id` — here, the capture
   id — in the refund `ChargeRef`'s `ref_extra`.
2. The `Origin` assertion in the conformance suite, relaxed by paying its
   three debts.

Two are this RFC's own, and both are small:

3. **The return page pulls the poll forward** (§ 5): the hosted checkout's
   return handler calls `pull_forward_in_tx` for the session's charge, with
   the callback handler's floor. It is a change in `vpay_api::browser`, it
   names no rail, and every redirect rail gains from it.
4. **`sandbox` joins `STUB_MARKERS`** (§ 2): one word in `vpay_config`, with
   the test the other four markers have.

And one is deferred until a real response earns it: `Money::from_provider_string`,
when `fee` is promoted from `ref_extra` to `Some` (§ 3.3).

If the implementation finds it needs a fifth — a worker branch, a reconciler
rule, a new `ChargeStatus` — the capture-in-`query_status` decision was
wrong and open question 1 is the place to say so, not a patch.

### 8. Proving it

- **Unit tests in the crate**, no container: the two mapping tables row by
  row, the issue snapshot, the order body a submit produces, the derived
  request ids, the local-deadline rule at the boundary, the three
  `Malformed` callback shapes, and a `Debug` canary on every type that holds
  a credential or a token.
- **WireMock mappings** under `backends/tests/conformance/wiremock/paypal/mappings/`:
  `token.json`; `orders-create.json`, keyed on `PayPal-Request-Id` as MTN's
  are on `X-Reference-Id`, the order id templated from it; `orders-get.json`
  and `orders-capture.json`, keyed on the id's path; `captures-refund.json`;
  `stub-hosted-page.json` and `demo-outcomes.json` on Orange's model. The
  suite's `REF_SCENARIO` ("first status query pending, second successful")
  becomes a `GET` answering `PAYER_ACTION_REQUIRED` then `APPROVED`, with a
  capture mapping answering `201 COMPLETED` — the case passes unchanged, and
  the capture-in-`query_status` design is what lets it. The decline block is
  a `GET APPROVED` followed by a capture `422` per issue, plus a
  `GET COMPLETED` whose capture is `DECLINED` for the reason row.
- **The shared conformance suite**, with one `RailUnderTest::PayPal` row per
  table function, under the three suite changes RFC-0010 § 8 already
  makes. `refund_body_pattern` is the capture refund path plus the
  `PayPal-Request-Id` header equal to the refund's reference.
- **The first real call**, against PayPal's sandbox: the default business
  and personal sandbox accounts, and the `PayPal-Mock-Response` header that
  forces `INSTRUMENT_DECLINED` and other issues on the create, get, capture
  and refund calls without a browser ([negative testing](https://developer.paypal.com/tools/sandbox/negative-testing/request-headers/)).
  Approval itself is a browser login on `sandbox.paypal.com`; there is no
  documented headless way, exactly as on Orange. On a dated verification
  page, with § 10 answered or left open by name.

### 9. What the operator and the merchant take on

| Obligation                                            | Model A (merchant's app)           | Model B (partner)                             |
| ----------------------------------------------------- | ---------------------------------- | --------------------------------------------- |
| A PayPal business account that can **receive**        | the merchant                       | each merchant; the partner's own account too  |
| The REST app, its secret and rotation                 | the merchant                       | the partner                                   |
| PayPal's partner approval and each merchant's consent | —                                  | the partner                                   |
| Registering the webhook URL                           | the merchant                       | the partner, once                             |
| PayPal's fees, disputes, holds                        | the merchant                       | the merchant                                  |
| `platform_fees`                                       | impossible                         | **not sent in v1** — RFC-0010 open question 5 |
| Guest-checkout eligibility                            | the merchant's account settings    | the merchant's                                |
| PCI                                                   | out of scope: no card reaches vpay | out of scope                                  |

### 10. To confirm with PayPal, in sandbox, before the first live key

1. What `GET /v2/checkout/orders/{id}` returns for an order nobody approved
   after three hours, and whether its `payer-action` link still works — the
   local timeout in § 5 assumes "nothing changes".
2. Whether `CHECKOUT.ORDER.APPROVED` fires for a wallet approval on a
   redirect flow; no sample exists in the docs.
3. What `seller_payable_breakdown.paypal_fee` on a refund means (§ 3.3).
4. Whether `INSTRUMENT_DECLINED` leaves the order re-approvable under the
   same id. vpay will not reuse it either way; it decides only what the
   second `GET` shows.
5. Whether PayPal scopes `PayPal-Request-Id` per endpoint. The derived ids
   make it moot; it is worth knowing.

## Delivery

| Step | Contents                                                                                                                                                                                                                                                                                                        | Depends on                 |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| 1    | RFC-0010 steps 1–3 (the shared core changes and suite changes)                                                                                                                                                                                                                                                  | RFC-0010                   |
| 2    | `sandbox` in `STUB_MARKERS`; the return page pulls the poll forward                                                                                                                                                                                                                                             | nothing                    |
| 3    | `vpay-adapter-paypal`: `token.rs`, `wire.rs`, `mapping.rs` with the snapshot, `lib.rs`; unit tests                                                                                                                                                                                                              | 1                          |
| 4    | Mappings and the conformance rows                                                                                                                                                                                                                                                                               | 3                          |
| 5    | Wiring: `adapters()`/`adapter_codes()`, `REQUIRED_RAIL_KEYS`, `compose.yml` (`wiremock-paypal`), `.env.example`, Helm, `config/application.yml`, `docs/api/`, both SDKs' value lists, `sdks/parity.md`, `examples/shop`'s crate map, `docs/flows/adapter-paypal.md`, `docs/flows/failures.md`, the status pages | 4                          |
| 6    | The first sandbox call, with the negative-testing header, on a dated verification page                                                                                                                                                                                                                          | 5; a sandbox app           |
| 7    | Model B: the assertion, the BN code, `payee.merchant_id`                                                                                                                                                                                                                                                        | RFC-0009; partner approval |
| 8    | `fee` promoted to `Some` with `Money::from_provider_string`, once § 10 item 3 is answered                                                                                                                                                                                                                       | 6                          |

Steps 2–6 are one PR against `master` and one against `vpay-skills`.

## Alternatives considered

- **A `complete` method on the port, and `ChargeStatus::Approved`.** The
  worker would read "approved", then call the new method. It is the
  explicit shape, and it is a port change plus a worker branch for one
  rail, when the status read already runs on exactly the schedule a capture
  needs. Open question 1.
- **Capture from the return page**, when the payer comes back. It would be
  the fastest path and it would put a money-moving rail call on a browser
  request vpay cannot retry, behind a page a payer can close. The return
  page pulls the poll forward instead, and the worker captures.
- **`processing_instruction=ORDER_COMPLETE_ON_PAYMENT_APPROVAL`.** Not
  available for the PayPal wallet on PayPal's own list. It becomes the right
  design the day PayPal adds the wallet to it, and the adapter would then
  lose the capture call and nothing else.
- **`invoice_id` as the reference**, so a payer's receipt names it. It is
  unique per merchant account and shown to the payer, and a vpay reference
  is neither a merchant's invoice number nor something a payer should read.
  `custom_id` carries the reference where reports and webhooks need it.
- **Declare `card` as a method type**, since guest checkout exists. Refused
  in § 4: the rail cannot see whether the payer will be offered it.
- **Treat an unapproved order as `Pending` forever**, as the port's
  `UNRESOLVED_POLL_INTERVAL` would. An abandoned PayPal order would then
  never fail, and a merchant's intent would sit in `requires_action`
  indefinitely. The local deadline is bounded by PayPal's own figures and
  is stated as the decision it is.

## Open questions

1. **Capture inside `query_status`** (this RFC), or a `complete` method and
   an `Approved` status on the port? (Maintainer.)
2. **The local timeout**: three hours by default, and may a merchant whose
   PayPal account was extended set more per intent? (Maintainer.)
3. **`IMMEDIATE_PAYMENT_REQUIRED` by default**, which refuses eCheck and
   other pending funding, or `UNRESTRICTED` with captures that can stay
   `PENDING` for days? (Maintainer, product.)
4. **`landing_page` default** `NO_PREFERENCE`, and whether a deployment may
   set `GUEST_CHECKOUT` without this rail ever declaring `card`.
   (Maintainer, with RFC-0008.)
5. **`platform_fees` on model B** — RFC-0010 open question 5. (Maintainer,
   counsel.)
6. **Signed callbacks** — RFC-0010 open question 4, with the note that
   PayPal's verification is an API call needing a `webhook_id` in
   configuration. (Maintainer; a port change.)
7. **`fee` on refunds**: promote after § 10 item 3, and what `Some(0)` should
   mean to a merchant if PayPal always answers zero. (Maintainer, with issue
   #46's integrator.)
8. **vpay's `Currency` enum** holds XAF and EUR. A PayPal deployment in USD,
   GBP or any of the other 22 needs the enum, `currencies`, `money.md` and
   `formatAmount` to grow — separate work this RFC depends on for any
   merchant not in the euro area. (Maintainer.)
9. **`brand_name` and a per-intent description** — RFC-0010 open
   question 6. (Maintainer.)

## Impact on existing invariants

- **Rails behind the port** holds: one crate, one `providers[]` entry, the
  two `Origin` changes RFC-0010 already makes, and two small core changes
  that name no rail.
- **"The authenticated status query is the only thing that moves money"**
  holds, and on this rail it is literal: `query_status` captures. The
  sentence was written about vpay's state and turns out to describe the
  call, which is the argument of § 3.2.
- **Callbacks are hints** holds, and gains a second reader: the return page.
- **Crash safety** holds: the order id is durable before the redirect, and a
  capture whose result was lost is found on the next read.
- **One charge per intent** holds: `INSTRUMENT_DECLINED` is a decline and a
  restart is a new intent, never a re-redirect.
- **No mocks in shipping processes** holds.
- **Pass-through** holds on models A and B, and only there (RFC-0009 § 1).
- **Integer minor units** holds: `value` is `to_provider_string()`, and the
  three PayPal currencies that "do not support decimals" render as integers
  under the same exponent rule.
- **`verify-status`:** no `NotImplemented` token is planned.
