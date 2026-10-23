# 2026-10-23 — An alert on the callback-endpoint counter, and unit tests for all six rules

## Whose decision

The maintainer, asked whether an alert on `vpay_callback_endpoint_checks_total`
was possible ("#15: is it even possible at first?"), was answered yes, with the
expression and a proposal to test every rule with `promtool`. Their reply was
"Fan out the work via sub-agents". Three calls below are the implementer's, and
each says so: the states that count as confirmed, the window, and `warning` in
`check` mode.

## What was built

| Piece                                                                                            | Where                                                 |
| ------------------------------------------------------------------------------------------------ | ----------------------------------------------------- |
| `VpayCallbackEndpointUnconfirmed`, the sixth rule                                                | `deploy/helm/vpay/templates/prometheusrule.yaml`      |
| `metrics.prometheusRule.callbackEndpointWindow: 3h`, and its schema                              | `values.yaml`, `values.schema.json`                   |
| Unit tests for all six rules                                                                     | `deploy/helm/vpay/ci/prometheusrule.test.yaml`        |
| `just test-alert-rules`: render, `promtool check rules`, a coverage check, `promtool test rules` | `justfile`                                            |
| CI: promtool 3.13.5, pinned by version and SHA-256, then the recipe                              | `.github/workflows/ci.yml`, `deploy (helm chart)` job |
| The runbook                                                                                      | `docs/runbooks/callback-endpoints.md`                 |

The rule:

```promql
sum by (provider) (increase(vpay_callback_endpoint_checks_total[3h])) > 0
unless on (provider)
sum by (provider) (increase(vpay_callback_endpoint_checks_total{state=~"present|created|foreign"}[3h])) > 0
```

It is `for: 10m`, `severity: warning` and `provisional: "true"`. The three
implementer's calls:

- **Confirmed means `present`, `created` or `foreign`.** `drifted` counts
  against the rail: vpay's endpoint lacks an event type, and nothing on Notch
  Pay can repair it. `foreign` counts for it: callbacks reach vpay's URL.
- **3h** is three hourly passes. A transient failure retries after five
  minutes, so the rule needs a sustained failure to fire.
- **`warning`, in `check` mode too, never `critical`.** Callbacks are hints and
  every charge still settles by polling. The metric has no `registration`
  label, and in `check` mode `missing` is exactly what the operator asked to be
  told about.

**The expression in the request would not have worked, and the tests show
it.** The proposal compared the confirming states' `sum(...)` with `== 0`,
joined by `and` to a `sum(...)` of all passes above zero. A rail that has
never been confirmed has no `present` or `created` series, so `sum(...) == 0`
over it returns nothing and the rule could never fire:

- not for a `check` deployment that was never registered;
- not for a key refused from the first pass;
- not for an endpoint drifted from the day it was made.

`unless` keeps the left side when the right side is empty.

## How it is proven

`just test-alert-rules`, run 2026-10-23 with helm v3.16.2 and promtool 3.13.5,
both on `PATH`:

```text
Checking /tmp/tmp.KRYYiRmn0e/rules.yaml
  SUCCESS: 6 rules found

  SUCCESS

test-alert-rules: ok — 6 rules, each with a firing and a silent case, against synthetic series. No real Prometheus has ever scraped vpay.
```

**What the tests cover:**

- 11 test groups and 20 evaluations: 8 expect an alert, with its full label set
  and annotation text, and 12 expect silence.
- The new rule has five groups:
  - a `check`-mode rail never registered fires, beside a `foreign` one that
    does not;
  - a `drifted` one fires;
  - a healthy one with a transient `failed` never fires;
  - a confirmed rail fires only once a whole window passes with no `present`;
  - a rail with no passes cannot fire.

  It also pins that nothing fires before a worker's second pass.

- The five older rules each have a firing case and a silent neighbour:
  - a second rail under the error ratio;
  - `to="succeeded"` beside `to="unresolved"`;
  - the queue gauge's healthy negative value;
  - a page event aged out of its window;
  - `outcome="finished"` beside `dead_letter`.

**Mutations, each reverted before the next:**

1. **The proposed `== 0 … and` form in place of `unless`.** Both
   never-confirmed groups fail:

   ```text
     FAILED:
       alertname: VpayCallbackEndpointUnconfirmed, time: 1h30m,
           exp:[
                 Labels:{alertname="VpayCallbackEndpointUnconfirmed", provider="notchpay", provisional="true", severity="warning"}
           got:[]
       alertname: VpayCallbackEndpointUnconfirmed, time: 1h30m,
           exp:[
                 Labels:{alertname="VpayCallbackEndpointUnconfirmed", provider="notchpay", provisional="true", severity="warning"}
           got:[]
   ```

2. **`VpayUnresolvedChargesRising` reading `to="succeeded"`.** It fires on
   the `mtn_momo` series at 12m and misses `notchpay` at 20m: FAILED.
3. **The silent `VpayJobsDeadLettered` case deleted.** The recipe refuses
   before promtool runs: `test-alert-rules: FAIL — VpayJobsDeadLettered needs
a firing case and a silent case in deploy/helm/vpay/ci/prometheusrule.test.yaml
(has: fires )`.
4. **A rule renamed in the template.** The recipe refuses, listing
   `rendered:` and `tested:` side by side.

## Gates

| Check                                                            | Result                                                                                                                                                                                |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just test-alert-rules`                                          | ok, above                                                                                                                                                                             |
| `just helm-check`                                                | ok: lint, 4 renders, 24 guards, kubeconform `Valid: 68, Invalid: 0`. `ci/values-full.yaml` enables the rule group, so the six-rule PrometheusRule is validated against the CRD schema |
| `just verify`                                                    | `verify: ok — the fifteen gates above passed`; `verify-links` 2696 links, `verify-doc-counts` 13 counts. `verify-citations`: ok                                                       |
| `prettier --check` on every changed Markdown, YAML and JSON file | clean                                                                                                                                                                                 |
| the skills' `node tools/verify-coverage.mjs` against this branch | parity                                                                                                                                                                                |

## Not done

- **No real Prometheus has ever scraped a vpay process.** The rules are
  evaluated against synthetic series written from the label sets the code
  emits (`vpay_core::metrics`), never against recorded ones. The rule group is
  still off by default.
- **The 3h window is proposed, not measured**, like every threshold in the
  file.
- **A pass that never runs cannot fire the rule**: no series, no alert. The job
  rules cover that, and nothing here tests that they would.
- **Not in `just ci`**: helm and promtool are not built by the workspace, for
  `helm-check`'s reason. CI runs the recipe in `deploy (helm chart)`, which runs
  only when `deploy/helm/**`, the justfile or the workflow changes.
- **No call to any rail**, and no endpoint was registered anywhere.
