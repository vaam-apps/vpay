# The local demo: one command, both rails, six outcomes, two checkout sessions

**What this page is.** The procedure that brings vpay up from nothing on one
machine and drives six payments through it — both rails, every outcome each
rail documents — with the output of a real run pasted below rather than
narrated. It is [issue #11](https://github.com/vaam-apps/vpay/issues/11)'s
"someone can bring it up from nothing and walk through a payment end to end".

**Status, stated before anything else.** Every command and every line of
output on this page was run on 2026-09-03/04 on the authoring machine. Two
things it does _not_ claim:

- ~~**`just demo` end to end has not been observed green on that machine.**~~
  **Updated 2026-09-04.** When this page was first written, four of six
  walkthrough attempts died on a `confirm` with a `500` — **a defect in vpay and
  not in the demo**, written up in
  [§9](demo/known-flake.md#9-the-known-flake-a-real-defect-the-demo-found) rather than retried
  until green, because a green obtained by retrying is not evidence. That defect
  was fixed later the same day. ~~`just demo` from nothing then ran green on a
  branch that carried the fix.~~ **Corrected 2026-09-04, and this is what the
  page claims about green runs:**
  **one green run from nothing exists** (lane A's rebased branch, 2026-09-04,
  **without** lane G; the race is timing-dependent and did not fire, so it is a
  green _pre-fix_ run and not evidence for the fix) — six outcomes for six,
  exit 0, zero `write_matched_no_row`; **lane A's own earlier count was two
  greens in six attempts and zero for three from nothing**; **lane G did not
  re-run the demo**. **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing _after_ the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both.
- **Step 9's additions are green from nothing three consecutive times.**
  Lane 4 first ran `just demo_port=18080 demo` on the authoring host on
  2026-09-04 (host port 8080 was held by an unrelated project, which is what
  that variable exists for): six outcomes for six, both checkout sessions
  created and read back, exit 0, eight services healthy on the first attempt.
  **The runs that stand are the integrator's, in the `vpay-ci` VM on the merged
  branch (`551ec80`): `just demo` from nothing, three times in a row, all
  green** — six outcomes for six each, XAF on both rails, both sessions minted,
  `write_matched_no_row` in no run's logs. **§4 below is no longer one of
  those three:** it is a fourth run, on the same branch at `e22b591` with
  this commit's one-line correction to `examples/merchant-demo` applied,
  re-captured so the paste quotes the program's corrected sentence rather
  than a hand-edited one. See §4's own note.
  Step 8's bar of three from nothing is therefore met **and consecutive**, which
  it was not before. One earlier attempt failed before `demo-up` even started,
  on a Docker Hub token fetch: the network, not the stack.
- **No rail has ever been called.** Both rails are WireMock hosts in
  configuration, reached over HTTP exactly as a real rail would be
  ([ADR-0006](../adr/0006-no-mocks-in-main-processes.md)). A `succeeded` below
  means the worker asked a stub and the stub said `SUCCESSFUL`. **No money has
  moved and the "do not deploy" banner in [../status.md](../status.md)
  stands.**

---

## 1. Prerequisites

Docker with Compose v2.24+ (the demo overlay uses `!reset`), the toolchain
`rust-toolchain.toml` pins, and `just`, `jq`, `curl`, `openssl` on `PATH`.
Every recipe below checks for the tools it needs and names the missing one
rather than failing later with a timeout.

Nothing else has to exist first. `just demo` generates its own throwaway keys,
builds both images, and creates its own database.

## 2. The commands

```bash
just demo-down          # start from nothing; safe when nothing is running
just demo               # keys + up --wait + the walkthrough
just demo-down          # containers AND volumes
```

`just demo` is a composition of two recipes that also exist on their own, so
nothing on this page runs a path the one-liner does not:

| Recipe             | What it does                                                                                                                                                                                                |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just demo-up`     | `gen-demo-keys`, `docker compose up -d --build --wait`, then poll `/healthz`                                                                                                                                |
| `just demo-walk`   | run `examples/merchant-demo` against a stack that is already up                                                                                                                                             |
| `just demo-status` | what is running, under which project, on which host ports                                                                                                                                                   |
| `just demo-down`   | stop the stack and delete its volumes                                                                                                                                                                       |
| `just demo`        | `demo-up` then `demo-walk`                                                                                                                                                                                  |
| `just demo-staff`  | create the dashboard's staff member against a **running** stack and write the one-time password to `.e2e/<demo_project>/staff-password.txt` ([§6](demo/dashboard-sign-in.md#6-signing-in-to-the-dashboard)) |

**`just demo-walk` takes about a minute** — 58 s measured on 2026-09-10, six
payments, of which two wait a ten-second rung of `vpay_worker::poll_delay` (the
settling MTN outcome, and since issue #58 the settling Orange one) and four are
terminal on the first ask. `just demo` is that plus `demo-up`, which is an
image build the first time.

`demo-walk` is separately re-runnable, which is what you want while reading its
output: each run mints fresh idempotency keys and fresh intents.

**Three variables**, and they are the whole of the no-collision story
([§7](demo/two-stacks-on-one-machine.md#7-two-demos-on-one-machine)):

| Variable             | Default     | What it moves                                                                                               |
| -------------------- | ----------- | ----------------------------------------------------------------------------------------------------------- |
| `demo_project`       | `vpay-demo` | the Compose project name — containers, network, `pgdata` volume, **and the generated Orange stub mappings** |
| `demo_port`          | `8080`      | the host port `vpay-server` is published on                                                                 |
| `demo_receiver_port` | `8083`      | the host port the merchant webhook receiver is published on                                                 |
| `demo_orange_port`   | `8082`      | the host port the Orange rail stub is published on — the host in every redirect URL a payer follows         |
| `demo_checkout_port` | `3080`      | the host port **vpay's own payment page** is published on                                                   |
| `demo_shop_port`     | `3001`      | the host port **the demo shop** is published on                                                             |

Six now, not three, and the last three arrived with Step 9. `demo_orange_port`
was a _checked_ value until then — 8082 was the only one that worked, because
the stub's `payment_url` comes from a committed mapping that spells it — and
is now a real variable: `gen-demo-keys` writes a per-project copy of those
mappings with the port substituted, and the demo mounts the copy. See
[§7](demo/two-stacks-on-one-machine.md#7-two-demos-on-one-machine).

**This page was run with `demo_port=18080 demo_receiver_port=18083`**, because
host port 8080 was occupied by an unrelated project on the authoring machine —
which is exactly the case the variables exist for. With 8080 free, plain `just
demo` is the same run.

**`demo_port` publishes the whole server, and since Step 8 that includes the
unauthenticated `POST /provider/{code}/callback`.** ~~Compose binds
`${VPAY_DEMO_PORT:-8080}:8080` on `0.0.0.0`, so anyone on the same LAN as your
demo stack can post a rail notification at it without a credential.~~
**Changed 2026-09-04 (Step 9): every publication in `compose.demo.yml` is bound
to `127.0.0.1`** — `vpay-server`, both published WireMock hosts, the checkout
page and the shop — measured against this machine's LAN address rather than
reasoned about. Step 9 published four more ports, one of them a payment page,
which is why they were bound rather than warned about a second time. Two things
that has **not** changed: `compose.e2e.yml` is untouched and still publishes on
`0.0.0.0`, so a bare `just test-e2e`-style stack or CI's own is still
LAN-reachable; and a bind address is not an authorisation — the callback route
still moves no state, because a callback is only ever a hint that pulls an
already-queued poll forward and the authenticated status query is the one thing
that settles a charge ([provider-port.md](../flows/provider-port.md)).

`demo_port` is propagated to the three places that must agree, and
`gen-demo-keys` regenerates the profile overlay when it changes: the published
port, `deployment.public_base_url` in `.e2e/application-demo.yml` (which the OP
turns into the `issuer` on every token), and `VPAY_BASE_URL` for the demo
binary. When those disagree the OP answers `invalid_client` and **no message
mentions a port**, which is why the check exists.

## 3. Bringing it up

Readiness is `docker compose up --wait` on healthchecks, not a sleep. Postgres
and all three WireMock containers carry one; WireMock's `/__admin/health` means
"the admin API is up _and_ the mappings under `/home/wiremock` have been
loaded", which a TCP probe cannot distinguish from a JVM that has merely bound
its port.

`vpay-server` and `vpay-worker` **cannot** carry one: their runtime image is
`FROM scratch` ([ADR-0004](../adr/0004-musl-mimalloc.md)), so there is no shell
to run a check in and the only executable in the image is the binary itself.
The honest fix is a `--healthcheck` self-check mode on the binaries; until that
lands, `demo-up` observes the server from outside, polling `/healthz` exactly
as `.github/workflows/ci.yml`'s e2e job does.

**That poll is load-bearing, and the paste below proves it**: Compose prints
`Container vpay-demo-vpay-server-1 Healthy` for a container that has no
healthcheck at all — for those it reports _running_, in a progress line that
says "Healthy" — and the very next line is a `curl` that got
`(52) Empty reply from server` because the server had not finished binding. A
demo that trusted `--wait` alone for those two services would fail in the
walkthrough and blame the wrong thing.

Real output, from `just demo_port=18080 demo_receiver_port=18083 demo`. The
image build is elided (it is several minutes of ordinary buildx output on a
cold cache and says nothing about vpay); everything after it is verbatim:

```console
$ just demo_port=18080 demo_receiver_port=18083 demo
gen-e2e-signing-key: .e2e/oauth-signing-key.pem already exists, keeping it
gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_port than 18080 — regenerating the pair
gen-demo-keys: wrote .e2e/demo-merchant/oauth-signing-key.pem (3072-bit RSA, mode 0600, host-only)
gen-demo-keys: wrote .e2e/application-demo.yml — client_id=demo-merchant kid=aZbYeC696RJXBacNAF3GOCe2P6e4eOSX9g9gETeOoGs
demo-up: project vpay-demo, server :18080, receiver :18083
[... docker buildx output elided ...]
 Image vpay-demo-vpay-server Built
 Image vpay-demo-vpay-worker Built
 Volume vpay-demo_pgdata Creating
 Volume vpay-demo_pgdata Creating
 Network vpay-demo_default Creating
 Network vpay-demo_default Creating
 Volume vpay-demo_pgdata Created
 Volume vpay-demo_pgdata Created
 Network vpay-demo_default Created
 Network vpay-demo_default Created
 Container vpay-demo-wiremock-mtn-1 Creating
 Container vpay-demo-wiremock-webhook-1 Creating
 Container vpay-demo-wiremock-orange-1 Creating
 Container vpay-demo-postgres-1 Creating
 Container vpay-demo-postgres-1 Created
 Container vpay-demo-wiremock-webhook-1 Created
 Container vpay-demo-vpay-worker-1 Creating
 Container vpay-demo-wiremock-mtn-1 Created
 Container vpay-demo-wiremock-orange-1 Created
 Container vpay-demo-vpay-server-1 Creating
 Container vpay-demo-vpay-worker-1 Created
 Container vpay-demo-vpay-server-1 Created
 Container vpay-demo-wiremock-orange-1 Starting
 Container vpay-demo-postgres-1 Starting
 Container vpay-demo-wiremock-webhook-1 Starting
 Container vpay-demo-wiremock-mtn-1 Starting
 Container vpay-demo-wiremock-orange-1 Started
 Container vpay-demo-postgres-1 Started
 Container vpay-demo-wiremock-webhook-1 Started
 Container vpay-demo-postgres-1 Waiting
 Container vpay-demo-wiremock-mtn-1 Started
 Container vpay-demo-postgres-1 Waiting
 Container vpay-demo-postgres-1 Healthy
 Container vpay-demo-vpay-server-1 Starting
 Container vpay-demo-postgres-1 Healthy
 Container vpay-demo-vpay-worker-1 Starting
 Container vpay-demo-vpay-server-1 Started
 Container vpay-demo-vpay-worker-1 Started
 Container vpay-demo-vpay-server-1 Waiting
 Container vpay-demo-vpay-worker-1 Waiting
 Container vpay-demo-wiremock-mtn-1 Waiting
 Container vpay-demo-wiremock-orange-1 Waiting
 Container vpay-demo-postgres-1 Waiting
 Container vpay-demo-wiremock-webhook-1 Waiting
 Container vpay-demo-vpay-server-1 Healthy
 Container vpay-demo-wiremock-webhook-1 Healthy
 Container vpay-demo-vpay-worker-1 Healthy
 Container vpay-demo-postgres-1 Healthy
 Container vpay-demo-wiremock-mtn-1 Healthy
 Container vpay-demo-wiremock-orange-1 Healthy
demo-up: waiting for http://localhost:18080/healthz
```

Six services, named rather than left to Compose's "everything in the file
set": `postgres`, `wiremock-mtn`, `wiremock-orange`, `wiremock-webhook`,
`vpay-server`, `vpay-worker`. The seventh, `dashboard`, is deliberately absent
— see [§6](demo/dashboard-sign-in.md#6-signing-in-to-the-dashboard).

## The rest of the procedure, in order

§4 to §9 were sections of this page until 2026-09-11, when it was 1 426 lines.
They are one page each now, **verbatim**. Read them in this order; §10 to §12
below are the end of the procedure and stay here.

| Step                                                  | Page                                                                   |
| ----------------------------------------------------- | ---------------------------------------------------------------------- |
| §4 — The walkthrough, with a real run's output pasted | [demo/walkthrough.md](demo/walkthrough.md)                             |
| §4a — Opening the hosted page, the shop, the numbers  | [demo/hosted-page.md](demo/hosted-page.md)                             |
| §5 — What this proves, and what it does not           | [demo/what-it-proves.md](demo/what-it-proves.md)                       |
| §6 — Signing in to the dashboard                      | [demo/dashboard-sign-in.md](demo/dashboard-sign-in.md)                 |
| §7 — Two demos on one machine                         | [demo/two-stacks-on-one-machine.md](demo/two-stacks-on-one-machine.md) |
| §8 — The two hazards                                  | [demo/hazards.md](demo/hazards.md)                                     |
| §9 — The known flake: a real defect the demo found    | [demo/known-flake.md](demo/known-flake.md)                             |

**§5 is the one to read if you read only one.** It is the page that says what a
green run does _not_ prove, and every claim on the others is bounded by it.

## 10. Tearing down

`just demo-down` removes the containers **and** the volumes, so the next `just
demo` starts on a freshly migrated database rather than one carrying a previous
run's rows.

It takes no port — Compose matches by project name and label — but it **does**
need the project name, which is the one thing it cannot guess:

```console
$ just demo_project=vpay-demo-b demo-down
[...]
demo-down: project vpay-demo-b is gone (containers and volumes)
$ just demo_project=vpay-demo demo-down
[...]
demo-down: project vpay-demo is gone (containers and volumes)
$ docker ps -a --filter name=vpay-demo --format '{{.Names}}'
$ docker volume ls --filter name=vpay-demo --format '{{.Name}}'
$ docker network ls --filter name=vpay-demo --format '{{.Name}}'
```

All three empty — verbatim, on 2026-09-04. `just demo-status` prints every
vpay-ish project on the machine, which is the command for "is anything of mine
still up, and whose is that other stack".

## 11. When something goes wrong

| Symptom                                                                            | Cause                                                                                                                                                                                                                                                                                                                                                    |
| ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `500 api_error` on a confirm                                                       | Was [§9](demo/known-flake.md#9-the-known-flake-a-real-defect-the-demo-found)'s confirm/worker race until it was fixed on 2026-09-04. If you see one now — check the log line's `code`: `write_matched_no_row` means that race is back and is worth reporting, anything else is a different fault.                                                        |
| `invalid_client` at step 2                                                         | Either another `demo-up` regenerated the shared key pair ([§7](demo/two-stacks-on-one-machine.md#7-two-demos-on-one-machine)), or `deployment.public_base_url` disagrees with `VPAY_BASE_URL`. Step 1 prints a note when it can see the second one coming.                                                                                               |
| `/healthz` never answers in 120 s                                                  | `demo-up` prints `docker compose ps` and the last 80 server log lines. Exit 78 there means a config or CLI prerequisite is missing.                                                                                                                                                                                                                      |
| `port is already allocated`                                                        | Something holds one of the five published ports. Pass the matching variable — `demo_port=`, `demo_receiver_port=`, `demo_orange_port=`, `demo_checkout_port=`, `demo_shop_port=`. Since Step 9 every one of them is free to move; `demo_orange_port` was fixed at 8082 before that ([§7](demo/two-stacks-on-one-machine.md#7-two-demos-on-one-machine)). |
| `vpay-shop` exits in `prisma migrate deploy` with `database "shop" does not exist` | A `pgdata` volume created before Step 9. The init script runs only on an empty data directory — `just demo-down` (which is `down -v`), then `just demo`.                                                                                                                                                                                                 |
| `rail 'mtn_momo' settles in EUR; this PaymentIntent is XAF` on a confirm           | The overlay predates Step 9's XAF `providers` block. `just gen-demo-keys` says so by name and regenerates; if it does not, `rm -f .e2e/application-demo.yml` and re-run it.                                                                                                                                                                              |
| The hosted `url` step 5 printed does not load                                      | Check `demo_checkout_port` against `docker ps`, and `checkout.public_base_url` in `.e2e/application-demo.yml` — they must be the same port. `gen-demo-keys` regenerates the overlay when the variable changes, so this means something edited the overlay by hand.                                                                                       |
| The embedded page is blank, or a merchant's iframe refuses it                      | `shop-merchant`'s `checkout_origins` in the overlay must name the origin doing the framing. The browser's console says so; no server log does.                                                                                                                                                                                                           |
| An order in the shop never turns `paid`                                            | The shop only ever writes that from vpay's webhook. `docker compose … logs vpay-shop \| grep 'vpay webhook'` — no line means the delivery has not arrived (check `vpay-worker`), a `400` means the secrets in the overlay and on `vpay-shop` disagree.                                                                                                   |
| The shop answers `invalid_client` on checkout                                      | `VPAY_OAUTH_AUDIENCE` on `vpay-shop` must name vpay's **own** token endpoint (`http://localhost:{demo_port}/v1/oauth/token`), not the URL the shop POSTs to. Both compose files set it; an overlay whose `deployment.public_base_url` moved without it is the failure Step 9's lane 6 found.                                                             |
| The walkthrough hangs on settlement                                                | `docker compose … logs vpay-worker`. A worker that is not running fails the step in under two minutes with a message saying so.                                                                                                                                                                                                                          |

What the demo actually sent a rail is one command:

```bash
docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml \
    exec wiremock-mtn curl -s localhost:8080/__admin/requests
```

## 12. See also

- [`examples/merchant-demo/README.md`](../../examples/merchant-demo/README.md) — the outcome table and how each outcome is steered
- [`docs/flows/payment-lifecycle.md`](../flows/payment-lifecycle.md) — why a failure is `requires_payment_method` and not `failed`
- [`docs/flows/failures.md`](../flows/failures.md) — the closed `failure_code` vocabulary the demo asserts
- [`docs/flows/crash-safety.md`](../flows/crash-safety.md) — the three kill points, and the recovery table §9 is about
- [`checkout.md`](checkout.md) — how a merchant integrates hosted and embedded checkout, with the demo shop as the worked example
- [`docs/flows/hosted-checkout.md`](../flows/hosted-checkout.md) — the page's design: the two modes, the credentials, the iframe protocol, and what is not proven
- [`docs/plans/step9-notes/lane-7.md`](../plans/step9-notes/lane-7.md) — the shop's own record
- [`docs/plans/step9-notes/lane-4.md`](../plans/step9-notes/lane-4.md) — what Step 9 changed about this stack, and what it did not
- [`docs/status.md`](../status.md) — what is actually built
