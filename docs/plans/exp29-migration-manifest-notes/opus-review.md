# exp29 — sabotage review of the migration-manifest gate (issue #76)

Reviewer: Opus, 2026-09-07. Draft under review: `f78f291` + `5518f1a` (haiku),
base `30fb8f1`. Worktree `.claude/worktrees/exp29-migration-manifest`, branch
`claude/exp29-migration-manifest`.

The brief's instruction was to treat every claim in the draft as unverified.
Doing so found one finding that would have hurt an on-call operator, one that
made the gate unusable for the documentation the brief asked for, and two that
let the gate be satisfied by a manifest that records two different truths.

## Verdict

**Not safe as drafted.** The gate itself was sound in shape and its three
headline mutations really did fail. The repair procedure it shipped alongside
was inverted, and no test or reader could have caught that by inspection.

## Findings

### F1 — the runbook's repair wrote the checksum a broken database already has (critical)

The draft's `docs/runbooks/migrations.md` gave the repair as

```sql
SET checksum = decode('f4d1a8e1…8ae252'::text, 'hex')
```

and said, correctly about itself, that this is "the SHA-384 of
`0028_create-checkout-sessions.sql` **in its original, unedited state**".

That is the value a broken database *already holds*. The repair exists for a
database that applied the original and is being asked to boot the current
binary; it must write the **current** file's checksum. Run as drafted the
statement reports `UPDATE 1`, changes nothing, and the binary keeps exiting 78
— with the page telling the operator it had worked.

Both values, and how each was obtained:

| | SHA-384 | How |
|---|---|---|
| current (what the repair must write) | `6eeb31eeaf5e8adfe033b01c9ae7a8c579e68e543fef93d8febd41d32f6234d02e8fb63f03cb2e4137e187f59007b5ec` | read from `_sqlx_migrations.checksum` on a fresh migrated `postgres:16-alpine`; equals `sha384sum backends/migrations/0028_create-checkout-sessions.sql` |
| original (what a broken database holds) | `f4d1a8e11606df3e3d0b3fb2a0a0483668b53813fb1baa6598ca3fdc2e105db162c4d43e8ea0e2790a4baecbce8ae252` | `git show d0b602e:backends/migrations/0028_create-checkout-sessions.sql \| sha384sum` |

Fixed, and the fix is executable rather than asserted:
`the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`
(`backends/tests/integration/tests/postgres_smoke.rs`) migrates a fresh
container, rewinds version 28's checksum to the original, confirms
`sqlx::migrate!` refuses with the message the runbook quotes, **parses the
`UPDATE` out of the markdown file**, runs it, and confirms the migrator then
runs clean. Measured: `1 test run: 1 passed`, 1203 s (slow because another
agent's `nextest run --workspace` was saturating the host and the Docker
daemon at the time).

The runbook now also states the original value beside the current one, with a
`psql` query and a table so an operator can determine which state their
database is in before writing anything.

### F2 — the gate ran in `just verify` but not in CI (high)

`.github/workflows/ci.yml`'s `self-checks` job lists its steps explicitly; the
draft added none. The justfile's own claim that "CI's self-checks job runs
exactly this list" was false the moment the draft landed, and the gate would
have protected local runs only. Added as the eleventh step, through
`just verify-migrations` like the four steps before it.

### F3 — a non-`.sql` file in `backends/migrations/` deadlocked the repository (high)

The draft hashed **every** file in the directory. Measured: dropping a
`README.md` there fails the gate ("file exists but is not in
MANIFEST.sha256"), `just migrations-manifest` will not add a line for it (it
globs `*.sql`, as it must), and a hand-written line is deleted by the next run
of that recipe. So the gate as drafted made
`backends/migrations/README.md` — the one place a contributor adding a
migration is guaranteed to look, which the brief asked for — impossible to
add.

`sqlx::migrate!` reads `*.sql` at the top level and nothing else, so a
`README.md` is not a migration and no database ever hashed it. The gate now
matches.

### F4 — duplicate manifest lines passed silently (medium)

Measured: prepending
`0000…0000  0001_create-currencies.sql` above the real line left the gate
green and its count unchanged at 35. `BTreeMap::insert` kept the last entry
and the stale one above it was invisible — a manifest recording two different
truths about one file, passing. Now a finding, with the line number.

### F5 — the manifest could not carry a header (medium)

A `#` line failed as "malformed", so the immutability rule had nowhere to live
at the top of the file a contributor opens to add a line. Comments and blank
lines are now skipped, `just migrations-manifest` preserves the header
verbatim, and the header is written.

### F6 — `just migrations-manifest` silently dropped a line whose file had gone (medium)

The draft's refusal loop only compared entries whose file still existed; a
manifest line for a deleted migration was quietly removed on the next run.
That is the "regenerate to make the problem go away" path the recipe exists to
close. It now refuses, and says a shipped migration is never deleted.

### F7 — documentation the draft did not update (medium)

`AGENTS.md` still said `just verify` is "**ten** gates", listed the ten by
name, and said "Nine of the ten are `cargo xtask` commands"; `CLAUDE.md` still
said "ten self-checks". Both are load-bearing here — AGENTS.md carries the
count *and* the history of every gate that moved it, and its own text records
how often that count has been wrong. The new runbook was also absent from
`docs/runbooks/README.md`'s index table. All corrected.

Left alone deliberately: `docs/status.md:507` ("all ten gates") and
`docs/reference/vpay-db.md:1446` ("not any of the ten `just verify` gates")
are dated records of what was true on a past run. This repo supersedes rather
than rewrites those.

### F8 — cosmetic, fixed in passing

`just migrations-manifest` wrote the manifest with mode `600`, because it
`mv`ed a `mktemp` file into place. The recipe now writes through `cat` and
`chmod 644`. `find | sort` is `LC_ALL=C` so the file does not reshuffle on a
different locale.

## Mutations, re-run by the reviewer

Against the tree, with the real gate, restoring between each. "before" is the
draft at `5518f1a`; "after" is this branch's head.

| # | Mutation | Before | After |
|---|---|---|---|
| M1 | edit a byte in `0001_create-currencies.sql` | FAILS, names the file | FAILS, names the file |
| M2 | add `0099_x.sql` with no manifest line | FAILS, prints the line to add | FAILS, prints the line to add |
| M3 | `just migrations-manifest` after M1 | REFUSED, exit 1 | REFUSED, exit 1 |
| M4 | delete a manifest line | FAILS | FAILS |
| M5 | reverse every manifest line | passes (order is not the contract) | passes |
| M6 | manifest line for a file that does not exist | FAILS | FAILS |
| M7 | put `README.md` in `backends/migrations/` | **FAILS — and is unfixable** | passes |
| M8 | `#` comment at the top of the manifest | **FAILS as malformed** | passes |
| M9 | edit the last migration, then the recipe | REFUSED | REFUSED |
| M10 | manifest entry duplicated with a wrong hash | **passes** | FAILS |
| M11 | edit a migration **and** its manifest line by hand | passes | passes — by construction; see below |
| M12 | delete a migration file, then the recipe | line silently dropped | REFUSED |

M5 is deliberate: the manifest is a set of claims about files, and a rebase
that interleaves two branches' appends must not fail the build.

M11 cannot be closed. A manifest whose own hash is checked has to pin that
hash somewhere in source, and whoever can edit two files can edit three; the
only difference would be a third file in the diff. **The recursion was
considered and rejected.** What the manifest actually buys is that the edit
becomes a one-line diff on a file whose sole purpose is to be reviewed — a
comment reflowed inside a 400-line `.sql` file is not visible in review, and
that is exactly how issue #76 happened. This is now stated plainly in the
runbook, in `backends/migrations/README.md`, in the gate's doc comment and in
`docs/status.md`, rather than left for a reader to discover.

Every mutation above is now a unit test (`migration_manifest_tests` in
`.xtask/src/main.rs`, twelve tests) rather than something a reviewer ran once,
because `check_migrations` was refactored into a pure function over
(manifest text, filename → hash) with the filesystem in the caller.

## Cross-platform

- The gate hashes **file bytes** (`fs::read`), not text: a CRLF checkout is a
  different migration and the gate says so, which is correct — sqlx would say
  the same. Noted in the source.
- Filenames are matched exactly, and the manifest's `<hash>  <name>` split is
  on the first two-space run, so a name is never trimmed or case-folded.
- The digest is SHA-256 in the spelling `sha256sum` writes, so any line can be
  checked by hand with coreutils.

## Verified

Two things the draft claimed that were true as claimed, and worth recording
because they were checked rather than assumed:

- `sha2` needed no manifest change: `.xtask/Cargo.toml` has taken
  `sha2.workspace = true` since `gen-signing-key`, and `Cargo.lock` already
  resolved `sha2 0.10.9`. `cargo deny check` is unaffected — no package was
  added to the graph.
- The draft's `MANIFEST.sha256` hashes are correct: all 35 match `sha256sum`.

## Not checked

- No `kubectl`/`helm` command in the runbook has been run; no deployment
  exists. §5 (a mismatch on any migration other than 0028) is reasoning, not
  measurement.
- `verify-citations` needs the network and is not part of `just ci`; the
  issue and PR numbers cited here (#39, #76, commit `d0b602e`) were resolved
  by hand — `gh issue view 76` and `git show d0b602e` — not by that gate.
- Nothing here was run on a CRLF checkout or on Windows; the byte-hashing
  behaviour is argued from the code and from what sqlx does, not observed.
