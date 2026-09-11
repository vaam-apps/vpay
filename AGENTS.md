# AGENTS.md

Instructions for any agent or human contributing to vpay. Read this before
writing code.

---

## The two rules

These are not style preferences. Both are machine-enforced by `just verify`, and
CI runs it.

`just verify` is the gates the `verify` recipe lists in the `justfile`, and
one report. **The recipe is the list; this paragraph is a description of it,
and it has gone stale at nearly every count it has carried** — see below. On
this commit the gates are twelve (`verify-no-mocks`, `verify-status`,
`verify-errors`, `verify-sdk-parity`, `verify-links`, `verify-npm-scope`,
`check-schema`, `verify-serde`, `verify-repositories`, `verify-toolchain`,
`verify-ui`, `verify-migrations`) and they fail the build. If that list and
the recipe disagree, the recipe is right: read it, and fix this paragraph in
the same commit. The report
(`verify-docs`) never does — it prints doc-comment volume per crate, in-file
comment volume per crate, the number of `#[doc = include_str!]` modules, the
production functions of 80 lines or more, every ` ```ignore ` doctest
fence and every `#[allow]`/`#[expect]`, and nothing more. Read it; it is not
a gate you can pass or fail.

This paragraph said "three gates" until 2026-09-05, and had been wrong since
`verify-sdk-parity` landed on 2026-09-03; `verify-links` made it wrong by two.
It then said "five" for the rest of that day, because `verify-npm-scope` and
`check-schema` both landed on 2026-09-05 on branches that did not see each
other — the count was corrected to seven where the two met, and `verify-serde`
and `verify-repositories` ([ADR-0016](docs/adr/0016-engineering-standards.md),
2026-09-05) made it nine. `verify-toolchain` makes it ten, later the same
day, out of the review of the 1.95.0 -> 1.98.0 toolchain bump: it fails when
`backends/Dockerfile`'s `FROM rust:` version and `rust-toolchain.toml`'s
`channel` disagree, a mismatch that was measured to pass every other gate.
`verify-migrations` makes it **twelve** on 2026-09-07 (after `verify-ui`, the eleventh, from the exp26 UI revamp the same day), out of issue #76: it
fails when a migration file's SHA-256 no longer matches
`backends/migrations/MANIFEST.sha256`, because `sqlx::migrate!` checksums a
migration's whole bytes and a comment reflowed after the file shipped stops
every database that applied the original from booting — which is what PR #39
did, with every job in CI green.
Ten of the twelve are `cargo xtask` commands; `check-schema` and `verify-ui` are
justfile recipes — the first shells out to the CrateStack CLI, a binary this
workspace does not build, and the second is a handful of `git grep`s.
There is one more check, `cargo xtask verify-citations` (`just
docs-check-citations`), which is a gate but **not** part of `just verify` or
`just ci`: it needs the network and a GitHub token. Run it when you add or
edit a document that cites a CI run id, a pull request or an issue.

### 1. No test doubles in shipping processes

No mock, fake, stub or dummy may be reachable from `vpay-server` — the one
shipping binary since 2026-09-07 (issue #77), in any of its modes (`serve`,
`worker`, `staff`). It was two, `vpay-server` and `vpay-worker-bin`.

- `vpay-testkit`, `wiremock`, `testcontainers`, `mockall`, `fake` may appear
  **only** under `[dev-dependencies]`.
- A stub rail is a **WireMock host in configuration** (`compose.yml`), reached
  over HTTP exactly as a real rail is. It is never a linked implementation or a
  conditionally-compiled variant.
- `cargo xtask verify-no-mocks` fails the build otherwise.

Why: a mock compiled into the server is a code path that exists only outside
production. It diverges silently, and "passes in CI, breaks in prod" becomes
structurally possible.

### 2. Never claim a feature is done when it is not

- Unwritten code returns `ProviderError::NotImplemented("<crate>::<fn>")`. It
  **never** returns a plausible-looking success, an empty list, or a zero.
- Every such token must appear in `docs/status.md`. `cargo xtask verify-status`
  fails the build otherwise — and it fails in both directions.
- Tests for unbuilt behaviour are `#[ignore = "not implemented: … — see
docs/status.md"]`, so a green run never overstates coverage.
- When you finish something, update the status pages in the same commit. A
  status page that lags is worse than none, because people trust it. **Since
  2026-09-11 that is more than one file:** `docs/status.md` is the current-state
  page and carries the declaration this gate reads; the row your change belongs
  to lives on a page under `docs/status/`, and your `just ci` evidence goes on a
  dated page under `docs/status/verification/`. `docs/status.md` § "Where a new
  row goes" says which is which, and
  [docs/status/README.md](docs/status/README.md) is the index.

If you are unsure whether something counts as done: would a test fail if it
broke? If no, it is not done.

---

## Standards

Six engineering standards, written down in
[ADR-0016](docs/adr/0016-engineering-standards.md) with the rationale for each,
what enforces it, and what only a reviewer can judge. Read the ADR once. These
are the parts you apply on **every** change:

1. **Errors** — a library crate's own failures are `thiserror` enums, each with
   one `impl vpay_core::error::Classify`; a layer that consumes several crates
   `#[from]`s them and delegates rather than re-deciding; `anyhow` only in
   `backends/apps/*` `main()` and `[dev-dependencies]`.
   ([ADR-0011](docs/adr/0011-error-modelling.md), `verify-errors`)
2. **Adapters** — a rail is reached only through `ProviderAdapter`, and its
   failures are _mapped_ into `FailureCode`/`ProviderError`, never flattened
   into a `String`. ([ADR-0002](docs/adr/0002-provider-port.md))
3. **serde** — every type deriving `Serialize`/`Deserialize` under
   `backends/crates/*/src` carries `#[serde(rename_all = "snake_case")]`, or
   renames every field/variant itself, or gets a row with a reason in
   ADR-0016's exemption table. Visibility is not part of the rule. `rename_all`
   is a statement about _our_ wire — a rail's casing is the rail's.
   (`verify-serde`; the table is checked in both directions, so a stale
   exemption fails too)
4. **SOLID and DRY** — no gate, and the ADR says so. Ask what would have to
   change for a piece of code to be wrong, and whether that lives in one place.
5. **Repositories** — `vpay-db` exposes traits; every implementation is
   `pub(crate)` and reached through `vpay_db::connect` or a function returning
   `impl Trait`. Name the trait, never `PgRepositories` or a `Sql…Store`.
   (`verify-repositories`)
6. **Docs** — an example in a doc comment is compiled (`just test-doc`); never
   reach for ` ```ignore ` to make one build. Long reasoning goes in
   `docs/reference/<crate>.md`, not in an 80-line module header; a module doc
   that _is_ a document uses `#[doc = include_str!("…md")]`. Prefer one more
   `///` explaining why over one more `//` restating what.

**The migration rule: existing code is migrated as it is touched; a new crate
complies from its first commit.** Do not open a sweep. Do not, in particular,
add an exemption row to make a gate pass on code you did not have to touch.

---

## Architecture rules

**Rails live behind the port.** `if provider == "mtn_momo"` outside
`backends/crates/vpay-adapter-*` is a defect. Branch on capability _values_
(`flow`, `supports_refunds`), never on a provider code. ([ADR-0002](docs/adr/0002-provider-port.md))

**No environment branching.** No `if (sandbox)`, no `NODE_ENV` check, no
profile-selected bean. A profile selects a _config file_, never a _code path_.
Sandbox and production are two deployments of the same image.
([ADR-0003](docs/adr/0003-yaml-configuration.md))

**Money is integer minor units.** XAF is zero-decimal: `5000` means 5,000 FCFA.
Floating-point arithmetic is denied workspace-wide. One conversion function,
`Money::to_provider_string`. ([docs/flows/money.md](docs/flows/money.md))

**Never let a payer act on a transaction you cannot name.** Push rails: persist
the reference before submitting. Redirect rails: persist the rail's token before
redirecting. ([docs/flows/crash-safety.md](docs/flows/crash-safety.md))

**One charge per intent, forever.** Enforced by a plain unique index. Retry
means a new PaymentIntent.

**Callbacks are hints.** `parse_callback` returns identifiers only, never a
status. The authenticated status query is the only thing that moves money.

---

## Rust conventions

- Edition 2024, resolver 3. MSRV in `Cargo.toml`; toolchain pinned in
  `rust-toolchain.toml`.
- `unwrap`, `expect`, `panic`, `todo`, `unimplemented`, float arithmetic: denied
  in production code. Tests are exempt via `clippy.toml`. `unsafe` is forbidden.
  ([ADR-0007](docs/adr/0007-lint-policy.md))
- Errors are typed at the leaves, composed per layer, classified once
  ([ADR-0011](docs/adr/0011-error-modelling.md),
  [docs/flows/errors.md](docs/flows/errors.md)). A library crate defines closed
  `thiserror` enums for its own concerns and each one implements
  `vpay_core::error::Classify`; a layer that consumes several crates defines a
  composite that `#[from]`s the leaves and _delegates_ its classification
  rather than re-deciding it; `anyhow` appears only at the boundary — in
  `backends/apps/*` `main()`, and in `[dev-dependencies]`. Status, retry
  policy, log severity and exit code are all derived from `Category`, never
  chosen at a call site. `cargo xtask verify-errors` fails the build if a `pub`
  `…Error`/`…Rejection` type in `backends/crates` has no `impl Classify`, or if
  a library crate lists `anyhow` under `[dependencies]`.
- TLS: rustls only. `openssl`, `openssl-sys` and `native-tls` are banned in
  `deny.toml`; do not add a dependency that needs them without a new ADR.
- Doc comments on every public item, explaining _why_, not restating the name.
  An example in one is compiled and run — `just test-doc` (`cargo test --doc
--workspace`) is part of `just ci` and of CI's `rust` job, because
  `cargo nextest` runs no doctests, so until 2026-09-03 not one of them had
  ever been compiled by CI. Do not reach for ` ```ignore ` or ` ```no_run `
  to make an example compile: an example nothing runs is a claim nothing
  checks. Use ` ```text ` if it is not Rust, and otherwise make it real.
- The reasoning behind a piece of code goes in `docs/reference/<crate>.md`, not
  in an 80-line module header. One paragraph of what and why plus a link;
  `# Errors`, `# Panics` and `# Examples` stay in the source.

## TypeScript conventions

- TS strict, plus `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`.
- Components: `class-variance-authority` for variants, `@base-ui/react` for
  behaviour, daisyUI 5 on Tailwind 4 for tokens (theme `bumblebee`). Do not
  hand-roll a component `@base-ui/react` already solves. **Corrected
  2026-09-07:** this line named Headless UI, framer-motion and vaul, none of
  which is a dependency of any `package.json` in this repository, and no
  motion or sheet library is. `just verify-ui` is the gate on the daisyUI
  half.
- Status colour and copy come from `@vpay/tokens`. Never inline a status colour
  in a component — a status must not be green in one view and grey in another.
- The dashboard never holds a merchant API key. It calls `/dash/v1` server-side
  under an OIDC session. ([ADR-0008](docs/adr/0008-dashboard-scope.md))

## Testing

| Layer               | Tool                                 | Where                        |
| ------------------- | ------------------------------------ | ---------------------------- |
| Rust unit           | `cargo nextest`                      | alongside the code           |
| Rust doctests       | `cargo test --doc` (`just test-doc`) | in `///` examples            |
| Rust integration    | testcontainers                       | `backends/tests/integration` |
| Adapter conformance | shared suite                         | `backends/tests/conformance` |
| TS unit             | vitest                               | alongside the code           |
| Browser e2e         | Cypress against `compose.e2e.yml`    | `frontends/tests/e2e`        |

**The conformance suite is one suite, parameterised over every adapter.** Adding
a rail means making it pass — not writing a new suite. If you find yourself
writing rail-specific conformance tests, the port leaked.

Do not stub inside the browser in Cypress. The rails are stubbed at the
infrastructure layer, so the app under test is the app that ships.

## Documentation

Every flow and process gets a document in `docs/flows/`, answering: what
happens, in what order, what can go wrong, and what invariant holds throughout.
Each ends with a **Status** section stating what is actually built.

Two things about a document are machine-checked, since 2026-09-05. Every
relative link in a tracked `*.md` must resolve to a **tracked** file or
directory (`cargo xtask verify-links`, in `just verify`), so a link satisfied
by an untracked scratch file fails rather than passing on your machine alone.
And every CI run id, pull request and issue a document cites as evidence must
exist (`cargo xtask verify-citations`, opt-in because it needs the network).
A citation that does not resolve is a false claim: strike it through with a
dated correction. Do not replace it with an id you have not checked.

**`verify-links` checks a destination path and never a `#anchor`.** A link to a
heading that has been renamed, or that moved to another page, still passes the
build. So re-read the anchors pointing into a document whose heading you rename.
A sweep on 2026-09-11 found **four** stale ones across the tree — three left
behind by heading renames on 2026-09-06 and 2026-09-07, and one naming a section
that does not exist. Three are fixed; the fourth is named in
[docs/README.md](docs/README.md) rather than quietly left.

**A document that has outgrown one sitting becomes an overview and a
directory**, not a shorter document. Eleven did on 2026-09-11: the page keeps
its own path and its own **Status** section, and indexes pages carrying what was
moved there **verbatim** — no dated measurement, struck-through claim or "this
said X until date Y and was wrong" may be dropped or paraphrased in the move,
because those are what make the page worth trusting.
[docs/README.md](docs/README.md) lists which pages, and what they were.

- A decision that has been made → an ADR (immutable; supersede, never edit).
- A process → a flow doc, as above.
- Why a piece of code is shaped the way it is → `docs/reference/<crate>.md`.
- A proposal under discussion → an RFC.
- Something an on-call person must do → a runbook.

## Commits and PRs

- Conventional commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`).
- A PR that changes behaviour updates the status pages and the relevant flow doc
  in the same PR. `docs/status.md` § "Where a new row goes" names the page for
  each kind of change; [docs/README.md](docs/README.md) is the index of the
  whole documentation tree.
- `just ci` must pass locally before review.

## Before you open a PR

```bash
just fmt
just ci
```

Then ask yourself the one question this repo cares about most: **does anything I
wrote imply something works that I have not actually seen work?** If so, fix the
claim, not just the code.
