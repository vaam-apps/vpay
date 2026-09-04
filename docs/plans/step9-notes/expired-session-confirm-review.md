# exp2 — sabotage review of the `opus` implementation

Reviewed `git diff 3b251f6..HEAD` on `claude/exp2-expired-confirm-opus`
(four commits: the `vpay-db` read, the `ApiError` variant, the gate in
`return_trip`, the seven integration cases). The implementer's own account is
`docs/plans/step9-notes/expired-session-confirm.md`; every number in it was re-measured here
rather than taken on trust. Everything below was measured on 2026-09-05 in
this worktree with `DOCKER_HOST=unix:///run/user/1000/docker.sock`,
`CARGO_BUILD_JOBS=4`, container suites at `-j 1`.

## 1. Claims checked against the tree

| Claim in `opus.md` | Verdict |
|---|---|
| `find_latest_by_intent` — `ORDER BY seq DESC LIMIT 1`, no status filter | true (`backends/crates/vpay-db/src/checkout_sessions.rs:844`) |
| `ClosedSession { Expired, Complete }` + `ApiError::CheckoutSessionNotOpen`, classified | true (`backends/crates/vpay-api/src/error.rs:239`, `:573`, `:853`, `:909`, `:1041`) |
| the gate sits after `load_confirmable_intent` and before `open_attempt` | true — `confirm_once`'s body is `load_confirmable_intent` → `admit_confirm` → `resolve_rail` → `open_attempt` → `submit_to_rail` → `finish_confirm`, in that order (`v1/payment_intents.rs:786-812`) |
| one read answers both the verdict and the return URL | true — `admit_confirm` calls `find_latest_by_intent` exactly once (`v1/return_trip.rs:145-172`) |
| 31 integration cases in `checkout_sessions.rs` | true (`grep -c '#[tokio::test]'` = 31) |
| 4 new unit cases (3 `return_trip`, 1 `error`) | true |
| `fmt` / `clippy -D warnings` clean | **re-measured: clean** |
| `465 passed, 0 skipped` on `-p vpay-db -p vpay-api -p vpay-worker -p vpay-core` | **re-measured: 465 passed, 0 skipped** |
| `92 passed, 0 skipped` on the five integration binaries, no retries | **re-measured: 92 passed, 0 skipped, no retries consumed** |
| an intent's session can only be its own tenant's | true, and it is the schema plus one call site: `checkout_sessions.payment_intent_id` is a real FK, and `v1::checkout_sessions::create` resolves the intent through `PaymentIntents::get_for_merchant` (`v1/checkout_sessions.rs:366`) before stamping `merchant_id`. Since `confirm_once` has already resolved the intent under a `MerchantScope`, an unscoped `find_latest_by_intent` cannot reach another tenant's row |
| "an open session is always the newest" is a property of the schema | true — `seq` is `BIGINT GENERATED ALWAYS AS IDENTITY` and `checkout_sessions_one_open_per_intent` is a partial unique index; nothing in `vpay_db` ever writes `status` back to `open`, so no row can be newer than an open one |
| `complete` is unreachable through the shipping API, and the case says so | true, and the reasoning holds: `settle_for_intent` writes `complete` in the same commit as the intent reaching `succeeded`, and `load_confirmable_intent` refuses a `succeeded` intent one step earlier. The staging is a single `UPDATE` in the test body, named in its own doc comment |
| the re-staged existing case (`a_payer_confirming_between_the_read_and_the_write_keeps_the_session`) is not weaker | **true.** Every assertion is unchanged; only how the window is opened moved. `expire_due`'s `UPDATE` carries `expires_at <= $2`, and `$2` is the sweep's own instant — so `now + 25h` satisfies the horizon predicate exactly as a rewritten past `expires_at` did, leaving the `NOT EXISTS` live-charge guard as the thing that refuses, which is what the case is for. The claim "deleting the `NOT EXISTS` from `expire_due` leaves this the only case that notices" is unchanged by the re-staging |
| neither merchant SDK enumerates server codes, so ADR-0015 is not engaged | true for the *code*: `sdks/nodejs/src/errors.ts` and `sdks/stripe-js/src/errors.ts` both type `code` as an open `string`, `CLIENT_ERROR_CODES` is the closed set the client originates, and `docs/sdks/parity.md:131` already carries the generic envelope row. **Not** true for `sdks/stripe-js/README.md`'s error table — F5 |

## 2. Mutations

Each applied to the tree as delivered, the named suite run at `--retries 0
-j 1 --no-fail-fast`, then `git checkout --` and `git status` confirmed clean.
Baseline for `binary(checkout_sessions)` is 31 passed, 0 skipped.

| # | Mutation | Result |
|---|---|---|
| 1 | delete `verdict(&session, …)?` from `SessionGate::admit_confirm` | **caught** — 26 passed, **5 failed**: cases 1, 2, 3, 4 and 6. Cases **5 and 7 still pass**, which is what makes it a precise break and not a blanket one. Reproduces `opus.md`'s own number exactly |
| 2 | `verdict` consults `status` alone (`OPEN => Ok(())`) | **caught** — exactly case 3 (`a_confirm_past_the_horizon_is_refused_by_the_read_and_writes_nothing`) plus both `v1::return_trip` horizon units. 32/35 pass |
| 3 | move the gate **after** `open_attempt` | **caught, and by the right assertion** — cases 1, 2, 3, 4, 6 fail on `write_footprint`: `left: (1, 1, 1), right: (0, 0, 0)`, "a refused confirm must leave no charge, no provider_requests row and no job". The ordering claim is proven, not asserted |
| 4 | change a code (`checkout_session_expired` → `checkout_session_gone`) | **caught** — 9 tests: the four integration refusals, `a_closed_checkout_session_is_its_own_409_…`, both `error::tests` walkers over `cases()`, and both `return_trip` units |
| 5 | let the merchant `/v1` confirm skip the gate (gate only when `SecretRendering::Include`) | **caught** — case 6 exactly (plus `confirm_rails::a_session_driven_confirm_is_refused_when_the_checkout_app_is_gone`, collateral from the same mutation swallowing `CheckoutNotConfigured` on `/v1`) |
| 6 | drop the intent filter (`WHERE payment_intent_id = $1` → `WHERE $1 IS NOT NULL`) | **caught, but not by any of the seven new cases** — only by the pre-existing `confirm_rails::a_browser_confirm_under_a_session_needs_no_return_url`, which happens to hold two intents in one harness. Recorded as a coverage observation, not a defect: the mutation is caught by the brief's own gate list. See §4 |
| 7 | make the gate read twice (verdict from the first row, return URL from a second read) | **NOT caught** — 57 passed. Expected: this is a race window no deterministic test can observe without injecting a clock or a concurrent sweep. The "one read" property is a code-review property here, not a tested one. Recorded, not fixed |
| 8 | (mine) `PostRequest::finish` releases a `4xx` instead of storing it, so every retry re-executes | **NOT caught** — 31 passed, 0 skipped. This is F2 |

## 3. Findings

| # | Severity | Where | Evidence | Status |
|---|---|---|---|---|
| F1 | robustness (money path) | `backends/crates/vpay-db/src/checkout_sessions.rs:844`; `backends/migrations/0028_create-checkout-sessions.sql` | `find_latest_by_intent` has **no usable index**. 0028's only lookup by intent is *partial* (`WHERE status = 'open'`), which this query cannot use because dropping that predicate is its entire purpose; there is no total index on `payment_intent_id`. Measured on `postgres:16-alpine` with 200,000 sessions, looking up an intent that has none — the **common** confirm: `Parallel Seq Scan … Rows Removed by Filter: 66667 (×3 workers), Execution Time: 11.685 ms`, against `Index Scan … 0.047 ms` with an index. Asked once per confirm, on all of them | **fixed** — `df16c1a` |
| F2 | misleading-claim (test) | `backends/tests/integration/tests/checkout_sessions.rs`, `the_merchant_confirm_is_refused_too_and_the_replay_is_the_stored_409` | Its doc comment said the byte-for-byte equality proves "the response is the stored one". It does not — a re-executed retry decides the same way against the same rows. Mutation 8 proves it: with `4xx` no longer stored, the case still passed | **fixed** — `0565378` |
| F3 | misleading-claim (docs) | `docs/api/README.md:309` and its `POST /v1/payment_intents/{id}/confirm` row | A section headed "Error codes a `/v1` caller can actually receive" that does not list two codes a `/v1` caller can now receive. (`checkout_not_configured` was already missing, from Step 9 lane 1 — folded in and disclosed) | **fixed** — `8a7dc29` |
| F4 | nit (docs) | `docs/reference/vpay-api.md` § the confirm path | `1b.` is not a CommonMark ordered-list marker, so the new step rendered as a paragraph that terminated the list and restarted it at 2 — on the one page whose subject is that the order is the safety property. Step 5 also still claimed the return URL is resolved there, contradicting the sentence two lines below it (pre-existing since lane 1b retired `return_url_for_charge`). Plus a stray double blank line | **fixed** — `e4e7141` |
| F5 | nit (docs) | `sdks/stripe-js/README.md` § Errors | The package that drives the browser confirm does not name `checkout_session_expired` in its error table — the code an integrator most needs a branch for, since it is the difference between "show the abandoned-checkout screen" and "retry". No source change is needed (open `string` code, no closed union), so ADR-0015 is genuinely not engaged; the README is. (`charge_declined` was already missing — folded in and disclosed) | **fixed** — `87ae961` |
| F6 | nit (residual) | `v1/payment_intents.rs`, between `admit_confirm` and `open_attempt` | The gate is a read-then-act: a sweep committing in the window between the verdict and the charge insert still lands a charge on a session that has just expired. It is a **much** smaller window than the defect being fixed (which was unbounded — a payer could pay hours later) and the resulting state is caught downstream by `expire_due`'s `NOT EXISTS` in the other direction, but it is not zero and `opus.md` does not mention it. Closing it properly means the charge insert re-checking the session in the same transaction, which is a design decision about where the invariant lives | **left visible** — see §4 |
| F7 | misleading-claim (docs) | `docs/status.md:1379`, `docs/flows/hosted-checkout.md`, `docs/flows/browser-checkout.md`, `docs/flows/errors.md` | `docs/status.md` still says `checkout_sessions.rs` holds "**17 container-backed cases**"; it holds 31. `errors.md`'s policy table still says `Category::Conflict` has one code; it has three. `browser-checkout.md`'s "authorises exactly one payment intent, **for its whole life**" now has an exception. All four are stale in the repository's own terms (AGENTS.md: a behaviour change updates `docs/status.md` and the flow doc in the same PR) | **left visible, deliberately** — the task brief forbids editing `docs/status.md` and `docs/flows/*`. `opus.md` discloses all four and writes the replacement sentences. Not the implementer's failure; it is the experiment's boundary, and it must not be read as "the implementation is documented" |

Nothing was found under **money**, **secret-leak**, **correctness** or
**rule-break**. Specifically checked and clean:

* no `#[allow]`, `unwrap`, `expect` or `panic` in any of the new production
  code (`error.rs`, `return_trip.rs`, `checkout_sessions.rs`);
* nothing hard-codes a success, and no new function returns a plausible
  value in place of an unimplemented one;
* the refusal renders only the `cs_…` id — not caller text, not a credential
  — through `bounded_message`, and the one new log line
  (`tracing::debug!` in `verdict`) carries the session id and nothing else;
* `verdict`'s fallthrough is `ApiError::Internal` (500, pages), not a guess
  folded into one of the two refusals — a merchant is never told "your payer
  abandoned this" about a row this binary cannot interpret;
* the two new tests-that-assert-nothing candidates are not: the four unit
  cases each assert a `code`, a status and a message, and
  `an_intent_with_no_session_confirms_exactly_as_before` asserts the three
  rows a successful confirm writes rather than only the `200`;
* `write_footprint`'s deployment-wide `jobs` count is safe: every test in
  this file gets its **own** Postgres container (`support::migrated_postgres`
  starts one per call), so there is no cross-test bleed;
* ADR-0006: no mock, fake or in-memory substitute was added; every new case
  runs against a real Postgres and the shared WireMock MTN tree.

## 4. Two things left visible, and why

**F6 — the read-then-act window.** `admit_confirm` decides, and `open_attempt`
writes some microseconds later. A sweep committing in between still produces
the state the change exists to prevent. Fixing it honestly means moving the
session check into the charge-insert transaction (or giving that insert a
`NOT EXISTS`-style guard against a non-`open` session, the shape `expire_due`
already uses in the other direction) — a decision about which layer owns the
invariant, with an ADR-0011 answer to choose for the loser of the race. That
is a maintainer's call, not a reviewer's, and papering it with a second gate
read would make it *worse* (mutation 7 is exactly that shape). Recorded here
so it is not mistaken for an oversight.

**Mutation 6 — intent scoping is proven only incidentally.** Dropping
`WHERE payment_intent_id = $1` is caught, but by a Step 9 case in a different
binary that happens to create two intents under one harness. None of the seven
new cases would notice a gate that read *some* session. This is not a defect —
the brief's gate list does catch it, and the tenancy argument in
`find_latest_by_intent`'s doc is sound (the FK plus `create`'s
`get_for_merchant` make a cross-tenant row unreachable) — but a reader of
`checkout_sessions.rs` alone would over-credit those seven cases. No test was
added: one that only re-proves what `confirm_rails` already proves would be
coverage theatre.

## 5. Final gate

Run after all five remediation commits.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-worker -p vpay-core` | **465 passed, 0 skipped** (unchanged — no unit test was added or removed) |
| `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(browser_checkout) \| binary(confirm_rails) \| binary(payment_intents) \| binary(postgres_smoke)' --retries 2 -j 1` | **93 passed, 0 skipped**, no retries consumed (was 92; +1 for `the_confirm_paths_session_lookup_is_served_by_an_index`) |
| `just verify` | ok — `verify-no-mocks` ok; `verify-status` ok, 1 unimplemented item, declared; `verify-errors` ok, **15 error types, all classified**; `verify-sdk-parity` ok, 342 proving tests named all exist, 26 dated gaps |
| `just verify-ignored` | **1159 total, 42 test binaries, 0 ignored** (was 1158/42/0; the floor stays at 1080) |
| `just test-doc` | **86 passed, 1 ignored** — unchanged; the ignored one is still `sdks/rust`'s README block |

No flakes. No container-start timeout was observed and no retry was consumed
in any run recorded here, baseline or mutation.

## 6. The five remediation commits

| Commit | Finding | Guard-failure proof |
|---|---|---|
| `df16c1a` `fix(db): index the confirm path's checkout-session lookup (migration 0030)` | F1 | with `backends/migrations/0030_*.sql` removed, `binary(postgres_smoke)` reports **13 passed, 2 failed** — `the_confirm_paths_session_lookup_is_served_by_an_index` (the plan falls back to `Index Scan Backward using checkout_sessions_seq_key` with `payment_intent_id` demoted to a filter) and `schema_migrates_cleanly_on_an_empty_database` (29, not 30). Restored: 15 passed, 0 skipped |
| `0565378` `test(checkout): make the merchant confirm's replay case actually prove a replay` | F2 | with mutation 8 re-applied (`PostRequest::finish` releases a `4xx`), the case now fails on `left: 200, right: 409`. Before this commit the same mutation left it green |
| `8a7dc29` `docs(api): list the two checkout-session codes a /v1 caller can now receive` | F3 | docs only |
| `e4e7141` `docs(api): fix the confirm-path list the session gate was inserted into` | F4 | docs only |
| `87ae961` `docs(sdk): name the checkout-session refusal in @vpay/stripe-js's error table` | F5 | docs only |

No test was weakened. `0565378` adds one step and one assertion to an
existing case and keeps every assertion it already had; nothing else in the
suite was touched.

## 7. What this review did not check

* **No frontend, TypeScript, Helm, Cypress or conformance suite was run.**
  The two SDK changes are README-only, so neither SDK suite was re-run;
  `just ci` as a whole was not run, only the Rust half of it, command by
  command.
* **No real rail.** Every assertion here settled against a
  `wiremock/wiremock` container, as everything in this repository does.
* **Nothing was checked in a browser.** No Cypress spec drives a confirm on a
  dead session, and none was added.
* **Concurrency was not exercised.** Mutation 7 and F6 are both about a race,
  and neither was raced — the window was reasoned about, not measured.
* **The 200,000-row measurement behind F1 was taken in a throwaway container**
  with the `payment_intents` foreign key dropped so the rows could be
  generated cheaply. That affects the insert, not the plan; the plan was
  re-confirmed on an ordinary migrated database, and the shipping guard
  (`postgres_smoke::the_confirm_paths_session_lookup_is_served_by_an_index`)
  asserts the plan rather than the timing.
* **`docs/status.md` and `docs/flows/*` were not read for anything beyond
  F7.** They are outside the brief's editable set, so their other claims were
  not audited.
