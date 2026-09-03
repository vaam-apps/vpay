# Runbook: rising rail-call failure rate

## Alert

**`VpayProviderErrorRateHigh`** — `deploy/helm/vpay/templates/prometheusrule.yaml`,
group `vpay.rules`, `severity: warning`, label `provisional: "true"`,
`runbook_url` pointing here.

```promql
sum by (provider) (rate(vpay_provider_requests_total{error_kind!=""}[15m]))
/
sum by (provider) (rate(vpay_provider_requests_total[15m]))
> 0.05
```

`for: 15m`. The window is `metrics.prometheusRule.providerErrorWindow`
(default `15m`) and the ratio is `metrics.prometheusRule.providerErrorRatio`
(default `0.05`). The `error_kind` label is `vpay_core::Classify::code` — the
same vocabulary the `provider_requests.error_kind` column stores and the same
one a merchant sees in an error envelope, so the alert's dimension and step 4
below are talking about the same thing.

> **Why `error_kind!=""` and not `error_kind="provider_error"`.** ~~The
> numerator used to select `error_kind="provider_error"` alone.~~ It named one
> failure code, and not the one an outage produces: a rail that stops
> answering raises `ProviderError::Transport`, whose code is
> `provider_unavailable`, so through a total rail outage the numerator stayed
> at zero while the denominator climbed — the alert could not fire during the
> incident it is named for. `error_kind` is `""` only on success, so `!=""` is
> "the call did not succeed", and the alert is now about the rail rather than
> about one classification of its answers.
>
> **⚠ That includes declines, and on mobile money that matters.**
> `error_kind="charge_declined"` is a rail *decision*
> (`vpay_provider::ProviderError::Rejected` — an insufficient balance, a payer
> timeout), not a rail failure, and it is routinely a large share of all
> calls. Against real traffic this rule will fire on an ordinary decline rate.
> Leaving it that way (one alert for "calls to this rail are not succeeding")
> or excluding declines with `error_kind!~"|charge_declined"` is a maintainer
> decision to make alongside the threshold itself, after the first week of
> measured traffic — it has not been made. Until it is, **split by
> `error_kind` before you act on this alert**: which code dominates decides
> which half of this runbook applies.

> **Two more things about that rule, both load-bearing.**
>
> **1. `vpay_provider_requests_total` is emitted, and has never been
> scraped.** As of 2026-09-03 (step 6, block C) it is recorded by
> `vpay_provider::Measured`, the port decorator every rail adapter is wrapped
> in — so it counts *port calls* rather than HTTP requests: an Orange
> `submit` that mints an access token first is two requests on the wire and
> one increment here, and a call refused before the socket opens is counted
> with that refusal's `error_kind`. Both binaries serve it on
> `--observability-bind`. What has never happened is a scrape: no Prometheus
> has polled a vpay process, so this rule has never been evaluated against
> real series, and `metrics.prometheusRule.enabled` is `false` by default.
>
> **2. The 5 % threshold is proposed, not measured** (step-6 decision (5)).
> This runbook contained no number to transcribe — it said "crosses its
> threshold" and nothing more — so the figure was invented against a system
> that has never taken a real payment. A 5 % `provider_error` rate on a live
> MTN account may be perfectly normal. Tighten or loosen it after the first
> week of real traffic; the `provisional: "true"` label exists so this is
> visible in Alertmanager and not only here.

## What it means

The alert says only that a large share of port calls to one rail are not
succeeding. **Which `error_kind` dominates decides what is actually wrong**, so
that is step 0:

```promql
sum by (error_kind) (rate(vpay_provider_requests_total{provider="<rail>"}[15m]))
```

| Dominant `error_kind` | What it is | Where to go |
|---|---|---|
| `provider_unavailable` | Transport: the rail was unreachable, or did not answer. Not a mapping problem and not something to fix in an adapter. | [unresolved-charges.md](unresolved-charges.md) — charges the poll ladder cannot resolve are the consequence. Check the rail's own status page first. |
| `charge_declined` | The rail decided. Ordinary business on a push rail, and the reason this alert can fire on a perfectly healthy system — see the ⚠ above. | Nothing operational, unless the decline *mix* changed: group `charges.failure_code`. A jump in `provider_account_blocked` is a page, not a decline. |
| `provider_error` | The adapter saw a rail error string it does not recognise. | The rest of this runbook. |
| `misconfigured` | Our YAML or our credentials — a missing `${VAR}`, a rejected key. | [rotate-rail-credentials.md](rotate-rail-credentials.md). Fix the deployment, not the mapping. |
| `operation_unsupported_by_rail` / `not_implemented` | vpay called something this rail cannot do, or something nobody has built. Both are bugs here, not rail problems. | `docs/status.md`, then a code fix. |

The rest of this runbook is the `provider_error` case.

`provider_error` is the escape hatch in the failure taxonomy: the adapter saw a
rail error string it does not recognise. A rising rate almost always means the
rail changed its error vocabulary and the adapter's mapping table has drifted.

It is **not** a payment problem in itself. It is a *visibility* problem, and it
degrades every downstream decision — merchants cannot tell a retryable failure
from a permanent one.

## Steps

1. Group recent `provider_error` charges by **`charges.failure_raw`** (the
   column's name; there is no `failure_reason_raw`). `charges.failure_code`
   is the taxonomy value; `failure_raw` is the rail's own words, capped at
   2000 characters by a CHECK.
2. If one raw string dominates, that is the new code. Map it to the right
   taxonomy entry in the adapter and ship.
3. If the raw strings are varied and include transport errors, check whether the
   rail is degraded — those may belong under `provider_unavailable`.
4. If the raw reason is empty, the adapter is failing to parse a changed
   response shape. **`provider_requests` will not show you the body — it
   stores none**, by design: only the charge, the operation, the reference,
   the attempt, `status_code`, `error_kind`, `sent_at` and `responded_at`.
   Use it to establish *whether and how* the rail answered, then go to the
   application logs for the (truncated) body.

   Reading `provider_requests` for that: `status_code IS NULL` means no
   answer was received; `status_code = 0` is a sentinel meaning the rail
   answered but the port carries no HTTP status (an accepted submit, or a
   decline) — see migration `0020`; anything else is the real HTTP status.
   `error_kind` is the error's own classification code, the same vocabulary
   the merchant's envelope uses: `provider_unavailable` (transport),
   `provider_error` (unparseable answer), `charge_declined` (the rail
   decided), `misconfigured` (our YAML or credentials — fix the deployment,
   not the mapping), `operation_unsupported_by_rail`, `not_implemented`.

5. **A rising `provider_error` rate with `status_code = 0` and
   `error_kind = 'charge_declined'`** is the mapping-drift case this runbook
   is about: the rail decided, and the adapter could not name the reason. A
   rising rate with `status_code IS NULL` and
   `error_kind = 'provider_unavailable'` is not — that is the rail being
   unreachable, and it belongs to the unresolved-charges runbook. Both now
   fire the *same* alert, which is why the `error_kind` split above is step 0
   rather than a footnote.

## Do not

- Do not widen an existing mapping to swallow the unknown string. A wrong
  mapping is worse than an honest `provider_error`, because it tells the
  merchant something false about whether to retry.
