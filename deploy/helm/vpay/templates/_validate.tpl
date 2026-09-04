{{/*
Template-time guards.

Every guard below is a combination of values that is well-typed (so
`values.schema.json` lets it through) and still cannot work. Each calls Helm's
`fail`, which aborts `helm lint`, `helm template`, `helm install` and
`helm upgrade` alike — so CI's `helm template` step is the gate, and an
operator hits the same error locally before anything reaches a cluster.

Each message starts with `vpay chart guard "<name>":` so a test can assert
*which* guard fired rather than merely that something did. `just helm-check`
renders one values file per guard under `ci/guards/` and asserts exactly that.

`vpay.validate` is included from the top of every template in this chart, so
it runs whatever subset of objects a given values file produces.
*/}}
{{- define "vpay.validate" -}}

{{/* ---------------------------------------------------------------- 1 */}}
{{/*
grace-period — the kubelet must not SIGKILL a process that is still draining.
Both binaries treat `shutdownGraceSeconds` as a deadline they exit non-zero
*at*; a terminationGracePeriodSeconds at or below it guarantees the kill lands
first and turns every rolling update into a truncated drain.
*/}}
{{- $grace := int .Values.terminationGracePeriodSeconds -}}
{{- $drain := int .Values.shutdownGraceSeconds -}}
{{- if lt $grace (add $drain 5) -}}
{{- fail (printf "vpay chart guard \"grace-period\": terminationGracePeriodSeconds is %d but shutdownGraceSeconds is %d; the kubelet would SIGKILL the process while it is still draining in-flight work. Set terminationGracePeriodSeconds to at least %d." $grace $drain (add $drain 5)) -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 2 */}}
{{/*
database-secret — DATABASE_URL comes from an existing Secret and from nowhere
else (step-6 decision (9): no in-cluster Postgres, and this chart creates no
Secret). Both binaries exit non-zero without a database URL.
*/}}
{{- if or (empty .Values.database.existingSecret) (empty .Values.database.existingSecretKey) -}}
{{- fail "vpay chart guard \"database-secret\": database.existingSecret and database.existingSecretKey must both be set. This chart creates no Secret and templates no Postgres; DATABASE_URL has no other source, and both binaries refuse to start without it." -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 3 */}}
{{/*
signing-key-secret — `vpay-server` exits 78 without the RS256 key the merchant
OP signs `/v1` access tokens with. There is no fallback and no generated key:
a generated one would mint tokens no other replica could verify.
*/}}
{{- if or (empty .Values.signingKey.existingSecret) (empty .Values.signingKey.key) -}}
{{- fail "vpay chart guard \"signing-key-secret\": signingKey.existingSecret and signingKey.key must both be set. vpay-server exits 78 without VPAY_OAUTH_SIGNING_KEY_FILE, and this chart deliberately generates no key." -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 4 */}}
{{/*
rails-secret — the image's baked `config/application.yml` carries a `${VAR}`
placeholder per rail credential, and an unresolved one is a fatal exit 78 on
BOTH binaries, never an empty string. The Secret is projected with `envFrom`,
so the chart cannot check the names; it can insist there is a Secret to
project.

The list of names is NOT owned by this chart. It is whatever
`config/application.yml` references in **the image you are deploying** —
`grep -o '\${[A-Z_]*}' config/application.yml` on that revision — and it grows
as features land. The seven below are the list as of 2026-09-03, now that
Step 5 (webhooks) has landed `MERCHANT_WEBHOOK_SECRET` on `master`, and they
are an example, not a contract.
*/}}
{{- if empty .Values.rails.existingSecret -}}
{{- fail "vpay chart guard \"rails-secret\": rails.existingSecret must name a Secret carrying every credential the image's baked config/application.yml references as ${VAR} — one key per placeholder, and an unresolved one is exit 78 on both vpay-server and vpay-worker. The chart does not own that list and cannot check it: read it off the revision you are deploying with `grep -o '${[A-Z_]*}' config/application.yml`. On this branch, 2026-09-03, it is MERCHANT_WEBHOOK_SECRET, MTN_API_KEY, MTN_API_USER, MTN_SUBSCRIPTION_KEY, ORANGE_CLIENT_ID, ORANGE_CLIENT_SECRET and ORANGE_MERCHANT_KEY; a later image needs more." -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 5 */}}
{{/*
image-digest-format — a digest that is not a full `sha256:` + 64 hex is not a
pull that fails at `helm install`; it is a pod that will not schedule, found
later, in a cluster.
*/}}
{{- range $component := list "server" "worker" "checkout" -}}
{{- $digest := (index $.Values.images $component).digest -}}
{{- if $digest -}}
{{- if not (regexMatch "^sha256:[0-9a-f]{64}$" $digest) -}}
{{- fail (printf "vpay chart guard \"image-digest-format\": images.%s.digest is %q, which is not a full digest. It must match sha256: followed by exactly 64 lowercase hex characters; a truncated one fails at image pull, in the cluster, not here." $component $digest) -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 6 */}}
{{/*
worker-replicas — zero workers is not "scaled to zero" for this system: no job
is claimed, every confirmed intent sits in `processing` for ever, and both the
Deployment and every readiness signal stay green while it happens. The binary
itself refuses `--worker-concurrency 0` for the same reason.
*/}}
{{- if lt (int .Values.worker.replicaCount) 1 -}}
{{- fail (printf "vpay chart guard \"worker-replicas\": worker.replicaCount is %d. With no worker, no job is ever claimed: confirmed payment intents stay in `processing` indefinitely while every probe and every Deployment reports healthy. If you mean to stop the queue, say so somewhere a human will see it, not here." (int .Values.worker.replicaCount)) -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 7 */}}
{{/*
pdb-minavailable — a PDB whose minAvailable is at least the replica count
permits no voluntary eviction at all, so every `kubectl drain` and every
node-pool upgrade blocks for ever. The failure looks like a stuck cluster, not
like a misconfigured chart.
*/}}
{{- if .Values.podDisruptionBudget.enabled -}}
{{- $min := int .Values.podDisruptionBudget.minAvailable -}}
{{- $replicas := int .Values.server.replicaCount -}}
{{- if ge $min $replicas -}}
{{- fail (printf "vpay chart guard \"pdb-minavailable\": podDisruptionBudget.minAvailable is %d and server.replicaCount is %d. A budget that requires every replica to stay up blocks every voluntary eviction, so node drains hang instead of the workload being protected. Keep minAvailable strictly below the replica count, or disable the budget." $min $replicas) -}}
{{- end -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 8 */}}
{{/*
observability-port — `/metrics` and `/livez` must never be served on the port
the Ingress routes to. Collapsing the two ports would publish the metrics
endpoint on the public API surface.
*/}}
{{- if eq (int .Values.observability.port) (int .Values.server.port) -}}
{{- fail (printf "vpay chart guard \"observability-port\": observability.port and server.port are both %d. /metrics and /livez are served on the observability listener precisely so they are NOT reachable through the Ingress; sharing the port publishes them." (int .Values.observability.port)) -}}
{{- end -}}
{{- if eq (int .Values.observability.port) (int .Values.service.port) -}}
{{- fail (printf "vpay chart guard \"observability-port\": observability.port and service.port are both %d; the Service cannot expose two ports with the same number, and the metrics port must not be the one the Ingress targets." (int .Values.observability.port)) -}}
{{- end -}}

{{/* ---------------------------------------------------------------- 9 */}}
{{/*
rate-limit-ordering — the whole reason there are two Ingress objects is that
the token endpoint gets a *tighter* limit than the rest of /v1. A looser one
inverts the intent silently: nothing else in this repository checks it
(docs/roadmap.md's open item), and ingress-nginx will happily serve it.
*/}}
{{- if .Values.ingress.enabled -}}
{{- $api := int .Values.ingress.api.limitRps -}}
{{- $token := int .Values.ingress.token.limitRps -}}
{{- if gt $token $api -}}
{{- fail (printf "vpay chart guard \"rate-limit-ordering\": ingress.token.limitRps is %d but ingress.api.limitRps is %d. The token endpoint exists as a separate Ingress so it can be limited more tightly than /v1; a looser limit there inverts that and nothing downstream would notice." $token $api) -}}
{{- end -}}
{{- if or (le $api 0) (le $token 0) -}}
{{- fail (printf "vpay chart guard \"rate-limit-ordering\": ingress.api.limitRps=%d and ingress.token.limitRps=%d; ingress-nginx treats a non-positive limit-rps as absent, which would render an Ingress that claims a rate limit and applies none." $api $token) -}}
{{- end -}}
{{- end -}}

{{/* --------------------------------------------------------------- 10 */}}
{{/*
ingress-host — an Ingress with no host matches every request that reaches the
controller, including traffic meant for another application in the cluster.
*/}}
{{- if and .Values.ingress.enabled (empty .Values.ingress.host) -}}
{{- fail "vpay chart guard \"ingress-host\": ingress.enabled is true but ingress.host is empty. A host-less rule matches every request the controller receives, so vpay would answer for hostnames that belong to something else." -}}
{{- end -}}
{{- if and .Values.ingress.enabled .Values.ingress.tls.enabled (and (empty .Values.ingress.tls.clusterIssuer) (empty .Values.ingress.tls.secretName)) -}}
{{- fail "vpay chart guard \"ingress-host\": ingress.tls.enabled is true but neither ingress.tls.clusterIssuer nor ingress.tls.secretName is set, so nothing would ever populate the TLS Secret and the listener would serve the controller's default certificate." -}}
{{- end -}}

{{/* --------------------------------------------------------------- 11 */}}
{{/*
overlay-empty — mounting an empty overlay is worse than mounting none: the
process reads a zero-byte YAML file, merges nothing, and runs on the baked
sandbox placeholders (a merchant registry whose only key is a placeholder
modulus) with no diagnostic at all.
*/}}
{{- if .Values.config.createOverlayConfigMap -}}
{{- if empty (trim .Values.config.overlay) -}}
{{- fail "vpay chart guard \"overlay-empty\": config.createOverlayConfigMap is true but config.overlay is empty. The process treats a missing or empty overlay as success and boots on the image's baked sandbox configuration — placeholder merchant keys and WireMock rail hosts — without saying so." -}}
{{- end -}}
{{- end -}}
{{- if empty .Values.config.profile -}}
{{- fail "vpay chart guard \"overlay-empty\": config.profile is empty. VPAY_PROFILE names the overlay file the process looks for beside its baked base config; an empty one names nothing." -}}
{{- end -}}

{{/* --------------------------------------------------------------- 12 */}}
{{/*
dashboard-not-templated — see values.yaml. The image is published; the
workload is not written. This is the chart's `NotImplemented`.
*/}}
{{- if .Values.dashboard.enabled -}}
{{- fail "vpay chart guard \"dashboard-not-templated\": dashboard.enabled is true, but this chart templates no dashboard workload. ghcr.io/vaam-store/vpay-dashboard is published by the release workflow; its Deployment is not written, because the image is node:22-alpine-based, declares no USER, and its behaviour under readOnlyRootFilesystem has never been observed. Deploy it separately, or leave this false — do not expect a silent no-op." -}}
{{- end -}}

{{/* --------------------------------------------------------------- 13 */}}
{{/*
networkpolicy-database — a default-deny egress policy with no rule for
Postgres locks the server away from its own database, and the symptom is a
CrashLoopBackOff whose logs blame the database.
*/}}
{{- if .Values.networkPolicy.enabled -}}
{{- $db := .Values.networkPolicy.database -}}
{{- if and (empty $db.cidrs) (empty $db.namespace) -}}
{{- fail "vpay chart guard \"networkpolicy-database\": networkPolicy.enabled is true but networkPolicy.database names no destination. Set networkPolicy.database.cidrs (a managed instance) or networkPolicy.database.namespace (an in-cluster one); the rendered policy denies all other egress, so without this the server cannot reach Postgres at all." -}}
{{- end -}}
{{- if and (not (empty $db.cidrs)) (not (empty $db.namespace)) -}}
{{- fail "vpay chart guard \"networkpolicy-database\": networkPolicy.database sets both cidrs and namespace. Pick one — two egress rules for the same database widen the policy in a way nobody reading it would expect." -}}
{{- end -}}
{{- end -}}

{{/* --------------------------------------------------------------- 14 */}}
{{/*
rails-egress-except — the rails egress rule is `0.0.0.0/0` with an `except`
list, and the `except` list is the entire security content of it. Empty, the
rule is "this pod may open a TCP connection to anything on the internet, to
every other workload in the VPC, and to the cloud metadata service" — which
is a default-allow egress policy wearing a default-deny one's name, and it
renders and validates exactly like the intended one.

`169.254.0.0/16` specifically, and not merely "non-empty": 169.254.169.254 is
the IMDS endpoint on AWS, GCP and Azure alike, and an SSRF in a rail adapter
that can reach it is credential theft rather than a wasted request. It is the
one entry whose absence has a consequence nothing else in this chart catches.
*/}}
{{- if and .Values.networkPolicy.enabled .Values.networkPolicy.railsEgress.enabled -}}
{{- $except := .Values.networkPolicy.railsEgress.except -}}
{{- if empty $except -}}
{{- fail "vpay chart guard \"rails-egress-except\": networkPolicy.railsEgress is enabled but networkPolicy.railsEgress.except is empty. The rule is 0.0.0.0/0 minus that list, so an empty list renders a policy that permits egress to every private range and to the cloud metadata service — a default-allow egress rule that looks exactly like the intended default-deny one. Set at least the RFC1918 ranges and 169.254.0.0/16." -}}
{{- end -}}
{{- if not (has "169.254.0.0/16" $except) -}}
{{- fail (printf "vpay chart guard \"rails-egress-except\": networkPolicy.railsEgress.except is %v, which does not contain 169.254.0.0/16. That block holds the instance metadata endpoint (169.254.169.254) on AWS, GCP and Azure; leaving it reachable turns an SSRF in a rail adapter into credential theft. Nothing else in this chart, and no test, would notice." $except) -}}
{{- end -}}
{{- end -}}

{{/* --------------------------------------------------------------- 15 */}}
{{/*
extra-env-collision — `server.extraEnv` and `worker.extraEnv` are appended
*after* the variables this chart sets, and Kubernetes resolves a duplicate
`env` name to the LAST entry. So an operator adding `RUST_LOG` or
`VPAY_SHUTDOWN_GRACE_SECONDS` there silently overrides the chart's own value —
including `DATABASE_URL`, whose chart entry is a `secretKeyRef` and whose
override would be a plain string in the values file. The kubelet accepts it,
the Deployment rolls, and the only symptom is the process using a value nobody
set on purpose.

The reserved list is *read from* `vpay.commonEnv` rather than copied, so a
variable added to that helper is covered here without a second edit. The three
per-Deployment names below are transcribed from `deployment-server.yaml` and
`deployment-worker.yaml`, which the guard cannot read; a rename there needs a
matching edit here, and that is the one drift this guard has.
*/}}
{{- $reserved := dict -}}
{{- range $entry := (include "vpay.commonEnv" . | fromYamlArray) -}}
{{- $_ := set $reserved $entry.name "vpay.commonEnv" -}}
{{- end -}}
{{- $_ := set $reserved "VPAY_BIND" "deployment-server.yaml" -}}
{{- $_ := set $reserved "VPAY_OAUTH_SIGNING_KEY_FILE" "deployment-server.yaml" -}}
{{- $_ := set $reserved "VPAY_WORKER_CONCURRENCY" "deployment-worker.yaml" -}}
{{- range $component := list "server" "worker" -}}
{{- range $entry := (index $.Values $component).extraEnv -}}
{{- if hasKey $reserved $entry.name -}}
{{- fail (printf "vpay chart guard \"extra-env-collision\": %s.extraEnv sets %s, which this chart already sets in %s. Kubernetes keeps the LAST entry with a given name, so this silently replaces the chart's value — for DATABASE_URL that means replacing a secretKeyRef with whatever is in your values file. Change the chart's input (%s has a value for it) instead of shadowing it." $component $entry.name (index $reserved $entry.name) "values.yaml") -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{/*
The checkout page's own reserved set, which shares NONE of the names above —
it is not a vpay binary, reads no YAML config and holds no database URL. Its
four are transcribed from `deployment-checkout.yaml` for the same reason the
three above are, and carry the same one drift.
*/}}
{{- $checkoutReserved := dict
    "PORT" "deployment-checkout.yaml"
    "HOSTNAME" "deployment-checkout.yaml"
    "VPAY_API_URL" "deployment-checkout.yaml"
    "NEXT_PUBLIC_VPAY_API_URL" "deployment-checkout.yaml" -}}
{{- range $entry := .Values.checkout.extraEnv -}}
{{- if hasKey $checkoutReserved $entry.name -}}
{{- fail (printf "vpay chart guard \"extra-env-collision\": checkout.extraEnv sets %s, which this chart already sets in %s. Kubernetes keeps the LAST entry with a given name, so this silently replaces the chart's value — for NEXT_PUBLIC_VPAY_API_URL that means pointing every payer's browser at an API nobody chose here. Use checkout.publicApiUrl / checkout.apiUrl / checkout.port instead of shadowing them." $entry.name (index $checkoutReserved $entry.name)) -}}
{{- end -}}
{{- end -}}

{{/* --------------------------------------------------------------- 16 */}}
{{/*
checkout-not-templated-by-default — the twin of "dashboard-not-templated", and
it fires in the OPPOSITE direction, which is the point.

`checkout.enabled: false` must render NO checkout workload, and this is what
says so as a value rather than as an absence somebody would have to notice.
The guard file under ci/guards/ enables the page's Ingress while leaving the
page itself off — a combination that is well-typed, reads as "I turned the
checkout on", and would otherwise render an Ingress routing to a Service that
does not exist. In a cluster that is a 503 on the payment page, found by a
payer.

There is no way to express "this template rendered nothing" as a `fail`, so
the assertion has two halves: this guard makes the ONE well-typed way to get a
half-enabled checkout a named error, and `just helm-check` greps the default
render for `-checkout` to prove the other half.
*/}}
{{- if and (not .Values.checkout.enabled) .Values.checkout.ingress.enabled -}}
{{- fail "vpay chart guard \"checkout-not-templated-by-default\": checkout.ingress.enabled is true but checkout.enabled is false. Nothing about the checkout page is templated while it is disabled — not the Deployment, not the Service — so this Ingress would route to a backend that does not exist, and the symptom is a 503 on the payment page found by a payer rather than an error found here. Set checkout.enabled: true, or leave both false." -}}
{{- end -}}

{{/* --------------------------------------------------------------- 17 */}}
{{/*
checkout-templated-when-enabled — the three ways an ENABLED checkout page can
be well-typed and still not work.

(a) No `publicApiUrl`. The app reads `NEXT_PUBLIC_VPAY_API_URL` at runtime and
    THROWS on a missing one rather than defaulting (lane 3's decision), so the
    consequence is a container that starts, fails its readiness probe and
    never joins the Service. There is no default this chart could invent: the
    value is whichever hostname the Ingress serves `/v1` on, and with
    `ingress.enabled: false` the chart does not know one.

(b) An Ingress with neither `host` nor `path`, or with both. Neither means a
    host-less rule that matches every request the controller receives — the
    same failure the API's "ingress-host" guard exists for, and one that makes
    vpay answer for hostnames belonging to something else. Both is ambiguous:
    it would render a prefix rule on the checkout's own hostname, which is
    never what either shape means.

(c) An Ingress with TLS and nothing to populate the Secret. A payer's session
    credential rides in this URL's fragment; serving the controller's default
    certificate on it is not a downgrade of an internal call.
*/}}
{{- if .Values.checkout.enabled -}}
{{- if empty .Values.checkout.publicApiUrl -}}
{{- fail "vpay chart guard \"checkout-templated-when-enabled\": checkout.enabled is true but checkout.publicApiUrl is empty. That value becomes NEXT_PUBLIC_VPAY_API_URL — the origin every payer's browser sends its /v1/browser confirm and poll to — and the app throws on a missing one rather than defaulting, so the pod starts, fails readiness and never serves. Set it to whichever hostname your Ingress serves /v1 on; this chart cannot guess it, and a wrong guess would be a payer's failed payment rather than an operator's error." -}}
{{- end -}}
{{- if .Values.checkout.ingress.enabled -}}
{{- $host := .Values.checkout.ingress.host -}}
{{- $path := .Values.checkout.ingress.path -}}
{{- if and (empty $host) (empty $path) -}}
{{- fail "vpay chart guard \"checkout-templated-when-enabled\": checkout.ingress.enabled is true but neither checkout.ingress.host nor checkout.ingress.path is set. Pick one: a hostname of its own (preferred — the app's routes are /c/…, /e/… and /healthz at the root, so nothing has to be rewritten), or a path prefix on ingress.host (which needs a rewrite-target annotation, because the app is not basePath-aware). With neither, the rendered rule falls back to ingress.host at / and would answer for every path on the API's hostname." -}}
{{- end -}}
{{- if and (not (empty $host)) (not (empty $path)) -}}
{{- fail (printf "vpay chart guard \"checkout-templated-when-enabled\": checkout.ingress sets both host (%q) and path (%q). They are two different deployment shapes — its own hostname, or a prefix on the API's — and setting both renders a prefix rule on the checkout's own hostname, which is neither. Clear one." $host $path) -}}
{{- end -}}
{{- if and .Values.checkout.ingress.tls.enabled (and (empty .Values.checkout.ingress.tls.clusterIssuer) (empty .Values.checkout.ingress.tls.secretName)) -}}
{{- fail "vpay chart guard \"checkout-templated-when-enabled\": checkout.ingress.tls.enabled is true but neither clusterIssuer nor secretName is set, so nothing would ever populate the TLS Secret and the payment page would be served under the controller's default certificate. A payer's session credential rides in that URL's fragment." -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- end -}}
