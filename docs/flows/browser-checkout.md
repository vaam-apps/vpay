# Browser checkout (`/v1/browser`, `@vpay/stripe-js`)

## The invariant

A payer's browser never holds a merchant credential, and a merchant's server
never has to relay the payer's every move. Two values pass between them
instead: a **publishable key** (names the tenant, authorises nothing) and a
PaymentIntent's own **`client_secret`** (authorises exactly that one intent,
once). Both are safe to put in a URL, a page's JavaScript, or a browser
`fetch` — neither is a bearer token and neither can be exchanged for one.

This is Stripe's own split (`pk_…` + `client_secret`, Stripe.js), reproduced
here because `@stripe/stripe-js` itself cannot be pointed at vpay —
`StripeConstructorOptions` has no host/base-URL member and the loader
hardcodes `https://js.stripe.com` (verified directly; see
`sdks/stripe-js/README.md`'s "Why this exists"). `@vpay/stripe-js` is a
package of vpay's own, Stripe.js-*shaped*, speaking the two routes below.

## Decisions (D1–D5)

Taken by the orchestrator under standing delegation; recorded in
[`docs/plans/2026-09-03-step5c-stripejs.md`](../plans/2026-09-03-step5c-stripejs.md).

- **D1 — publishable keys are an explicit YAML list.**
  `merchant_clients[].publishable_keys: [pk_test_…]`
  (`vpay_config::MerchantClient::publishable_keys`), never derived from
  `merchant_id`. Derivation would make a key unretirable and would let anyone
  who has seen one object's owner reconstruct every other merchant's key. An
  empty list — the default — is the fail-closed shape for a merchant with no
  browser checkout. `Config::validate_all` refuses a duplicate across all
  merchants, a key not matching `^pk_(test|live)_[A-Za-z0-9]{16,64}$`, and a
  `pk_live_` key under `deployment.livemode: false` (or the reverse) —
  `ConfigError::{DuplicatePublishableKey, MalformedPublishableKey,
  PublishableKeyLivemodeMismatch}`, each with its own fixture under
  `backends/tests/fixtures/publishable-key-*.yml`.
- **D2 — `client_secret` is rendered only by `create`, `retrieve`, and the two
  browser routes**, through a wrapper type that never touches the object
  every other response renders:

  ```rust
  pub struct PaymentIntentWithSecret {
      #[serde(flatten)] pub intent: PaymentIntentObject,
      pub client_secret: String,
  }
  ```

  `list` and `events.data` (webhook bodies) keep the 12-key
  `PaymentIntentObject` untouched — `PaymentIntentObject` itself was not
  touched by this step; `every_documented_key_is_present_including_the_null_ones`
  (`vpay-api/src/model.rs`) still asserts exactly 12 keys, and it is that
  tripwire, not a promise in this document, that keeps a future field off the
  webhook body by accident. Proven live by
  `the_client_secret_is_on_create_and_retrieve_and_never_on_the_list` and
  `no_event_body_carries_a_client_secret`
  (`backends/tests/integration/tests/browser_checkout.rs`).
- **D3 — vpay appends nothing to `return_url`.** Stripe's
  `payment_intent`/`payment_intent_client_secret`/`redirect_status` query
  parameters are absent from any URL vpay constructs. A page handling a
  return trip must carry its own state (put `client_secret` in the
  `return_url` you supply, or key on your own order id) and then call
  `retrievePaymentIntent` to learn the outcome.
- **D4 — Step 5c ships push-only. No bounce endpoint.** See "The redirect gap
  (D4)" below.
- **D5 — no in-process rate limiting.** See "Rate limiting (D5)" below.

## The credential model

Two values, both required:

- a **publishable key** (`pk_test_…`/`pk_live_…`) — not a secret. It is
  rendered into a merchant's public checkout page by construction, exactly
  like a JWKS entry, and `MerchantClient::publishable_keys` prints it in
  `Debug` and `config/application.yml` carries it as a plain literal.
- the intent's own **`client_secret`** (`pi_…_secret_…`) — 160 bits from the
  OS CSPRNG (`vpay_core::ids::client_secret_suffix`, two `Uuid::new_v4()`
  draws), minted once at `create` and stored as
  `payment_intents.client_secret_suffix` (migration `0026`,
  `client_secret_suffix_length CHECK` between 32 and 128 chars). This *is*
  the credential — it authorises exactly one payment intent, for its whole
  life, and there is no rotation endpoint: a retry is a new intent.

Neither is a bearer token and neither can be exchanged for one. A payer
holding both can read one intent and confirm it once — nothing else on the
deployment, and nothing at all about any other intent.

### Constant-time comparison, and `client_secret` never in a log line

`authenticate` compares the presented `client_secret` against the derived
expected value with a hand-rolled constant-time compare (`browser::ct_compare`,
`secrets_match`), not `==` — a short-circuiting compare would let response
timing leak how many leading bytes a guess got right.
`a_constant_time_compare_examines_every_byte_even_when_the_first_differs`
pins the property directly; a second, wiring-level test,
`secrets_match_rejects_every_shape_a_boolean_test_can_express`, exists
because a boolean-only test cannot itself distinguish a constant-time compare
from a correct short-circuiting one — its own doc comment explains what it
does and does not prove, and what actually catches the substitution
(`ct_compare` becoming unreachable outside `#[cfg(test)]`, which fails
`cargo clippy --all-targets -- -D warnings` as dead code).

Both `vpay_db::PaymentIntentRow` (the stored `client_secret_suffix`) and
`vpay_api::model::PaymentIntentWithSecret` (the whole joined credential, the
one value that actually authenticates a request) carry **hand-written**
`Debug` impls that redact the secret to `[N chars redacted]` — a derived
`Debug` would put a real credential into any `{:?}` log line or test-failure
message as readily as `println!` would. Proven by
`a_stored_rows_debug_output_never_carries_the_client_secret` and
`a_payment_intent_with_secrets_debug_output_never_contains_the_client_secret`.

### Every failure is the same 404

`browser::authenticate` (`vpay-api/src/browser/mod.rs`) has four ways to
refuse — unknown publishable key, intent not found, intent belongs to a
different merchant, wrong `client_secret` — and answers all four with a
byte-identical `ApiError::NotFound { resource: "payment intent", id }`. Not
politeness: it is the surface's entire confidentiality property. A distinct
answer for "unknown key" would let anyone enumerate which merchants a
deployment serves; a distinct one for "wrong merchant" would turn a stolen
key into an oracle for who owns which intent; a distinct one for "wrong
secret" would separate "this intent exists" from "your secret is wrong",
which is the first half of a guessing attack. Proven byte-for-byte by
`every_credential_failure_is_the_identical_404`.

## The routes

| Method | Path | Params | Answers |
|---|---|---|---|
| GET | `/v1/browser/payment_intents/{id}` | `key`, `client_secret` (query) | `PaymentIntentWithSecret` — the polling endpoint `waitForPaymentIntent` calls every couple of seconds. |
| POST | `/v1/browser/payment_intents/{id}/confirm` | `key`, `client_secret`, `payment_method_data[…]`, `return_url` (form) | Reaches the same `confirm_once` `/v1` itself calls, through a `MerchantScope` minted from the resolved `PayerScope`. |

Mounted `.nest("/v1/browser", …)` with its own `.fallback(not_found)` —
deliberately **not** part of `V1_ROUTES` (its own table is `BROWSER_ROUTES`,
exactly two entries): the OP nest already established the pattern
(`vpay-api/src/lib.rs`'s own note on `/v1/oauth/not_a_route` flattening), and
`Router::nest("/v1", …)` registering `/v1` and `/v1/{*rest}` would otherwise
swallow a browser path before it reached this nest and answer a `401` no
payer can ever clear (`the_browser_nest_is_outside_the_merchant_boundary_and_answers_its_own_404`).
There is no `create`, no `list`, and no `cancel` here — proved by
`the_browser_surface_has_no_create_no_list_and_no_cancel` — and no route
answers `401` (`every_browser_route_is_reachable_without_a_merchant_token`).

### No `Idempotency-Key`

`/v1`'s POSTs require one (D7 of the merchant surface). This one cannot: a
browser request carrying a custom header is CORS-preflighted, and Stripe.js —
what this surface is shaped after — sends none. `confirm` calls
`confirm_once` directly rather than going through the `/v1` POST helper that
reads the key. What stands in for it is what was always doing the real work:
`confirm_once` refuses before any insert if the intent already has a charge,
and the `one_charge_per_intent` unique index refuses even if two requests
race past that check together. A double-tap gets a `200` and a `409`, never
two charges — `a_browser_confirm_needs_no_idempotency_key_and_a_second_one_is_the_409`.
What is genuinely lost is *replay*: a merchant's `/v1` retry under a key
replays the stored response; a payer's retry on this surface re-executes and
is refused. A `409` telling the payer to poll is a worse experience than a
replayed `200`, and it is not a second charge — stated here rather than
papered over.

### CORS

`/v1/browser` is the **only** nest carrying a `CorsLayer`
(`allow_origin(Any)`, `GET`/`POST`/`OPTIONS`, `allow_headers([CONTENT_TYPE])`,
`max_age(600s)`, `allow_credentials` off). The merchant `/v1` nest gets none
at all — nothing legitimate calls it from a browser, and a permissive header
there would invite a browser to send a bearer token cross-origin, which is
what `allow_credentials: off` plus no header at all forecloses. Proven by
`the_browser_nest_answers_a_preflight_and_the_merchant_nest_does_not` and the
router-level unit test `cors_is_mounted_on_the_browser_nest_and_on_no_other`.

## Rate limiting (D5): none, in this process, deliberately

What stands between a guesser and an intent is 160 bits of `client_secret`,
the uniform 404, and one-charge-per-intent — not a counter. A per-process
limiter across N replicas is a limit of N times what it claims, and building
a correctly shared one is a shared-state design nobody asked for as part of
this step. **This is an operational requirement, not a solved problem**: a
deployment must put rate limiting in front of `/v1/browser` at the ingress
(reverse proxy, WAF, CDN) before serving real payers. Nothing in this repo
enforces that today, and nothing in CI checks that an operator has done it.

## The redirect gap (D4)

Step 5c ships **push-only**. `confirmPayment` still returns the rail's real
`next_action.redirect_to_url` and `@vpay/stripe-js` will navigate to it when
`redirect !== 'if_required'`, but the **return trip is not wired**: vpay has
no `/provider/{code}/callback` route (`docs/api/README.md`'s own section for
it says "Status: not implemented"). Concretely, for the Orange Money
adapter, `return_url = setting(config, "return_url").unwrap_or(callback_url)`
is a **deployment** setting, never the merchant's per-checkout one;
`config/application.yml` sets no `return_url`, so Orange redirects the payer
to `{public_base_url}/provider/orange_money/callback` — a path the router
does not mount. The merchant's own `return_url` is stored on `charges` and
echoed back in `next_action` as a label; nothing redirects to it. **Do not
ship a redirect-rail (Orange) checkout on `@vpay/stripe-js` until the
callback route lands.** This gap is owned by whichever step builds provider
callbacks, not by this one — tracked in `docs/status.md`.

## The merchant SDKs and `client_secret` (gap found and closed in this step)

When block C was written, neither merchant SDK's `PaymentIntent` type declared
`client_secret`: the Node type omitted it (readable only through a cast) and
the Rust struct's derived `Deserialize` dropped it silently. Both were fixed
the same day (`c40a137`): `sdks/nodejs/src/types.ts` declares
`client_secret?: string` (present on `create`/`retrieve`, `undefined` on list
items and events), and `sdks/rust/src/model.rs` carries
`client_secret: Option<String>` with `#[serde(default)]` and a hand-written
`Debug` that redacts it. Pinned by `sdks/rust/tests/resources.rs` (create and
retrieve surface it, a list item is `None`), `sdks/rust/tests/debug_redaction.rs`,
and `sdks/nodejs/src/client.test.ts`; the cast in
`frontends/tests/e2e/cypress/tasks/checkoutTasks.ts` is gone. A merchant hands
the value to `@vpay/stripe-js` in the browser and never logs it.

## What proves this surface exists

- **`backends/tests/integration/tests/browser_checkout.rs`** — real Postgres,
  a real WireMock MTN rail, the shipping adapters, the shipping router, the
  shipping merchant SDK (to create the intent) and raw `reqwest` (to speak
  the browser wire contract the merchant SDK cannot express). Every claim
  above with a test name attached lives here.
- **`sdks/stripe-js/src/*.test.ts`** (87 tests, vitest) — against a real
  `node:http` stub of `/v1/browser` (`src/testing/browser-stub.ts`), proving
  the package's own behaviour: form encoding, the redirect rule, the polling
  ladder, the error-envelope mapping, and the compile-time compatibility
  claims against `@stripe/stripe-js`'s own types
  (`src/compat.test.ts`). ADR-0006 governs test doubles reachable from
  `vpay-server`/`vpay-worker-bin`; an SDK's own unit-test stub, excluded from
  `dist/` and imported only from `*.test.ts`, is neither.
- **`examples/checkout-browser/`** — a plain HTML + JS payer page, no
  framework, importing the built `@vpay/stripe-js` ESM. See its own README
  for the 7-step walkthrough against `just demo`.
- **`frontends/tests/e2e/cypress/e2e/checkout.cy.ts`** — drives that example
  against the real compose stack: mints an intent server-side (Node, the
  `demo-merchant` OAuth keypair, never in the browser) via
  `cy.task('mintCheckoutPaymentIntent')`, visits the page, confirms an MTN
  push with MSISDN `237600000ce0` (`examples/merchant-demo`'s
  `DEMO_MSISDN`, keying WireMock scenario `mtn-e2e-poll`), and asserts the
  rendered status reaches `succeeded` once `vpay-worker` — running in the
  stack, not stubbed — settles the charge. Added to `.github/workflows/ci.yml`'s
  `e2e` job. See `docs/status.md` for whether this run's own output is
  attached to the current pass or still pending.

Nothing above has ever taken real money: both rails are WireMock hosts on a
compose network, same as everywhere else in this repository.

## Status

See `docs/status.md`'s Backend/Frontend/Merchant SDK tables for the
machine-checked picture (route count, migration count, test counts). This
document is the design and the proof map; that one is the ledger of what was
actually re-measured, when.
