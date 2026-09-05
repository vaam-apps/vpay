# exp9 (opus): `cratestack check` becomes a gate

2026-09-05, on `claude/exp9-cratestack-check-opus` (base `master` `a81b6b6`).
Everything below was run in this worktree, `CARGO_BUILD_JOBS=4`, no Docker.
The repository toolchain is the pinned one (`rust-toolchain.toml`, 1.95.0);
the CrateStack CLI is a separate binary built with `rustc 1.98.0` — see
"Installing the CLI" below for why the two cannot be the same.

## What changed

| file | change |
|---|---|
| `justfile` | new `cratestack_version := "0.11.1"` and a new `check-schema` recipe; `verify` gains it as the **sixth** gate, between `verify-links` and the advisory `verify-docs`; the file header and the `verify` preamble renumber five→six, sixth→seventh, seventh→eighth |
| `.github/workflows/ci.yml` | `self-checks` gains four steps: pinned `just`, read the version pin back out of the justfile, the upstream install action pinned by commit SHA, `just check-schema` |
| `docs/status.md` | the `schemas/*.cstack` row and the "CrateStack" section: the hand-run transcript is struck through and replaced by the gate; the CLI version, the real doc URLs, and the three mutations |
| `schemas/vpay.cstack` | **comments only.** Its header carried the same two stale claims (`cratestack.dev/docs 404s publicly`, `Verified against CrateStack 0.10.1`) and would have contradicted `docs/status.md` the moment that was corrected. No declaration was touched |

`docs/flows/*.md` Status sections: **none changed, and this was checked, not
assumed.** `grep -rn -i 'cratestack\|cstack' docs/flows/` matches only
[../../flows/ledger.md](../../flows/ledger.md) and
[../../flows/configuration.md](../../flows/configuration.md), and in both the
mention is about what the `.cstack` *grammar cannot express* (cross-column
CHECKs) rather than about whether the file parses. Neither Status section
makes a claim this gate could falsify.

No Rust source changed, so `just fmt-check` and `just clippy` have nothing new
to say. `.xtask/src/main.rs`'s module doc still opens "Five of these commands
are the gates `just verify` runs" — that stays literally true (five of the
*xtask commands* are gates; the sixth gate is not an xtask command) and was
deliberately left alone rather than edited into a sentence about a recipe that
lives elsewhere.

## Installing the CLI

`cratestack-cli 0.11.1` (published 2026-09-03; the latest on crates.io on
2026-09-05) declares `rust-version = "1.98.0"`. This worktree pins `1.95.0`,
so `cargo install` run **inside** it refuses. Measured, not assumed:

```
$ cd <worktree> && cargo install cratestack-cli --locked --version 0.11.1 --force --root /tmp/csforce
    Updating crates.io index
error: cannot install package `cratestack-cli 0.11.1`, it requires rustc 1.98.0 or newer,
while the currently active rustc version is 1.95.0
`cratestack-cli 0.8.15` supports rustc 1.95.0
```

Installed from `$HOME` instead, where the host toolchain applies:

```
$ rustup run stable rustc --version
rustc 1.98.0 (88d9e12ae 2026-08-18)

$ cd ~ && cargo +stable install cratestack-cli --locked --version 0.11.1
   Compiling cratestack-parser v0.11.1
   Compiling cratestack-sqlx v0.11.1
   Compiling cratestack-migrate v0.11.1
   Compiling cratestack-client-dart v0.11.1
   Compiling cratestack-client-typescript v0.11.1
   Compiling cratestack-mock-wiremock v0.11.1
   Compiling cratestack-cli v0.11.1
    Finished `release` profile [optimized] target(s) in 2m 52s
   Replacing /home/selast/.cargo/bin/cratestack
    Replaced package `cratestack-cli v0.11.1` with `cratestack-cli v0.11.1` (executable `cratestack`)

$ cratestack --version
cratestack 0.11.1
```

Two honest footnotes on that transcript. The binary on this machine was
`0.10.1` before this pass (dated 2026-09-02, the version the old `docs/status.md`
claim named). And "Replaced 0.11.1 with 0.11.1" is not a typo: a second agent
was installing the same version concurrently and finished first, so this
build replaced an identical binary. The prebuilt release asset was **not**
used locally — CI uses it, this machine built from source.

## The gate

```
$ just check-schema
check-schema: cratestack 0.11.1, schema schemas/vpay.cstack
schema OK: schemas/vpay.cstack
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.11.1
```

**The schema needed no edit.** It was last verified by hand at `0.10.1`; it
passes at `0.11.1` unchanged. Nothing in this branch touches a declaration in
`schemas/vpay.cstack` — the only diff there is its header comment.

Inside `just verify`, in position six:

```
$ just verify
...
cargo xtask verify-links
verify-links: ok — 676 repository link(s) in 115 tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)
check-schema: cratestack 0.11.1, schema schemas/vpay.cstack
schema OK: schemas/vpay.cstack
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.11.1
cargo xtask verify-docs
...
verify: ok — the six gates above passed; the verify-docs report is advisory
```

(That transcript was taken before this file was tracked. With it staged,
`verify-links` reports **678 repository link(s) in 116 tracked markdown
file(s)** — the two relative links in the table above are the new ones.
`docs/status.md`'s `verify-links` bullet is left at the 115/673 it measured
on its own branch, because that sentence is a dated record of that change,
not a running total.)

## Mutation 1 — `tags String[]` on a model

Added `tags String[]` to `PaymentIntent`. `just verify` **fails**, at
`check-schema`, with the list-arity rejection the schema's own header quotes —
and `verify-docs`, the step after it, never runs:

```
$ just verify ; echo "exit=$?"
cargo xtask verify-no-mocks
verify-no-mocks: ok — no test double reachable from a shipping binary
cargo xtask verify-status
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
cargo xtask verify-errors
verify-errors: ok — 15 error type(s), all classified; ...
cargo xtask verify-sdk-parity
verify-sdk-parity: ok — 342 proving test(s) named in docs/sdks/parity.md all exist, 26 dated gap(s)
cargo xtask verify-links
verify-links: ok — 676 repository link(s) in 115 tracked markdown file(s) ...
check-schema: cratestack 0.11.1, schema schemas/vpay.cstack
Error: Error: model `PaymentIntent` field `tags`: list-valued type `String[]` is not
supported on a database-backed model — there is no SQL bind representation for a
list-valued scalar or enum yet, so this schema would parse and emit valid DDL but panic
at `include_server_schema!`/`include_embedded_schema!` expansion. Use a single `String`
value, model this as a `@relation` to another model, or drop the `datasource` block if
this schema is only ever consumed via `include_client_schema!`.
error: Recipe `check-schema` failed with exit code 1
exit=1
```

**Reverted.** One observation worth recording rather than smoothing over: the
CLI's caret points at the wrong place (`schemas/vpay.cstack:193:66`, inside a
*comment* thirteen lines below the offending field). The message names the
right model and the right field, so the gate is usable, but the span is not
trustworthy — the same class of misleading pointer the schema's header already
warns about for column-aligned fields.

## Mutation 2 — the recipe points at a file that does not exist

Renamed the recipe's `schema=` to `schemas/vpay-renamed.cstack`. The gate
**fails**; it does not pass on having nothing to check:

```
$ just check-schema ; echo "exit=$?"
check-schema: cratestack 0.11.1, schema schemas/vpay-renamed.cstack
Error: Error: failed to read schema file schemas/vpay-renamed.cstack: No such file or directory (os error 2)
error: Recipe `check-schema` failed with exit code 1
exit=1
```

**Reverted.** The exit code is the CLI's own — the recipe does not test for
the file's existence itself, and does not need to.

## Mutation 3 — the binary is not installed

The failure mode this recipe exists to get right. With `cratestack` off
`PATH`:

```
$ env PATH=/usr/bin:/bin just check-schema ; echo "exit=$?"
check-schema: FAIL — needs the 'cratestack' CLI on PATH, and it is not there.
check-schema: this is a failure, not a skip: nothing checked schemas/vpay.cstack in this run.

  Install the pinned release, from a directory OUTSIDE this checkout:

      (cd ~ && cargo +stable install cratestack-cli --locked --version 0.11.1)

  Outside the checkout on purpose: cratestack-cli 0.11.1 declares
  rust-version = 1.98.0 and rust-toolchain.toml here pins 1.95.0, so cargo
  run from inside the worktree refuses with an msrv error. There is also a
  prebuilt binary for five target triples (x86_64/aarch64 linux-gnu and
  apple-darwin, x86_64-pc-windows-msvc) at
  https://github.com/cratestack/cratestack/releases/tag/v0.11.1 —
  linux MUSL has none, per https://cratestack.dev/tooling/cli-install
error: Recipe `check-schema` failed with exit code 1
exit=1
```

## CI

Four steps in `self-checks`, after `verify-links` and before `verify-docs`.
The job's `name:` is untouched — it is a required status check.

```yaml
      - uses: taiki-e/install-action@e67fa11c4b9316fa714ddf0abed07a0c3143b95b # v2.87.4
        with:
          tool: just
      - id: cratestack
        run: echo "version=$(just --evaluate cratestack_version)" >> "$GITHUB_OUTPUT"
      - uses: cratestack/cratestack/.github/actions/install-cratestack-cli@6b3053fa77924f5162915d594d457d3eda51afaa # v0.11.1
        with:
          version: ${{ steps.cratestack.outputs.version }}
      - name: check-schema (schemas/vpay.cstack)
        run: just check-schema
```

(Comments elided here; the file carries them.)

Four decisions in those twelve lines, each with a reason:

1. **The action, not a `curl | sha256sum -c`.** The install action documented
   at <https://cratestack.dev/tooling/cli-install> exists and does exactly
   what the fallback would: its `action.yml` resolves the target triple,
   downloads `cratestack-cli-<target>-v<version>.tar.gz` from that
   repository's releases, and compares the archive against the published
   `.sha256` sidecar before putting the binary on `PATH`. Confirmed by reading
   the file rather than the page.
2. **Pinned by commit SHA, not `@main`.** The docs show
   `install-cratestack-cli@main`. `6b3053fa77924f5162915d594d457d3eda51afaa`
   is the commit `v0.11.1` was tagged at, and its `action.yml` blob
   (`53033f5…`) is byte-identical to the one on `main` today — so pinning
   costs nothing and does not accept a promise from whoever can move a
   branch. This matches how the repository already pins
   `taiki-e/install-action` and `dtolnay/rust-toolchain`.
3. **The version pin lives in one file.** `cratestack_version` is a justfile
   variable and the workflow reads it back with `just --evaluate
   cratestack_version` — the same trick the Rust jobs already use to read the
   compiler channel out of `rust-toolchain.toml`. Writing `0.11.1` in both
   files would drift silently: CI would keep passing against a release the
   local gate had stopped using.
4. **`just check-schema`, not `cratestack check --schema …`.** The repository's
   own rule, already applied to `just audit-web` (`web`) and `just helm-check`
   (`deploy`): CI runs the recipe so the gate and the local check cannot
   drift.

`x86_64-unknown-linux-gnu` is what `ubuntu-latest` resolves to, and it is one
of the five triples with a prebuilt binary. Linux **musl** has none —
<https://cratestack.dev/tooling/cli-install> says so in as many words — which
matters only if this job ever moves to a musl runner.

## Gate results before reporting

| command | result |
|---|---|
| `just verify` | **ok**, six gates, on this tree |
| `just check-schema` alone | **ok** |
| `just docs-check` | **ok** (`verify-status` + `verify-links`) |
| `actionlint .github/workflows/ci.yml` | **clean**, exit 0 (also clean over `.github/workflows/*.yml`) |
| `just fmt-check` | **ok** — no Rust file changed in this branch, so this proves only that nothing regressed |

## The doc URLs, checked rather than repeated

`docs/status.md` said `cratestack.dev/docs 404s publicly and no authoritative
reference was found`. Half of that is still true. HTTP status codes, taken
2026-09-05:

| URL | status |
|---|---|
| `https://cratestack.dev/docs` | **404** |
| `https://cratestack.dev/getting-started/quickstart` | 200 |
| `https://cratestack.dev/tooling/cli-install` | 200 |
| `https://cratestack.dev/tooling/schema-diff` | 200 |
| `https://cratestack.dev/reference/field-attributes` | 200 |
| `https://cratestack.dev/reference/scalars` | 200 |

The site's own navigation lists sections under `/overview/*`,
`/getting-started/*`, `/guides/*`, `/architecture/*`, `/tooling/*` and
`/reference/*`. `/docs` is simply not one of them.

## `@@check(expr)` in 0.11.1

The two `GAP` comments in `schemas/vpay.cstack` claim CrateStack cannot
express a cross-column CHECK. Re-verified at the pinned version against the
crates' own sources in `~/.cargo/registry/src`, not against a changelog:

- `grep -rn '@@check' cratestack-parser-0.11.1/src cratestack-migrate-0.11.1/src`
  → **0 hits**.
- `KNOWN_ATTRIBUTE_NAMES` in
  `cratestack-parser-0.11.1/src/validate/misspelled_attributes.rs` — which its
  own doc comment describes as the union of every attribute name the language
  knows — contains no `check`.
- `cratestack-migrate-0.11.1/src/convert/checks.rs` still gates CHECK emission
  on `field_has_db_enforce(field: &Field)`, i.e. a **single field's**
  validator.

So the GAP comments stand, and `docs/status.md`'s sentence about them now
names 0.11.1 as well.

## What this branch did NOT do

- **No CI run of this branch exists.** It cannot: nothing has been pushed and
  no pull request opened. Every result above was measured on the authoring
  machine. In particular, the install action, the
  `just --evaluate cratestack_version` step and the `x86_64-unknown-linux-gnu`
  release asset have **never executed on a runner** — they are read-and-reasoned,
  not observed.
- **Nothing generates from the schema.** `schemas/vpay.cstack` is still
  excluded from the build graph. `cratestack check` parses and type-checks it;
  it does not emit a migration, a server, or a client, and `cratestack migrate
  diff` has still never been run against a vpay Postgres. Nothing compares this
  file to `backends/migrations/*.sql`, which remain the authoritative schema.
- **`cratestack diff` is not wired in.** <https://cratestack.dev/tooling/schema-diff>
  describes a command that classifies a schema change as breaking / additive /
  internal-only and exits non-zero on a breaking one. That would be the natural
  second step and is not taken here.
- **The 0.11 grammar was not surveyed for features this file could use.** Only
  the one question that affects an existing claim (`@@check`) was answered.
- **The version is reported, not enforced, locally.** `check-schema` prints the
  version it actually used and warns loudly on a mismatch with the pin, but does
  not refuse to run. CI installs the pin exactly and is the gate of record. This
  is the same division `helm-check` draws (presence locally, version in the
  workflow), and it is a deliberate choice, not an oversight: a gate that blocks
  every contributor carrying a newer release is a gate that acquires a local
  opt-out.
- **No `cargo nextest` run.** Nothing in this branch changes Rust, so the suite
  was not re-run and no test count in `docs/status.md` was touched.
