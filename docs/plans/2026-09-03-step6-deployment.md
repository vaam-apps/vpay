<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 6 — deployment and operations: implementation-ready design

Decisions taken by the orchestrator under the user's delegation (do not reopen): (1) images publish to
`ghcr.io/vaam-store/vpay-{server,worker,dashboard}`; (2) Helm chart at `deploy/helm/vpay` (not
kustomize); (3) cosign keyless signing via GitHub OIDC; (4) ingress-nginx with `limit-rps`, asserted in
CI on the rendered YAML; (5) alert thresholds are PROPOSED and marked provisional (the runbooks never had
numbers); (6) `metrics` + `metrics-exporter-prometheus`, delete the unused `opentelemetry` pin, OTLP
traces deferred and said so; (7) remove `--public-base-url`; (8) publish arm64 built on native
`ubuntu-24.04-arm` runners, add an explicit `[target.aarch64-unknown-linux-musl]` `+crt-static` entry,
and supersede ADR-0004's x86_64 wording with a short ADR-0014 ("the builder's host musl triple");
(9) external managed Postgres via an existing Secret, CloudNativePG documented not templated; a new
observability listener (`--observability-bind`, default 0.0.0.0:9090) serves `/livez` and `/metrics`
on BOTH binaries, never on 8080; the overlay ConfigMap mounts with `subPath`; no kind smoke test
this step (helm lint + template + kubeconform only). Coordinate the `vpay_jobs_*` metric names with
Step 4's worker loop.

## Step 6 — deployment and operations: implementation-ready design

Read against the working tree at `claude/step4-worker` (Step 4 is **uncommitted and in flight**: `backends/crates/vpay-worker/src/{jobs,handlers,recovery}.rs`, `backends/migrations/0021_create-jobs.sql` are untracked).

### 0. Seven things that are not what the brief implies — verify first

**S1 — `VPAY_LOG_FORMAT` already defaults to `json`.** `backends/crates/vpay-config/src/cli.rs:95`: `default_value = "json"`. Setting it in the chart is belt-and-braces, not a fix. Don't write a status row claiming it was added.

**S2 — the worker has no HTTP listener at all.** `backends/apps/vpay-worker-bin/src/main.rs` never calls `TcpListener::bind`; it is a `select!` over signals and a heartbeat (`:284-297`). `/livez` and `/metrics` on the worker are a **new listener plus a new CLI flag**, not a route. Note `backends/crates/vpay-worker/Cargo.toml:65` already declares `axum.workspace = true` and **no file in that crate uses it** — a spare seam, not a wired one.

**S3 — the runbooks contain no numeric thresholds to transcribe.** `docs/runbooks/provider-error-rate.md:5` says "crosses its threshold"; `unresolved-charges.md:5` says "more than one hour". There is nothing else. A `PrometheusRule` cannot transcribe thresholds that were never written — it must *propose* them (see D5).

**S4 — multi-arch is not free, and ADR-0004 does not cover it.** `backends/Dockerfile:5-26` deliberately builds the *builder's own host triple* and explains that hardcoding `x86_64-unknown-linux-musl` on arm64 "fails outright". Under buildx that works — but `.cargo/config.toml:9-10` scopes `+crt-static` to `x86_64-unknown-linux-musl` **only**, so an aarch64 build relies on rustc's musl target default rather than on this repo's explicit flag, and ADR-0004's Decision names x86_64 by name (`docs/adr/0004-musl-mimalloc.md:13`). `ring` asm + mimalloc's C build under QEMU is the slow, untested path; native runners are not.

**S5 — `--public-base-url` is documented as inert.** `docs/flows/configuration.md:41-49`: "accepted and parsed and read by nothing"; the issuer is `vpay_api::op::issuer_for(&config)` reading YAML `deployment.public_base_url` (`vpay-api/src/op/mod.rs:396`). Two spellings, one dead.

**S6 — baking config into the image makes the overlay mount a `subPath` problem.** `backends/Dockerfile:100-101` bakes `config/` at `/config`. A ConfigMap mounted at `/config` **shadows the baked base file** and the process exits 78. `compose.demo.yml` gets this right by bind-mounting a single file; Helm must use `subPath: application-<profile>.yml`, which disables ConfigMap live-update (restart required — consistent with ADR-0003's "no hot reload").

**S7 — `opentelemetry = "0.27"` (root `Cargo.toml:157`) is referenced by no crate** (grepped `backends/`, `sdks/`, `.xtask/`). It is a pin with no consumer.

---

### 1. Image publishing — `.github/workflows/release.yml`

Triggers: `push: tags: ['v*']` and `push: branches: [master]` → `:edge`. Three images, `ghcr.io/vaam-store/vpay-{server,worker,dashboard}` (repo owner confirmed `vaam-store`, public, default branch `master`). Note CodeQL runs via GitHub **default setup**, not a workflow file — no collision.

Matrix over `{server: backends/Dockerfile target=server, worker: backends/Dockerfile target=worker, dashboard: frontends/Dockerfile}`. Per S4, **native runners, not QEMU**: `ubuntu-latest` for amd64, `ubuntu-24.04-arm` for arm64, `docker/build-push-action@v6` with `outputs=type=image,push-by-digest=true`, then a `merge` job creating the manifest list with `docker buildx imagetools create`. This keeps the "host triple == target triple" invariant `backends/Dockerfile:14-26` was written around; QEMU also keeps it, but pays an emulated `ring`/mimalloc build.

Attestations: `provenance: mode=max`, `sbom: true` on `build-push-action`; `permissions: {contents: read, packages: write, id-token: write, attestations: write}`. Signing: `sigstore/cosign-installer@v3` + `cosign sign --yes ghcr.io/...@${digest}` keyless (see D3).

Tags: `type=semver,pattern={{version}}` / `{{major}}.{{minor}}`, `type=raw,value=edge,enable={{is_default_branch}}`, `type=sha,format=long`. OCI labels via `docker/metadata-action`, plus `org.opencontainers.image.source`, `.revision`, `.licenses`. `vpay-dashboard` is `node:22-alpine`-based (`frontends/Dockerfile:17`), not scratch — say so in the row rather than implying uniformity.

`just release-dry-run`: builds all three targets for both platforms with `--push=false --provenance=mode=max`, then runs `helm lint` + `helm template | kubeconform`. No registry credential needed.

---

### 2. Kubernetes — Helm, at `deploy/helm/vpay`

**Helm, not kustomize** (D2): the guard in §5 needs `fail`-on-invalid template logic and the rate-limit annotation set is controller-specific; kustomize has no failure primitive.

Objects: `Deployment/vpay-server` (`replicaCount: 2`), `Deployment/vpay-worker` (`replicaCount: 1`; the `jobs` lease — `0021`'s `lock_is_paired`, `jobs_claimable_idx` — makes >1 safe), `Service` (8080 + 9090), `ServiceAccount` (`automountServiceAccountToken: false`), `PodDisruptionBudget` (`minAvailable: 1`, server only), `ConfigMap` (profile overlay), `Ingress`, `NetworkPolicy`, optional `ServiceMonitor` + `PrometheusRule`. **No in-cluster Postgres**; `DATABASE_URL` comes from an existing Secret (`database.existingSecret`/`.key`). Document CloudNativePG as the alternative in the chart README and ADR-0013, do not template it.

Secrets, all `existingSecret` references (the chart creates none):
- `signingKey.existingSecret` → projected volume, `defaultMode: 0400`, mounted at `/secrets/oauth-signing-key.pem`, `VPAY_OAUTH_SIGNING_KEY_FILE` set to it. Server only — `vpay-worker-bin` deliberately takes no such flag (`vpay-config/src/cli.rs:179-181`).
- `rails.existingSecret` → `envFrom.secretRef`; six names today (`MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `MTN_API_USER`, `ORANGE_MERCHANT_KEY`, `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET` — `compose.e2e.yml:43-53`). Missing one is exit 78 on **both** binaries.

Overlay ConfigMap: key `application-{{ .Values.config.profile }}.yml`, mounted **`subPath`** into `/config/` (S6), `readOnly: true`. `VPAY_PROFILE` = same value.

Probes (server): `livenessProbe` → `GET /livez` :8080, `readinessProbe` → `GET /healthz` :8080. Worker: `livenessProbe` → `GET /livez` :9090, no readiness (no Service traffic).

**The `vpay-api` change, spelled out.** `backends/crates/vpay-api/src/lib.rs:279-295` is a doc comment that *pre-authorises* exactly this: "When real k8s manifests land, split this: a liveness probe should stay a static `ok` … and a new readiness probe should carry this DB check." Add `async fn livez() -> &'static str { "ok" }` with no `State`, mount `.route("/livez", get(livez))` beside `/healthz` at `lib.rs:721`, rewrite that doc comment from "deliberately not split" to what now exists (name the chart path), extend the route table at `lib.rs:567-575` with a `/livez | none` row, and add `healthz_is_still_unauthenticated`'s sibling `livez_is_static_and_needs_no_database`. **Do not** move `/healthz` off the DB check — CI's readiness gate (`ci.yml:171-184`) and `compose.e2e.yml` depend on its current meaning.

`securityContext` (pod): `runAsNonRoot: true`, `runAsUser/runAsGroup/fsGroup: 65532` (matches `backends/Dockerfile:105`), `seccompProfile: RuntimeDefault`. Container: `readOnlyRootFilesystem: true`, `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]`. `readOnlyRootFilesystem` is **unverified against a running pod** — the scratch image has no `/tmp`, and nothing in either binary writes a file; say "no observed writer" in the chart comment, not "proven".

Resources: default `requests: {cpu: 100m, memory: 128Mi}`, `limits: {memory: 256Mi}`, no CPU limit. State plainly that these are **placeholders, not measurements** — the only measurable footprint today is the `dist` binary size and the sqlx pool size; nothing has profiled RSS under load. No HPA this step.

`terminationGracePeriodSeconds: 35` vs `shutdownGraceSeconds: 25` (`vpay-config/src/cli.rs:107`), mirroring `compose.e2e.yml:76`. **No `preStop` hook** — `ShutdownSignals::install()` is the first statement in both `main`s and the startup race is already closed (`vpay-server/src/main.rs:177-178`).

Ingress: `nginx` (D4). `nginx.ingress.kubernetes.io/limit-rps` + `limit-burst-multiplier` on a **separate Ingress object** for `/v1/oauth/token` (tighter) than for `/v1`, because a single Ingress cannot carry two limits. `tls.secretName` from cert-manager. NetworkPolicy: ingress from the controller namespace to 8080 and from the monitoring namespace to 9090 only; egress to Postgres, DNS, and 443.

CI: new `deploy` job in `ci.yml` — `helm lint`, `helm template` (default + a `values-ci.yaml`), `kubeconform -strict -summary -schema-location default`. **kind smoke test: no** — it would need a real Postgres and the signing-key Secret, i.e. a second copy of the e2e job, for the ability to catch scheduling errors only. Follow-up. This job adds no test binary, so `justfile:143-145` (`expected_suites := "35"`, `min_tests := "640"`) is untouched.

---

### 3. Metrics

`metrics` + `metrics-exporter-prometheus` (D6), **not** the OTel SDK; delete the `opentelemetry` pin (S7). New shared module `backends/crates/vpay-core/src/metrics.rs` (describe/register only — no recorder installed from a library) and one `install_recorder()` per binary beside `install_crypto_provider()`.

New flag on `CommonArgs`: `--observability-bind` / `VPAY_OBSERVABILITY_BIND`, default `0.0.0.0:9090`; serves `GET /livez` and `GET /metrics`, **never mounted on the 8080 router**. Add the pair to the env-name table at `vpay-config/src/cli.rs:254-271` (a test fails if you don't).

Metric names, verbatim:

```
vpay_build_info{version,git_sha}                                  gauge
vpay_http_requests_total{route,method,status}                     counter
vpay_http_request_duration_seconds{route,method,status}           histogram
vpay_provider_requests_total{provider,operation,error_kind}       counter   # error_kind="" on success
vpay_provider_request_duration_seconds{provider,operation}        histogram
vpay_charge_transitions_total{provider,from,to}                   counter
vpay_jobs_claimed_total{kind}                                     counter
vpay_jobs_completed_total{kind,outcome}                           counter   # terminal|retry|dead_letter
vpay_jobs_oldest_claimable_age_seconds                            gauge
vpay_webhook_deliveries_total{outcome}                            counter   # Step 5
vpay_error_events_total{category,code,severity}                   counter
vpay_alert_events_total{category,code}                            counter   # Severity::Page only
```

`error_kind` uses `vpay_core::Classify::code`, the same vocabulary `provider_requests.error_kind` stores (`runbooks/unresolved-charges.md`). `vpay_alert_events_total` is incremented inside `ApiError::log`'s `Severity::Page` arm (`vpay-api/src/error.rs:464-470`) and `JobError`'s equivalent — **not** via a tracing layer, so `alert=true` in the log and the counter cannot diverge. JSON logs stay; **OTLP traces are explicitly deferred** — say so in `docs/status.md` rather than leaving the pin.

---

### 4. `docs/adr/0013-database-backups-and-retention.md`

Obligations already created: the ledger (`0005`), `provider_requests` (`0016`/`0020`, the audit trail ADR-0004 names as the *only* diagnosis path), `oauth_client_assertion_jtis` (`0011` — replay protection; losing it re-opens a replay window), `idempotency_keys` (`0015`, 24 h TTL), `events` (`0018`) and Step 5's `webhook_deliveries`, `oauth_signing_keys` (`0010` — public halves only; the private key is the Secret, so **backup and key custody are separate restore inputs**), `jobs` (`0021` — restoring an old snapshot re-runs already-completed jobs; the `dedupe_key` unique index bounds but does not eliminate this).

Propose RPO ≤ 5 min, RTO ≤ 60 min; WAL archiving with PITR (managed provider's, or CloudNativePG's `barmanObjectStore`); 30-day PITR window, 90-day full-backup retention. Restore drill → `docs/runbooks/restore-from-backup.md`, quarterly, into a scratch database, asserting the ledger balances and `one_charge_per_intent` holds.

---

### 5. Operational hardening

- **Grace-period guard as a template `fail`**, not a shell script: in `_helpers.tpl`, `{{ if lt (int .Values.terminationGracePeriodSeconds) (add (int .Values.shutdownGraceSeconds) 5) }}{{ fail "..." }}{{ end }}`. `helm template` in CI is then the gate, and `helm lint` catches it locally.
- **Ingress rate limit verified**, closing `docs/roadmap.md:346-352`: a `helm unittest` (or a `helm template | yq` assertion in the `deploy` job) asserting the `/v1/oauth/token` Ingress carries `limit-rps` and that its value is ≤ the `/v1` one. This is the *only* thing in the repo that will ever check ADR-0009's assumption.
- **Drain-under-load test** (`docs/status.md:474`, "no test exercises the timeout path"): an integration test that holds an in-flight `/v1/payment_intents` create against a WireMock rail with a delayed response, sends SIGTERM, and asserts exit 1 + the "grace period elapsed" line. Needs a slow *rail*, not a slow route — no test double enters the router, so `verify-no-mocks` stays clean.
- **`--public-base-url`: remove it** (D7). Delete `vpay-config/src/cli.rs:155-160` and the `("public_base_url", "VPAY_PUBLIC_BASE_URL")` row at `:270`, fix `:358`.
- Secrets: `kubectl describe` shows env *names* only for `envFrom.secretRef` — fine. `Debug` redaction is already covered (`vpay-config` "Secret redaction" row).

---

### 6. Runbooks

Update `provider-error-rate.md` and `unresolved-charges.md` to name the alert (`VpayProviderErrorRateHigh`, `VpayUnresolvedChargesRising`) and the metric/query each fires on. New: `deploy-and-rollback.md`, `rotate-signing-key.md`, `rotate-rail-credentials.md`, `restore-from-backup.md`. `rotate-signing-key.md` must state the two hard facts: rotation is **restart-based** (`TokenManager` holds one key for process life), and **rolling back to a retired kid crash-loops with exit 78**, not 69 — `DbError::SigningKeyRetired`, pinned by `a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69` (`vpay-server/src/main.rs:843`). Add the ADR-0010 dual-authority check (YAML `merchant_clients` **and** `disabled_clients`) to `rotate-rail-credentials.md` — `docs/roadmap.md:340-342` says no such runbook exists.

---

### 7. Work split (disjoint files)

**A — release + binary plumbing.** `.github/workflows/release.yml`; `justfile` (`release-dry-run`); `vpay-api/src/lib.rs` (`/livez`, doc comment, route table, test); `vpay-config/src/cli.rs` (`--observability-bind`, remove `--public-base-url`, env table); both `main.rs` (observability listener, `install_recorder`); root `Cargo.toml` (drop `opentelemetry`, add `metrics`/`metrics-exporter-prometheus`).

**B — chart + CI.** `deploy/helm/vpay/**`; `ci.yml` `deploy` job; NetworkPolicy/Ingress/PDB; the `fail` guard and the rate-limit assertion. Touches no Rust.

**C — instrumentation + docs.** `vpay-core/src/metrics.rs`; call sites in `vpay-api` (`error.rs`, `lib.rs` TraceLayer `on_response`), `vpay-worker`, the two adapters; `PrometheusRule`; four new runbooks + two updates; `docs/adr/0013-*.md`.

**C must coordinate with the in-flight Step 4 implementer C**, who owns `vpay-worker-bin/src/main.rs`'s `run_loop`, drain and "metrics" (`docs/plans/2026-09-03-step4-worker.md:152`). The `vpay_jobs_*` names above are the contract; agree them before either lands.

---

### 8. `docs/status.md`

New rows: `/livez` + probe split (✅ once the router test lands), `/metrics` + Prometheus exporter (🟡 — emitted, never scraped), `release.yml` / GHCR images (⛔ until a tag has actually pushed), `deploy/helm/vpay` (⛔), `PrometheusRule` (⛔), ADR-0013 (✅ as a decision, ⛔ as a drill). Update `--shutdown-grace-seconds` (`:474`) if the drain test lands.

**What stays ⛔, said plainly: no real cluster has run any of this.** CI proves only that (a) the three images build for both architectures and carry provenance/SBOM/signature, (b) `helm lint`/`helm template` succeed and every rendered object validates against upstream schemas via kubeconform, (c) the grace-period and rate-limit assertions hold on the *rendered* YAML. It proves nothing about scheduling, probe behaviour, `readOnlyRootFilesystem`, the NetworkPolicy, PDB behaviour during a rolling restart, or whether the ingress controller honours `limit-rps`.

---

## Decisions needed from a human

1. **Registry and image names.** *Default: `ghcr.io/vaam-store/vpay-{server,worker,dashboard}`.* Gained: zero new credentials (`GITHUB_TOKEN` + `packages: write`), same org as the repo, free for a public repo. Lost: GHCR is tied to GitHub availability and its retention/anonymous-pull policy; a later move to a cloud registry means changing every deployed tag reference.

2. **Helm vs kustomize.** *Default: Helm.* Gained: `fail`-based value validation (the grace-period guard is a template-time error, not a shell script), `helm lint`/`unittest`, and one artifact operators already know how to consume. Lost: templating indirection over plain YAML, and Helm-rendered manifests are harder to diff by eye than kustomize overlays.

3. **cosign keyless signing.** *Default: yes, keyless via GitHub OIDC.* Gained: no key to store or rotate; the Rekor entry ties an image to a workflow, a ref and a commit. Lost: verification requires network access to Rekor and pins your identity to `https://github.com/vaam-store/vpay/.github/workflows/release.yml@refs/tags/*` — renaming the workflow silently breaks every downstream `cosign verify`. Alternative (a stored key pair) trades that for a secret you own forever.

4. **Ingress controller.** *Default: ingress-nginx, with `limit-rps` annotations.* Gained: the rate limit ADR-0009 assumes exists is a two-line annotation, and the chart can assert it. Lost: nginx's limit is per-controller-replica, so the effective global limit is `limit-rps × replicas` — a real approximation. Envoy Gateway/Gateway API gives an exact global limit but needs a `BackendTrafficPolicy` CRD and a rate-limit service, i.e. a second component to run.

5. **Alert thresholds — genuinely undefined (S3).** *Default, proposed not transcribed: `vpay_provider_requests_total{error_kind="provider_error"}` > 5 % of that rail's requests over 15 m → warn; any `vpay_alert_events_total` increase over 5 m → page (ADR-0011 `Severity::Page`); `vpay_jobs_oldest_claimable_age_seconds` > 300 → warn.* Gained: the two existing runbooks become firable for the first time. Lost: every number is invented here, against a system that has never taken a payment — a 5 % `provider_error` rate on a real MTN sandbox may be normal. Tighten after the first week of real traffic, and say in the ADR that these are provisional.

6. **`metrics` crate vs OTel SDK.** *Default: `metrics` + `metrics-exporter-prometheus`; delete the unused `opentelemetry` pin.* Gained: a pull endpoint with no collector to run, a small dependency (no `tonic`/gRPC), and no risk to the single-`ring`-provider / rustls-only invariants `deny.toml:83-103` enforces. Lost: no traces, and a later OTLP migration means re-instrumenting call sites. Verify the exporter's transitive graph against `cargo deny` before merging.

7. **Remove or wire `--public-base-url` (S5).** *Default: remove.* Gained: one fewer inert flag in a payment binary, and no future operator setting `VPAY_PUBLIC_BASE_URL` and wondering why the issuer did not change. Lost: a chart or compose file that already sets it starts failing on an unknown flag — nothing in this repo does, but a downstream deployment might.

8. **arm64 at all, and how (S4).** *Default: publish arm64, built on native `ubuntu-24.04-arm` runners, and amend ADR-0004 (or supersede it) to say "the builder's host musl triple", which is what `backends/Dockerfile` already does.* Gained: Graviton/Apple-Silicon parity, and the `+crt-static` gap in `.cargo/config.toml:9-10` gets an explicit `[target.aarch64-unknown-linux-musl]` entry rather than an implicit default. Lost: two runner pools and a manifest-merge job; ADR-0004's Decision line has to change, and this repo's rule is supersede-never-edit. **Amd64-only** is the cheaper honest option if no arm64 target exists yet.

9. **Managed Postgres vs CloudNativePG (ADR-0013).** *Default: external managed Postgres, `DATABASE_URL` from an existing Secret; CloudNativePG documented, not templated.* Gained: PITR, WAL archiving and the restore drill are the provider's proven machinery, not code vpay owns; the chart stays a stateless-workload chart. Lost: backup policy lives outside this repo, so ADR-0013 can state obligations it cannot enforce, and the restore-drill runbook is provider-specific. CloudNativePG would put the whole thing in-repo at the cost of operating Postgres.
---

## Outcome — 2026-09-03

Written after the step landed, on the branch `claude/step6-deployment`. From
here on `docs/status.md` and the flow docs are the record; this section exists
so that a reader of the design above can see where the implementation
disagreed with it and why.

### What landed

* **§1 release pipeline** — `.github/workflows/release.yml`: three images, two
  native runner pools, push-by-digest plus a per-image `merge` job,
  `provenance: mode=max`, `sbom: true`, keyless cosign. `just release-dry-run`
  builds all three for the host architecture and then runs `just helm-check`.
  [ADR-0014](../adr/0014-builder-host-musl-triple.md) supersedes ADR-0004's
  x86_64 wording and `.cargo/config.toml` gained the explicit
  `[target.aarch64-unknown-linux-musl]` `+crt-static` entry. **Never run.**
* **§2 chart** — `deploy/helm/vpay`, 13 named template guards, `just
  helm-check`, a CI `deploy` job that runs the recipe rather than a copy of it.
  **Never applied to a cluster.**
* **§3 metrics** — all twelve names in `vpay_core::metrics`; eleven emitted,
  each at one seam; the observability listener on both binaries. See the
  deviations below for the twelfth.
* **§4 ADR-0013**, **§5 hardening**, **§6 runbooks** — as designed.
* **§8 status** — the new rows are in `docs/status.md`.

### Deviations, each deliberate

1. **`vpay_jobs_completed_total` has a fourth `outcome`: `lost`.** §3 lists
   `terminal|retry|dead_letter`. That list predates Step 4 landing
   `vpay_worker::Disposition::Lost` — a lease reaped mid-job, its answer
   thrown away. Folding it into a neighbour would make a handler outrunning
   its lease invisible, which is a real defect class, so it got its own value.
2. **`vpay_jobs_oldest_claimable_age_seconds` goes negative**, and the name in
   §3 is misleading about it. The underlying query is `min(run_at)` over every
   unleased, unparked row *including future ones*, so an idle deployment whose
   only queued work is the hourly sweep reports about `-3500` (observed on
   `just demo` at `-540.01`). The name is transcribed verbatim from §3 and was
   not changed; the correct reading is "seconds until (negative) or since
   (positive) the next queued work was due". Documented on the constant, in
   `docs/status.md` and in `docs/flows/deployment.md` §6a rather than papered
   over with an `abs()`.
3. **The drain-under-load test uses a stalled request body, not a slow rail.**
   §5 proposed holding a `/v1/payment_intents` confirm against a WireMock rail
   with `fixedDelayMilliseconds`. That would have required rebuilding the
   `backends/tests/integration` harness — container, merchant registration,
   minted `private_key_jwt`, token exchange — inside `vpay-server`'s own
   subprocess suite, to make one request slow. A client that promises
   `Content-Length: 200` and sends 29 bytes to `POST /v1/oauth/token` is slow
   for free, is not a stub of anything, and leaves the handler future genuinely
   pending inside the router. It does not prove a slow *rail* drains the same
   way; the drain is a property of the connection, not of what the handler
   awaits.
4. **The chart was written before the listener it probes existed.** §7 splits
   A (binary plumbing) from B (chart), and B landed first: `deployment-*.yaml`
   templated a liveness probe against `/livez` on 9090 while `vpay-worker-bin`
   still bound no socket at all. Both halves are on this branch and the gap is
   closed, but `docs/flows/deployment.md` keeps the struck-through sentence
   rather than deleting it, so nobody has to wonder whether it was ever true.
5. **No Grafana dashboard is templated.** §3 names the metrics and the chart
   templates a `ServiceMonitor` and a `PrometheusRule`; nothing renders a
   dashboard. A dashboard JSON committed against series nobody has ever
   scraped would be a screenshot of an assumption. The metric names and their
   caveats (negative gauge, port-calls-not-requests, `unknown` git sha) are
   written down instead.
6. **`just helm-check` is not in `just ci`.** It is in CI as a separate
   `deploy` job and in `just release-dry-run`. Adding it to `ci` would make
   every local `just ci` require `helm` and `kubeconform` on `PATH`, which no
   other recipe in this repository needs; the recipe fails with a named error
   if either is missing, so the omission is visible rather than silent.
7. **`vpay_webhook_deliveries_total` is described and emitted by nothing.**
   Step 5 is not on this branch, so there is no seam. Recording a zero so the
   series existed would read as "no failures" on a dashboard; a described name
   with no recorded handle does not appear in a scrape at all.
8. **`vpay_alert_events_total` does not cover every `alert = true` log line.**
   It covers the three sites that log a *classified* error at its own
   severity. Four other lines flag `alert = true` and carry no `Classify`
   value to derive labels from (or would double-count one that is already
   counted). Listed in full on `record_error_event` and in `docs/status.md`
   rather than left for someone to discover during an incident.

### What still has no evidence

No cluster has run the chart. No tag has been pushed, so no image exists at
`ghcr.io/vaam-store/vpay-*` and nothing has been signed. No Prometheus has
scraped a vpay process, so every alert rule is unevaluated. No backup has ever
been taken. `docs/status.md` says all of this in the rows themselves.
