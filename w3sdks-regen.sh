#!/usr/bin/env bash
# w3sdks- helper. BRANCH ASSERTION:
set -euo pipefail
test "$(git rev-parse --abbrev-ref HEAD)" = "refunds/w3-sdks" || { echo "wrong branch"; exit 1; }

export DOCKER_HOST=unix:///run/user/1000/docker.sock
export PATH=/home/selast/.nvm/versions/node/v22.23.2/bin:$PATH
export VPAY_DEMO_PROJECT=vpay-w3sdks
export VPAY_DEMO_PORT=18080
export VPAY_DEMO_RECEIVER_PORT=18083
export VPAY_DEMO_ORANGE_PORT=18082
export VPAY_DEMO_CHECKOUT_PORT=13080
export VPAY_DEMO_SHOP_PORT=13001
export VPAY_DEMO_DASHBOARD_PORT=13000

rm -f .e2e/application-demo.yml
just demo_project=$VPAY_DEMO_PROJECT \
     demo_port=$VPAY_DEMO_PORT \
     demo_receiver_port=$VPAY_DEMO_RECEIVER_PORT \
     demo_orange_port=$VPAY_DEMO_ORANGE_PORT \
     demo_checkout_port=$VPAY_DEMO_CHECKOUT_PORT \
     demo_shop_port=$VPAY_DEMO_SHOP_PORT \
     demo_dashboard_port=$VPAY_DEMO_DASHBOARD_PORT \
     gen-demo-keys

grep -n -i disburse .e2e/application-demo.yml

docker compose -p "$VPAY_DEMO_PROJECT" \
  -f compose.yml -f compose.e2e.yml -f compose.demo.yml \
  up -d --wait --force-recreate --no-build vpay-server vpay-worker
