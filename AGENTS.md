# AGENTS.md

Instructions for any agent or human contributing to vpay. Read this before
writing code.

---

## The two rules

These are not style preferences. Both are machine-enforced by `just verify`, and
CI runs it.

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
- Errors: `thiserror` for library crates, `anyhow` only in binaries.
- TLS: rustls only. `openssl`, `openssl-sys` and `native-tls` are banned in
  `deny.toml`; do not add a dependency that needs them without a new ADR.
- Doc comments on every public item, explaining *why*, not restating the name.

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

- A decision that has been made → an ADR (immutable; supersede, never edit).
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
