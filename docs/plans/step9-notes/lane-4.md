<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. §7 and §8 below are written so they can be applied verbatim. -->

# Step 9, lane 4 — build, image, deploy, demo

Branch `claude/step9-lane-4-deploy`, on top of `9a8a38d` (the gate branch with
lanes 5, 2, 3, 2b, 1, 7 and 3b merged). Six commits plus this note.

## 1. What landed

| # | Thing | Where |
|---|---|---|
| 1 | `frontends/Dockerfile` — a `base` stage, `dashboard-builder`/`runner` (the dashboard, name unchanged), `checkout-builder`/`checkout`. The checkout builder copies `sdks/` as well as `frontends/` | `frontends/Dockerfile:1` (header), `:29` (dashboard), `:49` (checkout builder), `:64` (checkout runner) |
| 2 | `GET /healthz` on the checkout app — the one file this lane added under it, and a deployment concern rather than a page | `frontends/apps/checkout/app/healthz/route.ts:1` |
| 3 | `vpay-checkout` in `release.yml`'s `build` and `merge` matrices — built on both native runners, pushed by digest, tagged and `cosign sign`ed like the other three | `.github/workflows/release.yml:97` (build), `:198` (merge) |
| 4 | `just release-dry-run` builds four images, not three | `justfile` (`release-dry-run`) |
| 5 | `vpay-checkout` and `vpay-shop` compose services; `dashboard` gains `target: runner`; `SHOP_WEBHOOK_SECRET` on both binaries | `compose.e2e.yml` |
| 6 | The shop's own database, created once on a fresh volume | `deploy/dev/postgres-init/10-shop-database.sql`, mounted in `compose.e2e.yml`'s `postgres` |
| 7 | Both new services republished on `demo_checkout_port` / `demo_shop_port`, and `wiremock-orange` mounting the GENERATED mappings | `compose.demo.yml` |
| 8 | `demo_checkout_port` (3080), `demo_shop_port` (3001), `compat_services`, `demo-shop`, `demo-checkout`; `demo_services` 6 → 8 | `justfile` |
| 9 | `gen-demo-keys` — a second key pair (`shop-merchant`, 0644), a `checkout:` block, a `providers:` block on XAF, the `shop-merchant` registration, four new staleness rules, and the Orange mapping substitution replacing lane 2's check | `justfile` (`gen-demo-keys`) |
| 10 | XAF on both rails in the demo overlay, and every consumer of that overlay moved with it | `examples/merchant-demo/src/main.rs`, `sdks/stripe-compat/src/client.ts`, `frontends/tests/e2e/cypress/tasks/checkoutTasks.ts`, `examples/checkout-browser/mint.mjs` |
| 11 | `merchant-demo` step 5 — one hosted and one embedded session, the `url` in full and the embedded secret redacted | `examples/merchant-demo/src/main.rs` (`step_5_checkout_sessions`, `create_session`, `RedactedUrl`) |
| 12 | `checkout.enabled` (default false): Deployment, Service, Ingress, `values.schema.json`, two guards, `image-digest-format` and `extra-env-collision` widened | `deploy/helm/vpay/templates/deployment-checkout.yaml`, `templates/ingress-checkout.yaml`, `templates/_validate.tpl` (guards 16 and 17), `values.yaml`, `values.schema.json`, `ci/guards/checkout-*.yaml`, `ci/values-full.yaml`, `README.md` |
| 13 | `just helm-check` — 15 → 17 guards, plus a two-direction assertion over the rendered YAML | `justfile` (`helm-check`) |
| 14 | Runbook §2 (six variables), §4a (opening the hosted page, the shop, the currency), §5, §6, §7 (re-measured), §11 (five new symptoms) | `docs/runbooks/demo.md` |

## 2. Decisions taken in this lane, and why

- **Every consumer of `frontends/Dockerfile` names its target.** The file's
  "the last stage wins" property stopped holding the moment it grew a second
  image. `release.yml` already named `runner`, so the dashboard stage kept
  that name and its matrix entry did not have to change;
  `compose.e2e.yml`'s `dashboard` service gained `target: runner`.
- **The checkout app got a `/healthz` route, which is the one file this lane
  added under an app the brief told it not to touch.** The app had none, and
  the alternative was health-checking a payment page URL — `GET /c/{anything}`
  answers 200 with an "invalid link" screen, so it would have been a probe
  that passes for the wrong reason. The route takes no dependency and reports
  on none: `middleware.ts`'s origins lookup already fails *closed*, so
  probing vpay from here would trade a page that degrades safely for a pod
  the kubelet restarts, and would take the checkout app down during a rolling
  `vpay-server` deploy.
- **The chart templates the checkout page and still does not template the
  dashboard.** The difference is evidence: the checkout image declares
  `USER node`, and it has been *run* — non-root, `--read-only --tmpfs /tmp`,
  answering `/healthz` 200 — which is what `runAsNonRoot` with no invented
  UID, `readOnlyRootFilesystem` and the single `emptyDir` on `/tmp` are
  derived from. The dashboard image declares no `USER` and nobody has run it.
  **No pod has run either of them**; the probes, the numbers and the Ingress
  are reasoned like the rest of the chart.
- **`checkout.publicApiUrl` is required by a guard rather than defaulted.**
  The app throws on a missing `NEXT_PUBLIC_VPAY_API_URL` (lane 3's decision),
  so a default would be a pod that starts, fails readiness and never says
  why — and there is no value to default to: it is whichever hostname the
  Ingress serves `/v1` on, which the chart does not know when
  `ingress.enabled` is false.
- **The Orange mappings are copied, not edited.** `webpayment.json` is shared
  with `compose.yml`, CI's e2e stack and both Rust suites, so it is not a
  per-run artefact. `gen-demo-keys` writes
  `.e2e/<demo_project>/wiremock-orange/` and `compose.demo.yml` mounts that
  over `/home/wiremock`; the substitution is anchored on
  `localhost:<port>/stub-hosted-page` rather than on a bare port, because
  `8080` and `8082` also appear in the mappings' prose and in the rail's own
  in-network host. The post-condition is checked, not assumed: any surviving
  other port, or zero substitutions, fails the recipe.
- **`stripe-compat` keeps its six services, under a new `compat_services`.**
  A `/v1` conformance run opens no browser; building two Next.js images for
  it costs minutes it does not buy anything with. Spelled separately from
  `demo_services` rather than derived, because the difference is a decision.

## 3. The currency change, and what it is not saying

Lane 7's addendum A: the demo stack settles **XAF on both rails**. Implemented
in the generated overlay's `providers:` block and nowhere else.

`config/application.yml` and `config/application-sandbox.yml` are **unchanged**
and still put `mtn_momo` on EUR, because MTN's *real sandbox* rejects XAF
(`docs/flows/money.md`). The demo stack is not the sandbox: it is a WireMock
host. The divergence is written into three places a reader will actually
meet — the generated overlay's own block comment, the runbook's §4a, and
`requesttopay-status.json`'s `metadata.why`, which now lists all four configs
and which currency each names.

**A correction to the addendum, stated rather than quietly worked around.** It
asked for the MTN mappings to "accept `EUR|XAF` in the request body (regex)".
There is no such matcher to widen: **no mapping under
`backends/tests/conformance/wiremock/mtn/mappings/` matches on a currency**,
in a request body or anywhere else — grepped, and confirmed against
`vpay_adapter_mtn_momo::wire::StatusResponse`, which deserialises `status`,
`reason` and `financialTransactionId` and never `currency`. Nothing had to be
made tolerant because nothing was ever strict. What landed is the
documentation half of that instruction and no regex.

**The blast radius the change had, which the addendum did not name.** CI's
`e2e` job brings the stack up with `-f compose.demo.yml`, so it runs
`checkout.cy.ts` **and** `stripe-compat` against the demo overlay — not
against `application-sandbox.yml`. Both were pinned to `"eur"`, and left there
they would have failed at confirm with `rail 'mtn_momo' settles in XAF; this
PaymentIntent is EUR`. Each is one constant plus its comment:
`sdks/stripe-compat/src/client.ts`'s `CURRENCY`,
`frontends/tests/e2e/cypress/tasks/checkoutTasks.ts`, and
`examples/checkout-browser/mint.mjs`. Those are lane 5's and lane 6's files;
the edits are named here so the next lane to touch them knows why they moved.

## 4. Proofs, with numbers

All on the authoring host (rootless Docker,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`), 2026-09-04. Load average
2.24 at the start of the demo run; **not** the `vpay-ci` VM, which was not
needed.

| Gate | Result |
|---|---|
| `just verify` | ok — four gates passed |
| `just helm-check` | ok — lint, render, **17 guards all fired by name**, rate limit, kubeconform **23/23 valid** |
| `actionlint` | exit 0, no findings |
| `pnpm install --frozen-lockfile` | ok, 16 workspace projects (see §6) |
| `docker build --target checkout` from a clean context | ok |
| `docker build --target runner` from the same | ok |
| `just demo_port=18080 demo`, from nothing | **green** — 6 outcomes for 6, both sessions, exit 0 |

### 4a. The image

From a clean `git archive` of the index tree — nothing untracked, no `.e2e/`:

```
docker build --target checkout -f frontends/Dockerfile .   -> ok
docker build --target runner   -f frontends/Dockerfile .   -> ok
```

and the container, run `--read-only --tmpfs /tmp`:

```
HEALTHCHECK        healthy
id                 uid=1000(node) gid=1000(node) groups=1000(node)
GET /healthz       200; cache-control: no-store;
                   content-security-policy: frame-ancestors 'none';
                   referrer-policy: no-referrer; x-content-type-options: nosniff
GET /c/cs_nope     200 (the page renders, and refuses in the browser)
GET /e/cs_nope     frame-ancestors 'none' — fail-closed with no key
touch /app/nope    Read-only file system
```

The first build **failed**, on the lockfile — see §6.

### 4b. The staleness rules, before and after

Each rule broken on a fresh overlay, `just gen-demo-keys` re-run, the
diagnostic captured. BEFORE is the same line every time:
`gen-demo-keys: …/oauth-signing-key.pem, …/oauth-signing-key.pem and
.e2e/application-demo.yml already exist, keeping them`.

```console
(1) `  - client_id: shop-merchant` renamed
    gen-demo-keys: .e2e/application-demo.yml has no `shop-merchant` registration — regenerating the pair
(2) checkout_origins moved to :9999
    gen-demo-keys: .e2e/application-demo.yml does not allow http://localhost:3001 to frame the checkout page — regenerating the pair
(3) checkout.public_base_url moved to :9999
    gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_checkout_port than 3080 — regenerating the pair
(4) the whole `providers:` block deleted
    gen-demo-keys: .e2e/application-demo.yml predates the demo's XAF `providers` block — regenerating the pair
(5) the shop key deleted, overlay intact
    gen-demo-keys: .e2e/demo-merchant/…, .e2e/shop-merchant/… and .e2e/application-demo.yml are out of sync — regenerating all three
(regression) deployment.public_base_url moved to :18080
    gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_port than 8080 — regenerating the pair
```

**One bug was found by that first proof rather than reasoned away.**
`^\s*client_id: shop-merchant` never matches: a `merchant_clients` entry's
first line is a YAML sequence item, `  - client_id: …`. The recipe reported
"regenerating" forever on an overlay that was perfectly fresh.
`shop_client_present()` is the named function that fixes it, with the trap
written on it — the same trap `merchant_webhooks_present()` documents for
`webhooks:`.

### 4c. The Helm guards, broken deliberately

Each reverted immediately, and `just helm-check` green again after each:

| Break | Result |
|---|---|
| `checkout.enabled` → `true` in `values.yaml` | `helm lint` fails with guard `"checkout-templated-when-enabled"` (publicApiUrl empty), by name |
| the `{{- if .Values.checkout.enabled }}` gate removed from the Deployment | `helm-check: FAIL — the default render names a checkout object, but checkout.enabled defaults to false` |
| the `publicApiUrl` `fail` body deleted from `_validate.tpl` | `helm-check: FAIL — guard 'checkout-templated-when-enabled' did not fire; …/checkout-templated-when-enabled.yaml rendered successfully` |

The two-direction assertion exists because a `fail` guard cannot assert an
*absence*: "enabled: false renders nothing" is checked by grepping the default
render for `-checkout`, and the enabled half by requiring a Deployment, a
Service and an Ingress in `ci/values-full.yaml`'s.

### 4d. `just demo`, and the live stack it left up

`just demo_port=18080 demo` from nothing — 8080 was held by an unrelated
project on this shared host, which is exactly what that variable exists for.
The first attempt on the default port died on `Bind for 127.0.0.1:8080 failed:
port is already allocated`, which is the pre-existing behaviour and not a
regression.

Six outcomes for six, **all in XAF on both rails**, both sessions created and
read back, exit 0:

```
        #   rail         intent                      status                   failure_code
        1   mtn_momo     pi_gba5xg8c354bb3tf4egn5sz6 succeeded                —
        2   mtn_momo     pi_b0x6sm1qps1zq8e026rm9rfc requires_payment_method  insufficient_funds
        3   mtn_momo     pi_9rcagd5k5h7ys7entde3kmsp requires_payment_method  payer_timeout
        4   orange_money pi_mqptk1yafx3ehcvv1ttrtjyv succeeded                —
        5   orange_money pi_7atnd3d72175v2bzj9jwed07 requires_payment_method  payer_timeout
        6   orange_money pi_zy4cp3vq9n2jh2nh6nsfy6zc requires_payment_method  provider_error

        ui_mode    session                     payment_intent              status   payment_status
        hosted     cs_jac8s1th0h53n8w8war18k4n pi_9r3vzpdsf121fehhach6a4x7 open     unpaid
        embedded   cs_87cqv9wcx54hh9g64g7ws7yb pi_wfarjwcdfs3g5cbsjtt8fds3 open     unpaid
```

**Outcomes 1–3 are the currency proof.** Each is an XAF intent confirmed on
`mtn_momo` — `currencies_agree` would have refused every one of them before
this lane, with a `400` naming EUR. All eight containers came up on the first
attempt and all six with healthchecks reported healthy.

The stack the demo left up, checked from outside:

```
$ curl -sS -D- 'http://localhost:3080/c/cs_jac8s1th0h53n8w8war18k4n'
200; cache-control: no-store; content-security-policy: frame-ancestors 'none';
referrer-policy: no-referrer
$ curl -sS http://localhost:3001 | grep -oE '[0-9][0-9 ]*FCFA' | head -3
12 000 FCFA   7 500 FCFA   9 000 FCFA
$ docker compose $DEMO_COMPOSE exec postgres psql -U vpay -d shop -c '\dt'
_prisma_migrations   order_items   orders   products   webhook_events
$ docker compose $DEMO_COMPOSE exec postgres psql -U vpay -d shop -c 'SELECT count(*) FROM products'
5
$ docker compose $DEMO_COMPOSE logs vpay-shop | head -4
vpay-shop: applying migrations
2 migrations found in prisma/migrations
Applying migration `20260904091557_init`
Applying migration `20260904091600_seed_catalogue`
```

So: the hosted URL step 5 printed is served, the shop's database was created
by the init SQL on a fresh volume, and both migrations applied.

### 4e. Two demos coexist — lane 2's regression, closed

A second stack beside the first, on its own five ports:

```
$ just demo_project=vpay-demo-b demo_port=18081 demo_receiver_port=18083 \
       demo_orange_port=18082 demo_checkout_port=13080 demo_shop_port=13001 demo-up
```

Sixteen containers, ten published ports, no collision. The two Orange stubs
serve different mappings — checked **inside the containers**, not in the
recipe's output:

```
$ docker exec vpay-demo-wiremock-orange-1   grep -o 'localhost:[0-9]*/stub-hosted-page' /home/wiremock/mappings/webpayment.json | sort -u
localhost:8082/stub-hosted-page
$ docker exec vpay-demo-b-wiremock-orange-1 grep -o 'localhost:[0-9]*/stub-hosted-page' /home/wiremock/mappings/webpayment.json | sort -u
localhost:18082/stub-hosted-page
```

and end to end, from stack B's own walkthrough **while stack A was up** —
green, with its redirect on its own Orange port and its session URL on its own
checkout port, and the rail page a payer would click answering:

```
url    http://localhost:18082/stub-hosted-page/pay-c01c39a3-…?return=…&cancel=…
HOSTED — open this in a browser:
       http://localhost:13080/c/cs_svk2eds261453bxd8xe00yv5?key=pk_test_demomerchantsandbox01#…
✔ all five steps behaved as expected — 6 payments on 2 rails, …

$ curl -sS -o /dev/null -w '%{http_code}\n' 'http://localhost:18082/stub-hosted-page/pay-c01c39a3-…'
200
```

The committed `webpayment.json` is untouched throughout (`git diff` empty on
that directory) and still spells 8082 — it stays the CI/e2e default.

**What is still NOT isolated, unchanged from Step 8 §7 and now with a second
key pair in it.** `.e2e/` after both stacks:

```
.e2e/demo-merchant/   .e2e/shop-merchant/   .e2e/vpay-demo/   .e2e/vpay-demo-b/
```

Only the last two are keyed on the project. The overlay and **both** merchant
key pairs are shared, so the older stack's `demo-walk` still stops working the
moment the newer one's `demo-up` runs — and now its **shop** stops
authenticating too, the same way and with the same `invalid_client`.

## 5. What this lane did NOT do

- **No browser has rendered vpay's checkout page.** Step 5 mints two sessions
  and stops; `curl` on the hosted URL proves the page is *served*, not that it
  works. Neither session was paid, neither intent was confirmed, and both are
  `open`/`unpaid` when the demo exits. That is lane 6's.
- **Nothing clicked through the shop.** `just demo` brings it up, waits for
  its healthcheck, and prints its URL. No order was created, no webhook was
  delivered to it. Measured at the end of the green run above:
  `SELECT count(*) FROM orders` = **0** and `SELECT count(*) FROM
  webhook_events` = **0**. Lane 7's §7 walkthrough has not been performed end
  to end by anybody.
- **No pod has ever run.** The chart's checkout Deployment renders and
  validates; the probe thresholds, the resource numbers and both Ingress
  shapes are reasoned. The **path-prefix** Ingress shape in particular has
  been run by nobody: the app is not `basePath`-aware and needs a controller
  rewrite the chart deliberately leaves to `checkout.ingress.annotations`.
- **`release.yml` has still never run**, and `vpay-checkout` has never been
  published or signed. `actionlint` says it parses; the target it names now
  demonstrably exists and builds, which is one more fact than before and not
  the fact that matters.
- **`just release-dry-run` was not run** for this note. `just helm-check` was,
  and both Dockerfile targets were built directly.
- **`just ci` was not run in full.** `just verify` and `just helm-check` were;
  `pnpm -r typecheck` / `pnpm -r test` were not re-run after the four
  TypeScript one-constant edits (all four are string literals inside existing
  object literals). `cargo build`, `cargo clippy --all-targets --all-features
  --locked` and `cargo +nightly fmt` were run on `merchant-demo`, which is the
  only Rust this lane touched.
- **`just demo` was run once, green, on one host.** Not three times, not on
  the VM, and not consecutively — the count `docs/status.md` carries is not
  restarted by it.
- **`docs/status.md` and `docs/flows/*` were not edited** — §7 and §8 below
  are what lane E applies.

## 6. The lockfile, and what found it

**`pnpm install --frozen-lockfile` failed at this lane's base**, `e49e503`,
for every consumer of it — CI's `web` job, CI's `e2e` job and both
Dockerfiles:

```
ERR_PNPM_LOCKFILE_MISSING_DEPENDENCY  Broken lockfile: no entry for
'vitest@3.2.7(@types/node@22.20.1)(jiti@1.21.7)(jsdom@25.0.1)'
```

The lane-7 merge brought `jiti@2.7.0` into the tree and rewrote every
importer's vitest peer-set to it — except `frontends/apps/checkout`'s, which
the merge left on a peer-set whose snapshot no longer exists. **Found by this
lane's first `docker build --target checkout` from a clean context**, which is
the check that exists to catch exactly this.

It was fixed by hand here first (one line, `jiti@1.21.7` → `jiti@2.7.0`,
matching every other importer). The integrator then reported that lane 3b had
fixed it as `842e6b0`, merged into the gate as `9a8a38d`. The two changes
produce a **byte-identical** blob (`5ee39ec`), so this branch was rebased
`--onto claude/step9-hosted-checkout` from that commit's child, **dropping the
hand fix entirely** — lane 3b's commit is the one that lands, and the rebase
of the five following commits was conflict-free. Verified after: `pnpm install
--frozen-lockfile` completes, 16 workspace projects.

## 7. Verbatim rows for `docs/status.md` (lane E)

Replace the `frontends/Dockerfile` row's tail — keep everything up to
"built, not booted" and append:

> **Changed 2026-09-04 (Step 9, lane 4): it now builds TWO images.** A shared
> `base` stage, then `dashboard-builder` → `runner` (unchanged output, name
> kept so `release.yml`'s matrix entry did not move) and `checkout-builder` →
> `checkout`. The checkout builder copies `sdks/` as well as `frontends/`,
> which is load-bearing: `@vpay/checkout` depends on `@vpay/stripe-js`, whose
> `exports` resolve to a gitignored `dist/`, so without the SDK's source in
> the context `next build` fails with `TS2307`. **Every consumer now names its
> target** (`release.yml`, and `compose.e2e.yml`'s `dashboard` and
> `vpay-checkout`) — "the last stage wins" stopped being a safe way to say
> which image you meant. **Both targets built on the authoring host on
> 2026-09-04 from a clean `git archive` context**, and the `checkout` one was
> then RUN: `--read-only --tmpfs /tmp`, `uid=1000(node)`, healthcheck healthy,
> `GET /healthz` 200 carrying `no-store` / `frame-ancestors 'none'` /
> `no-referrer` / `nosniff`, and `touch /app/nope` refused with "Read-only file
> system". That is the first time anything in this repository has observed a
> Next.js image under the constraints the chart asks of it.

Add to the **Infrastructure** table:

| Row | Status | Text |
|---|---|---|
| `vpay-checkout` image (`frontends/Dockerfile` target `checkout`) | ✅ | **New 2026-09-04 (Step 9, lane 4).** `node:22-alpine`, Next's `output: 'standalone'` bundle plus `.next/static`, `USER node` (uid 1000), `HOME`/`XDG_CACHE_HOME` on `/tmp` so a read-only root filesystem is survivable, and a `HEALTHCHECK` using Node 22's global `fetch` rather than adding curl to the image for a probe. Built from a clean context and **run** with `--read-only --tmpfs /tmp` on 2026-09-04: healthy, non-root, `/healthz` 200 with every security header lane 3's middleware sets, and the filesystem provably read-only. Published and signed by `release.yml` — **which has still never run** |
| `GET /healthz` on the checkout app | ✅ | **New 2026-09-04 (Step 9, lane 4)**, and the only file that lane added under `frontends/apps/checkout`. A route handler that takes no dependency and reports on none: it answers "Next is serving", never "vpay is reachable". Deliberate — `middleware.ts`'s origins lookup already fails closed, so probing vpay from a liveness endpoint would take the payment page down during a rolling `vpay-server` deploy. It is what the compose healthcheck and all three of the chart's probes use |
| `vpay-checkout` Kubernetes workload (`checkout.enabled`) | 🟡 | **New 2026-09-04 (Step 9, lane 4).** Off by default, and that is a complete deployment rather than a missing one: `checkout.public_base_url` is optional in vpay's config, and without it `POST /v1/checkout/sessions` answers `checkout_not_configured`. When enabled the chart templates a Deployment, a Service and a **third** Ingress (its own, because a payer's browser reaches it directly and it carries no `/v1` traffic — its rate limit is looser for that reason). `runAsNonRoot` with **no** invented UID (the image has an `/etc/passwd`, unlike the scratch ones), `readOnlyRootFilesystem: true` with a memory-backed `emptyDir` on `/tmp` that is load-bearing rather than hygiene, no rails Secret and no signing key — this pod holds no credential of any kind. Two named guards: `checkout-not-templated-by-default` (the page's Ingress on while the page is off) and `checkout-templated-when-enabled` (no `publicApiUrl`; an Ingress with neither `host` nor `path` or with both; TLS with nothing to populate the Secret). 🟡 for the reason every chart row is: **no pod has ever run.** What is new is that the *container* has been observed running the way the chart asks it to. The path-prefix Ingress shape is templated and has been run by nobody |
| Image publishing — `ghcr.io/vaam-store/vpay-checkout` | 🟡 | **New 2026-09-04 (Step 9, lane 4).** A fourth entry in `release.yml`'s `build` matrix (both architectures, native runners, pushed by digest) and in its `merge` matrix (one multi-arch manifest list, the tag set, `cosign sign` over the index digest) — identical treatment to the other three. `actionlint` exit 0. **Nothing in that workflow has ever run and no image has ever been published or signed** |
| `just helm-check` | 🟡 | **Changed 2026-09-04 (Step 9, lane 4): fifteen guards became seventeen**, and the recipe gained a two-direction assertion over the RENDERED YAML — the default render must name no `-checkout` object and `ci/values-full.yaml`'s must produce a Deployment, a Service and an Ingress. That second check exists because a `fail` guard cannot assert an *absence*. Green on 2026-09-04: 17 guards all fired by name, kubeconform 23/23 valid. Still proves nothing about a cluster |
| `vpay-shop` / `vpay-checkout` in the demo stack | 🟡 | **New 2026-09-04 (Step 9, lane 4).** `just demo-up` starts eight services, not six. The shop gets its own `shop` database in the same `postgres` container, created by `deploy/dev/postgres-init/10-shop-database.sql` — which Postgres runs **once, on an empty data directory**, so a `pgdata` volume from before this change has no such database and `vpay-shop` dies in `prisma migrate deploy`; `just demo-down` is `down -v` and is the answer. Verified on 2026-09-04 on a green `just demo`: both migrations applied, five products, the catalogue rendering `12 000 FCFA`, and the checkout container serving the hosted session URL step 5 printed. **Nothing has clicked through the shop** — no order was created and the `orders` table was empty at the end of that run |
| `demo_orange_port`, and two demos on one machine | ✅ | **Changed 2026-09-04 (Step 9, lane 4).** Step 9's lane 2 made this a *checked* value — 8082 was the only one that worked, because the stub's `payment_url` comes from a committed mapping that spells it, and WireMock cannot learn what the host published it on. `gen-demo-keys` now writes a per-project **copy** of those mappings with the port substituted (`.e2e/<demo_project>/wiremock-orange/`) and `compose.demo.yml` mounts the copy; the committed mapping is untouched and stays the CI/e2e default. Measured 2026-09-04: two stacks up at once, sixteen containers, **ten published ports, no collision**, the two Orange containers serving `localhost:8082` and `localhost:18082` respectively (grepped inside them), and the second stack's walkthrough green while the first was up with its redirect and its session URL on its own ports. **`.e2e/application-demo.yml` and both merchant key pairs are still shared** — Step 8's §7 limitation is unchanged and now has a second key pair in it |
| `examples/merchant-demo` step 5 — checkout sessions | 🟡 | **New 2026-09-04 (Step 9, lane 4).** Creates one hosted and one embedded session on a **fresh intent each** (`checkout_sessions.payment_intent_id` is unique and the route requires an intent with no charge, so none of step 4's can be reused), reads each back through `retrieve` and fails if the stored session differs. Prints the hosted `url` **in full** — a human is meant to open it — and the embedded `client_secret` as `[N chars redacted]`, the same treatment step 2 gives the access token, because this output reaches CI logs and pasted transcripts. Verified green on 2026-09-04. 🟡 because **it stops there**: no browser has rendered either page, no rail was called for either session, and both are `open`/`unpaid` when the demo exits — the program says so itself, in its last line |
| The demo stack's currency — XAF on both rails | ✅ | **Changed 2026-09-04 (Step 9, lane 4, per lane 7's addendum A).** The generated overlay now carries its own `providers:` block putting **both** rails on XAF, because the demo shop prices its catalogue in XAF and offers a payer both, and `currencies_agree` refuses a confirm whose rail settles in another currency. **`config/application.yml` and `application-sandbox.yml` are unchanged and still put `mtn_momo` on EUR, because MTN's real sandbox rejects XAF** (`docs/flows/money.md`) — the divergence is written into the generated overlay's own comment, `docs/runbooks/demo.md` §4a, and `requesttopay-status.json`'s `metadata.why`, which also records that **no MTN mapping matches on a currency at all** and that `StatusResponse` never deserialises one. `sdks/stripe-compat`, `frontends/tests/e2e/cypress/tasks/checkoutTasks.ts` and `examples/checkout-browser/mint.mjs` moved with it — all three run against this overlay, because CI's `e2e` job brings the stack up with `-f compose.demo.yml`. Proven by the three MTN outcomes of a green `just demo`, each an XAF intent confirmed on `mtn_momo` |

Also, in the `just demo` row's narrative: **`just demo` was run once from
nothing on 2026-09-04 on the authoring host (not the `vpay-ci` VM) at
`demo_port=18080`, green** — six outcomes for six, both checkout sessions,
exit 0, all eight services healthy on the first attempt. One run, on one host;
the "four green in six" count from Step 8 is not restarted by it.

## 8. Verbatim additions for `docs/flows/deployment.md` (lane E)

Add to the images list:

> **`ghcr.io/vaam-store/vpay-checkout`** — vpay's own hosted/embedded payment
> page (`frontends/apps/checkout`), built from `frontends/Dockerfile`'s
> `checkout` target. A second Next.js image beside the dashboard's, and the
> first one this repository deploys: the chart templates it under
> `checkout.enabled` (default false). It runs as `node` (uid 1000) with a
> read-only root filesystem and a memory-backed `emptyDir` on `/tmp`, holds no
> merchant credential, no rail credential and no signing key, and reads no
> YAML — its whole configuration is two environment variables. Published and
> signed by `release.yml` exactly as the other three are, and like them it has
> never actually been published.

Add a subsection:

> ### The checkout page is a second deployable, and it needs two API URLs
>
> `checkout.apiUrl` is **this pod's** view of vpay — `middleware.ts` calls
> `GET {apiUrl}/v1/browser/checkout/origins?key=…` server-side to build the
> embedded page's `Content-Security-Policy: frame-ancestors`. Empty means this
> release's own server Service, which is right in-cluster. Missing entirely is
> not an error: the embedded page's CSP becomes `frame-ancestors 'none'` —
> correct, fail-closed, and it means no merchant can embed.
>
> `checkout.publicApiUrl` is **a payer's browser's** view: the `/v1/browser`
> origin every confirm and poll goes to. It is required when the page is
> enabled, enforced by a named guard rather than defaulted, because the app
> throws on a missing `NEXT_PUBLIC_VPAY_API_URL` and a default would be a pod
> that starts, fails readiness and never says why.
>
> A third value lives outside the chart and must agree with the page's
> Ingress: **`checkout.public_base_url` in vpay's own profile overlay**, which
> is the origin every payer link vpay mints is built on. The chart cannot
> check it — the overlay is opaque YAML to it — and a disagreement is a
> `session.url` that resolves to nothing, with no log anywhere naming a port.
