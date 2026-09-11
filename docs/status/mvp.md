# What would have to be true to call this "an MVP"

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

## What would have to be true to call this "an MVP"

_Re-answered item by item on 2026-09-04 (Step 8, the production gate). **No
item moved to met.** Four moved *within* themselves — item 2's callback clause,
item 3's crash-test clause, item 4's confirm race and item 5's SSRF residual —
and each is marked below with what changed and what still blocks it. The one
sentence that decides items 2, 3, 4 and 5 together is unchanged: **no real rail
has ever been called.**_

_Re-answered again the same day for **Step 9** (hosted and embedded checkout).
**Item 6 — `just test-e2e` green against the compose stack — moved to met**,
the first item to move since item 1; **item 8 was added at the maintainer's
request and is answered without being met**; and item 5's webhook clause
narrowed. The list is **eight items** from here on. The sentence above still
decides items 2, 3, 4 and 5 — and item 8 with them._

1. ~~Database schema + migrations, with the `one_charge_per_intent` unique
   index.~~ **Done.** `backends/migrations/0004_create-charges.sql:73`
   creates `one_charge_per_intent` as a plain unique index on
   `charges (payment_intent_id)`, proven to reject a second charge by
   `one_charge_per_intent_is_enforced_by_the_database` in
   `backends/tests/integration/tests/postgres_smoke.rs`. This item is about
   the schema existing and its constraints holding, not about the
   application using it — see the "Database schema / migrations (core)" row
   above for that distinction; the remaining items below are unaffected.
2. Both adapters making real HTTP calls, passing the shared conformance suite
   with the `#[ignore]`s removed. **Updated 2026-09-03 (Step 3): the literal
   words of this item are met; the item is not.** Both adapters make real
   HTTP calls, and the shared suite passes with **no `#[ignore]`s left at all**
   — 26 tests, 26 passed, 0 skipped, measured on the authoring machine
   (`just verify-ignored` now pins `expected_ignored := "0"`). What every one
   of those 26 talks to is a `wiremock/wiremock` container. **Neither MTN's
   nor Orange's real sandbox has ever been called**, so this item stays open
   in the sense that matters for taking a payment: what is proven is that the
   adapters implement the protocol `docs/flows/adapter-*.md` describes, not
   that those documents are right. Also unproven: the 401-after-a-good-token
   re-mint path on both rails; Orange's duplicate-submit idempotency (an
   assumption about the rail, not an observation); and ~~the callback path,
   because no callback route exists~~ **the callback path end to end: the route
   exists as of 2026-09-04 (Step 8, lane C) and is proven against WireMock
   (`provider_callback.rs`, 9 cases), but no real rail has ever called it, and
   Orange's `notif_token` is still not compared against the stored one.**
   `mtn_momo::refund` remains `NotImplemented` — see the token list above.
   _**Re-answered 2026-09-04 (Step 8): still open, and the count moved.**_ The
   shared suite is now **28 tests, 0 skipped** (12 port cases over both rails
   plus 4 that are not rail-specific), the twelfth being
   `the_submit_tells_the_rail_where_to_call_back`. Every one of the 28 still
   talks to a `wiremock/wiremock` container, so **the literal words of this
   item remain met and the item remains unmet**, for exactly the reason it has
   always been unmet.
3. The worker's job loop, poll ladder and reconciler, with crash tests.
   **Updated 2026-09-03 (Step 4): substantially met, with two named gaps.**
   The loop, the ladder, the recovery table, the settlement transaction and
   the 24-hour `unresolved` escalation all exist and are proven by 20
   `worker_recovery` + 3 `worker_e2e` container-backed tests; a confirmed
   intent reaches `succeeded` without operator intervention. **The two gaps
   are stated rather than rounded off:** (a) the "crash tests" do not kill a
   process — they write the state each of the three documented kill points
   leaves and run the real handlers against it, which proves the recovery
   table and not the process's behaviour under a signal; (b) a rail answer
   contradicting an already-settled charge is classified by a tested pure
   function whose **call sites no test reaches**. And the rail behind every
   one of these observations is still a WireMock container.
   _**Re-answered 2026-09-04 (Step 8, lanes D and G): gap (a) is narrowed to
   one kill point; gap (b) is unchanged; the item is still not met.**_
   `backends/tests/integration/tests/worker_kill9.rs` `SIGKILL`s the shipping
   `vpay-worker-bin` mid-status-query and the shipping `vpay-server`
   mid-`requesttopay`, and both charges settle exactly once — so two of the
   three kill points are now _caused_ rather than written. **Kill point 1
   remains written, not caused**, and deliberately: there is no network call to
   interrupt at the moment before the reference is minted, so there is nothing
   for a signal to land during. Two clocks are simulated in that file (the dead
   worker's lease, and — since lane G — the crashed charge's age); nothing else
   is. **Orange is not exercised by the kill test at all**, and the rail is
   still a WireMock container. Lane G additionally fixed a defect this step's
   demo found: the worker was applying that same recovery table to a charge
   whose confirm was still running — see the confirm/worker race row.
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency,
   authenticated. **Updated 2026-09-03 (Step 3): confirm now reaches a rail
   and moves the intent — `processing` on a push rail, `requires_action`
   with a `next_action.redirect_to_url` on a redirect rail, `409
charge_declined` when the rail refuses, `502` when it cannot be reached.
   Seven integration tests in `confirm_rails.rs` drive all four outcomes
   against real Postgres and WireMock containers. The item is still not met,
   and now for exactly two reasons rather than one:** the rail behind every
   one of those observations is a stub, and **nothing polls the charge
   afterwards** — a confirmed intent stops at `processing` forever, so
   "create + confirm" produces a payment that is never resolved.
   _**Updated 2026-09-03 (Step 4): the second of those two reasons is
   closed** — the worker polls the charge and drives the intent to
   `succeeded`, `failed`/`requires_payment_method`, or `unresolved` with an
   alert. **The first is not, and it is the one that decides this item:** the
   rail is a WireMock host, so what has been proven is that vpay resolves a
   payment correctly against a stub that behaves the way these documents say
   a rail behaves._
   _**Re-answered 2026-09-04 (Step 8, lane G): a defect on this exact path was
   found and fixed, and the item is still not met.**_ Step 8's demo produced a
   `500 api_error` on confirm in four of six walkthrough runs — the worker
   claiming the poll job the confirm had just committed, applying the
   crash-recovery table to a charge whose process had not crashed, and moving
   it out from under the confirm's own compare-and-swap. On a push rail the
   merchant was told the confirm failed and then sent a
   `payment_intent.succeeded` webhook; on a redirect rail a live order was
   killed as `provider_unavailable`. The fix is a minimum charge age in
   `recovery_step`, proven by a unit table and three `worker_recovery` cases,
   the last of which runs the shipping loop against the confirm's own write.
   **This makes create + confirm _correct_ under contention; it does not make
   this item met**, and the reason is the same one it has been since Step 3:
   the rail is a stub. _The Step 2
   note follows, and its account of where confirm stopped is now history._
   **(Step 2): create is done; confirm reaches the rail boundary and stops
   there, so this item is not met.**
   `POST /v1/payment_intents` is form-encoded, idempotent, authenticated and
   merchant-scoped, and writes a real row — with `create_then_retrieve_round_trips_through_the_sdk`,
   `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` and
   `a_post_without_an_idempotency_key_is_the_documented_400` behind it, all
   run against a real Postgres on 2026-09-03. Retrieve, list and cancel are
   there too. **`confirm` is where this item stays open:** it performs the
   crash-safe write ordering and then gets `501 not_implemented` from the
   adapter, so no payment intent has ever reached `processing`,
   `requires_action` or `succeeded`, and no rail has been called. This item
   asks for create **and** confirm; half of it is now real and the other half
   is blocked on item 2. _The paragraph that follows was written when the
   resource half had not started; its account of the credential model still
   holds and is kept for that._ **The _authenticated_ half of this item is
   built; the resource half is now built as far as the rail.** As of 2026-09-02 (Step 1) a merchant can
   obtain an access token from `/v1/oauth/token` with `client_credentials` +
   `private_key_jwt` and carry it past the authentication boundary — see the
   "Merchant auth" and "Merchant OP" rows above for the six integration tests
   that cover it, and the header paragraph for the one thing that keeps those
   rows at 🟡 (they have run once, manually, never under Docker or in CI).
   **What an authenticated `/v1/payment_intents` request gets today is a 404**,
   and that is the honest answer rather than a placeholder: no payment-intent
   route, no handler, no idempotency implementation, no form-decoding of a
   create body. Nothing about creating or confirming a payment intent moved
   this pass. The credential-model history this item used to recount still
   holds: `merchant_api_keys` (migration `0008`) is dropped (migration
   `0009`) and the design it backed is reversed by
   [ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md) — there is no opaque
   key of any kind on `/v1`. The two decisions that were open on the wire
   contract are now decided _by the server_, not merely proposed by the SDKs:
   the token endpoint is `{public_base_url}/v1/oauth/token` and the audience
   is `vpay:v1` (see the Merchant SDKs section). The one ADR-0010 premise
   that shifted at `authkestra-op = "=0.7.1"` — that `SqlxOpStore::find_client`
   can now persist `token_endpoint_auth_method`/`jwks`, making a
   database-backed merchant registry buildable — was **resolved by keeping
   YAML**: `YamlClientStore` reads `merchant_clients` from the config file and
   the database only ever subtracts access via `disabled_clients`. That is
   ADR-0010's original choice, now implemented rather than merely recorded.
5. Signed webhooks with the two-step outbox. **Updated 2026-09-04 (Step 8,
   lane B): the one ⛔ this item carried is closed and the item is still not
   met.** Every delivery now goes through `vpay_worker::ssrf` — resolve once,
   classify every answered address, pin the connection to them, refuse a
   redirect — so the "no SSRF protection of any kind" row is retired and
   replaced by three named residuals (webhook delivery only; NAT64 refused
   fail-closed; the shared connection pool given up for the pin). What keeps
   the item open is unchanged and is not about SSRF: **no merchant endpoint has
   ever been POSTed to**, delivery is unordered, the ladder's 1 h / 6 h / 24 h
   rungs have never elapsed, and replaying an exhausted delivery is still a
   hand-written transaction.
   _**Re-answered 2026-09-04 (Step 9): one clause is now narrower, and the item
   is still not met.**_ `examples/shop`'s `POST /api/vpay/webhook` is the first
   receiver in this repository's history that **acts** on a delivery rather than
   recording it in a WireMock journal: it verifies the signature with
   `@vaam-apps/vpay-sdk`, dedupes by event id, and marks an order `paid` — and lane 6's
   Cypress specs assert an order reaching `paid` only through it, with the
   payer's return page reading the shop's own database and taking no decision
   from the return trip. **It is still not a merchant endpoint**: `vpay-shop` is
   a container on the same compose network, permitted by
   `webhooks.allow_private_targets`, and it is code this repository wrote and
   tests. The other three clauses are untouched.
6. ~~`just test-e2e` green against the compose stack.~~ **Done 2026-09-04
   (Step 9, lane 6) — the first item on this list to move since item 1, and it
   moved because the recipe was fixed, not because a spec was weakened.**
   `just test-e2e` from nothing in the `vpay-ci` VM, unpatched: **exit 0, 11
   tests across four specs, 0 failing, 0 skipped**, in two Cypress passes
   (`checkout.cy.ts`, `dashboard.cy.ts`, `shop-hosted.cy.ts`, then
   `shop-embedded.cy.ts` with `VPAY_E2E_FRAMED=1`). **What that took is worth
   recording rather than celebrating:** the recipe used to bring up
   `compose.yml -f compose.e2e.yml` alone, which registers no merchant anybody
   holds a private key for, so every spec that mints anything answered
   `invalid_client` — **`checkout.cy.ts` could not have passed under
   `just test-e2e` since Step 5c**, and CI's `e2e` job was the only place that
   spec had ever run. It now depends on `gen-demo-keys` and adds
   `-f compose.demo.yml`. This item asks for a green gate and the gate is
   green; it does **not** say anything about money, and the rails behind all
   eleven tests are WireMock hosts. Nothing about flakiness was measured — one
   green run, `retries: 2` unchanged. What Step 8 added beside it is
   `just demo`, a different artefact: a human-read walkthrough, not a build
   gate. Its own state is in the "Local demo" row, and since Step 9 the merged
   branch's own three consecutive green runs exist.
7. `/dash/v1` login working end to end against a real database — issuing an
   access token, verifying it on a subsequent call, and rotating a signing
   key at least once. **Two of those three verbs now have an implementation,
   and it belongs to the _merchant_ surface, not this one.** Tokens are
   issued and verified on `/v1` (item 4); a signing key is generated,
   loaded, announced in `oauth_signing_keys` and published at
   `/v1/oauth/jwks.json`. ~~**Nothing here has ever performed a login**, and
   the gap is not a matter of wiring the same parts to a second router:
   there is no `/login` or `/authorize` route, no `SessionStore` (
   `authkestra-engine` is pinned without its `sql-postgres` feature, so no
   SQL-backed session store is compiled in at all), and
   `authkestra-op`'s authorization-code handler mints `aud = <client_id>`
   with no requested-audience path, which `Surface::Dashboard`'s
   `vpay:dash/v1` would reject on every call — a design question whoever
   builds this must answer first.~~ **Corrected 2026-09-07
   ([ADR-0017](../adr/0017-staff-authentication.md)): a staff member can sign
   in.** `POST /dash/v1/staff/login`, `/totp`, `/password`, `/session`,
   `/logout` and `GET /dash/v1/oauth/authorize` + `POST /dash/v1/oauth/token`
   are mounted, and `backends/tests/integration/tests/staff_sign_in.rs`
   drives thirteen cases over a real socket against a real Postgres in which
   **every token presented came out of that token endpoint** after a
   password, a TOTP code and a PKCE exchange — the suite mints none itself.
   The design question above was answered rather than deferred; the audience
   is `Surface::Dashboard`'s and the server verified it through its own
   published JWKS. What item 7 still does **not** have is the third verb,
   key rotation, and a browser: no page of `frontends/apps/dashboard` calls
   any of these routes. See the "Dashboard auth" row and
   [flows/dashboard-auth.md](../flows/dashboard-auth.md) § Status. **And
   "rotating a signing key at least once" is still unmet in the sense this
   item means it:** `ensure_active_signing_key` will rotate the database
   record when a process boots with a different key, but `TokenManager`
   holds one key for the life of the process, so rotation is restart-based,
   nothing re-reads the key file, and no rotation has been observed on any
   deployment. Not part of "does this take payments"; listed here because it
   is the half of Phase 2 this pass deliberately did not build. **Unchanged by
   Step 8, deliberately:** the Step 8 plan names the dashboard as out of scope
   and says why — there is no `/dash/v1` API for it to read, so booting the
   screen would invite a reader to look at something that cannot show the
   payments just made (`docs/runbooks/demo.md` §6). Nothing about a login moved.

8. **A hosted page for driving payments on the web: one in-iframe version,
   one fully hosted page — "we need that before prod."** _(Added to this list
   2026-09-04 at the maintainer's request, verbatim, and answered by Step 9.)_
   **Both pages exist, both have been driven by a real browser, and this item
   is still not met for the purpose it names — going to production.** What is
   built: a `checkout.session` object with a hosted and an embedded mode
   (`cs_…`, migration `0028`); `frontends/apps/checkout`, a Next app vpay
   serves, in French and English, with `frame-ancestors` derived per merchant
   from a new `checkout_origins` list; the return trip for a redirect rail,
   which closes browser-checkout's D4 at both ends; `initEmbeddedCheckout` in
   `@vaam-apps/vpay-stripe-js` and `checkout.sessions` in both merchant SDKs; a fourth
   image, a Helm workload behind `checkout.enabled`, and a demo shop
   (`examples/shop`) that a payer can actually buy from. Lane 6 drove all of it
   in Chrome: MTN inside the frame, Orange breaking out to the rail's page and
   back, an order reaching `paid` only from the shop's own verified webhook,
   and an unregistered framer refused. **What stands between that and prod, in
   the order it would bite:** (a) every rail in every one of those runs is a
   WireMock container — the sentence that decides items 2, 3, 4 and 5 decides
   this one too; (b) **no browser has been observed enforcing vpay's
   `frame-ancestors`** — Cypress strips the header, so it is asserted as sent,
   and the refusal a browser was seen performing is the page's own origin
   check, which is a second lock and not the header; (c) this step added a
   **second unauthenticated surface** (the checkout app, plus three new
   `/v1/browser` reads) with the same ingress rate-limiting requirement
   browser-checkout's D5 already stated and nothing in this repository
   enforces or checks; (d) **no pod has ever run** the checkout Deployment, and
   its path-prefix Ingress shape has been run by nobody; (e) the demo shop's
   `ZenStackShopStore` — the code path every order actually goes through — has
   **no automated coverage of its own**; and (f) `checkout_not_configured`
   answers `500` where a `503` would be truthful, which is an ADR-level change
   left to the maintainer. The maintainer's sentence asked for the pages before
   prod; the pages are here, and the list of what "before prod" still means is
   above.

**Nothing in Step 8 moved an item on this list to met, and that is the honest
summary of the step.** What it moved is the _reasons_: four of the seven items
had a clause that was false or too broad by the end of 2026-09-04, and each is
struck and corrected above rather than quietly rewritten. The single fact that
keeps items 2, 3, 4 and 5 open is one fact, not four: every rail and every
webhook receiver in this repository's entire history is a `wiremock/wiremock`
container.

**Step 9 moved one item to met — item 6, the e2e gate — and added item 8,
which it answers without meeting.** That is the honest summary of this step:
the thing the maintainer asked for is built and a browser has walked it end to
end, and the one fact above is as true of Step 9 as of every step before it.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
