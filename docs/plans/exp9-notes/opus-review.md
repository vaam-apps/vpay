# exp9 (opus): sabotage review of the `cratestack check` gate

2026-09-05, on `claude/exp9-cratestack-check-opus`. Reviewed
`git diff a81b6b6..0719a4f` (two commits) against the task brief, this
repository's CLAUDE.md ("the failure mode to avoid") and the rule that a
skipped check is not a passing check. Everything below was re-run here;
nothing is taken from the implementer's own notes without re-measuring it.

Tools: `cratestack 0.11.1`, `just 1.45.0`, `actionlint v1.7.12`.

## 1. Claims verified

| claim (from `docs/plans/exp9-notes/opus.md` or the diff) | verdict |
|---|---|
| `cratestack --version` is `0.11.1`; `just check-schema` prints `schema OK` | **true**, re-run |
| `cratestack-cli 0.11.1` published 2026-09-03, latest on crates.io, `rust-version = "1.98.0"` | **true**, crates.io API |
| the pasted MSRV refusal, incl. "`cratestack-cli 0.8.15` supports rustc 1.95.0" | **true** — 0.8.15 is the highest release whose `rust_version` is 1.95.0 |
| `just --evaluate cratestack_version` prints `0.11.1` | **true** |
| the action path exists at `6b3053fa77924f5162915d594d457d3eda51afaa` | **true**, `gh api .../contents/...?ref=<sha>` |
| it accepts a `version` input (no leading `v`, default `latest`) | **true**, read `action.yml` at that SHA |
| it **verifies a SHA-256** against a published `.sha256` sidecar | **true** — step 4 of the action downloads `<asset>.sha256`, compares, and exits 1 on mismatch. Read in the action, not inferred from the docs page |
| `6b3053f…` is the commit `v0.11.1` was tagged at | **true**, `git/ref/tags/v0.11.1` -> that exact sha |
| that SHA is on the default branch, not a fork/PR head | **true** — `compare/main...<sha>` is `behind` by 1, i.e. an ancestor of `main`. It is a **released tag**, not a floating `main` commit |
| its `action.yml` blob is byte-identical to `main`'s today | **true**, blob `53033f5…` both refs |
| the v0.11.1 release carries `cratestack-cli-x86_64-unknown-linux-gnu-v0.11.1.tar.gz` **and** its `.sha256` | **true**, release assets listed |
| `taiki-e/install-action@e67fa11c…` is `v2.87.4` | **true**, tag ref matches |
| the docs show `install-cratestack-cli@main` and say to pin | **true** — "Pin `@main` to a released tag or commit SHA for reproducible CI" |
| five prebuilt triples; "Linux musl has no prebuilt binary yet" | **true**, verbatim on `/tooling/cli-install` |
| `@@check(expr)` still absent at 0.11.1 | **true on two of the three arguments given** — see finding F5 |
| `docs/flows/*.md` Status sections unchanged | **true**, and correctly reasoned |
| the missing-binary path exits non-zero with an install hint | **true** — `PATH=/usr/bin:/bin just check-schema` -> exit 1, no "skipped" |
| CI runs `just check-schema`, not a copy of the command | **true** |
| `check-schema` is in `just ci` | **true**, `ci:` depends on `verify` |
| "the same division `helm-check` draws — presence locally, version in the workflow" | **true**, `helm-check` checks `command -v` only and `deploy` pins `KUBECONFORM_VERSION` |
| `just` SHA-pinned "matching what `web` and `deploy` do" | **true**, both use the same SHA with an unpinned `tool: just` |
| runner has what the action needs (curl, tar, sha256sum, jq) | **true** for `ubuntu-latest`; `jq` is only on the `latest` path this workflow does not take |
| every URL cited returns what is claimed | **true**: `/docs` **404**; `/tooling/cli-install`, `/tooling/schema-diff`, `/getting-started/quickstart`, `/reference/field-attributes`, `/reference/scalars`, the v0.11.1 release page all **200**. `/guides/*` and `/architecture/*` are real prefixes (the site nav lists 23 and 3 pages under them); the bare `/guides` and `/architecture` are 404, and the docs are cited with the `/*` |
| status.md strike-through convention (dated, old text kept) | **followed** |
| "excluded from the build graph, drives no migration" survives | **survives, and is strengthened** — the row stays 🟡 and the section says explicitly that the gate does not change it |
| `just verify` echo count 5 -> 6 | **true** |
| `actionlint`, `just fmt-check`, `just docs-check` | all **clean** |

## 2. Findings

| # | severity | finding | fixed |
|---|---|---|---|
| F1 | **gate-bypass** | `cratestack check` prints `schema OK` and exits **0** for an **empty** `.cstack` file, and for `schemas/vpay.cstack` with its `datasource` block deleted **and** `tags String[]` added. The list-arity refusal is the mutation this gate is proven with, and the CLI's own error offers "drop the `datasource` block" as a way to silence it — so the gate's headline proof is one five-line deletion from a vacuous pass, in a log that looks identical to a real one. The delivered mutation 2 covers only a *missing* file, which the CLI does error on | `9fca2af` |
| F2 | **robustness** (pin integrity) | CI's `- id: cratestack` step wrote `echo "version=$(just --evaluate cratestack_version)" >> "$GITHUB_OUTPUT"`. Under `bash -e`, a failing `just` does not fail the step: `echo` succeeds and `version=` is written empty; the install action's `[ -z "$version" ]` branch then resolves **latest**. The SHA-pinned, version-pinned step silently becomes a floating one, on green | `a0e8742` |
| F3 | misleading-claim | `.xtask/src/main.rs` still opened "Five of these commands are the gates `just verify` runs", and called `verify-citations` "a sixth". `just verify` now runs six gates. The implementer's notes defend this as literally true of xtask *commands*; the sentence as written says "the gates `just verify` runs", and this file is the closest thing to a canonical gate list | `5650591` |
| F4 | misleading-claim | "the grammar moved four times in five weeks (0.7.8 -> 0.7.10 -> 0.10.1 -> 0.11.1)", in `justfile` and `docs/status.md`. crates.io: 0.7.8 is 2026-08-08, 0.11.1 is 2026-09-03 — **26 days**, and **29 releases** in between, not four in five weeks | `5b4cf58` |
| F5 | misleading-claim | The "no `@@check` at 0.11.1" argument cited `KNOWN_ATTRIBUTE_NAMES` as "the union of every attribute name the language knows". That list also omits `index`, `sql`, `paged`, `audit` and `soft_delete`, while this very schema uses `@@index` twice and passes — so absence from it shows nothing. Withdrawn and struck through; the conclusion still stands on the `grep` and on `convert/checks.rs` | `3752e8e` |
| F6 | nit | `schemas/vpay.cstack`'s header box right border does not line up. Pre-existing (22 short lines at `a81b6b6`) and made worse by the rewrite (37 at `0719a4f`) | `6fbbc11` |
| F7 | nit | `.github/workflows/ci.yml` says "Three steps rather than one" and adds four | `6fbbc11` |
| F8 | robustness, **pre-existing, NOT fixed** | Nothing couples `just verify`'s gate list to `.github/workflows/ci.yml`'s steps. Deleting `check-schema` from `verify`, or replacing the CI step with a bare `cratestack check --schema …`, is caught by no test — `grep -rn justfile` over `*.rs` finds nothing, and `verify-links`/`verify-status` do not read either file. This bit the repository once already: the justfile's own comment records that "CI's `self-checks` runs exactly this list" was false for `verify-sdk-parity` until 2026-09-04. Out of this task's scope, and a real guard is a gate of its own (matching `just <recipe>` against a step that may legitimately run `cargo xtask <cmd>` instead) — recorded rather than half-built |
| F9 | decision, **left as delivered** | The recipe *reports* a local CLI/pin version mismatch and does not refuse. Kept: it is documented with a reason, it matches `helm-check`'s existing division (presence locally, version pinned in the workflow), the recipe names the version it actually used in both its opening and its success line — so no log leaves the grammar in doubt — and CI installs the pin exactly. There is no `just install-cratestack`, because installing it needs a newer compiler than `rust-toolchain.toml` pins; a hard local refusal with no supported way to comply is how a gate acquires a local opt-out. This is materially unlike a skip-and-pass, which is silent about having checked nothing |
| F10 | nit, **left as delivered** | The task brief asked for a dated `just --list` description. No recipe description in this justfile carries a date (`verify-links`, `docs-check`, `helm-check` do not), and the date is in the recipe's comment block. Repo convention followed over the brief's letter |

## 3. Mutations

Every row re-run in this worktree and reverted. "raw CLI" is
`cratestack check --schema <file>` on its own.

| # | mutation | as delivered | after this review |
|---|---|---|---|
| M1 | `tags String[]` on `PaymentIntent` | `just verify` **fails** at `check-schema`, `verify-docs` never runs | unchanged |
| M2 | recipe points at a path that does not exist | **fails**, `failed to read schema file` | unchanged |
| M3 | `cratestack` off `PATH` | **fails**, exit 1, prints the install command | unchanged |
| M4 | `\|\| true` on the `cratestack check` line | M1 and M2 both **pass** (exit 0) — the check is load-bearing, not decorative | unchanged |
| M5 | schema truncated to zero bytes | raw CLI: `schema OK`, exit **0**. Recipe: **passed** | recipe **fails** (no `datasource` block) |
| M6 | `datasource` block deleted **and** `tags String[]` added | raw CLI: `schema OK`, exit **0**. Recipe: **passed** — the M1 proof is disarmed | recipe **fails** (no `datasource` block) |
| M7 | `datasource` kept, all 12 `model`/`enum`s deleted | raw CLI: `schema OK`, exit **0**. Recipe: **passed** | recipe **fails** (0 declarations < floor 12) |
| M8 | `cratestack_version` renamed in the justfile (CI step body under `set -e`) | step exits **0**, writes `version=` — the install action then resolves **latest** | step exits **1** with `::error::` |
| M9 | `check-schema` removed from `verify` | nothing catches it (F8) | unchanged — recorded, not fixed |
| M10 | CI step replaced by a bare `cratestack check --schema …` | nothing catches it (F8) | unchanged — recorded, not fixed |

## 4. Not checked

- **No CI run exists.** Nothing was pushed and no PR opened, so the install
  action, the version-resolution step and the release asset have still never
  executed on a runner. F2's fix is proven by running the step body under
  `set -e` locally, not on GitHub.
- The action's Windows/macOS/arm64 paths, and its `latest` branch (this
  workflow never takes it).
- Whether `cratestack check`'s **exit codes** are documented anywhere:
  `cratestack check --help` lists only `--schema` and `--format` and
  documents no exit code. The recipe relies on `set -e` and the observed
  1-on-error / 0-on-ok behaviour, which is reasonable but rests on
  observation rather than on a contract.
- The 0.11 grammar was not surveyed for features this schema could use — the
  implementer says so too.
- Nothing was run against a database; `cratestack migrate diff` still has
  never touched a vpay Postgres, and nothing compares this file to
  `backends/migrations/*.sql`.
