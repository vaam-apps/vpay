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
schemas/        *.cstack   (UNVERIFIED — see docs/STATUS.md)
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

### Known environment gotchas

- **Cypress binary.** `pnpm install` needs `pnpm exec cypress install`
  afterwards on a machine that can reach Cypress's CDN. In restricted networks,
  `CYPRESS_INSTALL_BINARY=0` lets the rest of the install proceed.
- **musl target.** `rustup target add x86_64-unknown-linux-musl` before
  `just build-dist`.

## Documentation

Start with [`docs/STATUS.md`](docs/STATUS.md), then:

- [Flows](docs/flows/) — one document per process, with invariants
- [ADRs](docs/adr/) — decisions and what they cost
- [RFCs](docs/rfc/) — proposals not yet decided
- [Runbooks](docs/runbooks/) — what to do when an alert fires

## Licence

Apache-2.0. See [LICENSE](LICENSE).
