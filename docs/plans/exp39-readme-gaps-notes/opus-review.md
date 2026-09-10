# exp39 — the sabotage review of the three honesty gaps (issue #87)

Reviewer tier `opus`, 2026-09-10, branch `claude/exp39-readme-gaps`, base
`ff1f507`. The draft this reviews is the six haiku commits `f2b35a3`
… `79b3594`, squashed away by this pass; §6 says why and what was in them.

**The draft passed `just ci` as delivered — exit 0, 1626 tests, twelve gates
green — and three of its claims were false.** That is the finding worth
carrying out of this sample: the gate this repository runs cannot check
whether a sentence in `docs/status.md` is true, and the draft's two new tests
passed while asserting the wrong thing with an `||`.

---

## 1. Findings

| # | Severity | What the draft delivered | What is true | Fixed in |
|---|---|---|---|---|
| F1 | correctness | `StartupError::MissingDatabaseUrl`'s message reads `--database-url / VPAY_DATABASE_URL is required`, and the test accepts it | There is no `VPAY_DATABASE_URL`. `CommonArgs::database_url` declares `env = "DATABASE_URL"` (`backends/crates/vpay-config/src/cli.rs`), pinned by `COMMON_ENV_VARS`. **Measured:** exporting `VPAY_DATABASE_URL` and running the binary gives the identical exit `78` and the identical message back — the message sends an operator to set a variable nothing reads, during a deploy that is already down | C1 |
| F2 | gate-hole | Both new cases spawn with the runner's inherited environment | `std::process::Command` inherits it, `.env.example` ships `DATABASE_URL=postgres://vpay:vpay@localhost:5432/vpay`, and sqlx tooling exports it. **Measured:** with that exact value in the environment, both cases fail with `69`. `tests/cli.rs` already had the answer — `staff_add_refuses_an_unregistered_merchant_before_it_opens_the_database` calls `.env_remove("DATABASE_URL")` for this reason — so this is a departure from a practised convention in the same file | C1 |
| F3 | gate-hole | `stderr.contains("--database-url") \|\| stderr.contains("VPAY_DATABASE_URL")` | An `||` passes on the flag alone, which is how F1 survived a green run. The file's own convention is `&&`: `worker::a_zero_worker_concurrency_…` asserts "the refusal must name **both** spellings of the knob to turn". The cases now assert `&&` on the two real spellings and assert the *absence* of `VPAY_DATABASE_URL` | C1 |
| F4 | misleading-claim | The worker case was inserted between `a_valid_config_lets_the_worker_boot`'s doc comment and its `#[test]` | The positive test was left undocumented and the new one inherited a doc describing something else ("The positive counterpart: a config that passes validation lets this mode boot all the way to its startup log line…"). Both were restored; the new case sits beside the `69` case, mirroring the parent module's order, with its own doc | C1 |
| F5 | nit | `.ok_or(StartupError::MissingDatabaseUrl).map_err(\|e\| anyhow::anyhow!(e))?` | The house idiom is the bare `?` — `signing_key`'s own site three functions below is `.ok_or(StartupError::MissingSigningKeyFile)?`, and `From<StartupError> for anyhow::Error` does the rest | C1 |
| F6 | misleading-claim | `StartupError`'s doc still says "both classify identically" and `Classify`'s says "Both variants" | Three variants now. `MissingDatabaseUrl` is also the first one **both** modes raise, which is the reason there are two subprocess cases rather than one — stated, because it is what a future reader needs to not delete one of them | C1 |
| F7 | misleading-claim | Four documents still said a missing `--database-url` exits `1`, and the draft touched none of them | `README.md` ("**Two of the three exit `78` and one does not**"), `docs/status.md`'s CLI row ("a pre-existing gap this pass looked at and left alone"), `docs/flows/configuration.md` ("a pre-existing gap this ordering does not fix"), `docs/flows/deployment.md`'s table ("non-zero exit at boot"). A fifth, `docs/reference/vpay-config.md`, asserted the *opposite* and was aspirational rather than wrong; it is true now and says since when | C2 |
| F8 | misleading-claim | `docs/status.md`'s CLI row still said no test spawns the binary without `DATABASE_URL` | The "Database connectivity" row's unproven claim (1) is exactly what the two new cases prove. Struck with the date and the two test names | C2 |
| F9 | correctness | The `schemas/*.cstack` row rewritten to "**nine** models: `AuthorizationCode`, `CheckoutSession`, `ConfigReconcile`, `Customer`, `DisabledClient`, `Invoice`, `Staff`, `StaffSession`, `WebhookDelivery`" | Three of the nine names are not models in the file (`AuthorizationCode` → `OauthAuthorizationCode`, `Staff` → `StaffMember`, and **`ConfigReconcile` is a `vpay-db` module, not a model at all**), and the count is wrong in both halves. Measured: **twelve of seventeen**, thirty-two statements, twelve tables — §2 | C3 |
| F10 | correctness | — (not touched by the draft) | `README.md` carried the same claim one revision older still — "**Nine of the file's thirteen** models" — with `checkout_sessions`, `invoices` and `invoice_items` missing and `refunds` absent from the four that do not | C3 |
| F11 | correctness | — (not touched by the draft) | `docs/reference/vpay-db.md` § "What runs through it today", which the draft's row points at as the authority, lists **sixteen** statements over nine tables: it omits `staff_members`, `staff_sessions` and `oauth_authorization_codes` — sixteen statements, half the total, the three tables its own prose calls wholly generated — carries `get_for_merchant` twice (once without its `run(ctx)`), and has a blank line mid-table that ends the table, so the five `checkout_sessions` rows render as a second table whose header is one of the rows | C3 |
| F12 | misleading-claim | — (not touched by the draft) | The same file's § CrateStack opens with two contradictory sentences one after the other — "runs **twenty-two** queries … over ten" and "**twenty-four** queries … over nine" — and repeats an increment paragraph twice. A conflict resolved by keeping both halves. Neither number was right | C3 |
| F13 | misleading-claim | — (not touched by the draft) | `docs/status.md` § CrateStack still says the file "models only `Currency`, `Provider`, `PaymentIntent`, `Charge`, and the new `LedgerTransaction`/`LedgerEntry` pair" and "deliberately omits … webhooks/outbox". The outbox is two models and has been since 2026-09-06 | C3 |
| F14 | correctness | `Steering::Msisdn`'s doc lists `237600000ce0`, `f01`, `f02` **and `237600000100`** as steering codes | `…100` is not a `Steering` value: `OUTCOMES` sends exactly `ce0`, `f01`, `f02`. It appears in `ACCOUNT_HOLDER_CASES` and nowhere else | C4 |
| F15 | correctness | — (the draft left `ACCOUNT_HOLDER_CASES` alone; it is half of gap (3) in the brief) | Its doc says "`237600000100` is the same number step 4's *settling* MTN outcome uses". It is not: step 4 sends `237600000ce0`. `…100` is that number's digits-only **twin** — `requesttopay-scenario.json` matches `237600000(ce0\|100)` into one `mtn-e2e-poll` scenario — so it walks the identical `PENDING` → `SUCCESSFUL` path but is not the number the program pays with | C4 |
| F16 | nit | Six commits including "remove backup file", which removes a 1319-line `main.rs.bak` an earlier commit in the same series added | Squashed away, so the blob is not in the branch's history at all. Working tree and `git ls-files` are clean of it (`git ls-files \| grep -iE '\.(bak\|orig\|rej\|tmp)$'` → empty) | §6 |

**Not a finding, recorded because it is the near miss:** the draft's two cases
*do* carry `#[test]`, *are* collected by nextest, and *did* run in the draft's
own `just ci` (`vpay-server::cli a_missing_database_url_is_exit_78_naming_the_problem`
at 1280/1626 and its `worker::` twin at 1302/1626). The brief's suspicion that
one had been committed without the attribute was correct about the history —
`c846455` is titled "restore missing `#[test]` attribute on worker test" — and
wrong about the final file.

---

## 2. The model count, measured

**The rule**, so the number is re-derivable rather than trusted: every
`find_unique` / `find_many` / `create` / `upsert` / `update_many` /
`delete_many` chain in `backends/crates/vpay-db/src/*.rs` outside a
`#[cfg(test)]` module, each terminating in exactly one `run(ctx)` or
`run_in_tx(tx, ctx)`, attributed to the model its accessor names.
`migrations.rs`'s `.run(&self.pool)` is `sqlx::migrate!`'s and is excluded.

**Thirty-two statements, twelve tables, twelve of the file's seventeen
models.**

| Model | Table | Statements | Where |
|---|---|---|---|
| `DisabledClient` | `disabled_clients` | 3 | `disabled_clients.rs` |
| `Currency` | `currencies` | 2 | `config_reconcile.rs` |
| `Provider` | `providers` | 1 | `config_reconcile.rs` |
| `WebhookDelivery` | `webhook_deliveries` | 1 | `webhook_deliveries.rs` |
| `Event` | `events` | 1 | `webhook_deliveries.rs` |
| `Customer` | `customers` | 2 | `customers.rs` |
| `Invoice` | `invoices` | 1 | `invoices.rs` |
| `InvoiceItem` | `invoice_items` | 1 | `invoices.rs` |
| `CheckoutSession` | `checkout_sessions` | 4 | `checkout_sessions.rs` |
| `StaffMember` | `staff_members` | 7 | `staff.rs` |
| `StaffSession` | `staff_sessions` | 6 | `staff_sessions.rs` |
| `OauthAuthorizationCode` | `oauth_authorization_codes` | 3 | `authorization_codes.rs` |
| **`PaymentIntent`** | — | **0** | — |
| **`Charge`** | — | **0** | — |
| **`Refund`** | — | **0** | — |
| **`LedgerTransaction`** | — | **0** | — |
| **`LedgerEntry`** | — | **0** | — |

**It agrees with the schema in both directions, which is the check worth
having**: counting `@@allow` arms in `schemas/vpay.cstack` (excluding the ones
*discussed* in comments — a naive count reads `model DisabledClient`'s long
header comment as three arms on `LedgerEntry`) gives a non-zero count for
exactly those twelve models and zero for exactly those five. So the file
grants no permission that has no caller, and no caller runs against a model
with no permission — the two failure modes `disabled_clients.rs`'s
`every_action_this_crate_calls_has_an_allow_arm` and `schema.rs`'s
`the_three_money_models_answer_no_rows_to_every_action` exist to catch, checked
once here across the whole file.

**One gap named rather than closed.** `schema.rs`'s guard pins the empty
`@@allow` slots of `PaymentIntent`, `Charge` and `Refund` — "an arm added here
would be a standing permission with no caller asking for it, and nothing else
in `just ci` would say a word". `LedgerTransaction` and `LedgerEntry` are in
the identical position and are in no such loop. Widening it is a two-line edit
to a `vpay-db` test; this pass did not open `vpay-db`'s tests and says so in
`docs/status.md`'s row rather than leaving it to be found.

**Where the draft's three bad names came from.** `ConfigReconcile` and `Staff`
are `vpay-db` *module* and *trait* names (`config_reconcile.rs`,
`vpay_db::Staff`); `AuthorizationCode` is the row type. Reading the module
list and calling it the model list is exactly the shortcut the file's own
prose warns about — 0.11.1 derives a table name from the *model* name, which
is why `model Staff` would have read `staffs` and the model is `StaffMember`.

---

## 3. Mutations

Every one run against the reviewed tree, `cargo nextest run -p vpay-server
--test cli -E 'test(a_missing_database_url)'`, with the environment's own
`DATABASE_URL` removed unless the row says otherwise.

| # | Mutation | serve case | worker case | Says |
|---|---|---|---|---|
| M0 | none | PASS | PASS | baseline |
| M1 | `main.rs`'s site back to `.context(..)` | **FAIL** | PASS | the serve site is pinned, and only by its own case |
| M2 | `worker.rs`'s site back to `.context(..)` | PASS | **FAIL** | so is the worker site; one call site cannot ride on the other's test |
| M3 | the draft's `VPAY_DATABASE_URL` back in the message | **FAIL** | (not reached) | the message's env-var name is pinned, which under the draft's `\|\|` it was not |
| M4 | drop `env_remove` from both cases, run with `DATABASE_URL=postgres://vpay:vpay@localhost:5432/vpay` (`.env.example`'s own value) | **FAIL** (69) | **FAIL** (69) | the removal is load-bearing, not defensive |

**And by hand, on the shipping binary rather than through a test**
(`target/debug/vpay-server`, the fixture config, a generated key):

```text
serve,  no DATABASE_URL   -> exit 78, "vpay-server: --database-url / DATABASE_URL is required: ..."
worker, no DATABASE_URL   -> exit 78, same message
serve,  DATABASE_URL set to an unroutable host -> exit 69   (past the configuration check)
```

The same three, run against the draft before any fix, gave `78` / `78` with
the message naming `VPAY_DATABASE_URL` — **and exporting `VPAY_DATABASE_URL`
changed nothing**, which is the whole of F1 in one command.

---

## 4. `just ci`

**`just ci` exit 0 on the review head**, exit code read from a file, recipe by
recipe:

| Recipe | Result |
|---|---|
| `fmt-check` | ok — `cargo fmt --all -- --check`, stable, `max_width = 100` |
| `clippy` | ok, `-D warnings` |
| `verify` | the twelve gates. `verify-no-mocks`; `verify-status` 1 unimplemented item; `verify-errors` 18 error types / 16 `#[from]` variants; `verify-sdk-parity` 448 proving tests, 35 dated gaps, 32 methods over 36 rows; `verify-links` **1031 links in 192 tracked files**; `verify-npm-scope`; `check-schema` cratestack 0.12.0, **25 declarations** — seventeen models and eight enums, against a floor of 15; `verify-serde` 83 types / 16 exemptions; `verify-repositories` 4 implementations named by none of 82 files; `verify-toolchain` 1.98.0; `verify-ui`; `verify-migrations` 37 files. `verify-docs` printed its report and failed nothing, as designed |
| `test-rust` | **1626 run, 1626 passed, 0 skipped** across **45** binaries, against a real Postgres and real WireMock rails |
| `test-doc` | **109 passed, 1 ignored** — the ignored one is `sdks/rust`'s README block, pre-existing |
| `verify-ignored` | **0 ignored (expected 0), 45 binaries (expected 45), 1626 total (floor 1080)** |
| `lint-web` | ok |
| `test-web` | ok, 0 skipped — `@vpay/checkout` 507 in 24 files, `@vpay/dashboard` 150 in 20, `@vpay-examples/shop` 102 in 12, `@vpay/ui` 74 in 18, `@vaam-apps/vpay-sdk` 207 in 9, `@vaam-apps/vpay-stripe-js` 146 in 9, `@vpay/api-client` 4 in 1, `@vpay/tokens` 8 in 1, `@vpay/config` 63 in 1 |
| `deny` | advisories, bans, licenses, sources all ok |

**1626 is master's 1624 plus this change's two, and master's number was
measured rather than subtracted:** `ff1f507`'s own `tests/cli.rs` was checked
out over this branch's, `cargo nextest list --workspace` answered **1624
across 45 binaries**, and the file was put back.

**One run of the four failed, and it was the host, not the change.** The
third full run on this head died at
`worker_e2e::a_decline_after_submission_returns_the_intent_to_requires_payment_method`
with `failed to create a container: Timeout error` after 120 s. Diagnosed
rather than retried: `docker ps -a` held **80 containers stuck in `created`**
(never started) plus 39 exited, all `postgres:16-alpine` and
`wiremock/wiremock:3.9.2` with no compose project — testcontainers leftovers
this review's own earlier runs abandoned, starving the daemon's create path.
Removing every non-compose container older than thirty minutes (83 of them;
nothing `running`, nothing belonging to a compose project, so the user's
`vpay-demo` stack was untouched) made `docker run postgres:16-alpine` answer
in 0.16 s, and the same test then passed in **3.085 s** on the identical
commit. Recorded because a green re-run after a red one is worth nothing
without the reason for the red.

**The draft passed this same gate as delivered** — `just ci` exit 0 on
`79b3594`, 1626 tests in 1936 s, twelve gates, `verify-links` 1032. That is
the point of §1: every false claim in the table above survived a green run,
because nothing in `just ci` reads a sentence. The two numbers that move here
are `verify-links` (−2 for a `docs/status.md` link the README no longer needs
and a duplicated ADR-0017 link that F12 unpicks, +1 for the link to this file)
and the tracked-markdown count (+1, this file).

---

## 5. Not checked, and not done

- **`just demo-walk` was not run.** It needs the eight-service compose stack,
  and the user's own `vpay-demo` project was up on this host throughout. The
  claim that it is unaffected rests on the change being **comment-only**,
  which is proven rather than asserted: `git diff ff1f507 --
  examples/merchant-demo/src/main.rs` has no added or removed line that is not
  a `///` line. `cargo build -p merchant-demo` was run.
- **`just test-e2e` was not run**, for the same reason and the one exp31 gave.
- **`cargo xtask verify-citations` was not run** — it needs the network and a
  token, and is not part of `just verify` or `just ci`.
- **`vpay-db`'s tests were not opened.** The `LedgerTransaction`/`LedgerEntry`
  guard gap in §2 is documented, not fixed.
- **The `.cstack` → migrations drift was not re-measured.** Nothing in this
  pass changes `schemas/vpay.cstack` or any migration, and
  `postgres_smoke.rs`'s pinned counts are unchanged and green.
- **Nothing was pushed**, per the brief.

---

## 6. The draft's history, and why it is not in the branch

Six commits, squashed into the four this branch now carries. Two reasons, both
mechanical:

1. `f2b35a3` added `backends/apps/vpay-server/src/main.rs.bak`, 1319 lines,
   and `82929c9` deleted it. Keeping either means keeping the blob.
2. `dd31dbd` ("fix rustfmt violations") and `c846455` ("restore missing
   `#[test]` attribute") repair the two commits above them; `79b3594` fixes a
   link the same series broke. None of the five is a step a reader of this
   history would want to walk.

The content the draft got right is in the branch and is not re-attributed to
this review: the `StartupError` variant and its placement, both call sites,
the two subprocess cases' shape, the decision to strike rather than delete in
`docs/status.md`, and the correct `docs/reference/vpay-db.md` link fix. What
the review changed is §1.
