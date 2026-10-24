# Runbook: callback endpoints

## Alert

**`VpayCallbackEndpointUnconfirmed`** is in
`deploy/helm/vpay/templates/prometheusrule.yaml`, group `vpay.rules`. It has
`severity: warning` and label `provisional: "true"`, and its `runbook_url`
points here. It was added 2026-10-23.

```promql
sum by (provider) (increase(vpay_callback_endpoint_checks_total[3h])) > 0
unless on (provider)
sum by (provider) (increase(vpay_callback_endpoint_checks_total{state=~"present|created|foreign"}[3h])) > 0
```

`for: 10m`. `3h` is `metrics.prometheusRule.callbackEndpointWindow`.

`vpay_callback_endpoint_checks_total{provider,state}` is one increment per
rail per `ensure_callbacks` pass ([RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md)).
The pass runs at every worker start and then hourly, and five minutes after
a rail or the database did not answer. The rule fires when passes ran for a
rail across the whole window and **none of them confirmed an endpoint at
vpay's callback URL**: every pass said `missing`, `drifted`, `failed` or
`locked`.

**What it costs while it fires: latency, not money.** Callbacks are hints in
vpay. Every charge on the rail still settles, because the worker polls it,
but nothing pulls the poll forward. A payer waits for the next rung of the
poll ladder instead of the callback.

**Three things in the rule that are easy to read wrongly.**

- It is `unless`, not `== 0`. A rail that has never been confirmed has no
  `present` or `created` series at all, for example a `register: check`
  deployment whose endpoint was never registered, or a key refused from the
  first pass. `sum(...) == 0` over a missing series returns nothing, so it
  could not fire in exactly those cases. The rule tests pin both
  (`deploy/helm/vpay/ci/prometheusrule.test.yaml`, run by
  `just test-alert-rules`).
- It cannot fire before a worker's **second** pass, an hour after the worker
  starts, plus `for:`. A counter's first sample shows no increase.
- It fires for `register: check` exactly as for `ensure`. The metric carries
  no `registration` label. In `check` mode, `missing` is the very thing the
  operator asked to be told about instead of having vpay write. It is a
  `warning` in every mode and never `critical`, for the latency reason above.
  _(The choice: decided 2026-10-23 with the rule. Paging on a hint was judged
  wrong.)_
- It never fires for a rail whose setting is `off`: the pass skips it and
  counts nothing. **That is Notch Pay's default since 2026-10-23**: with
  `providers[].callbacks.register` unset, vpay asks a Notch Pay account
  nothing, and the operator registers the endpoint in Notch Pay's dashboard
  ([RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md) § "As built",
  the amendment of that date). So on a default deployment this alert says
  nothing about Notch Pay at all. Set `register: check` to have it watch a
  hand-registered endpoint, which then must carry vpay's description
  (below).

## Which state, and what to do

Read the pass's log line for the rail first. `vpay-worker` logs one per rail
per pass, naming the state, the marker, the callback URL and the endpoint
ids. You can also run the pass by hand, read-only:

```bash
# With the deployment's own configuration and database URL in the environment,
# as the worker has them (VPAY_CONFIG, the profile, the database URL).
vpay-server callbacks ensure --check
# notchpay check missing -
```

It prints one line per rail, `{rail} {register} {state} {detail}`, and
exits non-zero for `missing`, `locked` or `failed`.

| State     | What it means                                                                                         | What to do                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --------- | ----------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `missing` | No endpoint of vpay's, and the rail's `callbacks.register` is `check`, so the pass may not create one | Either let vpay create it (set `register: ensure`, then run `vpay-server callbacks ensure` or restart a worker), or register it by hand with **vpay's description** (below). A hand-made endpoint with any other description is invisible on Notch Pay, which answers `"url": null`, and this alert keeps firing. On Notch Pay prefer the hand-made one: the sandbox stored the endpoint its API created with no URL and delivered nothing (2026-10-23), which is why `ensure` is not that rail's default |
| `drifted` | vpay's endpoint exists and is missing one of the five payment events                                  | Delete it in the rail's dashboard. With `register: ensure`, the next pass (or `vpay-server callbacks ensure`) creates a correct one. vpay cannot repair it: the Notch Pay client has no update call                                                                                                                                                                                                                                                                                                       |
| `failed`  | The rail refused, did not answer or answered something unreadable, or the database failed             | The log line says which. A refused key is logged at `error` with `alert = true`: see [rotate-rail-credentials.md](rotate-rail-credentials.md). A rail that did not answer is retried every five minutes, and [provider-error-rate.md](provider-error-rate.md)'s alert is probably firing too                                                                                                                                                                                                              |
| `locked`  | Another pass held the registration lock, and this one asked nothing                                   | One `locked` is normal, when a `callbacks ensure` ran beside the job. Three hours of nothing else means something holds the lock: look for advisory lock `vpaycbk` in `pg_locks` (below), and for a hung `vpay-server callbacks` process                                                                                                                                                                                                                                                                  |

`present` and `created` are healthy. `foreign` (an endpoint at vpay's URL
that vpay did not create, left as found) counts as healthy too, because
callbacks reach vpay through it. On Notch Pay it is never reported, for the
same `"url": null` reason. A rail set to `off` is never counted, so it cannot
fire this alert.

**Registering by hand, so that the pass recognises it.** A Notch Pay endpoint
is recognised by its description, never by its URL. Give it the five payment
events (`payment.created`, `payment.complete`, `payment.failed`,
`payment.canceled`, `payment.expired`), vpay's callback URL, and this
description:

```bash
URL="https://pay.example.com/provider/notchpay/callback"   # the rail's callback URL
DEPLOYMENT="acme-prod"                                       # deployment.name
printf 'vpay:%s:notchpay:default url:%s\n' "$DEPLOYMENT" \
  "$(printf %s "$URL" | sha256sum | cut -c1-16)"
# vpay:acme-prod:notchpay:default url:…16 hex digits…
```

**Who holds the lock.** `vpaycbk` is the bigint advisory key
`0x0076_7061_7963_626b`. Postgres reports it as `classid` 7762017 and `objid`
2036556395:

```sql
SELECT l.pid, a.application_name, a.state, a.query_start, a.query
  FROM pg_locks l JOIN pg_stat_activity a USING (pid)
 WHERE l.locktype = 'advisory' AND l.classid = 7762017 AND l.objid = 2036556395;
```

The lock is transaction-scoped, so ending the holder's transaction or
connection releases it. Nothing has to be cleaned up afterwards.

## How you know it is fixed

`vpay-server callbacks ensure --check` prints `{rail} {register} present
{endpoint id}` and exits 0. The alert resolves at the first evaluation after
a pass counts `present` or `created` and Prometheus has scraped it, so within
the hour, or straight away if you ran `vpay-server callbacks ensure` (without
`--check`) against a worker that a Prometheus scrapes.

## What this alert does not see

- **A pass that never runs.** A parked job or no worker emits nothing, and
  nothing can fire on nothing. `VpayJobsDeadLettered` and
  `VpayJobQueueBehind` ([worker-queue.md](worker-queue.md)) are the rules for
  that.
- **Whether callbacks actually arrive.** A `present` endpoint says the rail
  holds vpay's URL. It does not say the rail posts to it. On Notch Pay's
  sandbox, the endpoint `POST /webhooks` created on 2026-10-23 delivered
  nothing in 21 minutes, while one registered by hand in the dashboard
  delivered four webhooks on 2026-10-24
  ([verification](../status/verification/2026-10-24-notchpay-dashboard-webhooks.md)).
  No Notch Pay webhook has reached a running vpay. _(This said "No Notch Pay
  webhook has ever been received (RFC-0012 § 10 item 7)" until 2026-10-24.)_
- **Endpoints vpay does not own on Notch Pay**: superseded and duplicate ones.
  Each is logged at `warn` by id for an operator to delete in the dashboard.
  That is an operator's task since 2026-10-23, by the maintainer's decision.

**Status:** written with the rule, never exercised against a running system.
The rule is tested by `promtool` against synthetic series only. **No
Prometheus has ever scraped a vpay process**, the rule group is off by default
(`metrics.prometheusRule.enabled: false`), and the 3h window is proposed,
not measured. See [README.md](README.md).
