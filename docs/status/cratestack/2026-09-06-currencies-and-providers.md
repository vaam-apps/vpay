# CrateStack — `currencies` and `providers`: migration 0032 (2026-09-06)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### `currencies` and `providers`: migration 0032 and the first native-enum conversion (2026-09-06)

**What moved.** `ConfigReconcile::reconcile`'s **currency** pass runs through
CrateStack: `find_unique(code).for_update().run_in_tx(tx, ctx)` then
`upsert(CreateCurrencyInput).run_in_tx(tx, ctx)`, both inside the transaction
`reconcile` opens, after the same `pg_advisory_xact_lock`, in the same sorted
order. This is the first use of `run_in_tx` anywhere in vpay — the paragraph
above that says "the transaction seam is still unexercised" was true until
this change and is not now. It is also the first exercise of
`PersistenceError`'s `Denied` arm against a real database rather than a
synthetic error (mutation 3 below). `model Currency` gains
`@@allow("read"/"create"/"update", auth().isSystem())` and `model Provider`
gains `@@allow("read", …)`.

**Migration 0032** (`0032_currencies-providers-cratestack-shape.sql`, count
31 → 32) does three things: `currencies.exponent` `INT` → `BIGINT`; the two
hand-named `currencies` CHECKs renamed to the generator's
`<table>_<column>_<validator>_check` spelling with the generator's own
predicates; and `providers.flow` converted from the native `provider_flow`
enum to `TEXT` + `providers_flow_enum_check`, with `DROP TYPE provider_flow`.
`providers.partial_refunds_imply_refunds` is untouched, deliberately: it is
multi-column, invisible to `migrate baseline` in both directions, and guarded
only by `partial_refunds_without_refunds_is_rejected_by_the_database`.

**0032 is not backward compatible with the previous release's binary, and
that is an operational cost this section owes an operator.** It is the first
migration here to alter a column _type_ that shipping code binds. Measured on
2026-09-06 against a database that already had rows (the review pass; the
repository's own migration tests only ever apply to an empty one): after 0032,
the pre-0032 binary's boot-step-4 insert fails with `type "provider_flow" does
not exist` (SQLSTATE `42704`), and its `i32` read of `currencies.exponent`
fails to decode against `int8` — sqlx refuses the narrowing. Migrations here
are forward-only and both binaries migrate-then-reconcile at boot, so in a
rolling deploy, or after a rollback to the previous image, any old-version
process that restarts once 0032 has landed **crash-loops at boot step 4**.
Ship 0032 with the release that carries the matching code and do not roll that
release back past it. Turning this into an expand/contract pair across two
releases would remove the constraint; that was **not** done and is a
maintainer's decision, not the reviewer's — see
[docs/plans/exp17-notes/opus-review.md](../../plans/exp17-notes/opus-review.md),
finding 3. The rows themselves are safe: the same measurement confirmed every
`currencies.exponent` and `providers.flow` value survives, and a rail already
`enabled = false` stays disabled.

**`providers.flow` is the first of vpay's seven native enums to be
converted.** The other six — `intent_status`, `charge_state`, `failure_code`,
`account_kind`, `direction`, and `payment_intents.last_payment_error_code` —
are on tables no CrateStack query touches, and each needs the same treatment
before one can. CrateStack's generated row decoders read an enum column with
`try_get::<String>()`, so a native enum column fails to decode on **every**
read through that layer.

**Not done in this change, named, and the reason was measured rather than
pending — SUPERSEDED the same day by migration 0033; see "`providers` through
CrateStack" below.** `reconcile`'s **provider** pass was still a hand-written
`INSERT ... ON CONFLICT` when this section was written, and could not move
while the defaults were there. `cratestack-macros` drops every
`@default(...)` field from both `Create{Model}Input` and
`upsert_update_columns`, and `model Provider` carried one on all five
capability booleans because the live table did. The generated statement was

```text
INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
```

— `supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist` and `enabled` are in neither list. Shipping that would
insert every rail with the column defaults regardless of what the deployment
configured, and would never carry a capability change to an existing row: a
rail an operator had just disabled would come back enabled. That is a
plausible-looking success storing the wrong value, so it was not shipped.
`the_provider_upsert_cannot_carry_the_capability_columns` pinned the rendered
statement so that a fix would turn it red; it is
`the_provider_upsert_carries_all_eight_columns` now, asserting the inverse.

**A maintainer's decision, surfaced rather than taken — and taken the same
day.** Unblocking it on vpay's side meant removing the five `@default(...)`s
from `model Provider` _and_ `ALTER TABLE providers ALTER COLUMN ... DROP
DEFAULT` on all five — letting a code generator's input-shaping rule decide
vpay's DDL, and removing a defaulting behaviour any future writer of that
table would expect. The alternative was upstream growing a way to include a
defaulted column in an upsert input. The maintainer's decision (**D7**,
2026-09-06) was the first, on the ground that `reconcile` is the only writer
of those columns and always writes all five, so the default could only ever
invent a capability for a writer that had forgotten one. Migration 0033 is
that decision; see "`providers` through CrateStack" below.

**Also not done in _this_ change:** no other table moved; `reconcile` still
owns its own transaction and that did not change; the provider _disable_ pass
(`UPDATE providers SET enabled = false WHERE code <> ALL($1)`) is still raw
sqlx **and still is** after 0033, because it addresses rows by their absence
from a list and no generated builder expresses that;
`providers.code_length` and `providers.display_name_length` are still
hand-named (the rename would have bought zero drift and `display_name` has no
`@db_enforce` to converge on); and no production path _reads_ `providers`
through CrateStack — `model Provider`'s `read` policy still exists for one
test, and 0033 changed the write, not the read.

**Public API changed.** `CurrencySeed::exponent` and
`DbError::CurrencyExponentConflict`'s `stored`/`seeded` are `i64` rather than
`i32`, because the column is `BIGINT`. `vpay_api::v1::boot::boot_seeds` no
longer returns `ConfigError::Validation` at all: the "exponent does not fit
the column" arm became unreachable _by type_ (every `u32` fits an `i64`), so
the fallible conversion was replaced with `i64::from` rather than left as an
error branch nothing could take.

**Gate, run on this branch on 2026-09-06 against the pinned 1.98.0
toolchain.** `just ci` end to end, exit 0, with containers and with
`node_modules` installed from the pinned lockfile. All **ten** `just verify`
gates green — including `check-schema` at cratestack 0.11.1, still 13
model/enum declarations (this change adds policies, not declarations).
`just verify-ignored`: **0 ignored (expected 0), 43 test binaries (expected
43), 1382 total** — master's 1373 plus this change's eight and the review
pass's one, every one of them in a file that already existed, so
`expected_suites` stays 43 and the 1080 floor is untouched. (The whole gate
was re-run recipe by recipe in the review pass at the delivered commit and
was green there too, at 1381: `test-rust` 16 m 30 s, **1381 passed, 0
skipped**.) `just test-doc` **96 passed, 1 ignored** (the ignored one
is `sdks/rust`'s README block and is pre-existing). `just deny`:
`advisories ok, bans ok, licenses ok, sources ok`. `just fmt-check` and
`just clippy` clean.

The eight: four `vpay-db` unit tests in `config_reconcile` (two `preview_sql`
pins, the policy-slot assertion, and the container-backed provider parity
read), two container cases in `vpay-db/tests/repositories.rs`, and two in
`postgres_smoke.rs`.

The ninth is the review pass's:
`a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`,
also in `vpay-db/tests/repositories.rs`. It closes the one gap the mutations
above did not cover — that the CrateStack currency write is inside _vpay's_
transaction. Swapping that one `run_in_tx(&mut tx, &ctx)` for `run(&ctx)`
did not fail the suite, it **hung** it: `upsert`'s own conflict probe is
`SELECT … FOR UPDATE`, so off the transaction it waits on the row lock
`find_unique(...).for_update()` is holding while that transaction waits on
it, and `reconcile_is_idempotent_and_disables_a_dropped_provider_code`
reported `SLOW [>480.000s]` until the run was killed. In a deployment that
is a boot that never returns. `.for_update()` and `run_in_tx` are therefore
coupled, which the comment on that loop had not said — it argued only about
`gate_update_policy`'s policy probe, a different query that genuinely has no
`FOR UPDATE`.

**A hazard found while running the gate, recorded and not fixed:** `just fmt`
is `cargo fmt --all` _then_ `pnpm exec prettier --write .`. Running it
rewrites **222** tracked files with prettier's defaults (measured read-only
with `prettier --list-different .` in the review pass) and then **fails** on
`backends/crates/vpay-config/tests/fixtures/malformed.yml`, which is
deliberately malformed YAML — so the recipe leaves the tree reformatted _and_
reports failure. `just ci` is unaffected: it runs `fmt-check`
(`cargo fmt --all -- --check`), never `fmt`. This is a trap for the "Before
you open a PR" instruction in `AGENTS.md`, not a broken gate.

This entry said "this repository ships no prettier configuration" until the
review pass corrected it. It ships `.prettierignore` (Step 6), whose own
header comment describes exactly this failure mode for
`deploy/helm/**/templates/` and whose established remedy is an ignore entry.
That changes what is being left to the maintainer: not "introduce prettier
configuration", but (a) whether `.prettierignore` should grow an entry for a
fixture that is _deliberately_ unparseable, which is the pattern already in
that file, and (b) separately, what to do about the 222 files prettier's
defaults would rewrite — which an ignore entry does not address and a
`.prettierrc` or a narrower glob would.
[docs/plans/exp17-notes/opus.md](../../plans/exp17-notes/opus.md) § 6 has the
transcript and
[opus-review.md](../../plans/exp17-notes/opus-review.md) finding 2 the correction.

**Mutations run on 2026-09-06**, transcripts in
[docs/plans/exp17-notes/opus.md](../../plans/exp17-notes/opus.md):

| Mutation                                                                                                                                  | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Revert `ALTER COLUMN flow TYPE TEXT` in 0032 (and restore the `::provider_flow` bind cast, so the _read_ is the only thing that can fail) | `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` FAILS: `the CrateStack provider read failed: database: error occurred while decoding column "flow": mismatched types; Rust type` `alloc::string::String` `(as SQL type TEXT) is not compatible with SQL type provider_flow`. Pins the native-enum finding to a message rather than a paragraph                                                                                         |
| Delete `.for_update()` from the currency read                                                                                             | `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` still **PASSES** — the advisory lock, not the row lock, is what serialises boot against boot. **And so did every other test in the repository** (103/103 in `vpay-db`), which is why this change adds `reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer`: it is red under this mutation, with a concurrent writer's committed 3 clobbered back to 0 |
| Delete `@@allow("create", …)` from `model Currency`                                                                                       | LOUD, on every boot: `Currency: a model policy denied a system upsert: forbidden: create policy denied this upsert` → `PersistenceError::Denied` → `Category::Internal`. `every_action_this_module_calls_has_an_allow_arm` catches it in 5 ms with no container                                                                                                                                                                                              |
| Delete `@@allow("read", …)` from `model Currency`                                                                                         | SILENT at runtime and the dangerous direction. Three container tests go red — the two exponent-conflict cases and the row-lock case — because the read answers `None` for a row that exists and the upsert overwrites the stored exponent instead of refusing to. The no-container policy-slot test also catches it                                                                                                                                          |
| Delete `CONSTRAINT partial_refunds_imply_refunds` from 0002                                                                               | `partial_refunds_without_refunds_is_rejected_by_the_database` FAILS, **and the drift count does not move**: the report still says `drift detected in 16 table(s)/view(s) (84 change(s) total)`. The drift test fails only on its own `pg_constraint` read. Re-confirms the multi-column blind spot on this branch's numbers                                                                                                                                  |
