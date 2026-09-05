# exp13 (opus): sabotage review of the `migrate baseline --strict` drift test

**Date:** 2026-09-05 · **Branch:** `claude/exp13-baseline-drift-opus` ·
**Under review:** `70ca833` (`d086084..70ca833`) · **Implementer's account:**
[opus.md](opus.md) · **Reviewer's fixes:** `7a0255d`, `fabceca`, `2b1fe64`,
`331be79`, `d16eca2`, `0c5ce1d`.

Every number below was re-measured on this host, not read out of the account
under review.

## Verdict

**Safe as delivered — the central claims are true and independently
reproduced.** The 86, the 17 relations, the eleven undeclared tables, the ten
multi-column CHECKs and the "none of them is reported" finding all hold; the
decisive mutation (clean deletion of `CONSTRAINT no_over_refund`) does fail
both tests, for the reason claimed. Six defects were found and fixed, none of
them a false green: one leak, one stale-by-construction log line, two
brittle matches, one claim asserted in prose but not in code, and one finding
recorded as a measurement when it is CrateStack's own documented gap.

## Phase 1(a) — the gates, re-run

| Gate | Result |
|---|---|
| the test by name | **PASS**, 44.6s cold / 1.4–15s warm (one prior attempt died at 120s on a container-start timeout under load; passed on retry) |
| `just test-rust` | 1271 tests, 0 skipped. **One unrelated pre-existing timeout under load**: `vpay-server::cli an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so`, `CreateContainer(RequestTimeoutError)` at 120s — the same load-induced failure the account under review reports, in a test this branch does not touch. Final numbers below. |
| `just verify` | ok — the ten gates pass |
| `just docs-check` | ok — 1 unimplemented item, 712 links resolve |
| `just fmt-check` | ok |
| `just clippy` | ok |
| `just verify-ignored` | **0 ignored (expected 0), 42 test binaries (expected 42), 1271 total (floor 1080)** — master's 1270 plus one, as claimed, and no `expected_suites` bump was needed |

## Phase 1(b) — the measurement, reproduced independently

Not by running the test: a separate `postgres:16-alpine` container, the 30
migrations applied with `psql` file by file, and the CLI driven by hand.

```
drift detected in 16 table(s)/view(s) (85 change(s) total):
Error: migrate baseline: --strict refuses to baseline with 85 pending drift change(s); …
```

**85/16 by hand against 86/17 in the test, and the difference is exactly
`_sqlx_migrations`** — the one table `psql` does not create and `sqlx::migrate!`
does. That is the corroboration: the same database built two ways differs by
the one row the two ways differ by.

Category breakdown of the 85 (the 86th is the `_sqlx_migrations` table line):

| Category | Changes | Severity | Count |
|---|---|---|---|
| CHECK | 34 | `[safe]` | 46 |
| column | 27 | `[lossy]` | 28 |
| table | 10 | `[blocking]` | 11 |
| index | 7 | | |
| foreign key | 7 | | |

How the report represents each kind, verified against `cratestack-cli`
0.11.1's `src/migrate/drift_report.rs::describe` as well as observed:

- **Tables absent from the schema** — ``[lossy] table `x` exists in the live
  database but is not declared in the schema``. `Op::DropTable` is
  unconditionally `Lossy` (`cratestack-migrate` 0.11.1, `src/ir.rs`).
- **Extra columns** — ``column `x` exists in the live database but is not
  declared in the schema``; the reverse direction says "is declared in the
  schema but does not exist in the live database".
- **Enum vs TEXT** — ``column `status` type differs (live: Scalar("String"),
  schema: Enum("IntentStatus"))``. Upstream documents why: "An introspected
  enum-backed column always reports `ColumnType::Scalar("String")` … because
  the `.cstack`-side enum name has no catalog trace to recover it from".
- **Indexes** — by name, both directions. Expression and partial indexes are
  skipped upstream, "not guessed at".
- **Foreign keys** — every one appears as schema-only, because **foreign keys
  are not introspected at all** (`introspect/postgres/mod.rs`: "Neither the
  issue nor the design doc's §5.2 query list mentions `pg_constraint`'s
  `contype = 'f'` rows"). Seven of the 86 are that gap, not vpay's.
- **CHECK constraints** — the finding below.

## The CHECK finding, stated plainly

**`migrate baseline` does not model multi-column CHECK constraints at all, and
CrateStack says so itself.** `cratestack-migrate-0.11.1/src/introspect/
postgres/mod.rs`, "Known gaps": *"Multi-column and zero-column CHECK
constraints are skipped. `crate::ir::AddCheck` ties to exactly one column —
there's no IR shape for `CHECK (a < b)` — so `constraints::introspect_checks`
only considers `contype = 'c'` rows with `array_length(conkey, 1) = 1`;
anything else is silently absent from the result rather than mis-attributed to
one of its columns."* `constraints.rs` carries that filter verbatim.

So the brief's expectation — that `partial_refunds_imply_refunds` and
`no_over_refund` would appear in the report as things the schema cannot
express — **cannot hold**, and the implementer was right to report the
negative rather than manufacture the positive. Read straight out of
`pg_constraint` on my own container, the counts are:

| `public` CHECKs by column count | Number |
|---|---|
| 1 column (introspected) | 65 |
| 2 columns (skipped) | 7 |
| 3 columns (skipped) | 1 |
| 4 columns (skipped) | 2 |

Ten skipped, and they are exactly the ten the test pins. No multi-column CHECK
exists in the `authkestra` schema, and no zero-column CHECK exists on a
`public` table (the only two in the database, `cardinal_number_domain_check`
and `yes_or_no_check`, are `information_schema` domain constraints).

**The consequence for this repository is the one the account states**: a green
`--strict` run would say nothing about the over-refund guard or the
refund-capability rule. **The ask to CrateStack is bigger than the `@@check`
one already recorded**: grammar alone would not fix it, because there is no IR
op a cross-column CHECK could occupy. `@@check(expr)` **and** a non-single-
column IR op **and** introspection that reads `array_length(conkey, 1) > 1`.
And the narrower, cheaper ask worth making alongside it: `--strict` reports
success without saying what it declined to look at — the trailing
"N column(s) … review manually" block has no CHECK equivalent. Recorded in
`docs/status.md` and `schemas/vpay.cstack`'s header by `0c5ce1d`.

## Phase 1(e) — mutations, all re-run by the reviewer

| # | Mutation | Expected | Measured |
|---|---|---|---|
| 1 | `CONSTRAINT no_over_refund` deleted from migration 0003, **cleanly** (the line *and* the trailing comma on the preceding constraint, so the migration still applies) | both tests red | **both red.** `over_refund_is_rejected_by_the_database`: "amount_refunded + amount_refund_pending > amount must be rejected: PgQueryResult { rows_affected: 1 }". The drift test: the `pg_constraint` set assertion, ten entries vs nine. The report still said **86** — the count cannot see it, exactly as claimed. |
| 2a | `model DisabledClient` with `@default(dbgenerated())` added to the schema | count moves | **count did NOT move (86).** The table's "not declared" line was swapped for ``[safe] column `disabled_at` default value differs``. Caught by the exact-table-set assertion, not by the count. The implementer's account of this is accurate and is the reason the set is asserted. |
| 2b | the same model with `@default(now())`, matching the column's real default | count drops | **85 / 16 relations.** Count assertion fires: "the report counts 85 pending change(s), this test pins 86". |
| 3 | `EXPECTED_DRIFT_CHANGES` 86 → 87 | red | **red**, "left: 86, right: 87" |
| 4 | `--strict` removed from the invocation | must not write into the repository | **red** — on the exit-code assertion (`Some(0)` vs `Some(1)`), before the no-writes assertions are reached. It is still loud, and the repository is still untouched (`--out-dir` is outside the checkout). But the run *did* write a snapshot into the out-dir, and pre-fix that directory leaked; see finding 1. |
| 5 | a `cratestack` shim printing `9.9.9-review-fake` | passes, prints it | **passed and printed it — and, pre-fix, said nothing else.** See finding 2. |
| 6 | `cratestack` removed from `PATH` (running the test binary directly; `cargo` re-adds `~/.cargo/bin` to a child's `PATH`, so a `PATH=` prefix on the `nextest` command does **not** test this) | red, clear message, no skip | **red in 0.00s**, "the `cratestack` CLI must be on PATH for this test — it is a red failure, not a skip …". No `#[ignore]`, no early return. |
| 7 | `CREATE TABLE cratestack_migrations` appended to migration 0030 | ? | count unmoved at 86, table set unchanged, **nothing noticed** pre-fix. See finding 5. |

## Findings and fixes

1. **[robustness → fixed `7a0255d`] The out-dir leaked on every failing run.**
   `remove_dir_all` was the last statement, so every `?` and every failed
   assertion skipped it. Seven orphaned `vpay-cstack-baseline-*` directories
   had accumulated in `$TMPDIR` across one afternoon's mutation runs, and the
   one from mutation 4 still held the snapshot the tool had written — the
   non-empty case is exactly the one the old cleanup could not reach. Replaced
   with a `Drop` guard, the shape `worker_kill9.rs`'s `Workspace` already uses
   in a sibling binary. `tempfile` was correctly *not* added: it is in neither
   `Cargo.lock` nor any manifest, and the repository has a practiced pattern
   that needs no new `deny.toml` review.
2. **[misleading-claim → fixed `fabceca`] The banner restated the pin as a
   literal, and nothing warned on a mismatch.** Moving `cratestack_version` to
   0.12.0 left the log saying "justfile pins 0.11.1"; a 9.9.9 shim ran the
   whole measurement silently. The pin is read from the `justfile` now and a
   mismatch warns in `check-schema`'s words. Deliberately still a warning:
   **whether a version mismatch should make this test red — the asserted 86 is
   a number about one grammar — is a maintainer's decision and was left open,
   not taken.**
3. **[robustness → fixed `2b1fe64`] `!stdout.contains(name)` was a bare
   substring search.** This repository names constraints by pattern
   (`amount_non_negative` on three tables, `id_length` on two). Proven with a
   two-column `CONSTRAINT updated_at` on `currencies`: the old code failed
   accusing the tool of reporting it, when the report only ever printed
   ``column `updated_at` ``. Now matched as ``CHECK `name` ``, the only
   rendering `Op::AddCheck`/`Op::DropCheck` have — detection is unchanged, and
   that too is measured (a two-column `charges_amount_range_check` still trips
   it).
4. **[nit → fixed `331be79`] The missing-tables helper keyed on `[lossy]`.**
   Correct today, but had the label changed the helper would have returned an
   empty set and the test would have failed claiming the schema now declares
   every table — the wrong diagnosis. Keyed on the sentence now; proven by
   compiling the committed function verbatim against four lines.
5. **[misleading-claim → fixed `d16eca2`] "No baseline row was recorded" was
   prose, not an assertion.** `docs/status.md` listed it among the things
   `--strict` is *asserted* not to do; it had been checked by hand. It is the
   half nothing else could catch: introspection excludes
   `cratestack_migrations` from its own table list, so a recorded row moves
   neither the 86 nor the table set (mutation 7 measured exactly that). Now
   read from the database.
6. **[misleading-claim → fixed `0c5ce1d`] The CHECK finding was recorded as an
   inference from ten samples.** It is upstream's documented gap; citing it
   changes the ask. See "The CHECK finding" above.

## Recorded, not fixed

- **Format coupling is real and is the right trade.** The test keys on five
  literal strings: `drift detected in …`, the "exists in the live database but
  is not declared in the schema" sentence, ``CHECK `name` ``, the
  "could not confidently map" line, and the entire `--strict` refusal sentence
  on stderr (matched with `assert_eq!`, so any extra stderr line breaks it
  too). All five were checked against the 0.11.1 sources. A patch release that
  rewords any of them turns this test red with a message naming the string it
  wanted — loud, not silent, which is the right direction. It is a real
  maintenance cost worth stating: crates.io published 29 `cratestack-cli`
  releases in the month before the pin.
- **The version is printed, not asserted**, so a CLI release cannot make the
  test pass for the wrong reason silently — but see finding 2's open question.
- **Mutation 4 fails on the exit code before the no-writes assertions.** The
  property holds (the run is red and the repository is untouched); with the
  `Drop` guard the leak that made it visible is gone.
- **Timing under load.** 44.6s cold, 1.4–15s warm, and two 120s
  container-start timeouts across this review's ~15 container runs on a host
  running three other agents' suites. Both passed on retry. Nothing in this
  test is slow; the contention is.

## Final gate, after the six fixes

Run in the project's own recipes, in CI's order, on the tree this file is
committed with:

```
just fmt-check       ok
just clippy          ok
just verify          ok — the ten gates above passed
just docs-check      ok — 1 unimplemented item; 712 repository links resolve
just verify-ignored  0 ignored (expected 0), 42 test binaries (expected 42),
                     1271 total (minimum 1080)
just test-rust       Summary [1022.981s] 1271 tests run: 1271 passed, 0 skipped
```

**1271 passed, 0 skipped, 0 ignored** — the whole suite, no retry needed on
this run. The two container-start timeouts recorded above happened on earlier
runs during the review, both in tests this branch does not touch or in the new
test's own container start, and both passed on retry.

`just docs-check` and `just verify-links` were re-run after this file was
added.
