# exp16 — the first CrateStack writes (`disable_client` / `enable_client`)

Transcripts for `docs/status.md` § "The first CrateStack writes (2026-09-06)"
and [docs/reference/vpay-db.md § CrateStack](../../reference/vpay-db.md#cratestack).
Host: `selast-home-pc`, Docker 29.7.2 (rootless,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`), `postgres:16-alpine` from
the local cache, toolchain 1.98.0 as pinned. Base: `925fa72`.

Everything below was executed. Nothing here is designed-and-not-run; where a
thing was *not* measured, § 6 says so.

## 1. What the grammar calls the write actions

Measured by reading `cratestack-macros 0.11.1`, not guessed from prose. The
call sites in `src/model/descriptor.rs` name the action strings the policy
generator matches:

```text
$ grep -n 'generate_policies_for_action' cratestack-macros-0.11.1/src/model/descriptor.rs
45:  generate_policies_for_actions(model, …, &["list", "read"])
47:  generate_policies_for_actions(model, …, &["detail", "read"])
49:  generate_policies_for_action (model, …, "create")
53:  generate_policies_for_action (model, …, "update")
57:  generate_policies_for_action (model, …, "delete")
```

and `src/policy/model.rs::parse_policy_expression` accepts `"all"` as a
wildcard (`if rule_action != "all" && !actions.contains(&rule_action) { return
None }`). So the vocabulary is `list | detail | read | create | update |
delete | all`, a delete **does** need its own arm, and `@@allow("all", …)`
would have worked as a single line.

Four explicit arms were written instead. `"all"` would also grant `list` and
`detail`, which nothing in vpay calls, and it would collapse the four policy
mutations in § 3 into one — the per-action asymmetry in § 4 is the most
useful thing this change measured and a wildcard would have hidden it.

## 2. Why `enable_client` is `delete_many` and not `delete`

`cratestack-sqlx-0.11.1/src/query/write/delete_exec.rs`, the tail of
`delete_returning_record`:

```rust
match outcome {
    Some(record) => Ok(record),
    None => {
        // versioned models get an If-Match re-probe here; DisabledClient
        // has no @version, so this branch is skipped
        Err(CratestackError::Forbidden(
            "delete policy denied this operation".to_owned(),
        ))
    }
}
```

A `DELETE … WHERE pk = $1 AND (<policy>) RETURNING …` that matched no row is
reported as `Forbidden`, and with no version column there is nothing to
disambiguate it with. `DisabledClients::enable_client` documents "a no-op, not
an error, if `client_id` was not disabled to begin with", and two tests assert
it (`disabled_client_lookup_reflects_disable_and_enable`,
`find_client_reflects_the_disabled_clients_kill_switch`), so `.delete()` was
not usable without changing the trait's contract — which the task explicitly
told me to keep.

`delete_many` (`delete_many_exec.rs`) fetches all matching rows and returns
`BatchSummary { total: removed.len(), .. }`, so zero rows is `Ok`. Its policy
clause is part of the `WHERE` (`push_action_policy_query`), which is the price
— see § 4.

`batch_delete` was considered and rejected: it collapses "policy denied",
"tombstoned" and "never existed" into a per-item `NotFound` (its own comment
says so) and opens a transaction unconditionally, so it buys nothing over
`delete_many` here.

## 3. The rendered SQL

Probed with `preview_sql()` against a `connect_lazy` pool (no I/O), inside
`vpay-db`:

```text
UPSERT:      INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2)
             ON CONFLICT (client_id) DO UPDATE SET reason = EXCLUDED.reason
             RETURNING client_id AS "client_id", disabled_at AS "disabled_at", reason AS "reason"
DELETE:      DELETE FROM disabled_clients WHERE client_id = $1
             RETURNING client_id AS "client_id", disabled_at AS "disabled_at", reason AS "reason"
DELETE_MANY: DELETE FROM disabled_clients WHERE <filters> AND <delete_policy>
             RETURNING client_id AS "client_id", disabled_at AS "disabled_at", reason AS "reason"
```

The upsert is the hand-written statement it replaced, byte for byte, plus the
`RETURNING`. `disabled_at` appears in neither the insert column list nor the
`DO UPDATE SET` list, because `model/inputs.rs::create_input_fields` and
`model/descriptor/columns.rs`'s `upsert_update_columns` apply the same
`!is_generated_on_create` filter and `@default(now())` satisfies it. That is
what keeps "a second disable leaves the original `disabled_at` untouched"
true. `delete_many`'s preview is a placeholder string, which is why the two
unit tests assert the upsert's rendering and not the delete's.

## 4. The six mutations

Each: apply, run against a real Postgres, restore. Harness at
`scratchpad/exp16-opus-mut.sh` (it refuses to run off the branch).

| # | Mutation | Result |
|---|---|---|
| 1 | drop `@@allow("create", auth().isSystem())` | `a_client_disabled_through_cratestack_is_visible_to_both_paths` **FAILS** at the first `disable_client`: `DisabledClient: a model policy denied a system upsert: forbidden: create policy denied this upsert` |
| 2 | drop `@@allow("update", …)` | the same test **FAILS at the second** `disable_client`, not the first: `… forbidden: update policy denied this upsert`. `disabled_client_lookup_reflects_disable_and_enable` also fails; the read parity test **passes** |
| 3 | drop `@@allow("delete", …)` | the same test **FAILS** at the enable assertion — `enable_client` returned `Ok` and the row is still there. `disabled_client_lookup_reflects_disable_and_enable` fails too; the read parity test passes |
| 4 | drop `@@allow("read", …)` | `a_disabled_client_reads_the_same_through_both_paths` **FAILS**: `CrateStack says false, sqlx says true`. Re-run after the seed change of § 5, which is the point of running it again |
| 5 | replace the `upsert(..)` chain with `Ok(())` | `vpay-tests-integration::client_store find_client_reflects_the_disabled_clients_kill_switch` **FAILS**: "a disabled client must stop resolving immediately, with no restart and no config change" |
| 6 | replace `delete_many(..)` with `update_many(..).set(reason)` | the enable assertion **FAILS** on the row still being there |

A seventh was run and is recorded because its answer was *not* the expected
one: removing `@default(now())` from `disabled_at` does not reach the render
unit test at all — it also adds `disabled_at` to `CreateDisabledClientInput`,
and `disable_client`'s struct literal then fails to compile with
`error[E0063]: missing field 'disabled_at'`. The two filters are the same
predicate, so the insert list and the `SET` list cannot diverge on a schema
edit. The render tests' remaining value is as a guard on a **pinned external
crate's** output across a version bump, and their doc comment now says that
rather than claiming more.

### The asymmetry this measured

`create` and `update` fail **loudly**; `read` and `delete` fail **silently**.
`upsert_exec.rs` calls `evaluate_create_policies` in Rust before building any
SQL and returns `Forbidden` when the allow list is empty;
`upsert_resolve.rs::gate_update_policy` does the same for the conflict branch.
The read and `delete_many` paths instead render the policy into the `WHERE`,
where `push_allow_policy_query` emits `FALSE` for an empty list — a refused
row is then indistinguishable from an absent one.

Directions differ too: a missing `read` policy leaves every client
**admitted** (dangerous), a missing `delete` policy leaves a client
**revoked** (safe). Neither is caught by `just check-schema`, `cargo build`,
`just clippy` or any of the ten `just verify` gates, re-measured for the three
new arms.

## 5. Why the read parity test's seed changed

`a_disabled_client_reads_the_same_through_both_paths` seeded its row by
calling `disable_client`, and its own doc comment said that was the point:
"the sqlx write that deliberately did NOT move to CrateStack in this change —
so this asserts the two layers see one table, not that one layer is
self-consistent". Once `disable_client` became an `upsert`, keeping the call
would have made the test exactly what that sentence disclaimed. It seeds with
an inline `INSERT` and removes with an inline `DELETE` now, deliberately not
factored into a helper shared with the new write test — they exist to be
*unlike* the code under test. Mutation 4 confirms it still catches a missing
read policy afterwards.

## 6. What was not measured

- **The transaction seam.** No CrateStack `run_in_tx` is called anywhere in
  vpay. Neither of these writes has anything to be atomic with, so both use
  `.run(ctx)` and let CrateStack own its own transaction. Whether vpay's
  `UnitOfWork` and CrateStack's `run_in_tx` actually compose is still read out
  of the type signatures rather than executed.
- **The audit and event paths.** `model DisabledClient` has no `@@audit` and
  no `@@subscribe`, so `descriptor.audit_enabled` and `emits(..)` are both
  false and neither write calls `ensure_audit_table` or
  `ensure_event_outbox_table`. Read from the source; not exercised.
- **`PersistenceError::Unique` / `ForeignKey` / `Check` through a CrateStack
  write.** `disabled_clients` has no foreign key and no CHECK, and its only
  unique constraint is the primary key the upsert exists to absorb — so no
  real SQLSTATE reaches `classify_cratestack` from these two calls. Those arms
  are still unit-tested only.
- **The concurrent upsert path.** `upsert_resolve.rs`'s lost-race recovery
  (cratestack#745) is not exercised; nothing in vpay disables the same client
  from two connections at once.
- **The release image.** Not rebuilt. This change adds no dependency and no
  file the Dockerfile would have to copy, so the `COPY schemas` fix from the
  read change still covers it — read, not measured.
- **`just fmt`'s prettier half.** Fails on this host with
  `ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL Command "prettier" not found`; no node
  modules are installed in this worktree. `just fmt-check` (the Rust gate, and
  the one `just ci` runs) passes. No markdown was reformatted.
