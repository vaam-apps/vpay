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

### Where the payer comes back to

`return_url` and `cancel_url` are **per charge**, filled by the core and
carried on `vpay_provider::ChargeRef::return_url` (Step 9, D2). Both fields
get the same value: Orange's page distinguishes "paid" from "cancelled" and
vpay cannot — the outcome comes from the authenticated `transactionstatus`
read, and a charge the payer abandoned is `Pending` until it expires — so two
URLs would encode a distinction nothing checks. A charge with no `return_url`
is `ProviderError::Config` before the call; this adapter will not invent one.

Until 2026-09-04 both fields came from **deployment** settings
(`settings.return_url` / `settings.cancel_url`, falling back to `notif_url`),
which was one answer per deployment to a per-charge question. Those two
settings keys are gone; nothing shipped set them.

Where that value comes from is the core's business and not this adapter's: the
merchant's own `return_url` for a direct confirm, and vpay's own return page
when a Checkout Session drives the charge ([hosted-checkout.md](hosted-checkout.md)).

`lang` is unchanged and is still the one defaulted field in the request body.

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
8. **Whether Orange exposes an account-holder name lookup at all, and under
   which product and credential** (issue #47, added 2026-09-05). MTN Collections
   has `GET /v1_0/accountholder/msisdn/{msisdn}/basicuserinfo` under the same
   subscription key `submit` already uses; Orange has a KYC/customer product,
   and **its route is not confirmed from this repository and is not being
   claimed**. Until it is, `orange_money` declares
   `supports_account_holder_lookup: false` and inherits the port's
   `ProviderError::Unsupported` — a permanent capability answer the core
   branches on, *not* a `NotImplemented` token, because nothing here is unbuilt
   work someone owes. `a_number_the_rail_has_no_record_of_is_not_an_error` and
   its three siblings in the conformance suite assert exactly that for this
   rail. If the answer to this item turns out to be "yes, here", the flag
   becomes `true`, this adapter overrides
   `ProviderAdapter::account_holder_name` with its own `NotImplemented` token
   until it is written, and `docs/status.md` grows a row.

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

**All 13 conformance port cases now pass for this rail** — 33 tests in the
suite, 33 passed, 0 skipped, measured 2026-09-04 with `cargo nextest run
-p vpay-tests-conformance` (26 on 2026-09-03; `the_submit_tells_the_rail_where_to_call_back`
was added by Step 8 lane C and `the_submit_tells_the_rail_where_to_send_the_payer_back`
by Step 9 lane 2, and Step 9 lane 2b added three MTN-only cases that do not
parameterise over this rail). *(An earlier draft of this section said five of
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
  state — and, since Step 8 (2026-09-04), ~~comparing it with the stored one is
  the callback route's job, and that route is not built yet~~ **not by the
  callback route either**. `parse_callback` returns the received `notif_token`
  in `ref_extra` and fails closed when there is none;
  `vpay_api::provider_callback` **discards that `ref_extra`** rather than
  merging unverified rail material onto the charge, so the comparison is still
  unbuilt and `ref_extra` repair from a callback is still unavailable. The
  adapter's fail-closed check is now load-bearing in production, not only in
  tests: it is the only thing between an unauthenticated POST and a queued poll
  ([../reference/rails.md](../reference/rails.md)).
- The hosted page's `lang` defaults to `fr` when a deployment configures none.
  It is the one defaulted field in the request body.
- **The 401 → re-mint → retry path is unproven.** No mapping returns 401 from
  `webpayment` or `transactionstatus` *after* a good token, so only the 401 on
  the token endpoint itself is covered
  (`bad_credentials_are_not_reported_as_a_payer_problem`).
- **The stub's hosted page is not Orange's.**
  `backends/tests/conformance/wiremock/orange/mappings/stub-hosted-page.json`
  serves `/stub-hosted-page/{pay_token}` with a Pay link and a Cancel link so
  a browser can finish the redirect leg — and since Step 9 a browser does
  (`shop-hosted.cy.ts`). The real rail *stores* `return_url` and `cancel_url`
  against the `pay_token` at submit and renders them from its own state;
  WireMock can only template from the current request, so the submit's
  `payment_url` carries the two URLs as query parameters and the page templates
  them back. The pairing is real — those are the bytes that submit sent — but
  nothing here shows Orange would accept a `return_url` it had not been told
  about, and nothing claims it would. The stub also has no *cancel* semantics
  of its own: its "Cancel" link is the same return URL, which is why Step 9's
  Cypress proof of the cancel path is a **decline** forwarding to `cancel_url`
  rather than a payer abandoning the page.
- **Nothing here has ever called Orange.** Every wire assertion above is
  against WireMock; a mapping faithful to this document but not to Orange
  would pass. All **eight** "to confirm" items above still stand — the eighth,
Orange's account-holder route, was added on 2026-09-05 with issue #47 and is
the reason `supports_account_holder_lookup` is `false` for this rail rather
than unbuilt.
