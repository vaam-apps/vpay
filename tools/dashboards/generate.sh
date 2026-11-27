#!/bin/sh
# Regenerates every dashboard from its script into deploy/dashboards/. CI runs this and then fails on any difference
# or untracked file under deploy/dashboards: a dashboard is edited in tools/dashboards/, never in the JSON.
set -eu
cd "$(dirname "$0")"
for script in service.py rails.py platform.py; do
  python3 -B "$script"
done
python3 -B readme.py
