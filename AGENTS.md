# AGENTS.md

Instructions for any agent or human contributing to vpay. Read this before
writing code.

---

## The two rules

These are not style preferences. Both are machine-enforced by `just verify`, and
CI runs it.

`just verify` is **seven** gates and one report. The gates
(`verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity`,
`verify-links`, `verify-npm-scope`, `check-schema`) fail the build. The
report (`verify-docs`) never does — it prints doc-comment volume per crate,
the production functions of 80 lines or more, every ```` ```ignore ````
doctest fence and every `#[allow]`/`#[expect]`, and nothing more. Read it; it is not a gate you can pass or fail.

This paragraph said "three gates" until 2026-09-05, and had been wrong since
`verify-sdk-parity` landed on 2026-09-03; `verify-links` made it wrong by two.
It then said "five" for the rest of that day, because `verify-npm-scope` and
`check-schema` both landed on 2026-09-05 on branches that did not see each
other — the count is corrected here, where the two met. Six of the seven are
`cargo xtask` commands; `check-schema` is a justfile recipe, because it shells
out to the CrateStack CLI, a binary this workspace does not build.
There is an eighth check, `cargo xtask verify-citations` (`just
docs-check-citations`), which is a gate but **not** part of `just verify` or
`just ci`: it needs the network and a GitHub token. Run it when you add or
edit a document that cites a CI run id, a pull request or an issue.

### 1. No test doubles in shipping processes

No mock, fake, stub or dummy may be reachable from `vpay-server` or
`vpay-worker-bin`.

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
- When you finish something, update `docs/status.md` in the same commit. A
  status page that lags is worse than none, because people trust it.

If you are unsure whether something counts as done: would a test fail if it
broke? If no, it is not done.

---

## Architecture rules

**Rails live behind the port.** `if provider == "mtn_momo"` outside
`backends/crates/vpay-adapter-*` is a defect. Branch on capability *values*
(`flow`, `supports_refunds`), never on a provider code. ([ADR-0002](docs/adr/0002-provider-port.md))

**No environment branching.** No `if (sandbox)`, no `NODE_ENV` check, no
profile-selected bean. A profile selects a *config file*, never a *code path*.
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
  composite that `#[from]`s the leaves and *delegates* its classification
  rather than re-deciding it; `anyhow` appears only at the boundary — in
  `backends/apps/*` `main()`, and in `[dev-dependencies]`. Status, retry
  policy, log severity and exit code are all derived from `Category`, never
  chosen at a call site. `cargo xtask verify-errors` fails the build if a `pub`
  `…Error`/`…Rejection` type in `backends/crates` has no `impl Classify`, or if
  a library crate lists `anyhow` under `[dependencies]`.
- TLS: rustls only. `openssl`, `openssl-sys` and `native-tls` are banned in
  `deny.toml`; do not add a dependency that needs them without a new ADR.
- Doc comments on every public item, explaining *why*, not restating the name.
  An example in one is compiled and run — `just test-doc` (`cargo test --doc
  --workspace`) is part of `just ci` and of CI's `rust` job, because
  `cargo nextest` runs no doctests, so until 2026-09-03 not one of them had
  ever been compiled by CI. Do not reach for ```` ```ignore ```` or ```` ```no_run ````
  to make an example compile: an example nothing runs is a claim nothing
  checks. Use ```` ```text ```` if it is not Rust, and otherwise make it real.
- The reasoning behind a piece of code goes in `docs/reference/<crate>.md`, not
  in an 80-line module header. One paragraph of what and why plus a link;
  `# Errors`, `# Panics` and `# Examples` stay in the source.

## TypeScript conventions

- TS strict, plus `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`.
- Components: `class-variance-authority` for variants, Headless UI for
  behaviour, daisyU/Tailwind for tokens, framer-motion for motion, vaul for
  sheets. Do not hand-roll a component that Headless UI already solves.
- Status colour and copy come from `@vpay/tokens`. Never inline a status colour
  in a component — a status must not be green in one view and grey in another.
- The dashboard never holds a merchant API key. It calls `/dash/v1` server-side
  under an OIDC session. ([ADR-0008](docs/adr/0008-dashboard-scope.md))

## Testing

| Layer | Tool | Where |
|---|---|---|
| Rust unit | `cargo nextest` | alongside the code |
| Rust doctests | `cargo test --doc` (`just test-doc`) | in `///` examples |
| Rust integration | testcontainers | `backends/tests/integration` |
| Adapter conformance | shared suite | `backends/tests/conformance` |
| TS unit | vitest | alongside the code |
| Browser e2e | Cypress against `compose.e2e.yml` | `frontends/tests/e2e` |

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

- A decision that has been made → an ADR (immutable; supersede, never edit).
- A process → a flow doc, as above.
- Why a piece of code is shaped the way it is → `docs/reference/<crate>.md`.
- A proposal under discussion → an RFC.
- Something an on-call person must do → a runbook.

## Commits and PRs

- Conventional commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`).
- A PR that changes behaviour updates `docs/status.md` and the relevant flow doc
  in the same PR.
- `just ci` must pass locally before review.

## Before you open a PR

```bash
just fmt
just ci
```

Then ask yourself the one question this repo cares about most: **does anything I
wrote imply something works that I have not actually seen work?** If so, fix the
claim, not just the code.
