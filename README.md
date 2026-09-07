<div align="center">

# vpay

**A provider-agnostic payment gateway for Central Africa, with a Stripe-shaped API.**

MTN MoMo and Orange Money are the first two adapters. Neither is the architecture.

</div>

---

> ## ⚠️ vpay has never taken a real payment
>
> **What works, against stub rails.** A payment goes end to end: a merchant
> authenticates, creates a PaymentIntent, confirms it, `vpay-worker` polls the
> charge, settlement commits, and a signed webhook is delivered. `just demo`
> walks six of those on both rails to every outcome each rail documents, and a
> real browser has driven the hosted and the embedded checkout page
> (`just test-e2e`). Every rail in every one of those runs is a
> `wiremock/wiremock` container reached over HTTP.
>
> **What has never happened.** No HTTP call to MTN's or Orange's own
> endpoints — not production, not even their sandboxes. No payer has been
> prompted on a handset and no money has moved. No cluster has ever run vpay.
> And there is no dashboard a person can use: `/dash/v1`'s two read routes and
> staff sign-in answer over HTTP, but the app has no pages.
>
> Read [`docs/status.md`](docs/status.md) before forming any expectation of
> what works. It is machine-checked in both directions: `cargo xtask
> verify-status` fails the build if the code carries an unimplemented path that
> page does not declare, **and** if that page declares one no shipping code
> carries any more.

---

## What it is meant to be

A small payment gateway for Cameroon that merchants integrate the way they'd
integrate Stripe — same object model, same idempotency semantics, same webhook
signature scheme — while it talks underneath to mobile money rails that behave
nothing like cards.

**Authentication is the one place this comparison does not hold.** `/v1` does
not accept an `sk_live_`/`sk_test_`-style API key. It authenticates merchants
with OAuth2 `client_credentials` + `private_key_jwt` (RFC 7523): each merchant
is a statically registered client, holding its own private key, configured
directly in vpay's YAML — vpay stores only the public half
([ADR-0010](docs/adr/0010-merchant-auth-private-key-jwt.md),
[`docs/flows/merchant-auth.md`](docs/flows/merchant-auth.md)).

A Stripe SDK cannot do that handshake by itself, but it does not have to:
`stripe-node` takes an arbitrary `config.authenticator`, and
[`@vaam-apps/vpay-sdk/stripe`](sdks/nodejs/) supplies one, so `new Stripe("", {
authenticator, host, port, protocol })` reaches vpay with an empty key. That is
proven by [`sdks/stripe-compat`](sdks/stripe-compat/), which drives the real
`stripe` package against a live compose stack in CI's `e2e (compose)` job — as
far as a confirmed intent polling through to `succeeded` and a delivered
webhook verifying with `stripe.webhooks.constructEvent`. See
[`docs/flows/stripe-sdk-compat.md`](docs/flows/stripe-sdk-compat.md) for every
divergence, and [`examples/merchant-curl`](examples/merchant-curl/) for the
underlying two-step flow.

vpay also ships its own merchant SDKs — [`sdks/rust`](sdks/rust/) (`vpay-sdk`)
and [`sdks/nodejs`](sdks/nodejs/) (`@vaam-apps/vpay-sdk`) — plus a browser
client, [`sdks/stripe-js`](sdks/stripe-js/) (`@vaam-apps/vpay-stripe-js`), for
the payer-facing surface. The Rust SDK is what
[`examples/merchant-demo`](examples/merchant-demo/) and `just demo` drive
against a running `vpay-server`. **No test inside `sdks/nodejs` itself has ever
spoken to a vpay** — every server in that package's own tests is a `node:http`
stub; what has driven a live stack from Node is `sdks/stripe-compat` and
[`examples/shop`](examples/shop/). The two merchant SDKs are held to the same
capability matrix, machine-checked in both directions on every `just verify` —
see [`docs/sdks/parity.md`](docs/sdks/parity.md)
([ADR-0015](docs/adr/0015-sdk-parity.md)) for where they agree and the dated,
owned list of where they still don't.

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
reachable from `vpay-server` (either mode). A stub rail is a *WireMock
host in configuration* — the same mechanism production uses to reach a real
rail. `cargo xtask verify-no-mocks` walks `cargo metadata`'s dependency graph
from each shipping binary and fails the build otherwise.
([ADR-0006](docs/adr/0006-no-mocks-in-main-processes.md))

**2. Never claim a feature is done when it is not.** Unwritten code returns
`ProviderError::NotImplemented` — it never fabricates a success. Every such
path must appear in `docs/status.md`, and `cargo xtask verify-status` fails the
build otherwise, in both directions. Tests for unbuilt features are
`#[ignore]`d with a reason; `just verify-ignored` pins the workspace at **0**
of them, so a green run never overstates coverage.

## What is served on `/v1` today

`GET /healthz`, the merchant OP (`POST /v1/oauth/token`,
`GET /v1/oauth/.well-known/openid-configuration`, `GET /v1/oauth/jwks.json`),
and behind a merchant bearer token and a scope check:

| Resource | Methods |
|---|---|
| `/v1/payment_intents` | `POST`, `GET`, `GET {id}`, `POST {id}/confirm`, `POST {id}/cancel` |
| `/v1/checkout/sessions` | `POST`, `GET`, `GET {id}`, `POST {id}/expire` |
| `/v1/customers` | `POST`, `GET`, `GET {id}`, `POST {id}`, `DELETE {id}` |
| `/v1/events` | `GET`, `GET {id}` |
| `/v1/refunds/{id}` | `GET` |
| `/v1/account_holders` | `GET` |

An `Idempotency-Key` is required on every `POST`. The table is the constant
`vpay_api::V1_ROUTES`, and a boundary test walks it — it does not list paths of
its own — asserting every entry answers `401` without a token
(`every_registered_v1_path_answers_401_without_a_token`,
`backends/tests/integration/tests/payment_intents.rs`).

**`POST /v1/refunds` and `GET /v1/balance` are routed nowhere** and answer the
honest 404 from the nest's fallback. Creating a refund will keep doing so until
a rail can refund: `mtn_momo::refund` is `NotImplemented` (MTN refunds are the
Disbursements product, and no deployment holds those credentials) and
`orange_money` declares `supports_refunds: false`, which is a permanent
capability answer rather than unbuilt work. So `GET /v1/refunds/{id}` is a read
with no writer: **nothing in this repository creates a `refunds` row**, and
every row its tests read was inserted by the suite itself.

Two other surfaces exist: `/v1/browser`, which a payer's own page calls with a
publishable key and an intent's `client_secret` instead of a bearer token
([`docs/flows/browser-checkout.md`](docs/flows/browser-checkout.md)), and
`POST /provider/{code}/callback`, the one route a rail calls — proven against
WireMock, never called by MTN or Orange.

## Layout

```
backends/
  crates/       vpay-core, -config, -db, -ledger, -provider, adapters, -api, -worker, -testkit
  apps/         vpay-server                    (one musl → scratch image; `worker` is a subcommand)
  tests/        integration (testcontainers) · conformance (shared adapter suite) · webhook-receiver
frontends/
  packages/     @vpay/tokens · @vpay/ui (design system) · @vpay/api-client · @vpay/config
  apps/         checkout (the payment page vpay serves) · dashboard (a scaffold, see below)
  tests/        e2e (Cypress)
sdks/
  rust/         vpay-sdk                   — merchant SDK (workspace crate)
  nodejs/       @vaam-apps/vpay-sdk        — the same, zero-dependency Node ≥ 22 ESM
  stripe-js/    @vaam-apps/vpay-stripe-js  — the browser client for a payer's page
  stripe-compat/                           — the official `stripe` package, driven against a real stack
examples/       merchant-demo (`just demo`) · shop · checkout-browser · merchant-curl
                merchant-node · merchant-stripe-node · webhook-receiver
docs/           adr/ · rfc/ · flows/ · reference/ · runbooks/ · sdks/ · api/ · plans/ · status.md
schemas/        vpay.cstack   (compiled by vpay-db and gated by `just check-schema` — see docs/status.md)
deploy/         helm/vpay   (rendered and schema-validated; never applied to a cluster)
.xtask/         repo automation and the verify gates
```

`schemas/vpay.cstack` is no longer outside the build: `vpay-db` compiles it
(`include_server_schema!`) and `just check-schema` runs `cratestack check`
against the pinned CLI inside `just verify`. **Nine of the file's thirteen
models carry statements `vpay-server` and `vpay-worker` actually run** —
`currencies`, `providers`, `disabled_clients`, `customers`, `events`,
`webhook_deliveries`, `staff_members`, `staff_sessions` and
`oauth_authorization_codes`; the four that do not are `payment_intents`,
`charges`, `ledger_transactions` and `ledger_entries`, which stay a design
sketch a compiler now type-checks. `backends/migrations` remains the
authoritative schema, and this file has diverged from it on two `CHECK`
constraints CrateStack's grammar cannot express. See `docs/status.md`
§ CrateStack.

## Stack

**Backend** — Rust edition 2024, resolver 3, axum, sqlx, rustls only
(native-tls is banned in `deny.toml`), mimalloc, static musl binaries into
`FROM scratch`. Tests with `cargo nextest` and testcontainers.

**Frontend** — Next.js 15, React 19, TypeScript strict. Design system on
Tailwind 4 + daisyUI 5 (`bumblebee`) + `class-variance-authority` +
`@base-ui/react`. Storybook 10 with the a11y addon. Vitest for units, Cypress
for e2e.

## Getting started

```bash
just install          # toolchains + pnpm deps
just up               # Postgres + a WireMock host per rail
```

`just` with no argument lists every task.

### Try it locally

**Prerequisites:** Docker (with Compose v2.24+ — the demo overlay uses
`!reset`), the Rust toolchain `rust-toolchain.toml` pins, `just`, `jq`, `curl`
and `openssl`. `pnpm` is needed only to work on the web packages; `just demo`
builds every image it needs in Docker. The Node baseline is `.nvmrc` —
`22.23.2` — and `.npmrc` sets `engine-strict=true`, so `pnpm install` **fails**
rather than warns on an older Node.

```bash
just demo
```

`just demo` is `just demo-up` then `just demo-walk`, and both exist separately
so the walkthrough is re-runnable against a stack that is already up. `just
demo-status` says what is running and under which project; `just demo-down`
removes the containers and their volumes.

It generates a throwaway RS256 key for the server's OAuth provider and a second
one for a demo merchant (`.e2e/`, git-ignored, both discarded with the stack),
registers the merchant's **public** JWK in a `demo` profile overlay, and brings
up **eight** services with `up --wait` rather than a sleep: Postgres, both
WireMock rail stubs, the WireMock webhook receiver, `vpay-server`,
`vpay-worker`, `vpay-checkout` (the payment page) and `vpay-shop` (the demo
merchant's storefront). It then runs
[`examples/merchant-demo`](examples/merchant-demo/), a Rust binary built on the
real merchant SDK.

Six steps, the fourth of which is a table:

1. the OP's discovery document and JWKS — its issuer and the `kid` it signs with;
2. an access token obtained with `client_credentials` + `private_key_jwt`, shown
   as its decoded `iss`/`aud`/`sub`/`exp` claims (never the token itself);
3. a `/v1` call **without** a token — a `401` carrying vpay's error envelope,
   so you can see the authentication boundary is real;
4. **six payments, on both rails, to every outcome each rail documents.** Each
   one creates a PaymentIntent through the SDK, reads it back, confirms it,
   waits for `vpay-worker` to settle it, and then reads the webhook that
   settlement produced out of the receiver's own request journal and verifies
   its `Vpay-Signature` with the SDK:

   | # | Rail | Outcome | `last_payment_error.code` | Event delivered |
   |---|---|---|---|---|
   | 1 | `mtn_momo` | the payer approves → `succeeded` | — | `payment_intent.succeeded` |
   | 2 | `mtn_momo` | no balance → `requires_payment_method` | `insufficient_funds` | `payment_intent.payment_failed` |
   | 3 | `mtn_momo` | the prompt expires → `requires_payment_method` | `payer_timeout` | `payment_intent.payment_failed` |
   | 4 | `orange_money` | `requires_action` + the redirect URL → `succeeded` | — | `payment_intent.succeeded` |
   | 5 | `orange_money` | the hosted page expires → `requires_payment_method` | `payer_timeout` | `payment_intent.payment_failed` |
   | 6 | `orange_money` | the rail refuses → `requires_payment_method` | `provider_error` | `payment_intent.payment_failed` |

5. one hosted and one embedded Checkout Session, on a fresh intent each, read
   back and printed as a merchant would use them. It stops there — the program
   has no browser, and both sessions are still `open` when it exits;
6. `GET /v1/account_holders` — the three-way answer a name lookup has.

**Every outcome is chosen at the rail stub, never in the demo.** MTN's is
selected by the payer's MSISDN and Orange's by the amount, because those are
the only fields of each rail's protocol a merchant actually controls. The
three MSISDNs the walkthrough pays from — `237600000ce0`, `237600000f01`,
`237600000f02` — are **not phone numbers**: the last three characters are a
hex steering code the stub keys its scenario on, and step 6 shows
`GET /v1/account_holders` refusing one of them with a `400` because it is not
a Cameroon E.164 number. Nothing rewrites stored state to
make an outcome happen. The stubs are WireMock containers reached over HTTP
exactly as a real rail would be — that is the rule in [AGENTS.md](AGENTS.md): a
stub rail is a *host*, never a linked implementation — and **MTN's and Orange's
real endpoints have never been called by this code.** A `succeeded` here means
`vpay-worker` asked a stub and the stub said `SUCCESSFUL`; it does not mean
anyone paid.

**Every payment above is XAF, on both rails**, and that is a property of the
*demo overlay* alone — `.e2e/application-demo.yml`, the file `just
gen-demo-keys` writes. The demo shop prices its catalogue in XAF, offers a
payer both rails, and `/v1` refuses a confirm whose intent currency is not the
rail's settlement currency; one currency for both rails is what makes the
shop's MTN button payable. **Do not read that as "MTN accepts XAF".** It does
not: **MTN's real sandbox rejects XAF**, which is why
`config/application.yml` still puts `mtn_momo` on `currency: EUR` and why
`application-sandbox.yml` inherits it. Configuration either way — never a code
branch.

**The dashboard is the one service of the demo file set that stays down**, and
that is a statement rather than an optimisation: it renders a scaffold notice
and a status-badge reference, makes no call to `vpay-server` and has no login,
so there is no screen that could show the six payments the walkthrough just
made. `/dash/v1`'s two read routes and staff sign-in do exist and are proven
over HTTP against a real Postgres
(`backends/tests/integration/tests/dashboard_read_surface.rs`,
`…/staff_sign_in.rs`) — nothing in the app calls them yet. `docker compose -f
compose.yml -f compose.e2e.yml up` still starts the scaffold if you want to
look at it.

**Running two demos on one machine** is what the `just` variables are for:
`demo_project` picks the Compose project (so different containers, network and
`pgdata` volume) and `demo_port`, `demo_receiver_port`, `demo_orange_port`,
`demo_checkout_port` and `demo_shop_port` are the published *host* ports; the
server still binds 8080 inside its container.

```bash
just demo_port=18080 demo_receiver_port=18083 demo
just demo_project=vpay-demo demo-down          # teardown needs no port
```

**[`docs/runbooks/demo.md`](docs/runbooks/demo.md) is the full procedure** — the
exact commands, the real output of a real run, what that run proves and what it
does not, and the hazards it does not close.
[`docs/runbooks/checkout.md`](docs/runbooks/checkout.md) is where to start if
you want to buy something from the demo shop in a browser.

### Testing

Three commands, with genuinely different requirements:

| Command | Needs | Runs |
|---|---|---|
| `just verify` | Rust, and the pinned `cratestack` CLI on `PATH`; seconds | the gates the `verify` recipe lists in the [justfile](justfile) — eleven of them on this commit — and one advisory report, `verify-docs`, which never fails. The recipe echoes its own count on success, so the justfile is the number and this sentence is not. See [AGENTS.md](AGENTS.md) for what each gate refuses. `check-schema` **fails** rather than skips when the CLI is missing, because a skipped check checked nothing |
| `just test` | **Docker**, and Node | `cargo nextest run --workspace`, `cargo test --doc --workspace` and `pnpm -r test`. The Postgres-backed suites use testcontainers and **fail loudly** without a reachable daemon — they never skip, so a green run is a real one. The adapter conformance suite needs Docker too: it starts a real `wiremock/wiremock` container per rail rather than an in-process HTTP double, because a stub rail is a host reached over HTTP (ADR-0006) |
| `just test-e2e` | Docker, and Cypress's binary | builds the images, boots `compose.yml` + `compose.e2e.yml` + `compose.demo.yml`, runs the browser suite, tears the stack down. Four specs, 11 tests. This is what CI's `e2e` job does |

`just verify-ignored` is the count that keeps the suite honest. Measured on
this tree: **0 ignored, 46 test binaries, 1550 tests listed**; the recipe fails
if any of the three moves without the recipe and `docs/status.md` moving with
it.

`just ci` is what to run before opening a PR: CI's self-checks, `rust`, `web`
and supply-chain steps, in CI's order. The two jobs it does not cover are CI's
`e2e (compose)` (`just test-e2e`) and `deploy (helm chart)` (`just
helm-check`).

### Running the binaries directly

Both binaries take a `clap`-based CLI where every option auto-resolves from an
environment variable, with an explicit flag beating its env var
(`backends/crates/vpay-config/src/cli.rs`). Run `--help` on either to see the
live flag set — that is more trustworthy than any doc if the two disagree:

```bash
cargo run -p vpay-server -- --help
cargo run -p vpay-server -- worker --help
```

One binary since 2026-09-07 (issue #77): with no subcommand it serves the API,
`worker` runs the job loop, and `staff add` creates a dashboard account. It was
two packages and two images (`vpay-server`, `vpay-worker-bin`) before that.

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
export MTN_SUBSCRIPTION_KEY=dev MTN_API_KEY=dev \
       MTN_API_USER=11111111-2222-3333-4444-555555555555 \
       ORANGE_MERCHANT_KEY=dev ORANGE_CLIENT_ID=dev ORANGE_CLIENT_SECRET=dev

# flags win over env vars
cargo run -p vpay-server -- \
  --config config/application.yml \
  --database-url postgres://vpay:vpay@localhost:5432/vpay \
  --oauth-signing-key-file ./secrets/oauth-signing-key.pem \
  --bind 127.0.0.1:8080 --log-format text
```

Every one of those flags has an env var — `VPAY_CONFIG`, `DATABASE_URL`,
`VPAY_OAUTH_SIGNING_KEY_FILE`, `VPAY_BIND`, `VPAY_LOG_FORMAT` — which is how
`compose.e2e.yml` drives the same binary; a test fails if one is renamed or
dropped. The Postgres those URLs point at is the one `just up` starts.

**Both binaries call a payment rail.** `vpay-server` calls one when a merchant
confirms an intent; `vpay-worker-bin` runs the job loop
(`vpay_worker::run_loop`) that claims the `poll_charge` job the confirm
committed, asks the rail for the charge's status on a poll ladder, and commits
the charge, the intent and one event in a single transaction. It reaps leases
stranded by a crash at boot and on its own timer, and prints one `job loop
gauge` line a minute. Whether the rail either of them reaches is MTN, Orange or
a WireMock stub is a line in `config/application.yml`, and to date it has only
ever been a stub.

`--config`, `--database-url` and `--oauth-signing-key-file` are required and
genuinely consumed; a missing one exits `78` before the port is bound. The URL
a merchant's tokens carry comes from `Config`'s `deployment.public_base_url` in
the YAML, which the OP's issuer is derived from
(`vpay_api::op::issuer_for` → `{public_base_url}/v1/oauth`). There is no
`--public-base-url` flag: it was accepted, parsed and read by nothing, and was
removed on 2026-09-03, so a deployment that sets it now fails to start rather
than being silently ignored.

`--observability-bind` (`VPAY_OBSERVABILITY_BIND`, default `0.0.0.0:9090`) is a
second listener on **both** binaries, serving `GET /livez` (a static `ok`, the
liveness probe) and `GET /metrics` (Prometheus text). Neither is on the
`--bind` port, because that one is fronted by an Ingress and `/metrics` is an
operational map of the deployment. `/healthz` stays on 8080 and stays the
readiness probe. **Nothing has ever scraped `/metrics`** — every series it
exports is one a scrape *would* find, never one anyone has watched over time.
See [`docs/status.md`](docs/status.md) and
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
  `just build-dist`. `backends/Dockerfile` builds the host's *implicit* musl
  target rather than hardcoding the x86_64 triple
  ([ADR-0014](docs/adr/0014-builder-host-musl-triple.md)).
- **A stale `pgdata` volume.** The demo shop's database is created once, from
  Postgres's entrypoint, on an empty data directory. A volume from before the
  shop landed has no `shop` database and `vpay-shop` dies in `zen migrate
  deploy`. `just demo-down` removes volumes, which is the fix.

## Documentation

Start with [`docs/status.md`](docs/status.md), then:

- [Roadmap](docs/roadmap.md) — the phases from scaffold to a deployable
  gateway, and where the project stands in that sequence
- [Flows](docs/flows/) — one document per process, with invariants, each
  ending in a **Status** section stating what is actually built
- [ADRs](docs/adr/) — decisions and what they cost
- [RFCs](docs/rfc/) — proposals not yet decided
- [Reference](docs/reference/) — why the code that implements a flow is shaped
  the way it is
- [SDK parity](docs/sdks/parity.md) — the cross-SDK capability matrix, and
  every dated gap
- [Runbooks](docs/runbooks/) — what to do when an alert fires, including
  [demo.md](docs/runbooks/demo.md) and
  [checkout.md](docs/runbooks/checkout.md), the two procedures whose output is
  a real run rather than a design

Contributors: [AGENTS.md](AGENTS.md) is the source of truth for how to work
here.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
