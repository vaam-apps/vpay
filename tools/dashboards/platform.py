#!/usr/bin/env python3
"""Generates deploy/dashboards/vpay-platform.json: the job queue, PostgreSQL as the service sees it, and the exporter.
Standard library only.

Run `sh tools/dashboards/generate.sh`; CI regenerates and fails on any difference. Every metric is a series of
`vpay_core::metrics`, held to that vocabulary by `backends/crates/vpay-telemetry/tests/dashboards.rs`.
"""

from lib import dashboard, row, stat, text, timeseries, write

RATE = "[$__rate_interval]"

PANELS = [
    row("Job queue", y=0),
    stat(
        "Queue behind by",
        "Seconds since the next queued job was due, over the whole table. Negative is normal on an idle deployment (the next job is in the future); the panel is clamped at zero for reading. A value that grows is workers not claiming. Orange at 60 s, red at 5 minutes.",
        (0, 1, 6, 5),
        ['clamp_min(max(vpay_jobs_oldest_claimable_age_seconds{job=~"$job"}), 0)'],
        unit="s",
        steps=[(None, "green"), (60, "orange"), (300, "red")],
    ),
    timeseries(
        "Queue age over time",
        "The raw gauge, negative values included: the line sits below zero when the queue is idle and rises above it when jobs are due and unclaimed.",
        (6, 1, 18, 5),
        ['max(vpay_jobs_oldest_claimable_age_seconds{job=~"$job"})'],
        unit="s",
    ),
    timeseries(
        "Jobs claimed by kind",
        "Jobs this fleet claimed per second, by kind: the shape of the work (polls, deliveries, fan-out, sweeps).",
        (0, 6, 12, 8),
        [
            (
                'sum by (kind) (rate(vpay_jobs_claimed_total{job=~"$job"}' + RATE + "))",
                "{{kind}}",
            )
        ],
        unit="ops",
    ),
    timeseries(
        "Jobs completed by kind and outcome",
        "How claimed jobs ended: terminal (done), retry, dead_letter (parked for a human) or lost (the lease moved before the worker could settle).",
        (12, 6, 12, 8),
        [
            (
                'sum by (kind, outcome) (rate(vpay_jobs_completed_total{job=~"$job"}' + RATE + "))",
                "{{kind}} {{outcome}}",
            )
        ],
        unit="ops",
    ),
    stat(
        "Dead-lettered jobs in the range",
        "Jobs parked at run_at infinity for a human. Orange at one: each is work the system gave up on.",
        (0, 14, 6, 4),
        ['sum(increase(vpay_jobs_completed_total{job=~"$job",outcome="dead_letter"}[$__range]))'],
        unit="short",
        steps=[(None, "green"), (1, "orange"), (10, "red")],
    ),
    row("PostgreSQL as the service sees it", y=18),
    timeseries(
        "Connection pool by process",
        "Connections of each process's pool, idle and in use, against the configured maximum. A process at its maximum makes requests and jobs wait for a connection.",
        (0, 19, 12, 8),
        [
            ('sum by (instance, state) (vpay_db_pool_connections{job=~"$job"})', "{{instance}} {{state}}"),
            ('max by (instance) (vpay_db_pool_max_connections{job=~"$job"})', "{{instance}} maximum"),
        ],
        unit="short",
    ),
    stat(
        "Busiest pool in use",
        "The largest share of any process's pool checked out right now. Orange at 70%, red at 90%: raise the maximum or add a replica before callers queue. The server's own load (locks, replication, disk) is PostgreSQL's exporter, not this service's.",
        (12, 19, 6, 8),
        [
            'max(sum by (instance) (vpay_db_pool_connections{job=~"$job",state="in_use"})'
            ' / max by (instance) (vpay_db_pool_max_connections{job=~"$job"}))'
        ],
        unit="percentunit",
        steps=[(None, "green"), (0.7, "orange"), (0.9, "red")],
    ),
    stat(
        "Storage errors in the range",
        "Classified errors of category Storage (PostgreSQL unreachable, a query failed, a migration broke). Orange at one.",
        (18, 19, 6, 8),
        ['sum(increase(vpay_error_events_total{job=~"$job",category="Storage"}[$__range]))'],
        unit="short",
        steps=[(None, "green"), (1, "orange"), (10, "red")],
    ),
    row("The exporter itself", y=27),
    timeseries(
        "Telemetry exports by signal and result",
        "Export requests per second to the collector, by signal and result. failed is a collector that refuses or does not answer after retries: look at its address, its credential header and the network policy. This series does not exist when telemetry is off.",
        (0, 28, 12, 8),
        [
            (
                'sum by (signal, result) (rate(vpay_telemetry_exports_total{job=~"$job"}' + RATE + "))",
                "{{signal}} {{result}}",
            )
        ],
        unit="ops",
    ),
    timeseries(
        "Telemetry dropped by signal",
        "Spans, logs and metric series the exporter dropped (a full bounded queue, or the cap on distinct series). The exporter never blocks a request: it drops. A band here says the other panels are incomplete.",
        (12, 28, 12, 8),
        [
            (
                'sum by (signal) (rate(vpay_telemetry_dropped_total{job=~"$job"}' + RATE + "))",
                "{{signal}}",
            )
        ],
        unit="ops",
    ),
    text(
        "What is not here",
        "PostgreSQL's own metrics (replication lag, locks, disk, cache hit ratio) come from the database's exporter or "
        "its operator, not from this service. The pool panels show the service's view: the connections it holds.",
        (0, 36, 24, 3),
    ),
]

write(
    "vpay-platform",
    dashboard(
        "vpay-platform",
        "vpay: job queue and database",
        "The job queue, the database pool as the service sees it, and the OpenTelemetry exporter's own counters. Generated by tools/dashboards/platform.py.",
        PANELS,
    ),
)
