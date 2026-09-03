{{/*
Naming, labels, and the image-reference helper.
*/}}

{{- define "vpay.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "vpay.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "vpay.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Labels on every object. */}}
{{- define "vpay.labels" -}}
helm.sh/chart: {{ include "vpay.chart" . }}
{{ include "vpay.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: vpay
{{- with .Values.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end -}}

{{- define "vpay.selectorLabels" -}}
app.kubernetes.io/name: {{ include "vpay.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
Per-component selector labels. `component` is `server` or `worker`; it is what
the two Deployments, the two Services and the NetworkPolicies all agree on.
Call as: (dict "ctx" $ "component" "server")
*/}}
{{- define "vpay.componentSelectorLabels" -}}
{{ include "vpay.selectorLabels" .ctx }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "vpay.componentLabels" -}}
{{ include "vpay.labels" .ctx }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "vpay.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "vpay.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Image reference for one component.

A digest, when set, replaces the tag entirely: `repo@sha256:...`. That is the
only reference form that cannot silently change under a deployment, which is
why `images.<component>.digest` exists at all. Its shape is checked by the
"image-digest-format" guard rather than here, so the error names the guard.

Call as: (dict "ctx" $ "component" "server")
*/}}
{{- define "vpay.image" -}}
{{- $images := .ctx.Values.images -}}
{{- $c := index $images .component -}}
{{- $repo := printf "%s/%s/%s" $images.registry $images.namespace $c.name -}}
{{- if $c.digest -}}
{{- printf "%s@%s" $repo $c.digest -}}
{{- else -}}
{{- printf "%s:%s" $repo (default .ctx.Chart.AppVersion $c.tag) -}}
{{- end -}}
{{- end -}}

{{/*
Pod-level securityContext, shared by both workloads.

65532 matches `backends/Dockerfile`'s `USER 65532:65532`. The runtime image is
`FROM scratch` and has no /etc/passwd, so the UID is the only identity there
is — a `runAsNonRoot: true` with no explicit `runAsUser` would make the
kubelet refuse to start the pod ("container has runAsNonRoot and image has
non-numeric user"), which is a fine failure but not the one we want.
*/}}
{{- define "vpay.podSecurityContext" -}}
runAsNonRoot: true
runAsUser: 65532
runAsGroup: 65532
fsGroup: 65532
seccompProfile:
  type: RuntimeDefault
{{- end -}}

{{/*
Container-level securityContext.

`readOnlyRootFilesystem: true` is safe *as far as anyone can tell*: the
scratch image has no writable path to begin with, and no code in either binary
opens a file for writing. That is "no observed writer", not "proven" — no pod
has ever run.
*/}}
{{- define "vpay.containerSecurityContext" -}}
allowPrivilegeEscalation: false
readOnlyRootFilesystem: true
runAsNonRoot: true
capabilities:
  drop:
    - ALL
{{- end -}}

{{/*
Environment shared by both binaries — every name is a flag declared on
`CommonArgs` in `backends/crates/vpay-config/src/cli.rs`.
*/}}
{{- define "vpay.commonEnv" -}}
- name: VPAY_PROFILE
  value: {{ .Values.config.profile | quote }}
# Also set by the image (`ENV VPAY_CONFIG`); restated because a missing value
# is an exit 78 and this is where an operator looks first.
- name: VPAY_CONFIG
  value: {{ .Values.config.path | quote }}
- name: RUST_LOG
  value: {{ .Values.logFilter | quote }}
# Already the binary's default; belt-and-braces, not a fix.
- name: VPAY_LOG_FORMAT
  value: {{ .Values.logFormat | quote }}
- name: VPAY_SHUTDOWN_GRACE_SECONDS
  value: {{ .Values.shutdownGraceSeconds | quote }}
# `--observability-bind` on both binaries since 2026-09-03 (step-6 block A):
# the socket /livez and /metrics are served on, and never the traffic port.
- name: VPAY_OBSERVABILITY_BIND
  value: {{ printf "0.0.0.0:%d" (int .Values.observability.port) | quote }}
- name: DATABASE_URL
  valueFrom:
    secretKeyRef:
      name: {{ .Values.database.existingSecret | quote }}
      key: {{ .Values.database.existingSecretKey | quote }}
{{- end -}}

{{/*
The overlay ConfigMap's name and its single key. The key must be exactly what
`vpay_config`'s `profile_overlay_path` looks for beside the base file:
`<stem>-<profile>.<ext>`.
*/}}
{{- define "vpay.overlayConfigMapName" -}}
{{- printf "%s-config-overlay" (include "vpay.fullname" .) -}}
{{- end -}}

{{- define "vpay.overlayFileName" -}}
{{- $base := base .Values.config.path -}}
{{- $ext := ext $base -}}
{{- $stem := trimSuffix $ext $base -}}
{{- printf "%s-%s%s" $stem .Values.config.profile $ext -}}
{{- end -}}

{{/* Absolute path the overlay is mounted at, inside the baked /config dir. */}}
{{- define "vpay.overlayMountPath" -}}
{{- printf "%s/%s" (dir .Values.config.path) (include "vpay.overlayFileName" .) -}}
{{- end -}}
