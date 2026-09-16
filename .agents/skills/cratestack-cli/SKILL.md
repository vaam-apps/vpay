---
name: cratestack-cli
description: The cratestack CLI — check, generate-dart, generate-typescript, generate-wiremock, studio, print-ir, migrate, diff — every subcommand, every flag, exit codes, the --check drift mode, JSON diagnostic shapes, and CI recipes. Load when running or scripting the cratestack binary, wiring schema validation or generated-client drift checks into CI, or installing @cratestack/cli.
---

# The `cratestack` CLI

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

Binary name is `cratestack`, crate name is `cratestack-cli`.

## Exactly eight subcommands

```
check  generate-dart  generate-typescript (alias generate-ts)  generate-wiremock
studio  print-ir  migrate  diff
```

**There is no top-level `init`, `run` or `eject`** — those three are `studio`
subcommands. There is no `migrate apply`, `rollback`, `verify` or `drift`, no
`generate-rust`, and no `generate-studio`. Documentation that lists `init`, `run`
or `eject` at top level is describing a flatter CLI than the one that ships.

## Install

```bash
npm install --global @cratestack/cli     # prebuilt binary, no Rust toolchain
cargo install cratestack-cli             # from source
cargo binstall cratestack-cli            # same GitHub Release assets as npm
```

The npm postinstall downloads a per-target archive plus its `.sha256` sidecar and
verifies the hash. **Five targets are supported** — `darwin-x64`, `darwin-arm64`,
`linux-x64` (gnu), `linux-arm64` (gnu), `win32-x64`. The package's `cpu` field
advertises `arm64` on all three platforms, but **`win32-arm64` has no prebuilt
binary** and the installer throws, pointing you at `cargo install`. There is no
musl target. Windows extraction shells out to PowerShell `Expand-Archive`.

`CRATESTACK_CLI_SKIP_DOWNLOAD=1` skips the download;
`CRATESTACK_CLI_BINARY_PATH` points the shim at a vendored binary. The shim
propagates the child's exit status, so gating CI through `npx` works.

In GitHub Actions, prefer the composite action over a Rust toolchain step:

```yaml
- uses: cratestack/cratestack/.github/actions/install-cratestack-cli@main
  with:
    version: "0.12.0"   # optional, defaults to "latest"
```

Pin `@main` to a tag or SHA for reproducibility.

## `check`

```bash
cratestack check --schema schema.cstack [--format human|json]
```

Human mode prints `schema OK: <path>` or a rendered diagnostic on stderr. JSON
mode prints to stdout and **also exits 1 on failure**, so capture the output
before the step fails:

```json
{ "ok": false, "schema": "schema.cstack",
  "diagnostics": [ { "message": "…", "file": "…", "line": 12, "start": 240, "end": 252 } ] }
```

`line` is 1-based; `start`/`end` are byte offsets. The array always holds exactly
one diagnostic today — `check` parses one file and the parser returns a single
error. `file` is per-diagnostic and deliberately duplicated alongside the
top-level `schema` so the shape survives multi-file reporting later.

## Code generation

All three generators share a `--check` drift mode and `--base-path` (default
`/api`).

### `generate-dart`

| Flag | Default |
| --- | --- |
| `--schema` / `--out` | required |
| `--library-name` | `cratestack_client` |
| `--base-path` | `/api` |
| `--template-dir` | none |
| `--preset default\|riverpod` | `default` |
| `--check` | off |
| `--run-build-runner` | off |
| `--no-native-cbor` | off (native CBOR is on) |

`--run-build-runner` shells out to
`dart run build_runner build --delete-conflicting-outputs` in `--out`, and **has
no effect with `--check`**. `--no-native-cbor` is needed only on Linux arm64.
`--native-cbor` was removed and is now an unknown-flag error, not a silent no-op.

### `generate-typescript` (alias `generate-ts`)

| Flag | Default |
| --- | --- |
| `--schema` / `--out` | required |
| `--package-name` | `cratestack-client` |
| `--base-path` | `/api` |
| `--template-dir` | none |
| `--check`, `--full-selection` | off |
| `--swr`, `--refine`, `--tanstack`, `--rtk` | off |
| `--no-native-cbor` | off |

**The four layer flags are purely additive** — the default `src/` layout is
always emitted. `--swr` adds `src/swr/`, `--refine` adds `src/refine.ts`,
`--tanstack` adds `src/react-query.ts`, `--rtk` adds `src/rtk-api.ts` plus the
RTK peer dependencies. `--no-native-cbor` has **no effect on a REST-transport
schema**; the one napi gap is `win32-arm64`.

The docs site's flag table omits `--tanstack` and `--rtk` entirely.

### `generate-wiremock`

`--schema`, `--out`, `--base-path`, `--check`. Emits one stub per procedure and
**five per model**. Note that `transport rest` model CRUD stubs are stateful and
need more than a plain WireMock.

### `--check`, precisely

Generates in memory, diffs against `--out`, **never writes**. Reports three
kinds — `modified`, `missing` (generator produces it, disk lacks it) and
`unexpected` (on disk, generator no longer produces it) — and exits 1.

It shells out to `git check-ignore` and **excludes gitignored files from the
`unexpected` arm**. Without that, `.dart_tool/`, `*.g.dart` and `pubspec.lock`
left by `flutter pub get` all report as drift while content matches perfectly. So
**run `--check` inside a real git checkout with the project's `.gitignore`
present**, or you get false positives. It returns empty (old behaviour) when git
is unavailable, when `--out` is not in a work tree, or when `--out` is itself
ignored.

## `diff`

```bash
cratestack diff <OLD> <NEW> [--json]
```

**Positional paths, not `--old`/`--new`, and files, not git refs.** In CI, pull
the base copy out first:

```yaml
- run: git show origin/${{ github.base_ref }}:schema.cstack > /tmp/old.cstack
- run: cratestack diff /tmp/old.cstack schema.cstack
```

Matching is by name only — a rename reads as a removal plus an addition, matching
`cratestack-migrate`'s philosophy.

| Severity | Examples |
| --- | --- |
| `BREAKING` | removing a model/field/procedure; **adding or removing `@@paged`** (it flips `.list()` between `T[]` and `Page<T>`); **adding `@@internal(...)`**; adding a required field with no default; retyping anything; narrowing arity |
| `ADDITIVE` | adding a model, an optional field, a procedure; widening arity; **removing `@@internal(...)`** |
| `INTERNAL` | `@@soft_delete`, `@@audit`, `@@retain`, `@@emit` — a known scope gap, not an oversight |

Output is printed **first**, then the process exits 1 if there is any breaking
change — so a JSON consumer always gets a complete document even on a gating
failure.

```json
{ "changes": [ { "severity": "breaking", "category": "field_removed",
                 "subject": "…", "message": "…" } ],
  "summary": { "breaking": 1, "additive": 0, "internal": 0 } }
```

## `print-ir`

`cratestack print-ir --schema <path>` dumps the parser's fully-resolved
`Schema` as Rust pretty-`Debug`. **Not JSON, and not a machine contract** — it is
for seeing what mixins expanded to, what a `@relation` resolved to, and which
attributes actually attached. It is a *different* IR from `cratestack-migrate`'s
projections; nothing in the CLI prints that one.

## Exit codes

Everything non-success is **1**. There is no distinct code for "breaking" versus
"error".

| Situation | Code |
| --- | --- |
| success | 0 |
| `check` failure, human or JSON | 1 |
| `diff` with ≥1 breaking change | 1 |
| any `--check` drift | 1 |
| `migrate diff` lossy without `--allow-destructive` | 1 |
| `migrate baseline --strict` with drift, or snapshot already exists | 1 |

## CI recipes

**Validate a schema** — a bare `run:` step gates, since `check` exits 1.

**Drift-check generated clients** — the framework does exactly this to itself
with `just regen-examples --check`, which forwards `--check` to both generators.
Keep the drift step without `continue-on-error`, and put `if: ${{ !cancelled() }}`
on the steps behind it so a red drift check does not skip real coverage.

**Gate breaking schema changes** — `cratestack diff` as above.

**There is no shipped recipe for gating migration-snapshot drift.**
`migrate verify` does not exist; `migrate diff` is offline and idempotent, so a
plausible gate is running it and failing if it produces a new directory or dirties
`schema.snapshot.json` — but nothing in the repo does this, so treat it as an
idea, not a documented pattern.

## Build-from-source caveat

The CLI hard-enables `cratestack-migrate/{postgres-introspect, pgvector,
postgis}`. Without them, `migrate diff` on a schema declaring
`extension postgis {}` or `extension pgvector {}` hits a deliberate `unreachable!`
and **panics**.
