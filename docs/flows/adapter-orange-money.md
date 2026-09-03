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

All three wire calls are implemented in `vpay-adapter-orange-money`
(`submit`, `query_status`, `parse_callback`). `refund` is **not** implemented
and never will be on this rail: the adapter does not override the port's
default, so it answers `ProviderError::Unsupported` — a permanent capability
answer, not unbuilt work. There is no `orange_money::*` `NotImplemented` token
left. See [../status.md](../status.md).

**What is proven, and by what.** The pure halves — token-URL derivation, the
status table, the request body's shape (`amount` as a JSON *number*), callback
parsing, `ref_extra`'s shape, payment-URL validation — are **53 unit tests in
the crate, 53 passed, 0 skipped** (`cargo nextest run -p
vpay-adapter-orange-money`, measured 2026-09-03). The wire behaviour is proven
by `backends/tests/conformance` against a real `wiremock/wiremock` host reached
over HTTP exactly as the rail is (ADR-0006); the mappings live in
`backends/tests/conformance/wiremock/orange/mappings/`, which is the same
directory `compose.yml` bind-mounts, so a mapping fixed for one is fixed for
both.

**All 11 conformance port cases now pass for this rail** — 26 tests across
both rails, 26 passed, 0 skipped, measured 2026-09-03 with `cargo nextest run
-p vpay-tests-conformance`. *(An earlier draft of this section said five of
nine passed and four failed on `query_status` for want of a `pay_token` in
the suite's `ChargeRef`. That was fixed in the suite, where it belonged: a
`ProviderFlow::Redirect` rail is now seeded with the `pay_token` its previous
`submit` returned, mirroring how a `Push` rail is seeded with `payer_ref`.
The adapter's behaviour did not change — a `query_status` with no `pay_token`
is still `ProviderError::Config` and never `NotFound`, because "the rail has
no record" is what tells a reconciler nothing has happened yet, and a charge
whose `pay_token` we lost is the opposite case.)*

**What the transport refuses.** Every call goes through
`vpay_provider::http`: redirects are returned rather than followed
(`redirects_are_refused_and_never_followed`), proxies are ignored, and bodies
are capped at 256 KiB (`an_oversized_rail_body_is_refused_at_the_cap`,
`a_long_rail_body_is_bounded_before_it_reaches_a_log_line`). Each request
carries `ProviderConfig::request_timeout` explicitly. **It did not, until the
Step 3 security review** — MTN applied the deadline per request and Orange
silently did not, so a black-holed Orange host held a worker task for as long
as the shared client allowed. The `payment_url` the rail returns is validated
as `http(s)` and ≤2048 characters before it can reach a browser or the
`charges.redirect_url` column, and a refusal never quotes the URL
(`a_payment_url_that_is_not_an_http_url_is_refused`,
`a_payment_url_over_the_column_limit_is_refused`,
`refusing_a_payment_url_does_not_quote_it`). The bearer is cached behind a
length-prefixed SHA-256 fingerprint of `client_id` + `client_secret`, so
rotating only the secret evicts it (`rotating_only_the_secret_evicts_the_cached_bearer`,
`a_field_boundary_cannot_be_shifted_into_a_collision`), and its lifetime is
measured from the *send*, not the answer
(`the_lifetime_is_measured_from_the_send_not_from_the_answer`).

**Still unverified against the real rail**, and each blocks something concrete:

- The error-body vocabulary for `webpayment`. Orange documents none, so a 4xx
  that is not a 401/404 becomes `Rejected{provider_error}` carrying the raw
  body. That is the "unmapped, alert on it" bucket of
  [failures.md](failures.md), not a settled mapping.
- The sub-reasons of `FAILED`. Same bucket, same reason.
- Whether a repeated `order_id` really is idempotent. The stub returns the same
  `pay_token`, and the port requires a duplicate to be `Submitted` rather than
  an error, but this is an assumption about Orange, not an observation.
- Item 1 of the list above (does the notification carry `pay_token`?). The
  adapter carries it through *when present* and never requires it.
- `notif_token` equality is **not** performed by the adapter — it holds no
  state. `parse_callback` returns the received `notif_token` in `ref_extra` and
  fails closed when there is none; comparing it with the stored one is the
  callback route's job, and that route is not built yet.
- The hosted page's `lang` defaults to `fr` when a deployment configures none.
  It is the one defaulted field in the request body.
- **The 401 → re-mint → retry path is unproven.** No mapping returns 401 from
  `webpayment` or `transactionstatus` *after* a good token, so only the 401 on
  the token endpoint itself is covered
  (`bad_credentials_are_not_reported_as_a_payer_problem`).
- **Nothing here has ever called Orange.** Every wire assertion above is
  against WireMock; a mapping faithful to this document but not to Orange
  would pass. All seven "to confirm" items above still stand.
