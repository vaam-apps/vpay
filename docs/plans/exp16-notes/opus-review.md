# exp16 — sabotage review of the first CrateStack writes

Adversarial review of `b42e3f2` + `539407d` on
`claude/exp16-cratestack-writes-opus` (base `925fa72`), run on 2026-09-06 on
the same host the implementation was measured on: Docker 29.7.2 rootless
(`DOCKER_HOST=unix:///run/user/1000/docker.sock`), `postgres:16-alpine` from
the local cache, toolchain 1.98.0 as pinned, Node 22.23.2 as `.nvmrc` pins.

The implementer's account is [opus.md](opus.md). This file records what was
**re-measured** rather than re-read: every claim below was either reproduced
from the vendored 0.11.1 sources or run against a real Postgres by this
review, not taken from that document.

## 1. `just ci`, recipe by recipe

One end-to-end run, exit 0, after `pnpm install --frozen-lockfile` under Node
22.23.2 (`CYPRESS_INSTALL_BINARY=0`).

| Recipe | Result |
|---|---|
| `fmt-check` | ok |
| `clippy` (`--workspace --all-targets -D warnings`) | ok, no warnings |
| `verify` | ten gates ok; `verify-docs` printed its advisory report |
| `test-rust` | **1372 tests run, 1372 passed, 0 skipped** across 43 binaries, 700.731 s |
| `test-doc` | **96 passed, 1 ignored** across 14 doctest targets |
| `verify-ignored` | 0 ignored (expected 0), 43 binaries (expected 43), 1372 total |
| `lint-web` | ok (`pnpm -r typecheck`, `pnpm -r lint`) |
| `test-web` | ok — 63 + 3 + 119 + 180 + 4 + 3 + 57 + 302 vitest cases |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` |

`cargo build --workspace --all-targets` also exits 0 on its own. Every gate
number the implementer claimed is reproduced exactly, including the two counts
easiest to fudge: 1372 total and 96 doctests.

## 2. The framework claims, checked against 0.11.1

All paths below are under
`~/.cargo/registry/src/index.crates.io-*/cratestack-{macros,sqlx,sql}-0.11.1/`.

**The action vocabulary — confirmed, with one correction.**
`cratestack-macros/src/model/descriptor.rs:45-57` names the slots:

```text
45  generate_policies_for_actions(model, …, &["list", "read"])   -> read_allow_policies
47  generate_policies_for_actions(model, …, &["detail", "read"]) -> detail_allow_policies
49  generate_policies_for_action (model, …, "create")
53  generate_policies_for_action (model, …, "update")
57  generate_policies_for_action (model, …, "delete")
```

and `policy/model.rs:132` accepts `"all"` as a wildcard. So the vocabulary is
`list | detail | read | create | update | delete | all`, and a delete does
need its own arm — as claimed.

What is **not** true is the argument the change gave for preferring four arms
over one `@@allow("all", …)`: that `"all"` "would also grant `list` and
`detail`, which nothing in vpay calls". `"read"` is a member of *both* action
lists above, so `@@allow("read", auth().isSystem())` already populates
`read_allow_policies` **and** `detail_allow_policies`. The four arms and
`@@allow("all", …)` grant an identical policy set; the only real difference is
that four arms give four independently-droppable mutations. Finding F1.

**`delete(pk)` reports a missed row as `Forbidden` — confirmed.**
`cratestack-sqlx/src/query/write/delete_exec.rs`, tail of
`delete_returning_record`: `fetch_optional` → `None` → the `if_match` re-probe
is skipped when `version_column` is `None` (it is, for this model) →
`Err(CratestackError::Forbidden("delete policy denied this operation"))`. The
deviation to `delete_many` is justified.

**`delete_many` returns `Ok` with `total: 0` — confirmed.**
`delete_many_exec.rs` builds `DELETE … WHERE (<filters>) AND (<delete policy>)
RETURNING …`, `fetch_all`s it and reports `BatchSummary { total:
removed.len(), .. }`. It also refuses an unfiltered call outright
(`CratestackError::Validation`, "refusing table-wide delete"), so the
`.where_(client_id.eq(..))` is load-bearing in a second way the change does
not claim.

**Create loud, update loud-on-conflict-only, read/delete silent — confirmed.**
`upsert_exec.rs` calls `evaluate_create_policies` before it builds any SQL and
`create.rs:23` returns `Ok(false)` for an empty allow list, which becomes
`Forbidden("create policy denied this upsert")`.
`upsert_resolve.rs::gate_update_policy` does the same for the conflict branch
only. `query/support/policy.rs::push_allow_policy_query` pushes the literal
`FALSE` for an empty list, which is why the two `WHERE`-compiled paths (`read`
via `find_unique`, `delete` via `delete_many`) are silent instead.

**`reason` is always in the insert list, `None` or not.**
`cratestack-macros/src/model/inputs.rs:78` emits `sql_values()` as an
unconditional `vec![…]` over `create_input_fields`, so `reason: None` binds a
NULL rather than dropping the column. The trait's "passing no reason clears
the note" is therefore a property of the statement, not of a fallback.

**One thing nobody wrote down: the upsert's update branch needs TWO pooled
connections at once.** `UpsertRecord::run` opens its own transaction on
`runtime.pool()` (connection A) and `gate_update_policy` →
`row_passes_update_policy(runtime.pool(), …)` then `fetch_optional`s on
`policy_pool` — the *same* pool — while A is still held. `vpay-db`'s pool is
`MAX_CONNECTIONS = 10` with a 5 s `ACQUIRE_TIMEOUT` (`pool.rs`), so ten
concurrent *second* disables would each hold A and wait 5 s for B, and all ten
would fail as `PersistenceError::Backend` → `Category::Storage`. Not reachable
today — `disable_client` has no shipping caller (`docs/roadmap.md`) and the
insert branch takes only one connection, because `auth().isSystem()` is not a
relation predicate and `evaluate_create_policies` therefore issues no query.
Recorded because "the cost is a round trip" understates it. Finding F4.

## 3. The mutation table — re-run by this review

Each row: delete the named line from `schemas/vpay.cstack`, run three suites,
restore. Harness `scratchpad/exp16-review-mut.sh` (refuses to run off the
branch). Logs `scratchpad/exp16-review-mut-*.log`.

| Mutation | `-p vpay-db --lib` | `-p vpay-db --test repositories` | `client_store` kill switch |
|---|---|---|---|
| drop `@@allow("read", …)` | **26 passed** | 3 failed: read-parity at `repositories.rs:486`, lookup at `:328`, write-parity | FAIL at `client_store.rs:115` |
| drop `@@allow("create", …)` | **26 passed** | 2 failed: write-parity at the **first** disable, `… forbidden: create policy denied this upsert`; lookup test too. Read-parity **passes** | FAIL, "disabling the client" / same `Forbidden` |
| drop `@@allow("update", …)` | **26 passed** | 2 failed: write-parity at the **second** disable, `… forbidden: update policy denied this upsert`; lookup test too. Read-parity **passes** | **PASSES** — it disables once and never re-disables |
| drop `@@allow("delete", …)` | **26 passed** | 2 failed: write-parity at `repositories.rs:649` (the read-back after enable, `Ok` returned and the row still there); lookup at `:353` | FAIL at `client_store.rs:129`, "re-enabling must restore access" |

Every runtime effect the change claimed is reproduced, including the two
subtle ones: the `update` arm is consulted **only** on the conflict branch (a
test that disabled once and stopped would pass), and the `delete` arm fails
**silently** — `enable_client` returns `Ok` and the row survives, exactly as
the implementation's comment says, which answers the review's question (c) in
the affirmative: the read-back is what makes it red, and nothing else in
`vpay-db --lib`, `cargo build`, `just clippy`, `just check-schema` or the ten
`just verify` gates does.

The first column is the finding. **`cargo nextest run -p vpay-db --lib` passed
26/26 under all four mutations** — the whole no-database half of the gate is
blind to every policy hole, including the two silent ones, and the only thing
that turns red needs Docker. Finding F2.

## 4. Findings

| # | Severity | Finding |
|---|---|---|
| F1 | misleading-claim | "`@@allow("all", …)` would also grant `list` and `detail`" is false — `@@allow("read", …)` already fills both slots (§ 2). Stated in `schemas/vpay.cstack`, `docs/reference/vpay-db.md` and `docs/plans/exp16-notes/opus.md`. |
| F2 | gate-hole | Every policy arm's absence is invisible to the database-free gate (§ 3, first column). The compiled `ModelDescriptor` carries the answer as a `&'static [ReadPolicy]` per slot, so this is checkable in `-p vpay-db --lib` at zero cost, and was not. |
| F3 | nit | `MODEL`'s doc comment still says "the `.cstack` model `is_client_disabled` reads". Three statements carry it now. |
| F4 | correctness (documentation of a real cost) | The upsert's update branch holds two pooled connections at once (§ 2). `docs/reference/vpay-db.md` and `docs/status.md` say only "a round trip". |
| F5 | nit | `docs/status.md`: "Both silent directions are fail-safe or not:" is not a sentence. |
| F6 | nit | `docs/plans/exp16-notes/opus.md` § 6 says "no node modules are installed in this worktree" while the same document claims a green `just ci`, which runs `lint-web` and `test-web` and cannot pass without them. |

Checked and **not** findings:

- The `# Errors` move from `DbError::Query` to `DbError::Persistence` is
  complete: no other document names the old variant for these two methods
  (`docs/reference/vpay-db.md:808` is about `config_reconcile`,
  `docs/flows/configuration.md:319` likewise, and
  `docs/plans/exp14-notes/*` are historical records of the read's own move).
- The read-parity test's new inline-SQL seed is consistent with ADR-0006 and
  AGENTS.md rule 1 — it is a statement against the same real Postgres, not a
  double — and it still proves what it claims: mutation `read` above fails it
  at `repositories.rs:486` *after* the seed change.
- No `unwrap`/`expect`/`panic` outside `#[cfg(test)]`; `clippy` with `-D
  warnings` is the proof, and `clippy.toml`'s test exemption is what allows
  the `expect` in `lazy_cratestack`.
- `system_context()` is built fresh at all three call sites; nothing is
  cached across writes.
- No CrateStack `run_in_tx` is called anywhere in `backends/` — grep confirms
  the only mentions are in prose — and `docs/status.md` and
  `docs/reference/vpay-db.md` both say so.
- The drift constants: `EXPECTED_DRIFT_CHANGES = 85` /
  `EXPECTED_DRIFTED_RELATIONS = 16` / `EXPECTED_UNMAPPABLE_COLUMNS = 18` are
  untouched by this branch and the drift test passes inside the 1372.
  Policy arms are not schema shape, so this is the expected answer rather
  than a lucky one.
- `disable_client` twice with a different reason really does take the conflict
  branch: the `update` mutation fails at the *second* call and not the first,
  which is only expressible if the second call went through
  `gate_update_policy`.
- `CreateDisabledClientInput`'s struct literal is exhaustive by construction
  (no `..Default::default()`), so a new non-defaulted column in
  `model DisabledClient` is `error[E0063]` at this call site rather than a
  silent NULL. That is a *good* property here and a CrateStack ergonomics
  cost everywhere else: the generated input type's shape leaks into every
  caller, so adding a column to a model is a breaking change for every
  literal, in this repository and in any other. Worth an upstream note (§ 6),
  not a finding.

## 5. What this review did not check

- The concurrent upsert path (`upsert_resolve.rs`'s lost-race recovery,
  cratestack#745). F4's connection-pair analysis is read out of the source,
  not executed under contention.
- `cargo xtask verify-citations` / `just docs-check-citations` — needs the
  network and a GitHub token.
- The release image, Cypress, and `just helm-check`: outside `just ci` and
  outside this change.
- Whether `PersistenceError::{Unique, ForeignKey, Check}` are reachable
  through a CrateStack write. They are not, from this table, for the reason
  the implementer gives.

## 6. Worth sending upstream to CrateStack

1. **`.delete(pk)` cannot express "no such row".** `delete_exec.rs` collapses
   a zero-row `DELETE … RETURNING` into `Forbidden`, so an idempotent-delete
   caller has to reach for `delete_many` and lose the policy's loudness in
   exchange. A `DeleteOutcome { Deleted(M), NotFound }`, or the `if_match`
   re-probe generalised to "re-read under the read policy and say which", would
   let a caller have both.
2. **`upsert`'s update branch takes a second connection from the same pool
   while holding its transaction** (`gate_update_policy` →
   `row_passes_update_policy(runtime.pool(), …)`). At `max_connections = N`,
   N concurrent conflict-branch upserts deadlock until the acquire timeout.
   The probe could run on `&mut **tx`, which already holds the row lock.
3. **A model's create/update/delete policy holes are silent in different
   ways**, and nothing in the generated code lets a consumer assert "this
   action has an allow rule" other than reading `ModelDescriptor`'s public
   slots. A generated `descriptor.actions_with_policies()` — or simply
   documenting the slots as the supported way to test this — would save every
   adopter the test F2 adds by hand.
4. **`CreateModelInput` structs are exhaustive literals**, so adding a column
   to a `.cstack` model is a source-breaking change at every call site with no
   `#[non_exhaustive]` escape and no builder default. `generate_builder` emits
   a builder, but the literal is what the docs lead with.

## 7. What this review changed

One commit per finding, on top of `539407d`; each names its proof.
`just ci` was re-run end to end afterwards.
