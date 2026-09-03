# Deployment

How a vpay build becomes a running deployment: what an image contains, what
must exist around it before it can start, what order things happen in at boot,
and what stops a bad configuration before it becomes an outage.

> This document describes the **Kubernetes** deployment shape introduced in
> Step 6. `compose.yml` / `compose.e2e.yml` / `compose.demo.yml` remain the
> local and CI shape and are unchanged by it. See the Status section at the
> end for what is actually built — the short version is that the chart renders
> and has never run.

## 1. What ships

Three images, published to `ghcr.io/vaam-store/vpay-{server,worker,dashboard}`
(step-6 decision (1)).

| Image | Base | Contents |
|---|---|---|
| `vpay-server` | `scratch` | one static musl binary, plus `config/` baked at `/config` |
| `vpay-worker` | `scratch` | ditto, the worker binary |
| `vpay-dashboard` | `node:22-alpine` | the Next standalone server |

The two backend images have **no shell, no package manager and no writable
path** ([ADR-0004](../adr/0004-musl-mimalloc.md)). Three consequences follow
from that and they show up everywhere below:

* there is no `HEALTHCHECK` and no `kubectl exec` debugging — everything is
  observed from outside the container;
* the deployment configuration is *baked in*, so a config change is a rebuild
  or an overlay mount, never an edit in place ([ADR-0003](../adr/0003-yaml-configuration.md));
* `USER 65532:65532` is a raw UID, because `scratch` has no `/etc/passwd`.

## 2. Images: how they are published, tagged and signed

`.github/workflows/release.yml` is the only thing in this repository that
pushes an image; `ci.yml` builds both backend images for the e2e stack and
throws them away.

| Trigger | Tags applied to every image |
|---|---|
| `push` of a `v*` tag | `1.2.3`, `1.2`, `sha-<40 hex>` |
| `push` to `master` | `edge`, `sha-<40 hex>` |

No `latest`, deliberately: a real deployment pins a digest (§8), and a floating
`latest` invites the one deployment shape this documentation argues against.

**Both architectures are built natively.** amd64 on `ubuntu-latest`, arm64 on
`ubuntu-24.04-arm` (step-6 decision (8)) — one job per (image, architecture),
each pushing an untagged manifest **by digest**, and a `merge` job assembling
the manifest list with `docker buildx imagetools create` and applying the tags
above once. The reason is `backends/Dockerfile`: it compiles the *builder's own
host triple*, read from `rustc -vV`, so it is never a cross-compile. QEMU would
preserve that too, and pay for `ring`'s asm and mimalloc's C build emulated.
[ADR-0014](../adr/0014-builder-host-musl-triple.md) records the consequence for
`.cargo/config.toml`: `-C target-feature=+crt-static` is now stated for
`aarch64-unknown-linux-musl` as well as `x86_64-unknown-linux-musl`, so the two
images are static for the same stated reason rather than one of them relying on
a compiler default.

Both Dockerfiles are built from the **repository root** (`context: .`), the
same context `compose.e2e.yml` uses, because `backends/Dockerfile` copies
`sdks/rust` and `examples/merchant-demo` — cargo refuses to load a workspace
whose `members` list names a missing directory.

One `--build-arg` is passed: **`VPAY_GIT_SHA=${{ github.sha }}`**, which
`backends/Dockerfile` re-exports as an `ENV` into the builder stage (an `ARG`
alone is invisible to rustc) and which surfaces at runtime as
`vpay_build_info{git_sha="…"}` on `/metrics`. Its default is `unknown`, and
that is the value every build outside this workflow gets — `just
release-dry-run` deliberately passes nothing, because a dry run's images are
never pushed and stamping one with a real commit would advertise an artefact
nobody can pull. Nothing ever runs `git rev-parse`: the build context is a
`COPY` of source trees with no `.git` in it, so a sha obtained that way would
describe the build machine's checkout rather than the image.

Each image carries `provenance: mode=max` and an SBOM, both written into the
image index by buildx. For a `FROM scratch` image with no shell to inspect it
with, those are the only description of what is inside.

**Signing is keyless** (step-6 decision (3)). `cosign sign --yes <image>@<index
digest>` runs with the workflow's GitHub OIDC token; there is no key to store
or rotate, and verification names the workflow instead:

```
cosign verify \
  --certificate-identity-regexp '^https://github\.com/vaam-store/vpay/\.github/workflows/release\.yml@refs/(tags/v.*|heads/master)$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  ghcr.io/vaam-store/vpay-server:1.2.3
```

Two limits worth knowing before you build a policy on it: only the **manifest
list** digest is signed, not the per-architecture children, so pinning a child
manifest's digest is outside the signature; and renaming `release.yml` changes
the certificate identity and breaks every downstream `cosign verify`.

`just release-dry-run` rehearses what can be rehearsed locally — the three
builds for the host architecture, then `just helm-check`. It rehearses neither
the registry, nor the attestations, nor the signature, nor the other
architecture. [../runbooks/release.md](../runbooks/release.md) is the procedure.

## 3. What must exist before a pod can start

Both binaries refuse to start rather than start half-configured. Every item
here is a hard failure, not a degraded mode.

| Needed | Supplied as | Missing ⇒ |
|---|---|---|
| `VPAY_CONFIG` | baked `ENV` in the image | exit 78 |
| `DATABASE_URL` | Secret (`database.existingSecret`) | non-zero exit at boot |
| Every `${VAR}` in the config | Secret, `envFrom` (`rails.existingSecret`) | exit 78, **both** binaries |
| The RS256 signing key | Secret, mounted file (`signingKey.existingSecret`) | exit 78, **server only** |
| Postgres, reachable and migratable | outside the chart entirely | exit 69 |

The signing key is a *file*, never an environment value: that is how a
Kubernetes Secret reaches a pod, and migration `0010` dropped the column that
used to hold private key material so that the file is the only place it
exists.

## 4. Boot, in order

1. clap parses flags and environment (`vpay-config/src/cli.rs`).
2. `Config::load` reads the baked base file, then deep-merges
   `application-<profile>.yml` **if that file exists** — a missing overlay is
   silently fine, which is the single most dangerous thing on this page (see
   §6).
3. Every `${VAR}` is resolved from the process environment. An unresolved one
   is a named fatal error, never an empty string.
4. `Config::validate_all` refuses a configuration that would fail later at
   runtime instead of now.
5. The database connects and migrations run.
6. `vpay-server` binds `VPAY_BIND` and starts serving; `vpay-worker` starts its
   claim loop. Both install their shutdown signal handler *first*, before any
   of the above, so a SIGTERM during boot is never lost.
7. **Last**, both bind `VPAY_OBSERVABILITY_BIND` and serve `/livez` +
   `/metrics` on it. The ordering is the entire meaning of `/livez`: a probe
   against a process that is still in steps 1–5, or about to exit 78, is
   *refused* rather than answered `ok`. See §7a.

## 5. Shutdown

SIGTERM starts a drain bounded by `--shutdown-grace-seconds` (25 s by default).
Both binaries exit **non-zero when the deadline elapses**, i.e. a non-zero exit
means work was cut off, not that shutdown failed.

The Kubernetes side must therefore give the process more time than the process
gives itself: `terminationGracePeriodSeconds` (35) > `shutdownGraceSeconds`
(25) + margin. The chart makes that a template-time failure rather than a
comment — the `grace-period` guard.

There is **no `preStop` hook**. A sleep-based hook exists to cover a startup
race in which the process begins serving before it can handle a signal; that
race is already closed by installing the handler first.

## 6. Configuration overlays in Kubernetes: the `subPath` rule

The image bakes `config/` at `/config`. A ConfigMap mounted **at** `/config`
replaces the whole directory, the baked base file disappears, and the process
exits 78 naming a file it can no longer see.

So the overlay is mounted as a single file with `subPath`:

```
/config/application.yml            <- baked into the image
/config/application-<profile>.yml  <- ConfigMap, subPath mount
```

`subPath` mounts do not track ConfigMap updates. That is consistent with
ADR-0003 (no hot reload) and the chart closes the loop with a
`checksum/config-overlay` pod annotation, so editing the overlay produces a
rolling restart rather than a change nothing picks up.

**What can still go wrong, and nothing catches it:** the process treats a
missing overlay as success. A typo in `VPAY_PROFILE` produces a pod that boots
cleanly on the image's baked *sandbox* configuration — placeholder merchant
keys, WireMock rail hosts — and reports itself healthy. There is no diagnostic
because, from the process's point of view, nothing went wrong.

## 6a. Observability: two ports, twelve names, and no scraper

**The second listener.** `--observability-bind` / `VPAY_OBSERVABILITY_BIND`,
default `0.0.0.0:9090`, on **both** binaries — the worker had no HTTP
listener of any kind before it. It serves exactly two paths and nothing else:

| Path | Answers | Probe role |
|---|---|---|
| `GET /livez` | a static `ok`, no state, no database | **liveness**, both binaries |
| `GET /metrics` | Prometheus text exposition (`text/plain; version=0.0.4`) | — |
| `GET /healthz` (on `--bind`, port 8080) | `SELECT 1` against Postgres | **readiness**, server only |

Why the split is exactly this way round: a liveness probe that fails on a
database outage restarts every pod in the deployment, repeatedly, and a
restart cannot fix a database. A readiness probe that fails on one correctly
takes the pod out of service. Why a second *port* rather than two more
routes: `/metrics` names every rail this deployment talks to, every route
pattern it serves and every error code it has produced, and `--bind` is the
port an Ingress fronts. The NetworkPolicy admits 9090 from the monitoring
namespace only — a policy it can only express because the two are different
ports, where a path-based exclusion would depend on an ingress controller's
rule ordering staying correct forever.

**What is emitted.** Twelve names, owned by `vpay_core::metrics`, which
describes them and installs no recorder — each binary installs exactly one,
beside its rustls provider. All twelve have a live seam:
`vpay_build_info`; `vpay_http_requests_total` and
`vpay_http_request_duration_seconds` (labelled by matched route *pattern*,
never a concrete path, and by one of nine methods or `other` — both label
sets are closed, because an unauthenticated caller controls both the path
and the method); `vpay_provider_requests_total` and
`vpay_provider_request_duration_seconds` (per *port call*, not per HTTP
request); `vpay_charge_transitions_total`; the three `vpay_jobs_*`; and
`vpay_error_events_total` / `vpay_alert_events_total`, incremented in the
same statements that write `alert = true` to the log so the two cannot
diverge. `vpay_webhook_deliveries_total` is emitted at the two points a
delivery attempt's outcome becomes durable in
`vpay_worker::webhooks::handle_deliver` / `record_failure`, once Step 5's
webhooks landed on this branch (this rebase pass). `docs/status.md` names the
seam for each.

**What an operator should know before trusting a dashboard.**

* `vpay_jobs_oldest_claimable_age_seconds` **goes negative on a healthy idle
  queue** — it is `now - min(run_at)` over unleased rows *including future
  ones*, so a deployment whose only queued work is the hourly sweep reports
  around `-3500`. Read it as "seconds until (negative) or since (positive)
  the next queued work was due". A `> 300` alert is unaffected; an `abs()`
  applied to make the graph tidy would hide the case it exists for.
* `vpay_build_info{git_sha}` is `unknown` unless the image was built with
  `--build-arg VPAY_GIT_SHA=…`. `release.yml` passes `github.sha`; every
  local build and `compose*.yml` do not, and nothing ever shells out to
  `git`.
* **Nothing has ever scraped any of this.** The chart's `ServiceMonitor` and
  `PrometheusRule` are both off by default, no cluster has run the chart, and
  no Prometheus has polled a vpay process. The series exist; the alerts on
  them have never been evaluated.
* `VpayProviderErrorRateHigh` counts **every** non-successful port call
  (`error_kind!=""`), which is what lets it fire during a rail outage
  (`provider_unavailable`) — and which also means an ordinary decline
  (`charge_declined`) counts against it. On mobile money that is a large,
  normal share of traffic, so expect this rule to fire on a healthy system
  until someone decides, with measured traffic, whether to exclude declines.
  See `docs/runbooks/provider-error-rate.md`.

**Traces: deliberately absent.** Step-6 decision (6) chose `metrics` +
`metrics-exporter-prometheus` over the OpenTelemetry SDK and deleted the
unused `opentelemetry` pin rather than keeping it "for later". A slow request
can be seen in `vpay_http_request_duration_seconds` and cannot be
decomposed; JSON logs carrying a request id on every event are the
correlation mechanism until an OTLP decision is made.

## 7. What guards the deployment

| Guard | Where | Catches |
|---|---|---|
| 15 named `fail` guards | `deploy/helm/vpay/templates/_validate.tpl` | Value combinations that are well-typed and cannot work — see the chart README |
| `helm lint` + `helm template` + `kubeconform -strict` | CI `deploy` job / `just helm-check` | Malformed templates, objects that do not match their schema |
| `limit-rps` assertion on the rendered Ingress | same | The rate limit [ADR-0009](../adr/0009-dashboard-oidc-provider.md) assumes exists silently disappearing |
| `Config::validate_all` | the process | Configuration that would fail at runtime |

The rate limit deserves its own sentence. ingress-nginx applies `limit-rps`
per Ingress object, so the chart renders two: `/v1`, and a tighter one for
`/v1/oauth/token` (an RSA verification plus a database write per request). The
limit is enforced per controller *replica*, so the effective global limit is
approximately `limit-rps × replicas` — an approximation, stated rather than
hidden.

## 8. Rotation and rollback

Each of these now has a runbook; this section is the summary, not the
procedure.

**Signing key.** Rotation is restart-based: `TokenManager` holds one key for
the life of the process. Update the Secret, restart the server Deployment.
**Rolling back to a retired `kid` crash-loops with exit 78**, not 69 —
`DbError::SigningKeyRetired`. Roll forward.
→ [../runbooks/rotate-signing-key.md](../runbooks/rotate-signing-key.md)

**Rail credentials.** Update the Secret and restart both workloads; the worker
reads the same configuration as the server and will fail the same way if a
placeholder stops resolving.
→ [../runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md),
which also carries [ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)'s
dual-authority check for revoking a *merchant* client (YAML `merchant_clients`
**and** `disabled_clients`).

**Deploys and rollbacks.** `helm upgrade --atomic`, the `grace-period` guard,
and the three things a rollback does not undo — a migration, a signing-key
rotation, and anything a rail already did.
→ [../runbooks/deploy-and-rollback.md](../runbooks/deploy-and-rollback.md)

**The database.** Backups, PITR and retention are
[ADR-0013](../adr/0013-database-backups-and-retention.md), which is
**proposed** — no backup of any vpay database has ever been taken. The chart
templates no Postgres (decision (9)), so this is an obligation on whoever
operates the database.
→ [../runbooks/restore-from-backup.md](../runbooks/restore-from-backup.md)

**Images.** Pin by digest for anything real. `images.<component>.digest`
renders `repo@sha256:…` and the chart refuses a malformed one at template time
rather than at pull time.

## 9. What the deployment cannot do yet

* ~~**Serve `/livez` or `/metrics`.**~~ Built on 2026-09-03 — see §6a. The
  chart's liveness probes now point at a listener that exists.
* **Be scraped.** The endpoint is real and no Prometheus has ever collected a
  sample from it: the `ServiceMonitor` is off by default and no cluster has
  run this chart. Every alert in the `PrometheusRule` is therefore still
  unevaluated, which is a different statement from "the metric is missing"
  and is the one that is now true.
* **Deploy the dashboard.** The chart templates no dashboard workload; see the
  chart README for why.
* **Be pulled from anywhere.** §2 describes a workflow that has never run.
  There is no image at `ghcr.io/vaam-store/vpay-server`, none at `-worker`,
  none at `-dashboard`, and no signature to verify on any of them. Every
  `images.*.digest` a values file could pin today would be invented.
* **Prove any of the above in a cluster.** No cluster has run any of it.

---

## Status

**🟡 — designed, rendered, schema-validated, never applied.** Written
2026-09-03 (Step 6, block B).

What exists:

- `deploy/helm/vpay` — chart, values, `values.schema.json`, 15 named template
  guards, and a `ci/` directory holding one values file per guard plus a
  "everything on" values file.
- `just helm-check` and CI's `deploy` job (the job runs the recipe, so the two
  cannot drift): `helm lint` × 2, `helm template` × 2, one `helm template` per
  guard file asserting a non-zero exit whose message names that guard, a grep
  for `nginx.ingress.kubernetes.io/limit-rps` on the rendered Ingress plus an
  ordering check, and `kubeconform -strict -summary` over both renders with
  the Prometheus CRD schemas from the datreeio catalog.
- Measured on the authoring machine, 2026-09-03: **15 guards, all fired by
  name; 20 resources validated across the two renders — 20 valid, 0 invalid,
  0 skipped.** Negative controls run: disabling the `grace-period` guard, and
  separately the `rate-limit-ordering` guard, makes `just helm-check` fail;
  deleting the `limit-rps` annotation from the template makes it fail. The two
  guards the review pass added were put through the same control — neutering
  the `fail` for `rails-egress-except`, then for `extra-env-collision`, makes
  the recipe report `guard '<name>' did not fire` and exit non-zero.
  Thirteen when block B landed; the Step 6 review pass added
  `rails-egress-except` (a rails egress `except` list missing
  `169.254.0.0/16`, i.e. a pod that can reach the cloud metadata endpoint)
  and `extra-env-collision` (a `server.extraEnv`/`worker.extraEnv` name that
  silently shadows one the chart sets — Kubernetes keeps the last entry, and
  for `DATABASE_URL` that means replacing a `secretKeyRef` with a literal).
  `just helm-check` now also asserts that the fifteen names it expects are
  exactly the fifteen files under `ci/guards/`, so deleting a guard *and* its
  values file fails instead of passing quietly.

What does not exist, stated plainly:

- **No cluster has ever run this — not a real one, not kind.** A kind smoke
  test was deliberately excluded from this step (step-6 decision (9)). Nothing
  here is evidence about scheduling, admission, probe behaviour,
  `readOnlyRootFilesystem`, NetworkPolicy enforcement, PDB behaviour during a
  drain, or whether an ingress controller honours `limit-rps`. CI checks that
  the annotation is present in the rendered YAML. That is the entire claim.
- ~~**The liveness probes point at a listener that does not exist yet.**~~
  **Corrected 2026-09-03, same day:** block A landed `--observability-bind`,
  `/livez` and the worker's first HTTP listener, and both binaries' own
  `tests/cli.rs` prove the two paths answer on that port and 404 on the
  traffic port. This sentence is left struck through rather than deleted
  because the chart was written against the earlier state and a reader
  comparing the two should see that the gap closed rather than wonder whether
  it was ever real.
- **The `PrometheusRule`'s metrics exist; its rules have never been
  evaluated.** Block C landed the instrumentation the same day, so every
  query now names a series a scrape would find, with the label sets the
  queries select on. What has not happened is a scrape: no Prometheus has
  polled a vpay process, so no rule has ever fired, failed to fire, or been
  tested against real data. Every threshold is still *proposed* rather than
  derived from traffic (step-6 decision (5)), and each rule carries a
  `provisional: "true"` label so this is visible in Alertmanager.
- **The `release.yml` workflow now exists and has never run** (Step 6, block A,
  same day). §2 above describes it: three images, two native runner pools, a
  manifest merge, provenance/SBOM, keyless cosign signing. No tag has been
  pushed, so nothing has been published and nothing has been signed. `edge`,
  produced by the first merge to `master`, is the first evidence any of it
  works. `actionlint` reports the file clean; that is a syntax check, not a
  run. `aarch64-unknown-linux-musl` has never been compiled anywhere —
  [ADR-0014](../adr/0014-builder-host-musl-triple.md) adds its `+crt-static`
  entry to `.cargo/config.toml` and says so.
- [../runbooks/release.md](../runbooks/release.md) covers cutting a tag,
  verifying a signature and pinning a digest — written from the workflow file,
  never followed. **The deploy/rollback, key-rotation, rail-credential and
  restore runbooks now exist too** (block C, same day):
  [deploy-and-rollback.md](../runbooks/deploy-and-rollback.md),
  [rotate-signing-key.md](../runbooks/rotate-signing-key.md),
  [rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md),
  [restore-from-backup.md](../runbooks/restore-from-backup.md). **None of
  their `kubectl`/`helm` steps has been run against a cluster**, because no
  cluster has run vpay; the only part with evidence behind it is
  `restore-from-backup.md`'s SQL, executed against a scratch
  `postgres:16-alpine` with all 21 migrations applied, including a negative
  control on the ledger-balance check.
  [ADR-0013](../adr/0013-database-backups-and-retention.md) records the
  backup obligations and is **proposed** — no backup has ever been taken.

**Added 2026-09-03 (Step 6, block C): the instrumentation §6a describes.**
All twelve metric names are now emitted, each at exactly one seam (the
twelfth, `vpay_webhook_deliveries_total`, was described-but-unrecorded on the
pre-rebase branch, because Step 5's webhooks were not yet on it; wiring its
seam in `vpay_worker::webhooks::handle_deliver` is what this rebase pass
added), and the seams are asserted rather than described — `worker_e2e`
scrapes a real observability listener after a confirm has been driven to
`succeeded` through a WireMock MTN rail and asserts the exact series text for
the rail counter, all four charge-transition edges the poll ladder actually
produced, and the confirm's `/v1/payment_intents/{id}/confirm` route pattern;
`vpay-server`'s `tests/cli.rs` asserts
`vpay_http_requests_total{route="/healthz",…}` on the running binary;
`webhooks.rs`'s `the_ladder_walks_delivery_delay_and_then_succeeds` and
`a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled` scrape the
same listener and assert `vpay_webhook_deliveries_total{outcome="retry"} 3`,
`{outcome="succeeded"} 1` and `{outcome="exhausted"} 1` respectively. A live
`just demo` run then confirmed the series outside the test suite too:
`vpay_webhook_deliveries_total{outcome="succeeded"} 1` was present on a
`/metrics` scrape taken from inside the compose network after the demo's
webhook step delivered. What that does **not** establish, and what keeps this
page 🟡: no Prometheus has ever scraped a vpay process on a schedule, so no
rate, no histogram quantile and no alert rule has been evaluated against real
data; and `vpay_build_info{git_sha}` reads `unknown` on every build anyone has
made, because `release.yml` — the only thing that passes a real one — has
never run.
