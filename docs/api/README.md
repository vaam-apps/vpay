# API

vpay exposes two HTTP surfaces, and conflating them is a security bug.

## `/v1` — the merchant API (Stripe-shaped object model, not Stripe-shaped auth)

Authenticated with OAuth2 `client_credentials` + `private_key_jwt` (RFC
7523) — **not** an API key. Each merchant is a statically registered OAuth2
client, configured directly in vpay's YAML config
([ADR-0003](../adr/0003-yaml-configuration.md)), holding its own private
key; vpay stores only the merchant's **public** JWK. There is no
`sk_live_`/`sk_test_` key, no database-stored secret, and no other way to
authenticate on this surface. See
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md), which supersedes
the scope-boundary paragraph of
[ADR-0009](../adr/0009-dashboard-oidc-provider.md) that had kept `/v1` on
Stripe-shaped API keys.

Everything else about `/v1` is still Stripe-shaped: form-encoded bodies,
Stripe's object model, Stripe's error envelope, Stripe's idempotency
semantics.

**Errors.** Every non-2xx response is the Stripe envelope
`{ "error": { "type", "code", "message", "param"? } }`, and the status,
`type` and `code` are *derived* from the failing error's classification
(`vpay_core::error::Classify` → `vpay_api::ApiError`), never chosen per
handler — so the same failure always gets the same answer. The full
category → status/type/code table is in
[../flows/errors.md](../flows/errors.md); the decision is
[ADR-0011](../adr/0011-error-modelling.md). `message` is the error's
public message and never contains hosts, tables, library text or
credentials; the full chain goes to the server log.

**Status: the OAuth2 endpoints are served, and so is `/v1/payment_intents`
— up to the rail, which does not exist.** As of 2026-09-03, `vpay-server`
serves three unauthenticated OP endpoints plus five authenticated
payment-intent routes. The OP endpoints are unauthenticated by necessity,
not omission: requiring a bearer token to obtain a bearer token is
circular, and a client that has never spoken to vpay has to be able to find
the token endpoint and the keys.

| Method | Path | Auth | What it does |
|---|---|---|---|
| POST | `/v1/oauth/token` | none | `client_credentials` + `private_key_jwt` only. RFC 6749 error JSON; `invalid_client` is 401, every other error 400. |
| GET | `/v1/oauth/.well-known/openid-configuration` | none | Hand-built, so it advertises only what this deployment serves — no `/authorize`, no `/userinfo`, no device or refresh grant, `private_key_jwt` as the only client-auth method. |
| GET | `/v1/oauth/jwks.json` | none | Every publishable signing key (the active one plus any retired-but-unexpired key inside the rotation window), `Cache-Control: public, max-age=300`. |

The issuer is `{deployment.public_base_url}/v1/oauth` and is a deployment
setting, not a per-client one. `GET /v1/oauth/token` — the right path with
the wrong method — returns axum's bare `405` with an empty body rather than
the Stripe envelope; the status is correct and a `method_not_allowed`
renderer for the whole surface is the fix, not a special case here.

**Every other `/v1` path requires a merchant access token.** An
unauthenticated request gets `401`
`authentication_error`/`missing_bearer_token`
(`every_registered_v1_path_answers_401_without_a_token` walks the router's
own route table, `vpay_api::v1::V1_ROUTES`, so a route cannot exist without
being covered); an authenticated request to a path with no route gets `404
unknown_route`. A token must also carry a scope: `payments:write` for any
non-read method, `payments:read` **or** `payments:write` for `GET`/`HEAD`,
else `403` `forbidden`
(`a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not`).

### Served today

`vpay_api::v1::V1_ROUTES` is the router's source, not a copy of it. Five
methods across four paths:

| Method | Path | Request params | Answer |
|---|---|---|---|
| POST | `/v1/payment_intents` | `amount` (integer minor units, `1 ..= 2^53-1`), `currency` (lowercase, must be one this deployment configures), `payment_method_types[]` (rail codes, each enabled here), `metadata[…]` (≤50 keys, ≤40-char keys, ≤500-char values), `description` (≤1000 chars) | `200` + `payment_intent` in `requires_payment_method` (`create_then_retrieve_round_trips_through_the_sdk`) |
| GET | `/v1/payment_intents/{id}` | | `200` + `payment_intent`, or `404 resource_missing` — **including for another merchant's id**, byte for byte (`merchant_b_cannot_read_merchant_as_intent`) |
| GET | `/v1/payment_intents` | `limit` (default 10, capped at 100), `starting_after`, `ending_before` (ids; not both) | `200` + `list` envelope (`list_pages_forward_and_backward_with_cursors`) |
| POST | `/v1/payment_intents/{id}/confirm` | `payment_method_data[type]`, `payment_method_data[<type>][msisdn]` (push), `return_url` (redirect) | **`501 not_implemented`** — the adapter is reached and refuses (`confirm_reaches_the_adapter_and_renders_the_documented_501`) |
| POST | `/v1/payment_intents/{id}/cancel` | | `200` + `payment_intent` in `canceled`, or `409 invalid_state` (`cancel_is_legal_only_from_requires_payment_method`, `a_confirmed_intent_cannot_be_canceled`) |

**`confirm` never succeeds, on purpose.** It performs every write the
lifecycle requires — a `submitting` charge row committed with its
`provider_reference_id`, then a `provider_requests` row with no status —
*before* calling the adapter, and the adapter answers
`ProviderError::NotImplemented`, which is a `501`. The intent stays
`requires_payment_method`; a second confirm is `409` because the first one's
charge row is still there (`a_second_confirm_cannot_produce_a_second_charge`);
and a cancel after it is `409` too, because the rail may hold a live payment.
No rail has ever been called. See [../flows/crash-safety.md](../flows/crash-safety.md).

`next_action` is `null` on every intent this deployment can produce (only a
successful redirect `submit` writes the `redirect_url` it is derived from),
and a redirect confirm's `return_url` is validated and then dropped — there
is no column for it yet.

### Bodies, and the `Idempotency-Key` header

Request bodies are `application/x-www-form-urlencoded`, bracket-nested
Stripe-style, decoded by `vpay_api::form` (both `k[0]=v` and `k[]=v` array
spellings; `both_array_spellings_produce_the_same_array`). A JSON body is
refused with a 400 telling the caller to send a form
(`a_json_body_is_told_to_send_a_form`). Bodies over **64 KiB** are refused by
a layer on the whole `/v1` nest (`a_body_over_the_limit_is_refused_by_the_layer`).

**`Idempotency-Key` is required on every `POST`** — not optional as it is at
Stripe. A request without one is `400`
`invalid_request_error`/`invalid_request` naming `idempotency_key`
(`a_post_without_an_idempotency_key_is_the_documented_400`). Keys are 1–255
printable-ASCII bytes and live for 24 hours.

| Situation | Answer | Test |
|---|---|---|
| Same key, same body, first call finished | the **stored response, byte for byte** | `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` |
| Same key, different body | `400` `idempotency_error`/`idempotency_key_in_use` | `a_reused_key_with_a_different_body_is_the_400_envelope` |
| Same key, first call still running | `400` `idempotency_error`/`idempotency_key_in_flight` | `a_key_whose_first_request_is_still_running_is_answered_with_its_own_code` |
| First call answered `5xx` | key released; the retry re-executes | `a_5xx_releases_its_idempotency_key_so_the_retry_re_executes` |
| Deployment changed under a replay | the replay still answers what the original did | `a_replay_survives_the_rail_being_disabled` |

**Stripe answers `409` for a key still in flight; vpay answers `400`.**
[ADR-0011](../adr/0011-error-modelling.md) derives the status from the error's
`Category`, and `Category::Idempotency` is `400`/`idempotency_error` for both
idempotency codes. Splitting one `type` across two statuses is an ADR-level
change and is left as a maintainer decision; the `code` is what an SDK should
branch on. Recorded in `ApiError::IdempotencyKeyInFlight`'s own doc comment.

### Error codes a `/v1` caller can actually receive

`invalid_request` (400), `idempotency_key_in_use` (400),
`idempotency_key_in_flight` (400), `invalid_token` /`missing_bearer_token`
/`malformed_authorization_header` (401), `forbidden` (403),
`resource_missing` (404), `unknown_route` (404), `invalid_state` (409),
`resource_conflict` (409, a database uniqueness refusal),
`not_implemented` (501), `service_unavailable` (503), `internal_error` (500).
Every one is derived from a `Category`; see
[../flows/errors.md](../flows/errors.md).

### Not served — the honest 404 stands

| Method | Path | Why |
|---|---|---|
| POST | `/v1/refunds` | no refunds repository, no handler; migration `0017` is the schema only |
| GET | `/v1/events` | nothing emits events; migration `0018` is the schema only |
| GET | `/v1/balance` | no ledger read path |

Both SDKs can call all three. Each returns the `404` envelope to an
authenticated caller, because a `200` would mean someone invented a resource.

See [../status.md](../status.md) for the tests behind each claim above and
for the state of the evidence.

**Client-side, two SDKs implement this surface** — [`sdks/rust`](../../sdks/rust)
(`vpay-sdk`) and [`sdks/nodejs`](../../sdks/nodejs) (`@vpay/sdk`). They do
the `private_key_jwt` handshake, token caching, the form-encoded resource
calls in the table below and webhook verification. The exact wire contract
they implement, and the server must serve, is
[../flows/merchant-auth.md](../flows/merchant-auth.md). They are tested
against HTTP stubs of that contract and against the real Authkestra
assertion verifier. Since 2026-09-02 the **Rust** SDK completes the whole
handshake against a real `vpay_api::router` in
`backends/tests/integration/tests/merchant_token_flow.rs`, and since
2026-09-03 it also drives the five served payment-intent methods above
against a real router and a real Postgres in
`backends/tests/integration/tests/payment_intents.rs`. Three of its eight
resource methods still have no route to call. The **Node** SDK has still
never spoken to a vpay of any kind.

Everything not listed under "Served today" returns a Stripe-shaped 404 naming
vpay honestly, rather than pretending the route exists.

## `/dash/v1` — the dashboard API

Authenticated with **OIDC sessions** (authorization code + PKCE), never a
merchant credential of any kind. Called server-side from Next.js only.

Keeping these separate from `/v1` is deliberate, even though both surfaces
are OAuth2-shaped now: a merchant's `client_credentials` token authenticates
a *backend service* with standing payment authority over its own account,
while a dashboard session authenticates a *human*, scoped to one read-only
capability, for as long as they stay logged in. Conflating the two would let
a browser-held credential reach payment-authority endpoints, or a merchant
integration reach staff-only state. See
[ADR-0008](../adr/0008-dashboard-scope.md) and
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md).

**Status: not implemented.** No `/dash/v1` route, no `/login`, no
`/authorize`, no session store — `authkestra-engine` is pinned without its
`sql-postgres` feature, so none is compiled in. The signing keys and the
JWKS endpoint that landed on 2026-09-02 serve `/v1` and are not a step
toward this surface being reachable; there is also an unresolved audience
problem in the authorization-code grant that must be settled first. Tracked
as Phase 2b in [../roadmap.md](../roadmap.md); see
[../flows/dashboard-auth.md](../flows/dashboard-auth.md).

## `/provider/{code}/callback` — rail callbacks

Public and unauthenticated by necessity. A callback **never changes state** — it
only enqueues a status query. See [../flows/reconciler.md](../flows/reconciler.md).

**Status: not implemented.**
