# Roadmap

**Snapshot date: 2026-08-11, last refreshed 2026-09-04 (the Step 9 addendum at the foot of this page).** A point-in-time read
of the repository, for someone opening it cold or coming back after a break,
organized as the sequence of phases from scaffold to a deployable gateway. It
goes stale the moment code moves and nobody is obliged to update it on the
same schedule as a commit. **On any disagreement,
[`docs/status.md`](status.md) and the ADRs in [`docs/adr/`](adr/) win,
always** — they are the machine-checked and immutable sources of truth
respectively. This page does not restate
`docs/status.md`'s per-feature rows; if you're here to check whether some
specific thing works, go there instead.

**vpay still cannot take a payment — with real money.** _That sentence used to
continue "no HTTP call to any rail has ever been made by this code", which
stopped being true on 2026-09-03: both adapters call rails, and since Step 4 the
worker drives a confirmed intent all the way to `succeeded`. **Every one of
those calls went to a WireMock host — no real rail has ever been called, and no
money has ever moved.**_

_Refreshed 2026-09-03 (evening), and stated precisely in both directions._
**What is proven**, by tests that fail if it breaks, on a developer machine and
in CI: a merchant authenticates on `/v1` with `private_key_jwt`, creates a
`PaymentIntent`, confirms it, the worker polls the charge and drives the intent
to `succeeded` with nobody touching it, and a signed `payment_intent.succeeded`
webhook is delivered and verified — by both shipping SDKs and by the official
`stripe` package's own `constructEvent`. A payer's own browser can drive the
push half of that through `@vaam-apps/vpay-stripe-js` without a merchant credential.
**What is not proven, and is what stands between this and taking money**: every
rail in every one of those runs is a `wiremock/wiremock` host, **no real rail
has ever been called**; every webhook receiver is one too, **no merchant
endpoint has ever been POSTed to**; and **nothing has ever run on a cluster**.
Everything below explains what stands between here and that no longer being
true.

| #   | Phase                                                | Status, with the evidence that backs it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --- | ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Foundations                                          | ✅ Complete — seven commits through `932d8a4`; the **28** migrations in `backends/migrations/` apply cleanly and their constraints are proven to fire (`postgres_smoke.rs`; 26 when this row was written, 27 with Step 8's callback index and 28 with Step 9's `checkout_sessions`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| 2   | Authentication — merchant (`/v1`)                    | ✅ Delivered 2026-09-02 (Step 1, PR #15) — the OP is mounted at `/v1/oauth` and `AuthenticatedMerchant` gates the whole `/v1` nest; 7 `merchant_token_flow` tests. _`docs/status.md`'s own "Merchant auth" row is still 🟡 and names its own trigger — "when the CI `rust` job runs them green" — which has since happened on `master` (run `33792230584`) without that row being re-measured_                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| 2b  | Authentication — dashboard login (`/dash/v1`)        | 🟡 **The login and the pages are built; key rotation is not** (2026-09-07, [ADR-0017](adr/0017-staff-authentication.md) and exp28 — see the two amendments at the end of the phase). ~~The login is built; the pages are not.~~ ~~⛔ Still not started, and the goal below is unmet: no login has ever been performed.~~ ~~no `/dash/v1` route of any kind~~ — corrected 2026-09-06 (exp23): two `GET` _resource_ routes are now mounted and tenant-bound, which is a resource server with **no issuer**, since no grant this deployment serves can mint a token for them. Still no `/login`, no `/authorize`, no `SessionStore`. **A second blocker, larger than item 3 below, was found and recorded that day**: `handle_authorize` takes an already-authenticated `Identity` as a parameter, and how a human staff member proves who they are has never been decided anywhere in this repository. Split out of Phase 2 on 2026-09-02                          |
| 3   | Payment API (`/v1`)                                  | ✅ Delivered 2026-09-02→03 (Steps 2–3, PRs #16–#17) — create / retrieve / list / cancel / confirm, form-encoded, idempotent and merchant-scoped, with `confirm` moving the intent to `processing` or `requires_action`. **Against WireMock rails**, which is why the matching `docs/status.md` rows are 🟡                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| 4   | The rails                                            | → **split 2026-09-03.** **4a** ✅ delivered (Step 3, PR #17) — both adapters pass the one shared conformance suite, 26 tests, 0 `#[ignore]`s, every one of them against a `wiremock/wiremock` container. **4b** (push-rail recovery) delivered inside Phase 5 (Step 4, PR #18). The two headings below are current; this row is the pre-split one                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| 5   | The worker                                           | 🟡 In progress — the job loop, the poll ladder, recovery and settlement landed 2026-09-03 (Step 4, PR #18) against WireMock rails, and a confirmed intent reaches `succeeded` unattended. **Updated 2026-09-04 (Step 8):** ~~the callback route (`POST /provider/{code}/callback`) … did not~~ — **it exists now** (lane C), and ~~the "crash tests" kill no process~~ — **`worker_kill9.rs` `SIGKILL`s the shipping worker and the shipping server** (lane D), so two of the three kill points are caused rather than written. Lane G additionally fixed a `500` on confirm that this step's demo found. **Still 🟡:** prompt expiry is unbuilt, kill point 1 is still written rather than caused, Orange is not in the kill test, no rail has ever called the callback route, and every rail here is a WireMock host                                                                                                                                           |
| 5b  | Stripe SDK compatibility on `/v1`                    | ✅ Delivered 2026-09-03 (PR #20) — the real `stripe@22.6.1` package driven out of process against the compose stack: `sdks/stripe-compat`, **25 cases, 0 skipped**, run by CI's `e2e (compose)` job; [flows/stripe-sdk-compat.md](flows/stripe-sdk-compat.md). _`docs/status.md`'s row is 🟡 for the standing limit — the rail and the receiver are both WireMock hosts_                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| 5c  | Stripe.js-compatible browser checkout                | ✅ Delivered 2026-09-03 (PR #22), **push rails only** — `/v1/browser` plus `@vaam-apps/vpay-stripe-js`, no merchant credential in the browser, proven by `checkout.cy.ts` against the compose stack. ~~**The redirect return trip is a named gap**: there is no bounce endpoint, so an Orange checkout must not be shipped on this package~~ — **retired 2026-09-04 by Phase 5d**: the rail is told a per-charge return URL and vpay serves the page that receives the payer ([flows/browser-checkout.md](flows/browser-checkout.md), [flows/hosted-checkout.md](flows/hosted-checkout.md))                                                                                                                                                                                                                                                                                                                                                                      |
| 5d  | Hosted and embedded checkout                         | ✅ Delivered 2026-09-04 (Step 9), **against WireMock rails** — a `checkout.session` object (`cs_…`, migration `0028`) with a hosted mode (vpay mints a `url`) and an embedded mode (the merchant frames vpay's page), `frontends/apps/checkout` in French and English, the redirect return trip that closes 5c's named gap, `initEmbeddedCheckout` in `@vaam-apps/vpay-stripe-js` and `checkout.sessions` in both merchant SDKs, a fourth image and a Helm workload, and `examples/shop` — a merchant site a human can buy from. Proven in a real browser end to end by `shop-hosted.cy.ts` and `shop-embedded.cy.ts`. [flows/hosted-checkout.md](flows/hosted-checkout.md), [runbooks/checkout.md](runbooks/checkout.md). _`docs/status.md`'s rows are 🟡 where this is ✅, and for reasons this row must not hide: no browser has been observed enforcing vpay's `frame-ancestors` (Cypress strips it), no pod has ever run the page, and the rails are stubs_ |
| 6   | Webhooks                                             | 🟡 In progress — the outbox drain, signing and delivery landed 2026-09-03 (Step 5, PR #19) against a WireMock receiver. **Updated 2026-09-04 (Step 8, lane B):** ~~there is no SSRF protection of any kind~~ — **a runtime egress guard exists**, `vpay_worker::ssrf`, which resolves each endpoint's host once, refuses every non-public address in both families and pins the connection to what it classified; boot-time `validate_host` is still only a stub-host guard and never was an address check. **Still 🟡:** no merchant endpoint has ever been POSTed to, the guard has never refused a real one, delivery is unordered, replaying an exhausted delivery is a hand-written transaction, and the pin cost the shared connection pool                                                                                                                                                                                                                |
| 7   | Operability — including Step 6's groundwork (PR #21) | 🟡 In progress — **the repo is here now**, on Step 7 (see below). No longer "blocked by environment": CI runs the compose stack, both Cypress specs and the stripe-node conformance suite; the Helm chart is linted, rendered and kubeconform-checked by CI's `deploy` job; `release.yml` has built and signed images on every push to `master` since Step 6; ~~`just demo` runs seven steps~~ — **as of 2026-09-04 (Step 8, lane A) `just demo` is four steps whose fourth is six payments across both rails**, with `demo-up`/`demo-walk`/`demo-status`/`demo-down` split out, two stacks able to coexist on one machine, and [runbooks/demo.md](runbooks/demo.md) as the procedure with real pasted output. **Not done: no cluster, no Prometheus scrape, no runbook walked against a real fault, no real rail behind any of it — and the demo has not yet been run on the merged Step 8 gate branch**                                                        |

**Where the repo is now, 2026-09-03 (evening).** Steps 5b, 6 and 5c landed on
`master` in that order (PRs #20, #21, #22), followed by a frontend dependency
audit and a CI timeout fix (PRs #23, #24). The work in flight is **Step 7, the
cleanup rework** ([plans/2026-09-03-step7-cleanup-rework.md](plans/2026-09-03-step7-cleanup-rework.md)),
on a branch and unmerged: repository traits hiding the `sqlx` implementations
behind an interface, a real `#[source]` on the adapter transport/malformed
errors so a rail failure keeps its cause, doctests so the documentation cannot
lie, and flow prose moved out of long module comments into `.md` files. **It
moves no capability.** Nothing in the table above changes when it lands, and the
conformance, integration and SDK suites are the guard that says so.

**On the markers in that table.** ✅ means "delivered, with a test that would
fail if it broke" — it does not mean "safe with money". Every ✅ above is proven
against WireMock rails and a WireMock receiver, which is why most of the
matching per-feature rows in [`docs/status.md`](status.md) are 🟡. Where this
page and that one disagree, that one wins.

Phases 3 and 4 are listed in build order but genuinely interleave — see
Phase 3's note. `docs/status.md`'s own "What would have to be true to call
this an MVP" list is a numbered checklist, not a stated build sequence; read
that way it doesn't conflict with the order below. Read as a sequence it
would, on two points — flagged inline at Phases 3 and 2.

---

## The phases

Each phase was a section of this page until 2026-09-11, when it was 1 504 lines.
They are one page each now, **verbatim** — including every struck-through claim,
every "Open —" maintainer question and every dated amendment, because on this
page those are most of the content. Only the relative links changed.

| Phase                                     | Page                                                                             |
| ----------------------------------------- | -------------------------------------------------------------------------------- |
| 1 — Foundations                           | [roadmap/phase-1-foundations.md](roadmap/phase-1-foundations.md)                 |
| 2 — Authentication (merchant `/v1`)       | [roadmap/phase-2-authentication.md](roadmap/phase-2-authentication.md)           |
| 2b — Dashboard login (`/dash/v1`)         | [roadmap/phase-2b-dashboard-login.md](roadmap/phase-2b-dashboard-login.md)       |
| 3 — The payment API (`/v1`)               | [roadmap/phase-3-payment-api.md](roadmap/phase-3-payment-api.md)                 |
| 4a, 4b — The rail adapters, push recovery | [roadmap/phase-4-rail-adapters.md](roadmap/phase-4-rail-adapters.md)             |
| 5, 5d — The worker, hosted checkout       | [roadmap/phase-5-worker-and-checkout.md](roadmap/phase-5-worker-and-checkout.md) |
| 6 — Webhooks                              | [roadmap/phase-6-webhooks.md](roadmap/phase-6-webhooks.md)                       |
| 7 — Operability, and the dated addenda    | [roadmap/phase-7-operability.md](roadmap/phase-7-operability.md)                 |

Phases 5b and 5c are rows in the table above and never had a section of their
own; the page they are described on is Phase 5's.

_Two pointers in the text above no longer point at this page. The snapshot line
says the page was last refreshed by "the Step 9 addendum at the foot of this
page": that addendum, and the four others, are at the foot of
[roadmap/phase-7-operability.md](roadmap/phase-7-operability.md). And
"everything below" is everything on the eight pages in the table._

**The open maintainer questions are in Phase 2**
([roadmap/phase-2-authentication.md](roadmap/phase-2-authentication.md)) — the
access-token TTL and its revocation mitigation, the signing-key rotation overlap
window, the Keycloak/ZITADEL comparison, and the revocation-endpoint gap on
`authkestra-op` itself — **and one more in Phase 2b**
([roadmap/phase-2b-dashboard-login.md](roadmap/phase-2b-dashboard-login.md)):
whether to build the dashboard on CrateStack's refine integration. They are
questions reserved for the maintainer, and a default chosen in code is not an
answer to one.
