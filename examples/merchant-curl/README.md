# Raw HTTP

> **The token step below runs; nothing after it does.**
> `POST {base}/v1/oauth/token` exists and is served by `vpay-server` at that
> exact path, alongside `GET {base}/v1/oauth/.well-known/openid-configuration`
> and `GET {base}/v1/oauth/jwks.json`. Every *other* `/v1` path — every curl
> from "Create a PaymentIntent" down — is behind that token and answers a
> `404 unknown_route` envelope once past it: vpay implements no `/v1`
> resource route yet, so those requests and their responses are the intended
> shape, not something you can execute. See ../../docs/status.md and
> [ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md).
> Array parameters are shown in Stripe's curl style (`key[]=v`); the SDKs
> send the indexed form (`key[0]=v`) Stripe's own SDKs use, and the server
> must accept both, as Stripe does.

## Authenticate — `client_credentials` + `private_key_jwt`

`/v1` does not accept an API key ([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)).
A merchant signs a short-lived JWT assertion with the private half of the
keypair whose public JWK vpay has on file (registered via a YAML config PR —
see [ADR-0003](../../docs/adr/0003-yaml-configuration.md)), then exchanges
that assertion for a bearer access token:

```bash
# 1. Build a client assertion (RFC 7523). Pseudocode for the raw shape —
#    both vpay SDKs do this for you (`vpay_sdk::auth` in sdks/rust,
#    `mintClientAssertion` in sdks/nodejs); the exact claims the OP checks
#    are tabulated in docs/flows/merchant-auth.md. `merchant_a` is the
#    client_id vpay registered from the merchant's YAML config PR;
#    merchant-a-private-key.pem never leaves the merchant's own systems.
ASSERTION=$(build_signed_jwt \
  --iss merchant_a --sub merchant_a --aud https://api.vpay.example/v1/oauth/token \
  --exp "+300s" --jti "$(uuidgen)" \
  --key merchant-a-private-key.pem --alg RS256)

# 2. Exchange the assertion for an access token. No refresh token comes
#    back by design (client_credentials tokens are short-lived) — get a new
#    one by repeating this whole step.
#
#    `audience` is not optional in practice: vpay's /v1 boundary requires
#    `aud: vpay:v1` on every access token (vpay_config::MERCHANT_AUDIENCE).
#    Omit it and the OP mints a token whose `aud` is your client_id, which
#    /v1 then refuses with a bare 401. Ask for an audience your registration
#    does not list and the OP answers `invalid_target`.
curl -X POST https://api.vpay.example/v1/oauth/token \
  -d grant_type=client_credentials \
  -d client_id=merchant_a \
  -d client_assertion_type=urn:ietf:params:oauth:client-assertion-type:jwt-bearer \
  -d client_assertion="$ASSERTION" \
  -d audience=vpay:v1
# → { "access_token": "…", "token_type": "Bearer", "expires_in": 900 }
#   900 s is vpay_api::op::ACCESS_TOKEN_TTL_SECS, not a configurable.
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
