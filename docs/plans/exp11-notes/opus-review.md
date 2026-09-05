# exp11 (opus arm) — sabotage review of the 1.95.0 → 1.98.0 toolchain bump

Date: 2026-09-05. Reviewed tree: `claude/exp11-toolchain-opus`, base `046892a`,
implementation `git diff 046892a..9694786` (two commits). The implementer's own
account is [opus.md](opus.md); this file records what was re-run, what was
found, and what was changed.

Host: the authoring machine. `CARGO_BUILD_JOBS=4`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, Node `v22.23.2` from `.nvmrc`
(the exact version CI installs), `pnpm install --frozen-lockfile` before
`just test-rust`. **Another agent was running `just ci` and Docker builds on
this host throughout**, which is why no wall-clock here is offered as a
measurement of anything.

Every exit code in this file was read out of a file the command wrote itself,
never off a wrapper — the implementer's §5 records a background harness
reporting "completed (exit code 0)" for a run that failed, and the same harness
did the same thing to this review twice.

## 1. Verdict

**The implementation is correct and safe as delivered.** Every claim in
[opus.md](opus.md) that this review re-ran reproduced, including the two that
are easiest to fake (the MSRV refusal under 1.95.0, and `0 skipped` on a
container-backed suite). Nothing was reverted.

Six findings were fixed on top of it. Five are stale, self-contradicting or
imprecise claims; the sixth is the gap the brief pointed at — **nothing in the
repository enforced the one coupling this whole task is about**, and the
mismatch was measured to pass the entire `just ci`.

## 2. Findings

| # | Severity | Finding | Fix |
|---|---|---|---|
| F1 | **robustness** | Nothing enforced "bump both together". With `channel = "1.98.0"` and `backends/Dockerfile` left at `FROM rust:1.95.0-alpine3.22`, `just verify` **exited 0** and `just fmt-check` **exited 0** (mutation M1). No other `just ci` recipe reads either file — nothing here compiles the Dockerfile — so the drift is invisible until a release image is built by a compiler no local run and no CI job ever used. | `cargo xtask verify-toolchain`, the eighth gate in `just verify` and a step in CI's `self-checks`. Ten tests, four of them written from mutations of the gate itself. M2 proves it fires. |
| F2 | misleading-claim | `justfile`'s `check-schema` rationale still said `just install-rust` omits the CrateStack CLI because *"installing it needs a newer compiler than `rust-toolchain.toml` pins"* — false after the bump, and contradicted by the recipe's own failure message fifty lines below, which the same commit had rewritten. | Sentence rewritten; the *behaviour* (`install-rust` not installing it) is left alone as a maintainer's call. |
| F3 | rule-break | `CLAUDE.md` "Things that will waste your time" still said the pin is `1.95.0`. The implementer recorded this as a deliberate omission for want of authorisation; this review was authorised to fix it. | Corrected to `1.98.0`, with the date it moved and a pointer to the new gate. |
| F4 | misleading-claim | `rust-toolchain.toml`'s header said the image version is *"named TWICE there — the `chef` stage and the `builder` stage that is `FROM chef` — and the two must stay identical"*. `FROM chef` names no version; the same commit's Dockerfile header and notes say (correctly) that only `chef` names it. A reader following the toolchain file would go looking for a second literal to bump. | Corrected, and it now names the gate that enforces the coupling. |
| F5 | nit | "six architecture entries" in `backends/Dockerfile` and `docs/status.md`. `docker manifest inspect rust:1.98.0-alpine3.22` returns six *manifest* entries, of which **three are architectures** (`linux/amd64`, `linux/arm64/v8`, `linux/ppc64le`) and three are `unknown/unknown` attestation manifests. The claim's substance holds — arm64 is there, and the 1.95.0 tag's set is identical — but the number counts the wrong thing. | Both files now say what the six are. |
| F6 | misleading-claim | `docs/status.md` said `CLAUDE.md` was *"the one place in the tree that still names the old pin as current"*. F2 was a second. | Paragraph rewritten around both, now that both are fixed. |

Nothing was found that changes behaviour of shipping code. The only source
change in the whole branch is inside a `#[cfg(test)]` module (F-none: the
`byte_char_slices` fix, checked below).

## 3. The claims, re-run

### 3.1 The pin reaches the compiler

```
$ cargo --version           # inside the worktree, through rust-toolchain.toml
cargo 1.98.0 (797e8a9bc 2026-08-05)
$ rustc --version
rustc 1.98.0 (88d9e12ae 2026-08-18)
$ cargo clippy --version
clippy 0.1.98 (88d9e12ae1 2026-08-18)
$ rustup show active-toolchain
1.98.0-x86_64-unknown-linux-gnu (overridden by '.../exp11-opus/rust-toolchain.toml')
```

### 3.2 CI really reads the channel out of the file

The workflows' own `sed`, run by hand against the new file:

```
$ sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml
1.98.0
```

`toolchain:` appears five times across `.github/workflows/` (four jobs in
`ci.yml`, one in `docs.yml`) and every one is
`${{ steps.toolchain.outputs.channel }}`; the only `1.9x` strings anywhere in
that directory are in the `self-checks` comment the bump rewrote. `actionlint`
v1.7.12 over `ci.yml` and over every workflow after this review's edit: exit 0
both times.

### 3.3 One `FROM`, not two — and the toolchain file said otherwise

```
$ grep -n '^FROM' backends/Dockerfile
187:FROM rust:1.98.0-alpine3.22 AS chef
197:FROM chef AS planner
206:FROM chef AS builder
295:FROM scratch AS server
315:FROM scratch AS worker
```

The Dockerfile header and the implementer's notes are right; `rust-toolchain.toml`'s
header contradicted both (F4).

### 3.4 The Docker Hub tags

```
$ docker manifest inspect rust:1.98.0-alpine3.22 | jq -r '.manifests[].platform | ...'
      1 linux/amd64
      1 linux/arm64/v8
      1 linux/ppc64le
      3 unknown/unknown
```

Identical sets for `rust:1.95.0-alpine3.22` and `rust:1.98.0-alpine3.23`. Both
of the bump's substantive claims hold — the tag exists with an arm64 entry, and
the Alpine base did not have to move — and the *count* was wrong (F5).

### 3.5 `rust-version = "1.88"`, re-derived independently

```
$ cargo metadata --format-version 1 | jq '{total, with_rust_version, without}'
{ "total": 477, "with_rv": 342, "without_rv": 135 }
```

Max over the graph is **1.88.0**. The external crates that set it are exactly
the eleven the corrected comment names (`darling`/`darling_core`/`darling_macro`
0.23.0, `jsonwebtoken` 11.0.0, `serde_with`/`serde_with_macros` 3.21.0,
`time` 0.3.47, `time-core` 0.1.8, `time-macros` 0.2.27, `testcontainers`
0.27.3, `testcontainers-modules` 0.15.0). "135 of 477" reproduces exactly;
"63 of 317" was indeed stale. `rust-version` correctly did not move.

### 3.6 The async-trait note, re-checked

The bump's new comment claims a trait with `async fn` is still not
dyn-compatible under 1.98.0, failing with E0038 "…because method `call` is
`async`". Compiled here, `rustc +1.98.0 --edition 2024`:

```
error[E0038]: the trait `Port` is not dyn compatible
2 |     async fn call(&self) -> u8;
  |              ^^^^ ...because method `call` is `async`
```

Claim holds, verbatim.

### 3.7 The clippy fix is semantically identical

`for excluded in *b"ilou"` — `*b"ilou"` dereferences a `&'static [u8; 4]` to
`[u8; 4]`, which is the same type the array literal had, and `IntoIterator for
[u8; 4]` yields `u8` exactly as before; `ALPHABET.contains(&excluded)` and the
message are untouched. The whole change is inside `#[cfg(test)]`, so it cannot
reach a shipping binary. The test runs and passes in the suite below
(`vpay-core ids::tests::the_alphabet_is_crockfords_and_every_five_bit_value_maps_into_it`).

**Were there other new lints?** `cargo clippy --workspace --all-targets -- -D
warnings` (which is exactly what `just clippy` runs) is green on this tree
under 1.98.0 — as part of the full `just ci` in §5, and again after this
review's changes.

### 3.8 The decisive negative, re-run

Necessary, from inside the worktree, into a throwaway `--root` so it cannot be
short-circuited by the already-installed binary:

```
$ cargo +1.95.0 install cratestack-cli --version 0.11.1 --locked --root <tmp>
    Updating crates.io index
error: cannot install package `cratestack-cli 0.11.1`, it requires rustc 1.98.0 or newer,
while the currently active rustc version is 1.95.0
`cratestack-cli 0.8.15` supports rustc 1.95.0
EXIT_1950=101
```

Sufficient, re-run by this review from inside the worktree, under the pin,
into a private `--root` (see §7 for why a private one):

```
$ rustup show active-toolchain
1.98.0-x86_64-unknown-linux-gnu (overridden by '.../exp11-opus/rust-toolchain.toml')
$ cargo install cratestack-cli --version 0.11.1 --locked --root <scratch>
   Installed package `cratestack-cli v0.11.1` (executable `cratestack`)
INSTALL_EXIT=0
$ <scratch>/bin/cratestack --version
cratestack 0.11.1
```

Both directions of the bump's decisive test therefore reproduce
independently: it fails under 1.95.0 and succeeds under 1.98.0, from inside
this checkout.

### 3.9 Did the suite shrink?

The review brief carried "1257 tests on master 046892a". **It is 1220.**
Measured on both trees, each under its own pin, listing rather than running:

| tree | toolchain | `cargo nextest list --workspace` | binaries |
|---|---|---|---|
| `046892a` (base), extracted with `git archive` into a scratch dir with its own `CARGO_TARGET_DIR` | 1.95.0 (its own `rust-toolchain.toml`) | **1220** | **42** |
| `9694786` (this branch) | 1.98.0 | **1220** | **42** |

The counts are equal, as they must be: the branch adds and removes no `#[test]`
— its only Rust change is one loop expression inside an existing test function.
Nothing was lost; the brief's number was wrong. (`verify-ignored`'s floor is
`min_tests = 1080` and `expected_suites = 42`, so a 37-test loss would *not*
have been caught by the binary count — only by the floor, which 1220 clears.)

## 4. Mutations

| # | Mutation | Before this review | After |
|---|---|---|---|
| M1 | `channel = "1.98.0"`, `FROM rust:1.95.0-alpine3.22` | `just verify` **exit 0**, `just fmt-check` **exit 0** — nothing caught it | — |
| M2 | the same mutation, with `verify-toolchain` in place | — | `just verify` **exit 1**: `backends/Dockerfile:187: FROM rust:1.95.0-alpine3.22 builds with 1.95.0, but rust-toolchain.toml pins channel = "1.98.0"` |
| M3a | gate's version comparison always agrees (`if true \|\| …`) | — | `a_dockerfile_left_on_the_old_compiler_fails` **FAIL** (and one more) |
| M3b | vacuity guard removed (an empty Dockerfile accepted) | — | `a_dockerfile_with_no_rust_image_fails_rather_than_passing_vacuously` **FAIL** |
| M3c | comment filter removed | — | **all ten still pass** — see below |
| M3d | keyword match made case-sensitive | — | first run: **all ten passed** (a false green in this review's own test); after the fix: `a_lower_case_from_is_still_an_instruction` **FAIL** |

**M3c is reported as a miss, not as a pass.** A comment's `#` displaces the
`FROM` keyword by one character, so no comment can be read as an instruction
whether the filter is there or not: the filter is belt-and-braces and both its
doc comment and its test now say so, rather than the test implying it caught
something.

**M3d found a false green in this review's own work**, which is the reason it
is written down: `a_lower_case_from_is_still_an_instruction` originally
asserted only `expect_err`. Under a case-sensitive keyword match the lower-case
line stopped being an instruction, the *vacuity* guard fired instead, and the
mutation read green. The test now asserts the message is the version mismatch.

## 5. `just ci`, recipe by recipe

Run end to end twice: once on the implementation as delivered (`9694786`), once
on the final tree of this review. Both from a script that writes `$?` to a
file. The first run:

| Recipe | Result on `9694786` |
|---|---|
| `fmt-check` | clean |
| `clippy` | clean, `--workspace --all-targets -- -D warnings` |
| `verify` | `verify: ok — the seven gates above passed` |
| ↳ `verify-no-mocks` | `ok — no test double reachable from a shipping binary` |
| ↳ `verify-status` | `ok — 1 unimplemented item(s), all declared` |
| ↳ `verify-errors` | `ok — 15 error type(s), all classified; 14 #[from] variant(s)` |
| ↳ `verify-sdk-parity` | `ok — 342 proving test(s) …, 26 dated gap(s)` |
| ↳ `verify-links` | `ok — 692 repository link(s) in 122 tracked markdown file(s)` |
| ↳ `verify-npm-scope` | `ok — 2 publishable package(s) …` |
| ↳ `check-schema` | `cratestack 0.11.1 … ok — schemas/vpay.cstack type-checks` |
| `test-rust` | `Summary [806.131s] 1220 tests run: 1220 passed, 0 skipped` |
| `test-doc` | 86 passed, 1 ignored, 0 failed, across 14 doc-test binaries |
| `verify-ignored` | `0 ignored (expected 0), 42 test binaries (expected 42), 1220 total (minimum 1080)` |
| `lint-web` | exit 0 |
| `test-web` | every vitest suite passed |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` |
| **exit** | **0** (read from the file the runner wrote, 20 min 14 s wall clock) |

`0 skipped` is the load-bearing part and it reproduced: the container-backed
suites ran, with `vpay-tests-conformance::adapter_conformance` and
`vpay-tests-integration::*` in the count against real WireMock and Postgres
containers.

The second run, on the final tree of this review (`HEAD` after all six
commits), also exited **0** — 17 min 30 s wall clock, again read from a file
the runner wrote:

| Recipe | Result on the final tree |
|---|---|
| `fmt-check` | clean |
| `clippy` | clean, `--workspace --all-targets -- -D warnings` |
| `verify` | `verify: ok — the eight gates above passed` |
| ↳ `verify-links` | `ok — 695 repository link(s) in 123 tracked markdown file(s)` |
| ↳ `verify-toolchain` | `ok — rust-toolchain.toml pins 1.98.0 and all 1 FROM rust: instruction(s) in backends/Dockerfile name it (rust:1.98.0-alpine3.22)` |
| ↳ `check-schema` | **ran against the wrong CLI — see §7**; re-run against the pin: `ok — schemas/vpay.cstack type-checks under cratestack 0.11.1` |
| `test-rust` | `Summary [961.863s] 1230 tests run: 1230 passed, 0 skipped` (1220 + the ten this review adds) |
| `test-doc` | 86 passed, 1 ignored, 0 failed |
| `verify-ignored` | `0 ignored (expected 0), 42 test binaries (expected 42), 1230 total (minimum 1080)` |
| `lint-web`, `test-web` | exit 0; every vitest suite passed |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` |
| **exit** | **0** |

The ten new tests add no test *binary* (they are in `.xtask`, which already
had one), so `expected_suites` stays 42; `min_tests` is a floor and 1230
clears it, so neither pin in `justfile` moved.

## 7. Docker, and one piece of shared-host interference

Built from the final tree on a dedicated `docker-container` builder
(`vpay-exp11-opus-review`, removed afterwards; the shared default builder was
never touched or pruned):

```
BUILD_server_EXIT=0 elapsed=375s
BUILD_worker_EXIT=0 elapsed=5s      # builder stage cached by the run above
vpay-exp11-opus-review:server 15.7MB
vpay-exp11-opus-review:worker 12.6MB
$ docker run --rm vpay-exp11-opus-review:server --version
vpay-server 0.1.0                   # exit 0
$ docker run --rm vpay-exp11-opus-review:worker --version
vpay-worker-bin 0.1.0               # exit 0
$ docker run --rm rust:1.98.0-alpine3.22 rustc --version
rustc 1.98.0 (88d9e12ae 2026-08-18)
```

Both sizes reproduce the implementer's numbers exactly (15.7 MB / 12.6 MB).
The elapsed times are **not** comparable to anything: another agent was
building on this host at the same time, and the second build reused the
first's stages.

**The interference worth recording.** Between the two `just ci` runs, the
shared `~/.cargo/bin/cratestack` was replaced by another process on this host
(file mtime 14:17; this review's first run at 13:36 saw 0.11.1, the second at
14:18 saw **0.7.15**). The `check-schema` recipe behaved exactly as its
comments promise — it printed

```
check-schema: WARNING — cratestack 0.7.15 on PATH, this repository pins 0.11.1.
check-schema: the check below still ran in full, but against the 0.7.15 grammar.
```

and still exited 0, because that recipe reports the version rather than
enforcing it locally, by design. So `just ci` was legitimately green while one
gate's evidence was against the wrong grammar — which is why this review
installed the pinned CLI into a private `--root`, put only that on `PATH`, and
re-ran the recipe (green at 0.11.1, above). **The shared binary was left as it
was found**, because another agent may be relying on it and swapping it back
would be doing to them what was done to this run. Two observations follow:
a local gate whose version is "reported, not enforced" is one shared-host
interference away from evidence about a different grammar, and the recipe's
loud warning is the only reason anybody would notice.

## 8. What this review did not do

* **No `arm64` build**, exactly as the brief allows. The arm64 half of the
  manifest is evidenced only by the tag's platform list, not by a build.
* **No CI run of any of this exists.** `actionlint` is not GitHub Actions.
* **The 1.88 MSRV is still uncompiled** — re-derived, never built.
* **`docs/plans/*-notes/` were not rewritten** — they are dated records.
* **`docs/status.md`'s historical measurement sentences still say "with the
  toolchain pinned to `1.95.0`"** and must: those runs happened on that
  compiler. None of them is a claim about the current pin.
