# vpay task runner. `just` with no argument lists everything.
#
# Two invariants this repo enforces on itself, both wired into `just verify`:
#   * no test double is reachable from a shipping binary
#   * every unimplemented item is declared in docs/STATUS.md

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
verify: verify-no-mocks verify-status
    @echo "verify: ok"

verify-no-mocks:
    cargo xtask verify-no-mocks

verify-status:
    cargo xtask verify-status

# Everything CI runs, in CI's order.
ci: fmt-check clippy verify test-rust lint-web test-web deny

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
    @echo "note: link checking is not implemented yet — see docs/STATUS.md"
