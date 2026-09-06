# S2b review — sabotage pass over migration 0032 and the CrateStack currency pass

Adversarial review of `4385d0d..d52a0b2` (branch
`claude/exp17-cratestack-currencies-opus`), run on 2026-09-06 against
`postgres:16-alpine` under the pinned 1.98.0 toolchain, Node 22.23.2 from
`.nvmrc`, and `cratestack 0.11.1` on `PATH`. The implementer's account is
[opus.md](opus.md); this file is the independent measurement of it.

**Headline: every quantitative claim in `opus.md` reproduced exactly.** The
drift arithmetic, the four mutations, the upstream citations and the
`preview_sql` renderings were all re-derived here from scratch and none of
them moved. What follows is what the review found *on top* of that.

---

## 1. `just ci` as delivered, recipe by recipe

Run one recipe at a time so a failure could be attributed, containers up, no
`--no-fail-fast` masking:

| Recipe | Exit | Wall |
|---|---|---|
| `fmt-check` | 0 | 0.5 s |
| `clippy` | 0 | 48 s |
| `verify` | 0 | 11.5 s (ten gates + the advisory `verify-docs` report) |
| `test-rust` | 0 | 16 m 30 s — **1381 tests run: 1381 passed, 0 skipped** |
| `test-doc` | 0 | 4.8 s |
| `verify-ignored` | 0 | `0 ignored (expected 0), 43 test binaries (expected 43), 1381 total (minimum 1080)` |
| `lint-web` | 0 | — |
| `test-web` | 0 | — |
| `deny` | 0 | — |

`docs/status.md`'s claimed numbers (1381 total, 43 binaries, 0 ignored, ten
`verify` gates) are accurate.

**Re-run after the review's own commits**, same method, at
`974b72b`:

| Recipe | Exit | Wall |
|---|---|---|
| `fmt-check` | 0 | 0.6 s |
| `clippy` | 0 | 11.9 s |
| `verify` | 0 | 7.1 s |
| `test-rust` | 0 | 11 m 36 s — **1382 tests run: 1382 passed, 0 skipped** |
| `test-doc` | 0 | 5.1 s |
| `verify-ignored` | 0 | `0 ignored (expected 0), 43 test binaries (expected 43), 1382 total (minimum 1080)` |
| `lint-web` | 0 | 18.9 s |
| `test-web` | 0 | 7.7 s |
| `deny` | 0 | `advisories ok, bans ok, licenses ok, sources ok` |

The +1 is `a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
(finding 4). `expected_suites` and the ignored count are unmoved; the new
case lives in a file that already existed.

---

## 2. The upstream claims, checked against the vendored 0.11.1 sources

Every citation in `opus.md`, `schemas/vpay.cstack` and migration 0032 was
resolved to a line in `~/.cargo/registry/src/*/cratestack-{migrate,macros,sqlx}-0.11.1`.
All of them hold.

| Claim | Source | Verdict |
|---|---|---|
| The diff matches CHECKs by name first, then compares kinds; a kind change emits drop + add | `cratestack-migrate/src/diff/checks.rs:17-57` | ✔ |
| Introspection reports every validator-derived CHECK as `CheckKind::Raw`; only `Enum` is reconstructed | `introspect/postgres/constraints.rs:75-76`, `ir/checks.rs:60-74` | ✔ |
| The enum check is synthesised from `pg_enum` for a *native* enum column, under `check_name(table, column, "enum")` | `introspect/postgres/enums.rs:66-78` | ✔ |
| `resolve_column` projects `typtype == 'e'` and `text` onto the same `Scalar("String")` | `introspect/postgres/columns.rs:78-82` | ✔ |
| `int4` is deliberately unmapped, because `Int` emits `int8` | `introspect/postgres/types.rs:8-15`, and its own test `narrower_int_widths_are_unmapped_not_guessed` | ✔ |
| `@iso4217` renders `{c} ~ '^[A-Z]{3}$'`, `@range` renders `{c} >= {min} AND {c} <= {max}` | `emit/postgres/checks.rs:54-66` | ✔ |
| `<table>_<column>_<validator>_check`, with slugs `iso4217`/`range`/`length`/`enum` | `naming.rs:46-48`, `convert/checks.rs:38-52` | ✔ |
| Multi-column CHECKs are invisible to introspection | `introspect/postgres/constraints.rs:62` (`array_length(c.conkey, 1) = 1`) | ✔ |
| `Create{Model}Input` and `upsert_update_columns` both drop every `@default(...)` field | `cratestack-macros/src/model/inputs.rs:20-26`, `model/descriptor/columns.rs:92-101`, `shared/attrs.rs:91-93` | ✔ |
| `gate_update_policy` probes on `runtime.pool()` with **no** `FOR UPDATE`, so boot cannot deadlock against its own row lock | `cratestack-sqlx/src/query/write/upsert_resolve.rs:161-182`, `upsert_sql.rs:72-102` | ✔ |
| `run_in_tx` performs every write on `tx` and commits nothing itself | `cratestack-sqlx/src/query/write/upsert.rs:179-201`, `upsert_exec.rs:120-196` | ✔ |

A detail worth writing down because it is *why* the hand-written
`CHECK (flow IN ('push','redirect'))` is invisible to the report rather than
merely equivalent to the old enum: Postgres deparses it as
`flow = ANY (ARRAY['push'::text, 'redirect'::text])` (measured, § 4 below),
which is precisely the string `check_pattern.rs::reconstruct_enum` parses
back into `CheckKind::Enum { variants: ["push", "redirect"], list: false }` —
byte-identical to what `introspect_enum_checks` built from `pg_enum` before
the conversion.

---

## 3. The drift arithmetic, re-derived independently

Not by re-running the repository's test with constants edited, but by driving
`cratestack migrate baseline --strict` myself against four freshly built
databases (migrations applied with `psql`, plus a hand-created
`_sqlx_migrations` so the undeclared-table set matches the real run).

| Variant | Report |
|---|---|
| **A** — as delivered, all 32 migrations | **84 changes / 16 relations / 17 unmappable** |
| **B** — 0032 with the two CHECK renames reverted (widening + enum conversion kept) | **84 / 16 / 17** — only the *names* in the report change |
| **C** — 0032 with the enum conversion reverted (widening + renames kept) | **84 / 16 / 17** — the `providers` block is byte-identical |
| **D** — no 0032 at all | **85 / 16 / 18** |

So the claim is exact: the `INT → BIGINT` widening is the whole of the −1 and
the whole of the 18 → 17, and **neither the CHECK rename nor the native-enum
conversion moves the count by one**. Variant A reproduces the pinned
constants; variant D reproduces `opus.md`'s "before" block line for line.

A fifth variant, driven by finding 6 below:

| Variant | Report |
|---|---|
| **M5** — the five `@default(...)` removed from `model Provider`, DDL untouched | **89 / 16 / 17** — exactly five new `column … default value differs` lines on `providers` |

which confirms the sentence in `schemas/vpay.cstack` that removing the
defaults without the matching `DROP DEFAULT` migration is not an option.

---

## 4. Migration 0032 against a database that already has rows

The repository's own migration tests only ever apply to an empty database.
Applied 0001–0031 to a container, inserted three currencies and two rails
(one of them deliberately `enabled = false`), then applied 0032:

- exit 0, eight statements, no rewrite failure;
- `currencies.exponent` is `bigint`, values `0 / 2 / 3` unchanged;
- `providers.flow` is `text`, values `push` / `redirect` unchanged, and
  `orange_money` is still `enabled = false` — the conversion does not touch
  the capability columns or re-apply their defaults;
- `pg_get_constraintdef` for the two renamed `currencies` CHECKs is
  **byte-identical** to what it was before the rename
  (`CHECK ((code ~ '^[A-Z]{3}$'::text))` and
  `CHECK (((exponent >= 0) AND (exponent <= 4)))`), so the rename really is a
  rename;
- `providers_flow_enum_check` deparses to
  `CHECK ((flow = ANY (ARRAY['push'::text, 'redirect'::text])))`;
- `to_regtype('provider_flow')` is `NULL`, and nothing else in the tree
  references the type — `DROP TYPE` is safe.

---

## 5. Mutations

Every one applied to a clean tree, run, and reverted; `git status` clean
afterwards each time; final `git rev-parse HEAD` re-checked.

| # | Mutation | Measured result |
|---|---|---|
| 1a | Comment out 0032's `ALTER COLUMN flow TYPE TEXT` + `ADD CONSTRAINT` + `DROP TYPE` | `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` **FAILS** at the seed: `column "flow" is of type provider_flow but expression is of type text` |
| 1b | 1a + restore `$3::provider_flow` in `config_reconcile` | **FAILS** at the test's own raw read: `mismatched types; Rust type alloc::string::String (as SQL type TEXT) is not compatible with SQL type provider_flow` |
| 1c | 1b + cast the test's raw read to `flow::TEXT` | **FAILS** at the CrateStack read: `the CrateStack provider read failed: database: error occurred while decoding column "flow": mismatched types; Rust type alloc::string::String (as SQL type TEXT) is not compatible with SQL type provider_flow`. Reproduces `opus.md` § 2 verbatim |
| 2 | Delete `.for_update()` from the currency read | `reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer` **FAILS 10 runs out of 10** (4.16–4.42 s each), always at `expect_err` — the upsert's own probe blocks instead, then overwrites the committed 3 with 0. Unmutated, the same test **passes 10 out of 10**. Whole crate under the mutation, `--no-fail-fast`: **108 tests run, 107 passed, 1 failed** — the new test is the only thing in `vpay-db` that catches it, exactly as claimed |
| 3 | Delete `@@allow("create", …)` from `model Currency` | `every_action_this_module_calls_has_an_allow_arm` **FAILS in 4 ms with no container**, naming the consequence; three container reconcile cases fail with `Currency: a model policy denied a system upsert: forbidden: create policy denied this upsert` |
| 4 | Delete `CONSTRAINT partial_refunds_imply_refunds` from migration 0002 | `partial_refunds_without_refunds_is_rejected_by_the_database` **FAILS** (`rows_affected: 1`). `the_cstack_schema_drifts_…` also fails — **but at line 1403, its own `pg_constraint` exact-set assertion, and the header still reads `84 change(s) total`.** The drift count is unmoved, as claimed |
| 5 | *(added by this review)* Remove the five `@default(...)` from `model Provider` | **The crate stops compiling** — `E0063: missing fields delivers_callbacks, enabled, requires_ip_allowlist and 2 other fields in initializer of CreateProviderInput` at `config_reconcile.rs:501`. With the literal completed, `the_provider_upsert_cannot_carry_the_capability_columns` **FAILS**, printing the eight-column statement the docs predict. See finding 6 |
| 6 | *(added by this review)* Swap the currency `upsert(...).run_in_tx(&mut tx, &ctx)` for `.run(&ctx)` | Before this review, nothing **failed** — `reconcile_is_idempotent_and_disables_a_dropped_provider_code` **hung** instead: `SLOW [>480.000s]` and still going when the run was killed, because `upsert`'s conflict probe is itself `SELECT … FOR UPDATE` and off the transaction it waits on the row the transaction holds. `a_hand_seeded_currency_…` passed (it never reaches the upsert). The new test is red in 1.2 s. See finding 5 |

---

## 6. Findings

### 1 — misleading-claim: a comment cites a test that does not exist

`config_reconcile.rs`'s provider loop said a bad `flow` "is refused by the
CHECK exactly as the enum refused it, as
`an_unknown_provider_flow_is_refused_by_the_database` proves". There is no
test by that name anywhere in the tree; the test is
`an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`,
and this change is the commit that wrote it. In a repository whose discipline
is "point at the case that proves it", a citation to a test that does not
exist is worse than no citation. Nothing gates this — `verify-status` lexes
`NotImplemented` tokens, not test names.

### 2 — misleading-claim: the repository *does* ship a prettier configuration

`opus.md` § 6 and `docs/status.md` both say "this repository ships no prettier
configuration file". It ships `.prettierignore`, added in Step 6, whose own
header comment describes exactly this failure mode for
`deploy/helm/**/templates/` and whose established remedy is an ignore entry.
That matters because it changes what is being left to the maintainer: not
"decide whether to introduce prettier configuration", but "decide whether the
existing `.prettierignore` should grow an entry for a fixture that is
deliberately unparseable, and separately what to do about the 222 files
prettier's defaults would rewrite".

The hazard itself is real and was re-measured read-only:
`pnpm exec prettier --list-different .` reports **222** files, and
`--check` errors on
`backends/crates/vpay-config/tests/fixtures/malformed.yml`. `just ci` is
genuinely unaffected (it runs `fmt-check`, never `fmt`). Declining to fix it
here is the right call; the description of the starting point was wrong.

### 3 — correctness (operational): 0032 breaks the previous release's binary, and nothing said so

Measured on the populated database from § 4. After 0032 applies:

- the pre-0032 binary's boot-step-4 insert
  (`… VALUES ($1, $2, $3::provider_flow, …)`) fails with
  `ERROR: type "provider_flow" does not exist` (SQLSTATE `42704`);
- its `i32` read of `currencies.exponent` fails to decode against `int8`.

Migrations here are forward-only and both binaries run `run_migrations()` then
`reconcile` at boot, so in a rolling deploy — or in a rollback to the previous
image — any old-version process that restarts after 0032 has landed
crash-loops at boot step 4. That is a real operational cost of this migration
and it appeared in none of the migration header, `docs/status.md`, or
`docs/flows/configuration.md`, all of which say what the migration costs in
other respects at length. Recorded now; not "fixed", because forward-only
migrations are this repository's existing choice and a two-release
expand/contract sequence is a maintainer's decision, not a reviewer's.

### 4 — gate-hole: the "one transaction" invariant is now half-delegated and untested in the direction that matters

`config_reconcile`'s module doc leads with "Every statement below runs in one
transaction, so a failure part-way through leaves the tables exactly as they
were." Half of those statements now belong to an external crate.
`a_hand_seeded_currency_exponent_is_read_back_and_refused_not_overwritten`
proves the *other* direction (a currency refusal leaves no provider behind),
but nothing proved that a CrateStack write, once made, is rolled back by
vpay's own rollback — the case where a later raw-sqlx statement fails after
the currency upsert has already landed.

Reading `upsert.rs`/`upsert_exec.rs` says it must be (§ 2), but that is
exactly the kind of "true by reading the code" claim this repository does not
accept. Mutation 6 shows the gap was real, and shows something the module
comment did not say at all.

Swapping `run_in_tx(&mut tx, &ctx)` for `run(&ctx)` — a one-word edit, and
the only difference between joining vpay's transaction and opening a private
one — **failed nothing. It hung.** `upsert`'s own conflict probe is
`SELECT … FOR UPDATE`
(`cratestack-sqlx/src/query/write/upsert_sql.rs:24-60`), executed on
`&mut **tx` inside `run_upsert_in_tx`. Off the transaction it waits for the
row lock `find_unique(...).for_update()` is holding, and that transaction is
waiting for it: a self-deadlock. `reconcile_is_idempotent_and_disables_a_
dropped_provider_code` reported `SLOW [>480.000s]` and was still reporting it
when the run was killed. In a deployment that is a boot that never returns
and never logs an error.

So `.for_update()` and `run_in_tx` are **coupled**, and the module comment
argued only about `gate_update_policy`'s policy probe (which is a *different*
query, and genuinely has no `FOR UPDATE`). The two probes were being treated
as one. A hang is also the worst available signal — worse than a red test —
so the fix is both a comment that names the coupling and a test that converts
the hang into a 1.2-second assertion failure. The new case stays fast under
the mutation because its currency is an INSERT: the conflict probe finds no
row, locks nothing, and blocks on nothing.

### 5 — nit: the decisive assertion messages are damaged

Four messages in
`reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer`
carry 10–18-space runs from a line-join that lost its `\` continuation. They
are not cosmetic in context: they are the strings the row-lock mutation
prints, and mutation 2 above shows them in the failure output.

### 6 — the provider pin is *more* decisive than claimed (no defect; worth recording)

`the_provider_upsert_cannot_carry_the_capability_columns` is described as a
`preview_sql` pin that "an upstream fix turns red". It is stronger than that:
because `Create{Model}Input`'s *fields* are what the `@default(...)` filter
removes, the day those five columns become settable the crate **stops
compiling** at `config_reconcile.rs`'s struct literal, before any test runs.
Measured (mutation 5). And with the literal completed, the generated
statement is exactly the eight-column form `docs/reference/vpay-db.md`
predicts:

```text
INSERT INTO providers (code, display_name, flow, supports_refunds, supports_partial_refunds,
                       delivers_callbacks, requires_ip_allowlist, enabled)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow,
    supports_refunds = EXCLUDED.supports_refunds, … , enabled = EXCLUDED.enabled
```

So the maintainer's option 1 (drop the five `@default`s *and* the five DB
defaults) is not a guess — it demonstrably unblocks the provider upsert, at
the demonstrated cost of five drift lines if the DDL half is skipped.

One weakness worth naming rather than fixing: the test's first assertion is a
`starts_with` on the whole rendered prefix, so a harmless generator change
(different quoting, a reordered `RETURNING`) turns it red too. That is a loud
false alarm rather than a silent false green — the message prints the SQL —
and tightening it would trade a decisive assertion for a fuzzy one.

### 7 — nit: an off-by-one in prose

`schemas/vpay.cstack`'s new GAP note says "The four defaults are here because
the live table has them (migration 0002: `DEFAULT FALSE` x4, `DEFAULT TRUE`)"
— that is five, and the rest of the same paragraph says five.

---

## 7. What was checked and found sound

- **No test or assertion was deleted or weakened.** `git diff 4385d0d..HEAD`
  removes no `#[test]`, no `assert`, and no test function; the only removed
  error branch (`boot_seeds`' `i32::try_from`) is unreachable by type, and
  `Config::validate_all` refuses any exponent that does not *equal* the
  canonical `vpay_core::Currency::exponent`, so no input could ever have
  reached it. Both spellings of the range bound are still enforced
  (`Config::validate_all`, then `currencies_exponent_range_check`).
- **`DROP TYPE provider_flow` is safe.** The type is named by exactly two
  files in the tree (0002 creates it, 0032 drops it) plus prose; no
  `sqlx::Type` derive, no other migration, no view, no function.
- **Nothing else depends on `currencies.exponent`'s width.** The four foreign
  keys into `currencies` reference `code`; there are no views.
- **The public-API widening is complete.** Every `CurrencySeed` construction
  and every read of `currencies.exponent` in the tree moved to `i64`; sqlx
  refuses the narrowing rather than performing it, so a missed one is a
  failure and not a truncation.
- **Boot cannot deadlock against its own row lock**, for the reason
  `opus.md` § 4 gives, verified in the 0.11.1 sources (§ 2).
- **The row-lock test is deterministic**, 10/10 both ways (mutation 2). Its
  3-second window is used only to assert that reconcile has *not* finished,
  which is the safe direction: under the mutation the test fails at the
  later `expect_err` rather than at the window, so a slow machine cannot
  turn a real regression green.
- **`docs/status.md`, `docs/reference/vpay-db.md` and
  `docs/flows/configuration.md`** all carry the two decisions in prose, name
  the tests, and cite the upstream files; the "zero drift gain" statements
  are cited and, per § 3, correct.

## 8. What was fixed, and what was deliberately left

Seven commits on top of `d52a0b2`, one per finding:

| Commit | Finding | Proof |
|---|---|---|
| `0b4c741` | — | this file |
| `85ecb8f` | 1 | every backticked identifier long enough to be a test name in `config_reconcile.rs` resolved against `fn <name>` across `backends/`; the only non-match is a constraint name |
| `5ac1a79` | 4 | new test PASS 1.44 s; FAIL 1.23 s under `run(&ctx)`, message naming the cause |
| `07b82a9` | 3 | the 42704 and the `int8` decode failure, measured on a populated database; both migration tests re-run green after the comment edit |
| `d04d78c` | 5, 7 | the repaired message re-printed by re-applying the `.for_update()` mutation; `check-schema` green |
| `eb4927b` | 2 | `.prettierignore` exists (Step 6, `7d62751`); `prettier --list-different .` = 222, measured read-only |
| `1a3c638` | 6 | `E0063` at the struct literal; the eight-column `preview_sql`; drift 84 → 89 |
| `974b72b` | — | `verify-ignored` 1382 |

**Left alone deliberately:**

- The `just fmt` recipe. It is unrelated to S2b, the 222-file half is a real
  decision, and the implementer was right to decline it. Only the description
  was wrong.
- The expand/contract question on 0032 (finding 3). Forward-only migrations
  are this repository's existing choice; changing it is not a reviewer's
  call.
- The provider `@default` question. Both options are now measured rather
  than argued, which is the most a review should do to a decision explicitly
  reserved for a maintainer.
- One pre-existing instance of finding 5's whitespace defect at
  `repositories.rs:3435` (Step 8's checkout-session address assertion),
  outside this change's diff.
- `fn reconcile` is now 203 lines and the longest production function in the
  repository on `verify-docs`' advisory list, almost all of it comment. Not
  trimmed: every paragraph is explaining *why* rather than restating the line
  below, which is what ADR-0016 standard 6 leaves to review, and the same
  reasoning is linked rather than only duplicated. Flagged so a maintainer
  can disagree.

## 9. Not checked

- Anything about a Kubernetes cluster, `helm-check` (network) or
  `docs-check-citations` (network) — neither is in `just ci`.
- Cypress / the e2e compose stack.
- The other six native enums; no migration touches them.
- Whether `evaluate_create_policies` issues literally zero queries for
  `auth().isSystem()`. It runs on the pool either way and cannot block on the
  row lock, which is the part that matters; the exact connection arithmetic in
  a comment was not re-derived.
- Any behaviour of `cratestack` 0.11.1 beyond the files cited in § 2.
