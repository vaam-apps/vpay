# exp11 (opus arm) — moving the Rust pin 1.95.0 → 1.98.0

Date: 2026-09-05. Branch `claude/exp11-toolchain-opus`, base `046892a`.
Host: the authoring machine, rootless Docker
(`DOCKER_HOST=unix:///run/user/1000/docker.sock`), `CARGO_BUILD_JOBS=4`, an
isolated `docker-container` buildx builder (`vpay-exp11-opus`), and **another
agent building the same Dockerfile concurrently** — which is why the two build
wall-clocks below are reported as "not a matched pair" rather than as a
regression or an improvement.

Every command in this file was run; every output is pasted, not paraphrased.

## 0. Why

CrateStack 0.11.1 is what this repository has adopted, and `cratestack check`
is the seventh gate in `just verify` (`just check-schema`). Every crate in
that release declares `rust-version = "1.98.0"`. The maintainer's decision was
to move the pin rather than pin CrateStack back to 0.8.15, the last release
that supports 1.95.0.

**No CrateStack library crate was added to this workspace** — the brief
excluded it, and `cargo metadata` still shows none.

## 1. The decisive test: necessary and sufficient

Necessary — under the old pin, exit **101**, before anything is compiled:

```
$ cargo +1.95.0 install cratestack-cli --version 0.11.1 --locked
    Updating crates.io index
error: cannot install package `cratestack-cli 0.11.1`, it requires rustc 1.98.0 or newer,
while the currently active rustc version is 1.95.0
`cratestack-cli 0.8.15` supports rustc 1.95.0
```

Sufficient — the same command run **from inside the worktree**, taking the new
pin off `rust-toolchain.toml`, exit **0**:

```
$ rustup show active-toolchain
1.98.0-x86_64-unknown-linux-gnu (overridden by '.../exp11-opus/rust-toolchain.toml')

$ cargo install cratestack-cli --version 0.11.1 --locked
   Compiling cratestack-cli v0.11.1
    Finished `release` profile [optimized] target(s) in 2m 51s
   Installed package `cratestack-cli v0.11.1` (executable `cratestack`)

$ .../bin/cratestack --version
cratestack 0.11.1
```

That pair is what retires the "installing it locally needs a compiler this
repository does not pin" paragraph in `docs/status.md`, and it is why
`justfile`'s `check-schema` failure message no longer teaches a `cd ~`
workaround.

## 2. The pin reaches the compiler

```
$ cargo --version        # inside the worktree, through rust-toolchain.toml
cargo 1.98.0 (797e8a9bc 2026-08-05)
$ rustc --version
rustc 1.98.0 (88d9e12ae 2026-08-18)
$ cargo +1.98.0 --version
cargo 1.98.0 (797e8a9bc 2026-08-05)
$ rustup show active-toolchain
1.98.0-x86_64-unknown-linux-gnu (overridden by '.../exp11-opus/rust-toolchain.toml')
$ cargo clippy --version
clippy 0.1.98 (88d9e12ae1 2026-08-18)
```

## 3. What moved, and what deliberately did not

| Place | Change |
|---|---|
| `rust-toolchain.toml` | `channel = "1.98.0"`; comment rewritten with the dated reason. `components`/`profile` untouched. |
| `backends/Dockerfile` | `FROM rust:1.98.0-alpine3.22 AS chef`, plus the header's version paragraph. |
| `.github/workflows/*.yml` | **nothing.** No literal to change — see below. |
| `Cargo.toml` `rust-version` | **unchanged at `1.88`**, re-derived rather than assumed — see §4. Its comment was corrected. |
| `Cargo.toml` `async-trait` note | said "not dyn-safe in Rust 1.95"; re-checked under 1.98 and rewritten. |
| `backends/crates/vpay-core/build.rs` | header said "the workspace pins Rust 1.95". |
| `justfile` `check-schema` | its failure message taught the `cd ~` workaround the bump removes. |
| `.github/workflows/ci.yml` | the `self-checks` comment asserting `cargo install` *would fail* here. |
| `docs/status.md`, `docs/roadmap.md`, `docs/runbooks/release.md` | dated corrections. |

### The workflows genuinely need no edit

Every Rust job in `ci.yml` (4 of them) and in `docs.yml` (1) reads the channel
out of the file:

```
- id: toolchain
  run: echo "channel=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" >> "$GITHUB_OUTPUT"
- uses: dtolnay/rust-toolchain@6c977a6ca4077a0ceb28ffbe03f59d46e9ac8772
  with:
    toolchain: ${{ steps.toolchain.outputs.channel }}
```

Confirmed by grep (`toolchain:` appears five times, always as that expression;
no version literal anywhere in `.github/workflows/`) and by running the
extraction by hand against the new file:

```
$ sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml
1.98.0
```

`actionlint` v1.7.12: exit 0 on `ci.yml`, and exit 0 over every workflow.

### The Alpine base deliberately did not move

Both tags exist on Docker Hub (`docker manifest inspect`, six architecture
entries each, checked 2026-09-05):

```
1.98.0-alpine3.22 -> EXISTS (6 arch entries)
1.98.0-alpine3.23 -> EXISTS (6 arch entries)
1.95.0-alpine3.22 -> EXISTS (6 arch entries)
```

`alpine3.22` was kept. An Alpine **major** bump changes musl and gcc under a
static `+crt-static` build; that is its own decision with its own evidence,
not a rider on a compiler bump. Worth recording that the alpine *patch* did
move, because the two `rust:` images were built at different times:

```
$ docker run --rm rust:1.95.0-alpine3.22 sh -c 'rustc --version; cat /etc/alpine-release'
rustc 1.95.0 (59807616e 2026-04-14)
3.22.4
$ docker run --rm rust:1.98.0-alpine3.22 sh -c 'rustc --version; rustc -vV | sed -n "s/^host: //p"; cat /etc/alpine-release'
rustc 1.98.0 (88d9e12ae 2026-08-18)
x86_64-unknown-linux-musl
3.22.5
```

The host triple is unchanged, which is what ADR-0014 and the Dockerfile's
`rustc -vV` `--target` trick depend on.

### One `FROM`, not two

The brief warned that the `chef` and builder stages share the base image pin
and must stay identical. They cannot drift: **only `chef` names the image**,
and `planner` and `builder` are both `FROM chef`. One literal covers all
three. The Dockerfile header now says so, so nobody "fixes" it into three.

Diff of the Dockerfile ignoring comments — a single line:

```
$ diff <(git show 046892a:backends/Dockerfile) backends/Dockerfile | grep '^[<>]' | grep -v '^[<>] #'
< FROM rust:1.95.0-alpine3.22 AS chef
> FROM rust:1.98.0-alpine3.22 AS chef
```

## 4. `rust-version`, re-derived (not guessed, and it did not move)

Per its own comment: max `rust_version` over every package in the resolved
graph, dev-dependencies included.

```
$ cargo metadata --format-version 1 [--all-features] | jq ...
distinct rust_version values: 1.0, 1.26, ..., 1.85.1, 1.87.0, 1.88, 1.88.0
total: 477, with rust_version: 342, without: 135
```

The maximum is **1.88.0**, so `rust-version = "1.88"` stays. Both with and
without `--all-features`, identically.

The crates that set the ceiling (the comment named only three of them):

```
darling 0.23.0 -> 1.88.0
darling_core 0.23.0 -> 1.88.0
darling_macro 0.23.0 -> 1.88.0
jsonwebtoken 11.0.0 -> 1.88.0
serde_with 3.21.0 -> 1.88
serde_with_macros 3.21.0 -> 1.88
testcontainers 0.27.3 -> 1.88
testcontainers-modules 0.15.0 -> 1.88
time 0.3.47 -> 1.88.0
time-core 0.1.8 -> 1.88.0
time-macros 0.2.27 -> 1.88.0
```

Two stale numbers were carried in three files and are corrected: the graph is
**477 packages, 135 with no `rust_version`**, not "63 of 317" (that was
2026-09-02). The MSRV is still metadata-derived and **has still never been
compiled with a 1.88 toolchain** — this bump did not change that and did not
pretend to.

## 5. New clippy lints between 1.95 and 1.98

`just clippy` (`cargo clippy --workspace --all-targets -- -D warnings`) over
260 crates produced **exactly one** new diagnostic:

```
error: can be more succinctly written as a byte str
   --> backends/crates/vpay-core/src/ids.rs:396:25
    |
396 |         for excluded in [b'i', b'l', b'o', b'u'] {
    |                         ^^^^^^^^^^^^^^^^^^^^^^^^ help: try: `*b"ilou"`
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#byte_char_slices
    = note: `-D clippy::byte-char-slices` implied by `-D warnings`
error: could not compile `vpay-core` (lib test) due to 1 previous error
```

Fixed by taking the lint's own suggestion: `[b'i', b'l', b'o', b'u']` →
`*b"ilou"`. Same type (`[u8; 4]`), same four bytes, loop body untouched, and
the test still asserts exactly what it asserted. **No `#[allow]` was added and
`clippy.toml` was not touched** — the brief's escape hatch was not needed
because the lint is right.

`cargo fmt --all -- --check` under 1.98.0's rustfmt: clean, no reformatting.

**A process note worth recording, because it nearly became a false green.**
The first clippy run was backgrounded as `just clippy > log 2>&1; echo
"EXIT=$?"`. The harness reported the background command as *"completed (exit
code 0)"* — the exit code of the trailing `echo`, not of the recipe. The log
said `error: Recipe 'clippy' failed on line 259 with exit code 101`. The
finding above is the thing that self-report would have hidden. Every exit code
in this file is read out of the log, not off a wrapper.

## 6. `just ci` under the pin — exit 0

Run **twice**: once on the working tree once every code change was in, and
again on the tree exactly as committed (`79b04f2`), because the docs edits
landed after the first run and `verify-links`/`verify-status` read them. Both
exited 0. The numbers below are the second run's — the committed one.

| Recipe | Result |
|---|---|
| `fmt-check` | `cargo fmt --all -- --check`, clean |
| `clippy` | clean over `--workspace --all-targets -- -D warnings` |
| `verify` | `verify: ok — the seven gates above passed; the verify-docs report is advisory` |
| ↳ `verify-no-mocks` | `ok — no test double reachable from a shipping binary` |
| ↳ `verify-status` | `ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code` |
| ↳ `verify-errors` | `ok — 15 error type(s), all classified; 14 #[from] variant(s) delegate every Classify method they match on; anyhow confined to binaries` |
| ↳ `verify-sdk-parity` | `ok — 342 proving test(s) named in docs/sdks/parity.md all exist, 26 dated gap(s)` |
| ↳ `verify-links` | `ok — 692 repository link(s) in 122 tracked markdown file(s) resolve to a tracked path` (691/121 on the first run, before this file was tracked) |
| ↳ `verify-npm-scope` | `ok — 2 publishable package(s) under sdks/ ... and no retired package name outside docs/plans, docs/adr and docs/status.md` |
| ↳ `check-schema` | `cratestack 0.11.1, schema schemas/vpay.cstack (12 model/enum declarations, datasource present)` then `ok — schemas/vpay.cstack type-checks under cratestack 0.11.1` |
| `test-rust` | `Summary [971.509s] 1220 tests run: 1220 passed, 0 skipped` (1083.216 s on the first run) |
| `test-doc` | 86 passed, 0 failed, **1 ignored**, across 14 doc-test binaries |
| `verify-ignored` | `0 ignored (expected 0), 42 test binaries (expected 42), 1220 total (minimum 1080)` |
| `lint-web` | `pnpm -r typecheck` + `pnpm -r lint`, 15 of 16 projects + `examples/shop`, exit 0 |
| `test-web` | every vitest suite passed (checkout 17 files, nodejs 9, stripe-js 8, shop 7, four packages 1 each) |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` |

**`0 skipped`, not "skipped because Docker was missing":** the container-backed
suites ran. `vpay-tests-integration::worker_e2e` and
`vpay-tests-conformance::adapter_conformance` are in that 1220 with real
Postgres and WireMock containers.

## 7. Docker — environment parity

Both builds from the **same source tree** on the **same isolated builder**;
the only difference is the `FROM` line, so the comparison isolates the
compiler. (`backends/crates/vpay-core/src/ids.rs`'s change is inside
`#[cfg(test)]`, so it cannot reach either release binary.)

```
$ docker buildx build --builder vpay-exp11-opus \
    -f <046892a's backends/Dockerfile> --target server -t vpay-exp11-opus:server-1950 --load .
BEFORE_EXIT=0 elapsed=355s

$ docker buildx build --builder vpay-exp11-opus \
    -f backends/Dockerfile --target server -t vpay-exp11-opus:server-1980 --load .
AFTER_SERVER_EXIT=0 elapsed=306s

$ docker buildx build --builder vpay-exp11-opus \
    -f backends/Dockerfile --target worker -t vpay-exp11-opus:worker-1980 --load .
AFTER_WORKER_EXIT=0 elapsed=5s          # builder stage already cached by the run above
```

| | built on 1.95.0 | built on 1.98.0 |
|---|---|---|
| `docker images` SIZE | 15.9 MB | 15.7 MB |
| `/vpay-server` in the image | 10,873,248 B | 10,783,136 B (−90,112 B, −0.83 %) |
| layers | 2 | 2 |
| `config/` layer | 28.7 kB | 28.7 kB |
| `docker run --rm <img> --version` | `vpay-server 0.1.0`, exit 0 | `vpay-server 0.1.0`, exit 0 |

`vpay-exp11-opus:worker-1980` is 12.6 MB and `docker run --rm ... --version`
prints `vpay-worker-bin 0.1.0`, exit 0.

**The two wall-clocks are NOT a matched pair and must not be read as one.**
355 s and 306 s were single runs on a host another agent was building on
concurrently, with different cache-mount warmth (the second run's registry
cache mount was populated by the first). The size numbers are stable and
meaningful; the timings are not. Nobody re-measured the cargo-chef
cold/warm pairs from `docs/plans/exp8-notes/` on the new base, and
`docs/status.md`'s Dockerfile row now says so.

The builder and the three images were removed afterwards; the shared default
buildx builder was never touched or pruned.

## 8. What this pass did NOT do

* **No `arm64` build.** Expected by the brief. The `aarch64-unknown-linux-musl`
  half of every published manifest list is still only evidenced by the release
  runs recorded in `docs/runbooks/release.md` §6, all of which were built on
  `rust:1.95.0-alpine3.22`.
* **No CI run of this change exists.** Every number here is from one authoring
  host. `actionlint` is not GitHub Actions.
* **The 1.88 MSRV is still uncompiled.** Re-derived, not verified. A 1.88
  toolchain was never installed and never run.
* **`CLAUDE.md` still says the pin is `1.95.0`** (its "Things that will waste
  your time" section). This pass was not authorised to edit that file, so the
  stale line is recorded in `docs/status.md` and here instead of being
  silently fixed or silently dropped. It is the only place left in the tree
  that names the old pin as current.
* **No CrateStack crate was added to the workspace**, per the brief. The bump
  is what makes the *tooling* installable here; it is not what makes the
  workspace compile.
* **`docs/plans/*-notes/` were not rewritten.** Their `1.95.0` strings are
  dated records of runs that really happened on that compiler, and so is
  `docs/runbooks/release.md` §6's account of release run `33929374661`.

## 9. Rebased onto `02ae5cc` (2026-09-05)

**Everything above was measured on base `046892a`. This branch was rebased
onto `origin/master` `02ae5cc` — the merge of PR #42, ADR-0016's engineering
standards — later the same day, and the numbers in §6 and §7 were *not*
re-measured by that rebase; §9 is what was.** The rebase is why
`verify-toolchain` is described above as the eighth gate and is the **tenth**
in the tree you are reading.

PR #42 added `verify-serde` and `verify-repositories` as the eighth and ninth
gates, in the same five files this branch edits. Every conflict was resolved
by **keeping both**, never by choosing a side:

* `justfile` — `verify` now depends on all ten in master's order with
  `verify-toolchain` appended after `verify-repositories`, before the advisory
  `verify-docs`. The recipe bodies were reordered to match the gate order.
  Appending rather than inserting is deliberate: it keeps every ordinal
  already written down elsewhere true, `check-schema` included, which four
  other files call the seventh gate.
* `.github/workflows/ci.yml` — `self-checks` carries master's two new steps
  **and** this branch's. The `verify-toolchain` step was changed from
  `cargo xtask verify-toolchain` to `just verify-toolchain`, matching what
  ADR-0016's two steps and `check-schema` do; the paragraph justifying the
  `cargo xtask` spelling was rewritten rather than left to contradict the
  step beneath it.
* `.xtask/src/main.rs` — both sides' new functions and both sides' test
  modules coexist. The end-of-file conflict was the dangerous one: git had
  unified the trailing `}` of master's last test and this branch's, so
  neither marker-delimited block was a whole module. Both were reconstructed
  and then checked byte-for-byte against `origin/master` and `a30fff8`.
* `AGENTS.md`, `docs/status.md`, `rust-toolchain.toml`, `backends/Dockerfile`
  — every "nine gates"/"ninth" from either side became ten/tenth.

Re-run on the rebased tree, not carried over:

* `just ci` — **exit 0**. `cargo nextest run --workspace`:
  **1270 tests run, 1270 passed, 0 skipped** in 761.5 s. `just test-doc`: 86
  passed, 1 ignored. `just verify-ignored`: 0 ignored (expected 0), 42 test
  binaries (expected 42), 1270 total (minimum 1080). `cargo deny`:
  advisories ok, bans ok, licenses ok, sources ok.
* `cargo test -p xtask` — **194 passed, 0 failed, 0 ignored**: master's 184
  plus this branch's 10, which is the arithmetic that shows no test of either
  side was lost in the merge.
* `just verify` — the ten gates in order, then the advisory report.
* `actionlint .github/workflows/ci.yml` — exit 0.
* Both decisive mutations, re-run on this tree; see §9.1.

### 9.1 The two mutations, re-run after the rebase

`backends/Dockerfile`'s `FROM rust:` set back to 1.95.0 with
`rust-toolchain.toml` still at 1.98.0 — the gate fails, naming the file, the
line and both versions:

```
cargo xtask verify-toolchain
xtask: 1 toolchain pin(s) out of step:
  - backends/Dockerfile:197: `FROM rust:1.95.0-alpine3.22` builds with 1.95.0, but rust-toolchain.toml pins `channel = "1.98.0"` — every Rust job in CI reads that file, so this image would be the one thing in the repository compiled by a different compiler
error: Recipe `verify-toolchain` failed on line 752 with exit code 1
```

The nine gates before it all printed `ok` in that same run, which is the
point of the gate: nothing else in `just verify` notices.

A scratch crate whose only dependency is `cratestack-core = "0.11.1"`, under
each compiler — the test that makes the bump necessary and sufficient:

```
$ cargo +1.95.0 check
    Updating crates.io index
     Locking 96 packages to latest compatible versions
      Adding cratestack-core v0.11.1 (requires Rust 1.98.0)
error: rustc 1.95.0 is not supported by the following package:
  cratestack-core@0.11.1 requires rustc 1.98.0
Either upgrade rustc or select compatible dependency versions with
`cargo update <name>@<current-ver> --precise <compatible-ver>`
where `<compatible-ver>` is the latest version supporting rustc 1.95.0
exit = 101

$ cargo +1.98.0 check
    Checking cratestack-core v0.11.1
    Checking exp11-land-msrv v0.0.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.96s
exit = 0
```

Both files were restored afterwards and the tree is clean.

### 9.2 Still not done, after the rebase

The three gaps in §8 are unchanged by it: **no `arm64` build**, **no CI run
of this change**, and **the 1.88 MSRV is still uncompiled** (re-derived
numerically, never built). §8's fourth bullet — `CLAUDE.md` still naming
`1.95.0` — was fixed by this branch's own review commit and is no longer
outstanding.
