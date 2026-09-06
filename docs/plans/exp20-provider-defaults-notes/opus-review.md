# exp20 review pass — sabotage notes

Adversarial review of `claude/exp20-provider-defaults` (base `06e27f9`,
delivered head `bc18108`, two commits). Everything below was measured on this
branch on 2026-09-06 against `postgres:16-alpine` under the pinned toolchain,
or is a citation into the tree at the named commit. The implementer's own
transcript is [opus.md](opus.md); this file records only where the review
disagreed with it, or found something it had not looked at.

Verdict: **not safe as delivered.** One gate hole (a mutation the whole suite
survived), one false claim in a production doc comment about a boot exit code
that really did change, two coverage gaps the task brief named explicitly, and
three stale doc statements — one of them naming a database table that has
never existed. Everything found is fixed on this branch; nothing was weakened
to make a check pass.

What the review *confirmed* rather than overturned is worth saying first,
because most of the delivered change is right: the drift arithmetic, the
`for_update` omission, and the backward-compatibility claim in migration 0033
all hold up under measurement.

---

## 1. Confirmed by independent measurement

### 1a. The drift did not move, and the brief's "expect 84 → 79" was wrong

The implementer's § 1 says the brief read the earlier measurement backwards.
Re-measured here rather than accepted:

| Variant | Measured |
|---|---|
| As delivered (no `@default(...)`, no `DEFAULT`) | **84 / 16 relations / 17 unmappable** (`just ci`'s own run of `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`) |
| Migration 0033 deleted, schema edit kept | **89 / 16 / 17**, with exactly the five `providers` lines below |

```text
[safe] column `delivers_callbacks` default value differs from the schema
[safe] column `enabled` default value differs from the schema
[safe] column `requires_ip_allowlist` default value differs from the schema
[safe] column `supports_partial_refunds` default value differs from the schema
[safe] column `supports_refunds` default value differs from the schema
  left: 89, right: 84
```

`schema_migrates_cleanly_on_an_empty_database` also failed at `left: 32, right:
33` under the same mutation. The reasoning — a `DEFAULT` costs drift only when
the two sides disagree, so removing both halves cannot lower the count — is
correct, and `EXPECTED_DRIFT_CHANGES` was right to stay at 84.

### 1b. Migration 0033 is backward compatible with the previous release

Checked by reading `06e27f9`, not assumed. Master's provider pass is
`INSERT INTO providers (code, display_name, flow, supports_refunds,
supports_partial_refunds, delivers_callbacks, requires_ip_allowlist, enabled)
VALUES ($1 … $8)` — all eight columns named — and the only three-column form
anywhere on master is inside the pinning test's `preview_sql` assertion, which
touches no database. `git grep -i "insert into providers"` finds no other
writer in shipping code. So an old binary's boot step 4 against a 0033 database
still works, and 0033's header is right to say so where 0032's header could
not.

### 1c. Dropping `find_unique(...).for_update()` on the provider pass is sound

The argument in the implementer's § 2 holds. There is no read-modify-write to
lose: all eight columns are configuration-owned, and `upsert`'s own conflict
probe takes the row lock inside the same transaction. See § 2c for the one
property this depends on that nobody had written down.

---

## 2. Findings

### 2a. GATE HOLE — `unwrap_or_default()` on the flow parse survived the suite

Severity: **gate-hole**.

`reconcile` parses `ProviderSeed::flow` into the schema's `ProviderFlow` enum.
The comment above that call, `DbError::ProviderFlowUnknown`'s doc,
`docs/status.md`, `docs/reference/vpay-db.md` and `docs/flows/configuration.md`
all state that the `match` is there rather than `unwrap_or_default()` because
`cratestack-macros` marks the first variant of every generated enum
`#[default]` and `ProviderFlow`'s first variant is `push`.

Nothing checked it. **Mutation, measured:**

```rust
let flow = provider.flow.parse().unwrap_or_default();
```

```text
cargo nextest run -p vpay-db
Summary [184.980s] 112 tests run: 112 passed, 0 skipped
```

Under that mutation a rail configured `flow: redirekt` is stored as a **push
rail** and boot step 4 returns `Ok`. That is `AGENTS.md`'s second rule and
`CLAUDE.md`'s "plausible success storing the wrong value" exactly.

The reason it survives is that
`an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement`
constructs `DbError::ProviderFlowUnknown` **by hand** and asserts how it
classifies. It never calls `reconcile`, so it says nothing about the
production path, and its own doc comment ("the parse is upstream of every
statement, which is the whole change this test pins") overstates what it does.

**Fixed** by
`a_flow_label_the_schema_cannot_name_is_refused_by_reconcile_before_any_row_is_written`
(`vpay-db/tests/repositories.rs`), which calls the public trait and asserts the
variant, its `Category::Configuration`/exit-78 classification, and that neither
a `providers` row nor the `currencies` row the pass before it had already
upserted survives. Under the same mutation it now fails in **1.27 s**:

```text
thread '...' panicked at backends/crates/vpay-db/tests/repositories.rs:3141:
a flow that is neither `push` nor `redirect` must be refused: ()
```

### 2b. The `23514` classification DID change — boot exit 69 becomes 1

Severity: **misleading-claim** (with a real, unstated behaviour change behind
it).

`config_reconcile`'s module doc said:

> The *classification* is unchanged — a `23514` is still `Category::Internal`
> … because `persistence::classify_cratestack` and `error::classify_write` are
> asserted against each other

Both halves are false, and this branch is what makes the falsehood matter.

- The two functions are asserted against each other for `23505` and `23503`
  only (`a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx`
  in `persistence.rs`). There is no `23514` case, and there could not be.
- On `23514` they **disagree by design**. `error::classify_write` deliberately
  leaves a CHECK violation in the unclassified `DbError::Query` bucket →
  `Category::Storage` → exit **69**. `persistence::classify_cratestack` gives
  it `PersistenceError::Check` → `Category::Internal` → exit **1**.

`partial_refunds_imply_refunds` is the only `23514` boot step 4 can raise, and
until the provider pass moved it went through `classify_write` — which is
precisely why the delivered change had to edit
`a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
from `DbError::Query` to `Persistence(Check)`. The implementer noticed the
variant and wrote "same constraint name, same `Category::Internal`"; the old
one was `Storage`.

So a boot against an adapter whose declared `Capabilities` are incoherent told
a supervisor "wait for Postgres" (69) before this branch and pages someone (1)
after it. `Capabilities::is_coherent` exists but is called only from
`vpay-server`'s `#[cfg(test)]` assertion and the conformance suite — never at
boot — so this path is reachable.

**Fixed**: the module doc now states what actually moved; a category assertion
was **added** to
`a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
(measured `Internal` / exit 1); `docs/status.md` records the change and
surfaces the maintainer decision (§ 4).

### 2c. Nothing asked what a waiting `reconcile` does with committed state

Severity: **correctness** (evidence gap on the change's own load-bearing
argument).

The task brief asked for it directly, and it is the question the absent
`for_update` raises. `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`
proves the waiting, but releases the lock by **rollback**, so its waiter always
meets an empty table. `two_concurrent_reconciles_…_converge` runs two identical
configurations, so a lost update would be invisible.

**Fixed** by
`a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed`:
the lock is held on one connection, a `providers` row disagreeing with the
waiter's configuration on all eight columns is written and **committed**, and
the waiter's own configuration must be what survives. The answer is that the
provider pass needs no row lock of its own.

It also pins a dependency written down nowhere. **Mutation, measured:** add
`SET TRANSACTION ISOLATION LEVEL REPEATABLE READ` after `reconcile`'s
`pool.begin()` — a plausible "make boot safer" edit.

```text
FAIL [4.225s] a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed
Error: the reconcile itself must succeed against a row that now exists
Caused by: database error: could not serialize access due to concurrent update
```

The snapshot is taken when the `pg_advisory_xact_lock` statement *starts* —
before the holder commits — so the conflict probe cannot see the row it must
update and the INSERT collides with it (`40001`). Every other reconcile case
passes under that mutation, `reconcile_waits_…` and
`two_concurrent_reconciles_…` included. **Boot step 4 requires READ
COMMITTED**, now stated in `config_reconcile.rs`, `docs/status.md` and
`docs/reference/vpay-db.md`.

### 2d. The decisive test never turned a rail back ON

Severity: **nit**, and reported as such rather than inflated.

`a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile` asserts all
eight columns after the disable and after the idempotent repeat — the brief's
"every one of the eight columns" is satisfied — but nothing in the tree moved
`providers.enabled` from `false` back to `true` through `reconcile`, which is
the brief's "and the reverse". A fourth reconcile now does it.

**No single-line mutation was found that this step alone kills**; every one
tried is caught earlier by the first or second assertion. What it pins is a
behaviour `ProviderHost::enabled`'s doc and `docs/flows/configuration.md` both
promise (turning a rail off in the YAML is not the same as deleting its block)
and no test exercised.

### 2e. Migration 0033 was only ever applied to an empty table

Severity: **correctness** (unverified claim in a migration header).

The header's "NO DATA CHANGES … every existing row keeps exactly the
capabilities it had" is the claim an upgrade depends on, and
`a_hand_written_provider_insert_must_now_name_every_capability_column` runs
against a database migrated from empty — the one shape an upgrade never has.

**Fixed** by `migration_0033_changes_no_stored_capability_on_a_populated_table`
(`postgres_smoke.rs`): the five pre-0033 defaults are restored, `pg_attrdef` is
read **before** as well as after so the test cannot go vacuous, one row takes
all five defaults and one contradicts all five, and **0033's own text** is
applied through `include_str!` rather than a copy of its statements.

### 2f. Two stale doc comments in shipping code, and a table that never existed

Severity: **misleading-claim**.

- `vpay_api::v1::boot::flow_label` and the test beside it both said a flow
  label the column rejects is a `DbError::Query` at boot. Since this branch it
  is `DbError::ProviderFlowUnknown` — exit **78**, not **69**. Both were
  telling an operator to wait for a healthy database.
- `docs/flows/provider-port.md` step 3 tells a rail integrator to
  `INSERT INTO provider_hosts`. **No migration creates that table and nothing
  reads it.** A rail's hosts are `providers[].host` in the deployment's YAML
  (`vpay_config::ProviderHost`). The only other mention in the tree, a
  `vpay-testkit` module doc, inherited the error. Both corrected in place
  (struck through, not deleted) with the file's Status section saying so.
- `schemas/vpay.cstack` said restoring a `@default(...)` produces
  `error[E0063]`. Measured, and recorded correctly in the implementer's own
  notes and in `docs/reference/vpay-db.md`, it is `error[E0560]` — E0063 is the
  error going the other way. Corrected in `vpay.cstack` and in
  `config_reconcile.rs`'s test doc.

---

## 3. Concurrency with exp18

`claude/exp18-cratestack-outbox-opus` (head `cd86d85`) **adds no migration** —
`git diff 06e27f9..claude/exp18-cratestack-outbox-opus -- backends/migrations/`
is empty — so there is no 0033 number collision. Its `schemas/vpay.cstack`
edits are all below `model DisabledClient` (hunks at lines 566 and 575);
exp20's are in the header and in `model Provider` (lines 15–64, 251–350).
`git merge-tree --write-tree HEAD claude/exp18-cratestack-outbox-opus`
auto-merges `schemas/vpay.cstack`, `docs/status.md`,
`vpay-db/tests/repositories.rs` and `postgres_smoke.rs` cleanly. The **one**
conflict is `docs/reference/vpay-db.md`, where both branches rewrite the
"runs *N* queries through the generated data layer" sentence. Whoever merges
second resolves one paragraph.

---

## 4. Maintainer decisions surfaced, not taken

1. **The boot exit code for an incoherent adapter.** § 2b: it is
   `Category::Internal` / exit `1` now, was `Storage` / `69`. Either is better
   than the `69` it replaced, but the flow label one paragraph earlier got
   `Category::Configuration` / `78` for the same class of mistake, and the two
   now disagree. Related: should boot check `Capabilities::is_coherent` before
   it reconciles at all, rather than letting the CHECK answer?
2. **`fn reconcile` is 249 lines** on the review head — 203 before exp20, 237
   as delivered, and the review's two comment blocks (READ COMMITTED, the
   `23514` classification) took it the rest of the way. exp17's review left the
   length to a maintainer at 203; it is still almost all comment and still the
   longest production function on `verify-docs`' advisory list.

## 5. Gate

`just ci` end to end on `137a75e`, **exit 0**, under rustc 1.98.0 and Node
22.23.2 from `.nvmrc` with `pnpm install --frozen-lockfile`. Ten `verify`
gates green; `test-rust` **1389/1389, 0 skipped** (932 s); `verify-ignored`
**0 ignored, 43 binaries, 1389 total**; `test-doc` 96 passed / 1 ignored
(pre-existing); `deny` `advisories ok, bans ok, licenses ok, sources ok`;
`lint-web` and `test-web` green.

The first full run, on the delivered head, failed at test 1319/1386 on
`provider_callback::…::case_1_mtn_momo` with `failed to create a container:
Timeout error` — an abandoned `vpay-demo` compose stack was crash-looping on
this host and the load average was above 22. Every test it reached passed,
including the drift test at 84. Several targeted runs during the review hit the
same 120 s container-start timeout and passed on retry; none of them was an
assertion failure. Recorded rather than hidden.

## 6. Not checked

- The `just fmt` prettier hazard (222 unrelated files) — untouched, as in the
  delivered change. `just ci` runs `fmt-check`, never `fmt`.
- Whether `cratestack` 0.11.1's `upsert_exec.rs`/`upsert_sql.rs` behave as the
  comments cite them: taken from the implementer's citations and from the
  observed SQL, not re-read line by line in the vendored sources.
- Nothing was run against the real MTN or Orange sandboxes; unchanged by this
  branch.
