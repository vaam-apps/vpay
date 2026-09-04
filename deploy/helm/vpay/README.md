# vpay Helm chart

Renders the two shipping vpay workloads — `vpay-server` and `vpay-worker` —
and the Kubernetes objects around them.

**Read [Status](#status) before you install this.** Nothing in this chart has
ever been applied to a cluster, and no Prometheus has ever scraped a vpay
process. The listener its liveness probes point at does now exist — see
Status for what changed and what still has no evidence behind it.

---

## What it renders

| Object | Name | Notes |
|---|---|---|
| `Deployment` | `<release>-server` | `server.replicaCount` (2 by default) |
| `Deployment` | `<release>-worker` | `worker.replicaCount` (1); `strategy: Recreate` |
| `Deployment` | `<release>-checkout` | Optional (`checkout.enabled`, **false** by default) — vpay's own payment page |
| `Service` | `<release>` | ClusterIP, ports `http` (8080) and `metrics` (9090) |
| `Service` | `<release>-worker` | Headless, `metrics` only — exists so the worker can be scraped |
| `Service` | `<release>-checkout` | Optional; `http` only — the page emits no metrics |
| `ServiceAccount` | `<release>` | `automountServiceAccountToken: false` |
| `PodDisruptionBudget` | `<release>-server` | `minAvailable: 1`; server only |
| `ConfigMap` | `<release>-config-overlay` | Optional; the profile overlay, mounted with `subPath` |
| `Ingress` | `<release>-api` | `/v1`, ingress-nginx annotations incl. `limit-rps` |
| `Ingress` | `<release>-token` | `/v1/oauth/token`, a tighter `limit-rps` |
| `Ingress` | `<release>-checkout` | Optional; the payment page, on its own host or a path prefix |
| `NetworkPolicy` | `<release>-server`, `<release>-worker` | Optional, default-deny both directions |
| `ServiceMonitor` | `<release>-server`, `<release>-worker` | Optional; needs the prometheus-operator CRDs |
| `PrometheusRule` | `<release>` | Optional; **every threshold is proposed, every metric unemitted** |

It renders **no Secret** and **no database**. See
[Secrets](#secrets-the-chart-creates-none) and [Postgres](#postgres).

## Install

```bash
# 1. The three Secrets this chart references but does not create.
kubectl create secret generic vpay-database \
  --from-literal=url='postgres://vpay:...@db.example:5432/vpay?sslmode=require'

kubectl create secret generic vpay-oauth-signing-key \
  --from-file=oauth-signing-key.pem=./oauth-signing-key.pem

# One key per `${VAR}` in the baked config of the image you are deploying:
#   grep -o '${[A-Z_]*}' config/application.yml | sort -u
# Seven on this branch (2026-09-03): MERCHANT_WEBHOOK_SECRET, MTN_API_KEY,
# MTN_API_USER, MTN_SUBSCRIPTION_KEY, ORANGE_CLIENT_ID, ORANGE_CLIENT_SECRET,
# ORANGE_MERCHANT_KEY — the list grows as features land.
# `--from-env-file`, not `--from-literal`: a credential on a command line is
# in your shell history and in `ps` output. See
# docs/runbooks/rotate-rail-credentials.md §2.
umask 077 && : > rails.env && chmod 600 rails.env && "${EDITOR:-vi}" rails.env
kubectl create secret generic vpay-rails --from-env-file=rails.env
shred -u rails.env 2>/dev/null || rm -f rails.env

# 2. Render it and read it. This chart argues, in comments, for most of what
#    it does; the rendered output carries those comments.
helm template vpay deploy/helm/vpay -f my-values.yaml | less

# 3. Install.
helm upgrade --install vpay deploy/helm/vpay -f my-values.yaml
```

A minimal real `my-values.yaml`:

```yaml
images:
  server: { digest: "sha256:<64 hex>" }   # pin by digest for a real deployment
  worker: { digest: "sha256:<64 hex>" }

config:
  profile: production
  createOverlayConfigMap: true
  overlay: |
    deployment:
      name: vpay
      livemode: true
      public_base_url: https://api.vpay.example
    providers:
      - code: mtn_momo
        host: { url: https://proxy.momoapi.mtn.com, label: mtn-production }
        currency: XAF
        settings:
          subscription_key_header: Ocp-Apim-Subscription-Key
          target_environment: mtncameroon
          api_user: ${MTN_API_USER}
        credentials:
          subscription_key: ${MTN_SUBSCRIPTION_KEY}
          api_key: ${MTN_API_KEY}
    # ... and the rest of the deployment's configuration

ingress:
  enabled: true
  host: api.vpay.example

networkPolicy:
  enabled: true
  database:
    cidrs: ["10.0.4.7/32"]
```

## Two things about configuration that will bite you

**1. The overlay is mounted with `subPath`, and it must be.** `backends/Dockerfile`
bakes the whole `config/` directory into the image at `/config`. Mounting a
ConfigMap *at* `/config` replaces that directory, the baked
`application.yml` disappears, and the process exits 78 complaining about a
file it can no longer see. The chart therefore mounts a single file at
`/config/application-<profile>.yml`.

The consequence is that the mounted file does **not** update when the
ConfigMap changes. That is fine — ADR-0003 has no hot reload anyway — and the
chart puts a `checksum/config-overlay` annotation on both pod templates so an
overlay edit becomes a rolling restart instead of a silent no-op.

**2. A missing or wrong-named overlay is not an error to the process.**
`Config::load_with_env` merges the overlay only `if overlay_path.is_file()`.
A deployment that typos `config.profile` boots happily on the image's baked
sandbox configuration — placeholder merchant keys, WireMock rail hosts — and
says nothing. The `overlay-empty` guard catches the case where you asked for a
ConfigMap and gave it no content; nothing can catch a profile typo, so check
`kubectl exec`… except there is no shell in the image. Check the rendered
mount path against `VPAY_PROFILE` before you install.

**The OP's issuer comes from the overlay**, as `deployment.public_base_url`.
There is no environment variable for it: step-6 decision (7) removes the inert
`--public-base-url` flag, and YAML is the only spelling that ever worked.

## Secrets: the chart creates none

Three Secrets must exist in the release namespace before install.

| Value | Default name | Shape | Consequence if wrong |
|---|---|---|---|
| `database.existingSecret` / `.existingSecretKey` | `vpay-database` / `url` | one key holding a full `postgres://` URL | both binaries fail to start |
| `signingKey.existingSecret` / `.key` | `vpay-oauth-signing-key` / `oauth-signing-key.pem` | PEM RSA private key (PKCS#8 or PKCS#1) | `vpay-server` exits 78 |
| `rails.existingSecret` | `vpay-rails` | one key per `${VAR}` in the deployed image's `config/application.yml` — read it at upgrade time, the list grows | exit 78 on **both** binaries |

The signing key is mounted on the **server only**. `vpay-worker-bin` takes no
`--oauth-signing-key-file`, issues no token, and mounting the Secret there
would widen its blast radius for no capability.

The rail Secret is projected with `envFrom.secretRef`, so `kubectl describe pod`
shows the variable *names* and never the values.

### `signingKey.defaultMode` is `0440`, not `0400`

A Secret volume in a pod with `fsGroup` set is owned by `root:<fsGroup>`, and
these pods run as UID 65532. `0400` leaves the file readable by root alone —
i.e. unreadable by the only process in the image, which then exits 78 naming a
file it can see and cannot open. The group read bit is what makes it work.

This is reasoned from Kubernetes' documented ownership rule for projected
Secret volumes. **It has not been observed in a running pod**, because no pod
has run. The step-6 design document says `0400`; this is a deliberate
departure from it and the reason is above.

### Signing-key rotation

Rotation is **restart-based**: `TokenManager` holds one key for the life of the
process. Update the Secret, then `kubectl rollout restart deploy/<release>-server`.

**Rolling back to a retired `kid` crash-loops with exit 78**, not 69 —
`DbError::SigningKeyRetired`. Roll forward, never back.

## Postgres

There is none in this chart, deliberately (step-6 decision (9)). `DATABASE_URL`
comes from `database.existingSecret` and from nowhere else.

**CloudNativePG** is the documented in-cluster alternative and is deliberately
*not* templated here. If you want it, install the operator and a `Cluster`
separately, then point `database.existingSecret` at the Secret CNPG generates
(`<cluster>-app`, key `uri`):

```yaml
database:
  existingSecret: vpay-pg-app
  existingSecretKey: uri
networkPolicy:
  database:
    namespace: vpay
    podSelector:
      cnpg.io/cluster: vpay-pg
```

The reason it is not in this chart: backup, PITR and the restore drill are the
obligations ADR-0013 records, and a chart that templates a database implies it
owns them. A managed instance's provider owns them, and CNPG's
`barmanObjectStore` is a configuration decision that belongs with whoever
operates the cluster — not with an `if .Values.postgresql.enabled` in a
payment gateway's chart.

## Guards

The chart refuses to render on a combination of values that is well-typed and
still cannot work. Each guard calls Helm's `fail`, so `helm lint`,
`helm template`, `helm install` and `helm upgrade` all abort, and each message
names itself so a test can assert *which* one fired:

```
Error: execution error at (vpay/templates/deployment-server.yaml:1:4):
vpay chart guard "grace-period": terminationGracePeriodSeconds is 25 but
shutdownGraceSeconds is 25; the kubelet would SIGKILL the process while it is
still draining in-flight work. Set terminationGracePeriodSeconds to at least 30.
```

| Guard | Fires when | Why it matters |
|---|---|---|
| `grace-period` | `terminationGracePeriodSeconds < shutdownGraceSeconds + 5` | The kubelet kills the process mid-drain; every rolling update truncates in-flight work |
| `database-secret` | either `database.existingSecret` / `.existingSecretKey` is empty | `DATABASE_URL` has no other source and the chart creates no Secret |
| `signing-key-secret` | either `signingKey.existingSecret` / `.key` is empty | `vpay-server` exits 78; a chart-generated key would mint tokens other replicas cannot verify |
| `rails-secret` | `rails.existingSecret` is empty | An unresolved `${VAR}` is exit 78 on **both** binaries. The guard checks a Secret is named, never which keys are in it — that list is the image's, not the chart's |
| `image-digest-format` | a digest is set and is not `sha256:` + 64 hex | A truncated digest fails at image pull, in the cluster, not here |
| `worker-replicas` | `worker.replicaCount < 1` | No job is claimed; intents sit in `processing` while everything reports healthy |
| `pdb-minavailable` | `podDisruptionBudget.minAvailable >= server.replicaCount` | No voluntary eviction is ever allowed, so node drains hang for ever |
| `observability-port` | `observability.port` equals `server.port` or `service.port` | Publishes `/metrics` on the Ingress-facing port |
| `rate-limit-ordering` | token `limitRps` > api `limitRps`, or either is ≤ 0 | Inverts the whole reason there are two Ingress objects; nginx treats ≤ 0 as no limit at all |
| `ingress-host` | ingress enabled with an empty host, or TLS enabled with neither issuer nor secret | A host-less rule answers for other applications' hostnames; a TLS block nothing populates serves the controller's default certificate |
| `overlay-empty` | overlay ConfigMap requested with empty content, or an empty profile | The process treats an empty overlay as success and runs on baked sandbox placeholders |
| `dashboard-not-templated` | `dashboard.enabled: true` | This chart templates no dashboard workload — see below |
| `checkout-not-templated-by-default` | `checkout.ingress.enabled` with `checkout.enabled: false` | An Ingress routing to a Service the chart did not template: a 503 on the payment page, found by a payer |
| `checkout-templated-when-enabled` | enabled with no `publicApiUrl`; or an Ingress with neither `host` nor `path`, or with both; or TLS with nothing to populate the Secret | The app throws on a missing `NEXT_PUBLIC_VPAY_API_URL`, so the pod starts and never passes readiness; a host-less rule answers for other applications; a payer's session credential rides in that URL's fragment |
| `networkpolicy-database` | NetworkPolicy enabled with no database destination, or with two | Locks the server away from its own database, and the symptom blames the database |

`deploy/helm/vpay/ci/guards/<guard>.yaml` is one values file per guard, each
violating exactly that guard. `just helm-check` renders each and fails unless
the render fails *with that guard's name in the message* — so a guard that
stops firing, or a message that stops naming itself, fails CI. Verified by
disabling a guard and watching the check fail (2026-09-03).

`values.schema.json` is separate and does a different job: it checks *shape*
(types, enums, unknown keys) before a template renders. Semantics live in the
guards, so the error can explain the consequence.

## The checkout page IS deployed by this chart, and the dashboard is not

The two look like the same decision and are not, which is worth stating
because `checkout.enabled` and `dashboard.enabled` sit next to each other in
`values.yaml` and behave in opposite ways.

`vpay-checkout` is templated when enabled, because the evidence exists:
`frontends/Dockerfile`'s `checkout` target declares `USER node`, the image has
been built from a clean context and run with `--read-only --tmpfs /tmp`, and
it answered `GET /healthz` 200 in that state. That is what the Deployment's
`runAsNonRoot` (with no invented UID — the image has an `/etc/passwd`),
`readOnlyRootFilesystem` and single `emptyDir` on `/tmp` are derived from, and
`/tmp` is a mount rather than an omission for exactly that reason.

**No pod has ever run.** The probe thresholds, the resource numbers and the
Ingress are reasoned from the image and from Kubernetes' documented behaviour,
like the rest of this chart. What is new is only that the *container* has been
observed running the way the chart asks it to.

Off by default, and that is a complete deployment rather than a missing one:
`checkout.public_base_url` is optional in vpay's own config, and without it
`POST /v1/checkout/sessions` answers `checkout_not_configured` rather than
minting a `url` that resolves to nothing.

The path-prefix Ingress shape (`checkout.ingress.path`) is templated and has
**not** been run by anyone. The app's routes are `/c/…`, `/e/…` and `/healthz`
at the root and it is not `basePath`-aware, so a prefix needs a controller
rewrite that this chart deliberately leaves to
`checkout.ingress.annotations` — the correct value depends on your controller.
Prefer `checkout.ingress.host`.

## The dashboard is not deployed by this chart

`ghcr.io/vaam-store/vpay-dashboard` is published by the release workflow.
Its Deployment is not written here, and `dashboard.enabled: true` is a named
template failure rather than a silent no-op.

The reason: `frontends/Dockerfile` produces a `node:22-alpine` image that
declares no `USER`, and Next's standalone server's filesystem behaviour under
`readOnlyRootFilesystem` has never been observed. Writing a plausible-looking
Deployment with an invented UID and a guessed set of `emptyDir` mounts is
exactly the kind of thing this repository's AGENTS.md forbids. Deploy it
separately until someone has actually run it.

## Values

Every key, with its default. `values.yaml` carries the same information plus
the reasoning; this table is maintained by hand and can drift from it.

### Naming

| Key | Default | Meaning |
|---|---|---|
| `nameOverride` | `""` | Overrides the chart name in generated names |
| `fullnameOverride` | `""` | Overrides the full resource name outright |
| `commonLabels` | `{}` | Labels added to every object |
| `commonAnnotations` | `{}` | Annotations added to every object |

### Images

| Key | Default | Meaning |
|---|---|---|
| `images.registry` | `ghcr.io` | Registry host |
| `images.namespace` | `vaam-store` | Registry namespace/owner |
| `images.pullPolicy` | `IfNotPresent` | |
| `images.pullSecrets` | `[]` | `imagePullSecrets` entries; empty is right for a public package |
| `images.server.name` | `vpay-server` | |
| `images.server.tag` | `""` | Empty means `.Chart.AppVersion` |
| `images.server.digest` | `""` | When set, wins over the tag: `repo@sha256:…` |
| `images.worker.name` | `vpay-worker` | |
| `images.worker.tag` | `""` | |
| `images.worker.digest` | `""` | |
| `images.checkout.name` | `vpay-checkout` | Only read when `checkout.enabled` |
| `images.checkout.tag` | `""` | |
| `images.checkout.digest` | `""` | |

### Workloads

| Key | Default | Meaning |
|---|---|---|
| `server.replicaCount` | `2` | |
| `server.port` | `8080` | `VPAY_BIND` |
| `server.resources` | `100m` / `128Mi` request, `256Mi` limit | **Placeholders, not measurements** — nothing has profiled either binary's RSS |
| `server.podAnnotations` | `{}` | |
| `server.nodeSelector` / `.tolerations` / `.affinity` | empty | Scheduling pass-throughs |
| `server.extraEnv` | `[]` | Extra core/v1 `EnvVar` objects |
| `worker.replicaCount` | `1` | >1 is safe: jobs are leased with `FOR UPDATE SKIP LOCKED` |
| `worker.concurrency` | `4` | `VPAY_WORKER_CONCURRENCY`; the binary refuses 0 |
| `worker.resources` | as server | Same caveat |
| `worker.podAnnotations` / `.nodeSelector` / `.tolerations` / `.affinity` / `.extraEnv` | empty | |
| `shutdownGraceSeconds` | `25` | `VPAY_SHUTDOWN_GRACE_SECONDS` |
| `terminationGracePeriodSeconds` | `35` | Must exceed the above by ≥ 5 (`grace-period` guard) |

No CPU limit is set, deliberately: throttling a process whose latency is
dominated by an outbound rail call buys nothing and hides everything. There is
no HPA either — nothing has measured what would drive one.

### Config

| Key | Default | Meaning |
|---|---|---|
| `config.profile` | `sandbox` | `VPAY_PROFILE`; selects a *file*, never a code path |
| `config.path` | `/config/application.yml` | The baked base config; changing this is almost certainly wrong |
| `config.createOverlayConfigMap` | `false` | Render the overlay ConfigMap |
| `config.overlay` | `""` | The overlay's YAML content |

### Secrets

| Key | Default | Meaning |
|---|---|---|
| `database.existingSecret` | `vpay-database` | |
| `database.existingSecretKey` | `url` | |
| `signingKey.existingSecret` | `vpay-oauth-signing-key` | Server only |
| `signingKey.key` | `oauth-signing-key.pem` | |
| `signingKey.mountPath` | `/secrets/oauth-signing-key.pem` | Becomes `VPAY_OAUTH_SIGNING_KEY_FILE` |
| `signingKey.defaultMode` | `0440` (288) | See above — **not** `0400` |
| `rails.existingSecret` | `vpay-rails` | Projected with `envFrom` onto both workloads |

### Observability

| Key | Default | Meaning |
|---|---|---|
| `observability.port` | `9090` | `--observability-bind`; bound by both binaries |
| `observability.livenessPath` | `/livez` | Static `ok`, no database |
| `observability.metricsPath` | `/metrics` | Prometheus text format; never scraped by anything |
| `observability.readinessPath` | `/healthz` | Exists today; a real `SELECT 1` |
| `metrics.serviceMonitor.enabled` | `false` | Needs the prometheus-operator CRDs |
| `metrics.serviceMonitor.interval` | `30s` | |
| `metrics.serviceMonitor.scrapeTimeout` | `10s` | |
| `metrics.serviceMonitor.labels` | `{}` | e.g. `release: kube-prometheus-stack` |
| `metrics.prometheusRule.enabled` | `false` | |
| `metrics.prometheusRule.labels` | `{}` | |
| `metrics.prometheusRule.providerErrorRatio` | `0.05` | **Proposed, not measured** |
| `metrics.prometheusRule.providerErrorWindow` | `15m` | **Proposed** |
| `metrics.prometheusRule.jobQueueBehindSeconds` | `300` | **Proposed** |
| `metrics.prometheusRule.alertEventWindow` | `5m` | **Proposed** |

### Network

| Key | Default | Meaning |
|---|---|---|
| `service.type` | `ClusterIP` | |
| `service.port` | `8080` | |
| `service.annotations` | `{}` | |
| `ingress.enabled` | `false` | |
| `ingress.className` | `nginx` | Step-6 decision (4) |
| `ingress.host` | `""` | Required when enabled |
| `ingress.annotations` | `{}` | Merged onto both Ingress objects |
| `ingress.tls.enabled` | `true` | |
| `ingress.tls.clusterIssuer` | `letsencrypt-prod` | cert-manager annotation |
| `ingress.tls.secretName` | `""` | Empty means `<fullname>-tls` |
| `ingress.api.path` / `.pathType` | `/v1` / `Prefix` | |
| `ingress.api.limitRps` / `.limitBurstMultiplier` | `20` / `3` | |
| `ingress.token.path` / `.pathType` | `/v1/oauth/token` / `Exact` | |
| `ingress.token.limitRps` / `.limitBurstMultiplier` | `5` / `2` | Must be ≤ the api limit |
| `networkPolicy.enabled` | `false` | Off until you say where Postgres is |
| `networkPolicy.ingressControllerNamespace` | `ingress-nginx` | |
| `networkPolicy.monitoringNamespace` | `monitoring` | The only source allowed to reach 9090 |
| `networkPolicy.dnsNamespace` | `kube-system` | |
| `networkPolicy.database.cidrs` | `[]` | A managed instance's address |
| `networkPolicy.database.namespace` / `.podSelector` | `""` / `{}` | An in-cluster one |
| `networkPolicy.database.port` | `5432` | |
| `networkPolicy.railsEgress.enabled` | `true` | Outbound HTTPS to the rails |
| `networkPolicy.railsEgress.port` | `443` | |
| `networkPolicy.railsEgress.except` | RFC1918 + `169.254.0.0/16` | Keeps the rule from reaching the VPC or the metadata endpoint |
| `podDisruptionBudget.enabled` | `true` | Server only |
| `podDisruptionBudget.minAvailable` | `1` | Integer, never a percentage |

### Misc

| Key | Default | Meaning |
|---|---|---|
| `serviceAccount.create` | `true` | |
| `serviceAccount.name` | `""` | Empty means the chart fullname |
| `serviceAccount.annotations` | `{}` | |
| `logFilter` | `info` | `RUST_LOG` |
| `logFormat` | `json` | `VPAY_LOG_FORMAT` — already the binary's default |
| `dashboard.enabled` | `false` | `true` is a named template failure |
| `checkout.enabled` | `false` | vpay's own payment page. Off is a complete deployment — see below |
| `checkout.replicaCount` | `2` | |
| `checkout.port` | `3000` | The Next.js standalone server's `PORT`; set as an env var so it cannot drift from the Service |
| `checkout.resources` | `100m` / `128Mi` request, `512Mi` limit | **Placeholders**, as everywhere else here. The limit exists because an unbounded heap on a GC'd process evicts a node rather than restarting a pod |
| `checkout.apiUrl` | `""` | This pod's view of vpay, for the server-side origins lookup. Empty renders this release's own server Service |
| `checkout.publicApiUrl` | `""` | **Required when enabled.** A payer's browser's view of vpay; the app throws on a missing one |
| `checkout.service.type` / `.port` | `ClusterIP` / `3000` | |
| `checkout.ingress.enabled` | `false` | |
| `checkout.ingress.host` | `""` | Its own hostname — prefer this shape |
| `checkout.ingress.path` | `""` | A prefix on `ingress.host`. Needs a `rewrite-target` annotation, and **nobody has run this shape** |
| `checkout.ingress.limitRps` | `50` | Looser than `/v1`'s on purpose: it is a page, not an authenticated write surface |
| `checkout.extraEnv` | `[]` | `PORT`, `HOSTNAME`, `VPAY_API_URL` and `NEXT_PUBLIC_VPAY_API_URL` are reserved (`extra-env-collision`) |

## Why two Ingress objects

ingress-nginx applies `limit-rps` per Ingress object, so one object cannot
carry a tighter limit for `/v1/oauth/token` than for the rest of `/v1`. A token
request costs an RSA verification and a database write
(`oauth_client_assertion_jtis`); it is the expensive unauthenticated surface
and deserves the tighter limit. The `rate-limit-ordering` guard keeps the
tighter one tighter.

nginx enforces the limit **per controller replica**, so the effective global
limit is roughly `limitRps × controller replicas`. That is an approximation,
and naming it is the point — ADR-0009 assumes a rate limit exists, and until
now nothing in this repository checked that one was configured at all. An exact
global limit needs Gateway API's `BackendTrafficPolicy` and a rate-limit
service, i.e. a second component to operate.

## Verifying the chart

```bash
just helm-check
```

which is exactly what CI's `deploy` job runs. It:

1. `helm lint`s the chart with the defaults and with `ci/values-full.yaml`;
2. `helm template`s both;
3. renders every file under `ci/guards/`, requiring each to **fail** with its
   own guard name in the message;
4. greps the rendered Ingress for `nginx.ingress.kubernetes.io/limit-rps` and
   checks the token limit is the tighter of the two;
5. runs `kubeconform -strict -summary` over both renders, with
   `-schema-location default` for built-in kinds and the
   [datreeio/CRDs-catalog](https://github.com/datreeio/CRDs-catalog) location
   for `ServiceMonitor` and `PrometheusRule`.

It is **not** part of `just ci`: kubeconform downloads its schemas, and
`just ci` is expected to work offline. Run it by hand when you touch the
chart; CI runs it on every pull request either way.

---

## Status

Written 2026-09-03, step 6 block B.

### What has actually been verified

* `helm lint` passes on the defaults and on `ci/values-full.yaml`.
* `helm template` renders 6 objects with the defaults and 14 with
  `ci/values-full.yaml`.
* All 15 guards fire on their own values file, each with its own name in the
  message, and `just helm-check` also checks that the fifteen names it expects
  are exactly the fifteen files on disk — so deleting a guard *and* its values
  file fails rather than passing quietly. Proven negatively too, which is the
  only thing that says these are checks rather than decoration: disabling the
  `grace-period` and `rate-limit-ordering` guards makes `just helm-check`
  fail, and so — verified in the Step 6 review pass, by neutering each `fail`
  in `templates/_validate.tpl` and re-running — does disabling either of the
  two guards that pass added, `rails-egress-except` and
  `extra-env-collision`. In each case the recipe reported that the guard
  "did not fire" and named it.
* `kubeconform -strict` validates 20 rendered resources across both files —
  17 built-in and 3 Prometheus CRDs — with 0 invalid and 0 skipped.
* Removing the `limit-rps` annotation from the Ingress template makes
  `just helm-check` fail.

### What has NOT been verified — most of it

* **No cluster has ever run this.** Not a real one, not kind, not minikube.
  Step-6 decision (9) put a kind smoke test out of scope for this step, so
  nothing here says anything about scheduling, admission, or whether these
  objects can coexist.
* ~~**The liveness probes point at a listener that does not exist.**~~
  **Corrected 2026-09-03, same day.** Block A landed `--observability-bind`,
  `/livez` and the worker's first HTTP listener; both binaries' own
  `tests/cli.rs` drive the running process and assert that `/livez` and
  `/metrics` answer on that port and 404 on the traffic port. Struck through
  rather than deleted because this chart was written against the earlier
  state and a reader comparing the two should see the gap close rather than
  wonder whether it was ever real. What remains true: an image older than
  that listener has nothing on port 9090, and the kubelet will restart both
  pods in a loop against one — pin `images.*.digest`.
* ~~**Every PrometheusRule query names a metric no build emits.**~~
  **Corrected 2026-09-03, same day:** block C landed the instrumentation, so
  `vpay_provider_requests_total`, `vpay_charge_transitions_total`,
  `vpay_jobs_*` and `vpay_alert_events_total` are all recorded and served on
  `--observability-bind`. **What has never happened is a scrape.** No
  Prometheus has polled a vpay process, so no rule here has ever been
  evaluated against a real series — it has never fired, never failed to fire,
  and never been tested against real data. `metrics.prometheusRule.enabled`
  and `metrics.serviceMonitor.enabled` are both `false` by default.
* **`VpayProviderErrorRateHigh` will fire on ordinary declines.** Its
  numerator is `error_kind!=""` — every failed port call, which is what makes
  it able to fire during a rail outage (`provider_unavailable`) at all — and
  that set includes `charge_declined`, a rail *decision* rather than a rail
  failure. Whether to exclude declines is a maintainer decision to make with
  the threshold itself; see `docs/runbooks/provider-error-rate.md`.
* **Every alert threshold is proposed, not derived.** Step-6 decision (5): the
  runbooks contained no numbers to transcribe. Each rule carries
  `provisional: "true"`.
* `readOnlyRootFilesystem: true` is "no observed writer", not "proven". The
  `scratch` image has no writable path and nothing in either binary opens a
  file for writing, but no pod has run to confirm it.
* `signingKey.defaultMode: 0440` is reasoned from Kubernetes' documented
  ownership rule for `fsGroup`ed Secret volumes, not observed.
* The NetworkPolicy has never been enforced by a CNI. A cluster whose CNI
  ignores NetworkPolicy and one that honours it look identical from here.
* The PodDisruptionBudget's behaviour during a rolling restart or a node drain
  is untested.
* **Nothing has verified that ingress-nginx honours `limit-rps` at all.** CI
  checks that the annotation is *present in the rendered YAML*. That is the
  whole claim.
* The resource requests and limits are placeholders. No profiling exists.
* The images the chart references have never been pulled from GHCR by this
  chart; publishing them is block A.

### Follow-ups

* A kind smoke test — it needs a real Postgres and the signing-key Secret,
  i.e. a second copy of the e2e job, for the ability to catch scheduling
  errors. Deferred by decision (9), worth doing.
* `helm unittest` for the object shapes, rather than kubeconform alone.
* A dashboard workload, once someone has run that image with a non-root UID.
* A cluster run of the checkout page's path-prefix Ingress shape, which is
  templated and unexercised.
* An HPA, once anything has measured what would drive it.
