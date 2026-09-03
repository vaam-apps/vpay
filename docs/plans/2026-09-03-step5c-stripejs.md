# Step 5c — Stripe.js-compatible browser surface (`@vpay/stripe-js`)

Design produced 2026-09-03 by the scoper. Decisions taken by the orchestrator (standing delegation
"take all decisions yourself") are recorded first; the design follows verbatim.

## Decisions

- **D1 Publishable keys are an explicit YAML list** `merchant_clients[].publishable_keys: [pk_test_…]`,
  never derived from `merchant_id` (retirable, several per merchant, matches Stripe). An empty list is the
  fail-closed default and must be documented as such.
- **D2 `client_secret` is rendered only by `create`, `retrieve` and the two browser routes** through
  `PaymentIntentWithSecret`; webhooks `events.data` and `list` keep the 12-key `PaymentIntentObject`.
- **D3 vpay appends nothing to `return_url`**; `@vpay/stripe-js` documents that Stripe's
  `payment_intent`/`payment_intent_client_secret`/`redirect_status` params are absent.
- **D4 Step 5c ships push-only.** No bounce endpoint; the Orange return trip is recorded as not wired in
  `docs/status.md` and `docs/flows/browser-checkout.md` (the missing `/provider/{code}/callback` route is
  a named gap, owned by the step that builds provider callbacks).
- **D5 No in-process rate limiting**; the ingress requirement is documented in the flow and the runbook.
- **Ordering:** after Step 5b (both amend ADR-0010 and `docs/api/README.md`); migration number `0023`
  (Step 5 owns `0022`).

## Verified up front

`@stripe/stripe-js` cannot be pointed at vpay. `StripeConstructorOptions` has exactly five members —
`stripeAccount`, `apiVersion`, `locale`, `betas`, `developerTools`; no host/base-URL. The loader hardcodes
`ORIGIN = 'https://js.stripe.com'` and `isStripeJSURL` accepts only `js.stripe.com/v3` or
`js.stripe.com/{v3|[a-z]+}/stripe.js`. Elements iframes and the bundle's XHRs are Stripe-origin by
construction. So: a **drop-in-compatible package of our own** (`@vpay/stripe-js`) against a new browser
surface. **Not compatible, ever, and said in the package README:** Elements, cards, 3DS, Payment Element,
Checkout (hosted or embedded), Link, Payment Request/Apple/Google Pay, `confirmCardPayment`,
`createPaymentMethod`, ConfirmationTokens, SetupIntents.

## 0. Four things scoping found that the ticket does not imply

**S1 — `client_secret` on `PaymentIntentObject` trips a deliberate tripwire and leaks into webhooks.**
`backends/crates/vpay-api/src/model.rs`'s test `every_documented_key_is_present_including_the_null_ones`
asserts `object.len() == 12`, and `the_merchant_sdk_deserialises_what_this_renders` decodes through the
shipping `vpay_sdk::PaymentIntent`. `backends/crates/vpay-worker/src/handlers.rs` (`intent_snapshot`)
renders `events.data` from that same type, so a field added there is in every webhook body and every
`list` page by construction. **Do not touch `PaymentIntentObject`.** Add a wrapper in `model.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct PaymentIntentWithSecret {
    #[serde(flatten)] pub intent: PaymentIntentObject,
    pub client_secret: String,
}
```

used only by `create`, `retrieve` and the two browser handlers.

**S2 — the redirect half of a browser checkout is not reachable, and it is not 5c's fault.** The Orange
adapter sends `return_url = setting(config,"return_url").unwrap_or(callback_url)` — a deployment setting,
never the merchant's. `config/application.yml` sets no `return_url`, so Orange redirects the payer to
`{public_base_url}/provider/orange_money/callback` and **no such route exists** (the router mounts only
`/healthz`, `/v1/oauth`, `/v1`). The merchant's `return_url` is written to `charges.return_url` (migration
`0019`) and rendered back in `next_action` — a label, nothing redirects to it. Nothing in this repo
documents what Orange appends to `return_url`; do not guess. **5c ships push-only.** `confirmPayment` with
a redirect rail returns the real `next_action.redirect_to_url` and `redirect:'always'` navigates to it,
but the return trip is documented as not wired.

**S3 — browser routes must not join `V1_ROUTES`.** The integration test
`every_registered_v1_path_answers_401_without_a_token` walks `vpay_api::V1_ROUTES`. A second nest with its
own `.fallback` is the already-proven pattern (`lib.rs` records the `/v1/oauth/not_a_route` flattening).

**S4 — `Idempotency-Key` is mandatory on every `/v1` POST** (`idempotency.rs`, via `PostRequest::read`).
Requiring it in the browser turns a CORS simple request into a preflighted one. Stripe.js sends none.

## 1. Server: the browser surface

**Publishable keys.** New field on `MerchantClient` (`vpay-config/src/oauth.rs`):

```rust
#[garde(skip)] #[serde(default)] pub publishable_keys: Vec<String>,
```

Non-secret (same footing as `jwks`); prints in `Debug`. `Config::validate_all` refuses: duplicates across
all merchants, a key not matching `^pk_(test|live)_[A-Za-z0-9]{16,64}$`, and `pk_live_` under
`deployment.livemode: false` (or vice-versa).

`ResourceConfig` (`v1/mod.rs`) gains `merchant_id_by_publishable_key: BTreeMap<String,String>` and
`pub fn merchant_id_for_publishable_key(&self, key: &str) -> Option<&str>`.

**Migration `backends/migrations/0023_payment-intent-client-secret.sql`:**

```sql
ALTER TABLE payment_intents ADD COLUMN client_secret_suffix TEXT;
UPDATE payment_intents SET client_secret_suffix =
  replace(gen_random_uuid()::text,'-','') || replace(gen_random_uuid()::text,'-','')
  WHERE client_secret_suffix IS NULL;
ALTER TABLE payment_intents
  ALTER COLUMN client_secret_suffix SET NOT NULL,
  ADD CONSTRAINT client_secret_suffix_length CHECK (char_length(client_secret_suffix) BETWEEN 32 AND 128);
```

Suffix only — `pi_xxx_secret_yyy` is derived, never stored twice. New in `vpay-core/src/ids.rs`:
`pub fn client_secret_suffix() -> String` (32 alphabet chars = 160 bits, two `Uuid::new_v4()` draws) and
`pub fn client_secret(id: &str, suffix: &str) -> String` → `format!("{id}_secret_{suffix}")`.
`NewPaymentIntent`/`PaymentIntentRow` gain the column; `PaymentIntentRow` gets a hand-written `Debug`
redacting it.

**Routes** — new module `backends/crates/vpay-api/src/browser/mod.rs`, mounted `.nest("/v1/browser", …)`
with its own `.fallback(not_found)`, plus `pub const BROWSER_ROUTES: &[V1Route]`:

| Method | Path | Params |
|---|---|---|
| GET | `/v1/browser/payment_intents/{id}` | `key`, `client_secret` (query) |
| POST | `/v1/browser/payment_intents/{id}/confirm` | `key`, `client_secret`, `payment_method_data[…]`, `return_url` (form) |

Both go through one extractor, `browser::PayerScope` (constructed only by `browser::authenticate`):

```rust
pub struct PayerScope { merchant_id: String, intent_id: String }
async fn authenticate(config: &ResourceConfig, pool: &PgPool, id: &str, key: &str, secret: &str)
    -> Result<(PayerScope, PaymentIntentRow), ApiError>;
```

Order: `merchant_id_for_publishable_key(key)` → `payment_intents::get_by_id(pool, id)` →
`row.merchant_id == merchant_id` → constant-time compare of `secret` against
`ids::client_secret(&row.id, &row.client_secret_suffix)`. **Every** failure is the identical
`ApiError::NotFound { resource: "payment intent", id }`, byte-for-byte. No create, list or cancel exists
on this surface.

`confirm` reuses `v1::payment_intents::confirm_once` verbatim with a `MerchantScope` minted from
`PayerScope::merchant_id`; a payer can influence only `payment_method_data` and `return_url`. **No
`Idempotency-Key`**: the one-charge-per-intent unique index plus the pre-insert check is the double-submit
protection (409 with the right advice). `retrieve` returns `PaymentIntentWithSecret` and is the polling
endpoint.

**CORS.** Add `"cors"` to the workspace `tower-http` features. On the browser nest only:

```rust
CorsLayer::new()
  .allow_origin(Any)
  .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
  .allow_headers([CONTENT_TYPE])
  .max_age(Duration::from_secs(600))
```

`allow_credentials` stays off. **Rate limiting: none in-process, stated plainly**; document the ingress
requirement in `docs/flows/browser-checkout.md` and rely on 160-bit secrets, uniform 404, and
one-charge-per-intent.

**Merchant integration** (Stripe's model): server calls `paymentIntents.create` with the OAuth SDK, gets
`client_secret`, renders `pk` + `client_secret` into the page.

## 2. `@vpay/stripe-js` (`sdks/stripe-js`)

Browser ESM, zero runtime deps, TS strict, vitest. `package.json` name `@vpay/stripe-js`, `"type":"module"`,
`exports: { ".": {types:"./dist/index.d.ts", default:"./dist/index.js"} }`.

```ts
export interface VpayStripeOptions { baseUrl: string; fetch?: typeof fetch | undefined }
export function loadStripe(publishableKey: string, options: VpayStripeOptions): Promise<Stripe>;
export interface Stripe {
  retrievePaymentIntent(clientSecret: string): Promise<PaymentIntentResult>;
  confirmPayment(options: { clientSecret: string;
      confirmParams?: { return_url?: string; payment_method_data?: Record<string, unknown> };
      redirect?: 'always' | 'if_required' }): Promise<PaymentIntentResult>;
  handleNextAction(options: { clientSecret: string }): Promise<PaymentIntentResult>;
  confirmMobileMoneyPayment(clientSecret: string,
      data: { type: string; msisdn: string }): Promise<PaymentIntentResult>;
  waitForPaymentIntent(clientSecret: string,
      options?: { timeoutMs?: number; intervalMs?: number }): Promise<PaymentIntentResult>;
}
export type PaymentIntentResult =
  | { paymentIntent: PaymentIntent; error?: undefined }
  | { paymentIntent?: undefined; error: StripeError };
export interface StripeError { type: string; code?: string; message?: string; param?: string }
```

Shapes copied from `@stripe/stripe-js`'s `stripe.d.ts` (`PaymentIntentResult`, `StripeError`,
`retrievePaymentIntent`, the `confirmPayment` overloads, `handleNextAction`). `return_url` is optional
because push rails have no return trip. Ship a docs snippet showing
`import type {PaymentIntentResult as StripeResult} from '@stripe/stripe-js'` type-aliasing against ours.

**Never rejects.** Every failure becomes `{error}`: the envelope `{error:{type,code,message,param}}` maps
1:1; a network failure becomes `{type:'api_connection_error'}`.

`clientSecret` → id: `indexOf('_secret_')` → `slice(0, i)`, reject if absent or the prefix is not `pi_`.
`confirmPayment` POSTs `key`, `client_secret`, `payment_method_data[type]`,
`payment_method_data[<type>][msisdn]`, `return_url` as `application/x-www-form-urlencoded`, then: if the
result carries `next_action.redirect_to_url.url` and `redirect !== 'if_required'`,
`window.location.assign(url)` and return a never-resolving promise (Stripe's behaviour); otherwise resolve.
`handleNextAction` retrieves, then navigates the same way. `waitForPaymentIntent` polls every `intervalMs`
(default 2000, jittered) until `succeeded`, `canceled`, or `requires_payment_method`-with-
`last_payment_error`, or `timeoutMs` (default 180000) → `{error:{type:'api_error',code:'polling_timeout'}}`.

## 3. Hosted payer page

`PayerSheet` (`frontends/packages/ui`) is a vaul drawer for dashboard charge detail with zero callers; it is
not a checkout UI. **`examples/checkout-browser/` — one static `index.html` + `checkout.js`, no
framework**, importing the built `@vpay/stripe-js` ESM, with `pk`/`client_secret` from the query string.
Wiring `PayerSheet` to the package is a follow-up.

## 4. Security

Secret 160 bits from the OS CSPRNG; no rotation endpoint (retry = a new PaymentIntent). Redacted in
`PaymentIntentRow`'s hand-written `Debug`; `not_found`'s message renders the id, not the secret. Key+secret
must both resolve to the row's own `merchant_id`. Payer can set only `payment_method_data` + `return_url`.
CORS confined to the `/v1/browser` nest; the merchant `/v1` nest gets no `CorsLayer`.

## 5. Tests

**Rust integration** (`backends/tests/integration/tests/browser_checkout.rs`, Postgres + WireMock): create
through the OAuth SDK → browser confirm by raw `reqwest` with `key`+`client_secret` → `processing` and a
WireMock journal entry; wrong secret → 404; unknown `pk` → 404; another merchant's `pk` → 404,
**byte-identical to the wrong-secret body**; `POST /v1/browser/payment_intents` and `.../cancel` → 404;
`OPTIONS` preflight → `access-control-allow-origin: *`; a browser confirm without `Idempotency-Key`
succeeds; a second one → 409. A sibling of `every_registered_v1_path_answers_401_without_a_token` asserts
`BROWSER_ROUTES` is exactly two entries and that neither answers 401.

**Package unit tests** (`sdks/stripe-js/src/*.test.ts`, vitest) against a `node:http` stub of the browser
surface, as `sdks/nodejs` does. ADR-0006 forbids test doubles reachable from `vpay-server`/
`vpay-worker-bin`; an SDK's own unit-test stub is neither — say so in the test file header.

**Cypress** (`frontends/tests/e2e/cypress/e2e/checkout.cy.ts`): serve `examples/checkout-browser` as a
static route from the dashboard container, drive an MTN push confirm against the compose stack, poll to
`succeeded` with `vpay-worker` running. Needs a real merchant keypair (`config/application.yml`'s JWKS is a
placeholder), so the e2e job adds `-f compose.demo.yml` + `just gen-demo-keys` — the same blocker Step 5b
raises; solve it once.

## 6. Work split (parallel, after 5b)

**A — server.** Migration `0023`; `vpay-core/src/ids.rs`; `vpay-db/src/payment_intents.rs`;
`vpay-config/src/oauth.rs` (`publishable_keys`) + `config.rs` `validate_all` +
`ConfigError::{DuplicatePublishableKey, MalformedPublishableKey, PublishableKeyLivemodeMismatch}` +
fixtures; `vpay-api/src/v1/mod.rs`; `vpay-api/src/model.rs` (`PaymentIntentWithSecret`); **new**
`vpay-api/src/browser/mod.rs`; `vpay-api/src/lib.rs` (nest + `CorsLayer`); root `Cargo.toml` `+"cors"`;
the integration suite.

**B — package.** `sdks/stripe-js/{package.json,tsconfig.json,tsconfig.build.json,vitest.config.ts,README.md,src/{index.ts,client.ts,errors.ts,form.ts,types.ts,*.test.ts}}`.

**C — example, e2e, docs.** `examples/checkout-browser/`; the Cypress spec + dashboard static route + the
`compose.demo.yml` overlay in `.github/workflows/ci.yml`; `docs/flows/browser-checkout.md`;
`docs/api/README.md` (a third surface — amend the line "no other way to authenticate on this surface");
`docs/status.md` rows; ADR-0010 amendment on publishable keys + client secrets as the browser credential
model, alongside 5b's amendment.

## 7. Questions the scoper raised (answered by D1–D5 above)

1. Explicit publishable-key list vs derived — **explicit** (D1).
2. `client_secret` in webhooks/list — **no** (D2).
3. `redirect_status` / return-URL params — **vpay appends nothing** (D3).
4. Bounce endpoint now or later — **deferred, push-only** (D4).
5. Rate limiting — **ingress, documented** (D5).

## 8. Outcome (block C, 2026-09-03)

**Landed.** All three blocks: the server surface (A), `@vpay/stripe-js` (B),
and — this pass — `examples/checkout-browser`, `checkout.cy.ts`, and the
docs listed in §6's "C" split. Verified rather than assumed: `just
demo_port=18084 demo` (all 7 steps), `mint.mjs` against that stack, a real
browser session driven through confirm → processing → waiting → succeeded
(screenshotted), then `pnpm --filter @vpay/e2e e2e` against the same stack —
`checkout.cy.ts` passing (`2914ms`) alongside `dashboard.cy.ts`'s existing 3
tests, 4/4. `sdks/stripe-js` grew from 82 tests (as this plan's §5 was
written) to **87** during this pass, from a security-hardening commit
landing concurrently in the same shared worktree (redirect scheme allowlist,
`secrets_match` wiring test, a second redacting `Debug` impl on
`PaymentIntentWithSecret`) — not this block's own work, but its numbers are
now what `docs/status.md` cites.

**Deviations and things found, not in the original design:**

- **A real currency mismatch, not a documentation slip.** `examples/checkout-browser/mint.mjs`
  originally minted in `xaf`, matching this document's §2 example. The demo
  overlay's `mtn_momo` rail settles in **EUR** (matching
  `examples/merchant-demo`'s own `DEMO_CURRENCY`), and confirming an XAF
  intent against it produced a real, correctly-rendered error —
  `invalid_request_error/invalid_request: rail 'mtn_momo' settles in EUR;
  this PaymentIntent is XAF` — caught by actually running the example
  against `just demo`, not by inspection. Fixed in `mint.mjs` and
  `checkoutTasks.ts`; both now mint in EUR with a comment explaining why.
- **A path-traversal guard bug in `serve.mjs` that 403'd every request**,
  including `index.html` itself. `new URL(".", import.meta.url)` already
  yields a directory path ending in the OS separator; the original code
  appended a second one before `startsWith`, so nothing ever matched. Found
  the same way — running the server for real and getting 403 on `/index.html`
  — not from reading the code. Fixed by re-normalising with `join(ROOT, "")`
  instead of concatenating a separator, with the failure mode recorded in a
  comment at the fix site.
- **Neither merchant SDK's `PaymentIntent` type had exposed `client_secret`**,
  even though `/v1`'s own `create`/`retrieve` render one via this step's own
  `PaymentIntentWithSecret` (D2). `sdks/nodejs` hid it only at the type
  level (`http.ts`'s `JSON.parse(text) as T` is a cast, not a filter, so
  plain JavaScript still read it — which is why `mint.mjs` and
  `checkoutTasks.ts` were written the way they were). `sdks/rust` was worse:
  serde silently dropped the field with no workaround from inside that type,
  so `examples/merchant-demo` could not recover a `client_secret` at all.
  **Resolved in this same step, once the worktree constraint lifted**
  (`c40a137`, same day): `sdks/nodejs/src/types.ts` now declares
  `client_secret?: string` and `sdks/rust/src/model.rs` declares
  `client_secret: Option<String>` with `#[serde(default)]` and a
  hand-written redacting `Debug`; `checkoutTasks.ts`'s cast is removed. See
  `docs/flows/browser-checkout.md` and `docs/status.md` for the closed gap.
- **`sdks/stripe-js/README.md`'s "Type compatibility, precisely" section is
  accurate and was written by block B, not amended here**: Stripe's
  `PaymentIntentResult`/`StripeError` are assignable to ours (a widening);
  ours is *not* assignable to Stripe's in either direction for the object
  types themselves (`PaymentIntent`, and our `StripeError` is intentionally
  wider) — see that section for the precise, compile-time-pinned claims
  rather than restating them loosely here.
- **`waitForPaymentIntent` ends the poll on the very first `api_connection_error`
  (or any `{error}`), rather than retrying transient network failures until
  the timeout.** This is `client.ts`'s own documented choice (`an
  api_connection_error is something the caller must decide about —
  swallowing three minutes of connection failures and then reporting
  polling_timeout would describe the wrong fault`), not an oversight this
  pass found — but it is worth stating plainly here because it means a
  single dropped packet during a payer's wait aborts the whole poll instead
  of riding it out. A caller wanting resilience has to retry
  `waitForPaymentIntent` itself; nothing in `@vpay/stripe-js` does that for
  it. `examples/checkout-browser/checkout.js` does not retry either — a
  network blip during the wait shows the payer an error with no retry
  button, which is an honest gap in the example, not a hidden one.
- **The stale claim "`client_secret` is absent" in
  `docs/flows/stripe-sdk-compat.md`** (written before this step, when it was
  true) and the matching line in `docs/adr/0010-merchant-auth-private-key-jwt.md`'s
  earlier amendment were found while cross-checking D2 against the rest of
  the docs tree and corrected in this pass, not left for a future reader to
  trip over.

**Not done / explicitly out of scope for block C:**

- The `/provider/{code}/callback` route (D4's gap) — owned by a later step.
- Rate limiting at the ingress (D5) — an operational requirement, not code;
  nothing in this repository enforces or checks it.
- This pass's Cypress run is **local only** (`just demo_port=18084 demo` +
  `pnpm --filter @vpay/e2e e2e` on the authoring machine), with the new
  step and env vars added to `.github/workflows/ci.yml`'s `e2e` job. It has
  not yet been observed green in an actual CI run — `docs/status.md` says so
  explicitly rather than implying CI proof from a local one.
- ~~Fixing `sdks/nodejs`/`sdks/rust`'s `client_secret` typing gap~~ — resolved
  the same day, `c40a137` (see the deviation above).
- Retry-on-transient-failure for `waitForPaymentIntent`/the example page
  (see above).
