# ADR-0022: Surface isolation and independent scaling

- **Status:** Proposed — needs maintainer acceptance, and carries three
  decisions this document deliberately does not make (§ "Left to the
  maintainer")
- **Date:** 2026-09-16
- **Deciders:** vpay maintainers
- **Extends:** [ADR-0008](0008-dashboard-scope.md) (the dashboard's write
  boundary), [ADR-0017](0017-staff-authentication.md) (staff authentication
  and the per-request staff-row re-read), [ADR-0018](0018-cross-tenant-admin-reads.md)
  (the cross-tenant admin role). None of those is revisited except where named.
- **Numbering:** `0020` is an unused gap and `0018` is used **twice**
  (`0018-cross-tenant-admin-reads.md`, `0018-privacy-controls-and-evidence.md`).
  This ADR takes `0022` to stay monotonic above `0021` rather than filling the
  gap, because `docs/plans/2026-09-13-flutter-plugin.md:419` refers to a future
  ADR as "ADR-0020 shaped" and reusing the number would make that sentence
  point at this one. The duplicate `0018` is a separate defect, not fixed here.

## Context

`vpay` now runs in production. **That sentence contradicts this repository in
at least three places**, and the contradiction is listed here rather than
quietly repaired, because retiring a load-bearing claim is the maintainer's
call and the repo's own convention is a dated addendum, never an edit:

- `docs/status.md`'s banner — "**⛔ no pod has ever run**" and "Do not deploy
  it";
- `deploy/helm/vpay/templates/deployment-checkout.yaml:26` — "**No pod has
  ever run**: the probes' thresholds, the resource numbers and the Ingress are
  reasoned";
- `backends/crates/vpay-db/src/pool.rs:16` — the pool ceiling is "headroom,
  not a measured need", justified by "there is no `/v1/*` route yet".

Everything below is reasoned from the code as it stands. **No claim in this
ADR about production behaviour is a measurement**; where a number matters, the
ADR says what would measure it.

The operational requirement is: **autoscale the business workload; hold the
admin workload at two replicas each, backend and frontend.**

### What is already true, and is not what this ADR changes

Three of the four things a "split the APIs" refactor usually has to build are
already built, and building them again as crates or processes would be a
rewrite of working structure:

1. **The surfaces are already separate routers.**
   [`vpay-api/src/lib.rs:1659`](../../backends/crates/vpay-api/src/lib.rs)
   nests `/v1` (merchant; `Vec<MerchantClient>`, several tenants), `/dash/v1`
   (staff; `Option<DashboardClient>`, exactly one), `/provider` and
   `/v1/browser` into one `Router`.
2. **`/dash/v1` is already read-only, structurally.**
   [`dash/mod.rs:67`](../../backends/crates/vpay-api/src/dash/mod.rs) mounts
   `GET` and nothing else, and `require_dashboard_token` refuses every other
   method **before the router matches**, rather than relying on no `post(..)`
   being present.
3. **The dashboard already reaches `/dash/v1` through a BFF.**
   `frontends/apps/dashboard/app/api/dash/*` are Next route handlers; the
   browser never holds a `/v1` credential.

And `/dash/v1` is _already_ conditional: `router` takes
`dashboard_validator: Option<DashboardJwtValidator>`, so a deployment with no
`dashboard_client` in its YAML serves no management surface at all
(`lib.rs:1668`).

### The two things that actually block the requirement

**`/v1` is mounted unconditionally.** `.nest("/v1", v1)` at `lib.rs:1661` has
no `Option` around it, so there is no way to run a process that serves the
management surface and not the merchant one. Every replica is every surface.

**The chart refuses to template a dashboard workload, on purpose.** Guard 12
in `templates/_validate.tpl:194` fails the release if `dashboard.enabled` is
true, and its message is specific: the image "is `node:22-alpine`-based,
declares no `USER`, and its behaviour under `readOnlyRootFilesystem` has never
been observed." Reading `frontends/Dockerfile`, that is exactly right — the
`runner` stage (lines 42–48) has no `USER` and no `HOME`/`XDG_CACHE_HOME`,
where the `checkout` stage (lines 66–113) has `USER node`, both env vars
pointed at `/tmp`, and a healthcheck. The chart is not missing a Deployment;
it is declining to write one for an image that is not ready.

### The constraint that decides the shape: Postgres connections, not CPU

`vpay_db::pool::MAX_CONNECTIONS` is **10**, and it is a **compile-time
constant, not a chart value** (`pool.rs:12`). Every server and worker process
holds a pool of up to ten Postgres connections. Postgres' own default
`max_connections` is 100, and `pool.rs:19` says the ceiling was chosen so "a
single vpay process should never be able to starve the rest of that budget".

That arithmetic was written for a fixed replica count. Under an HPA it becomes
the binding constraint:

```
(business_max_replicas + management_replicas + worker_replicas) × 10  ≤  max_connections − operator_headroom
```

With `management = 2` and `worker = 1`, a business tier that scales to 8 wants
110 connections against a default budget of 100 — and it will hit that
ceiling **before** it hits any CPU target. The failure mode is not a refused
scale-up: it is `PgPoolOptions::acquire_timeout` firing on whichever path
happens to ask next, which on the worker's side is the crash-recovery branch
(`pool.rs:40`). An autoscaler that can quietly convert load into database
connection exhaustion is the single most dangerous thing this ADR could ship
without naming.

## Decision

### 1. Surface selection is a deployment concern, expressed in config

Make `/v1` mounting conditional, mirroring the `Option` shape `/dash/v1`
already has. One image, one binary, one router function; which surfaces a
process serves is decided by its configuration at boot.

**Rejected: separate crates (`vpay-api-business`, `vpay-api-management`).**
The tenancy boundary is not enforced at the router — it is enforced by
`vpay-db` repository methods that take a `merchant_id` and have no unscoped
variant (`dash/mod.rs:60`). Splitting the routers into crates does not move
that boundary and does not make it stronger; it duplicates `AppState`,
`model`, `error` and `resource_auth` across a seam, and every shared type then
needs a third crate to live in. The isolation being asked for is a network
property, and a network property is bought with network objects.

**Rejected: separate binaries.** Same reasoning, plus a second artifact to
build, sign, scan and pin a digest for. The config toggle gets the same
topology with one image.

**Rejected: selecting the surface from the token.** For ADR-0018's reason,
which this ADR adopts unchanged: a property minted into a token stays true for
its whole TTL, and `/dash/v1` authorisation deliberately re-reads
`staff_members` on every request so that disabling an account takes effect at
once.

### 2. Two server Deployments from one image

| Workload               | Serves                                    | Replicas    | Autoscaled    |
| ---------------------- | ----------------------------------------- | ----------- | ------------- |
| `<release>-server`     | `/v1`, `/v1/browser`, `/provider`         | HPA         | **yes**       |
| `<release>-management` | `/dash/v1` and the `staff` sign-in routes | fixed **2** | **no**        |
| `<release>-worker`     | job loop                                  | fixed (1)   | no, unchanged |
| `<release>-dashboard`  | the admin frontend                        | fixed **2** | **no**        |

The management tier is **not** autoscaled, and that is a decision rather than
an omission. Its load is a small, known number of staff sessions; an HPA on it
would add replicas — each carrying a ten-connection pool — in response to
noise, competing for the same connection budget as the tier that actually
needs it.

`staff`'s eight unauthenticated sign-in routes move **with** `/dash/v1`, not
with `/v1`. They exist only to produce the `Identity` the dashboard's
authorization-code grant consumes (`dash/mod.rs:10`), and leaving them on the
internet-facing tier would put the credential-stuffing surface on the
autoscaled workload while the thing it authenticates against sits elsewhere.

### 3. A chart guard bounding replicas against the connection budget

A new guard — `connection-budget` — fails the release when

```
(server.autoscaling.maxReplicas + management.replicaCount + worker.replicaCount) × 10
  > database.maxConnections − database.reservedConnections
```

with `database.maxConnections` a required value once autoscaling is enabled
(no default — guessing 100 on behalf of a managed instance is how this becomes
a 3am incident) and `reservedConnections` defaulting to a non-zero operator
headroom.

`10` is `MAX_CONNECTIONS` and is **duplicated** into the chart by this guard.
That duplication is a defect and is accepted knowingly: the constant lives in
a Rust binary the chart cannot read. It must be named in
`values.schema.json`'s description and in `pool.rs`'s doc comment as a pair
that has to move together, in the same way `worker.concurrency` and the pool
ceiling are already paired by the `worker-concurrency-pool` guard
(`_validate.tpl:435`).

**This guard is the load-bearing part of this ADR.** Without it, §2 is a
change that works in staging and exhausts a connection budget in production.

### 4. The dashboard Deployment, and the guard it retires

Template `<release>-dashboard` at a fixed 2 replicas, modelled on
`deployment-checkout.yaml` — which is the precedent for a `node:22-alpine`
Next standalone server in this chart, including `runAsNonRoot` with no
explicit `runAsUser`, `readOnlyRootFilesystem: true`, and a memory-backed
`emptyDir` on `/tmp`.

This is blocked until `frontends/Dockerfile`'s `runner` stage is brought to
parity with its `checkout` stage: `USER node`, `HOME` and `XDG_CACHE_HOME`
at `/tmp`, and a healthcheck. **Guard 12 must not be retired before that
lands**, and the commit that retires it is the commit that proves the image
starts read-only — not a deployment that discovers it.

`dashboard.publicOrigin` becomes **required** when `dashboard.enabled` is
true. `values.yaml:112` already anticipates this in writing: "Making it
REQUIRED is the follow-up, and it lands with the Deployment this chart does
not yet write."

### 5. Routing and network policy

A fourth HTTPRoute rule for `/dash/v1` → the management Service. The chart
already does per-path routing with separate rules for token, api and provider
(`templates/httproute.yaml:70`, `:85`, `:105`), so this is a rule, not a
mechanism.

The management tier should not be reachable from the public gateway at all
where the operator can arrange it — a separate internal Gateway, or a
NetworkPolicy ingress rule admitting only the dashboard Deployment. Path
routing alone leaves both tiers on one hostname, and a routing mistake then
reaches a live backend rather than nothing.

Each tier gets its own PodDisruptionBudget. The existing `pdb-minavailable`
guard (`_validate.tpl:106`) compares `minAvailable` against
`server.replicaCount`; with an HPA that value is `autoscaling.minReplicas`,
and the guard must be taught the difference or it compares against a field
that no longer governs.

## Consequences

### What is already safe to autoscale, and why

**The staff rate limiter is already shared across replicas.** It was a
`Mutex<HashMap<String, Window>>` until 2026-09-10 and is now a Postgres
counter (`staff/rate_limit.rs:1`, migration `0038`), specifically because
ADR-0017 had recorded that per-replica limiting "is the first thing to revisit
if a deployment runs many replicas." That revisit already happened; ten
attempts means ten attempts at any replica count.

**Idempotency is database-backed**, not in-process, so a retry landing on a
different replica behaves identically.

**The signing key is a mounted Secret** shared by every replica, so a token
minted by one verifies at another.

### What changes under autoscaling

**The JWKS cache is per-replica** (`jwks_cache.rs:33`, an
`RwLock<Option<(Jwks, Instant)>>`). More replicas means proportionally more
JWKS fetches against the issuer, and a cold replica's first request pays a
fetch. This is a cache, not correctness — but an HPA that churns replicas
converts it into steady load on the issuer.

**Scale-down interacts with the 25s drain deadline.**
`terminationGracePeriodSeconds` is 35s against the process's own 25s drain
(`deployment-server.yaml:46`). An HPA with an aggressive scale-down policy
will evict mid-request far more often than a fixed replica count ever did.
A `behavior.scaleDown.stabilizationWindowSeconds` well above the drain
deadline is required, not optional.

**Migrations run at boot, before the listener binds**
(`deployment-server.yaml:104`). Today that is a property of a fixed set of
replicas rolling. Under an HPA, a scale-up event during a deploy can start a
pod carrying a _different_ image version than the one that migrated. This ADR
does not solve that; it records it as the reason migrations must stay
backward-compatible with the previous image, and as an argument for moving
them out of pod boot in a later ADR.

### Documentation and status

`docs/status.md`'s banner, `deployment-checkout.yaml:26` and `pool.rs:16` each
carry a claim this ADR's premise contradicts. Per the repo's convention each
gets a **dated addendum retiring a specific sentence**, not an edit — and per
`CLAUDE.md`, the evidence goes on the area page (`docs/status/infrastructure.md`)
with a dated page under `docs/status/verification/`.

`deploy/helm/vpay/README.md`'s object table and guard table both need the new
workloads and the new guard. `docs/flows/dashboard.md`'s **Status** section is
a summary plus an index — evidence goes on the page it points at.

Per the maintainer's standing parity rule, the companion `vaam-apps/vpay-skills`
PR should be opened alongside this one, since the deployment topology a skill
describes is changing. `node tools/verify-coverage.mjs <path-to-vpay>` proves
only that every feature page is claimed and no skill cites a dead path; it
cannot read prose, so it will not catch a skill that still describes one
Deployment.

## Left to the maintainer

Three decisions are deliberately not made here.

1. **Connection pooler, or a smaller pool?** The `connection-budget` guard
   bounds the problem but does not solve it: a business tier that needs to
   scale past the budget needs either PgBouncer in transaction mode (which
   interacts with `sqlx`'s prepared statements and with `SET`-based session
   state, and would need its own verification) or a smaller per-process pool
   (which `pool.rs` says to change only "once real concurrent load is
   measured"). **Production is where that measurement now exists** — it should
   be taken before either path is chosen.
2. **The HPA metric and bounds.** CPU is the default and is a poor proxy for a
   workload that spends its time awaiting a rail. In-flight requests or RPS
   via `ServiceMonitor` would track the real thing. `minReplicas` and
   `maxReplicas` should come from observed production numbers, not from this
   document.
3. **Whether the management tier faces the public gateway at all.** §5
   recommends an internal-only path; that depends on how staff reach the
   cluster, which this repository does not know.

## Verification this ADR requires before it is Accepted

- `just helm-check` green, with new `ci/guards/` cases for `connection-budget`
  (both directions) and for the retired guard 12.
- A rendered default `helm template` containing **no** `-management` or
  `-dashboard` objects while both are disabled — the same two-halved assertion
  `checkout-not-templated-by-default` already uses (`_validate.tpl:309`).
- The dashboard image observed starting under `readOnlyRootFilesystem: true`,
  as a recorded run and not a reasoned claim. This is the one item on the list
  that no existing gate can stand in for.
- `just ci` green, including `just test-doc`.
