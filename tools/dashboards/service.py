#!/usr/bin/env python3
"""Generates deploy/dashboards/vpay-service.json. Standard library only.

Run `python3 tools/dashboards/service.py` (or `sh tools/dashboards/generate.sh`); CI regenerates and fails on any
difference. Every metric is the Prometheus name of a series of `vpay_core::metrics`; a test
(`backends/crates/vpay-telemetry/tests/dashboards.rs`) holds each query to that vocabulary.
"""

from lib import dashboard, row, stat, text, timeseries, write

RATE = "[$__rate_interval]"

PANELS = [
    row("Traffic and errors", y=0),
    timeseries(
        "Request rate by route",
        "Requests per second by route pattern (never the concrete path), the health and metrics listeners left out.",
        (0, 1, 12, 8),
        ['sum by (route) (rate(vpay_http_requests_total{job=~"$job"}' + RATE + "))"],
        unit="reqps",
    ),
    timeseries(
        "Requests by status class",
        "Requests per second as 2xx, 3xx, 4xx and 5xx, stacked.",
        (12, 1, 12, 8),
        [
            (
                'sum by (class) (label_replace(rate(vpay_http_requests_total{job=~"$job"}'
                + RATE
                + '), "class", "${1}xx", "status", "(.).."))',
                "{{class}}",
            )
        ],
        unit="reqps",
        fill=40,
        stacking="normal",
    ),
    stat(
        "5xx ratio",
        "The share of responses that were 5xx, over the range: the service or its database failing the caller.",
        (0, 9, 6, 5),
        [
            'sum(rate(vpay_http_requests_total{job=~"$job",status=~"5.."}' + RATE + "))"
            ' / sum(rate(vpay_http_requests_total{job=~"$job"}' + RATE + "))"
        ],
        unit="percentunit",
        steps=[(None, "green"), (0.001, "orange"), (0.01, "red")],
    ),
    stat(
        "4xx ratio",
        "The share of responses that were 4xx. A jump after a deployment is usually a merchant on an old contract or a credential that expired.",
        (6, 9, 6, 5),
        [
            'sum(rate(vpay_http_requests_total{job=~"$job",status=~"4.."}' + RATE + "))"
            ' / sum(rate(vpay_http_requests_total{job=~"$job"}' + RATE + "))"
        ],
        unit="percentunit",
        steps=[(None, "green"), (0.05, "orange"), (0.2, "red")],
    ),
    timeseries(
        "Classified errors by category, code and severity",
        "Errors the service classified (ADR-0011) per second. A category says whose problem it is: InvalidRequest and Authentication are callers, Rail and Storage are dependencies, Internal is a fault.",
        (12, 9, 12, 8),
        [
            (
                'sum by (category, code, severity) (rate(vpay_error_events_total{job=~"$job"}' + RATE + "))",
                "{{category}} / {{code}} ({{severity}})",
            )
        ],
        unit="ops",
    ),
    stat(
        "Page-severity errors in the range",
        "Errors at the severity that wakes someone. Orange at one: it is the counter an alert on 'a page happened' reads.",
        (0, 14, 6, 3),
        ['sum(increase(vpay_alert_events_total{job=~"$job"}[$__range]))'],
        unit="short",
        steps=[(None, "green"), (1, "orange"), (5, "red")],
    ),
    row("Latency", y=17),
    timeseries(
        "Latency by route (p50, p95, p99)",
        "Response time quantiles over every route: routing, authentication, the handler and the error renderer.",
        (0, 18, 12, 8),
        [
            (
                'histogram_quantile(%s, sum by (le) (rate(vpay_http_request_duration_seconds_bucket{job=~"$job"}%s)))'
                % (q, RATE),
                label,
            )
            for q, label in ((0.5, "p50"), (0.95, "p95"), (0.99, "p99"))
        ],
        unit="s",
    ),
    timeseries(
        "p95 latency by route",
        "The 95th percentile of each route: where a slow call is. A route with few requests has a noisy quantile.",
        (12, 18, 12, 8),
        [
            'histogram_quantile(0.95, sum by (le, route) (rate(vpay_http_request_duration_seconds_bucket{job=~"$job"}'
            + RATE
            + ")))"
        ],
        unit="s",
    ),
    row("Build", y=26),
    stat(
        "Running builds",
        "How many processes report each version and git sha. More than one row during a rollout is expected; a row that stays after it is a pod that did not restart.",
        (0, 27, 12, 4),
        [('count by (version, git_sha) (vpay_build_info{job=~"$job"})', "{{version}} {{git_sha}}")],
        unit="short",
        steps=[(None, "blue")],
    ),
    text(
        "Reading this dashboard",
        "These panels come from the metrics the service exports over OpenTelemetry. A metric exists only for what the code "
        "has recorded: **a panel that is empty means no such request has happened, not that it is healthy.** "
        "Which parts of vpay run against a real rail and which against a stub is in `docs/status.md`, not here.",
        (12, 27, 12, 4),
    ),
]

write(
    "vpay-service",
    dashboard(
        "vpay-service",
        "vpay: service",
        "The HTTP API: traffic, errors, latency, classified errors and the running builds. Generated by tools/dashboards/service.py.",
        PANELS,
    ),
)
