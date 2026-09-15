#!/usr/bin/env bash
# w3sdks- helper. BRANCH ASSERTION:
set -euo pipefail
test "$(git rev-parse --abbrev-ref HEAD)" = "refunds/w3-sdks" || { echo "wrong branch"; exit 1; }

export DOCKER_HOST=unix:///run/user/1000/docker.sock
export VPAY_DEMO_PROJECT=vpay-w3sdks
export VPAY_DEMO_PORT=18080
export VPAY_DEMO_RECEIVER_PORT=18083
export VPAY_DEMO_ORANGE_PORT=18082
export VPAY_DEMO_CHECKOUT_PORT=13080
export VPAY_DEMO_SHOP_PORT=13001
export VPAY_DEMO_DASHBOARD_PORT=13000

docker compose -p "$VPAY_DEMO_PROJECT" \
  -f compose.yml -f compose.e2e.yml -f compose.demo.yml \
  up -d --wait --force-recreate --no-build vpay-server vpay-worker

docker inspect vpay-w3sdks-vpay-server-1 --format '{{range .Config.Env}}{{println .}}{{end}}' | grep -i disburse
