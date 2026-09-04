<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. §7 below is written so it can be applied verbatim. -->

# Step 9, lane 6 — the e2e proof

Branch `claude/step9-lane-6-e2e`, on top of the gate with lanes 1–5 and 7
merged. Two new Cypress specs, a fixture server, the two-pass runner
configuration, and the `test-e2e` recipe that brings up a stack they can pass
against.

**The headline is not the specs. It is what running them found:** the demo
shop could not authenticate against vpay at all from inside the compose
network. That is §2, and it blocked every line of this lane until lane 5b
fixed it.

## 1. What landed

| #   | Thing                                                                                                                                       | Where                                                                       |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| 1   | `shop-hosted.cy.ts` — three journeys through `examples/shop` to vpay's hosted page and back                                                 | `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts`                         |
| 2   | `shop-embedded.cy.ts` — four cases on the shop's `/orders/{id}/embedded` page, including the unregistered framer                            | `frontends/tests/e2e/cypress/e2e/shop-embedded.cy.ts`                       |
| 3   | The frame fixture: a page on an origin in nobody's `checkout_origins`, which frames a real embedded session                                 | `frontends/tests/e2e/cypress/tasks/frameFixtureServer.ts`                   |
| 4   | The shared reads both specs use — the stack's URLs, `orders.get`, the steering MSISDNs                                                      | `frontends/tests/e2e/cypress/support/shop.ts`                               |
| 5   | Two Cypress passes, because `chromeWebSecurity` and the frame-busting rewrites are launch-time and the two page modes need opposite answers | `frontends/tests/e2e/cypress.config.ts`, `frontends/tests/e2e/package.json` |
| 6   | `just test-e2e` brings up the demo overlay and generates the keys, as CI's `e2e` job has always done and this recipe never did              | `justfile`                                                                  |
| 7   | CI's `e2e` job waits for `vpay-checkout` and `vpay-shop`, and passes the four browser-facing URLs                                           | `.github/workflows/ci.yml`                                                  |

`checkout.cy.ts` and `dashboard.cy.ts` are untouched.

## 2. The defect this lane existed to find

`POST /api/trpc/orders.create` on the containerised shop, on a stack that was
otherwise green:

```console
$ curl -sS -X POST http://localhost:13001/api/trpc/orders.create \
    -H 'content-type: application/json' \
    -d '{"email":"probe@example.test","lines":[{"productId":"njangi-tote","quantity":1}],"mode":"hosted"}'
{"error":{"message":"invalid_client: Client authentication failed","code":-32603,
  "data":{"code":"INTERNAL_SERVER_ERROR","httpStatus":500,"path":"orders.create"}}}
```

```
vpay-server WARN authkestra_op::client_assertion
  "client assertion failed signature or claim validation"
  client_id=shop-merchant error=InvalidAudience
```

**Not the key, the registration, the scopes or the publishable key.** The same
PEM and the same `client_id`, driven from the host with
`baseUrl=http://localhost:18080`, created a PaymentIntent on the first try:

```
OK from the host: pi_2916sxymc52257f418myqca3 requires_payment_method
```

The mechanism, in one sentence: **`@vpay/sdk` signed the client assertion's
`aud` with the token endpoint it was about to POST to, and derived both from
`baseUrl`** (`sdks/nodejs/src/auth.ts`: `tokenEndpoint = options.tokenEndpoint
?? ${baseUrl}/v1/oauth/token`, then `audience: this.#options.tokenEndpoint`),
**while vpay's OP derives its issuer solely from
`deployment.public_base_url`** (`vpay_api::op::issuer_for`) and
`authenticate_client` accepts only that issuer or `{issuer}/token`. The compose
files give `vpay-shop` `VPAY_API_URL: http://vpay-server:8080`, the generated
overlay names `http://localhost:{demo_port}`, and those two strings can never
match while a merchant's server reaches vpay by its compose service name.
`sdks/rust` had the same conflation (`sdks/rust/src/auth.rs:207`).

Why it survived three lanes: lane 7 never spoke to a running vpay (its §9 says
so), lane 4 brought the shop up but never clicked through it (its §5: `orders`
was empty at the end of its green `just demo`), and `merchant-demo`,
`checkoutTasks.ts` and `stripe-compat` all run **on the host**, where the
public issuer happens to be right. `examples/shop/.env.example` ships
`VPAY_API_URL=http://localhost:8080`, so a host-run shop worked and only the
containerised one did not.

No configuration fixed it, and each candidate was checked rather than
dismissed: `allowed_audiences` is the access **token**'s `aud`, not the
assertion's (`vpay_config::oauth::MERCHANT_AUDIENCE`);
`deployment.public_base_url` cannot be a name that resolves both inside the
compose network and from the host on the same port; a container cannot reach
the host's `localhost:{demo_port}`.

It was reported rather than patched around. **Lane 5b** took it: both SDKs
gained an explicit client-assertion audience, the shop reads
`VPAY_OAUTH_AUDIENCE`, and both compose files set it on `vpay-shop`.

## 3. Two Cypress passes, and the measurements that forced them

`cypress.config.ts` now branches on `VPAY_E2E_FRAMED`. Three settings differ,
and none of them can be overridden per test — they are browser launch flags
and proxy behaviour, resolved once per `cypress run`.

|                                               | default pass | framed pass (`shop-embedded.cy.ts`) |
| --------------------------------------------- | ------------ | ----------------------------------- |
| `chromeWebSecurity`                           | `true`       | `false`                             |
| `modifyObstructiveCode`                       | `true`       | `false`                             |
| `experimentalModifyObstructiveThirdPartyCode` | `true`       | `false`                             |

**Why the rewrites, and why they flip.** Cypress renders the application under
test inside an iframe of its own, and by default rewrites `window.top` /
`window.parent` reads in proxied JavaScript to hand the reading window itself
back. vpay's checkout page reads exactly that property and draws opposite
conclusions in its two modes (`src/lib/entry.ts`): `/c/{id}` refuses if it _is_
framed, `/e/{id}` refuses if it is _not_.

Both halves are measured, not reasoned:

- With the default only (no third-party rewriting), **every hosted test failed
  on `[data-screen="select_rail"]` never appearing**, with the refusal screen
  "This page will not load here" rendered instead — vpay's page is never the
  primary origin in these specs (the shop is), and the default rewrites only
  the primary origin's JavaScript.
- With the rewrite left on, **every nested frame reports `window.parent ===
window` to its own code**. Instrumented in a throwaway build of the checkout
  app that recorded `decideEntry`'s inputs:

  ```
  rewrite on : {"decision":"refused","framed":false,"parentIsSelf":true,"topIsSelf":true,
                "referrer":"http://localhost:13001/","allowedOrigins":["http://localhost:13001"]}
  rewrite off: {"decision":"ready","framed":true,"parentIsSelf":false,"topIsSelf":false,
                "referrer":"http://localhost:13001/","allowedOrigins":["http://localhost:13001"]}
  ```

  Same session, same shop, same referrer. Read from the runner's side the frame
  was framed all along (`el.contentWindow.parent !== el.contentWindow` was
  `true`); it was only the _page's own_ reads that had been rewritten. The
  instrumentation was reverted and the image rebuilt before anything was
  committed.

**Confirmed against a real browser before blaming the app.** With the same
stack running, Chrome was pointed at the shop's `/orders/{id}/embedded` page —
by a full page load and by a click-through soft navigation — and both times
vpay's page rendered inside the iframe with the amount, the merchant name and
both rails. So the refusal is a runner artefact, and this lane did not file a
defect against lane 3.

**What the default pass therefore does NOT prove:** that vpay's hosted page
refuses to render inside a frame. Nothing in Cypress can — the runner is a
frame. `frontends/apps/checkout/src/lib/entry.test.ts` covers it.

### 3a. `frame-ancestors` is proven _sent_, not proven _enforced_

Cypress strips the `Content-Security-Policy` header from every document it
proxies (`experimentalCspAllowList` defaults to `false`), and turning that off
is not available here: the hosted page sends `frame-ancestors 'none'`, and with
the policy restored Cypress's own AUT iframe could not load it. So the header
is asserted **as the server sends it**, with `cy.request` out of Cypress's Node
process:

```
content-security-policy: frame-ancestors http://localhost:13001
referrer-policy: no-referrer
cache-control: no-store
```

**No test in this repository has seen a browser refuse a frame because of
vpay's CSP.** What _is_ observed in a browser is vpay's second lock, which
does not depend on the header: the page resolves its framer from
`document.referrer` against the merchant's registered origins and refuses
before it reads anything.

## 4. Guard-failure proofs

Each break was applied, the failure observed, and the break reverted.

**A — the framer allow-list, at the deployment.** `http://localhost:4181` (the
fixture's origin) added to `shop-merchant`'s `checkout_origins` in the
generated overlay, `vpay-server` restarted:

```
✗ frames vpay's page with the exact src, and vpay names the shop's origin in frame-ancestors
✗ refuses to be framed by an origin the merchant never registered
    AssertionError: expected 'frame-ancestors http://localhost:13001 http://localhost:4181'
                    to equal 'frame-ancestors http://localhost:13001'
2 passing, 2 failing
```

and, with a throwaway spec that asserts the opposite of the negative test, the
same fixture page **renders** once its origin is registered:

```
✓ renders inside the fixture frame once its origin IS registered (225ms)
```

So the refusal in `shop-embedded.cy.ts` is caused by the origin not being on
the merchant's list, and not by the page failing for some other reason —
which, given §3's finding, was worth proving rather than assuming.

Overlay restored; `GET /v1/browser/checkout/origins?key=…` back to
`{"origins":["http://localhost:13001"]}`; both specs green again.

**B — `vpay-worker` stopped.** See §5.

## 5. The worker-down proof

The plan says a spec here must not be able to pass while `vpay-worker` is
down. It cannot. `docker compose … stop vpay-worker`, then the whole hosted
spec, with everything else untouched:

```
  the shop, paid on vpay's hosted page
    (Attempt 1 of 3) MTN push …            (Attempt 2 of 3) …   (Attempt 3 of 3) …
    (Attempt 1 of 3) Orange redirect …     …
    (Attempt 1 of 3) a payment that does not succeed …  …

  0 passing (18m)
  3 failing

  AssertionError: Timed out retrying after 120000ms: Expected to find element:
    `[data-outcome="succeeded"]`, but never found it.      (MTN)
  AssertionError: Timed out retrying after 120000ms: Expected to find element:
    `[data-outcome="succeeded"]`, but never found it.      (Orange)
  AssertionError: Timed out retrying after 120000ms: Expected to find element:
    `[data-outcome="failed"]`, but never found it.         (the decline)
```

**Every one of the three fails on the settlement wait**, and on nothing
earlier: the shop authenticated, created the intent and the session, the payer
reached vpay's page, chose a rail, submitted an MSISDN or followed the
redirect — and then nothing moved. The failure screenshot is the page sitting
on "Check your phone" with `GET /v1/browser/payment_intents/pi_…` repeating in
the command log.

Underneath, measured in the two databases while the run was failing:

```
vpay:  payment_intents   6 processing, 3 requires_action
shop:  the 9 orders that run created — all 9 unpaid, none paid
```

The worker was restarted and the same spec is green again (§6). The third
case is worth naming: **even the DECLINE needs the worker**, because a
declined MTN push is also a rail status query, so there is no branch of this
spec that a settlement-free stack could satisfy.

## 6. Runtimes

Per-spec, `cypress run`'s own figures.

| Spec                                       | Tests          | Runtime                                               |
| ------------------------------------------ | -------------- | ----------------------------------------------------- |
| `shop-hosted.cy.ts`                        | 3              | **27 s** (16.8 s / 5.7 s / 5.5 s)                     |
| `shop-embedded.cy.ts`                      | 4              | **11 s** (0.8 s / 5.3 s / 4.0 s / 0.2 s)              |
| `shop-hosted.cy.ts`, `vpay-worker` stopped | 3, all failing | 18 min (3 attempts each, two 120 s waits per attempt) |

Those three were measured on the AUTHORING HOST against a stack on
`demo_port=18080 demo_checkout_port=13080 demo_shop_port=13001
demo_orange_port=18082` (8080 was held by an unrelated project on this shared
machine), **and on a shop image carrying a one-line local patch that was never
committed** — the workaround for §2 while lane 5b was in flight. See §6a for
the numbers that stand.

### 6a. The final run, unpatched

`just test-e2e` **from nothing**, in the `vpay-ci` Multipass VM
(`~/dev/vpay-ci/run-ci.sh <worktree> test-e2e`), on the branch after
`claude/step9-hosted-checkout` (lane 5b's fix) was merged in. No local patch of
any kind. Default ports throughout; the images were built in the VM; the
recipe generated the keys, brought the stack up, waited on all four surfaces,
ran both Cypress passes and tore the stack down with `down -v`. **Exit 0.**

```
pass 1 (web security on, frame-busting rewrites on)
  ✔  checkout.cy.ts                           00:13        1        1        -        -        -
  ✔  dashboard.cy.ts                          395ms        3        3        -        -        -
  ✔  shop-hosted.cy.ts                        01:18        3        3        -        -        -
     ✔  All specs passed!                     01:32        7        7        -        -        -

pass 2 (VPAY_E2E_FRAMED=1 — web security off, rewrites off)
  ✔  shop-embedded.cy.ts                      00:13        4        4        -        -        -
     ✔  All specs passed!                     00:13        4        4        -        -        -
```

Per test, from that run:

| Spec                  | Test                                                         | Runtime |
| --------------------- | ------------------------------------------------------------ | ------- |
| `shop-hosted.cy.ts`   | MTN push → `paid` via the webhook                            | 68.1 s  |
|                       | Orange redirect through the rail's page → `paid`             | 4.2 s   |
|                       | a payment that does not succeed → `cancel_url`, never `paid` | 5.3 s   |
| `shop-embedded.cy.ts` | the exact frame `src` and `frame-ancestors`                  | 1.0 s   |
|                       | MTN completes in the frame → `paid` via the webhook          | 5.4 s   |
|                       | Orange breaks out → the shop's `return_url` → `paid`         | 4.5 s   |
|                       | an unregistered framer is refused                            | 0.3 s   |

**Eleven tests across two passes, 0 failing, 0 pending, 0 skipped.** The 68 s
first MTN test is the poll ladder plus the webhook delivery on a cold VM; the
same test takes 17 s on the authoring host.

## 7. Status rows for lane E

Add to `docs/status.md`'s **Testing** table (or beside the `checkout.cy.ts`
row, which this does not replace):

| `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts` | ✅ | **New 2026-09-04 (Step 9, lane 6).** `examples/shop` driven in a real browser against the real compose stack, to vpay's hosted page and back, three times: an MTN push on the digits-only steering MSISDN `237600000100`, an Orange redirect through the stub's own hosted page (D7) with its "Pay" link, and a decline (`237600000101`) that forwards the payer to `cancel_url`. Every "it was paid" assertion is made on the SHOP's return page, which polls the shop's own `orders.get` and reads nothing but the shop's database — written only by the shop's webhook handler after it verifies vpay's signature. **Proven not to pass without `vpay-worker`:** with that container stopped, all three fail on the settlement wait after 120 s, with the intents left `processing`/`requires_action` and all nine orders that run created left `unpaid`. Green from nothing in the `vpay-ci` VM on 2026-09-04, 3 tests in 1 m 18 s |
| `frontends/tests/e2e/cypress/e2e/shop-embedded.cy.ts` | 🟡 | **New 2026-09-04 (Step 9, lane 6).** vpay's page framed on the shop's own site: the iframe's `src` asserted byte for byte against D6 (id in the path, publishable key in the query, session secret in the fragment and nowhere else), MTN completed INSIDE the frame with `vpay:complete` reaching the shop, Orange breaking out to the rail and returning to the embedded session's `return_url`, and the same credential framed from an origin nobody registered being refused before the page reads anything — the refusal proven to be origin-driven by registering that origin and watching the same page render. It runs as a SECOND `cypress run` with `chromeWebSecurity` and Cypress's frame-busting rewrites off, because neither is settable per test and the two page modes need opposite answers from `window.parent`. 🟡 for what it cannot see: **Cypress strips `Content-Security-Policy` from every document it proxies**, so `frame-ancestors` is asserted as the server sends it (`cy.request`) and no test in this repository has watched a browser refuse a frame because of it; and the `vpay:redirect` hand-off is intercepted before `@vpay/stripe-js` performs the top-level navigation, because under Cypress that navigation would move the runner itself. Green from nothing in the `vpay-ci` VM on 2026-09-04, 4 tests in 13 s |
| `just test-e2e` | ✅ | **Changed 2026-09-04 (Step 9, lane 6): it now brings up a stack the specs can pass against.** It used to start `compose.yml -f compose.e2e.yml` alone, which registers no merchant anybody holds a private key for, so every spec that mints anything answered `invalid_client` — `checkout.cy.ts` since Step 5c. It now depends on `gen-demo-keys`, `build-sdk-node` and `build-checkout-browser`, adds `-f compose.demo.yml`, waits for `/healthz` and for the dashboard, the shop and the checkout page by name, runs BOTH Cypress passes and tears the stack down with `down -v`. Green from nothing in the `vpay-ci` VM on 2026-09-04: **11 tests across four specs, 0 failing, 0 skipped, exit 0** |

## 8. What this lane did NOT do

- **No browser has seen vpay's CSP enforced.** §3a. The header is proven sent.
- **No browser has seen the hosted page refuse to be framed.** The runner is a
  frame, and the rewrite that makes the hosted page work under Cypress is
  exactly the one that would hide the refusal. `entry.test.ts` covers it.
- **`window.top.location.assign` on `vpay:redirect` is not exercised.** The
  spec intercepts the message and performs the navigation itself; the SDK's
  own vitest is what covers the call.
- **No `just ci`, no `just demo`, no Rust.** This lane compiled nothing and
  changed no Rust, no SDK and no application source. `just lint-web`
  (`pnpm -r typecheck`, 16 projects) is green; `just test-web`, `cargo` and
  the conformance suite were not run and are untouched by anything here.
- **`checkout.cy.ts` and `dashboard.cy.ts` are unchanged**, as the plan
  required.
- **No `docs/status.md` and no `docs/flows/*` edits** — §7 is lane E's to
  apply.
- **The worker-down proof was run on the authoring host, not in the VM**, and
  on the shop image carrying the §2 workaround (the only thing that patch
  changed was the token exchange, which the proof does not touch: all three
  tests got as far as the settlement wait). The green runs that stand are §6a's,
  in the VM, unpatched.
- **`just test-e2e` has been run green once**, from nothing, in the VM. Not
  three times, and not on the authoring host at default ports — 8080 is held
  there by an unrelated project.
- **Nothing was measured about flakiness.** `retries: 2` is unchanged from
  before this lane, and no spec here was run repeatedly to see whether it is
  stable.
