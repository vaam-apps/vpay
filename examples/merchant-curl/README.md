# Raw HTTP

> Not runnable yet — `/v1/*` is not implemented. See ../../docs/STATUS.md.

## Create a PaymentIntent

Bodies are **form-encoded**, not JSON — that is what the Stripe SDKs send.

```bash
curl -X POST https://api.vpay.example/v1/payment_intents \
  -u sk_test_xxx: \
  -H "Idempotency-Key: order_1234_attempt_1" \
  -d amount=5000 \
  -d currency=xaf \
  -d "payment_method_types[]=mtn_momo" \
  -d "payment_method_types[]=orange_money" \
  -d "metadata[order_id]=1234"
```

`amount=5000` is **5,000 FCFA**. XAF is zero-decimal.

## Confirm — push rail (MTN)

```bash
curl -X POST https://api.vpay.example/v1/payment_intents/pi_xxx/confirm \
  -u sk_test_xxx: \
  -H "Idempotency-Key: order_1234_confirm_1" \
  -d "payment_method_data[type]=mtn_momo" \
  -d "payment_method_data[mtn_momo][msisdn]=237670000000"
```

→ `"status": "processing"`. The payer gets a prompt on their handset. Wait for
the webhook; **do not ship on `processing`**.

## Confirm — redirect rail (Orange)

```bash
curl -X POST https://api.vpay.example/v1/payment_intents/pi_xxx/confirm \
  -u sk_test_xxx: \
  -H "Idempotency-Key: order_1234_confirm_1" \
  -d "payment_method_data[type]=orange_money" \
  -d "return_url=https://shop.example/order/1234/return"
```

→ `"status": "requires_action"` with:

```json
{ "next_action": { "type": "redirect_to_url",
                   "redirect_to_url": { "url": "https://webpayment.orange-money.com/…" } } }
```

Send the payer's browser there.

## Retrying a failed payment

Create a **new** PaymentIntent. An intent gets exactly one charge for its entire
life — that is what makes double-charging structurally impossible.
