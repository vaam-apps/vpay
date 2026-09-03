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
