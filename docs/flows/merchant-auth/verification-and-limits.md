# Merchant auth — webhook verification, what can go wrong, and the known limitations

_Split out of [docs/flows/merchant-auth.md](../merchant-auth.md) on 2026-09-11 by exp57, which broke a 748-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## Webhook verification

Both SDKs ship the verifier for [webhooks.md](../webhooks.md)'s scheme — the
same one `examples/webhook-receiver` hand-rolls:

- Header `Vpay-Signature: t=<unix seconds>,v1=<hex>`; more than one `v1=`
  may be present during a secret rotation and any one matching is enough.
- Signed payload is the literal bytes `"<t>.<raw body>"`. **The raw request
  body must be used**; a parsed-and-reserialised body breaks the HMAC.
- HMAC-SHA256 with the endpoint secret, hex-encoded, compared in constant
  time.
- Reject if `|now − t|` exceeds the tolerance (default 300 s), if the header
  is malformed, or if no `v1` matches. Then, and only then, parse the body as
  an `event`.

Delivery is at-least-once; the verifier does not dedupe by `event.id` —
that is the merchant's job, and the docs say so where the verifier is used.

## What can go wrong

| Failure                                                              | Where it surfaces                                                                          | What the SDK does                                                                                               |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| Wrong private key, unregistered `kid`, `client_id` typo              | `401`/`400` from the token endpoint with `invalid_client`                                  | Returns an authentication error; no retry                                                                       |
| Merchant disabled via `disabled_clients` ([status](../../status.md)) | Token endpoint refuses, or a `/v1` route returns `401`                                     | One re-auth attempt, then the error                                                                             |
| Assertion `exp` too far out (> 300 s)                                | Refused by the OP                                                                          | Cannot happen: the SDK refuses to be configured that way                                                        |
| Clock skew beyond 60 s                                               | `invalid_client`                                                                           | Returned; the message names the check that failed only as far as the OP does (it is deliberately not an oracle) |
| Token endpoint path differs from the SDK default                     | `404` with the Stripe-shaped `unknown_route` envelope                                      | Returned as an unexpected-response error; the fix is the `issuer`/`token_endpoint` setting                      |
| Merchant's PR merged but a pod not yet restarted                     | `invalid_client` from one replica, success from another (ADR-0010's rolling-deploy window) | Returned; the merchant's own retry policy decides                                                               |

## Known limitations (security review, 2026-09-02)

Two findings from the review of the first server-side implementation are
recorded here rather than fixed, because each needs a decision a maintainer
has not made:

- **The `jti` replay namespace is global, not per merchant.**
  `oauth_client_assertion_jtis.jti` is the primary key on its own, and
  `authkestra_op::client_assertion::ClientAssertionStore::record_jti` hands
  the store no `client_id` to scope by. RFC 7523 only requires uniqueness
  per issuer, so a merchant whose library used a counter or a timestamp as
  `jti` would collide with — and could deliberately pre-spend — another
  merchant's values. **Onboarding requirement until this changes: `jti`
  MUST be a UUID v4** (both vpay SDKs do this). Scoping the key to
  `(client_id, jti)` needs a new migration and either an upstream seam or a
  per-client store instance; that is the decision left open.
- **No rate limit in front of `/v1/oauth/token` or `/v1`.** A known
  `client_id` (they are public) costs one `disabled_clients` `SELECT` per
  token request before any signature check, and ADR-0009 leaves `/token`
  rate limiting to the ingress. Confirm the ingress does it before relying
  on that; nothing in this repository enforces it.
