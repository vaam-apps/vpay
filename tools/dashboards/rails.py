#!/usr/bin/env python3
"""Generates deploy/dashboards/vpay-rails.json: payment rails, charges and webhooks. Standard library only.

Run `sh tools/dashboards/generate.sh`; CI regenerates and fails on any difference. Every metric is a series of
`vpay_core::metrics`, held to that vocabulary by `backends/crates/vpay-telemetry/tests/dashboards.rs`.
"""

from lib import dashboard, row, stat, text, timeseries, write

RATE = "[$__rate_interval]"

PANELS = [
    text(
        "What a rail panel does and does not say",
        "**These panels count calls vpay made to the host a rail is configured with, and what came back.** A rail "
        "configured against a stub host (a WireMock container in the dev, demo and end-to-end stacks) reports exactly "
        "like a live one, and a rail whose adapter is not written reports its `not_implemented` error here. "
        "Nothing on this dashboard says a rail *works*: `docs/status.md` says which rails have been called "
        "live, and when. The calls are port calls, not wire requests (`docs/reference/vpay-core.md`).",
        (0, 0, 24, 4),
    ),
    row("Rail calls", y=4),
    timeseries(
        "Rail calls by provider and operation",
        "Calls per second to each rail, by port operation (prepare, submit, query_status, refund, account_holder_name, ensure_callback_endpoint).",
        (0, 5, 12, 8),
        [
            (
                'sum by (provider, operation) (rate(vpay_provider_requests_total{job=~"$job"}' + RATE + "))",
                "{{provider}} {{operation}}",
            )
        ],
        unit="ops",
    ),
    timeseries(
        "Rail call errors by provider and kind",
        "Calls that failed, per second, by rail and by error code (the same words as provider_requests.error_kind). not_implemented is an adapter that does not do this yet, not an outage.",
        (12, 5, 12, 8),
        [
            (
                'sum by (provider, error_kind) (rate(vpay_provider_requests_total{job=~"$job",error_kind!=""}'
                + RATE
                + "))",
                "{{provider}} {{error_kind}}",
            )
        ],
        unit="ops",
    ),
    timeseries(
        "Rail call error ratio by provider",
        "The share of each rail's calls that failed. A rail with a handful of calls has a ratio that jumps on one failure; read it with the rate above.",
        (0, 13, 12, 8),
        [
            (
                'sum by (provider) (rate(vpay_provider_requests_total{job=~"$job",error_kind!=""}'
                + RATE
                + ')) / sum by (provider) (rate(vpay_provider_requests_total{job=~"$job"}'
                + RATE
                + "))",
                "{{provider}}",
            )
        ],
        unit="percentunit",
    ),
    timeseries(
        "Rail call latency by provider (p50, p95, p99)",
        "Wall-clock time of one call to a rail, per provider. Mobile-money rails are slow in a way an HTTP route is not.",
        (12, 13, 12, 8),
        [
            (
                'histogram_quantile(%s, sum by (le, provider) (rate(vpay_provider_request_duration_seconds_bucket{job=~"$job"}%s)))'
                % (q, RATE),
                "{{provider}} " + label,
            )
            for q, label in ((0.5, "p50"), (0.95, "p95"), (0.99, "p99"))
        ],
        unit="s",
    ),
    row("Charges", y=21),
    timeseries(
        "Charge transitions by new state",
        "Charges entering each state per second, by rail. A charge is the unit that moves through submitting, submitted, pending, unresolved, succeeded and failed; the payment intent's own status is not exported (it has no index to count by).",
        (0, 22, 12, 8),
        [
            (
                'sum by (provider, to) (rate(vpay_charge_transitions_total{job=~"$job"}' + RATE + "))",
                "{{provider}} -> {{to}}",
            )
        ],
        unit="ops",
        fill=40,
        stacking="normal",
    ),
    timeseries(
        "Charge transitions (from, to)",
        "Each edge of the charge state machine per second. The edge is the alertable quantity: submitted to failed is a decline, pending to unresolved is a poll ladder that ran out.",
        (12, 22, 12, 8),
        [
            (
                'sum by (from, to) (rate(vpay_charge_transitions_total{job=~"$job"}' + RATE + "))",
                "{{from}} -> {{to}}",
            )
        ],
        unit="ops",
    ),
    stat(
        "Charges that went unresolved in the range",
        "Charges whose poll ladder ended with no answer from the rail. Each one needs a human: orange at one.",
        (0, 30, 6, 4),
        ['sum(increase(vpay_charge_transitions_total{job=~"$job",to="unresolved"}[$__range]))'],
        unit="short",
        steps=[(None, "green"), (1, "orange"), (10, "red")],
    ),
    stat(
        "Charges that failed in the range",
        "Charges that ended failed: a rail declining or an error. Read with the error panels above; a payer who declines is not an incident.",
        (6, 30, 6, 4),
        ['sum(increase(vpay_charge_transitions_total{job=~"$job",to="failed"}[$__range]))'],
        unit="short",
        steps=[(None, "green")],
    ),
    timeseries(
        "Provider callbacks by verdict",
        "Callbacks received for a charge vpay holds, by what the rail's check said. invalid is a wrong signature (a misconfigured secret, or a forgery); no_secret is a rail that signs and a deployment with no secret set; not_checked is a rail whose callbacks vpay does not check.",
        (12, 30, 12, 8),
        [
            (
                'sum by (provider, verdict) (rate(vpay_provider_callbacks_total{job=~"$job"}' + RATE + "))",
                "{{provider}} {{verdict}}",
            )
        ],
        unit="ops",
    ),
    row("Webhooks to merchants", y=38),
    stat(
        "Webhook backlog",
        "Deliveries owed an attempt (state pending). It includes deliveries waiting for a retry rung that is not due yet, so it is a size, not a lateness: read it with the overdue age beside it. Orange at 1,000, red at 10,000.",
        (0, 39, 6, 5),
        ['max(vpay_webhook_deliveries_pending{job=~"$job"})'],
        unit="short",
        steps=[(None, "green"), (1000, "orange"), (10000, "red")],
    ),
    stat(
        "Oldest overdue delivery",
        "How long the most overdue pending delivery is past its next attempt; zero when none is due. A value that keeps growing is a worker that is not delivering. Orange at 5 minutes, red at 30.",
        (6, 39, 6, 5),
        ['max(vpay_webhook_oldest_pending_overdue_seconds{job=~"$job"})'],
        unit="s",
        steps=[(None, "green"), (300, "orange"), (1800, "red")],
    ),
    timeseries(
        "Webhook backlog over time",
        "The two backlog numbers together: a burst rises and falls, a stalled worker rises with the overdue age.",
        (12, 39, 12, 8),
        [
            ('max(vpay_webhook_deliveries_pending{job=~"$job"})', "pending deliveries"),
            ('max(vpay_webhook_oldest_pending_overdue_seconds{job=~"$job"})', "oldest overdue (s)"),
        ],
        unit="short",
    ),
    timeseries(
        "Webhook delivery attempts by outcome",
        "Delivery attempts per second as succeeded, retry (the ladder has a rung left) or exhausted (parked for a human, docs/runbooks/webhook-delivery-failures.md).",
        (0, 44, 12, 8),
        [
            (
                'sum by (outcome) (rate(vpay_webhook_deliveries_total{job=~"$job"}' + RATE + "))",
                "{{outcome}}",
            )
        ],
        unit="ops",
        fill=40,
        stacking="normal",
    ),
    stat(
        "Deliveries exhausted in the range",
        "Deliveries whose retry ladder ran out. Each one is a merchant who did not get an event and needs a person: orange at one.",
        (12, 47, 6, 5),
        ['sum(increase(vpay_webhook_deliveries_total{job=~"$job",outcome="exhausted"}[$__range]))'],
        unit="short",
        steps=[(None, "green"), (1, "orange"), (10, "red")],
    ),
    timeseries(
        "Account-holder lookups by outcome",
        "GET /v1/account_holders answers per second: found, not_found, unsupported (no rail could answer) and error (a rail or a malformed number). No label carries the number or the name.",
        (0, 52, 12, 8),
        [
            (
                'sum by (outcome) (rate(vpay_account_holder_lookups_total{job=~"$job"}' + RATE + "))",
                "{{outcome}}",
            )
        ],
        unit="ops",
    ),
    timeseries(
        "Callback endpoint checks by state",
        "What the ensure_callbacks pass found at each rail: present is healthy; created more than once means something deletes the endpoint; drifted, foreign and missing need a person.",
        (12, 52, 12, 8),
        [
            (
                'sum by (provider, state) (rate(vpay_callback_endpoint_checks_total{job=~"$job"}' + RATE + "))",
                "{{provider}} {{state}}",
            )
        ],
        unit="ops",
    ),
]

write(
    "vpay-rails",
    dashboard(
        "vpay-rails",
        "vpay: rails, charges and webhooks",
        "Calls to the payment rails, the charge state machine, provider callbacks and the webhook delivery backlog. Counts calls to whatever host a rail is configured with: it does not say a rail works. Generated by tools/dashboards/rails.py.",
        PANELS,
    ),
)
