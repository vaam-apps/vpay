<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 4 — the worker: implementation-ready design

Decisions taken by the orchestrator (do not reopen): (1) the worker keeps linking `vpay-api` for
`ResourceConfig`/`boot`; (2) a redirect-rail charge stuck in `submitting` is failed
(`provider_unavailable`, intent back to `requires_payment_method`) per crash-safety.md — keyed on
`Capabilities::flow`; (3) a post-submission decline goes through a new writer that moves the status
and stamps the error in one statement (verify `already_charged` gives the terminal wording for a
`failed` charge); (4) `events` rows for terminal transitions only this step; (5) new nullable
`charges.provider_txn_id` in 0021; (6) `prompt_ttl_seconds`/`prompt_expired_at` deferred, named in
reconciler.md's Status; (7) the e2e PENDING→SUCCESSFUL scenario lives in the shared WireMock tree at
priority 5 keyed on documentation MSISDN `237600000ce0`; (8) roadmap Phase 4a/4b split lands with
this step if the Step 3 docs pass has not already done it.

Read: `AGENTS.md`, `docs/flows/{crash-safety,reconciler,payment-lifecycle,failures,webhooks}.md`, `docs/plans/*`, `docs/roadmap.md` Phase 4/5, `docs/runbooks/*`, `docs/status.md` rows 460/473/474/475/491/492, `vpay-worker`, `vpay-worker-bin`, `vpay-db` (charges/payment_intents/provider_requests/idempotency/lock_keys), migrations 0003/0004/0014/0016/0018/0019/0020, `vpay-core::state`, `vpay-provider`, both adapters' `query_status`, `vpay_api::v1::{boot,mod,payment_intents}`, the WireMock mappings.

# Step 4 — the worker: implementation-ready design

## 0. Five things that are not what the ticket implies (verify these first)

**S1 — `next_status` cannot express a single worker transition, by design.** `vpay_core::state::Transition` is "one of the three verbs a *merchant* can apply" and `next_status(Processing, _) → None` (`/home/selast/dev/vpay/.claude/worktrees/vpay-production-readiness-56b122/backends/crates/vpay-core/src/state.rs:167-177`, `:210-227`). The doc comment explicitly reserves the rail-driven edges for the reconciler. **Do not extend `Transition`** — add a sibling pure function (§3). Anyone who "just adds a variant" makes `processing → succeeded` reachable from an HTTP handler, which is the thing that enum exists to prevent.

**S2 — the crash-safety recovery table is push-only, and Orange breaks it.** `orange_money::query_status` returns `ProviderError::Config` when `ref_extra` has no `pay_token` (`backends/crates/vpay-adapter-orange-money/src/lib.rs`, first block of `query_status`). A `submitting` Orange charge *by definition* has no `pay_token` — it never got the submit response. So "row with `status_code IS NULL` → poll" is unexecutable on the redirect rail. crash-safety.md:57-69 already answers this: "that `order_id` is dead: abandon it". Recovery must branch on `Capabilities::flow` (a capability value, ADR-0002-legal), never on a rail code. Implemented uniformly, every crashed Orange confirm dead-letters as `Config`.

**S3 — `vpay-db` has no writer for any terminal charge state.** `charges::mark_submitted` and `mark_failed` both guard `WHERE id = $1 AND state = 'submitting'` (`backends/crates/vpay-db/src/charges.rs:241-289`, `:293-319`). Nothing writes `pending`, `unresolved`, `succeeded`, and nothing writes `payment_intents.amount_received` (column exists, `backends/migrations/0003_create-payment-intents.sql:29`). All of §3's writes are new.

**S4 — the PENDING→SUCCESSFUL scenario cannot be steered from a real `confirm`.** Both scenario mappings key on the fixed reference `…0ce0` (`backends/tests/conformance/wiremock/mtn/mappings/requesttopay-scenario.json`, orange `transactionstatus.json`), and `confirm` mints `Uuid::new_v4()` (`backends/crates/vpay-api/src/v1/payment_intents.rs:648`). The MSISDN-steering precedent (`requesttopay.json`, "a payer the rail does not know, selected by MSISDN") works for the POST only; MTN's status query is a **GET carrying no body**. See §6.

**S5 — `vpay-worker-bin` already depends on `vpay-api`.** `Cargo.toml` lists `vpay-api.workspace = true` and `main.rs:189-190` calls `vpay_api::v1::boot::{adapters_by_code, boot_seeds}`. The "does the worker depend on `vpay_api`?" question is already decided in the affirmative; §5 recommends not reopening it.

Also: `roadmap.md:569` still says "Phase 4 — The rails / Not started" and its Scope still contains the recovery table, while `docs/status.md:492` already cites "the Phase 4a/4b split in docs/roadmap.md". The split has **not** landed in `roadmap.md`. `charges` has **no** `prompt_expired_at` column (grep: only `docs/flows/reconciler.md:19`), so reconciler.md's `prompt_ttl_seconds` behaviour needs a migration or is out of scope — see Q6.

## 1. Schema — migration `0021_create-jobs.sql`

```sql
CREATE TABLE jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind TEXT NOT NULL,
    dedupe_key TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    run_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INT NOT NULL DEFAULT 0,
    locked_at TIMESTAMPTZ, locked_by TEXT,
    last_error TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT kind_is_known CHECK (kind IN ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges')),
    CONSTRAINT payload_is_object CHECK (jsonb_typeof(payload) = 'object'),
    CONSTRAINT lock_is_paired CHECK ((locked_at IS NULL) = (locked_by IS NULL)),
    CONSTRAINT last_error_length CHECK (last_error IS NULL OR char_length(last_error) <= 2000)
);
CREATE UNIQUE INDEX jobs_dedupe_key ON jobs (dedupe_key);
CREATE INDEX jobs_claimable_idx ON jobs (run_at) WHERE locked_at IS NULL;
CREATE INDEX jobs_leased_idx ON jobs (locked_at) WHERE locked_at IS NOT NULL;
```

`kind_is_known` mirrors `events.type_is_a_documented_event` (`0018`); `lock_is_paired` mirrors `response_is_paired` (`0016`). `deliver_webhook` is **not** in the CHECK — Step 5 adds it in its own migration, so this step cannot enqueue one by accident.

Claim (exactly this, one row per task):

```sql
UPDATE jobs SET locked_at = now(), locked_by = $1, attempts = attempts + 1
WHERE id = (SELECT id FROM jobs WHERE run_at <= now() AND locked_at IS NULL
            ORDER BY run_at FOR UPDATE SKIP LOCKED LIMIT 1)
RETURNING id, kind, dedupe_key, payload, run_at, attempts, locked_at, locked_by, last_error, created_at;
```

`locked_at IS NULL` (not "or lease expired") keeps `jobs_claimable_idx` exact. **Lease expiry is a separate reaper**, run by `sweep_expired`: `UPDATE jobs SET locked_at = NULL, locked_by = NULL, last_error = 'lease expired' WHERE locked_at < now() - $1::interval` (default 5 min, ≥ 4× `DEFAULT_REQUEST_TIMEOUT` = 20 s, `backends/crates/vpay-provider/src/lib.rs:312`). Completion is `DELETE FROM jobs WHERE id = $1 AND locked_by = $2`; reschedule is `UPDATE jobs SET run_at = now() + $2, locked_at = NULL, locked_by = NULL, last_error = $3 WHERE id = $1 AND locked_by = $4` — the `locked_by` guard is the ABA close, exactly as `idempotency`'s `claim_id` is (`backends/crates/vpay-db/src/idempotency.rs:154`).

**Initial enqueue: same transaction as the charge insert, not a periodic scan.** `insert_charge` (`payment_intents.rs:1011-1040`) already owns a transaction that commits the charge *before* the network call; adding `jobs::enqueue_in_tx(kind='poll_charge', dedupe_key = format!("poll:{charge_id}"), payload = {charge_id}, run_at = now())` to it means **all three** crash-safety.md kill points leave a job behind. Enqueueing at step 6 (`persist_submitted`) would leave kill points 1 and 2 — precisely the recovery cases — with no job at all, and recovery would depend on a scan. Keep a low-frequency `scan_live_charges` job as a **backstop only** (`SELECT id FROM charges WHERE state IN ('submitting','submitted','pending','unresolved') AND updated_at < now() - interval '10 minutes'` — this is what `charges_live_idx`, `0014:103-104`, is for) enqueuing `poll_charge ... ON CONFLICT (dedupe_key) DO NOTHING`. It covers rows written before this migration and a job lost to operator error; it is not load-bearing.

## 2. Job kinds

All handlers are `async fn(&PgPool, &BTreeMap<String, Box<dyn ProviderAdapter>>, &ResourceConfig, &RecoveryPolicy, Job) -> Result<Outcome, JobError>` where `Outcome = { Done, RescheduleAfter(Duration) }`. **`vpay-worker` gets no `sqlx` dependency** (it is dev-only today, `Cargo.toml:48`): every transaction lives in `vpay-db`, so the worker composes repository calls.

| kind | payload | transaction boundary | idempotency after a crash | `JobError` | decision |
|---|---|---|---|---|---|
| `poll_charge` | `{charge_id}` | read charge+latest `provider_requests` (no tx); `query_status`; then **one** `vpay_db::settlement::apply_terminal` tx = charge UPDATE + intent UPDATE + `events` INSERT | every write is a compare-and-swap on the *expected* current state; re-running after a commit matches 0 rows → `Outcome::Done`, not an error. `provider_requests::insert_pending`/`record_response` around each query (operation `'query_status'`) | `Provider(_)` from the adapter; `Poisoned` if `charge_id` names no row or `charges.state` is unparseable; `Exhausted` when the ladder passes 24 h | `RetryAfter{delay,alert}` → reschedule at `delay`; `Terminal`/`DeadLetter` per §3 |
| `resubmit_charge` | `{charge_id, not_found_streak, first_not_found_at}` | `provider_requests::insert_pending(attempt = n+1)` → `adapter.submit` → `record_response` → `charges::mark_submitted` (still guards `submitting`, correct here) | **same `provider_reference_id`**, read from `charges` — never regenerated (crash-safety.md:32-38). MTN's 409 already maps to `Submitted` (`requesttopay.json`, "duplicate reference is reported as already existing"), so a re-run is a no-op at the rail | as above | on success enqueue/reschedule `poll:<charge_id>` |
| `sweep_expired` | `{}` | three independent statements, each own tx | naturally idempotent (deletes) | `Db(_)` | reschedule hourly, always |
| `deliver_webhook` | — | **Step 5. Name only. Not in `kind_is_known`.** | | | |

`sweep_expired` calls `vpay_db::idempotency::sweep_expired` and `vpay_db::delete_expired_client_assertion_jtis` — both exist and are today called **once at `vpay-server` boot** (`backends/apps/vpay-server/src/main.rs:379`, `:401`). Move both call sites out of the server into this job (status.md:485 and :491 both name the missing scheduler), plus the lease reaper. Seed it at worker boot with `ON CONFLICT (dedupe_key) DO NOTHING`, `dedupe_key = 'sweep:expired'`.

## 3. State transitions, exhaustively

New in `vpay-core` (`src/settlement.rs`), pure, no `Transition` variant added (S1):

```rust
pub enum Settlement { Stay, Live(ChargeState), Succeeded, Failed(FailureCode) }
pub const fn settle(status_kind: StatusKind, charge: ChargeState) -> Option<Settlement>;
```

| `ChargeStatus` | `submitting` | `submitted` | `pending` | `unresolved` |
|---|---|---|---|---|
| `Pending` | charge → `pending`; intent unchanged (`processing`); no event; reschedule `poll_delay(n)` | → `pending`; same | stay `pending`; reschedule; at 24 h → `unresolved` + `JobError::Exhausted` | stay; reschedule at `UNRESOLVED_POLL_INTERVAL` |
| `Succeeded{txn}` | → `succeeded` | → `succeeded` | → `succeeded` | → `succeeded` |
| `Failed{code,raw}` | → `failed` | → `failed` | → `failed` | → `failed` |
| `NotFound` | **recovery, §4** | recovery | treat as `Pending`, bump streak | treat as `Pending`, bump streak |

`Succeeded` row, one transaction: `charges.state='succeeded'`, `provider_txn_id` recorded (see Q5); `payment_intents SET status='succeeded', amount_received = amount WHERE status = 'processing'` (redirect rails must first move `requires_action → processing`, payment-lifecycle.md:29 — do it in the same statement chain: `WHERE status IN ('processing','requires_action')`); `events` row `type='payment_intent.succeeded'`, `fanout_state='pending'`, `object_id = pi_…`, `data` = the wire object. `Failed` row: `charges.state='failed'` + `failure_code`/`failure_raw`; `payment_intents::record_payment_error` with `expected` = current status — **status stays `requires_payment_method`? No**: a confirmed intent is `processing`/`requires_action`, and payment-lifecycle.md:57-59 says it *returns to* `requires_payment_method`. So this is a real transition (`processing → requires_payment_method`) plus the error pair, and `record_payment_error` (`payment_intents.rs:427`) does **not** move status — it needs a sibling `fail_after_submission`. Event `payment_intent.payment_failed`. Both cases end the job (`Outcome::Done` + `DELETE`).

## 4. Recovery table, precisely

Read: the charge, and `SELECT * FROM provider_requests WHERE charge_id=$1 AND operation='submit' ORDER BY sent_at DESC LIMIT 1`.

- **flow is `Redirect`** → do not poll. Charge → `failed` / `provider_unavailable`, intent → `requires_payment_method` + `last_payment_error` (crash-safety.md:64-69, "that order_id is dead"). This is the S2 branch.
- **No row** → `resubmit_charge`, same reference.
- **Row, `status_code IS NULL`** → `poll_charge`. On `NotFound`, increment `not_found_streak` in the payload and set `first_not_found_at` on the first. When `streak >= policy.not_found_streak (3)` **and** `now - first_not_found_at >= policy.not_found_window (60 s)`, enqueue `resubmit_charge`. Any non-`NotFound` answer resets both fields.
- **Row with a status code** (including the `0` sentinel, `0020`) → advance from §3.

`pub struct RecoveryPolicy { pub not_found_streak: u32, pub not_found_window: Duration, pub lease: Duration, pub unresolved_after: Duration }` with `Default` = `(3, 60s, 5min, 24h)`, constructed in `main` and passed to every handler — the tests override it (no `#[cfg(test)]` seam, AGENTS.md rule 1).

## 5. The worker loop (`vpay-worker-bin`)

Replace the heartbeat (`main.rs:276-297`) with `vpay_worker::run_loop(pool, adapters, config, policy, concurrency, shutdown)`. N tasks (`--worker-concurrency`, default 4), each: claim → run → settle → on empty claim sleep 1 s. Metrics log per completed job at `tracing_level(err.severity())` with `alert = true` when `severity() == Page` (`error.rs:300-322` says the loop must add that field), plus a 60 s gauge line: claimed / succeeded / rescheduled / dead-lettered / oldest `run_at`.

**Drain:** on `shutdown_signals.wait()`, stop claiming, `tokio::time::timeout(Duration::from_secs(args.common.shutdown_grace_seconds), join_all(tasks))`. On timeout, release the still-held leases (`UPDATE jobs SET locked_at=NULL WHERE locked_by=$1`) and exit non-zero, mirroring `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs`. This closes status.md:474's worker half.

**Adapters/config:** keep `adapters(http)` at `main.rs:106` and the existing `vpay_api::v1::boot::adapters_by_code` call; add `let resource_config = vpay_api::ResourceConfig::from_config(&config)?` and pass `rail.provider_config()` per charge (`backends/crates/vpay-api/src/v1/mod.rs:283-286`). **Do not move `ResourceConfig` to `vpay-config`** — S5, the edge already exists and `from_config`'s doc comment (`mod.rs:308-311`) says both binaries building it identically is the point. Remove `drop(pool)` at `main.rs:251`.

## 6. Tests

- **Crash injection at the three points**, without an in-process fake: a harness in `backends/tests/integration/tests/worker_recovery.rs` drives the *handler functions* against real Postgres + WireMock, and produces each state by **writing the state a crash leaves**, not by faking a process death — (1) `charges::insert_for_intent` + commit, no `provider_requests` row; (2) + `insert_pending`, no `record_response`; (3) + `record_response(status)` but no charge UPDATE. Assert `poll_charge` resolves each with exactly **one** distinct `provider_reference_id` across all `provider_requests` rows for that charge. This is the honest framing: it proves the recovery table, and the doc must not claim a `SIGKILL` test (crash-safety.md:154-159 currently disclaims exactly that).
- **PENDING→SUCCESSFUL from a real confirm** — S4. Recommended fix: add to `backends/tests/conformance/wiremock/mtn/mappings/requesttopay-scenario.json` a **priority-5** scenario pair on `GET /collection/v1_0/requesttopay/.*` in a new scenario `mtn-e2e-poll`, entered by a POST whose `$.payer.partyId` equals a new documentation MSISDN (`237600000ce0`), same technique and same justification as the existing MSISDN mapping. Priority 5 keeps every reference-keyed (priority 1) conformance case winning and still beats the catch-all 202/SUCCESSFUL (priority 10). Verify against the conformance suite before relying on it.
- `NotFound` ×3 → resubmit, with `RecoveryPolicy { not_found_streak: 3, not_found_window: 50ms }` and the `…0404` reference (`requesttopay-status.json`). No sleeps.
- `unresolved` escalation: `RecoveryPolicy { unresolved_after: 0 }`, assert charge → `unresolved`, job rescheduled ~1 h, `alert: true` logged, **not** dead-lettered.
- End-to-end: spawn `run_loop` in-process against the same pool the `confirm` test uses; poll `GET /v1/payment_intents/{id}` until `succeeded`; assert `amount_received == amount` and exactly one `events` row of type `payment_intent.succeeded` with `fanout_state='pending'`.
- Drain under load: 50 jobs whose handler blocks; SIGTERM; assert exit within grace + every lease released.

## 7. Work split (disjoint files, signatures verbatim)

**A — schema + db.** `backends/migrations/0021_create-jobs.sql`; `backends/crates/vpay-db/src/jobs.rs`, `events.rs`, `settlement.rs`. Owns:
```rust
pub struct JobRow { pub id: Uuid, pub kind: String, pub dedupe_key: String, pub payload: serde_json::Value, pub run_at: OffsetDateTime, pub attempts: i32, pub locked_by: Option<String>, pub last_error: Option<String> }
pub async fn enqueue_in_tx(tx: &mut PgConnection, kind: &str, dedupe_key: &str, payload: &serde_json::Value, run_at: OffsetDateTime) -> Result<bool, DbError>;
pub async fn claim(pool: &PgPool, worker_id: &str) -> Result<Option<JobRow>, DbError>;
pub async fn finish(pool: &PgPool, id: Uuid, worker_id: &str) -> Result<bool, DbError>;
pub async fn reschedule(pool: &PgPool, id: Uuid, worker_id: &str, delay: Duration, last_error: Option<&str>) -> Result<bool, DbError>;
pub async fn release_all(pool: &PgPool, worker_id: &str) -> Result<u64, DbError>;
pub async fn reap_expired_leases(pool: &PgPool, lease: Duration) -> Result<u64, DbError>;
// settlement.rs — the one transaction of §3
pub async fn apply_succeeded(pool: &PgPool, charge_id: &str, provider_txn_id: Option<&str>, event_id: &str, event_data: &serde_json::Value) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError>;
pub async fn apply_failed(pool: &PgPool, charge_id: &str, code: &str, raw: &str, message: &str, event_id: &str, event_data: &serde_json::Value) -> Result<Option<(ChargeRow, PaymentIntentRow)>, DbError>;
pub async fn set_live_state(pool: &PgPool, charge_id: &str, expected: &str, new: &str) -> Result<bool, DbError>;
pub async fn latest_submit_attempt(pool: &PgPool, charge_id: &str) -> Result<Option<AttemptRow>, DbError>;
pub async fn live_charges_stale_since(pool: &PgPool, cutoff: OffsetDateTime, limit: i64) -> Result<Vec<String>, DbError>;
```
Plus `charges::get_by_id`, and the sweeps moved out of `vpay-server/src/main.rs:379,401`.

**B — worker logic.** `backends/crates/vpay-core/src/settlement.rs`; `backends/crates/vpay-worker/src/{jobs.rs, handlers.rs, recovery.rs}`. Owns `RecoveryPolicy`, `settle`, and `pub async fn handle(pool, adapters, resource_config, policy, job) -> Result<Outcome, JobError>`. No `sqlx`.

**C — process + tests + API.** `vpay-worker-bin/src/main.rs` (`run_loop`, drain, metrics, `--worker-concurrency`); the `enqueue_in_tx` call inside `insert_charge` (`vpay-api/src/v1/payment_intents.rs:1011`); `backends/tests/integration/tests/worker_recovery.rs` + `worker_e2e.rs`; the WireMock scenario mapping; demo step 6.

**Docs.** status.md rows: `Poll ladder` (:475), `Reconciler` (:492), `JobError` (:460, drop "nothing calls decision()"), `--shutdown-grace-seconds` (:474), `Idempotency` (:491 reason 2), `Client-assertion replay` (:485), `Database schema` (:480, migration 0021). Flow docs: `crash-safety.md` Status (:95, :149-167), `reconciler.md` Status (:57-91), `payment-lifecycle.md` (:136-139), `webhooks.md` (:42-44 — TX 1 now real, TX 2 still Step 5). `roadmap.md` Phase 5 and the missing 4a/4b split. Runbooks `unresolved-charges.md` and `provider-error-rate.md` become exercisable for the first time.

---

# Decisions needed from a human

1. **Does the worker link `vpay-api` for `ResourceConfig`, or does the projection move to `vpay-config`?** *Default: link it — it already does (`vpay-worker-bin/Cargo.toml`, `main.rs:189`).* Gained: zero churn, one derivation both binaries share, which `ResourceConfig::from_config`'s doc comment says is the point. Lost: the worker's dependency graph keeps a crate named "api" whose HTTP router it never mounts, and a future `vpay-api` change can force a worker rebuild/redeploy. Moving it is ~150 lines and a `vpay-config → vpay-provider` edge that already exists.

2. **On a `Redirect`-rail charge stuck in `submitting`, do we fail it (crash-safety.md:64-69) or leave it live for a human?** *Default: fail it — charge `failed` / `provider_unavailable`, intent `requires_payment_method`, per the document.* Gained: the merchant can act (a new intent) instead of waiting on a charge nobody can query; the document's own reasoning is airtight (the payer was never handed the URL). Lost: if a future redirect rail *does* let the payer act before we persist, this auto-fails a live payment. Mitigation is that the rule is keyed on `Capabilities::flow`, so a rail that changes shape changes branch.

3. **Does `processing → requires_payment_method` after a post-submission decline go through a new `vpay-db` writer, or does the worker reuse `record_payment_error` and leave the status alone?** *Default: new writer `fail_after_submission` that moves the status **and** stamps the error in one statement.* Gained: matches payment-lifecycle.md:57-59 and the state diagram exactly; a merchant polling `GET` sees a resolved intent. Lost: an intent that was `processing` returns to a status that looks confirmable, and `confirm` would refuse it only because of the live-charge guard (`already_charged`, `payment_intents.rs:1077`) — verify that guard covers a `failed` charge before shipping (it currently advises "a retry is a new intent", `a_terminal_charge_still_says_a_retry_is_a_new_intent`).

4. **Is the `events` row written for the `Pending`/`unresolved` milestones too, or only terminal transitions?** *Default: terminal only this step.* Gained: bounded scope, and the row set matches the decision already taken ("write the `events` row in the same transaction as every terminal transition"). Lost: reconciler.md:18-23 wants `payment_intent.processing` with `expired: true` at `prompt_ttl_seconds`, and merchants' "check your phone" UI has nothing to turn off. That is Q6.

5. **Where does `provider_txn_id` from `ChargeStatus::Succeeded` go? There is no column.** *Default: a new nullable `charges.provider_txn_id TEXT` in `0021`.* Gained: the reconciliation field `runbooks/unresolved-charges.md` step 4 needs by name; `provider_ref_extra` is documented as *rail key material* (`0019` header) and stuffing it there is the mistake that migration exists to prevent. Lost: one more column, one more migration line.

6. **`prompt_ttl_seconds` / `prompt_expired_at`: in Step 4 or deferred?** *Default: defer, and say so in `reconciler.md`'s Status.* Gained: Step 4 stays "drive charges to terminal", and the column + config key + `payment_intent.processing` event are a coherent unit for Step 5 alongside fan-out. Lost: reconciler.md keeps documenting a behaviour nothing implements, and the doc's Status section must name it explicitly or the page starts lying.

7. **Does the MTN e2e scenario mapping go in the shared `backends/tests/conformance/wiremock/` tree (which `compose.yml` and the conformance suite also mount), or a new integration-only tree?** *Default: shared tree, priority 5, keyed on a new documentation MSISDN.* Gained: one stub definition, `just up`/demo behaviour unchanged, and it follows the precedent already argued at length in `requesttopay.json`. Lost: a mapping that exists for one integration test now sits in the conformance fixtures, and a careless future priority change can silently break a conformance case. A separate tree costs a second `start_wiremock` fixture and a duplicated `token.json`.

8. **Is `roadmap.md`'s Phase 4/4b split part of this step's commit?** *Default: yes — Step 4 must land it, since `status.md:492` already cites a split that does not exist in `roadmap.md:569-600`.* Gained: the two documents stop contradicting each other, and Phase 4's Definition of Done stops claiming recovery. Lost: nothing, beyond the edit; the concurrent docs agent may be doing it already — coordinate before duplicating.
---

# Outcome (2026-09-03, branch `claude/step4-worker`)

*Written after the pass landed. `docs/status.md` and the flow docs are the
record from here; this section exists so a reader of the design can see what
became of it.*

**What landed.** Migration `0021` (the `jobs` table plus
`charges.provider_txn_id`); `vpay_db::{jobs, settlement, events}`;
`vpay_core::settlement::{settle, contradiction}`;
`vpay_worker::{jobs, handlers, recovery, run_loop}`; the enqueue inside
`insert_charge`'s transaction; a real `vpay-worker-bin` (boot step 4 reconcile
under the `CONFIG_RECONCILE` advisory lock, lease reaping, singleton seeding,
`run_loop`, bounded drain, non-zero exit on a timed-out drain); the MTN
`PENDING → SUCCESSFUL` WireMock scenario; a sixth demo step that ends in
`succeeded`; 20 `worker_recovery` + 3 `worker_e2e` container-backed tests. The
whole workspace suite after the third remediation: **806 tests run, 806 passed, 0 skipped**.

**Where the implementation deviates from this design, and why.**

- **`contradiction` lives in `vpay-core`, not in the worker**, and it is
  wired but its call sites are untested. The design never named it; it came
  out of review finding F4. The classifier sits beside `settle` because it is
  the same table read the other way and must stay total in both dimensions.
  One call site is currently unreachable (behind the terminal guard, written
  out deliberately rather than removed) and the other needs a multi-worker
  race no test stages — so "vpay would tell you if a rail reversed a settled
  payment" is not yet a claim this repository can make.
- **Lease reaping is not only the `sweep_expired` job** (§1 and §2 said it
  was). It also runs at worker boot, unconditionally and before seeding, and
  then on a dedicated timer every `lease / 2` floored at the idle poll —
  because the sweep is itself a row in `jobs`, so a worker that died holding
  it would leave the only reaper unclaimable. Finding F2.
- **The intent guard is wider than §3's.** `SETTLEABLE_STATUSES` includes
  `requires_payment_method`, because that is where all three kill points leave
  a crashed confirm's intent. Finding F1.
- **The gauge line names two fields differently from §5** — `finished` rather
  than "succeeded" (it counts a declined charge too; calling it `succeeded`
  would read as a payment count) and `queue_behind_seconds` rather than the
  oldest `run_at` (a duration is the thing alerting thresholds; a timestamp
  would have to be diffed by whoever read the line). It also carries
  `worker_id`, which §5 omitted.
- **`resubmit_charge` merges `provider_ref_extra`** instead of assigning it,
  so a push rail's empty answer on a second submit cannot erase key material.
  Finding F5.
- **Q6 was taken as designed:** `prompt_ttl_seconds` / `prompt_expired_at` are
  deferred and named in `reconciler.md`'s Status. Q8 (the roadmap 4a/4b split)
  landed here.
- **The crash tests do not kill a process.** They write the state each kill
  point leaves and run the real handlers against it — §6's own framing, kept
  honest in `crash-safety.md` rather than described as a `SIGKILL` test.

**Review findings, one line each.**

- **F1 (blocker).** A crash between the charge insert and `persist_submitted`
  left the intent at `requires_payment_method`, which the settlement guard
  (`processing | requires_action`) rejected — so the recovered charge
  dead-lettered instead of settling. Fixed by widening the guard to
  `SETTLEABLE_STATUSES`.
- **F2 (high).** Expired leases were reaped only inside the hourly
  `sweep:expired` job and never at boot, so a job stranded by a `SIGKILL`
  waited up to an hour — forever, if the stranded job was the sweep. Fixed by
  reaping at boot before seeding, plus a reaper every `lease / 2`.
- **F3 (high).** A rail that never answered rode the retry ladder forever,
  because the `unresolved_after` horizon was only reached after a *successful*
  status query. Fixed by evaluating the horizon before the query. **The first
  fix over-corrected** — it returned before the query, so a charge past the
  horizon stopped being polled at all, contradicting this document's "still
  polled, once an hour" and "a late success at hour 30 is the normal
  transition". Caught by the remediation review and corrected in a second
  remediation: past the horizon the worker still queries the rail hourly, a
  terminal answer settles through the ordinary path, and a failed or
  non-terminal answer keeps the charge `unresolved` and re-raises the alert.
- **F4 (medium).** A rail answer contradicting an already-settled charge was
  silently dropped as `Done`. Fixed by `vpay_core::settlement::contradiction`
  (table-tested) wired to an `error!(alert = true, …)` log — **the classifier
  is tested and the two call sites are not**, which is why the Settlement row
  in `docs/status.md` is 🟡 and not ✅.
- **F5 (low).** `resubmit_charge` overwrote `provider_ref_extra` wholesale.
  Fixed in SQL (`COALESCE(existing, '{}') || new`), with a `NULL` argument
  leaving the column untouched.

Conventions review, addressed in the same remediation: the gauge line lacked
`worker_id`; `oldest_runnable_run_at` had no test; the "no `sqlx`" comments in
`vpay-worker` overclaimed (the rule is about where *statements* live, not about
the type being absent); the WireMock scenario lacked its demo-reset note; and
`worker_e2e` did not assert the contents of `events.data`.
