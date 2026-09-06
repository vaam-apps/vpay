# S2b — `currencies` and `providers` through CrateStack, and the first native-enum conversion

Working notes for migration 0032 and the CrateStack move of
`ConfigReconcile::reconcile`'s currency pass. Base: `4385d0d` (master with S1
and S2a merged — 43 test binaries, 1373 tests, 31 migrations, drift 85 / 16 /
18 unmappable). Everything below is a measurement taken on this branch on
2026-09-06 against `postgres:16-alpine` and `cratestack-cli 0.11.1`, or a
citation into the pinned 0.11.1 crate sources.

The conclusions live in
[docs/reference/vpay-db.md](../../reference/vpay-db.md) and
[docs/status.md](../../status.md). This file is the transcript.

---

## 1. The headline: two of migration 0032's three changes moved no drift at all

`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`, before
and after.

**Before (85 / 16, 18 unmappable):**

```text
currencies:
  [safe] CHECK `code_is_iso4217_shape` exists in the live database but is not declared in the schema
  [safe] CHECK `exponent_in_range` exists in the live database but is not declared in the schema
  [blocking] column `exponent` is declared in the schema but does not exist in the live database
  [blocking] CHECK `currencies_code_iso4217_check` is declared in the schema but does not exist in the live database
  [blocking] CHECK `currencies_exponent_range_check` is declared in the schema but does not exist in the live database

providers:
  [safe] CHECK `code_length` exists in the live database but is not declared in the schema
  [safe] CHECK `display_name_length` exists in the live database but is not declared in the schema
  [lossy] column `flow` type differs (live: Scalar("String"), schema: Enum("ProviderFlow"))
  [blocking] CHECK `providers_code_length_check` is declared in the schema but does not exist in the live database

18 column(s) have a Postgres type cratestack could not confidently map ...
  currencies.exponent: int4
```

**After (84 / 16, 17 unmappable):**

```text
currencies:
  [safe] CHECK `currencies_code_iso4217_check` exists in the live database but is not declared in the schema
  [safe] CHECK `currencies_exponent_range_check` exists in the live database but is not declared in the schema
  [blocking] CHECK `currencies_code_iso4217_check` is declared in the schema but does not exist in the live database
  [blocking] CHECK `currencies_exponent_range_check` is declared in the schema but does not exist in the live database

providers:
  [safe] CHECK `code_length` exists in the live database but is not declared in the schema
  [safe] CHECK `display_name_length` exists in the live database but is not declared in the schema
  [lossy] column `flow` type differs (live: Scalar("String"), schema: Enum("ProviderFlow"))
  [blocking] CHECK `providers_code_length_check` is declared in the schema but does not exist in the live database
```

`providers` is **byte-identical**. The entire -1 is `currencies.exponent`
leaving the unmappable block and matching.

### 1a. Why the CHECK rename bought nothing

`diff/checks.rs` matches by name, then compares kinds. The names now match —
and the kinds cannot. `introspect/postgres/constraints.rs` runs
`reconstruct_enum` on the deparsed predicate and falls back to
`CheckKind::Raw(text)` for everything it does not recognise, and the only
shape it recognises is enum membership (`= ANY (ARRAY[...])` /
`<@ ARRAY[...]`). `ir/checks.rs` says why this is deliberate:

> design doc §2.2 notes the compiled SQL for e.g. `@range(0, 150)` is
> indistinguishable from hand-written `CHECK (age >= 0 AND age <= 150)` once
> it reaches the catalog. Rather than guess which validator (if any) produced
> it, introspection always reports it as opaque text.

So `Raw("code ~ '^[A-Z]{3}$'::text") != Iso4217` and the diff emits a
drop-and-add on the *same name*. Two lines before, two lines after.

The rename was kept anyway: the database now carries the name a generated
`migrate diff` would emit DDL against, the names are the half that *can*
converge at 0.11.1, and doing this rename later means doing it on a table
with rows. But **do not expect a CHECK rename to move
`EXPECTED_DRIFT_CHANGES`** — that expectation is what this section exists to
correct.

`providers.code_length` and `providers.display_name_length` were left
hand-named for the same reason plus two of their own: `display_name` has no
`@db_enforce`, so its CHECK has no authored counterpart to converge on at
all, and `postgres_smoke.rs` asserts the report still carries
``CHECK `code_length` ...`` as its evidence that the report says *something*
about `providers` beside the invisible cross-column CHECK.

### 1b. Why the enum conversion bought nothing, and why it was still required

Two upstream behaviours make the conversion invisible to the report:

- `introspect/postgres/enums.rs` **already** synthesised
  `AddCheck { name: check_name(table, column, "enum"), kind: Enum { .. } }`
  out of `pg_enum` for the native column. Its module doc is explicit that this
  exists so "`diff_projections` doesn't see false drift on the CHECK". The
  hand-written `CHECK (flow IN ('push','redirect'))` deparses to the shape
  `reconstruct_enum` reads back, under the same generated name. Identical
  before and after.
- `introspect/postgres/columns.rs::resolve_column` maps `typtype == 'e'` to
  `Some("String")` — the same `ColumnType::Scalar("String")` a `text` column
  gets.

The remaining line, `column flow type differs (live: Scalar("String"), schema:
Enum("ProviderFlow"))`, is **permanent at 0.11.1**. `enums.rs`'s own doc:
"the `.cstack`-side enum *name* has no catalog representation to recover it
from, which is the same documented lossiness the design doc's §2.2 already
calls out". Every enum-typed column in `schemas/vpay.cstack` carries one
(`charges.state`, `charges.failure_code`, `payment_intents.status`,
`ledger_entries.account`, `ledger_entries.direction`).

The conversion was required for a runtime reason no report can express — see
mutation 1.

---

## 2. Mutations

Every one was applied, run, and reverted. `git status` was clean afterwards
except for the intended change.

### Mutation 1 — revert `ALTER COLUMN flow TYPE TEXT` (three stages, because the first two stop short)

The naive mutation does not reach the thing it is supposed to prove, and that
is worth recording rather than smoothing.

**1a.** Comment out the `ALTER COLUMN`, the `ADD CONSTRAINT` and the `DROP
TYPE`. `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx`
fails — but at the *seed*, because `reconcile`'s hand-written INSERT no
longer casts `$3`:

```text
Error: seeding one rail through boot step 4
Caused by: column "flow" is of type provider_flow but expression is of type text
```

**1b.** Also restore `$3::provider_flow` in `config_reconcile`. Now it fails
at the test's *own raw* read, because sqlx will not decode a native enum as a
`String` either:

```text
Error: the sqlx read must find the row boot step 4 just wrote
Caused by: error occurred while decoding column 2: mismatched types;
  Rust type `alloc::string::String` (as SQL type `TEXT`) is not compatible with SQL type `provider_flow`
```

**1c.** Also cast the test's own read to `flow::TEXT`, so the CrateStack read
is finally reached. **This is the measurement:**

```text
Error: the CrateStack provider read failed: database: error occurred while decoding column "flow":
  mismatched types; Rust type `alloc::string::String` (as SQL type `TEXT`)
  is not compatible with SQL type `provider_flow`
```

That is `emit/postgres/columns.rs`'s stated reason for never emitting a
native enum ("the generated row decoders read every enum field with
`try_get::<String>` and `.parse()`, so a native enum column fails to decode
on every read", upstream issue #228), reproduced.

The useful secondary finding: the failure is **sqlx-level, not
CrateStack-level**. Any Rust reader binding `String` to a native enum column
hits it identically. What is specific to CrateStack is that it gives you no
way to ask for the cast.

### Mutation 2 — delete `.for_update()` from the currency read

`reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`:
**PASS**, 4.39 s. As expected — the advisory lock, not the row lock, is what
serialises boot against boot.

Then the part the brief did not ask for and that mattered more:
`cargo nextest run -p vpay-db --lib --test repositories` with the mutation
still applied — **103 tests run, 103 passed, 0 skipped.** *Nothing in the
repository caught it.* The row lock was an unguarded guard.

So this change adds
`reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer`,
which is deterministic rather than raced:

1. seed `XAF` at exponent 0;
2. an outside transaction runs `UPDATE currencies SET exponent = 3` and does
   **not** commit — it holds the row's write lock, and 3 is invisible to
   every other snapshot;
3. `reconcile` starts with a seed of `XAF` at 0 — agreeing with what is
   committed, disagreeing with what is about to be. It takes the advisory
   lock (free) and blocks on the row;
4. the blocker commits.

With `.for_update()`: the *read* is what blocked, so it returns the
post-commit 3 and boot refuses with `CurrencyExponentConflict { stored: 3,
seeded: 0 }`; the stored value stays 3.

Without it: the plain `SELECT` returns the pre-commit 0 immediately, the
comparison passes, and the upsert (whose own internal probe blocks on the
same row) then writes `exponent = 0` over the committed 3.

Measured both ways. Under the mutation:

```text
FAIL reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer
  the read must see the committed 3 and refuse a seed of 0. If this returned `Ok`,
  the read did not block on the row and boot has just overwritten another writer's value: ()
```

With `.for_update()` restored: PASS, 4.20 s, alongside the three other
reconcile cases.

**Which guard is which, stated plainly:** the `pg_advisory_xact_lock` binds
every writer that goes *through* `reconcile`; the row lock binds a writer
that does not. Neither test covers the other's guard.

### Mutation 3 — delete `@@allow("create", …)` from `model Currency`

LOUD, on every boot, exactly as `upsert_exec.rs`'s pre-flight implies:

```text
Error: the first reconcile must succeed
Caused by: Currency: a model policy denied a system upsert: forbidden: create policy denied this upsert
```

`every_action_this_module_calls_has_an_allow_arm` (no container) catches it
in 5 ms with the message naming the consequence.

### Mutation 4 — delete `@@allow("read", …)` from `model Currency`

SILENT at runtime, and the dangerous direction. Three container tests go red:

```text
FAIL a_hand_seeded_currency_exponent_is_read_back_and_refused_not_overwritten
FAIL reconcile_is_idempotent_and_disables_a_dropped_provider_code
FAIL reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer
PASS reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released
PASS two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge
```

The read compiles the policy into the `WHERE`, so an empty allow list renders
`FALSE`, `find_unique` answers `None` for a row that exists, and the upsert
overwrites the stored exponent instead of refusing to. The no-container
`every_action_this_module_calls_has_an_allow_arm` also catches it.

### Mutation 5 — delete `CONSTRAINT partial_refunds_imply_refunds` from migration 0002

```text
FAIL partial_refunds_without_refunds_is_rejected_by_the_database
FAIL the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount
  drift detected in 16 table(s)/view(s) (84 change(s) total):
```

The count is **unmoved**. The drift test fails only on its own
`pg_constraint` read, which is the assertion that exists precisely because
the report cannot see a multi-column CHECK. Re-confirms the S0 finding on
this branch's numbers, which is why migration 0032 leaves that constraint
alone and says so in a comment.

---

## 3. The blocker: `providers` cannot be written through CrateStack at 0.11.1

Measured with `preview_sql` against a `connect_lazy` pool (no I/O):

```text
INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
RETURNING code AS "code", display_name AS "display_name", flow AS "flow", ...
```

`supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist` and `enabled` are in neither the insert list nor the
`SET` list. `cratestack-macros`' `model/inputs.rs::create_input_fields` and
`model/descriptor/columns.rs`'s `upsert_update_columns` both filter on
`is_generated_on_create`, which is exactly `has_default(field)`, and `model
Provider` carries a `@default(...)` on all five because migration 0002's
table does.

Shipping it would insert every rail with `supports_refunds = false … enabled
= true` regardless of configuration, and would never carry a capability
change to an existing row — a rail an operator had just disabled would come
back enabled. It was not shipped.
`the_provider_upsert_cannot_carry_the_capability_columns` pins the rendered
statement so an upstream fix turns it red.

For contrast, the currency input is unaffected because `model Currency` has
no `@default(...)`:

```text
INSERT INTO currencies (code, exponent) VALUES ($1, $2)
ON CONFLICT (code) DO UPDATE SET exponent = EXCLUDED.exponent
RETURNING code AS "code", exponent AS "exponent"
```

— and `SET exponent = EXCLUDED.exponent` is precisely why the read has to
come first (§2 mutation 2). The statement it replaced was `SET exponent =
currencies.exponent`, a deliberate no-op whose `RETURNING` handed back the
stored value.

**Left to the maintainer, not decided here.** Unblocking means removing the
five `@default(...)`s *and* `ALTER TABLE providers ALTER COLUMN ... DROP
DEFAULT` on all five — a code generator's input-shaping rule deciding vpay's
DDL. Removing the `@default`s without the DDL change is not an option: the
drift report grows five `default value differs` lines.

---

## 4. Two connections, and why boot does not deadlock against itself

`upsert_resolve.rs::gate_update_policy` runs `row_passes_update_policy` on
`runtime.pool()` — a *second* pooled connection — while `reconcile`'s own
transaction holds one. The obvious worry is a self-deadlock, because that
transaction has just taken `FOR UPDATE` on the same row.

It does not deadlock, and the reason is in `upsert_sql.rs`:
`row_passes_update_policy` builds `SELECT 1 FROM <table> WHERE <pk> = $1 AND
(<policy>)` with **no** `FOR UPDATE`, and a plain MVCC read is not blocked by
a writer. Two of `pool.rs`'s `MAX_CONNECTIONS = 10` per in-flight reconcile,
on a boot step the advisory lock already admits one at a time.
`two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge`
passes unchanged.

`evaluate_create_policies` issues no query at all for `auth().isSystem()` —
it is not a relation predicate — so the insert branch takes one connection.

---

## 5. Consequences elsewhere

- `CurrencySeed::exponent` and `DbError::CurrencyExponentConflict`'s
  `stored`/`seeded` are `i64`. sqlx refuses the narrowing rather than
  performing it (`mismatched types; Rust type i32 (as SQL type INT4) is not
  compatible with SQL type INT8`), so this surfaced as a test failure rather
  than as silent truncation.
- `vpay_api::v1::boot::boot_seeds` no longer returns `ConfigError::Validation`
  at all. Its "exponent does not fit the column" arm became unreachable *by
  type* — every `u32` fits an `i64` — so the `try_from` was replaced with
  `i64::from` rather than left as an error branch nothing could take. The
  real bound is unchanged and two layers up (`Config::validate_all` against
  `vpay_core::Currency::exponent`, then `currencies_exponent_range_check`).
- Every `'push'::provider_flow` cast in the tree is gone: `config_reconcile`,
  `postgres_smoke.rs::seed_providers`,
  `partial_refunds_without_refunds_is_rejected_by_the_database`, and
  `repositories.rs::provider_snapshot`'s `flow::TEXT`.
- `sql_audit.rs` refused a `format!("{column} = EXCLUDED.{column}")` in a new
  test — correctly; it scans for interpolation next to anything
  statement-shaped. The needles are written out in full instead of adding an
  audit exception.

---

## 6. A hazard in `just fmt` that is not this change's, found while running the gate

`just fmt` is `cargo fmt --all` **then** `pnpm exec prettier --write .`.
Running it on this tree rewrites 222 tracked files with prettier's defaults —
`*italic*` to `_italic_` in every Markdown file, every Markdown table
column-padded, `package.json`'s inline `engines` object exploded onto three
lines — and then **fails**, because
`backends/crates/vpay-config/tests/fixtures/malformed.yml` is deliberately
malformed YAML and prettier refuses to parse it:

```text
[error] backends/crates/vpay-config/tests/fixtures/malformed.yml: SyntaxError: Missing closing "quote (9:1)
error: Recipe `fmt` failed on line 265 with exit code 2
```

So the recipe leaves the tree reformatted *and* reports failure. Reverted in
full here; no prettier output is in this branch. Not fixed, because it is
unrelated to this task and the fix is a decision (add a `.prettierrc`
matching how the tree is actually written, narrow the recipe's glob to the
TypeScript trees, or add an ignore entry for the fixture).

**Correction, review pass 2026-09-06:** this section said "this repository
ships no prettier configuration file". It does — `.prettierignore`, added in
Step 6, whose header comment documents this exact failure mode for
`deploy/helm/**/templates/`. So the parse error has an established in-repo
remedy (one more ignore line) and is not a new decision; the 222 reformatted
files still are. The count was also "roughly two hundred" by eye and is 222
by `prettier --list-different .`. `just ci` is unaffected — it runs
`fmt-check` (`cargo fmt --all -- --check`), never `fmt`.

---

## 7. Not done

- No table other than `currencies` and `providers` moved.
- `reconcile` still owns its own transaction; the `UnitOfWork` /
  `PendingTransaction` seam is untouched.
- `reconcile`'s provider upsert and its disable pass (`UPDATE providers SET
  enabled = false WHERE code <> ALL($1)`) are both still raw sqlx.
- No production path reads `providers` through CrateStack. `model Provider`'s
  `@@allow("read", …)` exists for one test, and `model Provider` has no write
  arm on purpose.
- `providers.code_length` and `providers.display_name_length` are still
  hand-named (§1a).
- `partial_refunds_imply_refunds` untouched, deliberately (§2 mutation 5).
- The remaining six native enums are unconverted.
