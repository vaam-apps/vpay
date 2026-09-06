# D7 — dropping the `providers` capability defaults, and moving the provider pass onto CrateStack

Working notes for migration 0033 and for
`ConfigReconcile::reconcile`'s provider pass. Base: `06e27f9` (master with
S2b / PR #55 merged — 32 migrations, drift 84 / 16 / 17 unmappable, 1382
tests). Everything below was measured on this branch on 2026-09-06 against
`postgres:16-alpine` under the pinned 1.98.0 toolchain and
`cratestack 0.11.1`, or is a citation into the pinned 0.11.1 sources.

The conclusions live in [docs/reference/vpay-db.md](../../reference/vpay-db.md)
§ CrateStack, [docs/status.md](../../status.md) and
[docs/flows/configuration.md](../../flows/configuration.md). This file is the
transcript, and it exists mostly to record the two places where the measured
answer differs from what the brief expected.

---

## 1. The drift did **not** go 84 → 79. It stayed at 84, and that is correct

The task brief said "expect 84→79 if the DDL is right, say what you got". The
measured answer is **84 / 16 relations / 17 unmappable, before and after**, and
the `providers` block is byte-identical:

```text
providers:
  [safe] CHECK `code_length` exists in the live database but is not declared in the schema
  [safe] CHECK `display_name_length` exists in the live database but is not declared in the schema
  [lossy] column `flow` type differs (live: Scalar("String"), schema: Enum("ProviderFlow"))
  [blocking] CHECK `providers_code_length_check` is declared in the schema but does not exist in the live database
```

The expectation of −5 reads the earlier measurement backwards. exp17's own
note is explicit that while the five `@default(...)` were in the schema **and**
the five `DEFAULT`s were in the table, the two agreed and *cost zero drift*;
what its review measured was that removing the schema half alone takes the
report to **89**, five `column … default value differs` lines. A default is
only ever drift when the two sides disagree. Removing both halves leaves them
agreeing, so the count cannot fall — there was nothing there to remove.

Four variants, all with this branch's `the_cstack_schema_drifts_from_the_
migrations_by_a_measured_amount`:

| Variant | Report |
|---|---|
| **A** — as delivered (no `@default(...)`, no `DEFAULT`) | **84 / 16 / 17** |
| **B** — master (five `@default(...)`, five `DEFAULT`) | **84 / 16 / 17** |
| **C** — schema half only (no `@default(...)`, migration 0033 deleted) | **89 / 16 / 17**, five `default value differs` on `providers` |
| **D** — DDL half only (`@default(false)` restored on one field, 0033 kept) | **does not compile** — see § 3 mutation 4 |

C is the one that matters: it is why the migration and the schema edit are one
commit, and it reproduces exp17's review pass from the opposite side.

`EXPECTED_DRIFT_CHANGES`, `EXPECTED_DRIFTED_RELATIONS` and
`EXPECTED_UNMAPPABLE_COLUMNS` are therefore all **unmoved**, and nothing in
`postgres_smoke.rs` needed a constant bump. The general lesson, alongside the
CHECK-rename one exp17 recorded: **a DDL change being real is not a reason to
expect the count to move.** Only a type fix, or a whole table entering the
schema, does.

## 2. The one place the brief was not followed: there is no
`find_unique(...).for_update()` ahead of the provider upsert

The brief asked for "`find_unique().for_update()` + `upsert`, both `run_in_tx`".
The `upsert` is there and is `run_in_tx`. The read is not, and this is a
deliberate deviation rather than an omission.

For **currencies** the read is the guard, and that is why exp17 wrote it: the
generated upsert renders `SET exponent = EXCLUDED.exponent`, a stored exponent
must never be overwritten, so the value has to be read under a row lock and
compared before the write. `DbError::CurrencyExponentConflict` is the whole
point of the pass.

For **providers** there is nothing to compare. All eight columns are owned by
configuration and overwriting them is the entire job of the statement — that is
what `a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile` asserts.
A `find_unique` there would return a row no code reads.

The row lock a read would take is taken anyway, in the same transaction, a few
microseconds later: `upsert_exec.rs::run_upsert_in_tx` calls
`select_for_update_by_conflict_target(&mut **tx, …)` as its conflict probe.
So the only honest comment that could be written above such a read is "this
takes a lock the next statement takes and returns a row nothing uses", and a
guard whose comment says that is the kind of decorative safety this repository
is supposed to refuse. It is one line to add if a maintainer disagrees.

**One measured consequence, and it inverts the currency finding.** Because no
`providers` row lock is held in the transaction when the upsert runs, the
`run_in_tx` → `run` mutation on the provider pass **fails in 1.2 s** rather
than hanging (§ 3 mutation 1). The equivalent mutation on the currency pass
deadlocks — exp17's review measured `SLOW [>480.000s]` — precisely because
`find_unique(...).for_update()` is holding the row the private transaction's
own probe wants. So the brief's warning ("mind the measured deadlock") applies
to the shape it asked for and not to the shape delivered; the delivered shape
is the one whose failure signal is a red assertion instead of a boot that
never returns.

## 3. Mutations

Every one applied to a clean tree, run, and reverted; the tree was diffed back
against a pre-mutation snapshot each time.

### Mutation 1 — provider upsert `.run_in_tx(&mut tx, &ctx)` → `.run(&ctx)`

**FAILS in 1.226 s. Nothing hangs.**

```text
FAIL [1.226s] a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction
  assertion `left == right` failed: the CrateStack provider upsert must have been
  rolled back with everything else. A 1 here is `alpha_rail`, written on its own
  connection instead of joining this transaction — check that `reconcile`'s provider
  upsert still says `run_in_tx(&mut tx, &ctx)` and not `run(&ctx)`
    left: 1
   right: 0
```

The whole reconcile set was run under the mutation
(`-E 'test(/reconcile/) or test(/rolled_back/) or …'`, 14 cases): one failure,
no `SLOW` line, longest case 5.1 s. The other 12 passed —
`reconcile_is_idempotent_and_disables_a_dropped_provider_code` among them,
which is the case that hung under the currency version of this mutation. That
is § 2's point, measured.

The test is arranged so a valid rail lands first and an invalid one fails after
it: seeds are iterated in sorted `code` order, `alpha_rail` sorts before
`zulu_rail`, and `zulu_rail` sets `supports_partial_refunds` without
`supports_refunds`, which `partial_refunds_imply_refunds` refuses. Both rails
are INSERTs, so the conflict probe finds and locks nothing and the mutation
cannot block even in principle.

### Mutation 2 — delete `@@allow("update", …)` from `model Provider`

**LOUD, and only from the SECOND boot** — which is exactly what makes the
no-container slot test worth having.

```text
FAIL [0.003s] config_reconcile::tests::every_action_this_module_calls_has_an_allow_arm
FAIL [1.236s] reconcile_is_idempotent_and_disables_a_dropped_provider_code
  Error: a second, identical reconcile must succeed
  Caused by: Provider: a model policy denied a system upsert: forbidden: update policy denied this upsert
FAIL [19.131s] a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile
  Error: reconciling a disabled rail must succeed
  Caused by: Provider: a model policy denied a system upsert: forbidden: update policy denied this upsert
```

A fresh database's first reconcile takes the insert branch and succeeds. A test
that reconciled once and stopped would have passed under this mutation; both
container cases above reconcile at least twice, and
`every_action_this_module_calls_has_an_allow_arm` catches it in 3 ms with no
Docker at all.

### Mutation 3 — delete migration 0033, keep the schema edit

**Test-time, three ways.** Not compile-time: nothing in Rust names a column
default.

```text
FAIL the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount
  drift detected in 16 table(s)/view(s) (89 change(s) total):
    [safe] column `delivers_callbacks` default value differs from the schema
    [safe] column `enabled` default value differs from the schema
    [safe] column `requires_ip_allowlist` default value differs from the schema
    [safe] column `supports_partial_refunds` default value differs from the schema
    [safe] column `supports_refunds` default value differs from the schema
  left: 89, right: 84

FAIL schema_migrates_cleanly_on_an_empty_database          (left: 32, right: 33)
FAIL a_hand_written_provider_insert_must_now_name_every_capability_column
```

The third is the one that is about behaviour rather than bookkeeping: with the
defaults back, `INSERT INTO providers (code, display_name, flow) VALUES (…)` is
accepted again and records a rail as not refunding, not delivering callbacks,
and **enabled**.

### Mutation 4 — restore one `@default(false)` in `model Provider`, keep 0033

**Compile-time**, before any test runs:

```text
error[E0560]: struct `inputs::CreateProviderInput` has no field named `supports_refunds`
error: could not compile `vpay-db` (lib) due to 1 previous error
```

This is exp17's review finding 6 reproduced in the opposite direction: the five
columns are `CreateProviderInput` *fields*, not merely SQL, so the schema half
of D7 cannot regress quietly. `the_provider_upsert_carries_all_eight_columns`
is the second thing that would notice, not the first.

## 4. Consequences elsewhere

- **`DbError` grew a variant.** `ProviderFlowUnknown { code, flow }`,
  `Category::Configuration`, exit `78`. `CreateProviderInput::flow` is the
  schema's `ProviderFlow` enum, so the seed's `String` has to be parsed before
  a statement exists. `ProviderSeed::flow` stays a `String` — Step 2's D4
  ("Postgres enums are `String` in vpay-db; vpay-core parses") is a recorded
  decision and reversing it was not this task's to make.

  The *classification* deliberately changed. The label used to reach
  `providers_flow_enum_check` and come back as `DbError::Query` →
  `Category::Storage` → exit `69`, i.e. "wait for Postgres" for a typo in a
  YAML file. That was wrong about who has to act. The CHECK is untouched and
  `an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`
  still passes unchanged, because it writes through raw sqlx.

  The parse is a `match` and not `unwrap_or_default()`:
  `cratestack-macros/src/types/enums.rs::variant_tokens` marks the **first**
  variant `#[default]` on every generated enum, and the first variant of
  `ProviderFlow` is `push`. `unwrap_or_default()` would have stored `redirekt`
  as a push rail and returned `Ok`.

- **An existing test had to change its expected variant.**
  `a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
  matched `DbError::Query(sqlx_error)` and read the constraint off the driver
  error; the statement that raises the `23514` is a CrateStack upsert now, so
  it is `DbError::Persistence(PersistenceError::Check { constraint, .. })`.
  Same constraint name, same `Category::Internal`. This is the module doc's own
  warning — "a caller matching the variant would have silently stopped
  matching" — landing on a test rather than on a caller.

- **Three fixtures in `postgres_smoke.rs` had to name the capability columns**
  (`seed_providers`, `partial_refunds_without_refunds_is_rejected_by_the_database`,
  `an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`).
  In the third it is load-bearing: with the columns omitted the row would be
  refused for a missing `supports_refunds` and the assertion about the flow
  CHECK would read `None`. That is the operator note made concrete — it is
  what a `psql` prompt now does too.

- **Migration 0002's `COMMENT ON TABLE providers`** has said "reconciliation
  from YAML is not implemented yet" since before boot step 4 existed.
  `sqlx::migrate!` checksums applied files, so 0002 cannot be edited; 0033
  re-states the comment instead, and the new text names the missing defaults
  because a psql prompt is where an operator meets them.

## 5. Not done

- No table other than `currencies`, `providers` and `disabled_clients` runs
  through CrateStack, and nothing new *reads* `providers` through it — 0033
  changed the write. `model Provider`'s `read` arm is still there for one test.
- `reconcile`'s **disable pass** is still raw sqlx. It addresses rows by their
  absence from a list (`WHERE code <> ALL($1)`), which no generated builder
  expresses.
- `reconcile` still owns its own transaction; the `UnitOfWork` /
  `PendingTransaction` seam is untouched.
- `providers.code_length` and `providers.display_name_length` are still
  hand-named, for exp17 § 1a's reasons.
- The remaining six native enums are unconverted.
- No `find_unique(...).for_update()` on the provider pass — § 2, deliberate and
  the one deviation from the brief.
- The `just fmt` hazard exp17 recorded (222 files reformatted by prettier's
  defaults, then a parse error on a deliberately malformed fixture) is
  untouched. `just ci` runs `fmt-check`, never `fmt`; no prettier output is in
  this branch.
