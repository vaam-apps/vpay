"""Shared builders for the Grafana dashboards of vpay. Standard library only.

A dashboard is code: `timeseries` and `stat` take the panel's title, its description, its place on the 24-column
grid and its queries; the JSON keys Grafana wants are written here once. `write` puts the file in
`deploy/dashboards/`.
"""

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUTPUTS = [ROOT / "deploy" / "dashboards"]

DATASOURCE = {"type": "prometheus", "uid": "${datasource}"}

# Panel ids are numbered in the order the panels are given, so a dashboard is only its list.
_ids = iter(range(1, 10_000))


def row(title, y):
    return {
        "id": next(_ids),
        "type": "row",
        "title": title,
        "collapsed": False,
        "gridPos": {"x": 0, "y": y, "w": 24, "h": 1},
        "panels": [],
    }


def text(title, content, place):
    """A note: no query, and it says what the panels around it do not (and what they must not be read as)."""
    x, y, w, h = place
    return {
        "id": next(_ids),
        "type": "text",
        "title": title,
        "description": "A note for the reader of this dashboard.",
        "gridPos": {"x": x, "y": y, "w": w, "h": h},
        "options": {"mode": "markdown", "content": content},
    }


def _targets(queries):
    out = []
    for index, query in enumerate(queries):
        expr, legend = (query, "__auto") if isinstance(query, str) else query
        out.append(
            {
                "refId": chr(ord("A") + index),
                "expr": expr,
                "legendFormat": legend,
                "datasource": DATASOURCE,
            }
        )
    return out


def _panel(kind, title, description, place, queries, field_config, options):
    x, y, w, h = place
    return {
        "id": next(_ids),
        "type": kind,
        "title": title,
        "description": description,
        "datasource": DATASOURCE,
        "gridPos": {"x": x, "y": y, "w": w, "h": h},
        "targets": _targets(queries),
        "fieldConfig": field_config,
        "options": options,
    }


def timeseries(title, description, place, queries, unit, fill=10, stacking="none"):
    field_config = {
        "defaults": {
            "unit": unit,
            "custom": {"lineWidth": 1, "fillOpacity": fill, "stacking": {"mode": stacking}},
        },
        "overrides": [],
    }
    options = {
        "legend": {"displayMode": "list", "placement": "bottom"},
        "tooltip": {"mode": "multi"},
    }
    return _panel("timeseries", title, description, place, queries, field_config, options)


def stat(title, description, place, queries, unit, steps, minimum=None):
    """`steps` are (value, color) pairs; the first value is None (the base)."""
    defaults = {"unit": unit}
    if minimum is not None:
        defaults["min"] = minimum
    defaults["thresholds"] = {
        "mode": "absolute",
        "steps": [{"color": color, "value": value} for value, color in steps],
    }
    defaults["color"] = {"mode": "thresholds"}
    field_config = {"defaults": defaults, "overrides": []}
    options = {
        "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
        "colorMode": "background",
        "graphMode": "area",
    }
    return _panel("stat", title, description, place, queries, field_config, options)


def dashboard(uid, title, description, panels):
    return {
        "uid": uid,
        "title": title,
        "description": description,
        "tags": ["vpay"],
        "schemaVersion": 39,
        "version": 1,
        "editable": True,
        "timezone": "utc",
        "time": {"from": "now-6h", "to": "now"},
        "refresh": "30s",
        "templating": {
            "list": [
                {
                    "name": "datasource",
                    "label": "Data source",
                    "type": "datasource",
                    "query": "prometheus",
                    "current": {},
                    "hide": 0,
                },
                {
                    "name": "job",
                    "label": "Service",
                    "type": "query",
                    "datasource": DATASOURCE,
                    "query": {
                        "query": "label_values(vpay_build_info, job)",
                        "refId": "job",
                    },
                    "definition": "label_values(vpay_build_info, job)",
                    "refresh": 2,
                    "includeAll": True,
                    "multi": True,
                    "current": {},
                },
            ]
        },
        "panels": panels,
    }


def write(name, document):
    text = json.dumps(document, indent=2, ensure_ascii=False) + "\n"
    for directory in OUTPUTS:
        directory.mkdir(parents=True, exist_ok=True)
        (directory / f"{name}.json").write_text(text, encoding="utf-8")
