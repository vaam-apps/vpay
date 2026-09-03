# Deploy and roll back

**Nobody has done this.** No cluster has ever run vpay — not a real one, not
kind, not minikube ([../status.md](../status.md),
`deploy/helm/vpay/README.md`). Every command below is written from the chart
and the binaries' own shutdown code, and none of them has been run against a
Kubernetes API server.

**Pin an image built after 2026-09-03.** Both pods probe `/livez` on port
9090; `--observability-bind` and that route landed on 2026-09-03 (step-6
block A) and both binaries' `tests/cli.rs` prove they answer. Against an image
older than that there is nothing on 9090 and the kubelet restarts both pods in
a loop. No image has been published at all yet — see §6 — so today the only
correct value for `images.*.digest` is one that does not exist.

---

## 1. Before you upgrade

```bash
just verify                              # the self-checks
just helm-check                          # helm lint + template + 15 guards + kubeconform
helm template <release> deploy/helm/vpay -f my-values.yaml | less   # read it
```

Three things the tooling will not check for you:

1. **Both digests are pinned, and pinned together.** `vpay-server` and
   `vpay-worker` share a schema and a migration set; running two versions
   against one database is not a supported configuration
   ([release.md](release.md) §4).
2. **The three Secrets exist in the namespace** — `database.existingSecret`,
   `signingKey.existingSecret`, `rails.existingSecret`. The chart creates
   none of them, and the guards catch an empty *value*, not a missing
   *Secret*.
3. **`config.profile` is the profile you meant.** A typo boots cleanly on the
   image's baked **sandbox** configuration — placeholder merchant keys,
   WireMock rail hosts — and reports itself healthy. Nothing catches this;
   `Config::load_with_env` merges the overlay only `if overlay_path.is_file()`.
   Check the rendered mount path against `VPAY_PROFILE` in the output of the
   `helm template` above.

## 2. Upgrading

```bash
helm upgrade --install <release> deploy/helm/vpay \
  -f my-values.yaml \
  --atomic \
  --timeout 10m
```

**`--atomic`.** It implies `--wait`, so helm blocks until every Deployment
reports ready, and if the upgrade fails or times out it **rolls the release
back automatically**. That is the behaviour you want for a payment gateway:
the alternative is a half-applied release with a failing Deployment and a
human deciding under pressure. Read §4 first, though — an automatic rollback
of the *manifests* does not undo a migration, and it does not undo a signing
key rotation.

**`--timeout` must exceed a full rollout**, and a rollout includes both pods
draining. With the defaults that is `terminationGracePeriodSeconds` (35 s) per
pod plus start-up time, and start-up includes running migrations against a
cold database. 10m is a starting point, not a measurement.

Watch it:

```bash
kubectl rollout status deploy/<release>-server --timeout=10m
kubectl rollout status deploy/<release>-worker --timeout=10m
kubectl get pods -l app.kubernetes.io/instance=<release> -w
```

### What the two workloads do differently

| | server | worker |
|---|---|---|
| Strategy | rolling (Deployment default) | **`Recreate`** — the old worker is gone before the new one starts. It serves no traffic, so there is nothing to keep available, and overlapping claim loops buy nothing |
| PodDisruptionBudget | `minAvailable: 1`, so a node drain cannot take both replicas | none |
| Probes | liveness `/livez` :9090, readiness `/healthz` :8080 (a real `SELECT 1`), startup `/livez` :9090 with `failureThreshold: 30` | liveness + startup `/livez` :9090; no readiness — nothing routes to it |

The startup probe exists because migrations run at boot, before the listener
binds, and against a cold database that is the slowest part of a start. It is
what stops a slow migration being counted as a liveness failure.

## 3. The grace-period guard, and what draining actually does

The chart refuses to render if
`terminationGracePeriodSeconds < shutdownGraceSeconds + 5`. It is a Helm
`fail` in `templates/_validate.tpl` named `grace-period`, so `helm lint`,
`helm template`, `helm install` and `helm upgrade` all abort with:

```
vpay chart guard "grace-period": terminationGracePeriodSeconds is 25 but
shutdownGraceSeconds is 25; the kubelet would SIGKILL the process while it is
still draining in-flight work. Set terminationGracePeriodSeconds to at least 30.
```

Defaults are 35 and 25. **Never raise `shutdownGraceSeconds` without raising
`terminationGracePeriodSeconds` with it** — the guard will stop you at render
time, which is the point.

Why the ordering matters, per binary — this is
[../status.md](../status.md)'s `--shutdown-grace-seconds` row, not a
description of intent:

- **`vpay-server`**: `serve_with_bounded_drain` races the axum drain against a
  `shutdown_grace_seconds` clock and **exits non-zero if the clock wins**,
  logging that in-flight work was cut off. ~~No test exercises that timeout
  path.~~ Corrected 2026-09-03 (Step 6 review pass), and §6 below says the
  same: `an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so`
  (`backends/apps/vpay-server/tests/cli.rs`) holds a real request open on a
  real socket — a client that promises `Content-Length: 200` on
  `POST /v1/oauth/token` and sends 29 bytes, so the `Form` extractor is
  genuinely pending inside the router — across a real SIGTERM, and asserts
  exit **1** plus the forced-cutoff WARN. It synchronises on the
  `Expect: 100-continue` interim response rather than a sleep, so "the head
  was parsed and routed before the signal arrived" is proven by the server
  rather than guessed at with a timer. What it does **not** prove is that a
  slow *rail* drains the same way (the drain is a property of the connection,
  not of what the handler awaits), and nothing has measured the bound under
  real load.
- **`vpay-worker`**: this half is real and has a test. The grace clock starts
  **when the signal arrives**, not at boot. Tasks stop claiming at the top of
  an iteration, so a clean drain settles every claimed job and hands back no
  lease (`LoopReport::released == 0`). On timeout the remaining tasks are
  aborted, `jobs::release_all` returns every lease this worker still holds,
  and the binary exits **1**.
  `a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds`
  (`backends/tests/integration/tests/worker_e2e.rs`) drives exactly that path
  against a real Postgres and asserts zero rows still leased afterwards;
  delete the `release_all` call and it fails.

So **a non-zero exit during a rolling update means work was cut off, not that
shutdown failed.** The log line is the evidence. What a kubelet reports for a
container that exits 1 while terminating has never been observed here.

There is **no `preStop` hook**, deliberately. The race a sleep-based hook
covers — a process serving before it can handle a signal — is already closed:
`ShutdownSignals::install()` is the first statement in both `main`s.

## 4. Rolling back

### The manifests

```bash
helm history <release>
helm rollback <release> <revision> --wait --timeout 10m
```

Equivalently, and preferably for an image change: pin the **previous digests**
in your values file and `helm upgrade` forward to them. A rollback and a
forward-upgrade-to-old-digests produce the same pods; the second leaves your
values file telling the truth about what is running.

### The three things a rollback does not undo

**1. Migrations.** They run at boot and there are **no down-migrations in this
repository**. An older image against a newer schema is not a supported
configuration and nothing checks it. If a release contained a migration,
rolling the image back is a decision to run old code against a new schema —
read the migration first.

**2. A signing-key rotation.** If the release rotated the signing key,
rolling the **Secret** back to the retired key crash-loops the server with
**exit 78** (`DbError::SigningKeyRetired`), not 69. `helm rollback` does not
touch a Secret the chart does not own, so this only bites if somebody rolls
the Secret back by hand — but that is exactly what "undo the deploy" means to
most people. [rotate-signing-key.md](rotate-signing-key.md) §3.

**3. Anything a rail already did.** Rolling back does not un-submit a charge.
Charges left mid-flight are chased by the poll ladder once a worker is
running; see [unresolved-charges.md](unresolved-charges.md).

### If `--atomic` rolled back for you

Find out *why* before re-running. The automatic rollback restored the
manifests; it did not restore a migration and it did not tell you what
failed.

```bash
kubectl get pods -l app.kubernetes.io/instance=<release>
kubectl logs deploy/<release>-server --previous | tail -50
kubectl get events --sort-by=.lastTimestamp | tail -30
```

| Exit code | Meaning | Usually |
|---|---|---|
| **78** | configuration — fix the deploy, restarting will not help | a missing `${VAR}` from the rails Secret (**both** binaries), an unreadable or wrong signing key, a rail named without its required credentials ([ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md)), or a **retired `kid`** |
| **69** | the database is unavailable — transient, waiting is correct | `DATABASE_URL` wrong or Postgres unreachable. Note the sqlx acquire timeout makes this take a few seconds |
| **1** | the drain clock elapsed (§3), or an unclassified error | in-flight work was cut off |

Exit 78 in a crash loop is never fixed by restarting. That distinction is the
whole reason the codes are split.

## 5. Changing configuration without changing the image

The overlay is mounted with `subPath`, so **the mounted file does not update
when the ConfigMap changes**. The chart closes that loop with a
`checksum/config-overlay` pod annotation: editing `config.overlay` in your
values changes the annotation, which makes `helm upgrade` a rolling restart
rather than a silent no-op. This is consistent with [ADR-0003](../adr/0003-yaml-configuration.md)
— configuration loads once, at boot, and there is no hot reload.

Editing the ConfigMap **directly** with `kubectl edit` changes nothing at all,
because nothing re-reads the file and nothing restarts. Go through helm.

## 6. What is unproven

Everything operational on this page. Specifically:

- No cluster has run this chart. Nothing here is evidence about scheduling,
  admission, probe behaviour, `readOnlyRootFilesystem`, NetworkPolicy
  enforcement, or PDB behaviour during a drain.
- ~~**The liveness probes point at a listener no image has.**~~ Corrected
  2026-09-03: the listener landed the same day. See the note at the top for
  what is still true about older images.
- **No image exists** at `ghcr.io/vaam-store/vpay-server` or `-worker`;
  `release.yml` has never run, so every digest a values file could pin today
  would be invented ([release.md](release.md)).
- ~~The server's drain timeout has no test.~~ Corrected 2026-09-03: it has
  one. `an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so`
  in `backends/apps/vpay-server/tests/cli.rs` holds a real request open on a
  real socket across a real SIGTERM and asserts exit 1 plus the forced-cutoff
  WARN. What it does not prove is that a slow *rail* produces the same
  outcome; the drain is a property of the connection, not of what the handler
  awaits.
- **No Prometheus has scraped either process.** `/metrics` is served and its
  contents are asserted by both binaries' `tests/cli.rs`, but no alert rule
  has ever been evaluated against a real series.
- `--atomic`'s behaviour here is helm's documented behaviour, not something
  observed against this chart.
- 10m is a guessed timeout. Nothing has measured a rollout.
