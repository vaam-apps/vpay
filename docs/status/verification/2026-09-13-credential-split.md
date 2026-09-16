# Verification — the credential split (ADR-0019, migration 0044), 2026-09-13

Head `c87d16f4`, branch `claude/staff-credential-split`, based on
`claude/dash-admin-role-review` (`f14e31fc`, open as PR #166).

## Toolchain, pinned as CI pins it

|          |                                                            |
| -------- | ---------------------------------------------------------- |
| rustc    | `1.98.0 (88d9e12ae 2026-08-18)` — `rust-toolchain.toml`    |
| Node     | `v22.23.2` — **`.nvmrc`, not the host's v24.20.0**         |
| pnpm     | `9.15.0`, `pnpm install --frozen-lockfile` before the gate |
| Postgres | `postgres:16-alpine`, rootless Docker                      |

The Node line is not decoration. The host runs v24.20.0 and `.nvmrc` pins
22.23.2; this project has already lost a CI round to exactly that gap, so the
gate ran under `nvm use 22.23.2`.

## `just ci`

**Exit code `0`, read from a file** (`echo $? > cred-exit.txt`), not from the
harness banner — which reported `0` for an earlier run whose real exit was
`1`.

```
$ grep -nE "^ *FAIL|error\[|^error:" cred-ci.txt
(no output)
```

| Step                                                    | Result                                                                |
| ------------------------------------------------------- | --------------------------------------------------------------------- |
| `just verify`                                           | **twelve gates green**; `verify-docs` advisory                        |
| `verify-migrations`                                     | 44 files match `MANIFEST.sha256`                                      |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean                                                                 |
| `cargo nextest run --workspace`                         | **1777 run, 1777 passed, 0 skipped** (1501.8 s)                       |
| `verify-ignored`                                        | **0 ignored** (expected 0), 47 binaries (expected 47), 1777 total     |
| `cargo test --doc --workspace`                          | **114 passed, 1 ignored** (the pre-existing `sdks/rust` README block) |
| `cargo deny`                                            | `advisories ok, bans ok, licenses ok, sources ok`                     |
| `lint-web`, `typecheck`                                 | clean                                                                 |
| `test-web`                                              | dashboard 311/311 (30 files), checkout 523/523                        |

**0 ignored is asserted, not assumed.** `verify-ignored` pins the count at
zero and the binary count at 47, so a container-backed case that started
skipping — the failure mode this project's rules name first — is red rather
than quietly green.

## Two earlier runs that proved nothing, recorded because they cost rounds

1. **exit `1`** — `cargo fmt --all --check`, `just ci`'s **first** recipe.
   Nothing after it ran.
2. **exit `254`** — `pnpm exec prettier --check .`, the **second** recipe:
   `Command "prettier" not found`, because this worktree had never had
   `pnpm install --frozen-lockfile`. Then exit `1` again on three unformatted
   new markdown files.

## Mutations

Eleven run against the decisive cases; ten caught, one not, and the one that
was not caught **found a defect in the test rather than in the schema**. Full
table in the branch's report and in the commit messages; the load-bearing
ones:

| #   | Mutation                                                              | Result                                         |
| --- | --------------------------------------------------------------------- | ---------------------------------------------- |
| M1  | drop `advance_counter`'s `.lt(counter)` — the replay guard            | caught                                         |
| M2  | widen it to `.lte(counter)`                                           | caught                                         |
| M3  | migration seeds `counter` to `0` instead of carrying `last_totp_step` | caught                                         |
| M4  | migration rewrites the password material instead of copying it        | caught                                         |
| M5  | one-per-subject unique index becomes an ordinary index                | caught                                         |
| M6  | `(issuer, subject)` no longer globally unique                         | caught                                         |
| M7  | one-link-per-issuer no longer unique                                  | caught                                         |
| M8  | the subject FK dropped entirely                                       | caught                                         |
| M9  | the per-kind material/identity CHECK becomes `TRUE`                   | **NOT caught**, then fixed and re-run → caught |
| M10 | `Debug` prints `material` instead of redacting it                     | caught                                         |
| M11 | M1 again, but measured **end to end over HTTP**                       | caught (`200` where `401` is demanded)         |

**M9 is the one worth reading.** Replacing the whole of
`credentials_federated_carries_identity_and_no_material` with `TRUE` left
`a_kind_spelled_here_is_a_kind_the_database_admits` green, because its
negative cases sat on the same staff member the vocabulary loop had just given
one credential of every kind — so every `expect_err` was satisfied by a
**unique index** and the CHECK was never consulted. The fix moves them to
their own subject **and** asserts which constraint refused each one, so a case
that starts passing for the wrong reason says so. Commit `b9b71630`.

**M12 is recorded as not caught, deliberately.** Making
`credentials_one_singleton_kind_per_staff_member` an ordinary index leaves
`a_second_enrolment_cannot_replace_an_enrolled_second_factor` in
`staff_sign_in.rs` **green** — the handler reads the stored credential and
never reaches the create path, so the index is never consulted. The guard is
pinned at the repository level instead (M5, caught). The test's doc comment
claimed a decisive mutation it does not have; that claim is corrected in
`c87d16f4`'s parent rather than left standing.

## Drift, measured

| Constant                      | Before | After   |
| ----------------------------- | ------ | ------- |
| `EXPECTED_DRIFT_CHANGES`      | 179    | **190** |
| `EXPECTED_DRIFTED_RELATIONS`  | 24     | **25**  |
| `EXPECTED_UNMAPPABLE_COLUMNS` | 19     | **19**  |

Read off a freshly migrated database. The +11 is `credentials`' twelve lines
(eight CHECKs, four indexes) minus `staff_members_totp_step_is_not_negative`,
which went with its column. Not one `column ... type differs`. The full
account is in
[status/cratestack/2026-09-13-credentials.md](../cratestack/2026-09-13-credentials.md).
