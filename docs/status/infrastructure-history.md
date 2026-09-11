# Docker, `cargo deny`, the toolchain pin and the migration manifest

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

### Docker / compose — made bootable, proven by CI run `33647189156`

Both Dockerfiles and `compose.e2e.yml` date from 2026-08-09 (musl host
target, non-root UID 65532, `.dockerignore`). Two days later `--config` /
`VPAY_CONFIG` became mandatory in both binaries, and nothing in the image
or the compose file supplied it — so from 2026-08-11 to 2026-09-02 the
"never built" stack was also a stack that could not have booted if it had
been. A previous version of this section said the files were "rewritten
this pass"; `git log` says otherwise, and this section now says what the
files actually are.

What changed on 2026-09-02, and what each change is proven by:

- `backends/Dockerfile` bakes `config/` into both runtime stages and sets
  `VPAY_CONFIG`. Proven by CI's `e2e (compose)` job on run `33647189156`.
- `compose.e2e.yml` sets `VPAY_CONFIG` and the three rail `${VAR}`
  placeholders on both services. `docker compose config` renders it, and
  the same CI run proves the processes boot behind it.
- `config/application.yml` names the real WireMock service hosts. Proven by
  `vpay-config`'s `a_valid_config_loads_and_produces_the_expected_typed_values`,
  which loads the real file.
- No `HEALTHCHECK` was added to the `scratch` image — there is still no
  executable in it that could run one, and the `--healthcheck` self-check
  mode `compose.e2e.yml` describes was not built. CI observes readiness
  from outside by polling `/healthz` instead.

**Corrected 2026-09-03 (Step 6, block A): all three images have now been
built on an authoring machine.** This section used to say "none of it was
built on an authoring machine" — Docker Hub unreachable from one, a
rootless daemon that could not start a container on the other, and
`just build-dist` failing there at `ring`'s C build for want of an
`x86_64-linux-musl-gcc` cross compiler. The daemon works now, and
`just release-dry-run` built `vpay-server`, `vpay-worker` and
`vpay-dashboard` for `linux/amd64`, exit 0 (see the `just release-dry-run`
row above for the digests and the timing). The cross-linker problem never
applied inside the builder anyway — the alpine image's `cc` is musl-native,
exactly as the Dockerfile's header says; it applies to `just build-dist`
run on the host, and that is still unbuilt here. **What this does and does
not change:** the images build locally, which nothing had shown before. It
says nothing about whether they boot — no local run has started a
container from them, and CI's `e2e (compose)` job remains the only evidence
for that. The rows above cite CI run
`33647189156` (the pull request) and `33650294682` (the first run on
`master`) as their evidence; ✅ means exactly "the images build, the stack
boots, `/healthz` answers 200 and the one Cypress spec passes," and nothing
about what the stack can do once up — it still serves only `/healthz`.

### `cargo deny` — fixed properly, not suppressed

`cargo deny check` previously failed and is now clean, without adding a single
`ignore` entry to `deny.toml` (`ignore = []`, confirmed by reading the file).
Two real dependency upgrades did the work:

- `time` 0.3.45 → 0.3.47, a production dependency (reachable from
  `vpay-core` via `sqlx`'s `time` feature, not only from `dev-dependencies`),
  addressing a `time`-crate advisory reported as RUSTSEC-2026-0009.
- `testcontainers` 0.23 → 0.27 and `testcontainers-modules` 0.11 → 0.15 (a
  test-only dependency), which moved onto `bollard` 0.20. That drops
  `rustls-pemfile` entirely (RUSTSEC-2025-0134) and replaces the unmaintained
  `tokio-tar` with the maintained `astral-tokio-tar` fork (RUSTSEC-2025-0111).
  Both advisory IDs are cited in `Cargo.toml`'s own comment next to the
  `testcontainers` pin.

**2026-09-02:** `cargo deny check` had regressed to `advisories FAILED` on
`master` as the advisory database moved, not because of any vpay change:
RUSTSEC-2026-0258 (`h2` 0.4.15, unbounded empty DATA frames, reachable
through `reqwest`/`hyper` in every shipping binary) and a yanked `chacha20`
0.10.1 (dev-only, via `testcontainers → ferroid → rand`). Both fixed the same
way as before — `cargo update -p h2 -p chacha20` to 0.4.19 / 0.10.2 — with
`ignore` still holding only the `rsa` entry above. The `authkestra` upgrade to
0.7.1 added one new crate to the graph, `authkestra-crypto-util` (MIT OR
Apache-2.0); `cargo tree -i aws-lc-rs`/`-i aws-lc-sys`/`-i openssl-sys`/
`-i native-tls` all still report no match, so the single-`ring`-provider and
rustls-only invariants survived the bump.

**Also 2026-09-02, from the SDK work:** the workspace's `reqwest` pin was
`0.12`, and nothing had ever consumed it — every `reqwest` in the lock was
`0.13.4` via `authkestra-*`. `vpay-sdk` became the pin's first consumer,
which would have compiled two HTTP+TLS stacks (`cargo deny` warns on
duplicate majors but does not fail). The pin moved to `0.13` with
`rustls-no-provider` (0.13 has no `ring`-flavoured feature — its `rustls`
feature means aws-lc-rs, which `deny.toml` bans); `cargo tree -d` now shows
one `reqwest`, and the four "no match" checks above were re-run after the
move. The pre-existing duplicate `webpki-roots` (0.26 via `sqlx-core`, 1.0
via `hyper-rustls`) is unchanged and predates all of this.

`rust-version` moved `1.85` → `1.88` in the same pass, computed as the max
`rust_version` declared anywhere in the resolved dependency graph (`cargo
metadata`, including dev-dependencies). **Read the comment block at the top of
`rust-toolchain.toml` before trusting that number**: it states plainly that
1.88 has **not** been verified by actually compiling with a 1.88 toolchain,
and that a large minority of the graph declares no `rust_version` at all, so
the true floor could in principle be higher. **Re-derived 2026-09-05** during
the toolchain bump below and it did **not** move — still 1.88. Two figures in
this paragraph did: it used to say "only stable 1.95.0 was available here"
(the pin is 1.98.0 now) and "63 of 317 packages", which was the graph as it
stood on 2026-09-02; today's `cargo metadata` reports **135 of 477** packages
with no `rust_version`. The crates that set the 1.88 ceiling are now
`darling` 0.23.0, `jsonwebtoken` 11.0.0, `serde_with` 3.21.0, `time` 0.3.47
(with `time-core` 0.1.8 and `time-macros` 0.2.27) and `testcontainers` 0.27.3
/ `testcontainers-modules` 0.15.0 — the old comment named only `time` and the
two `testcontainers` crates.

### Toolchain pin — `1.95.0` → `1.98.0` (2026-09-05)

**Why it moved.** CrateStack 0.11.1 is what this repository had adopted when
this pin moved (0.12.0 since 2026-09-07, which declares the same floor), and
`cratestack check` is the seventh gate in `just verify`. Every crate in that
release declares `rust-version = "1.98.0"`. Under the old pin, obtaining the
tool a gate depends on required stepping outside the checkout or taking a
prebuilt binary — for which **Linux musl has none**. The maintainer's
decision was to move the pin rather than pin CrateStack back to 0.8.15, the
last release that supports 1.95.0.

**Proven in both directions, on the authoring host, 2026-09-05** — this is
the evidence that the bump is both necessary and sufficient, not an
assumption:

```
$ cargo +1.95.0 install cratestack-cli --version 0.11.1 --locked   # exit 101
error: cannot install package `cratestack-cli 0.11.1`, it requires rustc 1.98.0 or newer,
while the currently active rustc version is 1.95.0
`cratestack-cli 0.8.15` supports rustc 1.95.0

$ cargo install cratestack-cli --version 0.11.1 --locked   # inside the worktree, exit 0
Installed package `cratestack-cli v0.11.1` (executable `cratestack`)
```

**What moved, and what deliberately did not.** `rust-toolchain.toml`'s
`channel`; `backends/Dockerfile`'s single `FROM rust:1.98.0-alpine3.22`
(`planner` and `builder` are both `FROM chef`, so one literal covers all
three stages and they cannot drift). **No workflow file names a compiler
version** — every Rust job in `ci.yml` and `docs.yml` already `sed`s
`channel` out of `rust-toolchain.toml`, so nothing there had to change and
the extraction was re-run by hand to confirm it yields `1.98.0`. The **Alpine
base did not move**: `rust:1.98.0-alpine3.22` exists on Docker Hub
(`docker manifest inspect`: six manifest entries, of which **three are
architectures** — `linux/amd64`, `linux/arm64/v8`, `linux/ppc64le` — and three
are `unknown/unknown` attestation manifests; the 1.95.0 tag carries exactly
the same six, and the arm64 one is what `release.yml` needs. This page and
the Dockerfile read "six architecture entries" until the review pass of the
same day counted them), so this changes the compiler and nothing else about the build
environment. `rust:1.98.0-alpine3.23` also exists and was **not** taken — an
Alpine major bump changes musl and gcc under a static build and deserves its
own evidence rather than riding along on a compiler bump. `Cargo.toml`'s
`rust-version` did not move either (see the paragraph above).

**One new clippy lint fired**, workspace-wide, over `--all-targets`:
`clippy::byte_char_slices` at `backends/crates/vpay-core/src/ids.rs:396`, on
the test that asserts the four Crockford excludes are absent from the
alphabet. Fixed by taking the lint's own suggestion — `[b'i', b'l', b'o',
b'u']` → `*b"ilou"`, the same `[u8; 4]`, loop body untouched — **not** by an
`#[allow]`, and `clippy.toml` was not touched. That was the only new
diagnostic in the whole workspace; `cargo fmt --all -- --check` was clean
under 1.98.0's rustfmt with no reformatting.

**Two stale claims were left behind by the bump itself; both are fixed** (its
sabotage review, the same day). `CLAUDE.md`'s "Things that will waste your
time" section said _"`rust-toolchain.toml` pins `1.95.0`"_; the bump had no
authorisation to edit that file and recorded it here instead, and the review —
which did — corrected the number. The bump also claimed CLAUDE.md was **"the
one place in the tree that still names the old pin as current"**, and it was
not: `justfile`'s `check-schema` rationale still said `just install-rust`
leaves the CrateStack CLI out because _"installing it needs a newer compiler
than `rust-toolchain.toml` pins"_ — false since the bump, and contradicted by
the same recipe's failure message fifty lines below it, which that very commit
had rewritten. Both are corrected. Nothing in the tree now names 1.95.0 as
current; the remaining `1.95.0` strings are dated `docs/plans/*-notes/` records
and explicitly historical sentences, where they are correct.

**"Bump both together" became a gate, because the mismatch was measured to be
invisible.** `just verify-toolchain` (`cargo xtask verify-toolchain`) is the
**tenth** check in `just verify` and a step in CI's `self-checks` job since
2026-09-05: it fails when the version in `backends/Dockerfile`'s
`FROM rust:<version>-alpine…` and `rust-toolchain.toml`'s `channel` disagree.
It exists because the review pass ran that exact mutation on this branch —
`channel = "1.98.0"` with the `FROM` line left at `rust:1.95.0-alpine3.22` —
and `just verify` and `just fmt-check` both exited **0**, with no other `just
ci` recipe reading either file. The first symptom would have been a release
image built by a compiler no local run and no CI job had ever used. With the
gate, that same mutation fails `just verify` naming the file, the line and
both versions. See the "self-verification" section above for what it does and
does not cover.

**What was NOT verified by this bump:** nothing has compiled this workspace
on `aarch64`, no CI run of this change exists, and the 1.88 MSRV remains
metadata-derived and uncompiled, exactly as before. See
[plans/exp11-notes/opus.md](../plans/exp11-notes/opus.md) for every command and
its output, and
[plans/exp11-notes/opus-review.md](../plans/exp11-notes/opus-review.md) for the
sabotage review that re-ran all of it, its mutation table, and the two test
counts that settle whether the suite shrank (it did not: on the base this work
was written against, `046892a`, that base and this branch both listed **1220
tests in 42 binaries**, each measured under its own pin). Those two are a
matched pair on `046892a` and are left as measured. **Rebased onto `02ae5cc`
on 2026-09-05**, which brought [ADR-0016](../adr/0016-engineering-standards.md)'s
`verify-serde` and `verify-repositories` and their 40 tests, the branch lists
**1270 tests in 42 binaries, 0 ignored** — 1260 on `02ae5cc` plus this
branch's ten, all in `xtask` (184 → 194). `verify-toolchain` is the **tenth**
gate after that rebase, not the eighth it was written as.

### Migration manifest — applied migrations are immutable (2026-09-07, issue #76)

**Landed.** `backends/migrations/MANIFEST.sha256` records the SHA-256 of every
migration file's bytes, and `verify-migrations` is the **eleventh** gate in
`just verify` and a step in CI's `self-checks` job. It fails when a migration
file's hash has moved, when a `.sql` file beside the manifest has no line, and
when a line names a file that is gone. `just migrations-manifest` **refuses**
to rewrite an existing line or to drop a line whose file has vanished — it only
appends — so the gate cannot be silenced by regenerating.

**Why:** `sqlx::migrate!` stores a SHA-384 of each file's _whole bytes_,
comments included, in `_sqlx_migrations.checksum`. PR #39 (the `@vpay` ->
`@vaam-apps` npm rename) reflowed one comment inside
`0028_create-checkout-sessions.sql` after it had shipped; every job in CI stayed
green, and every database brought up between #37 and #39 stopped booting
(exit 78). Nothing in the workspace could have caught it: every test starts from
an empty database and applies the current files, so the mismatch is invisible
until a _pre-existing_ database meets a new binary.

**What is proved, and by what.** Twelve unit tests in `.xtask`
(`migration_manifest_tests`) pin each way the gate fails — an edited file, an
unlisted file, a deleted line, a line for a file that does not exist, a
duplicated line, a non-hex hash, and that manifest _order_ does not matter.
`the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`
(`backends/tests/integration/tests/postgres_smoke.rs`) is the one that matters
operationally: it migrates a fresh `postgres:16-alpine`, rewinds migration 28's
checksum to the original file's SHA-384, confirms `sqlx::migrate!` then refuses
with the message the runbook quotes, parses the `UPDATE` **out of
`docs/runbooks/migrations.md` itself**, runs it, and confirms the migrator runs
clean afterwards.

**What this gate does NOT stop, stated because the opposite would be the
comfortable thing to write:** a contributor who edits a migration _and_
hand-edits its line in `MANIFEST.sha256` passes. Nothing can stop that —
a manifest whose own hash is checked has to pin that hash somewhere, and
whoever can edit two files can edit three. The manifest makes the edit
**visible as a reviewable one-line diff**; it does not make it impossible. It
also says nothing about a database that is already broken; that is the
runbook's §4.

**Corrected during review (2026-09-07).** The first draft of
`docs/runbooks/migrations.md` gave the repair as the SHA-384 of 0028's
**original** bytes — which is precisely what a broken database already holds.
The `UPDATE` would have reported `UPDATE 1`, changed nothing, and left the
binary exiting 78, with the page telling an on-call operator it had worked.
The correct value is the **current** file's
(`6eeb31ee…07b5ec`); the original (`f4d1a8e1…8ae252`) is now stated beside it
so an operator can tell which state their database is in, and the integration
test above is what keeps both honest. The same draft's gate hashed _every_ file
in `backends/migrations/`, not just `*.sql`, which made
`backends/migrations/README.md` unaddable — the gate demanded a manifest line
for it and `just migrations-manifest` would never write one — and accepted
duplicate manifest lines silently.
