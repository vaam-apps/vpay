<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. §4 below is written so it can be applied verbatim. -->

# Step 9, lane 5b — the client-assertion audience

Branch `claude/step9-lane-5b-audience`, on top of `6abbaa0` (the gate). Five
commits including this note. Fixes a real defect lane 6 found: **no merchant
server that reaches vpay by an internal URL could authenticate at all.**

## 1. The mechanism

Three strings were two.

| The string | What it is | Who compares it |
|---|---|---|
| The **token endpoint** | Where the merchant's process POSTs the token request. Must resolve from *there* — a compose service name, a private DNS name, a mesh address. | Nobody. It is a routing fact; the request either arrives or it does not. |
| The **assertion audience** (`aud`) | What the OP calls *itself*. | `authkestra_op`'s `authenticate_client`, against `expected_audiences` = `[{deployment.public_base_url}/v1/oauth/token, {deployment.public_base_url}/v1/oauth]` (`vpay_api::op::issuer_for`). Nothing else is in that list. |
| The **`audience` request parameter** | `vpay:v1` — the *resource server* the minted token is for. | The merchant registration's `allowed_audiences`, and later `vpay_api::resource_auth::Surface::Merchant::audience()`. |

Both SDKs signed `aud` as the token endpoint, i.e. as the URL they happened to
POST to. That is correct — and only correct — when the merchant reaches vpay
at the same URL vpay publishes as its own. It is wrong for every merchant
behind an internal name, and the symptom carries no hint at the cause: the
signature verifies, `iss`/`sub` are the client id, the `kid` selects, the
lifetime is in range, and the answer is a bare `invalid_client` /
`InvalidAudience`.

The demo stack is exactly that shape. `vpay-shop` reaches vpay at
`http://vpay-server:8080` (the compose service), while the generated overlay's
`deployment.public_base_url` is `http://localhost:{{demo_port}}` (the
published host port, because that is what a payer's browser has to reach).
So the shop's `aud` was `http://vpay-server:8080/v1/oauth/token` and the OP
was looking for `http://localhost:8080/v1/oauth/token`.

**The fix is a third setting, not a redefinition of either existing one.**

- `@vpay/sdk`: `assertionAudience?: string`, defaulting to `tokenEndpoint`.
- `sdks/rust`: `ClientBuilder::assertion_audience(..)`, same default.
- `examples/shop`: `VPAY_OAUTH_AUDIENCE` (optional), forwarded to the client
  only when set.
- `compose.e2e.yml` / `compose.demo.yml`: set on `vpay-shop`.

**Not named `audience` in either SDK.** That name is already taken, in both,
by the OAuth2 `audience` *request parameter* (`vpay:v1`). Reusing it would
have made the assertion's `aud` default to `vpay:v1` — a different bug — and
would have broken the existing tests that pin the two apart
(`keeps aud on the token endpoint even when audience is overridden`,
`the_assertion_audience_follows_an_overridden_token_endpoint`). The shop's
environment variable is `VPAY_OAUTH_AUDIENCE` as briefed.

## 2. What landed

| # | Thing | Where |
|---|---|---|
| 1 | `assertionAudience` option; `TokenManagerOptions` carries it; `#fetchToken` signs it | `sdks/nodejs/src/auth.ts` |
| 2 | Five tests: default unchanged, explicit audience signed without moving the request, issuer form, and the verifier-shaped refused/accepted pair | `sdks/nodejs/src/client.test.ts` |
| 3 | `ClientBuilder::assertion_audience`, `Inner::assertion_audience`, both hand-written `Debug`s | `sdks/rust/src/client.rs` |
| 4 | Three builder unit tests, two wire tests, three real-verifier conformance tests | `sdks/rust/src/client.rs`, `tests/token_exchange.rs`, `tests/op_conformance.rs` |
| 5 | `percent_decode` moved into the shared test support so `op_conformance` needs no second copy | `sdks/rust/tests/support/mod.rs` |
| 6 | One parity row, ✅/✅, naming all thirteen proving tests | `docs/sdks/parity.md` |
| 7 | `VPAY_OAUTH_AUDIENCE` read (optional, verbatim, blank = unset) and forwarded | `examples/shop/src/server/config.ts`, `src/server/vpay.ts` |
| 8 | Eight shop tests, config half and wire half | `examples/shop/src/server/vpay.test.ts` (new) |
| 9 | Documented with the compose case as the worked example | `examples/shop/.env.example`, `examples/shop/README.md`, both SDK READMEs |
| 10 | `VPAY_OAUTH_AUDIENCE` on `vpay-shop`, interpolated with the demo port in the overlay | `compose.e2e.yml`, `compose.demo.yml` |

## 3. The guard-failure proof

The brief asked for one specific demonstration: with the shop's audience unset
and `VPAY_API_URL` an internal name, the verifier-shaped test **fails**; set,
it **passes**. It is in the suite as a pair rather than as a manual step,
because a manual step is not a gate.

`examples/shop/src/server/vpay.test.ts` drives the real `VpayClient` against a
real `127.0.0.1:<ephemeral>` server — an address reachable only from that
process, which is what "an internal name" means here — and reads the
assertion off the wire. `verifyAsTheOpWould` reproduces `authenticate_client`'s
`expected_audiences` comparison against `http://localhost:8080/v1/oauth/token`
and `http://localhost:8080/v1/oauth`.

```
✓ createVpayClient > reaches vpay at VPAY_API_URL and signs that URL when no audience is configured
✓ a shop that reaches vpay by an internal name > is refused by the OP audience check with VPAY_OAUTH_AUDIENCE unset
    → { ok: false, reason: "InvalidAudience" }
✓ a shop that reaches vpay by an internal name > authenticates once VPAY_OAUTH_AUDIENCE names vpay's own token endpoint
    → { ok: true }
```

And the tests are decisive, not decorative — each mutation was run, not
reasoned about:

| Mutation | Result |
|---|---|
| Drop the `assertionAudience` forwarding from `examples/shop/src/server/vpay.ts` | 3 of 8 fail, including "authenticates once VPAY_OAUTH_AUDIENCE names vpay's own token endpoint" |
| Pin `assertionAudience` to `tokenEndpoint` in `resolveMerchantAuth` (Node) | 3 of 168 fail |
| Pin `assertion_audience` to the token endpoint in `ClientBuilder::build` (Rust) | 4 of 132 fail, **two of them in `op_conformance`** — i.e. the real pinned `authkestra_op` verifier is the thing that refuses |
| Misspell one test name in the new parity row | `verify-sdk-parity` fails naming the cell |

The Rust conformance pair is the strongest evidence here, because nothing
about it is this repository's opinion of what the OP wants: a `Client` built
against a 127.0.0.1 stub is refused by
`authkestra_op::client_assertion::verify_client_assertion` at the pinned
`=0.7.1` with its audience unset, and accepted by the same call, with the same
registration and the same key, once `assertion_audience` names the OP.

## 4. For lane E — verbatim text

This is an ADR-0010-adjacent fact about `docs/flows/merchant-auth.md`. **The
decision in ADR-0010 does not change**; nothing here is an amendment to it.

### 4a. For `docs/flows/merchant-auth.md`, in "The client assertion"

> **`aud` is the OP's own name for itself, not the URL you POST to.**
> `authenticate_client` compares the claim against exactly two strings —
> `{deployment.public_base_url}/v1/oauth/token` and the
> `{deployment.public_base_url}/v1/oauth` issuer (`vpay_api::op::issuer_for`)
> — and against nothing else. A merchant whose server reaches vpay by an
> internal URL (a compose service name, a private DNS name, a mesh address)
> must say so: `assertionAudience` in `@vpay/sdk`,
> `ClientBuilder::assertion_audience` in `sdks/rust`. Both default to the
> token endpoint, which is right only when the two coincide. Left wrong, every
> token request answers `invalid_client` / `InvalidAudience` while the
> signature, the `client_id`, the `kid` and the lifetime are all correct — the
> response says nothing about audiences, so this is not a failure a merchant
> diagnoses from the wire.

### 4b. For that file's **Status** section

> Both SDKs separate the assertion's `aud` from the URL the token request is
> POSTed to (2026-09-04, Step 9 lane 5b). Proven by
> `sdks/rust/tests/op_conformance.rs`'s
> `the_real_verifier_refuses_a_client_that_reaches_vpay_internally_and_sets_no_audience`
> and `the_real_verifier_accepts_the_same_client_once_assertion_audience_is_set`,
> which run a real `Client`'s assertion through the real pinned
> `authkestra_op` verifier, and by `examples/shop/src/server/vpay.test.ts` on
> the Node side. `docs/sdks/parity.md` records the capability ✅/✅.

### 4c. For `docs/status.md`, wherever the shop's environment is described

> `examples/shop` reads an optional `VPAY_OAUTH_AUDIENCE` and both compose
> files set it on `vpay-shop`. It is the demo stack's only correct value for
> the assertion's `aud`: the shop reaches vpay at `http://vpay-server:8080`
> while the generated overlay's `deployment.public_base_url` is the published
> host port. Still unproven end to end — no vpay serves a token endpoint (see
> this file's own OP entry), so nothing has yet exchanged a token against a
> running server.

## 5. What this lane did NOT do

- **No token has been exchanged against a real vpay.** Nothing in this lane
  ran the demo stack. The compose values are correct against the overlay
  `gen-demo-keys` writes, read from the recipe, not observed in a running
  container. The proof this lane does have is the OP's own verifier code
  linked into a test — which is one step short of, and not a substitute for,
  a live `/v1/oauth/token` round trip.
- **`docs/flows/merchant-auth.md`, `docs/status.md` and `docs/roadmap.md` are
  untouched**, per the brief. §4 above is lane E's to apply.
- **No backend Rust, no checkout app, no `justfile`, no helm.** In particular
  `just gen-demo-keys` was not taught anything about this value; the compose
  files carry it as a literal (e2e) and an interpolation of `VPAY_DEMO_PORT`
  (demo), matching how the two files already spell the port. If a future
  change makes `deployment.public_base_url` configurable independently of
  `demo_port`, these two strings drift apart with no check to catch it.
- **The Node side still has no real-verifier conformance test.** Its new
  audience test is verifier-*shaped*: it reproduces the OP's comparison in
  TypeScript. That is the same standing gap `docs/sdks/parity.md` already
  records for `sdks/nodejs` ("Node cannot link the Rust verifier",
  2026-09-03); this lane did not close it and did not widen it.
- **`stripe-compat` was not exercised.** `createStripeAuthenticator` shares
  `resolveMerchantAuth`, so it inherits the option, and its 21 tests still
  pass — but no test passes `assertionAudience` through that entry point.
