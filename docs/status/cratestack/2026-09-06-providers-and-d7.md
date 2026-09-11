# CrateStack — `providers` through CrateStack: migration 0033 and D7 (2026-09-06)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### `providers` through CrateStack: migration 0033 and the D7 decision (2026-09-06)

`ConfigReconcile::reconcile`'s **provider** pass is
`upsert(CreateProviderInput).run_in_tx(tx, ctx)`, inside the same transaction
and after the same `pg_advisory_xact_lock` as the currency pass. The section
above records why it could not be, and this one records the decision that
changed that and exactly what the decision costs.

**The decision (D7, taken by the maintainer's delegate).** `cratestack-macros`
drops every `@default(...)` field from both `Create{Model}Input` and
`upsert_update_columns`, and `model Provider` carried one on all five
capability booleans because migration 0002's table did. The two ways out were
(1) drop the five `@default(...)` _and_ the five column `DEFAULT`s, or (2)
wait for upstream to grow a way to include a defaulted column in an upsert
input. **Option 1**, on the ground that `reconcile` is the _only_ writer of
those five columns and always writes all five from configuration — so a column
default can never help that writer and can only invent a capability for some
other writer that forgot one. A rail silently recorded as "does not refund",
or silently recorded as enabled, is worse than an `INSERT` that refuses.

**Migration 0033** (`0033_providers-drop-capability-defaults.sql`, count
32 → 33) is five `ALTER TABLE providers ALTER COLUMN … DROP DEFAULT` and one
`COMMENT ON TABLE`. It rewrites no rows (`DROP DEFAULT` touches `pg_attrdef`
only) and, unlike 0032, **is** backward compatible with the previous
release's binary: the pre-0033 `reconcile` names all eight columns in its
hand-written insert, and nothing else in the tree writes this table.

**What it costs, stated as a refusal rather than a difference.** All five
columns are `NOT NULL` and now have no default, so
`INSERT INTO providers (code, display_name, flow) VALUES (…)` typed at a psql
prompt fails with `23502` where it used to succeed and invent four `false`s
and a `true`.
`a_hand_written_provider_insert_must_now_name_every_capability_column`
(`postgres_smoke.rs`) asserts both halves — the refusal, and that an `INSERT`
naming all eight still succeeds — and reads `pg_attrdef` so that a migration
which dropped four defaults and missed one fails here rather than in
production. Three fixtures in that same file had to grow the columns in the
same commit; that is the whole blast radius in this repository.

**The drift did not move, and that is the measurement, not the expectation.**
84 changes / 16 relations / 17 unmappable before and after, and the
`providers` block is byte-identical. While the five `@default(...)` and the
five `DEFAULT`s were both there they agreed; with both gone they still agree.
What creates drift is doing _one_ half: with the five `@default(...)` removed
from the schema and 0033 absent, the report is **89 changes**, exactly five
`column … default value differs` lines on `providers` (measured here on
2026-09-06, reproducing exp17's review pass from the opposite side). That is
why the migration and the schema edit are one commit.

**A public API addition, and one classification deliberately changed.**
`CreateProviderInput::flow` is the schema's `ProviderFlow` enum rather than a
`String`, so `reconcile` parses `ProviderSeed::flow` before it builds a
statement. `ProviderSeed::flow` stays a `String` (Step 2's D4 is untouched),
and an unparseable label is the new `DbError::ProviderFlowUnknown` —
`Category::Configuration`, **exit 78**, where the `providers_flow_enum_check`
CHECK it replaces produced `DbError::Query` → `Category::Storage` → **exit
69**, i.e. "wait for Postgres" for a typo in a deployment. The CHECK is still
in the database and still fires for a writer that is not `reconcile`
(`an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`
is unchanged). The parse is a `match`, not `unwrap_or_default()`, because
`cratestack-macros` derives `Default` on every generated enum with the _first_
variant as the default — so `unwrap_or_default()` here would store a typo'd
rail as a push rail and return `Ok`.

**`model Provider` gained `@@allow("create", …)` and `@@allow("update", …)`**,
in the commit that moved the write and not before it, and still has no
`delete` arm: `reconcile` disables a dropped rail rather than removing one.

**No `find_unique(...).for_update()` ahead of the provider upsert, and that is
deliberate.** The currency pass needs its read because the upsert renders
`SET exponent = EXCLUDED.exponent` and a stored exponent must never be
overwritten — the read _is_ the guard. A provider has no such value: all eight
columns are owned by configuration and overwriting them is the point, so the
read would return a row nothing compares. The row lock it would take is taken
anyway, in the same transaction, by `upsert`'s own conflict probe
(`upsert_exec.rs::run_upsert_in_tx` → `select_for_update_by_conflict_target`
on `tx`). A measured consequence worth recording: because no `providers` row
lock is held when the upsert runs, swapping `run_in_tx` for `run` **fails in
1.2 s instead of hanging**, which is the opposite of what the same mutation
does to the currency pass.

**Tests: +3 net in `vpay-db`, +1 in `postgres_smoke.rs`, one replaced** (the
review pass below added more; see its own subsection for the counts)**.**
`the_provider_upsert_cannot_carry_the_capability_columns` became
`the_provider_upsert_carries_all_eight_columns` (same file, inverted claim).
New: `an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement`
(no container),
`a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile` and
`a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
(`vpay-db/tests/repositories.rs`), and
`a_hand_written_provider_insert_must_now_name_every_capability_column`
(`postgres_smoke.rs`).
`a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
had to change its expected variant from `DbError::Query` to
`DbError::Persistence(PersistenceError::Check { … })`, because the statement
that raises the `23514` is a CrateStack upsert now — the exact "a caller
matching the variant would have silently stopped matching" the module doc
warns about, caught by a test rather than in production.

**Gate, run on this branch on 2026-09-06 against the pinned 1.98.0 toolchain
and Node 22.23.2 from `.nvmrc`.** `just ci` end to end, **exit 0**, containers
included and `node_modules` installed with `pnpm install --frozen-lockfile`.
All **ten** `just verify` gates green, `check-schema` at cratestack 0.11.1
still 13 model/enum declarations (this change adds policies and removes
defaults, not declarations). `just test-rust`: **1386 tests run, 1386 passed,
0 skipped** (973 s). `just verify-ignored`: **0 ignored (expected 0), 43 test
binaries (expected 43), 1386 total** — 1382 plus this change's four, all in
files that already existed, so `expected_suites` and the 1080 floor are
untouched. `just test-doc`: 96 passed, 1 ignored (the `sdks/rust` README
block, pre-existing). `just deny`: `advisories ok, bans ok, licenses ok,
sources ok`. `lint-web` and `test-web` green.

Two earlier `just ci` runs on the same tree failed for reasons that were not
this change and are recorded rather than hidden: one on a
`failed to create a container: Timeout error` starting the MTN stub while
other suites were saturating the machine, and one on
`the_delivered_signature_verifies_with_the_shipping_node_sdk` with
`sh: 1: tsc: not found` — a worktree whose `node_modules` had not been
installed yet. Neither reproduced once the machine was quieter and the
lockfile install had run.

**`fn reconcile` is now 237 lines** (203 before), still the longest production
function on `verify-docs`' advisory list and still almost all comment. exp17's
review flagged the same thing at 203 and left it for a maintainer; this change
condensed the provider loop's comments once (the long-form argument lives in
migration 0033's header and in
[reference/vpay-db.md](../../reference/vpay-db.md#cratestack)) and did not split the
function, because the split would separate the advisory lock from the
statements it protects.

**Mutations run on 2026-09-06**, transcripts in
[docs/plans/exp20-provider-defaults-notes/opus.md](../../plans/exp20-provider-defaults-notes/opus.md):

| Mutation                                                     | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Provider upsert `.run_in_tx(&mut tx, &ctx)` → `.run(&ctx)`   | `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction` **FAILS in 1.226 s**, `left: 1, right: 0`, with the message naming `run_in_tx`. **Nothing hung** — the whole reconcile set finished, longest case 5.1 s. That is the difference from the currency pass, where the same mutation deadlocks against `find_unique(...).for_update()`                                                                                                                                                |
| Delete `@@allow("update", …)` from `model Provider`          | LOUD, and **only on the second boot**: `every_action_this_module_calls_has_an_allow_arm` fails in 3 ms with no container, and two container cases fail with `Provider: a model policy denied a system upsert: forbidden: update policy denied this upsert` — `reconcile_is_idempotent_…` at "a second, identical reconcile must succeed" and `a_rail_the_configuration_disables_…` at "reconciling a disabled rail must succeed". A fresh database's first boot succeeds, which is why the no-container slot test exists |
| Delete migration 0033, keep the schema edit                  | **Test-time**, three ways: `the_cstack_schema_drifts_…` fails at **89 vs 84** with five `column … default value differs` lines on `providers`; `a_hand_written_provider_insert_must_now_name_every_capability_column` fails because the three-column INSERT succeeds again; `schema_migrates_cleanly_on_an_empty_database` fails at 32 vs 33                                                                                                                                                                             |
| Restore one `@default(false)` in `model Provider`, keep 0033 | **Compile-time**: `error[E0560]: struct `inputs::CreateProviderInput`has no field named`supports_refunds``. The schema half cannot regress silently — the crate stops building before any test runs                                                                                                                                                                                                                                                                                                                      |

##### Review pass, 2026-09-06: one mutation the delivered change did not survive

The four mutations above were re-run and reproduce. A fifth, not run when the
change was written, **survived it**, and closing it is why this section has a
review subsection at all.

| Mutation                                                        | As delivered                                                                                                                                                                                                                                                             | After the review                                                                                                                                                                      |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `reconcile`'s flow parse `.map_err(…)` → `.unwrap_or_default()` | **SURVIVES.** `cargo nextest run -p vpay-db`: **112/112 passed**. `redirekt` is stored as a **push rail** — `cratestack-macros` marks the first variant of every generated enum `#[default]` and `ProviderFlow`'s first variant is `push` — and boot step 4 returns `Ok` | **FAILS in 1.27 s** on `a_flow_label_the_schema_cannot_name_is_refused_by_reconcile_before_any_row_is_written`, at `a flow that is neither ``push`` nor ``redirect`` must be refused` |

Nothing exercised `reconcile` with an unnameable flow.
`an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement`
constructs `DbError::ProviderFlowUnknown` **by hand** and asserts how it
classifies; it never calls `reconcile`, so it is green whatever `reconcile`
does with the label. The `unwrap_or_default()` trap that the code comment,
the variant's doc, this page and
[reference/vpay-db.md](../../reference/vpay-db.md#cratestack) all describe as
guarded was, until this test, guarded by prose only.

The new container test calls the public trait, asserts the variant, the
`Category::Configuration`/exit-78 classification, and that **neither** a
`providers` row nor the `currencies` row the pass before it had already
upserted survives — so it also pins that the refusal rolls the whole boot-step-4
transaction back.

**Two coverage gaps closed alongside it, both in the decisive direction the
task brief named.**

1. `a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile` asserted
   all eight columns after the disable and the repeat, but never the _reverse_:
   nothing in the tree turned `providers.enabled` from `false` back to `true`
   through `reconcile`. A fourth reconcile now does, asserting all eight
   columns again. Stated plainly, because it matters for how much this is
   worth: **no single-line mutation was found that this step alone kills** —
   every one tried is caught earlier by the first or second assertion. What it
   pins is a documented behaviour (`ProviderHost::enabled`'s doc and
   [flows/configuration.md](../../flows/configuration.md): turning a rail off in the
   YAML is not the same as deleting its block) that no test exercised.
2. Nothing asked what a `reconcile` that _waited_ on the advisory lock does
   with state the holder **committed** while it waited — which is the question
   the deliberately absent `find_unique(...).for_update()` on the provider
   pass raises. `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_
released` releases the lock by rollback, so the waiter always meets an
   empty table.
   `a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed`
   now holds the lock in one connection, writes a `providers` row that
   disagrees with the waiter's configuration on all eight columns, **commits**,
   and asserts the waiter's own configuration is what survives. The answer is
   that the provider pass needs no row lock of its own: the advisory lock
   serialises, and `upsert`'s conflict probe locks the committed row.

**A dependency this made visible, and it was not written down anywhere: boot
step 4 requires READ COMMITTED.** Adding `SET TRANSACTION ISOLATION LEVEL
REPEATABLE READ` after `reconcile`'s `pool.begin()` — a plausible "make boot
safer" edit — makes the new test FAIL in 4.2 s with `could not serialize
access due to concurrent update` (`40001`), because the snapshot is taken when
the `pg_advisory_xact_lock` statement starts, i.e. _before_ the holder
commits, so the conflict probe cannot see the row it must update. Measured
2026-09-06; **every other reconcile case passes under that mutation**,
`reconcile_waits_for_the_boot_lock_…` and
`two_concurrent_reconciles_…_converge` included. Recorded in
`config_reconcile.rs`'s comment on that loop and in
[reference/vpay-db.md](../../reference/vpay-db.md#cratestack).

**A second classification moved, and this page and the module doc both said it
had not.** `config_reconcile`'s "`# Errors` moved" paragraph claimed _the
classification is unchanged — a `23514` is still `Category::Internal` …
because `persistence::classify_cratestack` and `error::classify_write` are
asserted against each other_. Both halves are wrong. The two functions are
asserted against each other for `23505` and `23503` only
(`a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx`),
and on `23514` they **disagree by design**: `classify_write` deliberately
leaves a CHECK violation in the unclassified `DbError::Query` bucket →
`Category::Storage` → exit **69**, while `classify_cratestack` gives it
`PersistenceError::Check` → `Category::Internal` → exit **1**.

`partial_refunds_imply_refunds` is the only `23514` boot step 4 can raise, and
until the provider pass moved onto CrateStack it reached a supervisor as `69`,
"wait for Postgres", for an adapter whose declared `Capabilities` are
incoherent. It is `1` now. Measured 2026-09-06 and pinned by a new assertion
in `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`.

**Maintainer decision, surfaced not taken — and taken on 2026-09-10 (issue
#61).** The surfaced question was: `Internal`/`1` is defensible (the database
is healthy, and `Capabilities::is_coherent` was checked only in
`vpay-server`'s `#[cfg(test)]` assertion and the conformance suite, never at
boot), but `Category::Configuration`/`78` ("fix the deploy") is what the flow
label gets for the same class of mistake, and the two disagreed. **Decided:
boot checks first.** `boot_seeds` refuses an incoherent rail as
`ConfigError::IncoherentCapabilities` — `78` — before the reconcile, and the
CHECK stays exactly as it was, still `PersistenceError::Check` /
`Category::Internal` / `1` for any writer that reaches it. So the disagreement
is gone in the direction the issue chose, without weakening the last line:
both numbers are measured, in `main`'s order and against a real Postgres, by
`boot_refuses_an_incoherent_rail_as_78_before_the_check_can_answer_it_as_1`
(`backends/tests/integration/tests/boot_coherence.rs`), and
`a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
is unchanged. See the "Config guard rails" row above.

**Migration 0033 was only ever applied to an empty table.** Its header makes
three claims. The refusal it creates is asserted
(`a_hand_written_provider_insert_must_now_name_every_capability_column`, on a
database migrated from empty). Backward compatibility with the previous
release's binary is a fact about _code_ — the pre-0033 `reconcile` names all
eight columns in its hand-written `INSERT`, and nothing else in the tree writes
`providers`; both re-checked against `06e27f9` in the review, and no test can
execute a binary that is not in this checkout. The third, _"NO DATA CHANGES …
every existing row keeps exactly the capabilities it had"_, is the one an
upgrade depends on and nothing exercised it.
`migration_0033_changes_no_stored_capability_on_a_populated_table`
(`postgres_smoke.rs`) now does: it restores the five pre-0033 defaults,
asserts through `pg_attrdef` that the restore was real (so the test cannot go
vacuous), writes one row that takes all five defaults and one that contradicts
all five, applies **0033's own text** via `include_str!` rather than a copy of
its statements, and asserts both rows are byte-identical afterwards, that no
default survives, and that the three-column `INSERT` is refused on this
database too.

**Review pass test count: +3 test cases, no new test binary** (all three land
in files that already existed), so `expected_suites` and the `min_tests` floor
are untouched.

Full transcript, including the two mutations that were re-measured rather than
accepted and the exp18 merge check, in
[plans/exp20-provider-defaults-notes/opus-review.md](../../plans/exp20-provider-defaults-notes/opus-review.md).

**Gate on the review head (`137a75e`), 2026-09-06, rustc 1.98.0 and Node
22.23.2 from `.nvmrc` with `pnpm install --frozen-lockfile`:** `just ci` end to
end, **exit 0**. All ten `just verify` gates green (`check-schema` still 13
model/enum declarations at cratestack 0.11.1; `verify-links` 807 links in 147
files). `just test-rust`: **1389 tests run, 1389 passed, 0 skipped** (932 s) —
1386 plus the review's three. `just verify-ignored`: **0 ignored (expected 0),
43 test binaries (expected 43), 1389 total** (floor 1080). `just test-doc`: 96
passed, 1 ignored (the pre-existing `sdks/rust` README block). `just deny`:
`advisories ok, bans ok, licenses ok, sources ok`. `lint-web` and `test-web`
green.

An earlier full run on the delivered head failed on `provider_callback ::
case_1_mtn_momo` with `failed to create a container: Timeout error` at 120 s,
while an abandoned `vpay-demo` compose stack was crash-looping on this host
(`Restarting (69)`, its database gone) and the load average was above 22. Not
this change: every test it did reach passed, including the drift test at 84,
and the same suite is green above. Recorded rather than hidden.

**`fn reconcile` is 249 lines** on this head (203 before exp20, 237 as
delivered): the review added two comment blocks, one for the READ COMMITTED
dependency and one for the `23514` classification. Still the longest production
function on `verify-docs`' advisory list, still almost all comment, and still a
maintainer's call.
