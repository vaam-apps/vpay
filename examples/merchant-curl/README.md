# Raw HTTP

> Not runnable yet. `/v1/*` is not implemented, and neither is the OAuth2
> token endpoint merchant authentication now depends on. See
> ../../docs/status.md and
> [ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md). The
> token endpoint's exact path below is illustrative — it has not been
> decided.

## Authenticate — `client_credentials` + `private_key_jwt`

`/v1` does not accept an API key ([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)).
A merchant signs a short-lived JWT assertion with the private half of the
keypair whose public JWK vpay has on file (registered via a YAML config PR —
see [ADR-0003](../../docs/adr/0003-yaml-configuration.md)), then exchanges
that assertion for a bearer access token:

```bash
# 1. Build a client assertion (RFC 7523). Pseudocode — no vpay-provided
#    helper library exists. `merchant_a` is the client_id vpay registered
#    from the merchant's YAML config PR; merchant-a-private-key.pem never
#    leaves the merchant's own systems.
ASSERTION=$(build_signed_jwt \
  --iss merchant_a --sub merchant_a --aud https://api.vpay.example/v1/oauth/token \
  --exp "+300s" --jti "$(uuidgen)" \
  --key merchant-a-private-key.pem --alg RS256)

# 2. Exchange the assertion for an access token. No refresh token comes
#    back by design (client_credentials tokens are short-lived) — get a new
#    one by repeating this whole step.
curl -X POST https://api.vpay.example/v1/oauth/token \
  -d grant_type=client_credentials \
  -d client_id=merchant_a \
  -d client_assertion_type=urn:ietf:params:oauth:client-assertion-type:jwt-bearer \
  -d client_assertion="$ASSERTION"
# → { "access_token": "…", "token_type": "Bearer", "expires_in": 300 }
```

## Create a PaymentIntent

Bodies are **form-encoded**, not JSON — that is what the Stripe SDKs send,
and it is unchanged by the auth model above.

```bash
curl -X POST https://api.vpay.example/v1/payment_intents \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
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
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Idempotency-Key: order_1234_confirm_1" \
  -d "payment_method_data[type]=mtn_momo" \
  -d "payment_method_data[mtn_momo][msisdn]=237670000000"
```

→ `"status": "processing"`. The payer gets a prompt on their handset. Wait for
the webhook; **do not ship on `processing`**.

## Confirm — redirect rail (Orange)

```bash
curl -X POST https://api.vpay.example/v1/payment_intents/pi_xxx/confirm \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
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
