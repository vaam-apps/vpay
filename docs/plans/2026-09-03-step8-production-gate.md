<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 8 — the production gate: demo on both rails, SSRF guard, rail callbacks, a real kill test

User instruction (verbatim, 2026-09-03, evening): "What's the next step to make this vpay production
ready? Do a plan / todo list and propose me. [...] Push and open the PR once the gate is green. Merge
directly too, since you can test locally directly. Make it production ready and do a demo locally. Take
all decisions yourself. [...] The testing should be as simple as possible too: using just + docker
compose, with clear steps, and a demo app. [...] For testing & building, create a dedicated tiny
multipass vm."

## Where the repo is when this step starts

`origin/master` at `ca94eac` (PR #24). Steps 0–6, 5b and 5c are merged. Step 7 (the cleanup rework,
`claude/step7-cleanup`, 35 commits, 114 files) is in flight in a sibling session and unmerged; it moves
no capability by its own charter, so this step is planned against `master` and rebased onto Step 7 if it
lands first. Issue #11 ("Demo setup: one command, stub rails, one payment end to end") is the only open
issue and is almost verbatim the user's ask.

**What "production ready" can mean here, stated before any code.** No real rail credential exists and
none is in scope (issue #11's own "Out of scope"). So the bar is: every gap that does *not* need a real
rail is closed, proven by a test that fails when it breaks, and demonstrable from nothing with `just`
and Docker Compose. Everything that *does* need a real rail stays a named ⛔/🟡 row in `docs/status.md`,
and the "do not deploy" banner stays until a real rail has been called. This step does not remove it.

## The four functional gaps this step closes, and why these four

From `docs/roadmap.md` (Phases 5–7), `docs/status.md`, `docs/flows/crash-safety.md` and issue #11:

| # | Gap, as the repo states it today | Why it blocks a deployment even on stub rails |
|---|---|---|
| A | Issue #11: `just demo` is MTN-push-only, success-only; `compose.demo.yml` hardcodes `name: vpay`; no `docs/runbooks/demo.md`; dashboard not addressed; the two hazards not restated in one place | It is the artefact an integrator (Skyport, ADR-028) writes against. A demo that shows one rail and one outcome under-specifies the contract |
| B | `docs/status.md` row "Webhook URL validation — boot-time only, **no runtime SSRF filtering**" is the repo's only ⛔ on a shipping path | A merchant-supplied URL is POSTed to from inside the deployment's network. Without a resolve-time address check and pinning, a merchant can reach `169.254.169.254`, the Postgres service, or any peer |
| C | `docs/flows/adapter-mtn-momo.md`: "**No callback route exists.**" Both adapters implement `parse_callback`; nothing mounts a route, so `X-Callback-Url`/`notif_url` are never sent and settlement is polling-only | Both rails' documented protocol is push-then-callback. Polling alone works (Step 4 proves it) but a real MTN sandbox registration *requires* a callback host, and the poll ladder's first hop is seconds, not instant |
| D | `docs/flows/crash-safety.md`: "**Nothing in this repository kills a process.**" The three kill points are proven by writing the state a crash leaves, not by a crash | The crash-safety invariant is the one a payments worker is trusted for. A recovery table proven against hand-written state is a belief about signals, not evidence |

Explicitly **not** in this step, each named so it is a decision and not an omission:

- **Dashboard login and `/dash/v1` (Phase 2b).** Not started, its own phase, and it moves no payment.
  Issue #11 permits "the issue states that the dashboard is out of scope for this demo and why" —
  the demo runbook states it, and the reason is that there is no data source to show, not that the
  screen is unfinished.
- **The Orange redirect return trip (browser-checkout "redirect gap").** A product contract for the
  bounce URL is needed first; the API-driven merchant path (`next_action.redirect_to_url`) is
  demonstrated instead.
- **Ordered webhook delivery, replay as a procedure, `mtn_momo::refund`.** Unchanged, still listed.
- **Real rails.** Nothing here calls one; the banner stands.

## Lanes

Phase A of Step 7 is not a dependency; every lane is additive against `master` and lands in **new
files** wherever possible so the rebase onto Step 7's refactor is mechanical. Lanes run in parallel in
their own worktrees on their own branches, and are merged into `claude/step8-production-gate` by the
orchestrator in the order D, B, C, A (smallest blast radius first). `docs/status.md`, `docs/roadmap.md`
and `docs/flows/*.md` are edited **only by the orchestrator at the end** (lane E) from per-lane notes,
so four lanes never fight over one table.

### Lane A — the demo (issue #11), `examples/merchant-demo` + `compose.demo.yml` + `justfile`

1. **Both rails, three outcomes each where the rail has them.** The walkthrough grows from seven
   steps to a table: MTN push → `succeeded`; MTN push → `failed` (payer decline, the
   `requesttopay-status.json` `…0f01` mapping); MTN push → prompt expiry (`EXPIRED`, if the mapping
   tree has it — add it to the shared `backends/tests/conformance/wiremock` tree if not, keyed by
   documentation MSISDN like the existing e2e scenario, never by rewriting stored state in the demo);
   Orange redirect → `requires_action` with the redirect URL shown → `succeeded`; Orange → `failed`.
   Each outcome prints the intent's public fields, the charge's `failure_code` where there is one, and
   the webhook the receiver journal recorded for it (`payment_intent.succeeded` /
   `payment_intent.payment_failed`), signature-verified with the SDK as step 7 already does.
2. **No collisions across concurrent runs.** `compose.demo.yml`'s `name:` reads
   `${VPAY_DEMO_PROJECT}`; `just demo` derives one project name and its two host ports from three
   `just` variables (`demo_project`, `demo_port`, `demo_receiver_port`), and `just demo-down` uses the
   same variables so it tears down what `just demo` started and nothing else. Two demos with
   different variables coexist on one machine. Default project name stays `vpay-demo` so the
   single-user path needs no flags.
3. **Split the recipe so the walkthrough is re-runnable.** `just demo` = keys + `up --wait` +
   walkthrough; `just demo-up`, `just demo-walk`, `just demo-status`, `just demo-down` exist
   separately. Readiness is `--wait` on healthchecks, not a fixed sleep.
4. **`docs/runbooks/demo.md`** — the exact commands and the expected output, pasted from a real run
   (RAN, not narrated), with a "what this proves / what it does not" section, the dashboard
   out-of-scope statement, and the two hazards restated: rustls `CryptoProvider` (closed 2026-09-02,
   cite the row) and RUSTSEC-2023-0071 (open, `ignore`d in `deny.toml`, cite the row). README and
   `docs/runbooks/README.md` link it. Check `docs/status.md`'s authkestra pin claim against
   `Cargo.toml` and record the answer in the lane notes.
5. Lane notes → `docs/plans/step8-notes/lane-a.md` (status rows to add/change, verbatim).

### Lane B — runtime SSRF guard on webhook delivery, `vpay-worker`

1. New module `vpay_worker::ssrf` (or `vpay_provider::http::egress` if the adapters should share
   it later — decide by reading `handle_deliver`; the default is `vpay-worker`, one caller today):
   resolve the endpoint host, reject every address that is loopback, private (RFC 1918), link-local
   (`169.254/16`, `fe80::/10`), CGNAT (`100.64/10`), multicast, unspecified, or IPv4-mapped IPv6
   of any of those; reject a scheme other than `http`/`https`; then **pin** the connection to the
   vetted address (`reqwest::ClientBuilder::resolve_to_addrs`) so a DNS rebind between check and
   connect cannot redirect it, and **disable redirects** (`Policy::none()`) on the delivery client
   so a `302` cannot bounce to an internal address either.
2. A `webhooks.allow_private_targets: bool` (default `false`) in `vpay-config` — the compose stack's
   `wiremock-webhook` is a private address, so the `sandbox` and `demo` profiles set it `true` in
   YAML and livemode refuses to boot with it `true` (add the rule beside `validate_webhook_url`;
   ADR-0006: a profile selects a config file, never a code path).
3. A refused target is a **permanent** delivery failure with a reason a merchant can read
   (`ssrf_blocked`), never retried, recorded the same way `record_failure` records the others.
4. Tests: unit tests on the classifier (each range, both IP families, mapped v6); a container-backed
   integration test that a delivery to a private address is refused and recorded permanent while the
   same address with `allow_private_targets: true` is delivered; a config test that livemode +
   `true` fails validation. Guard-failure proof: bypass the classifier and show the private delivery
   goes through. Prefer no new dependency; if one is needed, name it and its licence in the notes.
5. Lane notes → `docs/plans/step8-notes/lane-b.md`.

### Lane C — the rail callback route, `vpay-api` + adapters

1. New module `vpay_api::provider_callback` mounting `POST /provider/{code}/callback`
   (unauthenticated by necessity — the rails sign nothing; **callbacks are hints**, AGENTS.md):
   look the adapter up by `code`, `parse_callback(body)` → `CallbackRef`, find the charge by
   provider reference (add a `vpay_db::charges::get_by_provider_reference` if none exists — a new
   `pub async fn`, not an edit to an existing one), and enqueue an **immediate** `PollCharge` job
   through the existing `enqueue_in_tx` with the same dedupe key the ladder uses, so the worker
   settles it now instead of at the next rung. The route never writes charge or intent state.
   Unknown code → 404; unparseable body → 400 and a `warn`; unknown reference → 202 anyway (the
   rail must not retry forever; log it). Body size bounded like every other route.
2. `X-Callback-Url` (MTN) and `notif_url` (Orange) are now sent on `submit`, derived from
   `deployment.public_base_url` + `/provider/{code}/callback`. Update the conformance mappings so
   the header/field is asserted present — the shared WireMock tree is the contract.
3. Tests: a container-backed test that POSTs each rail's documented callback body to the route and
   observes the charge settle **before** the poll ladder's next rung would have fired; the three
   refusal cases; and the guard-failure proof that an unparseable body cannot enqueue anything.
4. Lane notes → `docs/plans/step8-notes/lane-c.md`, including the flow-doc lines to retire
   ("No callback route exists", both adapter docs).

### Lane D — a real kill test, `backends/tests/integration/tests/worker_kill9.rs`

1. Spawn the **shipping** `vpay-worker-bin` (`CARGO_BIN_EXE_…`, a real OS process) against a
   real Postgres + WireMock; drive a confirm whose stub is slow to answer the status query
   (WireMock `fixedDelayMilliseconds`); `SIGKILL` the worker while the poll is in flight (a real
   `Child::kill()`, no destructor runs); assert the job's lease is the only trace; start a second
   worker process; assert the lease is reaped, the poll is re-run, the charge settles exactly
   once (one `charges` row, one ledger entry, one event) and the rail's request journal shows the
   status query count it should. Repeat for the `submitting` kill point if the harness allows it
   in the same file.
2. It runs under `just test-rust` like the rest of `backends/tests/integration` (container-backed,
   no `#[ignore]`). Guard-failure proof: disable `reap_expired_leases` and show the second worker
   never picks the job up.
3. Lane notes → `docs/plans/step8-notes/lane-d.md`; `docs/flows/crash-safety.md`'s "nothing
   kills a process" paragraph is retired by lane E from those notes.

### Lane F — SDK parity (added 2026-09-03, at the user's request: "vpay sdk parity: each should share the very same kind of features. We need a matrix for that.")

1. `docs/adr/0015-sdk-parity.md`: every merchant-facing capability lands in every merchant SDK in the
   same PR with the same wire semantics, or is a dated, named gap; the matrix is the record; the check
   is the enforcement. The browser package (`@vpay/stripe-js`) is a separate surface with its own rows;
   `sdks/stripe-compat` is evidence, not an SDK.
2. `docs/sdks/parity.md`: the matrix, measured by reading both SDKs — rows are capabilities, cells are ✅
   with the proving test names or ⛔ with a dated gap line.
3. `cargo xtask verify-sdk-parity` in `just verify` (and therefore CI's `self-checks`): a ✅ cell must
   name tests that exist in that SDK; no blank cells; a ⛔ must carry a date. Unit-tested on synthetic
   tables; revert-proofed by renaming a test in the matrix.
4. Lane notes → `docs/plans/step8-notes/lane-f.md` with the gap list. Merge order becomes D, B, C, A, F
   (F is additive: `docs/`, `.xtask`, `justfile`'s `verify`).

### Lane E — the record (orchestrator, after merging A–D and F)

`docs/status.md` (new/changed rows, the "What would have to be true to call this an MVP" list,
the header's "last verified" note with real counts), `docs/roadmap.md` (a Step 8 addendum and the
Phase 5/6/7 rows), the flow docs named in the lane notes, `docs/runbooks/README.md`, and this file's
"Outcome" section. ~~Issue #11 is closed by the PR~~ **Corrected 2026-09-04: issue #11's checklist
is answered item by item in the PR and in the Outcome; the issue is
deliberately not closed — the Outcome says why.** That includes the two items
answered by "out of scope, because".

## Environment

Builds and tests run in a dedicated multipass VM (`vpay-step8`: Ubuntu 24.04, 8 vCPU, 16 GB,
160 GB) with the host's `/home/selast/dev/vpay` mounted at the same path, `CARGO_TARGET_DIR` inside
the VM's own disk, and Docker inside the VM. Sources are edited and committed on the host. One
`target` dir is shared by every lane (the host disk was at 98% with five finished worktrees' 350 GB
of build caches, reclaimed at the start of this step).

## Definition of done

- `just ci` green in the VM on the integration branch, and the four lanes' container-backed tests
  green with their guard-failure proofs recorded in the PR.
- `just demo` from nothing (`just demo-down` first) walks both rails and every outcome above, and
  its output is what `docs/runbooks/demo.md` shows.
- `cargo xtask verify-status` / `verify-no-mocks` / `verify-errors` pass; no new
  `NotImplemented` token.
- Issue #11's checklist answered item by item in the PR; PR merged to `master`; the "do not
  deploy" banner unchanged and the reason restated.

## Outcome

*Written 2026-09-04 by lane E, on the merged gate branch `claude/step8-production-gate`
at `ef19991`. Every count below was measured on that commit unless it names who
measured it instead. **Extended 2026-09-04 for lane H**, which merged as
`1c742a4` after this section was written; its own counts name that commit.*

### What landed, per lane

| Lane | Landed | Named limit it leaves behind |
|---|---|---|
| **B — the runtime egress guard** | `vpay_worker::ssrf` on every webhook delivery: resolve once, classify every answered address in both families (mapped and IPv4-compatible spellings included), refuse permanently if any is non-public, pin the client with `resolve_to_addrs`. `webhooks.allow_private_targets` (default `false`), livemode + `true` is a refusal to boot. 9 unit cases, 2 container-backed cases, a revert proof. **This closed the repository's only ⛔ on a shipping path.** | Webhook delivery only, not the rail adapters. NAT64 receivers refused fail-closed. The pin cost the shared connection pool, unmeasured under load. No deployment has ever refused a real merchant's endpoint |
| **C — the rail callback route** | `POST /provider/{code}/callback` (`vpay_api::provider_callback`), `Charges::get_by_provider_reference`, `TxRepositories::pull_forward_in_tx`, migration `0027`'s index, and the conformance case that asserts `X-Callback-Url`/`notif_url` keep being sent. 9 container-backed cases; the route writes no charge or intent state | No rail has ever called it — every body it has parsed was transcribed from `docs/flows/adapter-*.md`, so a document wrong about a rail would pass. Orange's `notif_token` is not compared against the stored one and `ref_extra` is discarded. A payer's `GET` gets axum's bare `405` |
| **D — a real `SIGKILL`** | `backends/tests/integration/tests/worker_kill9.rs`: the shipping `vpay-worker-bin` killed mid-status-query and the shipping `vpay-server` killed mid-`requesttopay`, exit asserted *signalled with 9*, the charge settling exactly once by four independent counts. Guard-failure proof: disabling `reap_expired_leases` leaves the second worker healthy and permanently unable to claim | Kill point 1 is still written rather than caused — there is no request to interrupt at that instant. Orange is not exercised. Two clocks are simulated (the dead worker's lease; since lane G, the crashed charge's age) |
| **G — the confirm/worker race** | `RecoveryAction::Wait`: no `submitting` charge younger than `RecoveryPolicy::not_found_window` (60 s) is recovered, measured from `charges.created_at`. One predicate in `recovery_step`, reached by both callers. A unit table over five branches × four ages, three `worker_recovery` cases, and a revert proof that reproduces the merchant's own error text | The HTTP confirm handler is not in that suite: the racing write is `persist_submitted`'s compare-and-swap called directly. A charge orphaned by a genuine crash now waits up to a minute for its first recovery pass |
| **A — the demo** | Six payments, both rails, three outcomes each, every outcome steered at the stub by a field a merchant controls; `demo`/`demo-up`/`demo-walk`/`demo-status`/`demo-down`; `${VPAY_DEMO_PROJECT}` and three `just` variables; `docs/runbooks/demo.md` with real pasted output. It also found lane G's defect, and fixed an unquoted-heredoc bug in `gen-demo-keys` that had been running `pk_test_` as a command since Step 5c | `.e2e/` is not isolated between stacks. `just demo` has **not** been run on the merged gate branch. **Corrected 2026-09-04:** the one green run from nothing on record is lane A's rebased branch (2026-09-04, **without** lane G; the race is timing-dependent and did not fire), lane A's own earlier count was two greens in six attempts and zero for three from nothing, lane G did not re-run the demo. Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in): `just demo` from nothing six times, **four green** (six outcomes for six each, exit 0); the two failures were the VM's Postgres answering single statements in 14–36 s under host I/O pressure, with the settlement and the webhook both landing in the worker's log after the demo's budgets; `write_matched_no_row` appeared in no run. Three from nothing is met in count, not consecutively. |
| **F — SDK parity** | [ADR-0015](../adr/0015-sdk-parity.md), [`docs/sdks/parity.md`](../sdks/parity.md), and `cargo xtask verify-sdk-parity` in `just verify`. Re-measured on the gate: **267 proving tests named, 24 dated gaps** | **None of the 24 gaps was closed**, deliberately: the rule is that gaps become dated and owned, not that they are zero on adoption |
| **H — the correctness review's four findings** | `Charges::get_by_id_as_of` selects `now()` beside the row and `recovery_step`/`past_the_horizon` take **durations**, so the recovery window is Postgres' clock at both ends rather than the worker host's minus Postgres'; `RecoveryAction::Wait` carries `not_found_window - age` (clamped) and reschedules **once**, so a crashed charge's first real rung is `poll_delay(1)` and not `poll_delay(6)`; `pull_forward_in_tx` takes a floor and the callback route passes `PULL_FORWARD_FLOOR` (10 s, the ladder's fastest rung), with `the_pull_forward_floor_is_the_poll_ladders_first_rung` as the cross-crate join; the egress classifier refuses `192.88.99.0/24`, `2001:1::/32`, `2001:2::/48` and `2001:20::/28`. Five new tests, all in files that already existed | **F5 and F7 are not fixed** (below). **No rate limit** on the callback route, and the floor is not one — past the first rung a caller can still hold one live charge at roughly one `query_status` per worker claim. `scan_live_charges` still takes its ten-minute cutoff from the host clock. `2001::/23` as a whole is still deliverable. The floor costs lane C a behaviour: a callback inside the first rung no longer settles the charge early |

### Deviations from this plan

1. **Lane G was not in the plan.** It exists because lane A's demo found a real
   defect in a payment path — a `500 api_error` on confirm in four of six
   walkthrough runs, the worker applying the crash-recovery table to a charge
   whose process had not crashed — and lane A deliberately did not fix it,
   because the choice between a minimum charge age, a delayed first rung and a
   confirm-held lease is a design decision rather than a patch at the end of a
   lane. The maintainer took the first. **This is the strongest thing this step
   produced: a demo whose purpose was to be an artefact for an integrator found
   a defect that would have paged in production**, and it is the second time
   the demo has done that (the first was the `FROM scratch` trust-store panic).
2. **Lane F was added at the user's request** after the plan was written
   ("vpay sdk parity: each should share the very same kind of features. We need
   a matrix for that."), and the merge order became D, B, C, G, A, F.
3. **Two merge seams were made by the integrator and are not in any lane note.**
   Lane G's guard made lane D's `a_server_killed_mid_submit_…` fail on the gate
   (the killed server's charge is `submitting` and younger than 60 s, so the
   worker correctly waited); `worker_kill9.rs` gained `age_the_crashed_charge`,
   guarded on `state = 'submitting'`, and the file header now names both
   clocks. The failure before that fix was measured (TRY 3 FAIL, 63 s), so the
   ageing is load-bearing rather than cosmetic. Separately, lane C's
   `provider_callback.rs` and lane G's race test both predate lane B and now
   take `support::default_egress_policy()` / `Config.webhooks`.

4. **Lane H was not in the plan either.** It exists because Step 8's own
   correctness review, run against the merged gate branch, confirmed four
   defects — one each in lanes B and C, two in lane G — and recorded two more
   it deliberately did not fix. Two of
   the four sat in lane G's *fix* — a guard whose age came off the worker
   host's clock, which made it a silent no-op on a fast worker, and a `Wait`
   that spent six ladder rungs proving a charge was young — which is the
   argument for reviewing a remediation rather than trusting it: lane G's own
   tests all passed with both defects present, because they ran on a host whose
   clock agreed with the database's.
   [step8-notes/lane-h.md](step8-notes/lane-h.md) carries the four fixes, the
   mutation table that proves the clock one, and the two findings recorded
   rather than fixed.

### What was not done

- **The `.e2e/` per-project keypair.** Compose-layer isolation is proven; the
  checkout's single key pair and profile overlay are not isolated, so a second
  `demo-up` on a different `demo_port` regenerates the shared pair and the
  first stack's `demo-walk` fails `invalid_client`. Fixing it touches
  `.github/workflows/ci.yml`'s e2e job, `just stripe-compat`,
  `examples/merchant-stripe-node` and `sdks/stripe-compat`, whose failure mode
  here is a *silent* `invalid_client`.
- **Orange in the kill test.** Both kill cases use `mtn_momo`. A redirect-rail
  kill test needs its own scenario, because Orange's ordering is reversed.
- **Kill point 1 is still state-written.** There is no network call to
  interrupt at the moment before the reference is minted, so there is nothing
  for a real signal to land during. `worker_recovery.rs` remains its only
  proof.
- **NAT64 receivers (`64:ff9b::/96`) are refused**, fail-closed, even when the
  embedded IPv4 is public. No such receiver is believed to exist; it is written
  down so it is a decision rather than a surprise.
- **The 24 SDK parity gaps are open**, dated and owned, in the matrix's own gap
  ledger.
- **Orange's `notif_token` is not verified** and `ref_extra` repair from a
  callback is not built.
- **F5 — an SSRF-refused webhook delivery is destroyed on its first attempt.**
  An egress refusal is permanent (`state = 'exhausted'` on attempt 1) and
  replay is unbuilt, so a transiently poisoned DNS answer, or a receiver behind
  a resolver that briefly returns a private address, loses the event with
  nothing to re-drive it. Lane B did exactly what the plan asked and the plan
  asked for fail-closed; the gap is that "fail closed" and "destroy the event"
  are the same thing while replay does not exist. **Not fixed:** the remedy is
  a replay path or a retryable `ssrf_blocked` state, which is a decision about
  a merchant-visible delivery state machine. Lane H's recommendation is in
  [../flows/webhooks.md](../flows/webhooks.md).
- **F7 — the callback route's two `202`s are not the same duration.** The
  unknown-reference path returns after one indexed `SELECT`; the
  known-reference path additionally opens a transaction and runs two
  statements, which is a timing oracle for "does this deployment hold a charge
  with this rail reference". **Not fixed, and not obviously worth fixing:** it
  only matters to someone who has already guessed a v4 UUID, which is what they
  would need to exploit the answer. Recorded so nobody later reads the uniform
  status code as a complete answer.
- **No rate limit was added to the callback route**, per charge or per source,
  and `PULL_FORWARD_FLOOR` is not one. Past the ladder's first rung a caller
  repeating against one live charge still holds it at roughly one authenticated
  `query_status` per worker claim. A real deployment must front the path.
- **`scan_live_charges` still computes its ten-minute cutoff from the worker
  host's clock** and compares it against `charges.updated_at`, which Postgres
  wrote — the same cross-clock defect as F1, in its mildest direction (a fast
  worker re-enqueues sooner, and every re-enqueue is `ON CONFLICT DO NOTHING`).
  Left alone because it is outside the four findings and fixing it properly
  means a `vpay-db` signature change the reviewer did not ask for.
- **No real rail was called**, and the "do not deploy" banner is unchanged.
  `mtn_momo::refund` is still the one `NotImplemented` token.
- **`just demo` has not been run on the merged gate branch**, and no full
  `cargo nextest run --workspace`, `just ci`, Cypress or `sdks/stripe-compat`
  run of this exact tree is cited by lane E.

### Decisions left to the maintainer

- **`charges.provider_reference_id` has no `UNIQUE` constraint, and migration
  `0027`'s index is deliberately not one.** Every insert path mints the
  reference with `Uuid::new_v4()` before committing, so "one charge per rail
  reference" appears true by construction — but it is a schema-level invariant
  this repository has never claimed, and taking it as a side effect of adding a
  route would be deciding something reserved for whoever owns the schema. Lane
  C's recommendation is to make it `UNIQUE (provider_code,
  provider_reference_id)` in a commit of its own, with a test that the
  constraint fires. **Lane E did not decide it.**
- Whether a confirm-held lease (lane A's option 3) should eventually replace
  lane G's minimum charge age. Lane G's own notes call it the more correct fix.
- **Whether to refuse `2001::/23` as a whole.** The three IPv6 prefixes lane H
  added (`2001:1::/32`, `2001:2::/48`, `2001:20::/28`) all sit inside the block
  RFC 2928 gave IANA for IETF protocol assignments. Refusing the entire `/23`
  would be the broader and arguably more correct fix — nothing in it is a place
  a merchant's receiver lives — but it is wider than the review asked for, so
  it was not taken. It is also why no address inside `2001::/23` appears in the
  egress classifier's "ordinary public addresses" table: asserting one is
  deliverable would be a claim about IANA's unassigned space this lane is not
  in a position to make. **Lane H did not decide it.**

### The gate, as measured on `ef19991`

`just verify` **ok** — `verify-no-mocks` ok; `verify-status` ok, 1 unimplemented
item; `verify-errors` ok, **15 error types, 14 `#[from]` variants**;
`verify-sdk-parity` ok, **267 proving tests, 24 dated gaps**. `just
verify-ignored` **0 ignored, 41 test binaries, 1054 total**. `just test-doc`
**77 passed, 0 failed, 1 ignored** (the ignored one is `sdks/rust`'s and
pre-existing). `just docs-check` ok, link checking still unimplemented. The
container-backed suites were measured by the integrator on this commit:
`worker_kill9` + `provider_callback` **11 passed**; `worker_recovery` +
`confirm_rails` + `worker_e2e` + `worker_kill9` **35 passed**.

### After lane H, as measured on `claude/step8-production-gate` at `1c742a4`

`cargo nextest list --workspace` now lists **1059 tests in 41 binaries** (was
1054 in 41). The five new cases all landed in files that already existed, so
neither `expected_suites` nor `min_tests` moves and the justfile's comment says
why. `vpay-worker` 75, `vpay-db` 82, `vpay-api` 214 — **371** together, up from
368; `worker_recovery` 23, `provider_callback` 11 (up from 9), `confirm_rails`
7, `worker_kill9` 2 — **43** together. Lane H measured `cargo fmt --all
--check` and `cargo clippy --workspace --all-targets -- -D warnings` clean, all
371 and all 43 passing with no flakes, `just verify` ok on the same four gates,
and `just test-doc` **77 passed, 0 failed, 1 ignored**, on its own branch
`claude/step8-review-r1`; those runs are cited from
[step8-notes/lane-h.md](step8-notes/lane-h.md) §8a and were **not** re-run for
this section, which re-measured only the listing above.

**The review trail, so the remediations are not mistaken for the reviews.**
Step 8's merged gate was reviewed three times — for correctness, for
documentation accuracy, and for blast radius — and each remediation was itself
reviewed before it merged, twice in total. The documentation remediation is the
run of `docs(...)` commits from `0c9f767` to `4900740`; the correctness
remediation is lane H (`5ba6b11`, `605f4da`, `6987e31`, `8508b31`). **Reviewing
the remediation is what found the clock defect worth naming here:** lane G's
guard passed its own tests with the host-clock subtraction in place, because the
authoring host's clock agreed with the database's.

**The Definition of done above is not fully met, and the two unmet items are
named rather than rounded off:** `just ci` was not run by lane E; **the integrator ran it in the `vpay-ci` VM on the merged code (`1c742a4`, lane H in): 1059/1059, 41 binaries, 0 ignored, 77 doctests, web and deny green; `sdks/stripe-compat` 25/25.** **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing *after* the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both. See the "Why it is not auto-closed" note below.

### Issue #11, item by item

**The issue is answered here and in the PR, and is deliberately not
auto-closed.**

1. **One command, no host port collisions across concurrent runs** — **done at
   the Compose layer, incomplete at `.e2e/`.** `just demo` brings up six
   services; `compose.demo.yml`'s `name:` is `${VPAY_DEMO_PROJECT:-vpay-demo}`
   and three `just` variables move the project and both host ports; two stacks
   were run side by side. The unfixed half is the shared checkout key pair, not
   the ports.
2. **A registered demo merchant, `client_credentials` + `private_key_jwt`** —
   **done, and it predates this step.** `just gen-demo-keys` generates the
   pair, writes the public JWK into the git-ignored `demo` overlay and keeps
   the private half at mode 0600 on the host. The walkthrough's first steps are
   that flow, minted with the shipping SDK's own `mint_client_assertion`.
3. **A walkthrough on each rail, `succeeded` and `failed`/`expired`, with a
   signed webhook verified** — **done.** Six payments, both rails, three
   outcomes each, each with its own signature-verified webhook. The receiver is
   `wiremock-webhook` rather than `examples/webhook-receiver`, because that is
   the container the compose stack already runs and whose journal can be read
   from the host.
4. **The dashboard, or a statement of why it is out of scope** — **out of
   scope, stated** (`docs/runbooks/demo.md` §6): it renders a static scaffold
   notice and makes no call to `vpay-server`, and `/dash/v1` does not exist, so
   **there is no data source to show**. The reason is the missing API, not an
   unfinished screen.
5. **A `docs/status.md` demo row and a `docs/runbooks/demo.md`** — **both
   done**, with one correction the issue's own wording invites: `verify-status`
   polices `NotImplemented` tokens, not prose rows, so it does not by itself
   keep a demo row honest. The row's own evidence has to, which is why the demo
   rows say which branch each measurement came from.
6. **The two documented hazards fixed or made explicit** — **made explicit**,
   runbook §8.1 and §8.2. The rustls `CryptoProvider` hazard is **closed**
   (2026-09-02) and this step adds the evidence its row said it lacked: a
   containerised `vpay-server` serving authenticated `/v1` requests.
   RUSTSEC-2023-0071 is **open**, has no patched release, is `ignore`d in
   `deny.toml` with its reasoning, and signs every token the demo obtains.
7. **The authkestra pin reconciled** — **already reconciled; nothing to do.**
   `Cargo.toml` pins all four crates at `=0.7.1` and `docs/status.md` says
   exactly that. ~~Its one surviving `=0.3.4` mention is a historical note~~
   **Corrected 2026-09-04: its two surviving `=0.3.4` mentions** are historical
   notes — where migration `0006`'s DDL was transcribed from (`docs/status.md`
   line 1213), and what `SqlxOpStore::find_client` did at the then-pinned
   version (line 1215) — not claims about the current pin.

**Why it is not auto-closed.** Item 1 is incomplete (the `.e2e/` half), and
item 3's walkthrough has a history the issue's checkbox cannot carry: it was
flaky for a real reason, and that reason was fixed. ~~The green run that
followed was on lane A's rebased branch, which carried the fix, so the race fix
has been demonstrated from nothing once.~~
**Corrected 2026-09-04:** the plan's own bar for closing item 1 was **the race
fix demonstrated three times from nothing**, and it has been demonstrated
**zero** times. One green run from nothing exists (lane A's rebased branch,
2026-09-04, **without** lane G — that branch was rebased onto `068d8b7`, master
plus lanes B and D, and lane G merged later as `53f7a7e`; the race is
timing-dependent and did not fire), lane A's own earlier count was two greens in
six attempts and zero for three from nothing, lane G did not re-run the demo,
and lane G did not re-run the demo. **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing *after* the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both. Four of six is not three of three in a row, item 1's `.e2e/` half is still open, and a closed issue that is not fixed is worse than an open one.
