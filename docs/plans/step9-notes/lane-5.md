# Step 9, lane 5 — the SDKs and parity

Branch `claude/step9-lane-5-sdks`, off `f186d67` (master + the Step 9 plan).
Owns `sdks/stripe-js`, `sdks/nodejs`, `sdks/rust` and `docs/sdks/parity.md`.
Touched nothing under `backends/`, `frontends/`, `docs/status.md` or
`docs/flows/` — the verbatim replacements those two need are below, for lane
E to apply.

## What landed

Three commits, one per SDK, plus this record.

| Commit | What |
|---|---|
| `2dce84e` | `@vpay/stripe-js`: `initEmbeddedCheckout`, `retrieveCheckoutSession`, the session routes on the browser stub, the README retraction |
| `de9e480` | `sdks/nodejs`: `client.checkout.sessions.{create,retrieve,list,expire}` |
| `547eeb6` | `sdks/rust`: `client.checkout().sessions()`, `CheckoutSession` with a redacting `Debug` |
| `e5e59f3` | `docs/sdks/parity.md` rows, and this note |
| _this_ | `payment_intent` expanded on the browser read — the integrator's 2026-09-04 ruling (see item 1 below) |

### Counts

| Suite | Before | After | Ignored / skipped |
|---|---|---|---|
| `pnpm --filter @vpay/stripe-js test` | 87 | **119** | 0 |
| `pnpm --filter @vpay/sdk test` | 149 | **163** | 0 |
| `cargo nextest run -p vpay-sdk` | 113 | **124** | 0 |
| `cargo xtask verify-sdk-parity` | 267 proving tests, 24 gaps | **322 proving tests, 26 gaps** | — |
| `cargo test --doc -p vpay-sdk` | 3 pass, 1 ignored | 3 pass, 1 ignored | the ignored one is the pre-existing ```` ```rust,ignore ```` block under the Rust README's "Errors" |

## The guard-failure proofs

Both were applied, run, observed to fail, and restored. Neither weakened
version is in any commit.

**1. Remove the origin check → the wrong-origin test fails.**
Changed `if (event.origin !== allowedOrigin)` to
`if (false && event.origin !== allowedOrigin)` in
`sdks/stripe-js/src/embedded.ts`, then
`pnpm --filter @vpay/stripe-js test src/embedded.test.ts`:

```
× initEmbeddedCheckout > ignores a message from an origin that is not the checkout app  12ms
  Tests  1 failed | 19 passed (20)
```

Exactly one test failed, and it is the one that exists for the purpose. With
the check restored: `Tests 20 passed (20)`.

**2. Rename one proving test → `verify-sdk-parity` exits 1 naming the row.**
Renamed `expire_checkout_session_posts_an_empty_body_and_still_carries_an_idempotency_key`
in `sdks/rust/tests/resources.rs`, then `cargo xtask verify-sdk-parity`:

```
xtask: sdk parity violations:
  - docs/sdks/parity.md:95 ``checkout.sessions.expire` — empty-bodied POST, still idempotency-keyed` / sdks/rust: names the test `expire_checkout_session_posts_an_empty_body_and_still_carries_an_idempotency_key`, which does not exist under `sdks/rust` (…)
exit=1
```

Restored, and `git diff --stat sdks/rust` is empty.

**3. A leak the tests found, rather than one they were written around.**
The first version of the Node redaction hid `client_secret` and printed
`url` verbatim — and a hosted session's `url` carries the same secret in its
fragment (D6), so `console.log(session)` still leaked it. The test failed on
the `url` line. Both SDKs now redact the fragment as well as the field, and
each has a case pinning it.

## Decisions taken in this lane

Each is inside the lane's own files, but two of them touch what other lanes
must send.

- **D8-a — the parent posts nothing into the frame.** The protocol needed no
  handshake: the child learns its framer's origin from the CSP
  `frame-ancestors` vpay serves it (D4), not from a message. The strongest
  form of "never `postMessage(…, '*')`" is therefore "never `postMessage`",
  and a source scan pins it (`client.test.ts`, beside the `console` ban).
- **D8-b — the frame is sandboxed
  `allow-scripts allow-same-origin allow-forms`.** `allow-same-origin` is
  required, not a relaxation: without it the page runs in an opaque origin
  and its `/v1/browser` requests carry `Origin: null`, which the CORS layer
  refuses. What the list withholds is `allow-top-navigation` — so the framed
  page *cannot* navigate the merchant's tab, which is what makes
  `vpay:redirect` a necessity rather than a convention. **Lane 3 must not
  need popups, downloads or top-navigation from inside the frame.** If it
  does, this attribute is the thing to change, and changing it should be a
  decision rather than a debugging step.
- **The message handler also checks `event.source === frame.contentWindow`.**
  The origin check alone lets two embedded checkouts on one merchant page
  resize and complete each other. Both checks have their own test.
- **The frame starts at `height: 0px`.** The page owns its height and sends
  `vpay:resize`; a height this package guessed would be an iframe silently
  stuck at the wrong size, which is harder to diagnose than one that renders
  nothing. **Lane 3 must send a `vpay:resize` on first paint**, or an
  embedded checkout renders as an empty box.
- **`vpay:complete` is read strictly.** `session` and `status` must both be
  strings, or the message is ignored and `onComplete` does not fire. A
  callback fired with a half-understood payload is worse than one that did
  not fire. See "For lane 1 and lane 3 to confirm" below.
- **~~`retrieveCheckoutSession` surfaces the session object only.~~
  Overridden by the integrator, 2026-09-04.** This lane had modelled the
  session alone, on the reasoning that a merchant's outer page has no use
  for a payer's confirm credential. The ruling is that the browser read
  expands `payment_intent`, so the intent — `client_secret` and all — is
  part of the object this package returns. The merchant's display name is
  still not modelled. The exposure that reasoning was avoiding is real and
  is now documented on the type and in the README instead of designed away.
- **`Debug`/`util.inspect` redact; `Serialize`/`JSON.stringify` do not.**
  The same split `PaymentIntent` already has in Rust. An embedded
  integration must serialise the session secret to get it to the browser at
  all, so a redacting `toJSON` would break the one thing the field exists
  for.
- **The Node `CheckoutSession` redacts, though the Node `PaymentIntent`
  still does not.** ADR-0015's matrix records that as a dated gap
  (`client_secret` is not redacted from `PaymentIntent` diagnostics,
  `sdks/nodejs`, 2026-09-03). This lane did not close it — that is
  `PaymentIntent`'s own follow-up and would change a shape merchants already
  consume — but it did not repeat it for a new object either.

## For lane 1 and lane 3 to confirm

Three places where the plan's wire contract was readable two ways. The first
has since been ruled on by the integrator and is settled; the other two were
resolved by taking the plan's own JSON literally, and each is cheap to change
if lane 1 or lane 3 lands something else.

1. **`payment_intent` on the browser session read — SETTLED 2026-09-04.**
   The plan's session JSON says `"payment_intent": "pi_…"` (an id) while the
   browser route is described as "the session **plus** its intent with the
   intent's `client_secret`", which left it open whether the field is
   expanded on that route or the intent arrives under another key. The
   integrator ruled: **on the browser routes `payment_intent` is the
   expanded intent object** — every `PaymentIntent` field, with
   `client_secret` present on the session read and absent on the return
   read — and **on `/v1` it stays the `pi_…` string**. Applied in the commit
   `fix(sdks): payment_intent is the expanded intent on the browser session
   read` (named rather than hashed, because it carries this note and so
   cannot cite its own hash): `@vpay/stripe-js`'s
   `CheckoutSession.payment_intent` is now `PaymentIntent`, both merchant
   SDKs keep `String`/`string` with the divergence named on the field
   itself, and the browser stub renders the expansion.

   The consequence worth carrying forward, because it widens what a merchant
   page holds: `retrieveCheckoutSession` now returns a live **confirm**
   credential (`session.payment_intent.client_secret`), not only a
   session-read one. The type documents it, the README says it in bold, and
   a test asserts the two secrets are different values with different
   authority. This package still wraps no wire object, so there is no
   `CheckoutSession` inspect hook to redact through the way the merchant
   SDKs have one — what holds for both credentials equally is that the
   `Stripe` object retains neither and no error it builds quotes either,
   and that is what the new test pins.
2. **`vpay:complete`'s `session` member.** D8 writes
   `{type:'vpay:complete', session, status}` without saying whether `session`
   is the `cs_…` or the whole object. This lane reads it as the id (a string)
   and ignores a message where it is not. **If lane 3 sends the object,
   `onComplete` will not fire.**
3. **The uniform 404's message for a session.** The stub renders
   `No such checkout session: {id}`, by analogy with the intent's
   `No such payment intent: {id}`. The *shape* is what the tests assert
   (`invalid_request_error` / `resource_missing`, no `param`); the message
   text appears in a fixture only.

## What this lane did NOT do

- **Nothing has run against a real vpay.** Every server in these suites is
  `wiremock`, a `node:http` stub, or a jsdom `iframe` with an `src` nothing
  fetches. `/v1/checkout/sessions`, `/v1/browser/checkout/sessions/{id}` and
  the checkout app are lanes 1 and 3; the end-to-end proof is lane 6. Two
  ⛔ rows were added to `docs/sdks/parity.md` saying exactly this, dated
  2026-09-04.
- **No hosted-mode helper.** Hosted checkout needs no browser SDK: the
  merchant's server reads `session.url` and redirects. Adding a
  `redirectToCheckout()` would have been a method whose whole body is
  `location.assign`.
- **`checkout.origins` is not called from any SDK.**
  `GET /v1/browser/checkout/origins?key` exists for the checkout app's own
  middleware (D4); framing it in a merchant SDK would suggest a merchant
  should read it, which is not what it is for.
- **The `PaymentIntent` diagnostics gap in `sdks/nodejs` is still open**, as
  are the other 23 gaps ADR-0015 adopted with.
- **No `docs/status.md`, `docs/flows/*` or `docs/roadmap.md` edit** — lane
  E's, from the text below.

## Verbatim text for lane E

### For `docs/status.md`

Rows for the SDK half of Step 9. Drop into whichever table carries the SDK
rows, adjusting only the surrounding column formatting:

```markdown
| Checkout Sessions in the merchant SDKs | ✅ | `sdks/rust` `client.checkout().sessions()` and `sdks/nodejs` `client.checkout.sessions` both offer `create`/`retrieve`/`list`/`expire`, landed in one PR per ADR-0015. Proven against `wiremock` and a `node:http` stub, byte for byte, with the `create` bodies pinned as identical literals on both sides. **Not yet run against a real server** — `/v1/checkout/sessions` is Step 9 lane 1's, and `backends/tests/integration/tests/checkout_sessions.rs` is what will prove it. |
| Embedded Checkout in `@vpay/stripe-js` | ✅ | `initEmbeddedCheckout({ fetchClientSecret })` frames `{checkoutBaseUrl}/e/{cs_id}?key={pk}#{client_secret}` and acts only on `message` events whose origin is the checkout app's and whose source is that frame; `retrieveCheckoutSession` reads the browser session route (where `payment_intent` is the expanded intent, per the integrator's 2026-09-04 ruling — so it returns a live confirm credential as well as a session-read one) and never rejects. 20 jsdom tests against a real `<iframe>`. **Never run against vpay's checkout app** — that is Step 9 lanes 3 and 6. |
| A Checkout Session's secret in diagnostics | ✅ | Both merchant SDKs redact `client_secret` **and the fragment of a hosted session's `url`, which carries the same value**, from `Debug`/`util.inspect`, while leaving `Serialize`/`JSON.stringify` faithful. The equivalent gap for `PaymentIntent` in `sdks/nodejs` is still open (`docs/sdks/parity.md`, 2026-09-03). |
```

And, for whichever line summarises the parity check:

```markdown
`cargo xtask verify-sdk-parity` reads `docs/sdks/parity.md` on every
`just verify`: 322 named proving tests all exist, 26 dated gaps.
```

### For `docs/flows/stripe-sdk-compat.md`'s **Status** section

Append:

```markdown
**2026-09-04 (Step 9).** `@vpay/stripe-js`'s README no longer lists
"Checkout (hosted or embedded)" under "Not compatible, ever". The retraction
is narrower than the removal looks and is worded that way in the README:
vpay now serves its **own** checkout page, hosted and embedded, and
`initEmbeddedCheckout`/`retrieveCheckoutSession` speak to it. It is not
`@stripe/stripe-js`'s Checkout — Stripe's own method is
`createEmbeddedCheckoutPage` in the pinned 9.15.0, its options are not ours
in either direction, and vpay's `checkout.session` has no `line_items`,
`mode` or `amount_total` (Step 9's D10). What *is* portable, and is pinned as
a compile-time assertion in both directions in `src/compat.test.ts`, is the
handle: `{ mount(string | HTMLElement), unmount(), destroy() }` is
assignable to and from Stripe's `StripeEmbeddedCheckout`. The mounting
plumbing moves; the session model does not. `sdks/stripe-compat` gains no
row for any of this — D10 says the Checkout Session is evidence-free of a
Stripe promise, and the compat suite proves claims rather than making them.
```

### For `docs/flows/browser-checkout.md`

Nothing from this lane. D4's retirement and the hosted/embedded section are
lanes 2 and 3's material.

## Gate

Run in this worktree, `CARGO_BUILD_JOBS=4`, no Docker.

| Command | Result |
|---|---|
| `pnpm install --frozen-lockfile` | ok (the lockfile gained `jsdom ^25.0.1` under `sdks/stripe-js`, already resolved at 25.0.1 for `frontends/packages/ui`) |
| `pnpm --filter @vpay/stripe-js test` | 119 passed, 8 files |
| `pnpm --filter @vpay/sdk test` | 163 passed, 9 files |
| `pnpm -r typecheck` | ok |
| `just lint-web` | ok |
| `cargo nextest run -p vpay-sdk` | 124 passed, 0 skipped |
| `cargo clippy -p vpay-sdk --all-targets -- -D warnings` | ok |
| `cargo fmt --all --check` | ok |
| `cargo xtask verify-sdk-parity` | 322 proving tests, 26 dated gaps |
| `just verify` | ok |
| `just test-doc` | 3 passed, 1 ignored (pre-existing) |
