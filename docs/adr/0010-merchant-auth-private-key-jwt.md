# ADR-0010: `/v1` authenticates merchants with OAuth2 `client_credentials` + `private_key_jwt`, not API keys

- **Status:** Accepted
- **Date:** 2026-08-09
- **Deciders:** vpay maintainers

## Context

ADR-0009 scoped `/v1` to keep Stripe-shaped opaque `sk_live_`/`sk_test_`
bearer keys, reasoning that Authkestra has no opaque-API-key primitive and
that routing merchant auth through an OP would reimport ADR-0008's
revocation problem. That plan went on to become schema:
`backends/migrations/0008_create-merchant-api-keys.sql` (`merchant_api_keys`
— a SHA-256 key digest, a revocation flag, and constraints proven to fire in
`postgres_smoke.rs`; see `docs/status.md`).

That plan is reversed by this ADR — not on taste, but because the design it
was reaching for is not buildable on top of Authkestra as published.
`authkestra_op::sqlx_store::SqlxOpStore::find_client` hardcodes
`token_endpoint_auth_method: None` and `jwks: None` on every row it returns.
The crate's own comment at `authkestra-op-0.3.4/src/sqlx_store.rs:93-102`
says why: those columns aren't persisted yet, and until they are, "a
SqlxOpStore-backed client … cannot use `private_key_jwt`." A merchant client
registry backed by the OP's own database store therefore cannot serve
`private_key_jwt` at all, at this pinned version — the option was never
actually on the table. A YAML-configured registry (ADR-0003) has no such
gap: the JWK is just a config value.

Separately, the maintainer decided the device-authorization grant serves no
caller here — every `/v1` caller is a merchant's own backend, not a
human on an input-constrained device — and that `/v1` tokens should not carry
a refresh token.

## Decision

`/v1` requests authenticate via OAuth2 `client_credentials` (RFC 6749 §4.4)
using `private_key_jwt` client authentication (RFC 7523). Concretely:

- Each merchant is a statically registered OAuth2 client. Its `client_id`,
  allowed grant type, and **public** JWK live in vpay's YAML configuration
  ([ADR-0003](0003-yaml-configuration.md)), loaded at boot like everything
  else administrative. vpay never stores a merchant secret, in any form, in
  any table.
- No API key of any shape is accepted on `/v1` — not `sk_live_`/`sk_test_`,
  not any other bearer token format. `merchant_api_keys`
  (`backends/migrations/0008_create-merchant-api-keys.sql`) is no longer the
  credential store this surface uses; see Consequences.
- The device-authorization grant is not offered to `/v1` clients. Authkestra
  ships it for unattended, input-constrained devices, which describes no
  caller this API has.
- vpay issues **no refresh token** for this grant. A client re-authenticates
  with a freshly signed assertion instead of refreshing. This matches RFC
  6749 §4.4.3 ("the authorization server SHOULD NOT issue a refresh token")
  and `authkestra-op`'s own `client_credentials` handler, which already
  hardcodes `refresh_token: None` (`handlers/token.rs:816` in the pinned
  `=0.3.4`).
- A `disabled_clients` table supplements the YAML config as an operational
  kill switch: an operator flips a client to disabled and it takes effect
  immediately, no deploy required. It stores a `client_id` and a
  disabled flag/reason — never a credential. YAML stays authoritative for
  *identity* (does this client exist, what is its key); the table only ever
  *subtracts* access, never grants it. Its migration and any code that reads
  or writes it are outside this document's scope (`docs/adr/**`, not
  `backends/migrations/**`) — see `docs/status.md` for whether either
  exists yet.

**This supersedes the scope-boundary paragraph of
[ADR-0009](0009-dashboard-oidc-provider.md)** that kept `/v1` on
Stripe-shaped API keys ("`/v1`, the merchant API, keeps Stripe-shaped opaque
`sk_live_`/`sk_test_` bearer keys and does **not** move to Authkestra").
Nothing else in ADR-0009 changes: vpay is still its own OP for `/dash/v1`,
using authorization-code + PKCE for staff login — a separate client
registration, a separate grant, and a separate token audience from what this
ADR describes for `/v1`.

## Consequences

**No Stripe SDK can authenticate against vpay.** This is the big one, and it
narrows a claim the project has made about itself. Stripe's official SDKs
send their API key as a static `Authorization: Bearer <key>` value; none of
them implement RFC 7523 client-assertion signing or an OAuth2 token-endpoint
round trip. A merchant using an official Stripe SDK now needs custom glue
code around it to authenticate — the SDK is still Stripe-shaped for the
object model and idempotency semantics, not for this. `README.md` and
`examples/merchant-node/` have been corrected to say so plainly rather than
implying a drop-in that no longer exists on this one axis.
**[Amended 2026-09-03 — the bolded sentence above is too strong as written.
See "Amendment" at the end of this document. The decision itself is
unchanged.]**

**Revocation via config is a deploy**, which is why the `disabled_clients`
kill switch exists above. But that switch means YAML is no longer the *sole*
authority on "is this client allowed right now" — a correct answer needs
both: does the client exist in YAML, and has it since been disabled in the
database. Any future revocation runbook must say so explicitly and check
both, not just deploy history.

**`private_key_jwt` needs replay protection vpay must build.**
`authkestra-op` ships exactly two `ClientAssertionStore` implementations:
`NoClientAssertionStore`, the crate's own default, which fails closed and
refuses every assertion outright; and `MemoryClientAssertionStore`, a
single-process `Mutex<HashMap>` that the crate's own doc comment calls
"correct for single-node and for tests, not a production cluster answer."
vpay runs multiple replicas. Neither is sufficient as shipped:
`NoClientAssertionStore` would refuse every merchant call, and
`MemoryClientAssertionStore` would let a captured assertion replay against
every replica except the one that already recorded its `jti`. vpay must
implement its own shared `ClientAssertionStore` (Postgres or equivalent)
before this flow can accept a single real request. Not started — no such
implementation exists anywhere in this repository.

**Merchant onboarding is a PR, not a self-serve flow.** A merchant generates
their own keypair, sends vpay the public JWK, it is reviewed and lands in
YAML, then deployed. There is no config hot-reload (ADR-0003: configuration
loads once, at boot), so a rolling deploy has a real window where old pods
and new pods disagree about the client list — a merchant's first call right
after their PR merges can hit a pod that does not know about them yet and
must retry.

**It is more secure than the design it replaces.** No shared secret exists
anywhere — not in a database, not in an environment variable, not on the
wire; the assertion is signed, not transmitted. A compromised vpay database
no longer hands an attacker the ability to impersonate a merchant, the way a
stolen row (even hashed) in `merchant_api_keys` would have contributed to.

**`backends/migrations/0008_create-merchant-api-keys.sql` is now orphaned
schema relative to this decision.** Editing or removing it is outside this
document's scope (`docs/adr/**`, not `backends/migrations/**`), but a reader
must not infer from its continued presence in the repository that `/v1`
will ever read it.

## Amendment, 2026-09-03: an official Stripe SDK *can* authenticate, with glue this repository now ships

**What this amendment does not change.** Every decision in this ADR stands:
`/v1` accepts no API key, merchants are statically registered OAuth2 clients
with a public JWK in YAML, `client_credentials` + `private_key_jwt` is the
only grant, there is no refresh token, and `disabled_clients` is the
revocation seam. The status stays **Accepted**.

**What it retracts.** The Consequences section opens with "**No Stripe SDK
can authenticate against vpay.**" That was written from a correct premise —
Stripe's SDKs send a static `Authorization: Bearer <key>` and none of them
implement RFC 7523 — and reached a conclusion one step too far. `stripe-node`
also accepts `config.authenticator`: arbitrary async code, invoked once per
request attempt, handed the whole outbound request. That is a seam this ADR's
author did not account for, and it is enough. The accurate sentence is the
one the same paragraph goes on to make: *a merchant using an official Stripe
SDK needs custom glue code around it to authenticate.* As of 2026-09-03 that
glue is not something each merchant writes — `@vpay/sdk/stripe` exports
`createStripeAuthenticator`, and `sdks/stripe-compat` drives the real
`stripe` package through it against a real vpay stack.

**Scope of the correction.** It is about *authentication only*. The object
model, the form encoding and the idempotency semantics were always
Stripe-shaped and are unaffected; the divergences that remain — no API keys,
no dated API version, no Connect, no `client_secret`, `payment_method_data`
carrying a rail code — are catalogued in
[`docs/flows/stripe-sdk-compat.md`](../flows/stripe-sdk-compat.md), which is
where a reader should go next.

**No Rust equivalent.** `async-stripe` has no per-request async hook, so the
same result there means wrapping its transport in custom middleware. Scoped
as a follow-up; `sdks/rust` remains the Rust path.

**Why an amendment and not a superseding ADR.** The retraction belongs where
the wrong claim is. A new ADR would be more visible in a list, and a reader
of 0010 alone would still be told something false — which is the failure
mode worth avoiding here.

## Amendment, 2026-09-03 (Step 5c): a second, deliberately different credential model for `/v1/browser`

**What this amendment does not change.** Every decision above still governs
`/v1` — the merchant surface. No API key of any shape is accepted there;
`client_credentials` + `private_key_jwt` remains the only grant; there is
still no refresh token; `disabled_clients` is still the revocation seam. The
status stays **Accepted**.

**What is new.** `/v1/browser` — two routes, `GET
/v1/browser/payment_intents/{id}` and `POST
/v1/browser/payment_intents/{id}/confirm` — authenticates a **payer's
browser**, which cannot hold a merchant credential of any kind and is not
this ADR's caller. It uses neither an API key nor `private_key_jwt`: a
**publishable key** (`pk_test_…`/`pk_live_…`,
`vpay_config::MerchantClient::publishable_keys`, D1 of
[`docs/plans/2026-09-03-step5c-stripejs.md`](../plans/2026-09-03-step5c-stripejs.md))
names the tenant and authorises nothing on its own, and a per-PaymentIntent
**`client_secret`** (160 bits from the OS CSPRNG, minted once at `create`)
authorises exactly that one intent, once. Full design, the credential model,
what proves it, and a real gap this step found and left unfixed (neither
merchant SDK's `PaymentIntent` type exposes `client_secret`, even though
`/v1`'s own `create`/`retrieve` now render one — see the next paragraph):
[`docs/flows/browser-checkout.md`](../flows/browser-checkout.md).

**A narrow retraction of the previous amendment's own list.** "Scope of the
correction" above still lists `client_secret` among the divergences from
Stripe that remain — that was accurate on 2026-09-02 and is no longer
accurate for two of `/v1`'s own methods: `create` and `retrieve` now render
`client_secret` (`vpay_api::model::PaymentIntentWithSecret`), because a
merchant's page needs it to hand to the payer's browser. It remains genuinely
absent from `confirm`, `cancel`, `list`, and every webhook body — see
[`docs/flows/stripe-sdk-compat.md`](../flows/stripe-sdk-compat.md), corrected
in the same commit as this amendment.

**Why not a merchant credential of any kind, not even a scoped-down one.** A
browser cannot keep a secret — anything sent to it is visible to the page's
own JavaScript, an extension, or the network tab. Handing a browser even a
narrowly-scoped OAuth token would mean vpay minting and then trusting a
credential it knows will sit in a context it does not control. The
publishable-key + `client_secret` pair sidesteps the problem instead of
scoping around it: neither value is a bearer credential, so there is nothing
to protect once it reaches the page — see "Every failure is the same 404" in
`docs/flows/browser-checkout.md` for the confidentiality property this
actually rests on.
