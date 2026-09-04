<!-- Per-lane notes for Step 8. Lane E (the orchestrator) edits docs/status.md, docs/roadmap.md and docs/flows/*.md from these, so four lanes never fight over one table. This file is history once Step 8 lands. -->

# Step 8, lane A — the demo (issue #11)

Branch `claude/step8-lane-a-demo`. Everything below was run on 2026-09-03/04 on
the authoring machine, which was **heavily loaded throughout** (load average
16–34; three sibling lanes building and one running `just ci` in a multipass
VM). That matters for one finding and is stated where it does.

## 1. What landed

| Deliverable | State |
|---|---|
| Both rails, six outcomes, each printing the intent's public fields, the `failure_code` and the signature-verified webhook | **done** |
| `compose.demo.yml` `name: ${VPAY_DEMO_PROJECT:-vpay-demo}`; `demo_project`/`demo_port`/`demo_receiver_port` used consistently by every demo recipe | **done at the Compose layer, incomplete at the `.e2e/` layer** — §4 |
| Split recipes `demo`, `demo-up`, `demo-walk`, `demo-status`, `demo-down`; readiness by `--wait` on healthchecks | **done** |
| `docs/runbooks/demo.md` with real pasted output, linked from README and `docs/runbooks/README.md` | **done** |
| A green `just demo` from nothing | **NOT achieved — §3.** A real vpay defect, not a demo defect. |

Files: `examples/merchant-demo/src/main.rs`,
`backends/tests/conformance/wiremock/{mtn,orange}/mappings/demo-outcomes.json`,
`justfile` (demo recipes only), `compose.demo.yml`, `compose.yml`,
`compose.e2e.yml`, `docs/runbooks/demo.md`, `README.md`, `examples/README.md`,
`examples/merchant-demo/README.md`, `examples/checkout-browser/README.md`.

## 2. Evidence

- **`cargo nextest run -p vpay-tests-conformance`: 26 tests run, 26 passed, 0
  skipped**, in this worktree's own `target/` (166 s). The two new
  `demo-outcomes.json` mapping files keep the suite green: the MTN half is
  unreachable in a conformance run because its scenarios are armed only by
  MSISDNs no conformance case sends, and the Orange half is keyed on amounts
  (5001/5002) no existing test uses.

  Run three times in all. The first (26/26, 805 s) was in the shared
  `step8-target` the lanes were later told not to use. An intermediate run
  failed one case — `a_declined_charge_maps_to_the_documented_failure_code::case_1_mtn_momo`
  — and it is worth naming so nobody re-derives it: the panic was
  `the rail stub starts: Client(CreateContainer(RequestTimeoutError))`, the
  Docker daemon timing out creating a container at load average 20+, with
  `case_2_orange_money` passing beside it at 201 s. Infrastructure, not the
  mappings; it passed on a quieter machine with the same tree.
- **`cargo fmt --all -- --check`**, **`cargo clippy -p merchant-demo
  --all-targets -- -D warnings`**, **`just verify`**: clean.
- **The walkthrough went green twice**, six outcomes for six, exit 0 — once
  standalone and once on a second concurrent stack. One is pasted verbatim in
  `docs/runbooks/demo.md` §4.
- **`just demo-down` leaves nothing**: containers, volumes and networks all
  empty afterwards, pasted in the runbook §10.

### Outcome-steering, for the record

Nothing rewrites stored state to force an outcome; every one is selected at the
stub by a field the merchant controls.

| # | Rail | Steered by | Settles to | `failure_code` |
|---|---|---|---|---|
| 1 | `mtn_momo` | MSISDN `237600000ce0` (existing `mtn-e2e-poll`) | `succeeded` | — |
| 2 | `mtn_momo` | MSISDN `237600000f01` (**new** `mtn-demo-decline`) | `requires_payment_method` | `insufficient_funds` |
| 3 | `mtn_momo` | MSISDN `237600000f02` (**new** `mtn-demo-expiry`) | `requires_payment_method` | `payer_timeout` |
| 4 | `orange_money` | 5000 XAF (falls through to the catch-all SUCCESS) | `succeeded` | — |
| 5 | `orange_money` | 5001 XAF (**new**, `EXPIRED`) | `requires_payment_method` | `payer_timeout` |
| 6 | `orange_money` | 5002 XAF (**new**, `FAILED`) | `requires_payment_method` | `provider_error` |

**MTN's expiry is `COULD_NOT_PERFORM_TRANSACTION`, not `EXPIRED`.** `EXPIRED`
is Orange's status string; MTN documents `PENDING`/`SUCCESSFUL`/`FAILED` only,
and `vpay_adapter_mtn_momo::mapping::FAILURE_REASONS` already annotates
`COULD_NOT_PERFORM_TRANSACTION` as "the payer never entered their PIN". Both
land on `payer_timeout`. Inventing an MTN `EXPIRED` to make the rails look
alike would be a stub asserting something about a rail nobody has called.

## 3. BLOCKER, and it is not the demo: a confirm/worker race

**`just demo` from nothing did not go green: 0 for 3.** Six walkthrough
attempts total, two green, four failed — always a `500` on a confirm:

```
{"level":"ERROR","message":"api error","alert":true,"category":"Internal",
 "code":"write_matched_no_row",
 "error":"no row in charges matched ch_pk69syzy2x16s9f0wmpvx8gg, or it was no longer in the required state"}
```

### Mechanism

1. `insert_charge` commits the charge in `submitting` **and** its `poll_charge`
   job in one transaction with `run_at = OffsetDateTime::now_utc()` —
   immediately runnable: `backends/crates/vpay-api/src/v1/payment_intents.rs:1368`.
2. The confirm calls the rail, then CASes `submitting` → `submitted`:
   `charges::mark_submitted`, `backends/crates/vpay-db/src/charges.rs:463`
   (`WHERE id = $1 AND state = 'submitting'`).
3. The worker may claim that job at once — `IDLE_SLEEP` is 1 s
   (`backends/crates/vpay-worker/src/run_loop.rs:69`), zero if already busy. It
   finds the charge in `submitting` and applies the **crash-recovery** table
   (`backends/crates/vpay-worker/src/handlers.rs:272`), whose stated
   precondition is that the process died. Nothing distinguishes that from a
   confirm still in flight.
4. Either branch moves the charge, so the confirm's CAS matches no row → `500`,
   `alert: true`.

Window = the rail call plus two commits. Normally tens of ms; **measured at
3.7 s** here.

### Two observed outcomes, both bad

| Rail | Branch | Merchant got | Database holds |
|---|---|---|---|
| MTN (push) | `RecoveryAction::Advance` (`handlers.rs:295`) | `500` | intent `succeeded`; a `payment_intent.succeeded` webhook **was delivered** |
| Orange (redirect) | `RecoveryAction::FailDeadOrder` — taken **unconditionally** for `ProviderFlow::Redirect` with no age check, `backends/crates/vpay-worker/src/recovery.rs:179` | `500` | charge `failed`, `failure_code = provider_unavailable`, `failure_raw` = "the rail's submit response was lost before its token could be committed; the payer was never handed a redirect URL" — **while the confirm held exactly that token** |

Verified in the demo database:

```
             id              |         status          |   state   |     failure_code
 pi_payexeyke102bb15cewmhcyq | succeeded               | succeeded |
 pi_1ftncy3xd57t561xxw1ph2ne | requires_payment_method | failed    | provider_unavailable
```

### Why it is a defect and not tuning

`recovery_step`'s `Never` branch already guards this exact class of mistake
with a 60-second `not_found_window`, whose own comment says a count alone
"would look identical to [a rail] that never got it"
(`recovery.rs:44-51`). The `Answered` and `Redirect` branches have **no
minimum charge age at all**. A `submitting` charge is not only the state a
crash leaves — it is also the ordinary state of a confirm that is still
running.

### Not fixed here

It is in `vpay-api`/`vpay-worker`, outside this lane, and it wants a deliberate
choice rather than a patch at the end of a lane. Three candidate fixes, for
whoever takes it:

1. A **minimum charge age** before the recovery table applies at all (mirrors
   `not_found_window`; smallest change, one predicate in `recovery_step`).
2. **Delay the poll job's first run** — `run_at = now + first_rung` — so the
   confirm has the ladder's first rung to finish. Cheapest, but it weakens
   crash recovery by exactly that delay.
3. A **lease the confirm holds** on the `submitting` charge, which the worker
   respects. Most correct, most work.

**(1) or (3) is a maintainer decision; this lane does not pick one.** Note the
demo did not create this — it made it visible, which is what the "Local demo"
row in `docs/status.md` already credits it with once (it found the
`FROM scratch` trust-store panic on its first ever run).

## 4. The `.e2e/` collision — deliverable 2 is incomplete

Two demos coexist at the Compose layer, proven by running both: two projects,
two networks, two volumes, two databases with different row counts, distinct
published ports, and stack B's walkthrough green while stack A was up. Pasted
in the runbook §7.

They do **not** isolate `.e2e/`, which holds one merchant key pair and one
profile overlay for the whole checkout. Different `demo_port`s make
`gen-demo-keys` regenerate **the shared key pair**, so the older stack's server
holds a stale public JWK and its `demo-walk` then fails:

```
gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_port than 18088 — regenerating the pair
✘ step 2 (access token): the token endpoint refused this merchant with HTTP 401: {"error":"invalid_client",…}
```

**Not fixed**, deliberately: `.e2e/demo-merchant/oauth-signing-key.pem` is a
literal in `.github/workflows/ci.yml` (lines 339 and 363), `just
stripe-compat` (`justfile:1026`), `examples/merchant-stripe-node/index.mjs:48`,
`sdks/stripe-compat/src/env.ts:105` and as `examples/merchant-demo`'s
`DEFAULT_PRIVATE_KEY_FILE` (`main.rs:114`). Keying it on `demo_project` is the
right fix and touches the CI e2e job, whose failure mode here is a *silent*
`invalid_client`. Out of this lane's blast radius with three sibling lanes in
flight; recorded instead.

## 5. A bug fixed in passing

`gen-demo-keys`' heredoc is unquoted (`<<YAML`, so `"$kid"`/`"$n"` expand) and
three backticks in it were **unescaped**, i.e. command substitution. Every
`just demo` since `af09fdd` (Step 5c) ran `pk_test_`, `false` and `pk_live_` as
commands, printed two `command not found` lines into the middle of the recipe,
and wrote the overlay comment with those words deleted. Harmless as it stood;
not harmless as a pattern. Escaped, with a comment saying why.

## 6. Status rows to add or change (verbatim, for lane E)

**Change** the row `| Local demo (\`just demo\`, \`examples/merchant-demo\`, \`compose.demo.yml\`) | 🟡 |`
(currently `docs/status.md:1126`) — append to its body:

> **Updated 2026-09-04 (Step 8, lane A): six steps became four, and the fourth is six payments on both rails.** The walkthrough is now a table — MTN push to `succeeded`, to `insufficient_funds` (payer decline) and to `payer_timeout` (the prompt expired); Orange redirect to `succeeded` with the `next_action.redirect_to_url` printed, to `payer_timeout` (the hosted page expired) and to `provider_error` (the rail refused and documents no reason) — and each one prints the intent's public fields, asserts the exact `last_payment_error.code`, and verifies the `Vpay-Signature` of the webhook that settlement produced, read out of the receiver's own request journal. **Every outcome is selected at the rail stub by a field a merchant controls** — the MSISDN on MTN (documentation numbers `237600000f01`/`237600000f02`, carried to the status query by WireMock scenario, since MTN's status query is a `GET` that steers no other way) and the amount on Orange (5001/5002, which travel on its `POST` status body) — never by rewriting stored state. `just demo` is now `demo-up` + `demo-walk`, both of which exist separately, alongside `demo-status` and `demo-down`; readiness is `docker compose up --wait` on healthchecks (both rail stubs gained one) plus an external `/healthz` poll for the two `FROM scratch` services that cannot carry one. `compose.demo.yml`'s `name:` reads `${VPAY_DEMO_PROJECT:-vpay-demo}` and three `just` variables (`demo_project`, `demo_port`, `demo_receiver_port`) let two stacks run at once — **proven by running both**, two networks, two volumes, two databases, and the second stack's walkthrough green while the first was up. **Still 🟡, and for a new reason: `just demo` from nothing has never been observed green.** Six walkthrough attempts on 2026-09-03/04 gave two greens (six outcomes for six, exit 0) and four `500`s on a confirm, every one of them the `write_matched_no_row` race between `vpay-api`'s confirm and `vpay-worker`'s immediately-runnable poll job — **a defect in vpay, not in the demo**, written up in `docs/runbooks/demo.md` §9 and `docs/plans/step8-notes/lane-a.md` §3. `docs/runbooks/demo.md` is the procedure, with the output of a real run pasted rather than narrated.

**Change** the row `| \`just demo\` step 7 — the delivered webhook | 🟡 |`
(currently `docs/status.md:962`) — retitle to `| \`just demo\` — the delivered webhook, per outcome | 🟡 |` and append:

> **Updated 2026-09-04 (Step 8, lane A): it is no longer one webhook but six**, one per outcome, and the verified event's `type` is now asserted against what that outcome must produce (`payment_intent.succeeded` / `payment_intent.payment_failed`) — so a run in which every payment was delivered as a success could not pass, which the single-outcome version could not have caught. **Re-observed passing on 2026-09-04**, six for six, on this pass's own authority rather than a previous pass's: the journal paste is in `docs/runbooks/demo.md` §4. Still 🟡 for the unchanged reason — the receiver is a WireMock host, not a merchant, and the demo has never run in CI.

**Add** a new row, wherever the confirm path is described:

> \| Confirm vs. the worker's first poll (`write_matched_no_row`) \| ⛔ \| **Found 2026-09-04 by Step 8's demo.** `insert_charge` commits the `submitting` charge and its `poll_charge` job in one transaction with `run_at = now()` (`vpay-api/src/v1/payment_intents.rs:1368`), and the worker may claim that job before the confirm finishes its own `submitting` → `submitted` compare-and-swap (`vpay-db/src/charges.rs:463`; `IDLE_SLEEP` is 1 s, `vpay-worker/src/run_loop.rs:69`). The worker then applies the **crash-recovery** table to a charge whose process has not crashed (`vpay-worker/src/handlers.rs:272`), and either branch moves the charge out from under the confirm, which answers `500 api_error` with `alert: true`. Observed four times in six walkthrough runs on a loaded machine (confirm latency 3.7 s); the window is normally tens of milliseconds, so this is rare and not impossible. **Two outcomes, both wrong:** on a push rail the merchant is told the confirm failed and is then delivered a `payment_intent.succeeded` webhook; on a redirect rail `RecoveryAction::FailDeadOrder` (`vpay-worker/src/recovery.rs:179`, unconditional for `ProviderFlow::Redirect`, no age check) **kills a live order** and labels it `provider_unavailable` although the rail was never unavailable — with `failure_raw` claiming the payer "was never handed a redirect URL" while the confirm held that URL. The `Never` branch of the same table already guards this class with a 60 s `not_found_window`; the `Answered` and `Redirect` branches have no minimum charge age. **Not fixed:** the choice between a minimum charge age, a delayed first rung, and a confirm-held lease is a design decision — see `docs/plans/step8-notes/lane-a.md` §3 \|

**Add** to the demo row or beside it:

> \| Two demos on one machine \| 🟡 \| Compose-layer isolation is done and proven (`${VPAY_DEMO_PROJECT}`, `demo_project`/`demo_port`/`demo_receiver_port`; two projects, networks, volumes and databases observed side by side, the second stack's walkthrough green while the first was up). **`.e2e/` is not isolated:** one merchant key pair and one profile overlay serve the whole checkout, so a second `demo-up` on a different `demo_port` regenerates the shared pair and the first stack's `demo-walk` then fails `invalid_client`. Sequential use is fine; interleaved `demo-up` is not. Fixing it means keying `.e2e/` on `demo_project`, which touches `.github/workflows/ci.yml` (lines 339, 363), `just stripe-compat`, `examples/merchant-stripe-node` and `sdks/stripe-compat` — see `docs/plans/step8-notes/lane-a.md` §4 \|

## 7. Flow-doc lines for lane E

- `docs/flows/browser-checkout.md:235` cites "`examples/merchant-demo`'s
  `DEMO_MSISDN`". **That constant no longer exists** — the MSISDN is now
  `Steering::Msisdn("237600000ce0")` in `OUTCOMES[0]`. The number and the
  scenario (`mtn-e2e-poll`) are unchanged; only the name is stale.
- `docs/flows/stripe-sdk-compat.md:279` and `docs/flows/deployment.md:420`
  cite `just demo` runs by date. Both still true; neither needs changing.
- Nothing in `docs/flows/` claims the demo is single-rail, so no other line
  retires.

## 8. Issue #11's checklist, item by item

1. **"One command brings up vpay-server, vpay-worker, Postgres and two WireMock
   rails … with no host port collisions across concurrent runs (unique project
   name)"** — **done, with the §4 caveat.** `just demo` brings up six services
   (the two rails, the merchant webhook receiver, Postgres and both binaries).
   `compose.demo.yml`'s `name:` is `${VPAY_DEMO_PROJECT:-vpay-demo}` and three
   `just` variables move the project and both host ports. Two stacks were run
   side by side. The unfixed half is `.e2e/`, not the ports.
2. **"A registered demo merchant: key pair generated by a script, public half
   in the YAML, private half in a git-ignored file, and the two-step
   `client_credentials` + `private_key_jwt` flow runs against it"** — **done,
   and it predates this step.** `just gen-demo-keys` generates the pair, writes
   the public JWK into the git-ignored `demo` profile overlay, and keeps the
   private half at mode 0600 on the host, never mounted into a container. Steps
   1–3 of the walkthrough are that flow, and step 2 mints its assertion with
   the shipping SDK's own `mint_client_assertion`.
3. **"A scripted walk-through creates a PaymentIntent, confirms it on each
   rail, drives the stub to `succeeded` and to `failed`/`expired`, and shows
   the signed webhook arriving … with signature verification passing"** —
   **done.** Six payments, both rails, all three outcomes per rail, each with
   its own signature-verified webhook. The receiver is `wiremock-webhook`
   rather than `examples/webhook-receiver` because that is the container the
   compose stack already runs and whose journal can be read from the host.
4. **"The dashboard shows the intents … or the issue states that the dashboard
   is out of scope for this demo and why"** — **out of scope, stated.**
   `docs/runbooks/demo.md` §6: it renders a static scaffold notice and makes no
   call to `vpay-server`, and `/dash/v1` does not exist, so **there is no data
   source to show**. Booting it would invite a reader to look at a screen that
   cannot show the payments just made. The reason is the missing API, not an
   unfinished screen.
5. **"`docs/status.md` gains a 'demo' row that `cargo xtask verify-status`
   keeps honest, and a `docs/runbooks/demo.md` … with the exact commands and
   the expected output, in the same RAN-not-narrated style"** — **runbook
   done** (real output, including the failures); **status rows drafted in §6
   for lane E**, which owns `docs/status.md` this step. Note `verify-status`
   polices `NotImplemented` tokens, not prose rows, so it does not by itself
   keep a demo row honest — the row's own evidence has to.
6. **"The two documented hazards are either fixed or made explicit in the demo
   runbook"** — **made explicit**, §8.1 and §8.2 of the runbook. The rustls
   `CryptoProvider` hazard is **closed** (2026-09-02) and this step adds the
   evidence its row said it lacked: a containerised `vpay-server` serving
   authenticated `/v1` requests. RUSTSEC-2023-0071 is **open**, has no patched
   release, is `ignore`d at `deny.toml:49` with its reasoning at `deny.toml:14-49`,
   and signs every token the demo obtains — said out loud.
7. **"`docs/status.md`'s authkestra pin (cites `=0.3.4`) is reconciled with
   `Cargo.toml` (`=0.5.4`)"** — **already reconciled; nothing to do.** Checked
   against the tree: `Cargo.toml` pins all four crates at `=0.7.1`
   (`Cargo.toml:257`, `:258`, `:262`, `:264`) and `docs/status.md` says exactly
   that. Its one surviving `=0.3.4` mention is a historical note about where
   migration `0006`'s DDL was transcribed from, not a claim about the current
   pin. The DDL re-diff the item was really worried about was done by the
   SDK/authkestra pass and is recorded in the OP-tables row.

**The issue should not be auto-closed by this step's PR.** Item 1 is
incomplete (§4) and the walkthrough it asks for is currently flaky for the
reason in §3 — both are visible in the runbook, and a closed issue that is not
fixed is worse than an open one.
