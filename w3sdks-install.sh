#!/usr/bin/env bash
# w3sdks- helper. BRANCH ASSERTION:
set -euo pipefail
test "$(git rev-parse --abbrev-ref HEAD)" = "refunds/w3-sdks" || { echo "wrong branch"; exit 1; }
export PATH=/home/selast/.nvm/versions/node/v22.23.2/bin:$PATH
export CYPRESS_INSTALL_BINARY=0
node --version
pnpm install --filter @vaam-apps/vpay-sdk... --frozen-lockfile
