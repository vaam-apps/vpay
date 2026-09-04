# `checkout.session.expired` — sabotage review of tier `opus`

Reviewed `git diff d0b602e..a0e39c9` on branch
`claude/exp-session-expired-opus`, 2026-09-04. The implementer's own account
is `docs/plans/step9-notes/session-expired.md`. Everything below was run in this worktree
against a real Postgres (`DOCKER_HOST=unix:///run/user/1000/docker.sock`).

## 1. Verification of the implementer's claims

Every claim in `opus.md` that could be checked was checked. All of them held.

| Claim | Verified how | Result |
|---|---|---|
| `cargo fmt --all --check` clean | ran it | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` clean | ran it | clean, `Finished` in 50s |
| Unit suites pass | `cargo nextest run -p vpay-db -p vpay-api -p vpay-worker -p vpay-core -p vpay-sdk` | **595 run, 595 passed, 0 skipped** |
| Integration suites pass | `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(webhooks) \| binary(worker_recovery)' --retries 2 -j 1` | **63 run, 63 passed, 0 skipped** |
| `checkout_sessions` is 23 cases | `cargo nextest list` | 23, and all six named cases exist under the names claimed |
| `pnpm --filter @vpay/sdk test` 172 passed | ran it | **172 passed, 0 skipped**, 9 files |
| `pnpm --filter @vpay/sdk typecheck` clean | ran it | clean |
| Migration `0029` keeps the seven existing types | read `0018` and `0029` side by side | all seven present verbatim, `checkout.session.expired` added |
| Migration convention | compared with `0022`/`0023`/`0024` | identical `DROP CONSTRAINT` + `ADD CONSTRAINT` shape; the repo has no down migrations and `0013` states the convention explicitly ("sqlx migrations run once"), so idempotency by `IF EXISTS` is not the convention here and its absence is correct |
| Guard proof 1 (delete the event insert → case 1 FAILS) | re-ran it | reproduced, byte for byte: `Error: exactly one checkout.session.expired was expected: []` |
| Guard proof 2 (split the transaction → case 6 FAILS) | re-ran it | reproduced: `left: ("expired", "unpaid")`, `right: ("open", "unpaid")` |
| "the deviation": deliveries come from the two-step outbox | read `docs/flows/webhooks.md` "Two-step outbox" and `vpay_db::settlement` | **the deviation is right** — see §4 |

## 2. Mutation table

Each mutation was applied to the tree, the named test run, the result
recorded, and the tree restored (`git status` clean after each).

| # | Mutation | Test run | Caught? |
|---|---|---|---|
| 1 | Delete the `events::insert_in_tx` call from `expire_due` (`if false`) | `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint` | **caught** — `exactly one checkout.session.expired was expected: []` |
| 2 | Split the transaction: run the flip on `self.pool`, open the transaction afterwards for the event | `a_failed_event_insert_leaves_the_session_open` | **caught** — session `expired` with no event |
| 3 | Make `data.object` render the `url` carrying the joined `client_secret` | `an_expired_session_snapshot_is_the_thirteen_keys_and_carries_no_credential` (unit) **and** `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint` (integration) | **caught by both** |
| 4 | Widen both guards to `status IN ('open', 'complete')`, so the sweep picks up a session the settlement finished | `a_session_finished_by_settlement_emits_no_expiry_event` | **caught** — "a settled session must not also be reported as expired" |
| 5 | Change `EVENT_SESSION_EXPIRED` to `"payment_intent.canceled"` | `an_expiry_event_is_listable_and_retrievable_within_its_tenant` | **caught** — `exactly one event was expected: []` |
| 6 | Drop the tenant filter from `Events::get_by_id` (`WHERE ($1 = $1) AND id = $2`) | `an_expiry_event_is_listable_and_retrievable_within_its_tenant` | **caught** — merchant B got `200` where `404` was required |
| 7 | Delete the live-charge `NOT EXISTS` from **`expire_due`'s `UPDATE` only**, leaving the identical clause in `due_for_expiry` | whole `checkout_sessions` binary, `--no-fail-fast` | **NOT caught** — 23 run, 23 passed. Finding 1 |
| 8 | Delete the live-charge `NOT EXISTS` from **`due_for_expiry` only** | `a_session_with_a_live_charge_is_neither_expired_nor_evented`, `the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one` | **NOT caught** — both passed. Finding 2 |

Mutations 7 and 8 are the two halves of the read-then-write split the review
brief asked about. Each half's copy of the guard covers for the other's under
every case that drives the sweep, because the sweep reads and writes back to
back; nothing pinned either copy on its own.

## 3. Findings

| # | Severity | Where | Evidence | Status |
|---|---|---|---|---|
| 1 | **correctness** | `backends/crates/vpay-db/src/checkout_sessions.rs:915` (the `UPDATE` in `expire_due`) | Mutation 7: deleting the write's `NOT EXISTS` leaves all 23 cases green. The clause is what stops a payer who confirms between `due_for_expiry` and `expire_due` from having their session flipped to `expired` and their merchant sent `checkout.session.expired` for an abandoned checkout — while the rail holds a live payment the settlement transaction then cannot record, because `settle_for_intent`'s `WHERE status = 'open'` no longer matches. The session would sit `expired`/`unpaid` over a payment that succeeded | **fixed** — `a_payer_confirming_between_the_read_and_the_write_keeps_the_session` stages the window deterministically. Guard-failure proof recorded in the commit |
| 2 | robustness | `backends/crates/vpay-db/src/checkout_sessions.rs:880` (the `SELECT` in `due_for_expiry`) | Mutation 8: deleting the read's copy is caught by nothing. Losing it does not corrupt anything (the write refuses), but it is the clause that stops a session a rail is holding from being *rendered* at all — an `evt_…` minted and an object built claiming an abandoned checkout | **fixed** — one assertion added to `a_session_with_a_live_charge_is_neither_expired_nor_evented`. Guard-failure proof in the commit |
| 3 | misleading-claim | `backends/crates/vpay-worker/src/handlers.rs:982`, `docs/flows/hosted-checkout.md:344`, `docs/reference/vpay-worker.md:199` | All three justify having no attempt counter with "a session that cannot be expired is not at the head of every subsequent page". `due_for_expiry` is `ORDER BY expires_at LIMIT 100` and a failing session keeps both its `status` and its horizon, so it heads every subsequent page permanently. A hundred of them fill the page, make `expired` zero, and so do not fire the progress-conditional immediate reschedule — the healthy sessions behind them wait an hour a pass | **fixed** — the three places now state what the query does and name the unbounded case. The conclusion (no counter) survives; nothing today makes a deterministic per-session failure reachable, and that is now written down rather than assumed |
| 4 | nit | `backends/migrations/0029_events-checkout-session-expired.sql:44` | Calls `cs_` "the third prefix" immediately after naming the three that already exist; `docs/flows/webhooks.md` says fourth. The two disagreed inside one change | **fixed** — comment only, no DDL touched |
| 5 | nit | `backends/tests/integration/tests/checkout_sessions.rs:2162` | The block opened as "Claim 11" and numbered its cases 11a–11f; the file already had a Claim 11, a Claim 12 and a Claim 13 | **fixed** — renumbered to 14, 14a–14g |

### Hunted and found clean

- **No secret in the event body or in a log.** The unit case and the
  integration case both assert on the serialised *string*; mutation 3 proves
  both are load-bearing. `expired_snapshot` is a constructor, not a
  parameter, so no call site can pass a `url` through. The `WARN` in
  `expire_due_sessions` names the session id, the merchant id and the error,
  and no column of the row.
- **The `PaymentIntentObject` 12-key tripwire is untouched**
  (`backends/crates/vpay-api/src/model.rs:1187`), and the new object has its
  own 13-key equivalent.
- **A second sweep creates no second event** — mutation-independent: the
  compare-and-swap on `status = 'open'` is what makes it so, and
  `a_second_sweep_writes_no_second_expiry_event` re-arms the singleton and
  re-runs the shipping loop rather than calling the repository.
- **A replayed sweep cannot enqueue a delivery twice.** The delivery jobs are
  keyed by `webhook_dedupe_key(delivery_id)` and inserted
  `ON CONFLICT (dedupe_key) DO NOTHING`, and a second sweep produces no second
  event to fan out in the first place.
- **`expired_snapshot`'s `status: "expired"` cannot disagree with the row.**
  Three `UPDATE`s touch `checkout_sessions` and no other write does: `expire`
  and `expire_due` both require `status = 'open'` and set `expired`, and
  `settle_for_intent` also requires `status = 'open'` and moves it out. So
  nothing can change any column of an `open` session without moving it out of
  `open`, which makes `expire_due`'s own compare-and-swap refuse. The snapshot
  is rendered from the pre-write row and the only field it patches is the only
  field the transition changes.
- **No test double.** `CheckoutSessions` has exactly one implementation,
  `PgRepositories`; ADR-0006 holds. `vpay-api` was already a dependency of
  `vpay-worker` (for `intent_snapshot`) and of the integration suite.
- **No `#[allow]`, `unwrap`, `expect` or `panic` outside tests** in the diff;
  the only `expect`s are in `#[cfg(test)]` modules and in the integration
  suite, which carries the file-level allow it already had.
- **The housekeeping log line** carries `checkout_sessions = <expired>` beside
  the three existing counters, plus `checkout_sessions_page`.
- **Both parity rows name tests that exist** and that the SDK suites run;
  `just verify` (which runs `verify-status`, `verify-errors` and
  `verify-sdk-parity`) is green.

### Found and left, with the reason

- **nit / robustness: `due_for_expiry` sorts without an index for it.** The
  read is `WHERE status = 'open' AND expires_at <= $1 AND NOT EXISTS (…)
  ORDER BY expires_at LIMIT 100`, and `0028` gives `checkout_sessions` no
  index on `expires_at` — its two partial indexes are both on
  `payment_intent_id WHERE status = 'open'`. So the planner restricts to the
  open set and then sorts all of it to take a hundred, once an hour, where the
  bulk `UPDATE` this replaced never sorted. The open set is bounded by a day's
  creates, so this is a cost rather than a defect, and nothing measured it.
  **Not fixed.** The obvious answer is a partial index on
  `(expires_at) WHERE status = 'open'` in an `0030`, and adding an index to a
  payments table on reasoning alone — with no measurement, on a query whose
  real cardinality nobody here knows — is the kind of plausible addition
  `CLAUDE.md` warns about. It is a maintainer's call with a number attached to
  it, and it is recorded here so it is visible rather than made silently.

### Observed, not a finding

- The second parity row is phrased as a statement about what a *delivered*
  event carries, and the two tests it names assert that against fixtures the
  tests themselves write — they can only prove each SDK handles such a body,
  not that the server produces one. The server-side proof is
  `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint`, which the
  row does not cite. Left as it is: the table is a per-SDK capability table by
  ADR-0015's decision 1, and the file's own note two paragraphs above says the
  two SDKs are at parity on the capability rather than the shape.
- **Pre-existing, not introduced, and out of scope:** a payer who confirms
  inside the *single-statement* window of `expire_due`'s `UPDATE` — after its
  snapshot and before its commit — can still have their session expired, and
  the settlement that follows cannot flip it back. That race existed
  identically in the bulk `UPDATE` this change replaced; the change makes it
  audible (a spurious event) rather than creating it. Not fixed here, and not
  claimed to be.

## 4. The deviation: deliveries come from the outbox, not the flip's transaction

The task brief asks for the event **and** the deliveries "in the same
transaction as the status flip", and the implementer built the event in the
transaction and left the deliveries to the existing two-step outbox. **That is
the right call, and the brief's own governing clause is what makes it right:**
it asks for the deliveries "exactly as `payment_intent.succeeded` is — same
table, same fan-out, same signing, same retry ladder", and
`payment_intent.succeeded`'s deliveries are created by TX 2
(`webhooks::handle_fan_out`), never by the settlement transaction.
`docs/flows/webhooks.md`'s "Two-step outbox" section is explicit that the
split is deliberate and says why; fanning out inline would make a business
transaction depend on reading the endpoint table.

**The brief's crash-safety requirement is met.** "A crash between the flip and
the event must be impossible" is a property of the flip and the event row, and
both are in one transaction (`CheckoutSessions::expire_due`,
`backends/crates/vpay-db/src/checkout_sessions.rs:902`). Mutation 2 confirms
the test that pins it is real: split the transaction and
`a_failed_event_insert_leaves_the_session_open` fails with the session
`expired`. A crash after the commit and before the fan-out loses nothing —
the event row carries `fanout_state = 'pending'` and the drain is a singleton
that finds it.

## 5. Final gate

Run on the final tree, after all six commits.

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-worker -p vpay-core -p vpay-sdk` | **595 run, 595 passed, 0 skipped** |
| `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(webhooks) \| binary(worker_recovery)' --retries 2 -j 1` | **64 run, 64 passed, 0 skipped**; two flaky-on-first-try, both pre-existing cases (`a_declined_payment_leaves_the_session_expired_and_failed`, `a_session_pins_the_tenants_first_key_unless_the_merchant_names_another`), both green on retry. Host flakes: another agent was driving the same rootless Docker daemon throughout |
| `pnpm --filter @vpay/sdk test` | **172 passed, 0 skipped** |
| `pnpm --filter @vpay/sdk typecheck` | clean |
| `just verify` | ok — `verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity` all pass |
| `just verify-ignored` | **1147 total, 42 test binaries, 0 ignored** (minimum 1080). 1146 before this review; the one added case is finding 1's |
| `just test-doc` | **86 passed, 0 failed, 1 ignored** — unchanged; the ignored one is `sdks/rust`'s README block and is pre-existing |

## 6. What I did not check

- **No `checkout.session.expired` was delivered to a receiver.** The
  implementer says so themselves and it is the honest reason their status row
  is 🟡. I did not close that gap: doing it properly means a WireMock endpoint,
  a full ladder run and a signature verified with both SDKs, which is a piece
  of work rather than a review fix.
- **Cypress, `just demo`, and the `worker_e2e` / conformance suites** — none is
  in the brief's gate list and none exercises this path.
- **Concurrency under real contention.** Mutations 7 and 8 stage the
  read-then-write window deterministically rather than racing two sweeps; I did
  not run two workers against one deployment.
- **`docs/status.md`** — the branch does not edit it (correctly, per the task
  brief), and I did not either. The rows in `opus.md` are what a maintainer
  would apply; I corrected the case list in them for the case this review
  added.
