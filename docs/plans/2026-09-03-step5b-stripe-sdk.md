<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 5b — official Stripe SDK compatibility: implementation-ready design

Requirement (user, 2026-09-03): merchants must be able to use the official Stripe SDKs against vpay
with an empty key plus a custom `config.authenticator` performing vpay's private_key_jwt → bearer
handshake.

Decisions taken by the orchestrator under the user's delegation (do not reopen): (1) amend ADR-0010's
"No Stripe SDK can authenticate against vpay" with a dated note (keep it Accepted; own the error where
it was made); (2) emit `stripe-should-retry` derived from `Classify::retry`; (3) keep
`IdempotencyKeyInFlight` at 400 and cover it with `stripe-should-retry: true`; (4) DO duplicate the
webhook signature under `Stripe-Signature` (already done in the Step 5 deliverer) so copy-pasted Stripe
recipes work unchanged — `Vpay-Signature` stays authoritative in docs; (5) the conformance job uses the
demo overlay's real merchant keypair (`-f compose.demo.yml` + `just gen-demo-keys` in the e2e job);
(6) advertise no `apiVersion`; (7) `async-stripe` (Rust) is a follow-up after the Node suite defines
"compatible". Package decision: an entry point `@vpay/sdk/stripe` (no new package), `stripe` as an
optional peer dependency.

## 0. Four things that are not what the ticket implies

**S1 — an Accepted ADR says this is impossible, in bold.** `docs/adr/0010-merchant-auth-private-key-jwt.md:78-86`: *"**No Stripe SDK can authenticate against vpay.** … none of them implement RFC 7523 client-assertion signing or an OAuth2 token-endpoint round trip. A merchant using an official Stripe SDK now needs custom glue code around it… `README.md` and `examples/merchant-node/` have been corrected to say so plainly rather than implying a drop-in."* That claim is now false as written: `config.authenticator` is arbitrary async code invoked **per attempt** with the whole request. Four prose sites assert the "cannot": `docs/adr/0010…md:78`, `README.md:37`, `examples/README.md:9`, `examples/merchant-node/index.mjs:11-16`. This step is either an ADR amendment or a superseding ADR — **Q1**. It is *not* a straight port: the honest framing is "ADR-0010 said glue is needed; this step ships the glue", which keeps 0010's decision intact and only retracts one over-strong sentence.

**S2 — vpay's in-flight idempotency answer is a status stripe-node will not retry, and its `Conflict` is one it *will*.** `RequestSender._shouldRetry` retries **409 unconditionally** (`RequestSender.ts:343-346`) and every `>=500` (`:353-356`), honouring `stripe-should-retry: false|true` above both (`:337-342`). Default `maxNetworkRetries` is **2** (`platform/PlatformFunctions.ts:84-86`). vpay answers `409` for `Category::Conflict` (`vpay-core/src/error.rs:148`) — e.g. cancelling an already-`processing` intent — so stripe-node will silently re-POST it twice with the same key and then surface it as `StripeAPIError` (409 falls through every branch of `generateV1Error`, `Error.ts:13-44`). Conversely vpay's `IdempotencyKeyInFlight` is `400` (`error.rs:242-253` documents this as an open maintainer question) which stripe-node will **not** retry — the one case where retrying is correct. Both need the `stripe-should-retry` header. **Q2/Q3.**

**S3 — `Idempotency-Key` is on *every* v1 POST, unconditionally.** `_defaultIdempotencyKey` (`RequestSender.ts:388-408`) keys every v1 POST "*including when maxNetworkRetries is 0*". vpay's D7 requirement (`idempotency.rs:114-119`) therefore costs stripe-node users nothing — no server change. The generated key is `stripe-node-retry-<uuid>`, ASCII, 43 bytes: inside vpay's 255-byte printable-ASCII rule (`idempotency.rs:44,102`).

**S4 — `Stripe-Signature` needs no server work at all.** `constructEvent(payload, header, secret, tolerance)` takes the header **value**, not the request (`Webhooks.ts:140-171`); it parses `t=…,v1=…` (`:505-533`), HMACs `` `${t}.${payload}` `` (`:329-334`) with lowercase hex SHA-256 (`crypto/NodeCryptoProvider.ts:9-14`), tolerance default 300 (`:138`). That is byte-identical to Step 5's `Vpay-Signature` (`docs/plans/2026-09-03-step5-webhooks.md:102`). `buildEvent` validates **nothing** except rejecting `object === 'v2.core.event'` (`Webhooks.ts:128-135`) — no `id`/`type`/`api_version` requirement. So a merchant writes `constructEvent(raw, req.headers['vpay-signature'], secret)` today. Duplicating the header is cosmetic — **Q4**.

## 1. The authenticator

**Decision: an entry point in `@vpay/sdk`, not a new package.** `TokenManager` (`sdks/nodejs/src/auth.ts:180-286`) already does cache + single-flight + `invalidate()`, is tested against a local `node:http` server, and duplicating the `private_key_jwt` minting into a second package is how the two drift.

New file `sdks/nodejs/src/stripe-auth.ts`, new export condition `"./stripe"` in `package.json` (`dist/stripe-auth.js`), so `@vpay/sdk` core keeps zero dependency on `stripe`. `stripe` goes in `peerDependencies` (`optional: true` via `peerDependenciesMeta`) **and** `devDependencies` for the type import only.

```ts
export interface StripeAuthenticatorOptions {
  baseUrl: string; clientId: string; privateKeyPem: string | KeyObject;
  kid?: string | undefined; scope?: string | undefined;
  issuer?: string | undefined; tokenEndpoint?: string | undefined;
  audience?: string | undefined; assertionLifetimeSeconds?: number | undefined;
  timeoutMs?: number | undefined; fetch?: typeof fetch | undefined;
}
export interface VpayStripeAuthenticator {
  (request: { host: string; port: string; path: string; method: string;
              headers: Record<string, string | number | string[]>;
              body: string; protocol: string; }): Promise<void>;
  invalidate(): void;
}
export function createStripeAuthenticator(
  options: StripeAuthenticatorOptions
): VpayStripeAuthenticator;
```

The parameter type is structurally `StripeRequest` (`Types.ts:79-87`) written out rather than imported, so the module type-checks with `stripe` absent; a `types.test.ts` case asserts assignability to `Stripe.StripeConfig['authenticator']` when it is present. `RequestAuthenticator = (request: StripeRequest) => Promise<void>` (`Types.ts:88`).

Body: `request.headers.Authorization = 'Bearer ' + await tokenManager.getToken()`. Mutating other fields is *possible* (`RequestSender.ts:671-690` re-reads `request.host/port/path/method/headers/body/protocol` after the authenticator resolves) but the authenticator must mutate **headers only** — anything else desynchronises the `Content-Length` computed at `:612-620`.

**It cannot see the 401.** The authenticator runs before the request and its promise resolves before any response exists; a rejection becomes `StripeError('Unable to authenticate the request')` (`RequestSender.ts:781-786`). So expiry is handled by `TokenManager`'s existing `expires_in − min(30, expires_in/2)` margin (`auth.ts:278-282`), and `invalidate()` is exposed on the returned function so a merchant can force re-auth from their own `catch (e instanceof Stripe.errors.StripeAuthenticationError)`. Do **not** try to be clever: 401 does not retry (`_shouldRetry` returns false for 4xx) so there is no retry loop to hook.

Merchant usage (the snippet that goes in docs verbatim):

```js
import Stripe from 'stripe';
import { createStripeAuthenticator } from '@vpay/sdk/stripe';
const authenticator = createStripeAuthenticator({
  baseUrl: 'http://localhost:8080', clientId: 'acme-cameroon',
  privateKeyPem: readFileSync('./merchant-key.pem', 'utf8'), kid: 'acme-cameroon-2026-08',
});
const stripe = new Stripe('', {
  authenticator, host: 'localhost', port: '8080', protocol: 'http',
  maxNetworkRetries: 2, timeout: 30_000, telemetry: false,
});
```

`new Stripe('', {authenticator})` is supported: `_setAuthenticator` throws only if *both* or *neither* are supplied (`stripe.core.ts:1312-1328`). `host`/`port`/`protocol` are in `ALLOWED_CONFIG_PROPERTIES` (`:942-960`) and a non-default `host` overrides every base address (`:1395-1402`). **`basePath` is fixed at `/v1/` and is not configurable** (`:928`, absent from the allowed list) — and it is moot anyway, because generated resources hardcode absolute paths: `'/v1/payment_intents'`, `` `/v1/payment_intents/${encodeURIComponent(id)}/confirm` `` (`resources/PaymentIntents.ts:38, 61-64, 155, 218`). `getAPIMode` returns `'v1'` for anything not starting `/v2` (`utils.ts:475-480`), which selects form encoding. So vpay's existing paths match with no rewriting.

**Rust twin — follow-up, not this step.** `async-stripe` builds its `Client` around a `Headers`/`ClientSecret` pair and has no per-request async hook equivalent to `RequestAuthenticator`; the reachable approach is a custom `hyper`/`reqwest` middleware layer wrapping `async_stripe::Client`'s transport, which is a larger surface than the Node hook and is not required by the stated requirement. Scope it as its own step after this one lands and the Node conformance suite defines "compatible".

## 2. Server compatibility work

1. **`request-id` response header.** stripe-node reads `headers['request-id']` in three places (`RequestSender.ts:71, 100, 249`) — never `x-request-id`. Without it `err.requestId` and `obj.lastResponse.requestId` are `undefined`, which is exactly the "contact support with the request id" promise (`error.rs:60-62`) failing for Stripe SDK users. Add a `PropagateRequestIdLayer::new(HeaderName::from_static("request-id"), ...)` alongside the existing `x_request_id()` one at `lib.rs:731`, both fed by the same `REQUEST_ID_HEADER` constant (`lib.rs:101`). Two headers, one value. Do not rename the existing one.
2. **`stripe-should-retry`.** Emit `stripe-should-retry: false` on `409` (`Category::Conflict`) and `true` on `IdempotencyKeyInFlight`'s `400` **only if** Q3 answers "keep 400". Set it in `ApiError::into_response` (`error.rs:761-786`) from `self.retry()` — `Retry::AfterBackoff` → `true`, otherwise `false` — which is derived, not hand-picked, and so respects ADR-0011.
3. **Headers accepted and ignored.** `Stripe-Version` (`2026-08-26.dahlia`, `apiVersion.ts:3`), `Stripe-Account`, `Stripe-Context`, `X-Stripe-Client-User-Agent`, `X-Stripe-Client-Telemetry`, `User-Agent: Stripe/v1 NodeBindings/22.6.1` (`RequestSender.ts:452-460, 497-511`). axum ignores unknown headers already — **no code change**; only a documented statement. Do **not** 400 on `Stripe-Account`: it is `removeNullish`-stripped when unset (`:490-494`), so it is only ever present if a merchant deliberately set it, and a 400 there is a worse diagnostic than a documented "Connect is not a thing here".
4. **Error vocabulary already matches.** `Category::stripe_type` (`vpay-core/src/error.rs:162-176`) emits exactly `invalid_request_error`/`authentication_error`/`idempotency_error`/`rate_limit_error`/`api_error`. `generateV1Error` (`Error.ts:13-44`) branches on **status first**, `type` only inside 400/404: 429 → `StripeRateLimitError`, 400/404 + `idempotency_error` → `StripeIdempotencyError`, other 400/404 → `StripeInvalidRequestError`, 401 → `StripeAuthenticationError`, 403 → `StripePermissionError`, 402 → `StripeCardError`. vpay's 404/`resource_missing` (`error.rs:187`) lands as `StripeInvalidRequestError` with `code: 'resource_missing'` — the assertion the conformance suite pins. vpay's `403` (missing scope) becomes `StripePermissionError` despite carrying `type: invalid_request_error`; harmless, document it. **429 is unreachable** — nothing in `backends/crates` constructs `Category::RateLimited`; say so in docs rather than shipping a shape nothing emits.
5. **`expand[]` — ignore.** `CreateParams`/`ConfirmParams`/`ListParams` (`v1/payment_intents.rs:116-125, 455-458, 331-335`) have no `deny_unknown_fields`, so `expand` is already dropped silently. stripe-node encodes arrays **indexed**, `expand[0]=charge` (`utils.ts:110-118`) — not `expand[]` — and vpay's decoder handles both (`form.rs:32-40`). No change; document that expansion is not supported.
6. **Params stripe-node sends that vpay refuses.** `payment_method_types` is **required and non-empty** and must name an enabled rail (`payment_intents.rs:1293-1310`), so a copy-pasted `automatic_payment_methods: {enabled: true}` snippet gets a clean 400 naming `payment_method_types`. `confirm: true` on create is **silently ignored** — the worst outcome in the set, because the merchant believes they confirmed. Add `confirm` to `CreateParams` and 400 with `param: "confirm"` and a message pointing at `/confirm`. Everything else — encoding, `+` handling — already agrees: stripe-node percent-encodes with `encodeURIComponent` and then *decodes brackets back* (`utils.ts:62-75`), so a literal `+` is `%2B` and a space is `%20`, exactly what `form.rs:56-60` requires.
7. **`Stripe-Signature` on deliveries** — see Q4; coordinate with Step 5 block B (`signature_header`, plan `:99`).

## 3. Conformance suite

New workspace package `sdks/stripe-compat` (`@vpay/stripe-compat`, private, vitest — matched by `pnpm-workspace.yaml`'s `sdks/*`), depending on the real `stripe@^22.6.1` and on `@vpay/sdk`. **Out-of-process against the compose stack**, not an in-process harness: ADR-0006 and the fact that half of what is being proved is header behaviour through the real router.

Cases: create → retrieve → `autoPagingToArray({limit})` (needs only `data[]` with `id` and `has_more` — `autoPagination.ts:70-95, 133-146`; `ListObject` (`model.rs:194-208`) supplies both plus `url`) → confirm → cancel; `retrieve('pi_nope')` → `StripeInvalidRequestError` with `code === 'resource_missing'`; a client whose authenticator hands a garbage bearer → `StripeAuthenticationError`; the same `Idempotency-Key` twice via `{idempotencyKey}` request options → identical `id`; same key, changed `amount` → `StripeIdempotencyError`; `webhooks.constructEvent` against a body+`Vpay-Signature` pulled from the WireMock receiver's `GET /__admin/requests` journal (Step 5 §7).

**CI: extend the existing `e2e (compose)` job** (`.github/workflows/ci.yml:129-205`) with `pnpm --filter @vpay/stripe-compat test` after `pnpm --filter @vpay/e2e e2e` at `:198`, against `localhost:8080`. A new job would rebuild both images for a second stack. The e2e stack's `acme-cameroon` client has a placeholder JWK (`config/application.yml:113-118`), so this needs the **demo overlay's** real keypair: add `-f compose.demo.yml` and `just gen-demo-keys` to the job, or extend `gen-e2e-signing-key` to also emit a merchant keypair — **Q5**.

Honestly not covered: real Stripe API-version drift (nothing pins vpay against a future `stripe` release; a Renovate bump is what will surface it), card/3DS flows, Connect, `search`, `expand`, webhooks endpoints CRUD, and the fact that `PaymentIntentObject` (`model.rs:129-170`) omits `client_secret`, `amount_received`, `capture_method` and `confirmation_method` that stripe-node's TS types declare as present — a *type-level* lie with no runtime effect (`StripeResource._makeRequest` casts, it never validates).

## 4. Docs

New `docs/flows/stripe-sdk-compat.md`: the snippet above; the divergences (no API keys — OAuth handshake; `Idempotency-Key` mandatory not optional; XAF/EUR only; no `automatic_payment_methods`, `confirm: true`, `expand`, `search`, Connect; `payment_method_data.type` is a rail code so TS needs `as any` or a declaration-merging snippet; `next_action` is only ever `redirect_to_url` (`model.rs:88-99`); 429 never emitted; `client_secret` absent). Amend `docs/adr/0010-…md:78-86`, `README.md:37`, `examples/README.md:9`, `examples/merchant-node/index.mjs:11-16`. `docs/api/README.md` gains a "Using the official Stripe SDKs" subsection. `docs/status.md`: a new row, plus the "SDK cannot authenticate" claim at `:372`. New `examples/merchant-stripe-node/index.mjs` mirroring `examples/merchant-node/index.mjs`'s flow.

## 5. Work split

**A — server (Rust).** `lib.rs`: second propagate layer for `request-id`. `error.rs`: `stripe-should-retry` in `into_response`, derived from `Classify::retry`. `v1/payment_intents.rs`: `confirm: Option<String>` on `CreateParams` → 400. Tests: `a_response_carries_both_request_id_headers`, `a_conflict_tells_stripe_node_not_to_retry`.

**B — `@vpay/sdk/stripe` (Node).** `sdks/nodejs/src/stripe-auth.ts` with the two signatures in §1 verbatim; `package.json` `exports["./stripe"]`, `peerDependencies`/`peerDependenciesMeta`; `stripe-auth.test.ts` against a local `node:http` server as `auth.test.ts` already does — assert one token fetch for N concurrent calls, `Authorization` set on the mutated object, `invalidate()` forces a re-mint, and that nothing but `headers` is written.

**C — conformance + CI + example + docs.** `sdks/stripe-compat/**`, the `ci.yml` step, `examples/merchant-stripe-node/`, all of §4.

## 6. Decisions needed from a human

1. **ADR-0010's "No Stripe SDK can authenticate against vpay" — amend in place, or supersede with a new ADR?** *Default: amend §Consequences of 0010 with a dated note, keep the ADR Accepted.* Gained: 0010's actual decision (no API keys, YAML registry, no refresh token) is untouched and still correct; the retraction sits where the wrong claim is, per "own errors explicitly". Lost: an amendment is less visible than ADR-0013 in a list of ADRs, and future readers may miss that the compatibility story changed. **This paragraph is a maintainer statement about the project's own claims — do not let implementation quietly delete it.**
2. **Emit `stripe-should-retry` at all?** *Default: yes, derived from `Classify::retry`.* Gained: stripe-node stops re-POSTing 409s twice (measured behaviour, `RequestSender.ts:343-346`) and starts retrying in-flight idempotency, which is the one case retry fixes. Lost: a Stripe-specific header on every response of a surface that also serves `@vpay/sdk`, and one more thing ADR-0011's renderer must stay consistent about.
3. **Move `IdempotencyKeyInFlight` from 400 to 409?** *Default: leave it at 400 and cover it with `stripe-should-retry: true`.* Gained: no ADR-0011 change (the status is derived from `Category`; moving it drags `IdempotencyKeyReused` along — `error.rs:242-253` spells this out and explicitly reserves it), and the retry header gets the same practical outcome. Lost: divergence from real Stripe stays, and a hand-rolled client that branches on status rather than `code` still mis-handles it. `error.rs:242-253` already marks this as a maintainer decision.
4. **Duplicate the webhook signature as `Stripe-Signature` on deliveries?** *Default: no — document `constructEvent(raw, req.headers['vpay-signature'], secret)`.* Gained: one header, one name, no ambiguity about which one is authoritative; costs Step 5 nothing and needs no coordination. Lost: a merchant's existing `req.headers['stripe-signature']` line needs a one-word edit, and copy-pasted Stripe recipes break at that line rather than working unchanged.
5. **How does the conformance job get a real merchant keypair?** *Default: add `-f compose.demo.yml` and `just gen-demo-keys` to the existing `e2e` job.* Gained: reuses a tested, already-scripted key path; no new config fixture. Lost: the e2e job's stack becomes the demo stack, which republishes ports differently (`compose.demo.yml:87`) and couples Cypress's environment to the demo overlay — the alternative is a third overlay used only by CI, which is one more thing to keep in sync.
6. **`apiVersion` to advertise.** *Default: advertise none — accept and ignore `Stripe-Version`, echo nothing.* Gained: no false claim that vpay implements a dated Stripe API version, and no obligation to track `2026-08-26.dahlia`. Lost: `obj.lastResponse.apiVersion` is `undefined` for Stripe SDK users, and a merchant pinning `apiVersion` gets no signal that the pin is meaningless.
7. **`async-stripe` twin — now or follow-up?** *Default: follow-up step, after the Node conformance suite exists.* Gained: the Node suite defines what "compatible" means before a second implementation has to satisfy it. Lost: Rust merchants have no Stripe-SDK path in this step, and `sdks/rust` keeps being the only Rust option.