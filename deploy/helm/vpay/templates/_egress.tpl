{{/*
Egress rules shared by both NetworkPolicies: DNS, Postgres, and outbound
HTTPS to the rails. Both binaries need all three — the worker calls the rails
directly (that is what it exists for) and the server does too, on `confirm`.

Emitted at zero indentation and indented by the caller, so the two policies
cannot drift on it.
*/}}
{{- define "vpay.egressRules" -}}
{{- $np := .Values.networkPolicy }}
- to:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ $np.dnsNamespace | quote }}
  ports:
    - protocol: UDP
      port: 53
    - protocol: TCP
      port: 53
{{- if $np.database.cidrs }}
- to:
{{- range $cidr := $np.database.cidrs }}
    - ipBlock:
        cidr: {{ $cidr | quote }}
{{- end }}
  ports:
    - protocol: TCP
      port: {{ $np.database.port }}
{{- end }}
{{- if $np.database.namespace }}
- to:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ $np.database.namespace | quote }}
{{- if $np.database.podSelector }}
      podSelector:
        matchLabels:
{{ toYaml $np.database.podSelector | indent 10 }}
{{- end }}
  ports:
    - protocol: TCP
      port: {{ $np.database.port }}
{{- end }}
{{- if $np.railsEgress.enabled }}
# Outbound to the rails. `0.0.0.0/0` minus the private ranges and the
# link-local block, so a compromised process cannot use this rule to reach
# another workload in the VPC or the cloud metadata endpoint at
# 169.254.169.254 — the difference between "can call MTN" and "can call
# anything".
- to:
    - ipBlock:
        cidr: 0.0.0.0/0
        except:
{{- range $cidr := $np.railsEgress.except }}
          - {{ $cidr | quote }}
{{- end }}
  ports:
    - protocol: TCP
      port: {{ $np.railsEgress.port }}
{{- end }}
{{- end -}}

{{/*
Egress to the staff SSO identity provider, for the ONE pod that calls it.

`staff_auth.sso` (see README.md, "Staff single sign-on") makes the process that
serves `/dash/v1` open outbound HTTPS to the IdP for discovery, the JWKS and
the token endpoint. That process is `-management` when `management.enabled`,
otherwise `-server` (which then serves every surface). The worker never
serves `/dash/v1`, so it never gets this rule: a rule written onto all three
policies, like `vpay.egressRules` above, would give the worker and (when split)
the business tier a path to the IdP they have no use for.

What `vpay.egressRules` already allows, which is why this exists at all:
`railsEgress` is `0.0.0.0/0` minus the private ranges and `169.254.0.0/16`,
TCP 443. A PUBLIC IdP on 443 is therefore already reachable from all three
pods while `railsEgress.enabled` is true (the default) — this rule is NOT
needed for it. It is needed when the IdP is on a private address or in-cluster
(`railsEgress.except` excludes those on purpose), on a port other than the
rails' one, or when `railsEgress.enabled` is false.

Peers are named destinations, the same shape as `networkPolicy.database`; there
is no wildcard form, so the "cannot reach the metadata endpoint / the VPC at
large" property of the rails rule is not weakened by anything this chart
renders. An operator who lists a wide CIDR here has widened it themselves;
`values.schema.json` refuses `0.0.0.0/0`, `::/0` and anything under
`169.254.` but cannot judge a merely too-wide block.

Emits nothing (an empty string, not a blank line) when no peer is configured,
or when the caller is `-server` and the management tier exists. Call with
`(dict "ctx" . "component" "server"|"management")`; the caller indents the
result, and only when it is non-empty (`with`), so the default render is
byte-identical to what it was before this rule existed.
*/}}
{{- define "vpay.idpEgressRules" -}}
{{- $np := .ctx.Values.networkPolicy.idpEgress -}}
{{- if not (and (eq .component "server") .ctx.Values.management.enabled) -}}
{{- if $np.cidrs }}
- to:
{{- range $cidr := $np.cidrs }}
    - ipBlock:
        cidr: {{ $cidr | quote }}
{{- end }}
  ports:
    - protocol: TCP
      port: {{ $np.port }}
{{- end }}
{{- if $np.namespace }}
- to:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ $np.namespace | quote }}
{{- if $np.podSelector }}
      podSelector:
        matchLabels:
{{ toYaml $np.podSelector | indent 10 }}
{{- end }}
  ports:
    - protocol: TCP
      port: {{ $np.port }}
{{- end }}
{{- end -}}
{{- end -}}
