# `vpay-db` — CrateStack: `currencies`, `providers`, and decision D7

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### Migration 0032: what `currencies` and `providers` cost

Three statements, and their effects on the drift report are wildly unequal.
All three numbers below were measured with
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` on
2026-09-06, before and after; `docs/plans/exp17-notes/opus.md` has both
reports in full.

| Change                                                                      | Drift effect                        | Why it was made                                                                                                                     |
| --------------------------------------------------------------------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `currencies.exponent` `INT` → `BIGINT`                                      | **-1 change, -1 unmappable column** | `Int` emits `int8` and the introspector refuses to map `int4` back onto it, so the column was excluded from the comparison entirely |
| The two `currencies` CHECKs renamed to `<table>_<column>_<validator>_check` | **0**                               | The names are the half that _can_ converge at 0.12.0; the kinds cannot (below)                                                      |
| `providers.flow` native enum → `TEXT` + `providers_flow_enum_check`         | **0**                               | Nothing to do with drift: CrateStack cannot _decode_ a native enum column                                                           |

#### The rename that buys nothing, and why it is still right

`diff/checks.rs` matches CHECK constraints by name first and compares kinds
second. A hand-named CHECK is therefore proposed for `DROP` however correct
its predicate is, which is what `code_is_iso4217_shape` and
`exponent_in_range` were doing in the report. Renaming them to
`currencies_code_iso4217_check` and `currencies_exponent_range_check` — with
the predicates the generator emits, `code ~ '^[A-Z]{3}$'` byte for byte and
`exponent >= 0 AND exponent <= 4` in place of `BETWEEN 0 AND 4` — fixes the
name half and **moves the count by zero**.

The reason is a deliberate upstream decision, not a bug and not something a
better rename could avoid. Introspection reports _every_ validator-derived
CHECK as `CheckKind::Raw(<deparsed text>)`; it reconstructs only
`CheckKind::Enum`, and never `Iso4217`, `Range` or `Length`. `ir/checks.rs`
says why: "design doc §2.2 notes the compiled SQL for e.g. `@range(0, 150)` is
indistinguishable from hand-written `CHECK (age >= 0 AND age <= 150)` once it
reaches the catalog. Rather than guess which validator (if any) produced it,
introspection always reports it as opaque text." So a matching name turns two
unrelated report lines ("this CHECK is in the database and not the schema"
plus "this one is in the schema and not the database") into a same-named
drop-and-add pair. Clearer report, same number.

It is still right, for two reasons that are about the future rather than the
count. The database now carries the name a generated `migrate diff` would
emit DDL against. And when upstream grows validator reconstruction, the names
will already be in place and the count will fall on its own — whereas leaving
them hand-named would mean doing this rename later, on a table with rows.

**The general lesson for anyone moving the next table:** do not expect a
CHECK rename to move `EXPECTED_DRIFT_CHANGES`. Only a _type_ fix or a whole
table entering the schema does.

`providers.code_length` and `providers.display_name_length` were deliberately
**not** renamed. The count would not have moved for the reason above;
`display_name` carries no `@db_enforce` so its CHECK has no authored
counterpart to converge on at all; and `postgres_smoke.rs` asserts the report
still carries ``CHECK `code_length` ...`` as its proof that the report says
_something_ about `providers` beside the invisible cross-column CHECK.

#### Migration 0033: dropping a default moves no drift, and skipping it moves five

The mirror of the rename finding above, and the reason migration 0033 and the
`schemas/vpay.cstack` edit are one commit. Measured on 2026-09-06 with the same
test:

| Variant                                                                      | Report                                                                       |
| ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Five `@default(...)` in the schema, five `DEFAULT`s in the database (before) | **84 / 16 / 17**                                                             |
| Neither (after 0033)                                                         | **84 / 16 / 17**, `providers` block byte-identical                           |
| Schema half only — the five `@default(...)` removed, 0033 absent             | **89 / 16 / 17**, five `column … default value differs` lines on `providers` |

So a `DEFAULT` costs drift only when the two sides disagree, and the general
lesson is the one the CHECK rename taught in the other direction: **do not
expect a DDL change to move `EXPECTED_DRIFT_CHANGES` just because it is real.**
What 0033 buys is not a smaller number; it is that
`Create{Model}Input` can carry the five columns at all.

#### The enum conversion no report can see

`providers.flow` was `provider_flow`, a native Postgres enum. It is `TEXT`
with `CHECK (flow IN ('push', 'redirect'))` now, named
`providers_flow_enum_check` — `naming.rs::check_name(table, column, "enum")`.

The conversion had to happen and has nothing to do with drift. CrateStack
never emits a native enum on any backend: its generated row decoders read an
enum column with `try_get::<String>()` and `.parse()`, so a native enum column
**fails to decode on every read** (`emit/postgres/columns.rs`, upstream issue
#228). Any CrateStack query touching `providers` would have errored.

And the drift report is structurally blind to the whole thing, which is the
finding worth carrying:

- `introspect/postgres/enums.rs` _already synthesised_ an
  `AddCheck { name: "providers_flow_enum_check", kind: Enum { .. } }` out of
  `pg_enum` for the native column, so the CHECK matched before and matches
  after.
- `introspect/postgres/columns.rs::resolve_column` maps a native enum and a
  `TEXT` column onto the _same_ `ColumnType::Scalar("String")`.

So `providers` reports exactly the same four lines before and after. One of
those four, `column flow type differs (live: Scalar("String"), schema:
Enum("ProviderFlow"))`, is **permanent at 0.12.0** and is not a defect in
`schemas/vpay.cstack`: the enum's _name_ has no catalog representation to
recover it from, which `enums.rs`'s own doc comment calls documented
lossiness. Every enum-typed column in the schema carries one such line
(`charges.state`, `charges.failure_code`, `payment_intents.status`,
`ledger_entries.account`, `ledger_entries.direction`), and no migration can
remove them.

This is the mirror image of the multi-column-CHECK blind spot: there, the
report cannot see a constraint that exists; here, it cannot see a conversion
that happened. Both mean the same thing operationally — **a green drift report
is not evidence about either**, and a hand-written test is. For this
conversion that test is
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx`
(`vpay-db`'s own `config_reconcile::tests`), and reverting the `ALTER COLUMN
flow TYPE TEXT` is what makes it red.

`providers.flow` is the **first of vpay's seven native enums** to be
converted. The other six — `intent_status`, `charge_state`, `failure_code`,
`account_kind`, `direction`, and `payment_intents.last_payment_error_code` —
are on tables no CrateStack query touches, and each will need the same
treatment before one can.

### `reconcile`: read first, then write, and why that is the guard

`ConfigReconcile::reconcile`'s currency pass is two CrateStack calls where it
used to be one hand-written statement, and the extra round trip is not a
regression to be optimised away later. It is the invariant.

The statement it replaced was:

```text
INSERT INTO currencies (code, exponent) VALUES ($1, $2)
ON CONFLICT (code) DO UPDATE SET exponent = currencies.exponent
RETURNING exponent
```

`SET exponent = currencies.exponent` is a deliberate **no-op** write: it locks
the row and makes `RETURNING` yield the _stored_ value, so the comparison
behind `DbError::CurrencyExponentConflict` could happen in Rust. CrateStack's
`upsert` renders `SET exponent = EXCLUDED.exponent` — measured with
`preview_sql` and pinned by
`the_currency_upsert_would_overwrite_a_stored_exponent_on_its_own` — which is
the _overwrite_ that error exists to refuse. Letting it land would reinterpret
every amount already stored in that currency, silently, with no write to any
of those rows.

So the shape is:

```text
SELECT code, exponent FROM currencies WHERE code = $1 LIMIT 1 FOR UPDATE   -- find_unique(code).for_update()
   -> Some(row) with a different exponent?  CurrencyExponentConflict, transaction rolls back
   -> otherwise
INSERT INTO currencies (code, exponent) ... ON CONFLICT DO UPDATE SET exponent = EXCLUDED.exponent
```

Both `run_in_tx` on the transaction `reconcile` opened, after the same
`pg_advisory_xact_lock`, in the same sorted order. CrateStack joins vpay's
transaction; it never opens one of its own on this path. This is the first use
of `run_in_tx` anywhere in vpay.

**Two guards, and they are not the same guard.** The advisory lock serialises
`reconcile` against every other `reconcile` — that is what
`reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` proves,
and it still passes with `.for_update()` deleted. The row lock binds a writer
that does _not_ go through this function. Measured on 2026-09-06: with
`.for_update()` deleted, **every test in the repository stayed green**, which
is why this change adds
`reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer`
— an outside transaction updates the row uncommitted, `reconcile` blocks on
the read, and the assertion is that the outside writer's value _survives_.
Without the row lock the plain `SELECT` returns the pre-commit value, the
comparison passes, and the upsert writes over a committed change. See
`docs/plans/exp17-notes/opus.md` § 2.

**Two pooled connections on the conflict branch**, as with `disable_client`:
`upsert_resolve.rs::gate_update_policy` runs its policy probe on
`runtime.pool()` while this transaction holds a connection of its own. It does
**not** deadlock against the row this transaction just locked, and that is
worth stating explicitly because it is the obvious worry:
`row_passes_update_policy` emits a plain `SELECT 1 ... AND (<policy>)` with no
`FOR UPDATE`, and Postgres's MVCC reads are not blocked by writers. Two of
`MAX_CONNECTIONS = 10` per in-flight reconcile, on a boot step the advisory
lock already admits one at a time.

### `providers` is written through CrateStack (D7, resolved 2026-09-06)

`reconcile`'s **provider** pass is
`upsert(CreateProviderInput).run_in_tx(tx, ctx)`. It was a hand-written
`INSERT ... ON CONFLICT` for one day, and the reason it was is the reason
migration 0033 exists, so both are recorded here rather than only the outcome.

**The blocker.** `cratestack-macros`' `model/inputs.rs::create_input_fields`
and `model/descriptor/columns.rs`'s `upsert_update_columns` both drop every
field carrying a `@default(...)`, and `model Provider` carried one on all five
capability booleans because the live table did (migration 0002: `DEFAULT
FALSE` ×4, `DEFAULT TRUE`). Measured with `preview_sql` on 2026-09-06:

```text
INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
```

`supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist` and `enabled` were in neither list. Boot step 4 would
have inserted every rail with the column defaults regardless of what the
deployment configured, and would never have carried a capability change to an
existing row, so a rail an operator had just disabled would come back enabled.

**The decision.** Two ways out were measured, and the maintainer's delegate
took the first: remove the five `@default(...)` from `model Provider` **and**
`ALTER TABLE providers ALTER COLUMN … DROP DEFAULT` on all five. The argument
is not that a code generator should get to shape vpay's DDL — it is that the
default was never doing anything for the only writer there is. `reconcile` is
the sole writer of those five columns and always writes all five from
configuration; a default cannot help it. What a default _can_ do is invent a
capability for some other writer that forgot one, and a rail silently recorded
as "does not refund", or silently recorded as enabled, is worse than an
`INSERT` that refuses. The second way out — upstream growing a way to include
a defaulted column in an upsert input — remains open (re-checked at 0.12.0
on 2026-09-07: `create_input_fields` still filters every `@default(...)`
field) and would now be a simplification rather than an unblocking.

**The two halves are one commit, and that is measured.** With the five
`@default(...)` removed from the schema and the DDL untouched, the drift report
goes **84 → 89**: exactly five `column … default value differs` lines on
`providers`. With both halves done it stays at **84 / 16 relations / 17
unmappable**, and the `providers` block is byte-identical to what it was
before — the two sides agreed when both had the defaults and they agree now
that neither does. Dropping a default moves no drift by itself.

```text
INSERT INTO providers (code, display_name, flow, supports_refunds,
    supports_partial_refunds, delivers_callbacks, requires_ip_allowlist, enabled)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name,
    flow = EXCLUDED.flow, supports_refunds = EXCLUDED.supports_refunds, … ,
    enabled = EXCLUDED.enabled
```

is what renders now, pinned by
`the_provider_upsert_carries_all_eight_columns`.

**Two things notice a regression, and the compiler is the first.** The five
columns are `CreateProviderInput` _fields_, not merely SQL: restore a
`@default(...)` in `schemas/vpay.cstack` and `vpay-db` stops building with
`error[E0560]: struct `inputs::CreateProviderInput`has no field named`supports_refunds`` at `reconcile`'s struct literal (measured). Delete the
migration instead and the failure is at test time, three ways: the drift test
at 89 vs 84, the migration-count test at 32 vs 33, and
`a_hand_written_provider_insert_must_now_name_every_capability_column`, which
finds the three-column `INSERT` accepted again.

**What the drop costs an operator.** All five columns are `NOT NULL` with no
default, so a hand-written `INSERT INTO providers (code, display_name, flow)`
is a `23502` rather than a row with invented capabilities. That is the whole
cost, it is a refusal rather than a silent difference, and it is asserted in
both directions by the test named above. Three fixtures in `postgres_smoke.rs`
had to name the columns in the same commit.

**No `find_unique(...).for_update()` ahead of the provider upsert.** This is
the asymmetry with the currency pass and it is deliberate. The currency read is
the _guard_: `upsert` renders `SET exponent = EXCLUDED.exponent`, a stored
exponent must never be overwritten, so the value has to be read under a row
lock and compared before the write. A provider has no such value — every one of
the eight columns is owned by configuration and overwriting it is the point, so
the read would return a row nothing compares. The row lock itself is taken
anyway, in the same transaction, by `upsert`'s own conflict probe
(`upsert_exec.rs::run_upsert_in_tx` calls
`select_for_update_by_conflict_target` on `tx`). One measured consequence is
worth writing down because it inverts the currency finding: with no
`providers` row lock held when the upsert runs, swapping `run_in_tx` for `run`
**fails in 1.2 s** rather than hanging.
`a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
is the assertion that fails; the currency equivalent had to be written
specifically because its mutation produced `SLOW [>480.000s]` and no error at
all.

What the conflict probe needs in exchange is **READ COMMITTED**, and that is a
real constraint rather than an incidental one: the transaction has to see a
`providers` row another writer committed while it was waiting on
`lock_keys::CONFIG_RECONCILE`. `pool.begin()` opens Postgres's default and
that is what makes this work.
`a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed`
(`vpay-db/tests/repositories.rs`, added in the review pass on 2026-09-06) is
the only test that says so: under `SET TRANSACTION ISOLATION LEVEL REPEATABLE
READ` the snapshot is taken when the advisory-lock statement starts, before
the holder commits, the probe cannot see the row, and boot fails with `40001`
— measured, with every other reconcile case still passing.

**A classification changed on purpose.** `CreateProviderInput::flow` is the
schema's `ProviderFlow` enum, so `reconcile` parses `ProviderSeed::flow` — a
`String`, and left one, because Step 2's D4 says `vpay-db` binds strings —
before it builds a statement. An unparseable label is
`DbError::ProviderFlowUnknown`, `Category::Configuration`, exit `78`. It used
to reach `providers_flow_enum_check` and come back as `DbError::Query` →
`Category::Storage` → exit `69`, which told a supervisor to wait for a
database that was working perfectly. The CHECK is untouched and still refuses
a writer that is not `reconcile`. The parse is a `match` rather than
`unwrap_or_default()`, because `cratestack-macros` derives `Default` on every
generated enum with the _first_ variant as the default
(`types/enums.rs::variant_tokens`) and the first variant of `ProviderFlow` is
`push` — so `unwrap_or_default()` would have stored a typo'd rail as a push
rail and returned `Ok`.

**`model Provider` gained `create` and `update` arms**, in the commit that
moved the write and not before it, and has no `delete` arm: `reconcile`
disables a dropped rail and never removes one, and every `charges` and
`provider_requests` row references this table. Deleting the `update` arm is
loud but only from the _second_ boot — the first boot of a fresh database
takes the insert branch — which is why
`every_action_this_module_calls_has_an_allow_arm` asserts the slot without a
database.
