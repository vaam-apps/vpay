#!/usr/bin/env bash
# w3sdks- helper. BRANCH ASSERTION:
set -uo pipefail
test "$(git rev-parse --abbrev-ref HEAD)" = "refunds/w3-sdks" || { echo "wrong branch"; exit 1; }

export DOCKER_HOST=unix:///run/user/1000/docker.sock
export PATH=/home/selast/.nvm/versions/node/v22.23.2/bin:$PATH
export VPAY_BASE_URL=http://localhost:18080
export VPAY_MERCHANT_CLIENT_ID=demo-merchant
export VPAY_MERCHANT_PRIVATE_KEY_PATH="$PWD/.e2e/demo-merchant/oauth-signing-key.pem"

cargo nextest run -p vpay-sdk --features live-stack --test live_invoices --test live_refunds
rust=$?
( cd sdks/nodejs && pnpm run test:live )
node=$?

echo
echo "w3sdks-live: rust exit $rust, node exit $node"
echo "$rust $node" > w3sdks-live-exit.txt
