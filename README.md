<div align="center">

# vpay

**A provider-agnostic payment gateway for Central Africa, with a Stripe-shaped API.**

MTN MoMo and Orange Money are the first two adapters. Neither is the architecture.

</div>

---

> ## ⚠️ vpay cannot take a payment yet
>
> This repository is a **scaffold**. It compiles, lints clean and its tests pass,
> but **no HTTP call to any payment rail has ever been made by this code**.
>
> Read [`docs/STATUS.md`](docs/STATUS.md) before forming any expectation of what
> works. That page is machine-checked: `cargo xtask verify-status` fails the
> build if the code contains an unimplemented path that the status page does not
> declare.

---

## What it is meant to be

A small payment gateway for Cameroon that merchants integrate the way they'd
integrate Stripe — same object model, same idempotency semantics, same webhook
signature scheme — while it talks underneath to mobile money rails that behave
nothing like cards.

Two rails ship in the MVP, and they have genuinely different payer journeys:

| | **MTN MoMo** (`push`) | **Orange Money** (`redirect`) |
|---|---|---|
| Payer acts by | Entering a PIN on their handset | Being redirected to Orange's hosted page |
| Intent status after `confirm` | `processing` | `requires_action` |
| Can the payer act before we persist? | **Yes** | **No** |

That last row is why crash safety has two enforcement points rather than one.
See [`docs/flows/crash-safety.md`](docs/flows/crash-safety.md).

## Two rules this repo enforces on itself

Both are wired into `just verify` and CI, because a promise nothing checks is a
promise that decays.

**1. No test doubles in shipping processes.** No mock, fake or stub may be
reachable from `vpay-server` or `vpay-worker-bin`. A stub rail is a *WireMock
host in configuration* — the same mechanism production uses to reach a real
rail. `cargo xtask verify-no-mocks` enforces it.
([ADR-0006](docs/adr/0006-no-mocks-in-main-processes.md))

**2. Never claim a feature is done when it is not.** Unwritten code returns
`ProviderError::NotImplemented` — it never fabricates a success. Every such path
must appear in `docs/STATUS.md`, and `cargo xtask verify-status` fails the build
otherwise. Tests for unbuilt features are `#[ignore]`d with a reason, so a green
run never overstates coverage.

## Layout

```
backends/
  crates/       vpay-core, -config, -ledger, -provider, adapters, -api, -worker, -testkit
  apps/         vpay-server, vpay-worker-bin   (musl → scratch images)
  tests/        integration (testcontainers) · conformance (shared adapter suite)
frontends/
  packages/     @vpay/tokens · @vpay/ui (design system) · @vpay/api-client · @vpay/config
  apps/         dashboard (Next.js)
  tests/        e2e (Cypress)
examples/       merchant-curl · merchant-node · webhook-receiver
docs/           adr/ · rfc/ · flows/ · runbooks/ · api/ · STATUS.md
schemas/        *.cstack   (syntax verified, design sketch, excluded from the build — see docs/STATUS.md)
.xtask/         repo automation and the two self-checks
```

## Stack

**Backend** — Rust edition 2024, resolver 3, axum, sqlx, rustls only
(native-tls is banned in `deny.toml`), mimalloc, static musl binaries into
`FROM scratch`. Tests with `cargo nextest` and testcontainers.

**Frontend** — Next.js 15, React 19, TypeScript strict. Design system on
Tailwind + daisyUI + `class-variance-authority` + Headless UI, with
framer-motion and vaul for motion and sheets. Storybook with the a11y addon.
Vitest for units, Cypress for e2e.

## Getting started

```bash
just install          # toolchains + pnpm deps
just up               # Postgres + a WireMock host per rail
just test             # cargo nextest + vitest
just verify           # the two self-checks above
just ci               # everything CI runs, in CI's order
```

`just` with no argument lists every task.

### Running the binaries directly

Both binaries take a `clap`-based CLI where every option auto-resolves from an
environment variable, with an explicit flag beating its env var
(`backends/crates/vpay-config/src/cli.rs`). Run `--help` on either to see the
live flag set — that is more trustworthy than any doc if the two disagree:

```bash
cargo run -p vpay-server -- --help
cargo run -p vpay-worker-bin -- --help
```

```bash
# flags win over env vars
cargo run -p vpay-server -- --bind 127.0.0.1:8080 --log-format text

# or drive it by env, as compose.yml does
VPAY_BIND=127.0.0.1:8080 VPAY_LOG_FORMAT=text cargo run -p vpay-server
```

Neither binary calls a payment rail. `vpay-server` writes rows and serves only
`/healthz` today; `vpay-worker-bin` stays up answering shutdown signals but
its job loop is not implemented, and it says so in a startup banner and a
repeating heartbeat log line. `--database-url`, `--config` and
`--public-base-url` are accepted but not yet consumed by anything — see
[`docs/STATUS.md`](docs/STATUS.md) and
[`docs/flows/configuration.md`](docs/flows/configuration.md).

### Known environment gotchas

- **Cypress binary.** The e2e specs (`frontends/tests/e2e`, run via
  `pnpm --filter @vpay/e2e run e2e`) need `pnpm exec cypress install`
  afterwards on a machine that can reach Cypress's CDN — its binary is not
  fetched by a plain `pnpm install` and is not present in every environment.
  In restricted networks, `CYPRESS_INSTALL_BINARY=0` lets the rest of the
  install proceed without it. `pnpm -r test` no longer touches Cypress at all
  (`@vpay/e2e`'s own test script is `e2e`, not `test`), so the ordinary unit
  test sweep works regardless of whether the binary is installed.
- **musl target.** `rustup target add x86_64-unknown-linux-musl` before
  `just build-dist`. `backends/Dockerfile` now builds the host's *implicit*
  musl target rather than hardcoding the x86_64 triple, but the Dockerfiles
  themselves have not been built in this repo's own development environment —
  see [`docs/STATUS.md`](docs/STATUS.md)'s Infrastructure section for why.

## Documentation

Start with [`docs/STATUS.md`](docs/STATUS.md), then:

- [Flows](docs/flows/) — one document per process, with invariants
- [ADRs](docs/adr/) — decisions and what they cost
- [RFCs](docs/rfc/) — proposals not yet decided
- [Runbooks](docs/runbooks/) — what to do when an alert fires

## Licence

Apache-2.0. See [LICENSE](LICENSE).
