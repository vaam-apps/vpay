# 2026-09-23 — Retiring stale claims found while writing vpay-docs

## Why this page exists

Writing a human documentation site against tag `v0.4.1` meant reading most of
`docs/flows/`, the SDK READMEs and the examples end to end. Fourteen claims
turned out to contradict the code, or a newer page in this same tree. Each
was re-checked against `master` at `dd1a48b0`, the tip on 2026-09-23. All
fourteen were still wrong there, and seven more sentences making the same
false claims turned up nearby (listed under "Found while checking").

**Nothing here changes behaviour.** Every edit is to a document or a doc
comment, and every correction is dated in place rather than silently
rewritten. No migration, route, gate or `NotImplemented` token moved, so no
row on an area page changed except the MTN real-sandbox cell in
[backend.md](../backend.md), which was itself one of the stale claims.

## The claims, and what the code says

1. **`backends/apps/vpay-server/src/main.rs` module doc**
   - Said: the API server "never calls a payment rail — that is the worker's job"
   - On `master`: `vpay_api::v1::payment_intents::submit_to_rail` awaits `adapter.submit` on confirm; `v1::refunds` calls `adapter.refund`; `v1::account_holders` calls `adapter.account_holder_name`
   - Wrong since: Step 3, 2026-09-03 (the sentence is from the 2026-08-09 scaffold)
2. **[payment-lifecycle.md](../../flows/payment-lifecycle.md) state diagram**
   - Said: `requires_action --> processing` and `requires_action --> failed : submit response lost`
   - On `master`: `vpay_core::next_status` gives `requires_action` no edge; `SETTLEABLE_STATUSES` settles or fails straight from it; a lost redirect submit leaves the intent in `requires_payment_method`
   - Wrong since: Step 4, 2026-09-03 (the edges are from the scaffold)
3. **[account-holder-lookup.md](../../flows/account-holder-lookup.md)**
   - Said: "MTN's real sandbox has never been called — for this or for any other operation"
   - On `master`: [2026-09-15.md](2026-09-15.md): token, `requesttopay` and status query ran against the real sandbox; `basicuserinfo` did not, so the lookup half still stands
   - Wrong since: 2026-09-15
4. **[failures.md](../../flows/failures.md)**
   - Said: "neither rail's real sandbox has ever been called"
   - On `master`: the same run, which produced no decline
   - Wrong since: 2026-09-15
5. **[backend.md](../backend.md) adapter matrix**
   - Said: MTN real sandbox "⛔ never called"; wire calls "never against MTN or Orange"
   - On `master`: the same run
   - Wrong since: 2026-09-15
6. **[reconciler.md](../../flows/reconciler.md)**
   - Said: callback sketch ends `→ 200 OK`; Status lists "No fan-out"
   - On `master`: `vpay_api::provider_callback` answers `202` (unknown reference too), `404` for no adapter, `400` for a bad body; `vpay_worker::webhooks` fans out `pending` events
   - Wrong since: `202`: from the route's first day, Step 8, 2026-09-04. Fan-out: Step 5, 2026-09-03
7. **[errors.md](../../flows/errors.md)**
   - Said: "no job loop exists to call `JobError::decision()`"
   - On `master`: `vpay_worker::run_loop` calls `error.decision(..)` for every job whose handler returns `Err`
   - Wrong since: Step 4, 2026-09-03
8. **[docs/api/README.md](../../api/README.md)**
   - Said: event `type` is "one of the thirteen", eight written; invoice has "eighteen keys"; `verify-status` prints zero, "no `501` at all"
   - On `master`: CHECK in migration `0039` lists fifteen types, [webhooks.md](../../flows/webhooks.md) names thirteen writers; `the_invoice_object_is_the_documented_nineteen_keys`; `an_unbuilt_rail_refund_fails_and_releases_its_reservation` asserts `501`
   - Wrong since: types: 2026-09-10. keys: 2026-09-10 (migration `0042`). `501`: from the day it was written, 2026-09-16
9. **[merchant-auth.md](../../flows/merchant-auth.md), [resource-contract.md](../../flows/merchant-auth/resource-contract.md)**
   - Said: `"expires_in": 300`; no scheduled idempotency or `jti` sweep; `sdks/nodejs` "has still never spoken to a vpay"
   - On `master`: `ACCESS_TOKEN_TTL_SECS = 900`; `vpay_worker::handlers::sweep_expired` deletes both hourly; `invoices.live.test.ts` and `refunds.live.test.ts`, run by CI's `e2e` job
   - Wrong since: `300`: from the day it was written, 2026-09-02. Sweeps: Step 4, 2026-09-03. Node: 2026-09-10
10. **[README.md](../../../README.md)**

- Said: `just demo` brings up "eight" services, the dashboard "stays down … no login"; layout calls the dashboard "a scaffold"
- On `master`: the justfile's `demo_services` names nine, including `dashboard`; [dashboard-sign-in.md](../../runbooks/demo/dashboard-sign-in.md)
- Wrong since: exp28, 2026-09-07

11. **[examples/README.md](../../../examples/README.md)**

- Said: refund creation "has no route"; both examples make a balance and a refund call
- On `master`: `POST /v1/refunds` mounted 2026-09-16; neither example calls either route
- Wrong since: 2026-09-16

12. **[dashboard-auth.md](../../flows/dashboard-auth.md) Status**

- Said: "The rate limit is per replica"
- On `master`: `rate_limit_windows` (migration `0038`); `two_replicas_share_one_sign_in_budget`
- Wrong since: 2026-09-10

13. **[sdks/stripe-js/README.md](../../../sdks/stripe-js/README.md)**

- Said: "Not yet on the registry … no workflow publishes"; "Step 5c ships push-only … return trip not wired"
- On `master`: `publish-stripe-js-sdk` in `release.yml` (2026-09-18); `npm view` lists `0.2.1` (2026-09-19) to `0.4.1`; [browser-checkout.md](../../flows/browser-checkout.md) § "The redirect gap (D4) — closed 2026-09-04"
- Wrong since: registry: 2026-09-19. Return trip: 2026-09-04

14. **[frontends/apps/dashboard/README.md](../../../frontends/apps/dashboard/README.md)**

- Said: "Webhooks, checkout sessions … are not built"; the BFF's handlers are called by nothing
- On `master`: deliveries screen `f9c7d2c` (2026-09-13), checkouts screen `8be7850` (2026-09-14); `app/(dash)/layout.tsx` wires `dashDataProvider("/api/dash")`
- Wrong since: deliveries: 2026-09-13. Checkouts: 2026-09-14. BFF: 2026-09-12

### Found while checking, and corrected the same way

- [docs/runbooks/demo.md](../../runbooks/demo.md): "Six services … the
  seventh, `dashboard`, is deliberately absent". Nine since exp28.
- [examples/merchant-curl/README.md](../../../examples/merchant-curl/README.md):
  "nothing asks the rail whether the payer approved … the reconciler is a later
  step" (wrong since Step 4), and `POST /v1/refunds` "not routed at all" (wrong
  since 2026-09-16).
- [sdks/nodejs/README.md](../../../sdks/nodejs/README.md): the same "not yet
  on the registry" claim as item 13, and the same dates.
- [README.md](../../../README.md): `/dash/v1` routes "nothing in the app calls
  them yet". The app has called them through its BFF since 2026-09-12. And
  "No test inside `sdks/nodejs` itself has ever spoken to a vpay", wrong since
  2026-09-10 (see below).
- `docs/api/README.md`: the example invoice object lacked `amount_refunded`.

### What the brief had wrong

Item 9 cited README.md as evidence that the Node SDK had spoken to a vpay. It
does not say that: README.md made the **same** stale claim ("No test inside
`sdks/nodejs` itself has ever spoken to a vpay"), and it is corrected here
too. The evidence is the two `*.live.test.ts` files and CI's `e2e` job.

## Left alone, on purpose

- `vpay_core::state`'s test comment, "written out from
  `docs/flows/payment-lifecycle.md`'s diagram". It is still true of the
  merchant verbs, which are all that table covers, and item 2 changed no verb.
- The skills. [vaam-apps/vpay-skills](https://github.com/vaam-apps/vpay-skills)
  was grepped for every one of the fourteen claims on 2026-09-23 and repeats
  none of them. Its `known-wrong-docs.md` census lists past corrections as
  examples, not these. No parity trigger in AGENTS.md § "Docs↔skills
  parity" applies: no flow page, route, token, gate, toolchain pin or path
  moved.

## What was run

On macOS, `cratestack` 0.12.0 on `PATH`, branch `claude/retire-stale-doc-claims`
off `master` at `dd1a48b0`:

- `just verify` **before** any edit: exit 0, "the fifteen gates above
  passed". `verify-links` 1 876 links in 413 files; `verify-doc-counts` 13
  counts in 11 files.
- `just verify` **after**, with every change staged (so the new page is
  tracked, as `verify-links` and `verify-doc-counts` require): exit 0, all
  fifteen. `verify-links` **1 921** links in **414** files. `verify-doc-counts`
  13 counts agree, including the `files-with-suffix docs/status/verification
.md` marker in [../README.md](../README.md), moved from 60 to **61** by
  this page. Every other gate printed what it printed before the edit:
  `verify-status` 1 unimplemented item, `verify-sdk-parity` 750 / 45 / 35 / 39,
  `verify-migrations` 48 files, `verify-versions` 24 references at 0.4.1.
- **Re-run after merging `master` at `51082a7` (#242)**, which landed the same
  day with its own verification page: exit 0, all fifteen. `verify-links`
  **1 928** links in **415** files, and the `files-with-suffix` marker is
  **62**, with both pages indexed. The one conflict was that marker and its
  hand-kept tallies in [../README.md](../README.md), resolved to 62 files, 39
  named and 23 omitted.
- `cargo clippy -p vpay-server --all-targets -- -D warnings`: clean, for the
  `main.rs` doc comment.
- `pnpm exec prettier --write` on every changed Markdown file. Only
  [backend.md](../backend.md) moved, re-padding the adapter table.
- `just docs-check-citations` (network, `GITHUB_TOKEN` from `gh`): every
  issue this change cites resolves (#45, #77, #79). The recipe exits **1** on
  one id that predates this branch: it reads the MTN-sandbox test MSISDN in [../../runbooks/live-sandbox-test.md](../../runbooks/live-sandbox-test.md)
  and [2026-09-15.md](2026-09-15.md) as a CI run id. That is a false positive
  in the gate's matcher, not a missing run, and this branch does not touch it.
  `#138` in the dashboard README is written bare, which the matcher does not
  collect; `gh pr view 138` shows it merged.

**Not run:** `just ci`. Nothing here is compiled except one doc comment, which
clippy covered, and no test asserts on these documents. `just test-doc` runs
no example this change touches.
