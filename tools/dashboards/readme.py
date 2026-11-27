#!/usr/bin/env python3
"""Writes deploy/dashboards/README.md from the dashboards the other scripts wrote. Standard library only.

The panel tables are read back from the JSON, so the README cannot name a panel that does not exist or miss one that
does (a test, `backends/crates/vpay-telemetry/tests/dashboards.rs`, holds both). Run it after the others:
`sh tools/dashboards/generate.sh` does.
"""

import json
from pathlib import Path

from lib import ROOT

DIRECTORY = ROOT / "deploy" / "dashboards"

ORDER = [
    ("vpay-service.json", "For the HTTP API: traffic, errors, latency and the running builds"),
    ("vpay-rails.json", "For payment rails, the charge state machine, callbacks and the webhook backlog"),
    ("vpay-platform.json", "For the job queue, the database pool and the exporter itself"),
]

INTRO = """# Dashboards

Three Grafana dashboards for the operators of vpay, as code: the JSON in this folder is the dashboard, and **it is
generated** by the Python scripts in [`tools/dashboards/`](../../tools/dashboards/) (`sh tools/dashboards/generate.sh`;
standard library only). Edit the script, never the JSON or this page: CI regenerates every file and fails on any
difference or untracked file. A panel is one call of `timeseries`, `stat` or `text`.

They read the metrics the services export over OpenTelemetry ([`vpay-telemetry`](../../backends/crates/vpay-telemetry/README.md),
[ADR-0039](../../docs/adr/0039-opentelemetry-through-the-sdk-and-generated-dashboards.md)) once a collector has put them in a
Prometheus-compatible store (Grafana's Prometheus data source, an OpenObserve instance's PromQL endpoint, Mimir,
VictoriaMetrics, and so on). The names are `vpay_core::metrics`'s own, so the Prometheus listener (`/metrics`) and these
dashboards read one vocabulary.

## What these dashboards do not claim

**A panel counts what the code recorded, nothing more.** The rail panels count calls vpay made to the host a rail is
configured with. A rail configured against a stub host reports like a live one, and a rail whose adapter is not written
reports its `not_implemented` error. Which rails have been called live, and when, is in
[`docs/status.md`](../../docs/status.md), and this folder never says a rail works. An empty panel means no such event
happened, not that it is healthy.

| File | Dashboard | For |
|---|---|---|
{files}

## Importing

Grafana 10 or later. In Grafana: *Dashboards, New, Import*, upload each file, and pick the Prometheus data source. Or
from a shell, with a service account token in `GRAFANA_TOKEN` (never in a file of this repository):

```sh
for f in deploy/dashboards/*.json; do
  curl -sS -X POST "$GRAFANA_URL/api/dashboards/db" \\
    -H "Authorization: Bearer $GRAFANA_TOKEN" -H 'Content-Type: application/json' \\
    -d "{{\\"dashboard\\": $(cat "$f"), \\"overwrite\\": true}}"
done
```

Every dashboard has a **Data source** variable and a **Service** variable (the `job` label, which a collector sets from
`service.name`; `vpay` unless `OTEL_SERVICE_NAME` says otherwise). Names are the Prometheus names: a collector turns a
histogram `vpay_http_request_duration_seconds` into `_bucket`, `_count` and `_sum`. **Unverified:** that the deployment's
collector and store apply exactly this naming (it is the OpenTelemetry to Prometheus translation as specified, and the
vocabulary's units are chosen so it appends no suffix of its own); a store with another scheme needs the queries
adjusted. **Unverified:** that Grafana imports these files on the version a deployment runs; they use only `timeseries`,
`stat` and `text` panels and the Prometheus data source.

{sections}
## What to alert on

Alert rules are the operator's, and none is shipped. Starting points from the thresholds above: `Queue behind by` over
300 s; `Oldest overdue delivery` over 1,800 s; `5xx ratio` over 1% for 10 minutes; `Busiest pool in use` over 90% for
10 minutes; `Page-severity errors in the range` above zero; `Deliveries exhausted in the range` above zero.
"""


def main():
    files = []
    sections = []
    for name, purpose in ORDER:
        document = json.loads((DIRECTORY / name).read_text(encoding="utf-8"))
        files.append(f"| [`{name}`]({name}) | {document['title']} | {purpose} |")
        rows = [
            f"| `{p['title']}` | {p['options']['content'] if p['type'] == 'text' else p['description']} |"
            for p in document["panels"]
            if p["type"] not in ("row",)
        ]
        sections.append(
            f"## {document['title']}\n\n{document['description']}\n\n| Panel | What it watches, and what to do |\n|---|---|\n"
            + "\n".join(rows)
            + "\n"
        )
    text = INTRO.format(files="\n".join(files), sections="\n".join(sections))
    (DIRECTORY / "README.md").write_text(text, encoding="utf-8")


main()
