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
> Read [`docs/status.md`](docs/status.md) before forming any expectation of what
> works. That page is machine-checked: `cargo xtask verify-status` fails the
> build if the code contains an unimplemented path that the status page does not
> declare.

---

## What it is meant to be

A small payment gateway for Cameroon that merchants integrate the way they'd
integrate Stripe — same object model, same idempotency semantics, same webhook
signature scheme — while it talks underneath to mobile money rails that behave
nothing like cards.

**Authentication is the one place this comparison does not hold.** `/v1`
does not accept an `sk_live_`/`sk_test_`-style API key. It authenticates
merchants with OAuth2 `client_credentials` + `private_key_jwt` (RFC 7523):
each merchant is a statically registered client, holding its own private
key, configured directly in vpay's YAML — vpay stores only the public half.
No Stripe SDK can authenticate against vpay as a result. See
[ADR-0010](docs/adr/0010-merchant-auth-private-key-jwt.md) for why, and
[`examples/merchant-curl`](examples/merchant-curl/) for the resulting
two-step flow. vpay ships its own merchant SDKs that do that handshake —
[`sdks/rust`](sdks/rust/) (`vpay-sdk`) and [`sdks/nodejs`](sdks/nodejs/)
(`@vpay/sdk`) — implementing the wire contract in
[`docs/flows/merchant-auth.md`](docs/flows/merchant-auth.md). The Rust one
has completed a real handshake against a running `vpay-server` — that is
what [`examples/merchant-demo`](examples/merchant-demo/) and `just demo`
below exist to show — and then reached the honest `404`, because **no `/v1`
business resource exists yet**. The Node SDK is still tested only against
stubs of the contract.

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
must appear in `docs/status.md`, and `cargo xtask verify-status` fails the build
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
sdks/
  rust/         vpay-sdk   — merchant SDK (workspace crate; private_key_jwt handshake, /v1 resources, webhooks)
  nodejs/       @vpay/sdk  — the same, zero-dependency Node ≥ 22 ESM
examples/       merchant-demo (runnable: `just demo`) · merchant-curl · merchant-node · webhook-receiver
docs/           adr/ · rfc/ · flows/ · runbooks/ · api/ · status.md
schemas/        *.cstack   (syntax verified, design sketch, excluded from the build — see docs/status.md)
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
```

`just` with no argument lists every task.

### Try it locally

**Prerequisites:** Docker (with Compose v2.24+ — the demo overlay uses `!reset`),
the Rust toolchain `rust-toolchain.toml` pins, `just`, `jq`, `curl` and
`openssl`. `pnpm` is needed only if you want to work on the dashboard; `just
demo` builds it in a container.

```bash
just demo
```

That generates a throwaway RS256 key for the server's OAuth provider and a
second one for a demo merchant (`.e2e/`, git-ignored, both discarded with the
stack), registers the merchant's **public** JWK in a `demo` profile overlay,
brings up Postgres + both WireMock rail stubs + `vpay-server` + `vpay-worker` +
the dashboard, waits for `/healthz`, and then runs
[`examples/merchant-demo`](examples/merchant-demo/) — a small Rust binary built
on the real merchant SDK ([`sdks/rust`](sdks/rust/)).

What you will see, four steps, one line each:

1. the OP's discovery document and JWKS — its issuer and the `kid` it signs with;
2. an access token obtained with `client_credentials` + `private_key_jwt`, shown
   as its decoded `iss`/`aud`/`sub`/`exp` claims (never the token itself);
3. `GET /v1/payment_intents/pi_demo` **without** a token — a `401` carrying
   vpay's error envelope, so you can see the authentication boundary is real;
4. the same call **with** a token, through the SDK — a typed `404
   unknown_route`, and the sentence *"payment intents are not built yet — this
   is where the next step lands"*.

Step 4 is the point. Everything up to the bearer-token boundary works; past it
there is nothing, and the demo exits `0` for saying so. If it ever prints a
payment intent, something fabricated one — treat that as a defect, not a
feature.

Then:

```bash
just demo-down        # containers and volumes
```

> **Note on the runtime image.** The first `just demo` run on 2026-09-02
> found that `vpay-server` could not boot inside its own `FROM scratch`
> image: the JWKS validator's HTTP client loaded trust roots from the OS
> store, which that image does not have. Fixed the same day — the client is
> now built on vendored `webpki-roots` (`vpay_api::http_client`) and pinned
> by a subprocess test that boots the server with an empty trust store. See
> the "Resource-server JWT validation" row in [`docs/status.md`](docs/status.md).

### Testing

Three commands, with genuinely different requirements:

| Command | Needs | Runs |
|---|---|---|
| `just verify` | nothing but Rust; seconds | the three self-checks — no test double reachable from a shipping binary, every unimplemented path declared in `docs/status.md`, every error type classified |
| `just test` | **Docker** | `cargo nextest run --workspace` + `pnpm -r test`. The Postgres-backed suites use testcontainers and **fail loudly** without a reachable daemon — they never skip, so a green run is a real one |
| `just test-e2e` | Docker, and Cypress's binary | builds the images, boots `compose.yml` + `compose.e2e.yml`, runs the browser suite, tears the stack down. This is what CI's `e2e` job does |

`just ci` runs everything CI runs, in CI's order, and is what to run before
opening a PR.

### Running the binaries directly

Both binaries take a `clap`-based CLI where every option auto-resolves from an
environment variable, with an explicit flag beating its env var
(`backends/crates/vpay-config/src/cli.rs`). Run `--help` on either to see the
live flag set — that is more trustworthy than any doc if the two disagree:

```bash
cargo run -p vpay-server -- --help
cargo run -p vpay-worker-bin -- --help
```

`vpay-server` signs merchant tokens, so it needs an RS256 signing key before it
will start. Generate one once, offline:

```bash
cargo xtask gen-signing-key --out ./secrets   # writes ./secrets/oauth-signing-key.pem
```

The private key stays in that file — nothing prints it, logs it or stores it in
the database. In a real deployment it is a Kubernetes Secret and
`--oauth-signing-key-file` points at the mount.

```bash
# The rail credentials in config/application.yml are ${VAR} placeholders, and
# an unresolved one is a fatal, named startup error — not an empty string.
export MTN_SUBSCRIPTION_KEY=dev MTN_API_KEY=dev ORANGE_MERCHANT_KEY=dev

# flags win over env vars
cargo run -p vpay-server -- \
  --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5432/vpay \
  --oauth-signing-key-file ./secrets/oauth-signing-key.pem \
  --bind 127.0.0.1:8080 --log-format text

# or drive it by env, as compose.e2e.yml does
VPAY_CONFIG=config/application.yml \
DATABASE_URL=postgres://vpay:vpay@localhost:5432/vpay \
VPAY_OAUTH_SIGNING_KEY_FILE=./secrets/oauth-signing-key.pem \
VPAY_BIND=127.0.0.1:8080 VPAY_LOG_FORMAT=text cargo run -p vpay-server
```

The Postgres those URLs point at is the one `just up` starts.

Neither binary calls a payment rail. `vpay-server` connects to Postgres, runs
migrations, and serves `/healthz` plus the merchant OP —
`POST /v1/oauth/token` (`client_credentials` + `private_key_jwt`),
`GET /v1/oauth/.well-known/openid-configuration` and
`GET /v1/oauth/jwks.json`. Every other path under `/v1` is behind a merchant
bearer token, and past that boundary answers the honest 404: no `/v1` resource
route exists yet. `vpay-worker-bin` stays up answering shutdown signals but its
job loop is not implemented, and it says so in a startup banner and a repeating
heartbeat log line.

`--config`, `--database-url` and `--oauth-signing-key-file` are required and
genuinely consumed; a missing one exits `78` before the port is bound.
`--public-base-url` is accepted and read by nothing: `ServerArgs::public_base_url`
(`backends/crates/vpay-config/src/cli.rs`) is only ever touched by that module's
own tests — grep the workspace for `public_base_url` and every remaining hit is
`Config`'s `deployment.public_base_url`, which is a *different* value, read from
the YAML. That YAML field is what the OP's issuer is derived from
(`vpay_api::op::issuer_for` → `{public_base_url}/v1/oauth`), so the URL a
merchant's tokens carry comes from the config file, never from the flag. See
[`docs/status.md`](docs/status.md) and
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
- **Rootless Docker.** `testcontainers` talks to `/var/run/docker.sock` by
  default. If your `docker` CLI uses a rootless context, point the tests at
  it: `DOCKER_HOST=unix:///run/user/$(id -u)/docker.sock cargo nextest run
  --workspace`. The Postgres-backed suites need `postgres:16-alpine` pulled.
- **musl target.** `rustup target add x86_64-unknown-linux-musl` before
  `just build-dist`. `backends/Dockerfile` now builds the host's *implicit*
  musl target rather than hardcoding the x86_64 triple, but the Dockerfiles
  themselves have not been built in this repo's own development environment —
  see [`docs/status.md`](docs/status.md)'s Infrastructure section for why.

## Documentation

Start with [`docs/status.md`](docs/status.md), then:

- [Roadmap](docs/roadmap.md) — the phases from scaffold to a deployable
  gateway, and where the project stands in that sequence
- [Flows](docs/flows/) — one document per process, with invariants
- [ADRs](docs/adr/) — decisions and what they cost
- [RFCs](docs/rfc/) — proposals not yet decided
- [Runbooks](docs/runbooks/) — what to do when an alert fires

## Licence

Apache-2.0. See [LICENSE](LICENSE).
