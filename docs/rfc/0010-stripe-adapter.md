# RFC-0010: A Stripe adapter — hosted Checkout behind the port

- **Status:** Draft — Delivery step 1 (the two core changes) landed
  2026-10-17; steps 2–7 are unbuilt
- **Author:** vpay maintainers
- **Date:** 2026-10-17
- **Related:** [RFC-0004](0004-billing-on-top-of-invoices.md) § 7 (one
  adapter per provider, the four hard rules, stored cards),
  [RFC-0008](0008-payment-method-routing.md) (method types),
  [RFC-0009](0009-provider-accounts-per-merchant.md) (provider accounts per
  merchant), [RFC-0003](0003-refunds-destinations-and-the-first-ledger-postings.md)
  § 1 (refund destinations), [ADR-0002](../adr/0002-provider-port.md),
  [ADR-0006](../adr/0006-no-mocks-in-main-processes.md),
  [ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md),
  [ADR-0021](../adr/0021-flutter-checkout-plugin.md) (no native card field),
  [flows/adapter-orange-money.md](../flows/adapter-orange-money.md) (the
  redirect rail this one is modelled on),
  [flows/stripe-sdk-compat.md](../flows/stripe-sdk-compat.md).

Nothing in this document is built. It is the design of one adapter crate,
`vpay-adapter-stripe`, rail code `stripe`, and of the two things the core must
change for any rail that refunds to the instrument that paid. Every Stripe fact
below was read on docs.stripe.com on 2026-10-17 and is cited; nothing has been
called, in sandbox or anywhere else.

## Problem

vpay carries two mobile-money rails and no card rail. RFC-0004 § 7 admits any
provider that meets four hard rules, and Stripe's hosted Checkout meets all
four on its public documentation:

| Hard rule (RFC-0004 § 7)               | How Stripe Checkout meets it                                                                                                                                                                                                                                                    |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| vpay never holds the money             | A direct charge "appears in the connected account's balance, not in your platform's balance" ([Connect charges](https://docs.stripe.com/connect/charges)); on the merchant's own key there is no platform at all                                                                |
| `vpay-server` never sees a card number | The card is typed on `checkout.stripe.com`. For vpay this is the `Redirect` flow Orange already takes                                                                                                                                                                           |
| A secret-authenticated status query    | `GET /v1/checkout/sessions/{id}` with the secret or restricted key ([Checkout Session](https://docs.stripe.com/api/checkout/sessions/object))                                                                                                                                   |
| A stub-able contract                   | [stripe/openapi](https://github.com/stripe/openapi) publishes the OpenAPI 3.0 spec and fixtures for every version, so WireMock mappings are transcribed, not reconstructed. Orange's were reconstructed from community SDKs; this is the first rail with a vendor specification |

Two facts from the 2026-10-16 research bound what this adapter is for:

- **Stripe lists no CEMAC country** among the 51 it opens accounts in
  ([stripe.com/global](https://stripe.com/global)), so a Cameroon-registered
  merchant cannot hold one. The adapter serves merchants in Stripe's countries,
  and Cameroonian owners who hold a company there.
- **Only two account models keep pass-through.** The merchant's own restricted
  key, and Connect direct charges. Destination charges, separate charges and
  transfers, cross-border payouts and Global Payouts all route money through
  the operator's balance and are out.

The adapter is also the first rail whose refund returns to the instrument that
paid — [`RefundDestination::Origin`](../../backends/crates/vpay-provider/src/lib.rs) —
and the first the core has to hand a rail transaction id to on refund. Both
arms of the port exist and neither has been driven end to end. A card rail is
what they were written for, and building one is what proves them.

## Proposal

### 1. Shape

| Property                         | Value                                                                                                                             |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Rail code                        | `stripe`                                                                                                                          |
| Crate                            | `backends/crates/vpay-adapter-stripe/`, the `lib.rs` / `wire.rs` / `mapping.rs` split of the two existing crates; no `token.rs`   |
| `flow`                           | `Redirect`                                                                                                                        |
| `supports_refunds`               | `true`                                                                                                                            |
| `supports_partial_refunds`       | `true` — `amount` on `POST /v1/refunds` ([Refunds](https://docs.stripe.com/api/refunds/create))                                   |
| `delivers_callbacks`             | `true` — webhooks, registered out of band (§ 6)                                                                                   |
| `requires_ip_allowlist`          | `false`                                                                                                                           |
| `supports_account_holder_lookup` | `false`                                                                                                                           |
| `refund_destination`             | **`Origin`** — "refunds go only to the original payment method" ([Refunds](https://docs.stripe.com/refunds)); the first such rail |
| `payer_fields`                   | the default `&[]` — a redirect rail collects nothing before the redirect                                                          |
| `parse_destination`              | the port's default `Unsupported`, which is the permanent, correct answer on an `Origin` rail                                      |
| Method type (RFC-0008)           | `card`. Today a merchant sends `payment_method_types[]=stripe`                                                                    |

No `token.rs`: Stripe authenticates every request with a static bearer key, so
there is nothing to mint, cache or fingerprint. The port's `token` module
stays unused by this crate.

**The name.** vpay's own `/v1` is Stripe-shaped, so a merchant on stripe-node
against vpay will write `payment_method_types: ['stripe']`, meaning "a card,
through Stripe". That reads oddly and it is correct under today's rule that
`payment_method_types` holds rail codes. RFC-0008 turns it into `card`.

### 2. Configuration, on RFC-0009's two levels

```yaml
# Illustrative. The account model decides which level each key sits on.
providers:
  - code: stripe
    host:
      url: https://api.stripe.com # a WireMock host in compose
      label: stripe-live
    display_name: { en: Card, fr: Carte bancaire }
    currency: EUR
    settings:
      session_lifetime_seconds: 1800 # 1800..86400, Stripe's own bounds
      line_item_name: Payment
      lang: fr
    credentials: {} # rail-level; see the two models below
```

| Model                               | Rail-level (`providers[]`)                    | Account-level (`provider_accounts[]`, RFC-0009)     | Who the operator is to Stripe                                                                                                                                                                                 |
| ----------------------------------- | --------------------------------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A. The merchant's own key**       | nothing                                       | `credentials.secret_key` = the merchant's `rk_…`    | Nobody. Stripe's documented route for "your users run your software on-premise" is a Stripe App that issues the merchant a restricted key ([RAK](https://docs.stripe.com/stripe-apps/api-authentication/rak)) |
| **B. Connect, direct charges only** | `credentials.secret_key` = the platform's key | `settings.stripe_account` = the merchant's `acct_…` | A Connect platform, in one of Stripe's platform countries, sending `Stripe-Account` on every call                                                                                                             |

**Until RFC-0009 lands, there is one level**, so a deployment serves one
merchant on model A, or one connected account on model B. That is the same
limit MTN and Orange have today, and it is stated rather than worked around.

Boot refusals, in `vpay_config::config::REQUIRED_RAIL_KEYS` per ADR-0012:

- `secret_key` is required, must start with `rk_live_`, `rk_test_`, `sk_live_`
  or `sk_test_` ([keys](https://docs.stripe.com/keys)), and its
  `live`/`test` half must agree with `deployment.livemode` — the
  `PublishableKeyLivemodeMismatch` rule, applied to the other side of the wire.
- `stripe_account`, when present, must start with `acct_`.
- `session_lifetime_seconds` must be within 1800 and 86400, because Stripe
  refuses an `expires_at` outside 30 minutes to 24 hours after creation
  ([create](https://docs.stripe.com/api/checkout/sessions/create)).

A restricted key needs **Checkout Sessions: Write**, **Payment Intents: Read**
and **Charges and Refunds: Write**. The permission names are from the Stripe
Apps reference ([permissions](https://docs.stripe.com/stripe-apps/reference/permissions));
whether the Dashboard's restricted-key form uses the same labels is on the
"To confirm" list.

### 3. The wire — three calls, one pinned version

Every request carries `Authorization: Bearer <secret_key>`, a **pinned**
`Stripe-Version: 2026-09-30.endive` ([versioning](https://docs.stripe.com/api/versioning)),
and `Stripe-Account: <acct_…>` on model B. The version is a constant in the
crate, not configuration: the request and response shapes the stub models are
the pinned version's, and bumping it is a code change that moves the stub in
the same commit. Bodies are form-encoded, as Stripe's are. `vpay_api::form`
already spells vpay's own form encoding; the adapter needs the encoder's
direction, not the decoder's, and reuses nothing from the checkout path.

Two facts of the pinned version decide the request shape, and a stub copied
from an older guide gets both wrong:

- `payment_method_types` on a Checkout Session is **gone** since
  `2026-09-30.endive`; passing it is a 400. The replacement is
  `allowed_payment_method_types` ([changelog](https://docs.stripe.com/changelog/endive/2026-09-30/remove-payment-method-types-checkout-sessions)).
- `ui_mode` is `hosted_page` since `2026-03-25.dahlia`; `hosted` fails
  ([changelog](https://docs.stripe.com/changelog/dahlia/2026-03-25/updates-available-checkout-session-ui-modes)).

#### 3.1 `submit` — `POST {base_url}/v1/checkout/sessions`

```http
POST /v1/checkout/sessions
Idempotency-Key: <ChargeRef::reference_id>
Stripe-Version: 2026-09-30.endive

mode=payment
ui_mode=hosted_page
line_items[0][price_data][currency]=xaf
line_items[0][price_data][unit_amount]=5000
line_items[0][price_data][product_data][name]=Payment
line_items[0][quantity]=1
client_reference_id=<reference_id>
metadata[vpay_charge]=<reference_id>
payment_intent_data[metadata][vpay_charge]=<reference_id>
success_url=<ChargeRef::return_url>
cancel_url=<ChargeRef::return_url>
expires_at=<now + session_lifetime_seconds>
locale=fr
allowed_payment_method_types[0]=card
→ 200 { "id": "cs_…", "url": "https://checkout.stripe.com/c/pay/cs_…",
        "status": "open", "payment_status": "unpaid",
        "payment_intent": null, "expires_at": … }
```

- **The reference is the idempotency key.** Stripe keeps a key for at least
  24 hours and replays the original response for a repeat with the same
  parameters ([idempotent requests](https://docs.stripe.com/api/idempotent_requests)),
  so a duplicate submit — the crash-recovery case — gets the **same**
  `cs_…`, which is what `duplicate_submit_reports_submitted_not_an_error`
  demands of a redirect rail. A 409 means the first request is still running
  and is a transport error retried with the same key. An
  `idempotency_error` means the parameters changed between attempts, which a
  charge's cannot; it is `ProviderError::Config`, not a retry.
- **`unit_amount` is `Money::to_provider_minor()`**, in the charge's own
  currency, lowercased. XAF and XOF are zero-decimal on Stripe's list exactly
  as in [money.md](../flows/money.md); EUR is two-decimal with a 0.50 EUR
  minimum ([currencies](https://docs.stripe.com/currencies)). An amount under
  the minimum is a 400 on this call and surfaces as a decline (§ 3.4), not a
  boot refusal, until RFC-0008's `amount_limits` exists.
- **`success_url` and `cancel_url` carry the same value, verbatim**, for the
  reason Orange's do: the outcome comes from the authenticated status read,
  never from which URL the payer came back on. The core's `return_url` is sent
  as given — the conformance suite asserts equality — so the
  `{CHECKOUT_SESSION_ID}` template is **not** appended; vpay's return page
  knows its own session. A charge with no `return_url` is
  `ProviderError::Config` before the call.
- **`client_reference_id` and both `metadata` keys carry the reference**
  (36 characters, under the 200 and 500 limits). The first is what webhooks
  are parsed by (§ 6); the metadata is for a human in the Stripe Dashboard and
  for the PaymentIntent, which otherwise names no vpay object.
- **`allowed_payment_method_types[0]=card`.** The rail code `stripe` promises a
  card today, and the Dashboard's dynamic payment methods would otherwise put
  Link, wallets or local methods on the page. It is open question 1.
- **`expires_at`** bounds the `Pending` window (§ 5). The default 1800 s is
  Stripe's minimum.
- **`line_item_name`** is a deployment setting because `ChargeRef` carries no
  description (open question 6).

Out of the response the adapter keeps `id` and `url`:
`Submitted { redirect_url: url, ref_extra: { session_id: cs_…, expires_at } }`.
A 200 with a null `url` or no `id` is `ProviderError::Malformed`. The session
id persisted in `charges.provider_ref_extra` before the redirect is what the
redirect-flow precondition in
[provider-port.md](../flows/provider-port.md) § "Preconditions, per flow shape"
requires: "status is queryable by material you hold after that persist".

#### 3.2 `query_status` — `GET {base_url}/v1/checkout/sessions/{session_id}?expand[]=payment_intent`

The outcome is a pure function of `(status, body)`, so `mapping.rs` is
provable without a network:

| HTTP      | `status`   | `payment_status`      | `payment_intent.last_payment_error` | → `ChargeStatus`                                                                                                                               |
| --------- | ---------- | --------------------- | ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| 200       | `complete` | `paid`                | —                                   | `Succeeded { provider_txn_id: payment_intent.id }`                                                                                             |
| 200       | `complete` | `unpaid`              | —                                   | `Pending` — a delayed-notification method; unreachable with cards only, mapped rather than panicked                                            |
| 200       | `complete` | `no_payment_required` | —                                   | `Failed(provider_error)` — a state this adapter never asked for; no money moved                                                                |
| 200       | `open`     | any                   | any                                 | `Pending` — **a decline does not end the session**; Checkout lets the payer try again ([card declines](https://docs.stripe.com/declines/card)) |
| 200       | `expired`  | `unpaid`              | present                             | `Failed(map(decline_code, code))` — the last thing the issuer said                                                                             |
| 200       | `expired`  | `unpaid`              | absent                              | `Failed(payer_timeout)` — the payer never finished                                                                                             |
| 404       |            |                       |                                     | `NotFound`                                                                                                                                     |
| 401, 403  |            |                       |                                     | `Err(Rejected(provider_account_blocked))`                                                                                                      |
| 429, 5xx  |            |                       |                                     | `Err(Transport)` — `Retry::AfterBackoff`; a 429 carries `Stripe-Rate-Limited-Reason` into the context                                          |
| 3xx       |            |                       |                                     | `Err(Malformed)` — redirects are never followed                                                                                                |
| other 4xx |            |                       |                                     | `Err(Config)`                                                                                                                                  |

The enum values are Stripe's: `status` is `open`, `complete` or `expired`,
`payment_status` is `paid`, `unpaid` or `no_payment_required`
([object](https://docs.stripe.com/api/checkout/sessions/object)). The
PaymentIntent is created only when the payer confirms, so `payment_intent` is
null on a fresh session ([changelog](https://docs.stripe.com/changelog/2022-08-01/deferred-paymentintent-checkout-session))
and the `expand` costs nothing until then.

**A charge whose `ref_extra` has no `session_id` is `ProviderError::Config`,
never `NotFound`** — Orange's `pay_token` rule, for Orange's reason: `NotFound`
means "the rail never saw this", and a session id vpay lost is the opposite
case; a payer may already have paid.

**The expired-after-decline row is a decision** (open question 2). Orange has
to collapse "said no" and "walked away" into `EXPIRED`; Stripe tells them
apart, and a merchant reading `last_payment_error` on the intent is better
served by `insufficient_funds` than by `payer_timeout`. Whether
`payment_intent` is still populated on an expired session after a failed
attempt is unverified and on the "To confirm" list; the absent arm is the one
that holds if it is not.

#### 3.3 `refund` — `POST {base_url}/v1/refunds`

```http
POST /v1/refunds
Idempotency-Key: <the refund's own reference>

payment_intent=<the charge's provider_txn_id>
amount=<Money::to_provider_minor()>
metadata[vpay_refund]=<refund reference>
metadata[vpay_charge]=<charge reference>
→ 200 { "id": "re_…", "status": "succeeded" | "pending", … }
```

- `destination` is `None` by construction on an `Origin` rail; the core has
  already refused a nominated payee with a 400 (`vpay_api::v1::refunds`).
- `status` `succeeded` or `pending` → `Ok(Refunded { ref_extra: { refund_id,
status }, fee: None })`. **`fee` is `None` because the Refund object has no
  fee field** — confirmed against the spec — and Stripe does not return its
  processing fee on a refund ([refunds](https://docs.stripe.com/refunds)).
  `None` is "the rail did not report a fee", exactly the distinction
  [status.md](../status.md) draws for MTN.
- `status` `failed` → `Err(Rejected(map(failure_reason)))`: `insufficient_funds`
  → `insufficient_funds`; `lost_or_stolen_card`, `expired_or_canceled_card` →
  `payer_account_blocked`; `declined` → `payer_declined`;
  `charge_for_pending_refund_disputed`, `merchant_request`, `unknown` →
  `provider_error` ([object](https://docs.stripe.com/api/refunds/object)).
- A duplicate is the idempotency key replaying the same `re_…`, which is what
  `a_duplicate_refund_reference_is_accepted_and_never_paid_twice` demands. A
  400 `charge_already_refunded` is a decline (`provider_error`), never an
  accepted refund.
- **`Ok` is an instruction, not a settlement, and this rail exposes the gap
  harder than MTN does.** Stripe usually answers `succeeded` synchronously,
  and the port has no way to say so: `vpay_db::Refunds` leaves the row
  `pending`, and nothing settles a pending refund (RFC-0003 open question 8).
  The `status` in `ref_extra` is the evidence a future refund status read
  would start from.

#### 3.4 Declines → `FailureCode`

`mapping.rs` holds the table below, case-insensitive, `decline_code` first
and `code` second, an unmapped string falling through to `provider_error`
**carrying the raw string**. Stripe publishes the full list at
[declines/codes](https://docs.stripe.com/declines/codes); the rows marked ✔
were read there on 2026-10-17, and the whole published list is snapshotted as
a `#[cfg(test)]` const when the file is written, so every code is either
mapped or deliberately unmapped (the shape issue #59 established for MTN).

| `FailureCode`              | Stripe `decline_code`                                                                                                                                                                                                                                                                                                                                                                |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `insufficient_funds`       | `insufficient_funds` ✔                                                                                                                                                                                                                                                                                                                                                               |
| `payer_limit_reached`      | `card_velocity_exceeded` ✔, `withdrawal_count_limit_exceeded`                                                                                                                                                                                                                                                                                                                        |
| `payer_account_blocked`    | `lost_card` ✔, `stolen_card` ✔, `pickup_card`, `restricted_card`                                                                                                                                                                                                                                                                                                                     |
| `invalid_payer`            | `expired_card` ✔, `incorrect_cvc` ✔, `invalid_cvc`, `incorrect_number`, `invalid_number`, `invalid_expiry_month`, `invalid_expiry_year`, `incorrect_zip`, `card_not_supported`, `currency_not_supported`                                                                                                                                                                             |
| `payer_declined`           | `generic_decline` ✔, `do_not_honor` ✔, `fraudulent` ✔, `authentication_required` ✔, `call_issuer`, `transaction_not_allowed`, `not_permitted`, `security_violation`, `merchant_blacklist`, `revocation_of_authorization`, `revocation_of_all_authorizations`, `stop_payment_order`, `invalid_account`, `new_account_information_available`, `no_action_taken`, `service_not_allowed` |
| `provider_unavailable`     | `processing_error` ✔, `try_again_later`, `reenter_transaction`, `issuer_not_available`                                                                                                                                                                                                                                                                                               |
| `payer_timeout`            | an expired session with no last error (§ 3.2)                                                                                                                                                                                                                                                                                                                                        |
| `provider_account_blocked` | HTTP 401 and 403 — a revoked key, or a restricted key without the permission ([restricted keys](https://docs.stripe.com/keys/restricted-api-keys))                                                                                                                                                                                                                                   |
| `provider_error`           | anything unmapped; `no_payment_required`; a submit the rail refuses (an amount under the minimum, a currency the account cannot present)                                                                                                                                                                                                                                             |

So `PRODUCED_FAILURE_CODES` is **nine of the eleven**: everything but
`invalid_payee` and `payee_account_blocked`, which are about a payee, and a
card refund has none. [failures.md](../flows/failures.md) gains a column.

Stripe says `fraudulent`, `lost_card` and `stolen_card` should be shown to the
payer as a generic decline. vpay's `FailureCode` is the merchant's vocabulary,
not the payer's, and `payer_actionable()` is already what the checkout copy
reads; nothing here puts a raw decline code in front of a payer.

### 4. What the payer sees

The existing checkout. `build_rails` renders `stripe` as one more `Redirect`
rail with the deployment's `display_name` ("Card"), the "Redirecting to the
rail" screen sends the payer to `checkout.stripe.com`, and vpay's return page
reads the intent's real status before forwarding to the merchant's
`success_url` ([hosted-checkout.md](../flows/hosted-checkout.md)). No checkout
change. ADR-0021's refusal of a native card field is untouched: the mobile
SDKs open Stripe's page the way they open Orange's.

For the merchant, a redirect to Stripe's page keeps them on PCI SAQ A; the
PCI SSC's FAQ 1588 says the 2025 script-attack criterion applies to embedded
forms, not redirects ([FAQ 1588](https://www.pcisecuritystandards.org/faqs/1588/)).
`vpay-server` holds a key and a session id and never a card number.

### 5. The `Pending` window and the poll ladder

A session is `open` for `session_lifetime_seconds`, and the worker's ladder
(`vpay_worker::poll_delay`) asks at 10, 20, 30, 45, 60 and 90 s, then every
120 s to the twentieth attempt, then every 15 min, then hourly once unresolved.
With the 1800 s default, a charge the payer abandons is `Failed(payer_timeout)`
within one rung of expiry; with Stripe's 24-hour maximum it is `Pending` for a
day, which is why the default is the minimum and the maximum is a deployment's
decision. Stripe's `expires_at` is read back on every status call and kept in
`ref_extra`, so an operator can see the window a charge is in.

`Expire` (`POST /v1/checkout/sessions/{id}/expire`) is **not called**. The port
has no cancel-at-rail operation, and an intent can be cancelled only before it
is confirmed, when no session exists. It is listed so nobody adds it to
`cancel` as a courtesy: a courtesy call in the money path is a call with no
test behind it.

### 6. Webhooks are callbacks, and callbacks are hints

`parse_callback(body)` reads a Stripe event:

```json
{ "type": "checkout.session.completed",
  "data": { "object": { "id": "cs_…", "client_reference_id": "<reference>" } } }
```

→ `CallbackRef { reference_id, ref_extra: { session_id, event_type } }` for the
four `checkout.session.*` types (`completed`, `async_payment_succeeded`,
`async_payment_failed`, `expired` — [event types](https://docs.stripe.com/api/events/types));
any other `type`, a missing or non-UUID `client_reference_id`, or `{}` is
`ProviderError::Malformed`. The handler then does what it does for every
rail: pulls the charge's `poll_charge` job forward and discards `ref_extra`
([provider_callback.rs](../../backends/crates/vpay-api/src/provider_callback.rs)).
The authenticated status read is the only thing that moves money.

Two things about this rail's callbacks are different, and neither changes the
design:

- **The URL is registered with Stripe, not sent per request.** MTN takes
  `X-Callback-Url` and Orange `notif_url` on every submit; a Checkout Session
  carries no such field. The operator (model A: the merchant) registers
  `{public_base_url}/provider/stripe/callback` as a webhook endpoint for the
  four event types; on model B the platform registers one Connect-scoped
  endpoint, and events arrive with a top-level `account`
  ([Connect webhooks](https://docs.stripe.com/connect/webhooks)). The
  conformance suite's `the_submit_tells_the_rail_where_to_call_back` cannot
  hold on this rail (§ 8).
- **Stripe signs its webhooks, and vpay cannot check the signature today.**
  `Stripe-Signature: t=…,v1=…` is HMAC-SHA256 over `"{t}.{body}"`
  ([webhooks](https://docs.stripe.com/webhooks)) — byte-identical to vpay's
  own `Vpay-Signature`, which `vpay_worker::signing` already computes. But
  `parse_callback` receives the body and no headers, so the check is a port
  change (open question 4). Until then an unverified Stripe webhook is worth
  exactly what an unverified Orange notification is: one authenticated status
  read, and no money.

### 7. The two core changes, and why they are the port working

Adding a rail is meant to change nothing in the core. This one changes two
things, and both are the port's `Origin` arm being used for the first time:

1. **The refund call must name the payment.** `refund_at_rail` in
   `vpay_api::v1::refunds` builds the adapter's `ChargeRef` with an **empty**
   `ref_extra`, "because a refund carries no key material from a submit".
   True of a transfer to a payee; false of a refund to the instrument that
   paid, which Stripe takes by `payment_intent`. The core puts the charge's
   `provider_txn_id` into `ref_extra` under `provider_txn_id` —
   `vpay_api::v1::refunds::REFUND_PROVIDER_TXN_ID_KEY`, spelled in one place
   so the adapter and the handler cannot disagree — or nothing when the
   charge recorded none; `Required` rails ignore it; a `Required`-shaped
   adapter that reads it is a bug. A charge with none is not refundable on an
   `Origin` rail, and that refusal is **the adapter's**, as
   `ProviderError::Config`: the port's settled answer for a broken invariant,
   which stops the poll ladder, pages, and never reaches a payer as a
   decline. **Landed 2026-10-17.** _(This said "that is the handler's refusal
   before any rail call" until it landed. The handler cannot refuse on the
   capability without learning what every `Origin` rail needs, and
   `charges.provider_txn_id` is nullable on purpose — a `Required` rail may
   settle a charge without naming a transaction — so the core carries what
   it has and the rail that needs it is the one that says so. The port's
   `refund` docs carry the rule.)_
2. **The assertion that no shipping rail is `Origin` is relaxed, by paying
   what it names.** `no_shipping_rail_returns_a_refund_to_the_paying_instrument`
   in the conformance suite lists three debts: an integration case sending
   `destination` to an `Origin` rail through `POST /v1/refunds` and asserting
   the 400; a case that a refund **without** a destination succeeds on it;
   and RFC-0003 § 1 amended, since it says every rail vpay carries needs a
   payee. All three land in the same change as the capability value.
   **Landed 2026-10-17, ahead of the capability value:** the two cases are
   `an_origin_rail_refuses_a_destination_through_the_route` and
   `a_refund_without_a_destination_succeeds_on_an_origin_rail_and_names_the_payment`
   in `backends/tests/integration/tests/refunds.rs`, driven on an `Origin`
   rail that exists only in that test binary (ADR-0006 permits a double
   there; `cargo xtask verify-no-mocks` is what proves it reaches no shipping
   process); the conformance case became
   `every_shipping_rails_refund_destination_agrees_with_its_parser`, a rule
   (`Required` overrides `parse_destination`, `Origin` does not) rather than
   a census; RFC-0003 § 1 carries the dated amendment. Step 3's suite changes
   (§ 8) are the part that still waits on a shipping `Origin` rail.

Nothing else moves: not the lifecycle, not the reconciler, not the checkout,
not the settlement transaction. If step 9 of the implementation turns out to
be "and also patch the reconciler", the port leaked and that is the finding.

### 8. Proving it

- **Unit tests in the crate**, no container: the mapping table row by row,
  the decline snapshot, the form body a submit produces, the three
  `Malformed` shapes of a callback, the livemode prefix rule, and a `Debug`
  canary on every type that holds the key.
- **WireMock mappings** under `backends/tests/conformance/wiremock/stripe/mappings/`,
  transcribed from the pinned version's spec: `checkout-sessions.json`
  (create, keyed on `Idempotency-Key` as MTN's are on `X-Reference-Id`, the
  session id templated from it; retrieve, keyed on the id's path),
  `refunds.json`, `stub-hosted-page.json` and `demo-outcomes.json` on
  Orange's model, with the same `REF_*` references the suite steers by. The
  stub's hosted page is a stub of **Stripe's** page, two links and a form,
  never of a vpay page.
- **The shared conformance suite, unchanged in its cases**, plus one
  `RailUnderTest::Stripe` row in each table function. Three of its
  assumptions do not hold on this rail and need a suite change that is
  named here so it is not mistaken for a leak:
  - `callback_url_pattern` becomes `Option`, `None` meaning "registered out
    of band", and the case then asserts the callback URL appears **nowhere**
    in the stub's journal — the stronger statement `return_url` already makes
    on a push rail.
  - The refund cases that address a payee (`the_refund_is_addressed_to_the_payee_the_merchant_nominated`,
    `a_refund_to_a_payee_the_rail_rejects_…`) gate on
    `refund_destination == Required` and, on `Origin`, assert
    `parse_destination` answers `Unsupported` and that the refund body names
    the payment (`refund_body_pattern` on `payment_intent`).
  - The hosted-page walk (`a_payer_lands_on_the_rails_page` and the cases on
    it) is parameterised over every `Redirect` rail rather than built for
    Orange alone.
- **The first real call**, against Stripe's sandbox with a test-mode key, on
  the day the crate lands and before any claim is made about Stripe rather
  than about the stub — the 2026-09-15 MTN precedent. It goes on a dated
  verification page, with the four "To confirm" items below answered or
  left open by name.

### 9. What the operator and the merchant take on

| Obligation                                                  | Model A (merchant's key)           | Model B (Connect)                           |
| ----------------------------------------------------------- | ---------------------------------- | ------------------------------------------- |
| A Stripe account in a Stripe country                        | the merchant                       | the platform **and** each connected account |
| The restricted key, its three permissions, and its rotation | the merchant, through a Stripe App | the platform                                |
| Registering the webhook endpoint                            | the merchant, in their Dashboard   | the platform, once, Connect-scoped          |
| Stripe's fees, disputes and chargebacks                     | the merchant                       | the connected account (direct charges)      |
| An `application_fee_amount`                                 | impossible                         | **not sent in v1** — open question 5        |
| PCI                                                         | SAQ A, redirect                    | SAQ A, redirect                             |

### 10. To confirm with Stripe, before the first live key

1. Whether hosted Checkout presents **XAF** at all. XAF is on the presentment
   list for every Stripe country, and no page says Checkout refuses it; no
   page says it accepts it either.
2. Whether `payment_intent` is still populated on an **expired** session
   after a failed attempt (§ 3.2's decline row).
3. Whether `allowed_payment_method_types[0]=card` on an account with cards
   disabled is a 400 or an empty page.
4. The Dashboard's labels for the three restricted-key permissions, and
   whether "Checkout Sessions: Read" alone can `expand[]=payment_intent`.

## Delivery

| Step | Contents                                                                                                                                                                                                                                                                                                        | Depends on                            |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| 1    | Core: `refund_at_rail` passes `provider_txn_id`; the `Origin` assertion relaxed with its three debts paid; RFC-0003 § 1 amended — **landed 2026-10-17** ([verification/2026-10-17-origin-refund-core.md](../status/verification/2026-10-17-origin-refund-core.md))                                              | nothing                               |
| 2    | `vpay-adapter-stripe`: `wire.rs`, `mapping.rs` with the snapshot, `lib.rs`; unit tests                                                                                                                                                                                                                          | 1                                     |
| 3    | Mappings, the conformance rows, the three suite changes in § 8                                                                                                                                                                                                                                                  | 2                                     |
| 4    | Wiring: `adapters()`/`adapter_codes()`, `REQUIRED_RAIL_KEYS`, `compose.yml` (`wiremock-stripe`), `.env.example`, Helm, `config/application.yml`, `docs/api/`, both SDKs' value lists, `sdks/parity.md`, `examples/shop`'s crate map, `docs/flows/adapter-stripe.md`, `docs/flows/failures.md`, the status pages | 3                                     |
| 5    | The first sandbox call, on a dated verification page                                                                                                                                                                                                                                                            | 4; a test-mode key                    |
| 6    | Model B: `Stripe-Account` on every call, the Connect webhook endpoint                                                                                                                                                                                                                                           | RFC-0009                              |
| 7    | Saved cards: `payment_intent_data[setup_future_usage]=off_session`, `customer_creation=always`, a `charge_stored` on the port                                                                                                                                                                                   | RFC-0004 § 7 stored methods, RFC-0009 |

Steps 1–5 were planned as one PR against `master` and one against
`vpay-skills`; step 1 landed on its own on 2026-10-17, because it is the part
no rail depends on and the part every `Origin` rail does, so steps 2–5 are
the pair that remains. The
`vpay-provider-adapters` skill says today that `supports_off_session`,
`charge_stored` and `method_types` do not exist, and after step 5 it must say
what this rail does and does not do in the same terms.

## Alternatives considered

- **Stripe's `card` payment method directly, with Stripe.js or Elements in
  vpay's checkout.** Rejected: the card is typed into vpay's page, which puts
  the checkout and every merchant embedding it into PCI scope (SAQ A-EP at
  best), and breaks RFC-0004 § 7's second rule. The hosted page is the whole
  reason this adapter needs no checkout change.
- **Dynamic payment methods, no `allowed_payment_method_types`.** Simpler and
  what Stripe recommends; but then `payment_method_types[]=stripe` promises
  whatever the merchant's Dashboard enables, and vpay cannot say what that is.
  Revisited under RFC-0008, where the rail declares `method_types`.
- **Collapse expired-after-decline to `payer_timeout`**, as Orange must.
  Cheaper to reason about, and it throws away the one piece of information a
  card rail has that a wallet rail does not. Open question 2.
- **Verify `Stripe-Signature` in the adapter now**, by widening
  `parse_callback` to take headers. Correct, and a change to every rail's
  signature for a check whose only effect is to refuse a job pull-forward.
  Deferred to open question 4, behind the hint design that already bounds the
  damage.
- **Destination charges with `on_behalf_of`**, so the platform takes its fee
  without an `application_fee_amount`. Rejected by RFC-0001: the money
  appears in the platform's balance.
- **One adapter per account model** (`stripe` and `stripe_connect`).
  Rejected: the wire is identical but for one header, and two codes would be
  two rows in every table for one rail.

## Open questions

1. **Pin `card`, or let the Dashboard decide?** (Maintainer, with RFC-0008.)
2. **Expired after a decline: the decline, or `payer_timeout`?** (Maintainer.)
3. **Default session lifetime** 1800 s, and whether a merchant may set it per
   intent. (Maintainer, product.)
4. **Signed callbacks.** Does `parse_callback` grow a headers argument so a
   rail that signs can be verified, with the hint design kept as the
   fallback? (Maintainer; a port change.)
5. **`application_fee_amount` on model B.** It is not holding merchant money,
   and it is the operator earning on a merchant's payment; RFC-0001's
   licence conversation decides whether a CEMAC operator may. (Maintainer,
   counsel.)
6. **A per-intent description.** Stripe shows `line_item_name` on the page
   and on the statement; `ChargeRef` carries no description and the intent's
   `description` is dropped today. (Maintainer.)
7. **`setup_future_usage` and `customer_creation`** are step 7's, and the
   `pm_…` object they need is RFC-0004 § 7's. Which lands first? (Maintainer.)
8. **A refund status read on the port** (RFC-0003 open question 8), now that a
   rail answers `succeeded` synchronously and vpay cannot record it.
   (Maintainer.)

## Impact on existing invariants

- **Rails behind the port** holds: one crate, one `providers[]` entry, two
  core changes that are the `Origin` arm's first use and nothing else.
- **Branch on capability values, never on a provider code** holds, and gains
  its first `Origin` rail. `refund_destination` has been inert on every
  shipping rail until now.
- **Callbacks are hints** holds and is, as on Orange, the only defence on an
  unverified callback (§ 6).
- **Crash safety** holds by construction: the session id is durable before
  the payer can act, because the payer cannot act before the redirect.
- **One charge per intent** holds. A payer retrying a declined card on
  Stripe's page is inside one session and one charge; the retry invariant is
  about intents, not about attempts on the rail's page.
- **No mocks in shipping processes** holds: the stub is a WireMock host in
  configuration, the Rust `wiremock` crate stays out of the conformance
  manifest.
- **Pass-through** holds on models A and B, and only there (RFC-0009 § 1).
- **Integer minor units** holds: `unit_amount` is `to_provider_minor()`, and
  Stripe's zero-decimal list agrees with vpay's about XAF.
- **`verify-status`:** no `NotImplemented` token is planned. Every method
  the adapter overrides is written in the same change, and the two it does
  not — `parse_destination`, `account_holder_name` — keep the port's
  `Unsupported`, which is the truth about the rail.
