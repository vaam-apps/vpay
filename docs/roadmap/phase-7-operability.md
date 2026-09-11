# Roadmap — Phase 7: Operability, and the dated addenda

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

The five dated addenda at the foot of the old page are at the foot of this one.

## Phase 7 — Operability

**Goal.** The whole stack (server, worker, Postgres, two rail stubs) starts
with one command, the e2e specs run green against it, and runbooks have
been walked against a real fault, not just written.

**Status.** _Corrected 2026-09-02:_ this used to say "blocked by
environment, not by unwritten code," and that was wrong on two counts. The
`ci` workflow had run five times and failed five times at the same
self-inflicted step (`CYPRESS_INSTALL_BINARY: 0` set for the very job that
runs Cypress), and the compose stack could not have booted even with
registry access — nothing supplied the `VPAY_CONFIG` both binaries require,
and the image did not contain a config file to name. Both are fixed; see
`docs/status.md`'s "GitHub Actions" and "Docker / compose" entries for what
is proven by what. The environment description that follows is still true
of the original authoring machine and is kept for context. Docker Hub is
unreachable from this development machine: `docker pull alpine:3.22` did
not complete in five minutes, and `rust:1.95.0-alpine3.22`,
`node:22-alpine`, `wiremock/wiremock:3.9.2` and `docker/dockerfile:1` are
all missing from the local image cache and unpullable here. Only
`postgres:16-alpine` is cached, which is why Phases 1–4's Postgres-backed
tests work fine while this phase's own exit criteria cannot be met on this
machine at all. Cypress is blocked the same way: its binary needs
`pnpm exec cypress install` against a CDN this environment cannot reach.

**This blocker is independent of the other phases and can be lifted at any
time on a machine with real registry/CDN access** — it does not need
Phases 2–6 to finish first to start being _attempted_. What it does need
those phases for is a _meaningful_ green run: the one Cypress spec that
exists today (`frontends/tests/e2e/cypress/e2e/dashboard.cy.ts`) only
exercises the dashboard's scaffold notice, not a real payment flow, so
running it green proves the environment works before it proves anything
about payments.

> **Addendum, 2026-09-03 (evening) — the environment blocker is lifted, and
> what is left of this phase is a cluster nobody has rather than a registry
> nobody could reach.** The 2026-09-02 correction above stands and is not
> deleted; the paragraph before this one describes the _original_ authoring
> machine and is still true of it. What Step 6 (PR #21) and the CI runs since
> then changed:
>
> - **The compose stack runs in CI, on every pull request and every push.**
>   `ci`'s `e2e (compose)` job builds both images, brings the stack up against
>   WireMock rails, and runs **both Cypress specs** — `dashboard.cy.ts`'s three
>   tests and Step 5c's `checkout.cy.ts`, which drives a real browser through
>   confirm → processing → succeeded — plus the stripe-node conformance suite.
>   Green on `master` at run `33792230584`. The one spec this phase's Status
>   paragraph above calls "only the dashboard's scaffold notice" is no longer
>   the only spec.
> - **The Helm chart exists and CI checks it.** `deploy/helm/vpay`, fifteen
>   named template guards, `helm lint` / `helm template` / `kubeconform`
>   through `just helm-check`, run by CI's `deploy (helm chart)` job — the job
>   runs the recipe rather than a copy of it, so the two cannot drift. 20
>   resources validated, 0 invalid; every guard fires by name.
> - **The release pipeline has run, and signed what it built.** `release.yml`
>   builds three images across two native architectures, merges each into a
>   manifest list and signs it with keyless cosign. Four runs on `master`
>   since Step 6 landed — `33772512791`, `33784613048`, `33789060270`,
>   `33792230539` — nine jobs each, all green. **The `edge` images exist
>   because of those runs and for no other reason.** No `v*` tag has been
>   pushed, so the semver tag path is still unexercised; and the first push
>   creates each GHCR package **private**, so nobody outside CI can pull one
>   until a human flips the visibility by hand
>   ([runbooks/release.md](../runbooks/release.md)).
> - **`just demo` runs seven steps** end to end against the containerised
>   stack, the seventh being a signed `payment_intent.succeeded` read back out
>   of a WireMock receiver's own request journal and verified with the
>   shipping SDK. _(**Corrected 2026-09-04, Step 8 lane A:** four steps now,
>   the fourth being six payments across both rails, each with its own signed
>   webhook.)_
> - **Both binaries have an observability listener** on a second port —
>   `/livez` and `/metrics`, twelve metric names with one seam each.
>
> **What is still not done is the whole of this phase's Definition of done.**
> **No cluster has ever run the chart** — not a real one, not kind — so
> nothing above is evidence about scheduling, admission, probe behaviour,
> `readOnlyRootFilesystem`, NetworkPolicy enforcement, PDB behaviour during a
> drain, or whether an ingress controller honours the `limit-rps` annotation
> CI greps for. **No Prometheus has ever scraped a vpay process**, so all five
> alert rules are unevaluated and every threshold is provisional rather than
> derived from traffic. **No runbook has been walked against a real fault** —
> eight are written, none has been followed against a deployment, because no
> deployment exists. No backup has ever been taken
> ([ADR-0013](../adr/0013-database-backups-and-retention.md) is _proposed_). And
> under all of it, no real rail has ever been called. See
> [flows/deployment.md](../flows/deployment.md) and `docs/status.md`'s
> Infrastructure rows for the per-artefact account.

**Scope.**

- Build `backends/Dockerfile` and `frontends/Dockerfile` to completion (both
  rewritten this cycle — musl target, non-root UID 65532, `.dockerignore` —
  neither ever built).
- Bring up `compose.yml`/`compose.e2e.yml`.
- Install the Cypress binary and run the existing spec(s); write the
  payment-flow specs that give this phase something meaningful to prove
  once Phases 3–6 exist.
- Walk each `docs/runbooks/*` document against a real, not imagined,
  instance of the condition it addresses.

**Definition of done.** `just test-e2e` exits 0 against the compose stack;
both Dockerfiles build to completion; each runbook's steps have been
exercised at least once for real.

**Unblocks.** Nothing further — this is "can we run what we built,"
verified, not a functional dependency of any other phase.

**Risks carried by this phase.** The environment blocker itself: until
Docker Hub / the Cypress CDN is reachable from wherever this work continues,
this phase cannot be closed regardless of how much other work is done.

---

_Written 2026-08-11 against `master` at `33f2913`. If this page and the code
disagree, the code — and `docs/status.md`'s machine-checked account of it —
is correct._

**Addendum, 2026-09-02.** Two things landed that this snapshot does not
place in a phase: the merchant SDKs (`sdks/rust`, `sdks/nodejs`) implement the
_client_ half of Phase 3's `/v1` contract — pinned down in
[`docs/flows/merchant-auth.md`](../flows/merchant-auth.md) — ahead of any server
route existing, so Phase 3 now has a consumer to build against; and the
dependency floor moved (`authkestra-*` 0.5.4 → 0.7.1 with migration `0013`,
CrateStack re-verified at 0.10.1). See `docs/status.md` for the row-by-row
account.

**Second addendum, 2026-09-02 (Step 1).** Phase 2's "assembled but not
mounted" status is no longer accurate for the merchant half: the OP is
mounted at `/v1/oauth` and `/v1` has an authentication boundary in front of
it. The dashboard half is unbuilt and is now Phase 2b. The phase's own
Status block carries the detail, including the one thing that matters most
about the evidence — the six integration tests that cover the flow have run
once, manually, against a scratch database, and never under Docker or in
CI. Phases 3–7 are untouched.

**Third addendum, 2026-09-03 (evening).** Eight pull requests merged to
`master` in one day, and the table at the top of this page is rewritten against
them rather than patched around them: **#17** the rail adapters (Step 3),
**#18** the worker — job queue, poll ladder, recovery, settlement (Step 4),
**#19** signed webhook delivery with its retry ladder and the events API
(Step 5), **#20** compatibility with the official `stripe` package and the
`sdks/stripe-compat` conformance suite (Step 5b), **#21** the Helm chart, the
release pipeline, the observability listener, twelve metrics and four new
runbooks — eight in total (Step 6), **#22** the Stripe.js-compatible browser checkout — `/v1/browser`,
`@vaam-apps/vpay-stripe-js`, `examples/checkout-browser` (Step 5c), **#23** a frontend
dependency audit clearing 21 Dependabot alerts and adding `just audit-web` as a
gate, and **#24** a raised Cypress verify budget in the CI e2e job. **Step 7,
the cleanup rework, is on a branch and unmerged**; it moves no capability.

**The caveats none of those merges touched**, and which this page must not be
read as having retired: no HTTP call to a **real** rail has ever been made —
every payment in this repository's history settled against a
`wiremock/wiremock` host answering the way these documents say a rail answers;
**no merchant endpoint has ever been POSTed to** and ~~there is no SSRF
protection on webhook destinations~~ _(retired 2026-09-04 by Step 8's egress
guard — see the fourth addendum)_; **no cluster has run the Helm chart and no
Prometheus has scraped a vpay process**; the GHCR packages those release runs
created are **private** — nothing in this repository can publish them, and
making one pullable is a one-time change a human makes in the package's own
settings ([runbooks/release.md](../runbooks/release.md)); there is still no
`/dash/v1` login; and **no runbook has been followed against a real fault**.
[`docs/status.md`](../status.md) records each of those per feature, and it wins.

**Fourth addendum, 2026-09-04 (Step 8, the production gate).** One branch,
`claude/step8-production-gate`, six lanes plus a seventh the step's own reviews
produced, and the point of the step was to close every gap that does **not**
need a real rail credential:

- **Lane B — the runtime egress guard.** `vpay_worker::ssrf` on every webhook
  delivery: resolve once, classify every answered address in both families
  (mapped and compatible spellings included), refuse the delivery permanently
  if any is non-public, and pin the client to what was classified. This closed
  the repository's only ⛔ on a shipping path. Three residuals remain, named in
  Phase 6's risks.
- **Lane C — the rail callback route.** `POST /provider/{code}/callback`, plus
  migration `0027`'s index behind its unauthenticated lookup. It writes no
  charge or intent state: it pulls the charge's existing poll job forward.
  Orange's `notif_token` is still not compared against the stored one, and the
  route discards `CallbackRef::ref_extra` rather than trusting it.
- **Lane D — a real `SIGKILL`.** The shipping `vpay-worker-bin` killed
  mid-status-query and the shipping `vpay-server` killed mid-`requesttopay`,
  against real Postgres and WireMock. Kill point 1 is still written rather than
  caused, and Orange is not exercised.
- **Lane G — the confirm/worker race**, which was _not_ in the plan. Lane A's
  demo produced a `500 api_error` on confirm in four of six runs: the worker
  claimed the poll job the confirm had just committed and applied the
  crash-recovery table to a charge whose process had not crashed. The fix is a
  minimum charge age in `recovery_step`. **A demo found a real defect in a
  payment path, which is the strongest argument this step makes for having a
  demo at all.**
- **Lane A — the demo.** Six payments, both rails, three outcomes each, every
  outcome steered at the stub by a field a merchant controls; split recipes;
  two stacks on one machine at the Compose layer; and
  [runbooks/demo.md](../runbooks/demo.md) with real pasted output. `.e2e/` is
  still shared between stacks and that is a named gap.
- **Lane F — SDK parity**, added at the user's request outside the original
  plan: [ADR-0015](../adr/0015-sdk-parity.md), [docs/sdks/parity.md](../sdks/parity.md)
  and `cargo xtask verify-sdk-parity` in `just verify`. 267 proving tests named,
  24 dated gaps, **none of them closed by this step**.

- **Lane H — the correctness review's four confirmed findings**, 2026-09-04,
  after the six lanes above had merged. (1) The recovery window compared
  Postgres' `charges.created_at` against the **worker host's** clock, so a
  worker sixty seconds fast made lane G's guard a silent no-op on exactly the
  deployment whose fleet clocks had drifted; the age now comes from
  `Charges::get_by_id_as_of`, which selects `now()` beside the row, and
  `recovery_step` takes durations rather than instants so there is no parameter
  left for a caller to read off the wrong clock. (2) `RecoveryAction::Wait`
  rescheduled at `poll_delay(0)`, and every reschedule spends a rung, so a
  genuinely crashed charge burned six of them waiting the window out; `Wait`
  now carries `window - age`, clamped, and reschedules once, so the first real
  recovery rung is `poll_delay(1)` — twenty seconds. (3) The callback route's
  pull-forward matched any unleased future job, so an anonymous caller drove
  rail traffic at their own rate and the module doc said the opposite; it now
  refuses a job already due within `PULL_FORWARD_FLOOR` — ten seconds, the
  ladder's own fastest rung — and the cost is stated rather than implied: a
  callback arriving while the charge sits on that first rung no longer settles
  it early. (4) The egress classifier let `192.88.99.0/24`, `2001:1::/32`,
  `2001:2::/48` and `2001:20::/28` through as ordinary public addresses; all
  four are refused now. [plans/step8-notes/lane-h.md](../plans/step8-notes/lane-h.md)
  is the record, including the two findings it deliberately did not fix.

**What Step 8 did not do, stated so it is a decision and not an omission.** No
real rail was called and the "do not deploy" banner is untouched. The dashboard
and `/dash/v1` are unbuilt, and the demo says why: there is no data source to
show. The Orange redirect return trip is still missing, so a redirect-rail
checkout must not ship on `@vaam-apps/vpay-stripe-js`. `mtn_momo::refund` is still the
one `NotImplemented` token. `just demo` has **not** been run on the merged gate
branch. And `charges.provider_reference_id` is still not `UNIQUE` — lane C
recommends it and deliberately left the decision to the maintainer.
**Lane H adds five more, each named rather than rounded off:** an SSRF-refused
webhook delivery is exhausted on its first attempt and there is **no replay
path** (finding 5); the callback route's two `202`s are distinguishable in
_time_, because the known-reference path runs a transaction the unknown one
does not (finding 7); **no rate limit** was added to that route, per charge or
per source, and the pull-forward floor is not one; `scan_live_charges` still
computes its ten-minute cutoff from the worker host's clock and compares it
against a column Postgres wrote — the same defect class as the recovery
window's, in its mildest direction; and **`2001::/23` as a whole is still
deliverable**, because refusing IANA's whole protocol-assignments block is a
wider call than the review asked for and is left to the maintainer.

**Issue #11 is answered item by item in the PR and is not auto-closed.** Two of
its seven items are incomplete (the `.e2e/` half of the concurrency item, and
the walkthrough's own history), and a closed issue that is not fixed is worse
than an open one.

**Fifth addendum, 2026-09-04 (Step 9, hosted checkout).** One branch,
`claude/step9-hosted-checkout`, twelve lanes, and it delivered the thing the
maintainer asked for in the sentence at the head of Phase 5d. What that phase
is and what it carries are written out there; this addendum records the three
things about the _step_ that a phase description would flatten.

- **A defect the demo shop found, which three lanes had walked past.** No
  merchant server that reaches vpay by an internal URL could authenticate at
  all. Both SDKs signed the client assertion's `aud` with the token endpoint
  they were about to POST to and derived both from `baseUrl`, while vpay's OP
  derives its issuer solely from `deployment.public_base_url` — so
  `http://vpay-server:8080/v1/oauth/token` was compared against
  `http://localhost:8080/v1/oauth/token` and refused with a bare
  `invalid_client` / `InvalidAudience`, with the signature, the `client_id`,
  the `kid` and the lifetime all correct. It survived because lane 7 never
  spoke to a running vpay, lane 4 brought the shop up but never clicked through
  it, and every other consumer runs _on the host_, where the public issuer
  happens to be right. Lane 6 found it by putting a merchant's own server
  inside the compose network; lane 5b fixed it with a third setting
  (`assertionAudience` / `ClientBuilder::assertion_audience`), proven by the
  real pinned `authkestra_op` verifier refusing and then accepting the same
  client. **ADR-0010 does not change** — the SDKs had conflated two of its
  three strings.
- **`just test-e2e` could not have passed since Step 5c**, and nobody had
  noticed because CI's `e2e` job was the only place `checkout.cy.ts` ever ran.
  The recipe brought up a stack with no merchant anybody holds a key for. It is
  fixed, and this is the first item on `docs/status.md`'s MVP list to move to
  met since item 1.
- **Two reviews and a second round.** Correctness/money-and-secrets and
  conventions/blast-radius on the merged gate, then a second round after the
  first remediation, and every remediation reviewed. Lanes 1b, 3b, 5b and r2
  are what came out of them, and the reviews found things the lanes' own tests
  could not: a return-URL lookup that answered `None` for every intent, a
  browser read that never stopped issuing a credential, a page that refused to
  paint when a merchant had no display name, a staleness check that was a
  presence proxy, and five demo publications on `0.0.0.0`.

**What Step 9 did not do, stated so it is a decision and not an omission.** No
real rail was called and the "do not deploy" banner is untouched: a browser now
walks an entire checkout, and every rail in that walk is a `wiremock/wiremock`
container. **No browser has been observed enforcing vpay's `frame-ancestors`** —
Cypress strips the header, so it is asserted as sent, and the refusal a browser
was seen performing is the checkout app's own origin check. No pod has run the
page. There is still no rate limiting in front of either unauthenticated
surface. The demo shop's `ZenStackShopStore` has no automated coverage of its
own. `checkout_not_configured` still answers `500` where `503` would be
truthful, and moving it is an ADR-level change **left to the maintainer** —
along with whether `checkout.public_base_url` should be a separate host or a
path under the API host in production, and whether a session may create its
PaymentIntent inline in a later step. The dashboard is untouched and `/dash/v1`
is still unbuilt.
