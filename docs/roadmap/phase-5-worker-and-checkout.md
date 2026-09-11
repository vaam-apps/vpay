# Roadmap — Phase 5 and 5d: The worker, and hosted and embedded checkout

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

## Phase 5 — The worker

**Goal.** Charges in `processing` reach a terminal state without operator
intervention, and a crash at any of three documented points resolves
without double-charging.

**Status. 🟡 In progress — the loop landed 2026-09-03 (Step 4).**
`vpay-worker-bin` no longer logs a heartbeat saying the loop is not
implemented: it boots, reconciles configuration under the same advisory lock
`vpay-server` uses, reaps stranded job leases, seeds its its singleton jobs (two then; four since Step 5 — `sweep:expired`, `scan:live`, `fanout:events`, `scan:deliveries`) and
runs `vpay_worker::run_loop` — N claim/settle tasks over a `jobs` table
(migration `0021`, `FOR UPDATE SKIP LOCKED`, leases guarded on `locked_by`,
dead letters parked at `run_at = 'infinity'`), a 60-second gauge line, and a
bounded drain on SIGTERM. A confirmed payment reaches `succeeded` without
anyone touching it, and `just demo`'s sixth step is exactly that, end to end
through the containerised stack.

**Every settlement observed so far came from a WireMock rail.** No real rail
has been called, and the loop has never run anywhere but a developer machine
and CI.

**Scope, and what became of each item.**

- ✅ The job loop consuming charges needing action — `vpay_worker::run_loop`,
  driven by the same function the integration suite runs (there is no
  `#[cfg(test)]` variant and no injected clock).
- ✅ The poll ladder wired to that loop, indexed by `jobs.attempts - 1`.
- ✅ The reconciler: the settlement table (`vpay_core::settlement::settle`),
  the 24-hour `unresolved` escalation with its hourly re-poll and alert, and
  the one transaction that moves charge + intent + `events` row together.
  (Still not [RFC-0001](../rfc/0001-settlement-and-payouts.md)'s settlement/payouts
  scope, which stays parked pending a licensing conversation.)
- 🟡 The three crash-test injection points from
  [`docs/flows/crash-safety.md`](../flows/crash-safety.md) — all three are
  exercised, by _writing the state each one leaves_ and running the real
  handlers against it. ~~**No process is killed.**~~ **Corrected 2026-09-04
  (Step 8, lane D): two of the three are now killed for real.**
  `backends/tests/integration/tests/worker_kill9.rs` `SIGKILL`s the shipping
  `vpay-worker-bin` mid-status-query and the shipping `vpay-server`
  mid-`requesttopay`, asserts the exit was _signalled with 9_, and asserts the
  charge settles exactly once with one submit in the rail's journal. **Kill
  point 1 is still written rather than caused** — there is no network call to
  interrupt before the reference is minted — and Orange is not exercised at
  all.
- ⛔ Absorbed from Phase 4b but **not** delivered: nothing else. Not in this
  phase's original scope and still unbuilt: `prompt_ttl_seconds` /
  `prompt_expired_at` / `payment_intent.processing` — named in
  [`docs/flows/reconciler.md`](../flows/reconciler.md)'s Status. _(The callback
  route was on this list until 2026-09-04; Step 8's lane C built it.)_

**Definition of done — met in substance, with one honest gap.** The recovery
table resolves every injection point without a double charge, asserted by a
single distinct `provider_reference_id` across every `provider_requests` row
for the charge. ~~What is _not_ met is the literal wording: these are not
kill-the-process tests~~ — **narrowed 2026-09-04 (Step 8): two of the three
now are.** The literal wording is unmet for **kill point 1 only**, and for the
reason above rather than for want of effort. Calling the remaining case a
kill-the-process test would still be the overstatement this repository exists
to avoid.

**Unblocks.** Reliable terminal states for Phase 6 to notify on.

**Risks carried by this phase.**

- ~~`--shutdown-grace-seconds` does nothing on `vpay-worker-bin`.~~ Closed:
  the flag now bounds a real drain — tasks stop claiming, in-flight jobs
  finish, and on timeout the remaining tasks are aborted, every lease this
  worker holds is handed back and the process exits non-zero
  (`a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds`).
- **Open: a rail answer that contradicts a settled charge is detected and
  logged, and neither call site is covered by a test.** The classifier is
  table-tested; the wiring is not. See `docs/status.md`.
- **Open: no real rail.** Everything above is proven against WireMock hosts,
  so what is proven is that vpay executes its own documents correctly.

---

## Phase 5d — Hosted and embedded checkout _(delivered 2026-09-04, Step 9)_

**Goal, in the maintainer's own words (2026-09-04):** _"We need a hosted page
for driving payments on the web: one in-iframe version, one fully hosted page.
We need that before prod."_

**Where this phase started.** Phase 5c had shipped `/v1/browser` and
`@vaam-apps/vpay-stripe-js` — a merchant could build its own payer page —
but vpay served no HTML at all, `tower-http` was built without `fs`, there was no
`frame-ancestors` or `X-Frame-Options` anywhere, no `success_url`/`cancel_url`
on any object, no per-charge return URL (so a redirect rail sent every payer to
a `POST`-only callback path that answers an empty `405`), and no i18n.
`@vaam-apps/vpay-stripe-js`'s README listed "Checkout (hosted or embedded)" under "Not
compatible, ever".

**What landed** ([plans/2026-09-04-step9-hosted-checkout.md](../plans/2026-09-04-step9-hosted-checkout.md),
twelve lanes):

- **A `checkout.session` object** (`cs_…`, migration `0028`) a merchant creates
  from its server against an intent it already has. `ui_mode: hosted` answers a
  `url` to redirect the payer to; `ui_mode: embedded` answers a `client_secret`
  the merchant hands to `@vaam-apps/vpay-stripe-js`. Two payer credentials, not one: the
  session secret rides in the hosted URL's **fragment**, and a separate
  `return_token` rides in the return page's query string, because a fragment
  does not survive a rail's redirect.
- **The page** — `frontends/apps/checkout`, a Next 15 App Router app vpay
  serves, French and English, with the amount in integer minor units, a rail
  selector, an MSISDN form for MTN, the redirect for Orange, the outcome
  screen, and the forward to the merchant's URL with `{CHECKOUT_SESSION_ID}`
  substituted.
- **The return trip**, which closes Phase 5c's named gap
  ([flows/browser-checkout.md](../flows/browser-checkout.md)'s D4): the provider
  port carries a per-charge `return_url`, Orange sends it, and vpay has a page
  to receive the payer.
- **`frame-ancestors` from configuration** — a per-merchant `checkout_origins`
  list, empty by default, resolved server-side by the page's middleware before
  any script runs, plus the page's own origin check against the same list.
- **`initEmbeddedCheckout`** in `@vaam-apps/vpay-stripe-js` and `checkout.sessions` in
  both merchant SDKs, in one PR per [ADR-0015](../adr/0015-sdk-parity.md).
- **A fourth image** (`ghcr.io/vaam-apps/vpay-checkout`), a Helm workload
  behind `checkout.enabled`, and an eight-service demo stack.
- **`examples/shop`** — a Next.js merchant site with a seeded XAF catalogue,
  tRPC and ZenStack 3, whose orders turn `paid` only from vpay's
  signed webhook and never from the return trip.

**What proves it.** `shop-hosted.cy.ts` and `shop-embedded.cy.ts` drive a real
browser through the shop to vpay's page and back on both rails, hosted and
embedded — and are proven not to pass with `vpay-worker` stopped. `just
test-e2e` from nothing is green in the `vpay-ci` VM: 11 tests over four specs,
0 failing, 0 skipped.

**Risks carried by this phase.**

- **No real rail, as everywhere else.** Every payment a browser has completed
  through vpay's page settled against a `wiremock/wiremock` host.
- **`frame-ancestors` is proven _sent_, never proven _enforced_.** Cypress
  strips `Content-Security-Policy` from every document it proxies, so the
  header is asserted with `cy.request` out of the runner's Node process. What a
  browser _was_ seen enforcing is the page's own origin check refusing an
  unregistered framer.
- **A second unauthenticated surface with the same ingress requirement.**
  Phase 5c's D5 said rate limiting belongs at the ingress and nothing here
  enforces or checks it; Step 9 added the checkout app and three more
  `/v1/browser` reads under the same unmet requirement.
- **No pod has ever run the page**, and its path-prefix Ingress shape has been
  run by nobody.
- **The demo shop's `ZenStackShopStore` has no automated coverage of its own** —
  verified by hand against a real Postgres, and exercised by the Cypress specs
  without being asserted on.
- **`checkout_not_configured` answers `500`, not `503`.** A truthful `503`
  needs an ADR-level change to [ADR-0011](../adr/0011-error-modelling.md)'s
  category table, and that is left to the maintainer.

---
