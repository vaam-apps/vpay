# vpay Helm chart

Renders the two shipping vpay workloads — the API server and the job worker —
and the Kubernetes objects around them.

Both run **one image**, `ghcr.io/<ns>/vpay-server`. The worker Deployment
passes `args: ["worker"]` to it. There were two images and two `images.*`
value blocks until 2026-09-07 (issue #77); `images.worker` no longer exists,
and a values file that still sets it is refused by `values.schema.json`
rather than silently ignored.

**Read [Status](#status) before you install this.** Nothing in this chart has
ever been applied to a cluster, and no Prometheus has ever scraped a vpay
process. The listener its liveness probes point at does now exist — see
Status for what changed and what still has no evidence behind it.

---

## What it renders

| Object                | Name                                   | Notes                                                                                                                               |
| --------------------- | -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `Deployment`          | `<release>-server`                     | `server.replicaCount` (2 by default)                                                                                                |
| `Deployment`          | `<release>-worker`                     | `worker.replicaCount` (1); `strategy: Recreate`; the server image with `args: ["worker"]`                                           |
| `Deployment`          | `<release>-checkout`                   | Optional (`checkout.enabled`, **false** by default) — vpay's own payment page                                                       |
| `Service`             | `<release>`                            | ClusterIP, ports `http` (8080) and `metrics` (9090)                                                                                 |
| `Service`             | `<release>-worker`                     | Headless, `metrics` only — exists so the worker can be scraped                                                                      |
| `Service`             | `<release>-checkout`                   | Optional; `http` only — the page emits no metrics                                                                                   |
| `ServiceAccount`      | `<release>`                            | `automountServiceAccountToken: false`                                                                                               |
| `PodDisruptionBudget` | `<release>-server`                     | `minAvailable: 1`; server only                                                                                                      |
| `ConfigMap`           | `<release>-config-overlay`             | Optional; the profile overlay, mounted with `subPath`                                                                               |
| `Ingress`             | `<release>-api`                        | `/v1`, ingress-nginx annotations incl. `limit-rps`                                                                                  |
| `Ingress`             | `<release>-token`                      | `/v1/oauth/token`, a tighter `limit-rps`                                                                                            |
| `Ingress`             | `<release>-provider`                   | `/provider`, the rail callback. **On by default** — its own object so it can carry its own `limit-rps` and `whitelist-source-range` |
| `Ingress`             | `<release>-checkout`                   | Optional; the payment page, on its own host or a path prefix                                                                        |
| `HTTPRoute`           | `<release>`                            | Optional (`route.enabled`); **one** object, three rules — `/v1`, `/v1/oauth/token` and `/provider`                                  |
| `HTTPRoute`           | `<release>-checkout`                   | Optional; the payment page, on its own hostname or a rewritten path prefix                                                          |
| `NetworkPolicy`       | `<release>-server`, `<release>-worker` | Optional, default-deny both directions                                                                                              |
| `ServiceMonitor`      | `<release>-server`, `<release>-worker` | Optional; needs the prometheus-operator CRDs                                                                                        |
| `PrometheusRule`      | `<release>`                            | Optional; **every threshold is proposed, every metric unemitted**                                                                   |

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
  # One image for both backend workloads — pin it by digest for a real
  # deployment. There is no `images.worker` (issue #77, 2026-09-07).
  server: { digest: "sha256:<64 hex>" }

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
ConfigMap _at_ `/config` replaces that directory, the baked
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

| Value                                            | Default name                                       | Shape                                                                                                           | Consequence if wrong            |
| ------------------------------------------------ | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| `database.existingSecret` / `.existingSecretKey` | `vpay-database` / `url`                            | one key holding a full `postgres://` URL                                                                        | both Deployments fail to start  |
| `signingKey.existingSecret` / `.key`             | `vpay-oauth-signing-key` / `oauth-signing-key.pem` | PEM RSA private key (PKCS#8 or PKCS#1)                                                                          | the server Deployment exits 78  |
| `rails.existingSecret`                           | `vpay-rails`                                       | one key per `${VAR}` in the deployed image's `config/application.yml` — read it at upgrade time, the list grows | exit 78 on **both** Deployments |

The signing key is mounted on the **server Deployment only**. The worker
issues no token and reads no key, and mounting the Secret there would widen
its blast radius for no capability.

It is worth being exact about what changed on 2026-09-07 (issue #77). Two
images ago, `vpay-worker-bin` did not accept `--oauth-signing-key-file` at
all — the flag did not exist on that binary. Now one binary serves both
roles, so the flag exists, and **both spellings of "hand the worker the key"
are still refused**: `vpay-server worker --oauth-signing-key-file …` by clap
(the flag is not on the subcommand), and `vpay-server
--oauth-signing-key-file … worker` by `vpay_config::cli`'s `SERVE_ONLY_FLAGS`
check, which clap's derive cannot express. The second spelling parsed and was
read by nothing for the length of one review pass, and was closed in it.

That is defence in depth and not the guarantee. `VPAY_OAUTH_SIGNING_KEY_FILE`
in the _environment_ is still ignored rather than refused, deliberately — a
shared env block must not `CrashLoopBackOff` a worker — so a flag naming a
path is not what keeps the key away. **The volume list in
`deployment-worker.yaml` is**, and this chart sets no
`VPAY_OAUTH_SIGNING_KEY_FILE` on the worker either.

The rail Secret is projected with `envFrom.secretRef`, so `kubectl describe pod`
shows the variable _names_ and never the values.

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
_not_ templated here. If you want it, install the operator and a `Cluster`
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
names itself so a test can assert _which_ one fired:

```
Error: execution error at (vpay/templates/deployment-server.yaml:1:4):
vpay chart guard "grace-period": terminationGracePeriodSeconds is 25 but
shutdownGraceSeconds is 25; the kubelet would SIGKILL the process while it is
still draining in-flight work. Set terminationGracePeriodSeconds to at least 30.
```

| Guard                               | Fires when                                                                                                                                                                      | Why it matters                                                                                                                                                                                                                                                                                                                                 |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `grace-period`                      | `terminationGracePeriodSeconds < shutdownGraceSeconds + 5`                                                                                                                      | The kubelet kills the process mid-drain; every rolling update truncates in-flight work                                                                                                                                                                                                                                                         |
| `database-secret`                   | either `database.existingSecret` / `.existingSecretKey` is empty                                                                                                                | `DATABASE_URL` has no other source and the chart creates no Secret                                                                                                                                                                                                                                                                             |
| `signing-key-secret`                | either `signingKey.existingSecret` / `.key` is empty                                                                                                                            | `vpay-server` exits 78; a chart-generated key would mint tokens other replicas cannot verify                                                                                                                                                                                                                                                   |
| `rails-secret`                      | `rails.existingSecret` is empty                                                                                                                                                 | An unresolved `${VAR}` is exit 78 on **both** binaries. The guard checks a Secret is named, never which keys are in it — that list is the image's, not the chart's                                                                                                                                                                             |
| `image-digest-format`               | a digest is set and is not `sha256:` + 64 hex                                                                                                                                   | A truncated digest fails at image pull, in the cluster, not here                                                                                                                                                                                                                                                                               |
| `worker-replicas`                   | `worker.replicaCount < 1`                                                                                                                                                       | No job is claimed; intents sit in `processing` while everything reports healthy                                                                                                                                                                                                                                                                |
| `pdb-minavailable`                  | `podDisruptionBudget.minAvailable >= server.replicaCount`                                                                                                                       | No voluntary eviction is ever allowed, so node drains hang for ever                                                                                                                                                                                                                                                                            |
| `observability-port`                | `observability.port` equals `server.port` or `service.port`                                                                                                                     | Publishes `/metrics` on the Ingress-facing port                                                                                                                                                                                                                                                                                                |
| `rate-limit-ordering`               | token or provider `limitRps` > api `limitRps`, or any is ≤ 0                                                                                                                    | Inverts the whole reason there are separate Ingress objects; nginx treats ≤ 0 as no limit at all. `/provider` joined it 2026-09-11 — it is the **unauthenticated** surface and the one with no in-process limit                                                                                                                                |
| `ingress-host`                      | ingress enabled with an empty host, or TLS enabled with neither issuer nor secret                                                                                               | A host-less rule answers for other applications' hostnames; a TLS block nothing populates serves the controller's default certificate                                                                                                                                                                                                          |
| `route-attachment`                  | route enabled with no `parentRefs`, a `parentRef` with no `name`, or no hostname after the `ingress.host` fallback                                                              | An HTTPRoute with no parent is accepted, listed by `kubectl`, and routes nothing; a hostname-less one answers for other applications' hostnames                                                                                                                                                                                                |
| `route-rate-limit`                  | route enabled, no `ExtensionRef` filter on the token rule, and no `route.rateLimitedBy`                                                                                         | Gateway API has no portable rate limit, so ADR-0009's assumption would be dropped in silence — see [Gateway API](#gateway-api)                                                                                                                                                                                                                 |
| `provider-callback-routable`        | routing enabled with the `/provider` rule off and no `servedElsewhere`; that sentence contradicted by `config.overlay`; or the rule pointed anywhere but `/provider` or `/`     | Every MTN MoMo and Orange Money callback gets the controller's 404 — vpay never receives it, logs nothing, and settlement silently degrades to the poll ladder — see [The rail callback](#the-rail-callback)                                                                                                                                   |
| `overlay-empty`                     | overlay ConfigMap requested with empty content, or an empty profile                                                                                                             | The process treats an empty overlay as success and runs on baked sandbox placeholders                                                                                                                                                                                                                                                          |
| `dashboard-not-templated`           | `dashboard.enabled: true`                                                                                                                                                       | This chart templates no dashboard workload — see below                                                                                                                                                                                                                                                                                         |
| `dashboard-public-origin`           | `dashboard.publicOrigin` set to something that is not `scheme://host[:port]` — a bare hostname, a path, a trailing slash                                                        | It is compared against the `Origin` header a browser sends, byte for byte after normalisation; anything else never matches, and every server action on the dashboard is refused                                                                                                                                                                |
| `checkout-not-templated-by-default` | `checkout.ingress.enabled` or `checkout.route.enabled` with `checkout.enabled: false`                                                                                           | Routing to a Service the chart did not template: a 503 on the payment page, found by a payer                                                                                                                                                                                                                                                   |
| `checkout-templated-when-enabled`   | enabled with no `publicApiUrl`; or an Ingress/HTTPRoute with neither host nor `path`, or with both; or TLS with nothing to populate the Secret; or a route with no `parentRefs` | The app throws on a missing `NEXT_PUBLIC_VPAY_API_URL`, so the pod starts and never passes readiness; a host-less rule answers for other applications; a payer's session credential rides in that URL's fragment                                                                                                                               |
| `networkpolicy-database`            | NetworkPolicy enabled with no database destination, or with two                                                                                                                 | Locks the server away from its own database, and the symptom blames the database                                                                                                                                                                                                                                                               |
| `worker-concurrency-pool`           | `worker.concurrency` above 5                                                                                                                                                    | `vpay-server worker` refuses it at boot (exit 78, issue #63): its pool holds 10 connections and one webhook fan-out can hold two. Without the guard the release installs and CrashLoopBackOffs — including on a `helm upgrade` of a working one. The 5 is a literal here; the pool size is a constant in the image, and the chart exposes none |
| `rails-egress-except`               | `networkPolicy.egress.rails` names a CIDR the `except` list does not fit inside                                                                                                 | _This row and the one below were missing from this table until 2026-09-10; both guards have existed and fired since 2026-09-03_                                                                                                                                                                                                                |
| `extra-env-collision`               | `server.extraEnv` / `worker.extraEnv` / `checkout.extraEnv` sets a name the chart already sets                                                                                  | Kubernetes keeps the last entry with a given name, so the chart's own value is silently replaced                                                                                                                                                                                                                                               |

`deploy/helm/vpay/ci/guards/<guard>.yaml` is one values file per guard, each
violating exactly that guard. `just helm-check` renders each and fails unless
the render fails _with that guard's name in the message_ — so a guard that
stops firing, or a message that stops naming itself, fails CI. Verified by
disabling a guard and watching the check fail (2026-09-03).

`values.schema.json` is separate and does a different job: it checks _shape_
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
like the rest of this chart. What is new is only that the _container_ has been
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

`ghcr.io/vaam-apps/vpay-dashboard` is published by the release workflow.
Its Deployment is not written here, and `dashboard.enabled: true` is a named
template failure rather than a silent no-op.

The reason: `frontends/Dockerfile` produces a `node:22-alpine` image that
declares no `USER`, and Next's standalone server's filesystem behaviour under
`readOnlyRootFilesystem` has never been observed. Writing a plausible-looking
Deployment with an invented UID and a guessed set of `emptyDir` mounts is
exactly the kind of thing this repository's AGENTS.md forbids. Deploy it
separately until someone has actually run it.

### `dashboard.publicOrigin`, which this chart also does not read

Added 2026-09-10 (issue #88 item 4). The dashboard refuses any server action
whose `Origin` is not `VPAY_DASHBOARD_PUBLIC_ORIGIN`; Next's own check
compares `Origin` against `X-Forwarded-Host`, which is two caller-supplied
values agreeing whenever the caller wants them to. Unset, the app falls back
to comparing the `Host` header — never `X-Forwarded-Host`, so still stronger
than Next's, and **wrong behind a proxy that rewrites `Host`**, where every
action is refused.

The key is here and no template consumes it, for the same reason
`dashboard.enabled` is here: **the thing an operator has to get right should
be named where they will look**, not discovered from a sign-in that refuses
itself. What the chart _can_ do about a value it does not read is check its
shape, and it does — see the `dashboard-public-origin` guard.

Making it **required** is the follow-up, and it belongs with the Deployment
this chart does not yet write: a required value on a workload that does not
exist would fail a deployment for a setting nothing reads.

## Values

Every key, with its default. `values.yaml` carries the same information plus
the reasoning; this table is maintained by hand and can drift from it.

### Naming

| Key                 | Default | Meaning                                     |
| ------------------- | ------- | ------------------------------------------- |
| `nameOverride`      | `""`    | Overrides the chart name in generated names |
| `fullnameOverride`  | `""`    | Overrides the full resource name outright   |
| `commonLabels`      | `{}`    | Labels added to every object                |
| `commonAnnotations` | `{}`    | Annotations added to every object           |

### Images

| Key                      | Default         | Meaning                                                                           |
| ------------------------ | --------------- | --------------------------------------------------------------------------------- |
| `images.registry`        | `ghcr.io`       | Registry host                                                                     |
| `images.namespace`       | `vaam-apps`     | Registry namespace/owner                                                          |
| `images.pullPolicy`      | `IfNotPresent`  |                                                                                   |
| `images.pullSecrets`     | `[]`            | `imagePullSecrets` entries; empty is right for a public package                   |
| `images.server.name`     | `vpay-server`   |                                                                                   |
| `images.server.tag`      | `""`            | Empty means `.Chart.AppVersion`                                                   |
| `images.server.digest`   | `""`            | When set, wins over the tag: `repo@sha256:…`. **Both** backend Deployments use it |
| `images.checkout.name`   | `vpay-checkout` | Only read when `checkout.enabled`                                                 |
| `images.checkout.tag`    | `""`            |                                                                                   |
| `images.checkout.digest` | `""`            |                                                                                   |

### Workloads

| Key                                                                                    | Default                                 | Meaning                                                                                                                                                                                                                                        |
| -------------------------------------------------------------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `server.replicaCount`                                                                  | `2`                                     |                                                                                                                                                                                                                                                |
| `server.port`                                                                          | `8080`                                  | `VPAY_BIND`                                                                                                                                                                                                                                    |
| `server.resources`                                                                     | `100m` / `128Mi` request, `256Mi` limit | **Placeholders, not measurements** — nothing has profiled either binary's RSS                                                                                                                                                                  |
| `server.podAnnotations`                                                                | `{}`                                    |                                                                                                                                                                                                                                                |
| `server.nodeSelector` / `.tolerations` / `.affinity`                                   | empty                                   | Scheduling pass-throughs                                                                                                                                                                                                                       |
| `server.extraEnv`                                                                      | `[]`                                    | Extra core/v1 `EnvVar` objects                                                                                                                                                                                                                 |
| `worker.replicaCount`                                                                  | `1`                                     | >1 is safe: jobs are leased with `FOR UPDATE SKIP LOCKED`                                                                                                                                                                                      |
| `worker.concurrency`                                                                   | `4`                                     | `VPAY_WORKER_CONCURRENCY`; `vpay-server worker` refuses 0, and refuses anything above **5** — `vpay_db::MAX_CONNECTIONS / 2` (issue #63, `worker-concurrency-pool` guard). More throughput is more `worker.replicaCount`, not more concurrency |
| `worker.resources`                                                                     | as server                               | Same caveat                                                                                                                                                                                                                                    |
| `worker.podAnnotations` / `.nodeSelector` / `.tolerations` / `.affinity` / `.extraEnv` | empty                                   |                                                                                                                                                                                                                                                |
| `shutdownGraceSeconds`                                                                 | `25`                                    | `VPAY_SHUTDOWN_GRACE_SECONDS`                                                                                                                                                                                                                  |
| `terminationGracePeriodSeconds`                                                        | `35`                                    | Must exceed the above by ≥ 5 (`grace-period` guard)                                                                                                                                                                                            |

No CPU limit is set, deliberately: throttling a process whose latency is
dominated by an outbound rail call buys nothing and hides everything. There is
no HPA either — nothing has measured what would drive one.

### Config

| Key                             | Default                   | Meaning                                                        |
| ------------------------------- | ------------------------- | -------------------------------------------------------------- |
| `config.profile`                | `sandbox`                 | `VPAY_PROFILE`; selects a _file_, never a code path            |
| `config.path`                   | `/config/application.yml` | The baked base config; changing this is almost certainly wrong |
| `config.createOverlayConfigMap` | `false`                   | Render the overlay ConfigMap                                   |
| `config.overlay`                | `""`                      | The overlay's YAML content                                     |

### Secrets

| Key                          | Default                          | Meaning                                      |
| ---------------------------- | -------------------------------- | -------------------------------------------- |
| `database.existingSecret`    | `vpay-database`                  |                                              |
| `database.existingSecretKey` | `url`                            |                                              |
| `signingKey.existingSecret`  | `vpay-oauth-signing-key`         | Server only                                  |
| `signingKey.key`             | `oauth-signing-key.pem`          |                                              |
| `signingKey.mountPath`       | `/secrets/oauth-signing-key.pem` | Becomes `VPAY_OAUTH_SIGNING_KEY_FILE`        |
| `signingKey.defaultMode`     | `0440` (288)                     | See above — **not** `0400`                   |
| `rails.existingSecret`       | `vpay-rails`                     | Projected with `envFrom` onto both workloads |

### Observability

| Key                                            | Default    | Meaning                                           |
| ---------------------------------------------- | ---------- | ------------------------------------------------- |
| `observability.port`                           | `9090`     | `--observability-bind`; bound by both Deployments |
| `observability.livenessPath`                   | `/livez`   | Static `ok`, no database                          |
| `observability.metricsPath`                    | `/metrics` | Prometheus text format; never scraped by anything |
| `observability.readinessPath`                  | `/healthz` | Exists today; a real `SELECT 1`                   |
| `metrics.serviceMonitor.enabled`               | `false`    | Needs the prometheus-operator CRDs                |
| `metrics.serviceMonitor.interval`              | `30s`      |                                                   |
| `metrics.serviceMonitor.scrapeTimeout`         | `10s`      |                                                   |
| `metrics.serviceMonitor.labels`                | `{}`       | e.g. `release: kube-prometheus-stack`             |
| `metrics.prometheusRule.enabled`               | `false`    |                                                   |
| `metrics.prometheusRule.labels`                | `{}`       |                                                   |
| `metrics.prometheusRule.providerErrorRatio`    | `0.05`     | **Proposed, not measured**                        |
| `metrics.prometheusRule.providerErrorWindow`   | `15m`      | **Proposed**                                      |
| `metrics.prometheusRule.jobQueueBehindSeconds` | `300`      | **Proposed**                                      |
| `metrics.prometheusRule.alertEventWindow`      | `5m`       | **Proposed**                                      |

### Network

| Key                                                   | Default                     | Meaning                                                                                                            |
| ----------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `service.type`                                        | `ClusterIP`                 |                                                                                                                    |
| `service.port`                                        | `8080`                      |                                                                                                                    |
| `service.annotations`                                 | `{}`                        |                                                                                                                    |
| `ingress.enabled`                                     | `false`                     |                                                                                                                    |
| `ingress.className`                                   | `nginx`                     | Step-6 decision (4)                                                                                                |
| `ingress.host`                                        | `""`                        | Required when enabled                                                                                              |
| `ingress.annotations`                                 | `{}`                        | Merged onto both Ingress objects                                                                                   |
| `ingress.tls.enabled`                                 | `true`                      |                                                                                                                    |
| `ingress.tls.clusterIssuer`                           | `letsencrypt-prod`          | cert-manager annotation                                                                                            |
| `ingress.tls.secretName`                              | `""`                        | Empty means `<fullname>-tls`                                                                                       |
| `ingress.api.path` / `.pathType`                      | `/v1` / `Prefix`            |                                                                                                                    |
| `ingress.api.limitRps` / `.limitBurstMultiplier`      | `20` / `3`                  |                                                                                                                    |
| `ingress.token.path` / `.pathType`                    | `/v1/oauth/token` / `Exact` |                                                                                                                    |
| `ingress.token.limitRps` / `.limitBurstMultiplier`    | `5` / `2`                   | Must be ≤ the api limit                                                                                            |
| `ingress.provider.enabled`                            | `true`                      | The rail callback's own Ingress. **On by default** — see below                                                     |
| `ingress.provider.path` / `.pathType`                 | `/provider` / `Prefix`      | `/provider` is a literal owned by `vpay-api`; the guard refuses anything but it or `/`                             |
| `ingress.provider.limitRps` / `.limitBurstMultiplier` | `10` / `5`                  | Must be ≤ the api limit. Tighter on purpose: unauthenticated, and unlimited inside the process                     |
| `ingress.provider.sourceRange`                        | `""`                        | `whitelist-source-range`, rendered only when set                                                                   |
| `ingress.provider.servedElsewhere`                    | `""`                        | Required to set `enabled: false`                                                                                   |
| `route.enabled`                                       | `false`                     | Gateway API instead of Ingress. Renders nothing without the Gateway API CRDs                                       |
| `route.parentRefs`                                    | `[]`                        | Verbatim `ParentReference` list. Required when enabled — there is no `className` equivalent                        |
| `route.hostnames`                                     | `[]`                        | Empty falls back to `[ingress.host]`                                                                               |
| `route.annotations` / `.labels`                       | `{}`                        | Merged onto the rendered HTTPRoute                                                                                 |
| `route.rateLimitedBy`                                 | `""`                        | Required unless the token rule carries an `ExtensionRef` filter. Rendered as the `vpay/rate-limited-by` annotation |
| `route.api.path` / `.pathType`                        | `/v1` / `PathPrefix`        | `PathPrefix`, not the Ingress spelling `Prefix`                                                                    |
| `route.api.filters`                                   | `[]`                        | Verbatim `HTTPRouteFilter` list                                                                                    |
| `route.token.path` / `.pathType`                      | `/v1/oauth/token` / `Exact` |                                                                                                                    |
| `route.token.filters`                                 | `[]`                        | Where the rate-limit `ExtensionRef` goes                                                                           |
| `route.provider.enabled`                              | `true`                      | The rail callback, as a third rule on the same HTTPRoute                                                           |
| `route.provider.path` / `.pathType`                   | `/provider` / `PathPrefix`  |                                                                                                                    |
| `route.provider.filters`                              | `[]`                        | Where a rate limit or source-IP `ExtensionRef` goes                                                                |
| `route.provider.servedElsewhere`                      | `""`                        | Required to set `enabled: false`                                                                                   |
| `networkPolicy.enabled`                               | `false`                     | Off until you say where Postgres is                                                                                |
| `networkPolicy.ingressControllerNamespace`            | `ingress-nginx`             |                                                                                                                    |
| `networkPolicy.monitoringNamespace`                   | `monitoring`                | The only source allowed to reach 9090                                                                              |
| `networkPolicy.dnsNamespace`                          | `kube-system`               |                                                                                                                    |
| `networkPolicy.database.cidrs`                        | `[]`                        | A managed instance's address                                                                                       |
| `networkPolicy.database.namespace` / `.podSelector`   | `""` / `{}`                 | An in-cluster one                                                                                                  |
| `networkPolicy.database.port`                         | `5432`                      |                                                                                                                    |
| `networkPolicy.railsEgress.enabled`                   | `true`                      | Outbound HTTPS to the rails                                                                                        |
| `networkPolicy.railsEgress.port`                      | `443`                       |                                                                                                                    |
| `networkPolicy.railsEgress.except`                    | RFC1918 + `169.254.0.0/16`  | Keeps the rule from reaching the VPC or the metadata endpoint                                                      |
| `podDisruptionBudget.enabled`                         | `true`                      | Server only                                                                                                        |
| `podDisruptionBudget.minAvailable`                    | `1`                         | Integer, never a percentage                                                                                        |

### Misc

| Key                               | Default                                 | Meaning                                                                                                                                                                   |
| --------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `serviceAccount.create`           | `true`                                  |                                                                                                                                                                           |
| `serviceAccount.name`             | `""`                                    | Empty means the chart fullname                                                                                                                                            |
| `serviceAccount.annotations`      | `{}`                                    |                                                                                                                                                                           |
| `logFilter`                       | `info`                                  | `RUST_LOG`                                                                                                                                                                |
| `logFormat`                       | `json`                                  | `VPAY_LOG_FORMAT` — already the binary's default                                                                                                                          |
| `dashboard.enabled`               | `false`                                 | `true` is a named template failure                                                                                                                                        |
| `dashboard.publicOrigin`          | `""`                                    | `VPAY_DASHBOARD_PUBLIC_ORIGIN` on the dashboard you deploy separately. **Nothing in this chart reads it** — see below. Empty means the app falls back to comparing `Host` |
| `checkout.enabled`                | `false`                                 | vpay's own payment page. Off is a complete deployment — see below                                                                                                         |
| `checkout.replicaCount`           | `2`                                     |                                                                                                                                                                           |
| `checkout.port`                   | `3000`                                  | The Next.js standalone server's `PORT`; set as an env var so it cannot drift from the Service                                                                             |
| `checkout.resources`              | `100m` / `128Mi` request, `512Mi` limit | **Placeholders**, as everywhere else here. The limit exists because an unbounded heap on a GC'd process evicts a node rather than restarting a pod                        |
| `checkout.apiUrl`                 | `""`                                    | This pod's view of vpay, for the server-side origins lookup. Empty renders this release's own server Service                                                              |
| `checkout.publicApiUrl`           | `""`                                    | **Required when enabled.** A payer's browser's view of vpay; the app throws on a missing one                                                                              |
| `checkout.service.type` / `.port` | `ClusterIP` / `3000`                    |                                                                                                                                                                           |
| `checkout.ingress.enabled`        | `false`                                 |                                                                                                                                                                           |
| `checkout.ingress.host`           | `""`                                    | Its own hostname — prefer this shape                                                                                                                                      |
| `checkout.ingress.path`           | `""`                                    | A prefix on `ingress.host`. Needs a `rewrite-target` annotation, and **nobody has run this shape**                                                                        |
| `checkout.ingress.limitRps`       | `50`                                    | Looser than `/v1`'s on purpose: it is a page, not an authenticated write surface                                                                                          |
| `checkout.route.enabled`          | `false`                                 | The page's HTTPRoute                                                                                                                                                      |
| `checkout.route.parentRefs`       | `[]`                                    | Empty means `route.parentRefs`                                                                                                                                            |
| `checkout.route.hostnames`        | `[]`                                    | Its own hostname — prefer this shape                                                                                                                                      |
| `checkout.route.path`             | `""`                                    | A prefix on `route.hostnames`; the chart renders the `URLRewrite` that strips it. **Nobody has run this shape**                                                           |
| `checkout.route.pathType`         | `PathPrefix`                            |                                                                                                                                                                           |
| `checkout.route.filters`          | `[]`                                    | Appended after the `URLRewrite`, never before it                                                                                                                          |
| `checkout.extraEnv`               | `[]`                                    | `PORT`, `HOSTNAME`, `VPAY_API_URL` and `NEXT_PUBLIC_VPAY_API_URL` are reserved (`extra-env-collision`)                                                                    |

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
service, i.e. a second component to operate — and the chart can now render the
Gateway API routing that policy would attach to, though not the policy itself.
See [Gateway API](#gateway-api).

## The rail callback

**`POST /provider/{code}/callback` is not under `/v1`, and until 2026-09-11 no
shape of this chart routed it.** That is a defect in the Ingress path this
chart has shipped since 2026-09-03, not something the Gateway API work
introduced; it is fixed on both mechanisms here because fixing one and not the
other would have left the new path carrying a bug it could have been born
without.

Three facts, none of which lives in the same crate as the others:

|                                                                                        | Where                                                                                               |
| -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| The route is mounted at the **root**, beside `/v1` and not inside it                   | `vpay-api/src/provider_callback.rs` — `PROVIDER_NEST = "/provider"`, nested in `lib.rs`'s router    |
| The URL each rail is handed is `{deployment.public_base_url}/provider/{code}/callback` | `vpay_config::ProviderHost::effective_callback_url`, unless `providers[].callback_url` overrides it |
| This chart's `ingress.api.path` / `route.api.path` are `/v1`                           | `values.yaml`                                                                                       |

Nothing compiles those against each other. Together they mean a deployment
that enabled this chart's routing and nothing else answered **every MTN MoMo
and Orange Money callback with the ingress controller's 404** — vpay never
received the request, wrote no log line, and settlement degraded to the poll
ladder while every object reported healthy. `docs/reference/rails.md` already
records that the callback **path** is the half that drifts silently, "because
the route lives in `vpay-api` and the derivation in `vpay-config`, and neither
crate compiles against the other".

So `ingress.provider` and `route.provider` are **on by default**: a rail
callback is not optional for any deployment that takes money. On the Ingress
side it is a **fourth object**, not a third path on the API's, for the same
reason the token endpoint is a second — ingress-nginx applies both `limit-rps`
and `whitelist-source-range` per Ingress object, and this surface wants its own
value for each. On the Gateway API side it is a third **rule**, because filters
there are per-rule.

**Its rate limit is tighter than `/v1`'s (10 vs 20), and the
`rate-limit-ordering` guard keeps it no looser.** `/provider` takes no bearer
token and `/v1` does; more to the point, `provider_callback.rs`'s own header
says of the route that "nothing else here is a rate limit: **there is none**".
A caller who knows a live charge's v4 `provider_reference_id` can hold that
charge at roughly one authenticated rail request per worker claim, because the
pull-forward floor is ten seconds while the poll ladder's rungs grow. The edge
limit is the only one that exists for this surface, which is not true of `/v1`.
The number itself is proposed, like every number in this chart. The body cap is
`16k`, matching the handler's own `CALLBACK_BODY_LIMIT_BYTES` rather than
`/v1`'s 64k.

**Turning it off is defensible and has to be said out loud.** MTN additionally
allows an IP-allowlisted callback host of its own
(`docs/reference/rails.md`), reached through `providers[].callback_url`, so an
operator may legitimately terminate callbacks somewhere this chart does not
render. The `provider-callback-routable` guard therefore makes it **opt-out,
not opt-in**: `enabled: false` needs one line in `servedElsewhere` naming what
serves the prefix instead — the same mechanism, and the same reasoning, as
`route.rateLimitedBy`.

That guard has one arm the others do not, and it is the only place in this
chart that reads `config.overlay` as anything but an opaque string: when the
overlay is present and parses, it lists `providers[]`, so a `servedElsewhere`
sentence while an **enabled** provider carries no `callback_url` override is a
claim contradicted by the configuration this same release mounts. Absent,
unparseable, or listing no providers, that arm says nothing and the
requirement for a sentence still stands. A third arm refuses a `path` that is
neither `/provider` nor `/`: the literal belongs to `vpay-api`, and this chart
does not get to rename it.

## Gateway API

`route.enabled: true` renders `HTTPRoute`s instead of `Ingress`es, for a
cluster that runs a Gateway (Traefik, Envoy Gateway, Istio, Cilium) rather than
ingress-nginx. It was added 2026-09-11 because until then this chart could
express routing in exactly one way: a Gateway API cluster had to hand-write its
own HTTPRoutes outside the chart and keep their paths in step with
`ingress.api.path` / `ingress.token.path` by hand — a duplicated list of things
that move together, which is the defect this repository's own conventions name.

It is an **alternative** to `ingress:`, not a migration of it. Enabling one does
not disable the other; a cluster that turns on both gets two controllers
answering for the same hostname, and that is an operator's decision rather than
the chart's. Both templates are guarded by
`.Capabilities.APIVersions.Has "gateway.networking.k8s.io/v1"`, so a cluster
without the Gateway API CRDs renders nothing rather than failing at install.

```yaml
ingress:
  enabled: false
route:
  enabled: true
  parentRefs:
    - name: traefik-gateway
      namespace: traefik
      sectionName: websecure
  hostnames: [api.vpay.example]
  token:
    filters:
      - type: ExtensionRef
        extensionRef: { group: traefik.io, kind: Middleware, name: vpay-token-ratelimit }
```

**One HTTPRoute, two rules — where the Ingress path needs two objects.** The
split exists because ingress-nginx applies `limit-rps` per Ingress _object_.
Gateway API attaches filters per _rule_, so one object carries both, and
precedence between them is the specification's rather than a controller's: an
`Exact` match is ordered ahead of a `PathPrefix` one, so `/v1/oauth/token` takes
the token rule and the rest of `/v1` takes the api rule. The
`rate-limit-ordering` guard has no counterpart here — there are no two numbers
to invert.

**Four things the Ingress path does that this one does not**, because Gateway
API has no portable spelling for any of them:

|                  | Ingress                                                  | Gateway API                                                                                                                                                                          |
| ---------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| rate limit       | `nginx.ingress.kubernetes.io/limit-rps`                  | nothing portable — see below                                                                                                                                                         |
| request body cap | `proxy-body-size: 64k`                                   | a listener/implementation setting on the Gateway                                                                                                                                     |
| HTTP → HTTPS     | `ssl-redirect`                                           | a second HTTPRoute on the `:80` listener with a `RequestRedirect` filter — **not rendered**, because the chart would have to guess the listener and a wrong guess is a redirect loop |
| TLS              | the Ingress names its Secret and its cert-manager issuer | the **listener's** `certificateRefs`. `ingress.tls` has no equivalent here and is not consulted                                                                                      |

**The rate limit is the one the chart refuses to drop in silence.** ADR-0009
assumes a limit in front of `/v1/oauth/token`. Gateway API has no core filter
and no standard-channel policy for one: every implementation does it with its
own object, reached through an `ExtensionRef` filter (Traefik: a `traefik.io`
`Middleware`; Envoy Gateway: a `BackendTrafficPolicy`). A chart that guessed at
one would be wrong on every other controller, and a chart that quietly emitted
none would drop the assumption with no symptom but an unmetered token endpoint.

So the `route-rate-limit` guard insists on one of two things, and fails the
render without either:

1. **an `ExtensionRef` filter on `route.token.filters`** — your controller's own
   rate-limit object, whatever it is. Preferred.
2. **`route.rateLimitedBy`**, one line naming what enforces it somewhere this
   chart cannot see (a CDN, a WAF, a policy attached to the Gateway listener).
   It is rendered onto the HTTPRoute as the `vpay/rate-limited-by` annotation,
   so the claim lives in the cluster and `just helm-check` can grep for it the
   same way it greps the Ingress for `limit-rps`.

Free text, deliberately, and not a boolean: a boolean is a box to tick, and
"ADR-0009's assumption still holds" should not be assertable by typing `true`.
The chart cannot check that either spelling is _true_. What it refuses is
silence.

**The checkout page's path-prefix shape is the one place Gateway API is
strictly better here.** The app is not `basePath`-aware, so a prefix has to be
rewritten away; the Ingress shape leaves that to a controller-specific
`rewrite-target` annotation the operator supplies, while `checkout.route.path`
renders a `URLRewrite` filter with `ReplacePrefixMatch: /`, which is in the
specification. It is still _extended_ support rather than core — an
implementation may decline it — and, like the Ingress shape it improves on,
**nobody has run it**.

## Verifying the chart

```bash
just helm-check
```

which is exactly what CI's `deploy` job runs. It:

1. `helm lint`s the chart with the defaults, with `ci/values-full.yaml` and
   with `ci/values-route.yaml`;
2. `helm template`s all three — the route one needs `--api-versions`, see (5);
3. renders every file under `ci/guards/`, requiring each to **fail** with its
   own guard name in the message;
4. greps the rendered Ingress for `nginx.ingress.kubernetes.io/limit-rps` and
   checks the token limit is the tighter of the two, then renders the rail
   callback's own Ingress and checks it carries a `/provider` path and a
   `limit-rps` no looser than `/v1`'s;
5. renders `ci/values-route.yaml` with
   `--api-versions gateway.networking.k8s.io/v1` and checks the rendered
   HTTPRoute either carries an `ExtensionRef` filter on the token rule or
   declares `vpay/rate-limited-by` and carries a `/provider` rule, then renders
   the same file **without** the flag and checks no `HTTPRoute` appears at all
   — the `.Capabilities` gate, asserted rather than assumed;
6. runs `kubeconform -strict -summary` over all three renders, with
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

- `helm lint` passes on the defaults and on `ci/values-full.yaml`.
- `helm template` renders 6 objects with the defaults and 14 with
  `ci/values-full.yaml`.
- All **22** guards fire on their own values file, each with its own name in
  the message, and `just helm-check` also checks that the twenty-two names it
  expects are exactly the twenty-two files on disk — so deleting a guard _and_
  its values file fails rather than passing quietly. (**This said "15" until
  2026-09-10** and had been wrong since the sixteenth landed; `worker-concurrency-pool`
  made it nineteen, `route-attachment` + `route-rate-limit` made it twenty-one,
  and `provider-callback-routable` makes it twenty-two. Measured:
  `22 guards, all fired by name (22 expected)`.) Proven negatively too, which is the
  only thing that says these are checks rather than decoration: disabling the
  `grace-period` and `rate-limit-ordering` guards makes `just helm-check`
  fail, and so — verified in the Step 6 review pass, by neutering each `fail`
  in `templates/_validate.tpl` and re-running — does disabling either of the
  two guards that pass added, `rails-egress-except` and
  `extra-env-collision`. In each case the recipe reported that the guard
  "did not fire" and named it.
- `kubeconform -strict` validates 20 rendered resources across both files —
  17 built-in and 3 Prometheus CRDs — with 0 invalid and 0 skipped.
- Removing the `limit-rps` annotation from the Ingress template makes
  `just helm-check` fail.

**Added 2026-09-11, the Gateway API path** (`route:` and `checkout.route:`):

- `helm lint` passes on `ci/values-route.yaml` too, and `just helm-check` now
  lints and renders three value sets rather than two.
- The two new guards, `route-attachment` and `route-rate-limit`, fire on their
  own values files by name, and — with `provider-callback-routable`, below —
  the expected set in `just helm-check` is twenty-two rather than nineteen.
- Proven negatively, the same way the earlier guards were: neutering the
  `route-rate-limit` `fail` in `templates/_validate.tpl` makes `just
helm-check` report that the guard "did not fire" and name it. See the
  "httproute rate limit" step for the other half — it asserts the _rendered_
  YAML carries either the `ExtensionRef` filter or the annotation, so a
  template that stopped emitting one would fail even with the guard intact.
- `kubeconform -strict` validates the rendered `HTTPRoute`s against the
  upstream `gateway.networking.k8s.io/v1` schema from the CRDs catalog —
  35 resources across the three renders, 0 invalid, **0 skipped** (a skipped
  one would mean the schema was never found and nothing was checked).
- The `.Capabilities.APIVersions.Has` gate is asserted, not assumed: the same
  values file rendered without `--api-versions gateway.networking.k8s.io/v1`
  produces no `HTTPRoute` and no error.
- **The Ingress path gains exactly one object and changes nothing else.** It is
  NOT byte-identical, and an earlier draft of this section claimed it was —
  that claim was true of the Gateway API work alone and stopped being true when
  the rail callback fix landed in the same change. Measured against the parent
  commit (`6b1b7d8`), rendered from a throwaway worktree:
  - `helm template` with the **defaults** (`ingress.enabled: false`) is
    byte-identical — the new object is behind the same switch as the old ones.
  - `helm template --set ingress.enabled=true --set ingress.host=…` differs by
    **one added document and nothing else**: the `<release>-provider` Ingress.
    Stripping that document from the new render makes the two files `diff`
    clean, so `<release>-api` and `<release>-token` are byte-for-byte what they
    were.
  - `ci/values-full.yaml` differs the same way: one added `Ingress`,
    `vpay-provider`, and no other line.

  **A `helm upgrade` of an existing release therefore creates one new Ingress
  object it did not have.** That is the fix, not a side effect — before it, the
  release was dropping every rail callback — but it is a change to a shipped
  path and an operator who has a hand-written `/provider` Ingress of their own
  should set `ingress.provider.enabled: false` with a `servedElsewhere`
  sentence before upgrading, or the two objects will both claim the prefix.

### What has NOT been verified — most of it

- **No cluster has ever run this.** Not a real one, not kind, not minikube.
  Step-6 decision (9) put a kind smoke test out of scope for this step, so
  nothing here says anything about scheduling, admission, or whether these
  objects can coexist.
- ~~**The liveness probes point at a listener that does not exist.**~~
  **Corrected 2026-09-03, same day.** Block A landed `--observability-bind`,
  `/livez` and the worker's first HTTP listener; both Deployments' own
  `tests/cli.rs` drive the running process and assert that `/livez` and
  `/metrics` answer on that port and 404 on the traffic port. Struck through
  rather than deleted because this chart was written against the earlier
  state and a reader comparing the two should see the gap close rather than
  wonder whether it was ever real. What remains true: an image older than
  that listener has nothing on port 9090, and the kubelet will restart both
  pods in a loop against one — pin `images.*.digest`.
- ~~**Every PrometheusRule query names a metric no build emits.**~~
  **Corrected 2026-09-03, same day:** block C landed the instrumentation, so
  `vpay_provider_requests_total`, `vpay_charge_transitions_total`,
  `vpay_jobs_*` and `vpay_alert_events_total` are all recorded and served on
  `--observability-bind`. **What has never happened is a scrape.** No
  Prometheus has polled a vpay process, so no rule here has ever been
  evaluated against a real series — it has never fired, never failed to fire,
  and never been tested against real data. `metrics.prometheusRule.enabled`
  and `metrics.serviceMonitor.enabled` are both `false` by default.
- **`VpayProviderErrorRateHigh` will fire on ordinary declines.** Its
  numerator is `error_kind!=""` — every failed port call, which is what makes
  it able to fire during a rail outage (`provider_unavailable`) at all — and
  that set includes `charge_declined`, a rail _decision_ rather than a rail
  failure. Whether to exclude declines is a maintainer decision to make with
  the threshold itself; see `docs/runbooks/provider-error-rate.md`.
- **Every alert threshold is proposed, not derived.** Step-6 decision (5): the
  runbooks contained no numbers to transcribe. Each rule carries
  `provisional: "true"`.
- `readOnlyRootFilesystem: true` is "no observed writer", not "proven". The
  `scratch` image has no writable path and nothing in either binary opens a
  file for writing, but no pod has run to confirm it.
- `signingKey.defaultMode: 0440` is reasoned from Kubernetes' documented
  ownership rule for `fsGroup`ed Secret volumes, not observed.
- The NetworkPolicy has never been enforced by a CNI. A cluster whose CNI
  ignores NetworkPolicy and one that honours it look identical from here.
- The PodDisruptionBudget's behaviour during a rolling restart or a node drain
  is untested.
- **Nothing has verified that ingress-nginx honours `limit-rps` at all.** CI
  checks that the annotation is _present in the rendered YAML_. That is the
  whole claim.
- No Gateway has ever accepted one of these HTTPRoutes.** Not a Traefik one,
  not an Envoy Gateway one, not any. Nothing has checked that a `parentRef`
  resolves, that a Gateway's `allowedRoutes` admits a route from this
  namespace, that the `Exact`-before-`PathPrefix` precedence the two rules rely
  on behaves as specified in a real implementation, or that the `URLRewrite`
  filter on the checkout page's path shape is supported by anything. The claim
  is the same one the Ingress path makes: it renders, and it validates against
  the published schema.
- `route.rateLimitedBy` is an assertion by a human, checked by nobody.** The
  `route-rate-limit` guard refuses an empty one; it cannot tell a true sentence
  from a false one, and neither can CI. The `ExtensionRef` spelling is no
  better off — the chart renders the reference and never looks for the object
  it names, so a typo'd `Middleware` name is an unmetered token endpoint that
  renders, validates and reports healthy.
- The resource requests and limits are placeholders. No profiling exists.
- The images the chart references have never been pulled from GHCR by this
- The resource requests and limits are placeholders. No profiling exists.
- The images the chart references have never been pulled from GHCR by this
  chart; ~~publishing them is block A.~~ **Updated 2026-09-05: publishing has
  happened** — release run `33929374661` (2026-09-04) pushed and signed all
  four. The unproven half is the pull, not the push: nobody has pulled one,
  and GHCR package visibility could not be measured (no `read:packages` scope;
  anonymous pull refused).

### Follow-ups

- A kind smoke test — it needs a real Postgres and the signing-key Secret,
  i.e. a second copy of the e2e job, for the ability to catch scheduling
  errors. Deferred by decision (9), worth doing.
- `helm unittest` for the object shapes, rather than kubeconform alone.
- A dashboard workload, once someone has run that image with a non-root UID.
- A cluster run of the checkout page's path-prefix Ingress shape, which is
  templated and unexercised. Its Gateway API twin — `checkout.route.path` and
  the `URLRewrite` filter — is in exactly the same position.
- A cluster run of the Gateway API path at all, on any implementation, and a
  rendered example of the rate-limit object `route.token.filters` is meant to
  reference. The chart names `traefik.io`/`Middleware` in a comment and
  templates nothing; whether it should is an open question, not an omission.
- An HPA, once anything has measured what would drive it.
