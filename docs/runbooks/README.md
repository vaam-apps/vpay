# Runbooks

Operational procedures. Each answers: how do I know this is happening, what do I
do, and how do I know it is fixed.

| Runbook | Trigger | Alert |
|---|---|---|
| [unresolved-charges.md](unresolved-charges.md) | A charge passed 24h with no terminal answer | `VpayUnresolvedChargesRising` |
| [provider-error-rate.md](provider-error-rate.md) | Failed rail calls rising as a share of all calls (any `error_kind`) | `VpayProviderErrorRateHigh` |
| [worker-queue.md](worker-queue.md) | A dead-lettered job, a stranded lease, or a rail contradicting a settled charge | `VpayJobQueueBehind`, `VpayJobsDeadLettered` |
| [webhook-delivery-failures.md](webhook-delivery-failures.md) | A delivery in `exhausted`, an endpoint with no signing secret, or a secret rotation | — |
| [release.md](release.md) | Cutting a `v*` tag, verifying an image signature, pinning a digest in Helm values | — |
| [deploy-and-rollback.md](deploy-and-rollback.md) | A `helm upgrade`, a rollback, or a pod exiting 78/69/1 during a rollout | — |
| [rotate-signing-key.md](rotate-signing-key.md) | Rotating the OAuth signing key; a server crash-looping on a retired `kid` | — |
| [rotate-rail-credentials.md](rotate-rail-credentials.md) | Rotating an MTN or Orange credential; revoking a merchant client (ADR-0010's dual-authority check) | — |
| [restore-from-backup.md](restore-from-backup.md) | Restoring a database, and the quarterly drill [ADR-0013](../adr/0013-database-backups-and-retention.md) proposes | — |
| [demo.md](demo.md) | Bringing vpay up from nothing and walking six payments through both rails — the one page here whose output is a real run | — |
| [checkout.md](checkout.md) | Integrating vpay's own payment page, hosted and embedded — and seeing an unregistered origin refused | — |

The `Alert` column names the rule in
`deploy/helm/vpay/templates/prometheusrule.yaml` whose `runbook_url` points at
that page. Two facts about all of them, stated here so no page has to be
trusted on its own:

- **Every metric these rules query is now emitted — and nothing has ever
  scraped one.** As of 2026-09-03 (step 6, block C) both binaries serve
  `/metrics` on `--observability-bind` (default `0.0.0.0:9090`) and
  `vpay_provider_requests_total`, `vpay_charge_transitions_total`,
  `vpay_jobs_*`, `vpay_error_events_total` and `vpay_alert_events_total` are
  recorded on real request, rail and settlement paths (`docs/status.md` names
  the seam for each). What is still untrue is the other half: no Prometheus
  has ever polled a vpay process, the chart's `ServiceMonitor` is off by
  default, and `metrics.prometheusRule.enabled` is `false`. So these rules
  have never been *evaluated* against real series — the metric existing and
  the alert working are two claims, and only the first has evidence.
- **Every threshold is proposed, not derived** (step-6 decision (5)). The two
  runbooks that predate the chart contained no numbers to transcribe
  ("crosses its threshold", "more than one hour"), so the figures were
  invented against a system that has never taken a real payment. The
  `PrometheusRule` is off by default and each rule carries
  `provisional: "true"`. `VpayProviderErrorRateHigh` additionally counts
  declines (`error_kind="charge_declined"`) as failures, which on a mobile
  money rail is a normal and large share of traffic — see that page.

`VpayPageableErrorEvents` has no runbook of its own and points here: it fires
on any error [ADR-0011](../adr/0011-error-modelling.md) classifies
`Severity::Page`, and the classification *is* the alert — there is no
threshold to tune.

**Status:** written from the design, never exercised against a running system.
No runbook here has been followed against a deployment, because no deployment
exists. See [../status.md](../status.md).

**[demo.md](demo.md) and [checkout.md](checkout.md) are the exceptions, and are
a different kind of page.** Neither describes an alert or an incident:
`demo.md` is the procedure for bringing the whole stack up on one machine, and
`checkout.md` is how a merchant integrates the checkout page, written from
`examples/shop`'s own source with the demo stack as its worked example. Both
have been executed: `demo.md`'s §4 is a paste of a real run, and
`checkout.md`'s §4 buying flow is driven end to end in a real browser by
Cypress (its `docker compose` and `psql` verifications are not). **Neither has
been followed against a deployment**, because there is none, and neither is
evidence about MTN or Orange — the rails are WireMock hosts. One thing
`checkout.md` says about itself is worth repeating here: its `frame-ancestors`
header has been read off the wire (`cy.request`) but by nobody's browser, and
what a browser has been observed refusing is the checkout page's own origin
check.

The paragraph that follows is `demo.md`'s own history and is unchanged.

**[demo.md](demo.md) is the exception, and is a different kind of page.** It
describes no alert and no incident: it is the procedure for bringing the whole
stack up on one machine and driving six payments through it, and every command
and every line of output on it was run on 2026-09-03/04. It is also the only
page here that reports its own failure — the walkthrough found a real race
between `vpay-api`'s confirm and `vpay-worker`'s first poll (its §9). ~~`just
demo` end to end has not been observed green~~ **— corrected 2026-09-04: that
race was fixed the same day (`docs/status.md`'s confirm/worker race row).** What
exists is this: **one green run from nothing (lane A's rebased branch,
2026-09-04, *without* lane G; the race is timing-dependent and did not fire),
lane A's own earlier count was two greens in six attempts and zero for three
from nothing, lane G did not re-run the demo. Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in): `just demo` from nothing six times, **four green** (six outcomes for six each, exit 0); the two failures were the VM's Postgres answering single statements in 14–36 s under host I/O pressure, with the settlement and the webhook both landing in the worker's log after the demo's budgets; `write_matched_no_row` appeared in no run. Three from nothing is met in count, not consecutively.** **Updated 2026-09-04
(Step 9): `just demo` from nothing ran green three times in a row in the
`vpay-ci` VM on the merged Step 9 branch, so that bar is now met consecutively
as well as in count.** The rails it drives are WireMock hosts, so nothing on
that page is evidence about MTN or Orange.

- [worker-queue.md](worker-queue.md) and
  [webhook-delivery-failures.md](webhook-delivery-failures.md) are the two
  whose states a test actually produces — the worker's integration suite
  makes dead letters, stranded leases, `unresolved` escalations and exhausted
  deliveries on purpose — and both had their SQL run against a database with
  every migration applied, the webhook one including its replay transaction,
  run twice to confirm the second run is a no-op. No replayed delivery has
  been observed reaching a receiver.
- [restore-from-backup.md](restore-from-backup.md) is the newest with real
  evidence behind part of it: every SQL statement in it was executed on
  2026-09-03 against a scratch `postgres:16-alpine` with all 21 migrations
  applied, including a negative control in which the ledger-balance check
  found a deliberately torn transaction and then reported clean once it was
  repaired. **Nothing about backups, PITR or the restore itself was
  exercised — no backup of any vpay database has ever been taken.**
- [deploy-and-rollback.md](deploy-and-rollback.md),
  [rotate-signing-key.md](rotate-signing-key.md) and
  [rotate-rail-credentials.md](rotate-rail-credentials.md) are written from
  the chart, the binaries' own shutdown and boot code, and their tests. **No
  `kubectl` or `helm` command in any of them has been run against a
  cluster**, because no cluster has ever run vpay.
- [release.md](release.md): at the time it was written no tag had been pushed,
  `.github/workflows/release.yml` had never run, and no image existed to
  verify a signature on.
