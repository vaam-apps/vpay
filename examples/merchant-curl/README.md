# Raw HTTP

> **What runs, and what does not.** The token step, create, retrieve, list
> and cancel below all execute against a running `vpay-server`. **Confirm
> does not complete**: no rail adapter implements `submit` yet, so a confirm
> reaches the rail and answers `501 not_implemented` — the response shown
> under each confirm is the *intended* shape, not what you will get today.
> See ../../docs/status.md and
> [ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md).
>
> `/v1/refunds`, `/v1/balance` and `/v1/events` are not routed at all and
> answer the honest `404 unknown_route`.
>
> **Every `POST` under `/v1` requires an `Idempotency-Key` header.** The
> *server* never defaults one: a `POST` without the header is refused with a
> `400` naming `idempotency_key`, before anything is created. Both SDKs do
> default one (a UUIDv4 per call), which is why this is only ever a curl
> problem. Reuse the same key to retry the *same* request safely; use a new
> key for a new request. See "Idempotency" below.
>
> Array parameters are shown in Stripe's curl style (`key[]=v`); the SDKs
> send the indexed form (`key[0]=v`) Stripe's own SDKs use, and the server
> accepts both, as Stripe does.

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

## Idempotency

Every `POST` under `/v1` **must** carry an `Idempotency-Key`:

```
-H "Idempotency-Key: 5f0d5e4c-9f4e-4d2a-9c1b-4a2a1e0f7f31"
```

**Both SDKs send one for you, and the default is a fresh UUIDv4 per call** —
`vpay_sdk::RequestOptions` (Rust) and the `RequestOptions` argument (Node)
generate one when you do not supply a key, so a `POST` from either SDK is
never the `400` below. That default is the right one for the common case: it
makes the SDK's own network-level retry safe.

Override it — `RequestOptions::new().with_idempotency_key("…")` in Rust,
`{ idempotencyKey: "…" }` in Node — when the thing you must not do twice is
bigger than one HTTP call: a key derived from your order id (say
`order_1234_create`) makes the *whole operation* idempotent across process
restarts, a crashed job runner, or a queue that delivers the same message
twice. A per-call UUID cannot protect you there, because the retry is a
different call.

Keys are scoped to your merchant and kept for 24 hours. Two rules follow from
that, and both matter if you derive keys: a derived key must be unique per
logical operation (`order_1234_create` and `order_1234_confirm`, never
`order_1234` for both), and reusing one for a genuinely different request is
an error rather than a new object — see below.

What the server does with a key:

- **Missing or empty** → `400`, `{"error":{"type":"invalid_request_error",
  "param":"idempotency_key", …}}`. Nothing is created.
- **Same key, same body** → the *stored* response is replayed, byte for byte.
  Nothing is created a second time. This is what makes a network retry safe.
- **Same key, different body** → `400`,
  `{"error":{"type":"idempotency_error","code":"idempotency_key_in_use", …}}`.
- **Same key, first request still running** → `400`,
  `{"error":{"type":"idempotency_error","code":"idempotency_key_in_flight", …}}`
  — *"A request with this Idempotency-Key is still in progress; retry
  shortly."* Retry the same call after a moment. Do **not** switch to a new
  key until you know how the first one ended: a new key is a new operation,
  and the first one may still be about to succeed.

  Branch on `error.code`, not on the status or the sentence:
  `idempotency_key_in_flight` is the one error here that clears itself, while
  `idempotency_key_in_use` needs you to change something. Both SDKs surface
  the envelope's `code` (`vpay_sdk::Error::Api { code, .. }` in Rust,
  `VpayApiError.code` in Node); neither maps it to a distinct exception type.

Which answers are *stored* for a replay, and which hand the key back:

- **`2xx` and most `4xx` are stored.** A `4xx` is your request's own outcome —
  re-running it would produce it again — so the retry is answered identically
  rather than re-executed. (Stripe behaves the same way.)
- **A validation failure on `POST /v1/payment_intents` is the exception**: the
  key is released, because what is valid can change. If your currency was not
  configured, an operator configures it, and you retry under the same key, you
  get the intent you asked for — not a 24-hour-old refusal that is no longer
  true. Nothing was written, so re-executing is exactly as if the request had
  never been made.
- **`5xx` is not stored, and the key is released** — including the confirm
  `501` below. "We do not know whether the rail saw it" is the only honest
  thing to say, and freezing that for 24 hours would answer a merchant
  retrying after the deployment was fixed with the old outage. The retry
  therefore *re-executes*; that is safe because it is not the key that
  prevents a double charge — the unique index behind "one charge per intent,
  forever" is, and a re-executed confirm meets it and answers `409`.

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
  -d "metadata[order_id]=1234" \
  -d "description=Order #1234"
```

```json
{ "id": "pi_…", "object": "payment_intent", "amount": 5000, "currency": "xaf",
  "status": "requires_payment_method",
  "payment_method_types": ["mtn_momo", "orange_money"],
  "next_action": null, "last_payment_error": null,
  "metadata": { "order_id": "1234" }, "description": "Order #1234",
  "created": 1753401600, "livemode": false }
```

`amount=5000` is **5,000 FCFA**. XAF is zero-decimal.

The parameters, exactly:

| Parameter | Required | Notes |
|---|---|---|
| `amount` | yes | Integer minor units, `1..=2^53-1`. |
| `currency` | yes | Case-insensitive on the way in, lowercase on the way out. Must be one this deployment configures. |
| `payment_method_types[]` | yes, ≥ 1 | Rail codes. Each must be enabled on this deployment, or the create is refused — an intent naming a rail that is off could never be confirmed. |
| `metadata[k]` | no | ≤ 50 keys, key ≤ 40 chars, value ≤ 500 chars. |
| `description` | no | ≤ 1000 chars. Shown to you, never to the payer. |

## Retrieve, and list

```bash
curl https://api.vpay.example/v1/payment_intents/pi_xxx \
  -H "Authorization: Bearer $ACCESS_TOKEN"

# Newest first. `limit` defaults to 10 and is capped at 100.
curl -G https://api.vpay.example/v1/payment_intents \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -d limit=10 \
  -d starting_after=pi_xxx      # next page
# ... or -d ending_before=pi_yyy for the previous one.
```

```json
{ "object": "list", "data": [ /* payment_intent objects */ ],
  "has_more": true, "url": "/v1/payment_intents" }
```

An id belonging to another merchant answers exactly the same `404` as an id
that never existed. That is deliberate: this API is not an oracle for which
ids exist.

## Confirm — push rail (MTN)

```bash
curl -X POST https://api.vpay.example/v1/payment_intents/pi_xxx/confirm \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Idempotency-Key: order_1234_confirm_1" \
  -d "payment_method_data[type]=mtn_momo" \
  -d "payment_method_data[mtn_momo][msisdn]=237670000000"
```

*Intended* → `"status": "processing"`; the payer gets a prompt on their
handset, and you wait for the webhook — **do not ship on `processing`**.

*Today* → `501`:

```json
{ "error": { "type": "api_error", "code": "not_implemented",
             "message": "This operation is not implemented yet." } }
```

`mtn_momo::submit` is not written (`../../docs/status.md`). The request is
real and gets that far: vpay records the charge and the attempt before
calling the rail, so the confirm is refused rather than silently dropped. The
intent stays `requires_payment_method`.

## Confirm — redirect rail (Orange)

```bash
curl -X POST https://api.vpay.example/v1/payment_intents/pi_xxx/confirm \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Idempotency-Key: order_1234_confirm_1" \
  -d "payment_method_data[type]=orange_money" \
  -d "return_url=https://shop.example/order/1234/return"
```

*Intended* → `"status": "requires_action"` with:

```json
{ "next_action": { "type": "redirect_to_url",
                   "redirect_to_url": { "url": "https://webpayment.orange-money.com/…" } } }
```

— send the payer's browser there.

*Today* → the same `501` as the push rail, for the same reason
(`orange_money::submit` is not written).

## Cancel

Legal only while the intent is `requires_payment_method` — once a confirm has
handed a charge to a rail, cancelling would tell you a payment was withdrawn
while the payer's handset was still prompting.

```bash
curl -X POST https://api.vpay.example/v1/payment_intents/pi_xxx/cancel \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Idempotency-Key: order_1234_cancel_1"
```

→ `"status": "canceled"`, or `409` if the status no longer allows it.

## Retrying a failed payment

Create a **new** PaymentIntent. An intent gets exactly one charge for its entire
life — that is what makes double-charging structurally impossible.
