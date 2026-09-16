---
name: cratestack-contributing
description: Working inside the cratestack framework repository itself — the just recipes that gate a PR, the 200-line file ceiling, layering enforcement, the changelog Unreleased rule, transport parity, the unsafe_code forbid opt-in, MSRV and cargo-deny policy, the release pipeline, and the AI-governance requirements for issues and pull requests. Load when editing the framework's own crates, adding a CI check, cutting a release, or opening an issue or PR against cratestack/cratestack.
---

# Contributing to the framework

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

This is for working **on** CrateStack, not with it.

## The pre-PR gate

```bash
just all-checks
```

Runs, in order: `just _fmt`, `cargo fix`, `clippy --fix -D warnings`,
`cargo check --all-targets`, **`just lint`**, and `cargo deny check` — all scoped
`--workspace --exclude embedded_flutter_native`.

**That trailing `just lint` is not redundant with the `--fix` clippy line, and
that is the whole point.** `clippy --fix` demotes a `-D warnings` error to a
warning whenever the crate still compiles, so a lint with no machine-applicable
rewrite passes `--fix` silently *and* leaves no working-tree diff. Measured: with
such a violation present, `just lint` exited 101 while `all-checks` exited 0 on
the same tree, and the PR turned `main` red. Consequence by design:
`all-checks` now fails on lint drift that `--fix` used to absorb.

`just lint` itself runs clippy twice — once over the workspace, and once for
`-p cratestack-client-rust --features middleware`, because `--workspace` resolves
*default* features and would never lint the middleware-gated code.

Never add `--all-features`. See `cratestack-troubleshooting` for why, and note
that the *reason* given in several places in the repo is stale.

## Tests

```bash
cargo test --workspace --exclude embedded_flutter_native   # plain, no DB
just test-pg                                               # compose Postgres on :55432
just test-pg-only -- <test_name>                            # faster inner loop
just test-pg-tc                                            # ephemeral testcontainers (what CI uses)
```

`just test-pg` hardcodes `--workspace`, which conflicts with `-p` — use
`test-pg-only` to scope.

CI runs the suite as **parallel shards** (`just test-ci-db`, `test-ci-redis`,
`test-ci-studio`, `test-ci-host`, and friends) because no single runner can
compile the whole workspace — the Tauri, wasm and napi example crates exhaust
runner disk.

Four of those shards exist because a 2026-08 coverage audit found their tests had
**never run in CI at all**: outbox, migrate-introspect, cli-baseline and
studio-db. One of them is how the duplicate-column bug a PR "fixed" shipped in
the first place — its decisive test never ran.

`just test-ci-ignored-report` is report-only and always exits 0. It exists
because nothing else ever ran an `#[ignore]`d test, which is how four
policy-enforcement tests — two catching real authorization bugs — went unrun for
months.

Read `cratestack-troubleshooting` before trusting a green run.

## Conventions that cost a review round-trip

**The ~200-line file ceiling.** Split by concern rather than growing a file —
that is why `cratestack-macros/` and `cratestack-axum/` are nested so deeply.
Enforced by `just verify-file-length`, scoped to `crates/*/src/**/*.rs`. Tests,
examples and non-Rust files are deliberately out of scope (one integration test
is 4000 lines).

`.ci/file-length-allowlist.toml` grandfathers the backlog. Every entry names
**exactly one file** — there is no glob form, deliberately — and requires `path`,
`lines`, `issue`, `added` and `reason`. **A stale entry is a hard failure**: the
allowlist can only shrink, so it cannot become a second, silent way to disable
the check. The same rule applies to the layering allowlist, which is currently
empty.

**Transport parity.** REST and RPC ship together, never REST first. See
`cratestack-rpc` for the full checklist and the reason it exists.

**Docs and skills parity.** A PR that adds a `### ` entry under `## Unreleased`
must fill in section 9 of the PR template with a link for `cratestack-docs` and
for `cratestack-skills`, or `n/a — <reason>`; a bare `n/a` is rejected. Gate:
`just verify-parity-declaration`. **It reads the PR body and nothing else** — it
cannot see the other repositories, so green means "declared", not "in sync".

**Filenames.** Rust sources are `snake_case` (rustfmt convention); everything
else is `kebab-case`. Public types are `PascalCase`. Edition 2024.

**The changelog.** Entries go under `## Unreleased` at the top. **Do not create
that heading yourself** — a release bump promotes it into a dated section and
re-seeds a fresh empty one, so it is always there. Never file under the newest
dated section; `just verify-changelog` is diff-based and only examines the `###`
lines *your* PR adds, with a carve-out for a release bump's own promotion.

The declared changelog set lives in `.ci/changelog-files.sh` and is currently
four files, not two — the root plus three `dart-packages/*`.

**`unsafe_code = "forbid"`** is declared once in the root `Cargo.toml` and
**Cargo silently ignores `[workspace.lints]` for any member that does not opt in**
with `[lints] workspace = true`. That is exactly the drift `just
verify-lints-optin` guards. Five FFI-boundary crates declare
`[lints.rust] unsafe_code = "allow"` manually instead, because Cargo rejects
combining `workspace = true` with a per-package override in the same manifest.
Standalone example workspaces excluded from the root members list must declare
the `forbid` locally — the root table cannot reach a disjoint workspace.

**Layering.** A crate at layer N may depend on crates at layers ≤ N.
`docs/adr/layers.toml` must be updated **in the same PR** that adds a crate under
`crates/` — **an unassigned crate is a hard failure, not a skip**. Enforced by
`just verify-layering` over `cargo metadata --no-deps`; `dev` and `build`
dependencies are exempt, target-gated normal dependencies are not. Do not edit
that file to legalise a new edge.

## MSRV, toolchain, deny

`rust-toolchain.toml` pins **1.98.0** with `rustfmt` and `clippy`, and declares
`targets = ["wasm32-unknown-unknown"]` so `trunk build` works under the pin. The
pin is deliberate: a floating `stable` surfaces new rustfmt formatting and new
clippy lints over time and spontaneously turns CI red.

`rust-version = "1.98.0"` in `[workspace.package]`. CI asserts **three-way**
agreement between the resolved `rustc --version`, the toolchain file's channel,
and the manifest's `rust-version`, with a distinct error per mismatch.

`deny.toml`: `unknown-registry = "deny"` and **`unknown-git = "deny"`** — which
is why a sibling crate must be depended on by published version, never by git.
`multiple-versions` is a warning. MPL-2.0 is deliberately **excluded from the
global allow list** so a new one still fails the gate and gets a human look; five
crates carry by-name exceptions, each with the dependency chain written out. Every
`[advisories.ignore]` entry carries a prose reason, and `advisory-not-detected` is
a **hard failure** so a stale ignore cannot outlive the problem it documented.

## Releasing

```bash
just bump 0.x.y          # rewrites every version literal, refreshes every lock
just release-check       # bundle studio UI, check, test (SKIP_TESTS=1 overrides)
just release 0.x.y       # bump -> validate -> publish in topo order -> tag (PUSH=1 to push)
```

**Publish order is topo-sorted from `cargo metadata` at recipe time, never
hand-maintained.** The subtle part is the edge predicate: what constrains order
is not dependency *kind* but whether the edge survives into the published
manifest. `cargo publish` strips a path-only dependency, so every edge carrying a
real version requirement must resolve from the registry — **dev-dependencies
included**. Both mistakes have shipped: counting every edge read a legitimate
path-only pair as a cycle and aborted; excluding every dev edge dropped a real
constraint and failed *after three crates had already uploaded*.

`just bump`'s pre-flight tool check, its globbed pathspecs, and its
`cargo metadata` (not `cargo check`) lockfile refresh each exist because of a
specific shipped failure. The comments name them. Read before changing.

Dry mode has a structural limit: a crate with any internal `cratestack-*`
dependency **can never** be packaged for a version never published anywhere,
because cargo resolves path+version workspace deps against the real crates.io
index even to build the manifest. The recipe skips those with an explanation
rather than attempting a guaranteed failure.

Releases can be **rehearsed** — the rehearsal is a flag on the real pipeline,
never a second copy, because a diverging copy stops representing reality within
one release.

## Issues and pull requests

**Blank issues are disabled.** Seven forms, in two groups: four short reporting
forms for users (bug, idea, question, docs) and three governance planning forms
(Epic, User Story, Dev Ticket) that maintainers fill in on a reporter's behalf.

For a bug, **the smallest reproducing `.cstack` schema is worth more than
anything else you can add**.

The PR template has nine sections. Three things are enforced by CI:

1. an **AI Usage Declaration**,
2. a **source-of-truth reference** (a URL or `#123`),
3. **verification evidence** (commands, logs, links, or checked boxes).

The governance workflow posts a sticky comment listing what is missing rather
than only failing, and is pinned to an immutable upstream release tag rather than
`@main`.

Work is **Ready** only when intent is clear, the source of truth is linked, and
any AI-generated content has been reviewed by a human. It is **Done** only when
acceptance criteria are met, tests pass, evidence is attached, and **a named
human owner accepts responsibility**.

> AI may accelerate the work, but humans own intent, verification, and
> consequences. AI output is not truth: review AI-generated code as untrusted,
> and never submit work you cannot explain.

## One cultural note that will save you time

This codebase documents its own mistakes, in comments, with the evidence and the
incident. A justfile recipe explains which release a glob replaced a hand-listed
path after; a `.ci` script retracts an earlier claim in the same file and tells
you not to "fix" it back; a workflow comment names the version whose tag cascade
silently published nothing.

**Read the comment block before changing the code it sits on**, and when
troubleshooting, grepping the justfile and workflows for a version number is
often faster than the changelog.

One standing correction worth knowing: **direct `sqlx` use in this repo is
correct and must not be "fixed".** Downstream consumers operate under a policy
banning hand-written SQL and direct `sqlx` dependencies — and that policy
explicitly exempts this repository, which is the layer that wraps sqlx.
