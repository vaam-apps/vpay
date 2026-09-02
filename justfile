# vpay task runner. `just` with no argument lists everything.
#
# Three invariants this repo enforces on itself, all wired into `just verify`:
#   * no test double is reachable from a shipping binary
#   * every unimplemented item is declared in docs/status.md
#   * every error type is classified (ADR-0011) and anyhow stays in the binaries

set shell := ["bash", "-uc"]

export CYPRESS_INSTALL_BINARY := env_var_or_default("CYPRESS_INSTALL_BINARY", "")

default:
    @just --list

# ---------------------------------------------------------------- setup ----

install: install-rust install-node
    @echo "toolchains ready"

install-rust:
    rustup show
    cargo install cargo-nextest --locked || true
    cargo install cargo-deny --locked || true

install-node:
    corepack enable
    pnpm install

# ----------------------------------------------------------------- build ---

build: build-rust build-web

build-rust:
    cargo build --workspace

build-web:
    pnpm -r build

# CI's `web` job builds this too; not part of `pnpm -r build`.
build-storybook:
    pnpm --filter @vpay/ui build-storybook

# musl static binaries, as shipped
build-dist:
    cargo build --profile dist --target x86_64-unknown-linux-musl -p vpay-server -p vpay-worker-bin

# ------------------------------------------------------------------ test ---

test: test-rust test-web

test-rust:
    cargo nextest run --workspace

# Includes the cases that are #[ignore]d because they are not implemented.
# Expect failures; this is for seeing what is NOT covered, not for CI.
test-rust-all:
    cargo nextest run --workspace --run-ignored all

test-web:
    pnpm -r test

# `test-rust` above already covers this (it runs --workspace, which now
# includes sdks/rust); this recipe exists to scope a run to just the SDK
# while iterating on it.
test-sdk-rust:
    cargo nextest run -p vpay-sdk

test-sdk-node:
    pnpm --filter @vpay/sdk test

build-sdk-node:
    pnpm --filter @vpay/sdk build

# Cross-language conformance: mint a private_key_jwt assertion with the Node
# SDK and verify it with the REAL OP verifier vpay will run
# (authkestra_op::client_assertion::verify_client_assertion at the pinned
# version, wrapped by sdks/rust/examples/verify_assertion.rs). The Rust SDK
# has this as a test; the Node SDK cannot link the Rust crate, so this
# recipe is the bridge. Not part of `just ci` — run it by hand, and record
# the outcome in docs/status.md when you do.
sdk-conformance-node: build-sdk-node
    set -o pipefail; \
      tmp=$(mktemp -d);       node -e 'const {generateKeyPairSync}=require("node:crypto"); const {privateKey}=generateKeyPairSync("rsa",{modulusLength:2048}); process.stdout.write(privateKey.export({type:"pkcs8",format:"pem"}))' > "$tmp/key.pem";       VPAY_CLIENT_ID=merchant_a VPAY_PRIVATE_KEY_FILE="$tmp/key.pem" VPAY_KID=k1         VPAY_AUDIENCE=https://api.vpay.example/v1/oauth/token         node sdks/nodejs/scripts/mint-assertion.mjs       | cargo run -q -p vpay-sdk --example verify_assertion -- - merchant_a           https://api.vpay.example/v1/oauth/token https://api.vpay.example/v1/oauth;       status=$?; rm -rf "$tmp"; exit $status

test-e2e:
    docker compose -f compose.yml -f compose.e2e.yml up -d --build
    pnpm --filter @vpay/e2e e2e; \
      e2e_status=$?; \
      docker compose -f compose.yml -f compose.e2e.yml down -v; \
      exit $e2e_status

# ------------------------------------------------------------------ lint ---

lint: fmt-check clippy lint-web

fmt:
    cargo fmt --all
    pnpm exec prettier --write .

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

lint-web:
    pnpm -r typecheck

deny:
    cargo deny check

# ---------------------------------------------------- self-verification ----

# The checks that keep this repository honest. CI runs exactly this.
verify: verify-no-mocks verify-status verify-errors
    @echo "verify: ok"

verify-no-mocks:
    cargo xtask verify-no-mocks

verify-status:
    cargo xtask verify-status

# ADR-0011: every pub error type in backends/crates implements
# vpay_core::error::Classify, and anyhow stays in backends/apps.
verify-errors:
    cargo xtask verify-errors

# Pins how much of the suite is `#[ignore]`d, how many test binaries exist,
# and how big the suite is, so a new `#[ignore]` cannot quietly shrink
# coverage and a test binary dropping out of the workspace cannot read as
# "fewer tests, still green". The binary count is the one that actually
# catches a dropped binary: 18 of the 30 hold eight tests or fewer, so a
# global floor alone would let any one of them vanish unnoticed. All three
# numbers are deliberate and must be bumped in the same commit that
# legitimately changes them — the three ignored tests today are the three
# not-implemented conformance cases listed in docs/status.md.
expected_ignored := "3"
expected_suites := "30"
min_tests := "320"

verify-ignored:
    #!/usr/bin/env bash
    set -euo pipefail
    ignored=$(cargo nextest list --workspace --run-ignored ignored-only --message-format json \
        | jq '[."rust-suites"[]."testcases" | to_entries[] | select(.value.ignored)] | length')
    listing=$(cargo nextest list --workspace --message-format json)
    total=$(printf '%s' "$listing" | jq '[."rust-suites"[]."testcases" | to_entries[]] | length')
    suites=$(printf '%s' "$listing" | jq '."rust-suites" | length')
    echo "verify-ignored: $ignored ignored (expected {{expected_ignored}}), $suites test binaries (expected {{expected_suites}}), $total total (minimum {{min_tests}})"
    if [ "$suites" -ne "{{expected_suites}}" ]; then
        echo "verify-ignored: FAIL — $suites test binaries listed, expected {{expected_suites}}; a test binary was added or dropped out of the workspace" >&2
        printf '%s' "$listing" | jq -r '."rust-suites" | keys[]' >&2
        exit 1
    fi
    if [ "$ignored" -ne "{{expected_ignored}}" ]; then
        echo "verify-ignored: FAIL — expected exactly {{expected_ignored}} ignored tests; update docs/status.md and this recipe together" >&2
        cargo nextest list --workspace --run-ignored ignored-only >&2
        exit 1
    fi
    if [ "$total" -lt "{{min_tests}}" ]; then
        echo "verify-ignored: FAIL — only $total tests listed, fewer than the {{min_tests}} floor; did a test binary drop out of the workspace?" >&2
        exit 1
    fi

# Everything CI runs, in CI's order.
ci: fmt-check clippy verify test-rust verify-ignored lint-web test-web deny

# -------------------------------------------------------------- dev loop ---

up:
    docker compose up -d
    @echo "postgres :5432 · wiremock-mtn :8081 · wiremock-orange :8082"

down:
    docker compose down -v

storybook:
    pnpm --filter @vpay/ui storybook

dev-dashboard:
    pnpm --filter @vpay/dashboard dev

# ------------------------------------------------------------------ docs ---

# Fail if a doc links to a file that does not exist.
docs-check:
    cargo xtask verify-status
    @echo "note: link checking is not implemented yet — see docs/status.md"
