# Adapter: Orange Money Cameroun

**Flow: redirect.** `supports_refunds: false`.

> **Sourcing caveat.** Orange's full technical specification sits behind Orange
> Partner and a signed merchant agreement. What follows is reconstructed from
> Orange Developer's public overview and from several independently-written
> community SDKs that agree with each other. Treat it as a strong prior to
> verify at onboarding, not as gospel.

## Preconditions

| Precondition | Orange |
|---|---|
| Submit response persistable before the payer can act | **Yes, by construction** — the payer's only route in is `payment_url` |
| Status queryable by material held after that persist | **Yes** — `order_id` + `amount` + `pay_token` |

Orange fails the *literal* push precondition ("queryable by a reference you
generated") and is still safe. That is exactly why preconditions are stated per
flow shape. See [crash-safety.md](crash-safety.md).

## The calls

```http
POST https://api.orange.com/oauth/v2/token
Authorization: Basic <base64(client_id:client_secret)>
grant_type=client_credentials
→ { "access_token": "…", "expires_in": … }
```

```http
POST https://api.orange.com/orange-money-webpay/{env}/v1/webpayment
{ "merchant_key": "…", "currency": "XAF", "order_id": "<reference>",
  "amount": 5000, "return_url": "…", "cancel_url": "…",
  "notif_url": "https://…/provider/orange_money/callback", "lang": "fr" }
→ { "pay_token": "…", "payment_url": "https://webpayment.orange-money.com/payment/pay_token/…",
    "notif_token": "…", "status": 201 }
```

```http
POST https://api.orange.com/orange-money-webpay/{env}/v1/transactionstatus
{ "order_id": "…", "amount": 5000, "pay_token": "…" }
→ { "status": "INITIATED|PENDING|EXPIRED|SUCCESS|FAILED", "order_id": "…", "txnid": "…" }
```

The payer obtains a one-time code by USSD and enters it on Orange's page.

**`{env}` is `dev` in sandbox** and country-specific in production — the
environment sits in the **URL path**, not only the host. So the configured
`base_url` must include the path prefix.

## Status mapping

| Orange | Core |
|---|---|
| `INITIATED`, `PENDING` | `Pending` |
| `SUCCESS` | `Succeeded` |
| `EXPIRED` | `Failed(payer_timeout)` |
| `FAILED` | `Failed(…)`; `provider_error` if unrecognised |

`INITIATED` deserves care: the token exists but the payer has not started. It is
`Pending`, not a failure — it is the state a charge sits in if the merchant
never redirects.

## Why refunds are off

No refund API is documented for Web Payment. `supports_refunds: false` means the
core refuses `POST /v1/refunds` on this rail with a Stripe-shaped 400, and **no
core code special-cases Orange to produce that**. If refunds turn out to be
available under a different agreement, it is a config flag and an adapter
method, not a core change.

## To confirm with Orange Cameroun

1. Exact `notif_url` payload, and whether it carries `pay_token` — this decides
   whether `parse_callback` can *repair* a charge whose `ref_extra` write failed.
2. Whether callbacks are verifiable beyond `notif_token`.
3. Production `{env}` path segment and host for Cameroon.
4. Whether `transactionstatus` stays queryable indefinitely, or ages out. If it
   ages out, the 24-hour escalation carries more weight here than on MTN.
5. Refund/disbursement availability.
6. Pass-through settlement, or forced aggregation (a regulatory question).
7. Transaction and daily limits for XAF.

## Status

Capabilities are declared and tested. **No wire call is implemented** — see
[../status.md](../status.md).
