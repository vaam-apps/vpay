# vpay task runner. `just` with no argument lists everything.
#
# Ten invariants this repo enforces on itself, all wired into `just verify`:
#   * no test double is reachable from a shipping binary
#   * every unimplemented item is declared in docs/status.md
#   * every error type is classified (ADR-0011) and anyhow stays in the binaries
#   * the merchant SDKs stay at parity (ADR-0015): every claimed capability
#     names a test that exists, every gap is dated and owned
#   * every relative link in a tracked *.md resolves to a tracked path
#   * every publishable npm package under sdks/ is named @vaam-apps/vpay-*
#     and is publishable for real (`verify-npm-scope`, 2026-09-05)
#   * schemas/vpay.cstack parses and type-checks against the real CrateStack
#     grammar, at the pinned CLI version (`check-schema`, 2026-09-05)
#   * every serialisable type in backends/crates carries
#     #[serde(rename_all = "snake_case")], renames every field itself, or is
#     exempted with a reason in ADR-0016's table (`verify-serde`, 2026-09-05)
#   * nothing outside vpay-db names a concrete repository implementation
#     (`verify-repositories`, 2026-09-05)
#   * backends/Dockerfile's `FROM rust:<version>` names the compiler
#     rust-toolchain.toml pins (`verify-toolchain`, 2026-09-05)
#
# `just verify` prints an eleventh thing that is NOT an invariant and never
# fails the build: `verify-docs`, a report on doc-comment volume, in-file
# comment volume, externalised module docs, long functions, ```ignore fences
# and #[allow]s (Step 7 decision 4; ADR-0016 standard 6 keeps it a report).
#
# A twelfth check is a gate that is NOT in `just ci`, because it needs the
# network: `just docs-check-citations` resolves every run id, PR and issue a
# document cites against GitHub. See its recipe at the bottom of this file.

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

# The Node baseline lives in `.nvmrc` (`22.23.2`, the current 22 LTS) and is
# enforced, not merely suggested: `.npmrc` sets `engine-strict=true`, the root
# `package.json` declares `engines.node: >=22.13.0`, and CI's `web`, `rust` and
# `e2e` jobs all install Node with `node-version-file: .nvmrc`. It moved from
# `22.11.0` to `22.x` on 2026-09-05 because the ESLint 9.39 dependency tree
# (`eslint-visitor-keys@5.0.1`) requires `^20.19.0 || ^22.13.0 || >=24`, which
# `22.11.0` does not satisfy — `pnpm install --frozen-lockfile` exits 1 under
# it. `frontends/Dockerfile` and `examples/shop/Dockerfile` build on
# `node:22-alpine`, a floating tag with no minor pin, which resolves to the
# same 22 LTS; nothing there needs to change when this line does.
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

test: test-rust test-doc test-web

test-rust:
    cargo nextest run --workspace

# Includes the cases that are #[ignore]d because they are not implemented.
# Expect failures; this is for seeing what is NOT covered, not for CI.
test-rust-all:
    cargo nextest run --workspace --run-ignored all

# Doctests — a SECOND test runner, not a flag on the first one.
#
# `cargo nextest` does not run doctests at all, so from this repository's
# first commit until 2026-09-03 (Step 7) neither `just test-rust` nor CI's
# `rust` job ever compiled one. There was exactly one to miss
# (`vpay_core::money`), which is the point: an example in a doc comment that
# nothing compiles is a claim about the code that nothing checks, and this
# repo's whole discipline is that a claim nobody checks decays. `just ci` and
# CI's `rust` job both run this recipe now, and `just test` above depends on
# it so that "I ran the tests" means the doctests too.
#
# `--workspace`, so `sdks/rust` and `.xtask` are covered as well as
# `backends/`. It compiles the workspace a second time under rustdoc, which
# is why it is its own recipe and its own CI step: a doctest failure should
# be reported by the step whose job it is.
#
# Run the workspace's doctests (nextest does not).
test-doc:
    cargo test --doc --workspace

test-web:
    pnpm -r test

# `test-rust` above already covers this (it runs --workspace, which now
# includes sdks/rust); this recipe exists to scope a run to just the SDK
# while iterating on it.
test-sdk-rust:
    cargo nextest run -p vpay-sdk

test-sdk-node:
    pnpm --filter @vaam-apps/vpay-sdk test

build-sdk-node:
    pnpm --filter @vaam-apps/vpay-sdk build

test-sdk-browser:
    pnpm --filter @vaam-apps/vpay-stripe-js test

# `@vaam-apps/vpay-stripe-js` is browser ESM: nothing in the workspace imports it as a
# TypeScript source, so `lint-web` does not need it built the way it needs
# `build-sdk-node`. The static checkout example loads `dist/index.js`
# directly, which is what this recipe is for.
build-sdk-browser:
    pnpm --filter @vaam-apps/vpay-stripe-js build

# Vendors `@vaam-apps/vpay-stripe-js`'s build output into
# `examples/checkout-browser/dist/stripe-js/`, which its `index.html` imports
# as a plain relative ESM path (no bundler, no import map). A COPY rather
# than a symlink: the compose/CI environment that eventually serves this
# directory should not need `sdks/` on the same filesystem, and a symlink
# would silently stop working the day that stops being true. The whole
# `dist/index.js` module graph is needed, not just that one file — it
# imports `./client.js`, `./errors.js` and `./types.js` by relative
# specifier, so copying only `index.js` (as this recipe used to be
# documented as doing) would 404 in the browser on the first import.
#
# `examples/checkout-browser/dist/` is covered by the repo-wide `dist/`
# .gitignore entry, same as every other package's build output — nothing new
# to ignore.
build-checkout-browser: build-sdk-browser
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf examples/checkout-browser/dist/stripe-js
    mkdir -p examples/checkout-browser/dist/stripe-js
    cp sdks/stripe-js/dist/*.js sdks/stripe-js/dist/*.d.ts examples/checkout-browser/dist/stripe-js/
    echo "build-checkout-browser: vendored sdks/stripe-js/dist/ into examples/checkout-browser/dist/stripe-js/"

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

# Cypress against the real stack, from nothing.
#
# CHANGED IN STEP 9 (lane 6), and the change is a correction rather than an
# extension: this recipe used to bring up `compose.yml -f compose.e2e.yml`
# only, which has no registered merchant anybody holds a private key for
# (`config/application.yml`'s `acme-cameroon` carries a placeholder modulus).
# Every spec that mints anything — `checkout.cy.ts` since Step 5c, both shop
# specs now — failed there with `invalid_client`. CI's `e2e` job has always
# added `-f compose.demo.yml` and run `gen-demo-keys` first; this recipe now
# does what that job does, so "green in CI" and "green locally" mean the same
# run.
#
# The three build steps are prerequisites rather than lines in the body so
# that a failure in one names itself. `gen-demo-keys` writes the throwaway key
# pairs and the profile overlay; `build-sdk-node` makes `@vaam-apps/vpay-sdk`'s gitignored
# `dist/` exist for `cy.task('mintCheckoutPaymentIntent')`; `build-checkout-browser`
# vendors `@vaam-apps/vpay-stripe-js`'s into `examples/checkout-browser/`.
#
# Every service, not `demo_services`: the dashboard is in the file set and
# `dashboard.cy.ts` visits it.
test-e2e: gen-demo-keys build-sdk-node build-checkout-browser
    #!/usr/bin/env bash
    set -uo pipefail
    export VPAY_DEMO_PROJECT={{demo_project}}
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    export VPAY_DEMO_ORANGE_PORT={{demo_orange_port}}
    export VPAY_DEMO_CHECKOUT_PORT={{demo_checkout_port}}
    export VPAY_DEMO_SHOP_PORT={{demo_shop_port}}

    set -e
    docker compose {{demo_compose}} up -d --build --wait
    set +e

    # `vpay-server` is `FROM scratch` and can carry no healthcheck (see
    # compose.e2e.yml), so `--wait` above reports it merely *started*.
    # Readiness is observed from outside, exactly as CI's e2e job does it.
    echo "test-e2e: waiting for http://localhost:{{demo_port}}/healthz"
    deadline=$((SECONDS + 120))
    until curl -fsS -o /dev/null http://localhost:{{demo_port}}/healthz; do
        if [ "$SECONDS" -ge "$deadline" ]; then
            echo "test-e2e: FAIL — /healthz did not answer within 120s" >&2
            docker compose {{demo_compose}} ps >&2
            docker compose {{demo_compose}} logs --tail 80 vpay-server >&2
            docker compose {{demo_compose}} down -v
            exit 1
        fi
        sleep 2
    done

    # The three browser surfaces the specs visit. Polled rather than assumed:
    # a Cypress failure on "cannot verify this server is running" says nothing
    # about which of them is missing.
    for probe in "dashboard http://localhost:3000/" \
                 "shop http://localhost:{{demo_shop_port}}/healthz" \
                 "checkout http://localhost:{{demo_checkout_port}}/healthz"; do
        name=${probe%% *}; url=${probe##* }
        echo "test-e2e: waiting for $name on $url"
        deadline=$((SECONDS + 120))
        until curl -fsS -o /dev/null "$url"; do
            if [ "$SECONDS" -ge "$deadline" ]; then
                echo "test-e2e: FAIL — $name never answered $url" >&2
                docker compose {{demo_compose}} logs --tail 60 >&2
                docker compose {{demo_compose}} down -v
                exit 1
            fi
            sleep 2
        done
    done

    # What the specs need, all of it a published host port or a public key.
    # No merchant credential is exported here: `checkoutTasks.ts` reads the
    # PEM from `.e2e/` in Node, and nothing hands one to a browser.
    VPAY_BASE_URL=http://localhost:{{demo_port}} \
      VPAY_SHOP_URL=http://localhost:{{demo_shop_port}} \
      VPAY_CHECKOUT_URL=http://localhost:{{demo_checkout_port}} \
      VPAY_ORANGE_STUB_URL=http://localhost:{{demo_orange_port}} \
      VPAY_MERCHANT_CLIENT_ID=demo-merchant \
      VPAY_MERCHANT_PRIVATE_KEY_PATH="$PWD/.e2e/demo-merchant/oauth-signing-key.pem" \
      pnpm --filter @vpay/e2e e2e
    e2e_status=$?

    docker compose {{demo_compose}} down -v
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

# Depends on `build-sdk-node` because the typecheck does: `sdks/stripe-compat`
# imports `@vaam-apps/vpay-sdk/stripe`, whose types resolve through that package's
# `exports` map to `dist/stripe-auth.d.ts`, and `dist/` is gitignored. Without
# the build this recipe fails on a clean checkout with `TS2307: Cannot find
# module '@vaam-apps/vpay-sdk/stripe'` — a missing artefact reported as a broken import.
#
# WHAT IS LINTED, AND WHAT IS NOT. Until 2026-09-05 this recipe ran the
# typecheck alone, and `pnpm -r lint` was broken repo-wide: four packages
# declared a `lint` script for a tool no package had installed, so the sweep
# died on the first of them (`@vpay/tokens`, `eslint: not found`) before a
# single rule ran, and the `web` gate claimed a lint it did not perform.
#
# It now runs both, and `pnpm -r lint` is real: ESLint 9.39.5 in flat config,
# one shared rule set exported from `@vpay/config/eslint`, over **all 15**
# TypeScript/JavaScript workspace packages — 214 files, every `.ts`, `.tsx`,
# `.js` and `.mjs` in the tree that is not build output. Each package's
# `lint` script carries `--max-warnings 0`, so a rule configured at `warn`
# cannot pass this gate quietly. The rules: `@eslint/js` recommended — on
# TypeScript as well as `.js`, which is a composition order the config file
# spells out and which the first version of it got wrong,
# `typescript-eslint` recommended-**type-checked** (a real type checker, from
# each package's own tsconfig — so `strict`, `noUncheckedIndexedAccess` and
# `exactOptionalPropertyTypes` are what the rules reason about),
# `eslint-plugin-react-hooks` on the four React packages,
# `@next/eslint-plugin-next` on the three Next apps, `no-console` as an error
# in shipping source, and `no-restricted-imports` refusing `testing/**` from
# shipping code in `frontends/apps/checkout` and `examples/shop` — a second
# lock on the hand-written vitest guards those two already carry.
#
# WHAT KEEPS THAT TRUE. A lint gate reports its own absence as success, so
# `frontends/packages/config/src/eslint.test.js` (run by `just test-web`)
# asserts the gate is still a gate: every package `git ls-files` finds still
# declares `eslint . --max-warnings 0` and still reaches the shared factory,
# each rule family above is still on for the files it is claimed for, and no
# file carries a whole-file disable directive or a suppression with no stated
# reason. Every assertion in it was written against a mutation that had been
# measured to leave `pnpm -r lint` at exit 0 — a deleted `lint` script (pnpm
# skips a missing script in silence, which is how ten packages went unlinted
# before this), an `eslint.config.js` rewritten to `export default []`, a
# dropped `--max-warnings 0`, and a deleted rule block.
#
# NOT linted, and each for a stated reason: build output (`dist/`, `.next/`,
# `storybook-static/`) and the `next-env.d.ts` Next regenerates, because none
# of it is authored here; and `frontends/packages/ui/.storybook/{main,preview}.ts`
# is linted WITHOUT the type-aware rules, because `include: [".storybook"]` in
# that package's tsconfig does not actually reach it — TypeScript's
# include-glob expansion skips dot-directories, so `tsc --listFiles` lists
# neither file and `pnpm -r typecheck` has never covered them either. That is
# a gap in the typecheck, recorded rather than papered over: fixing it means
# changing what `pnpm -r typecheck` covers, a different gate, so it is left
# as a maintainer's call.
#
# `no-console` is off in tests, Storybook stories, Cypress specs, `testing/`
# helpers and the command-line examples (`examples/*/index.mjs`,
# `checkout-browser`'s `mint.mjs`/`serve.mjs`, `sdks/nodejs/scripts/`) —
# those print on purpose. It is ON everywhere else, including
# `examples/checkout-browser/checkout.js`, the payer page.
lint-web: build-sdk-node
    pnpm -r typecheck
    pnpm -r lint

deny:
    cargo deny check

# `deny`'s counterpart for the JavaScript half of the repo, and CI's `web`
# job runs THIS recipe rather than a copy of its commands, so the gate and
# the local check cannot drift.
#
# Two runs, not one, and the narrower one first. `--prod` walks only the
# production dependency graph — what a merchant would actually receive from
# `@vaam-apps/vpay-sdk` / `@vaam-apps/vpay-stripe-js`, and what `frontends/Dockerfile` ships —
# so a failure there is a different kind of news from a failure in the build
# tooling, and reporting both under one exit code would flatten that. The
# second run covers everything, dev dependencies included, because every
# advisory this repo has had on the JS side has been in dev tooling
# (vitest/vite/esbuild, Next's pinned postcss, Cypress's request stack,
# Storybook's uuid) — gating only `--prod` would have been a green light
# over all sixteen of them, which is exactly the "suite goes green while
# proving nothing" failure CLAUDE.md names.
#
# `--audit-level=high` on both: high and critical fail, moderate does not.
# A deliberate ceiling, not an oversight — a moderate advisory in a
# transitive dev dependency appears most weeks, and blocking every merge on
# one trains people to reach for the ignore list, which is the only escape
# hatch here (`pnpm.auditConfig.ignoreCves` in `package.json`, currently
# ABSENT — no advisory is being suppressed as of 2026-09-03). A bare
# `pnpm audit` still reports moderates; this recipe just does not fail on
# them.
#
# NOT part of `just ci`, for the reason `helm-check` below is not: it talks
# to the npm registry's advisory endpoint on every run and cannot work
# offline at all, and `just ci` is expected to run on a machine with no
# network. CI runs it on every PR regardless.
#
# A registry outage is not an advisory. On 2026-09-04 the audit endpoint
# (`/-/npm/v1/security/audits`) timed out or answered 503 for about two hours
# and failed seven consecutive CI runs of a branch whose lockfile had not
# changed; every one of those runs printed `ERR_SOCKET_TIMEOUT`, none printed
# a finding. So each `pnpm audit` below is given a longer fetch timeout than
# pnpm's 60 s default and is retried a bounded number of times, and when it
# still fails the recipe says which of the two things happened: an advisory
# (the audit answered and found something — the exit code is pnpm's own) or
# the registry (no answer at all). Both still fail the job. What this does
# NOT do is pass when the registry is down: an unreachable audit is an audit
# that did not run, and this is a payment system.
audit_attempts := "4"
audit_retry_wait_seconds := "90"
audit_fetch_timeout_ms := "180000"

audit-web:
    #!/usr/bin/env bash
    set -euo pipefail
    export npm_config_fetch_timeout="{{audit_fetch_timeout_ms}}"
    audit() {
        # $1: label; the rest: pnpm audit arguments. Retries only when the
        # output carries the registry's own failure signatures; an advisory
        # fails at once, with pnpm's report already printed.
        local label="$1"; shift
        local attempt out
        for attempt in $(seq 1 {{audit_attempts}}); do
            echo "audit-web: ${label} (attempt ${attempt} of {{audit_attempts}})"
            if out=$(pnpm audit --audit-level=high "$@" 2>&1); then
                printf '%s\n' "$out"
                return 0
            fi
            if printf '%s' "$out" | grep -qE 'ERR_SOCKET_TIMEOUT|ERR_PNPM_AUDIT_BAD_RESPONSE|ECONNRESET|EAI_AGAIN|FetchError'; then
                printf '%s\n' "$out" | grep -E 'ERR_|FetchError|WARN' | tail -3
                if [ "$attempt" -lt {{audit_attempts}} ]; then
                    echo "audit-web: the npm registry's audit endpoint did not answer — retrying in {{audit_retry_wait_seconds}}s"
                    sleep {{audit_retry_wait_seconds}}
                    continue
                fi
                echo "audit-web: REGISTRY UNREACHABLE — the audit did not run ({{audit_attempts}} attempts, fetch timeout {{audit_fetch_timeout_ms}} ms). This is not an advisory; re-run when https://status.npmjs.org is clear." >&2
                return 2
            fi
            printf '%s\n' "$out"
            echo "audit-web: ADVISORY — pnpm audit found a high or critical advisory (${label}); see the report above" >&2
            return 1
        done
    }
    audit "production dependency graph only" --prod
    audit "whole workspace, dev dependencies included"
    echo "audit-web: ok — no high or critical advisory in the workspace"

# ---------------------------------------------------- self-verification ----

# The checks that keep this repository honest, plus one report. CI's
# `self-checks` job runs exactly this list, in this order:
# verify-no-mocks, verify-status, verify-errors, verify-sdk-parity,
# verify-links, verify-npm-scope, check-schema, verify-serde,
# verify-repositories, verify-toolchain, and then verify-docs last.
#
# That sentence was false until 2026-09-04: `verify-sdk-parity` ran here but
# had no step in `.github/workflows/ci.yml`, so ADR-0015's decision 3 ("CI
# enforces parity") was a claim nothing executed on a pull request — and the
# list ran `verify-docs` in the middle rather than last. Both are fixed
# together, and they have to move together, because the only thing that keeps
# this comment honest is someone reading the workflow beside it.
#
# `verify-docs` is NOT a check: it exits 0 whatever it finds, so the
# "verify: ok" below means the ten gates passed and says nothing about the
# numbers `verify-docs` printed. It is last so that the report a human reads
# is the final thing on the terminal, after every gate has had its say.
#
# `verify-links` joined the list on 2026-09-05 as the fifth gate. It is here
# rather than only in `docs-check` because a link is a claim like any other
# and `just ci` is where claims get checked; it needs no network and no
# database, so it costs a `git ls-files` and a pass over 113 files.
#
# `verify-npm-scope` joined it the same day as the sixth, for a reason
# measured rather than assumed: deleting `publishConfig.access` from
# `sdks/nodejs/package.json` — the one line between `npm publish` and a
# scoped package's default `restricted` — was caught by nothing. Not the
# lockfile, not `pnpm -r typecheck`, not `lint-web`, not `test-web`, not any
# of the five gates above.
#
# `check-schema` joined on 2026-09-05 as the seventh, and it is the first
# gate here that shells out to a tool this repository does not build. Until
# then `schemas/vpay.cstack` was verified by someone running `cratestack
# check` by hand and pasting the output into docs/status.md — a claim with a
# date on it and nothing re-running it, which is the shape of every claim
# this file exists to replace. See the recipe below for what happens when the
# binary is absent (it fails; it does not skip).
#
# `verify-serde` and `verify-repositories` joined on 2026-09-05 as the eighth
# and ninth, both from ADR-0016 — the ADR that finally wrote down the six
# engineering standards this repository had been applying in prose. They are
# after `check-schema` rather than beside their nearest relative
# (`verify-errors`) because the list is chronological and a reader comparing
# it with `.github/workflows/ci.yml` should be comparing two lists in the same
# order.
#
# What they were worth on the day they landed, measured rather than assumed:
# `verify-serde` found 28 of 64 serialisable types under `backends/crates`
# without the workspace's `rename_all`, of which 13 were fixed and 15 carry a
# reason in the ADR's table; `verify-repositories` found `vpay-api` naming
# `vpay_db::SqlClientAssertionStore`, a concrete implementation that had been
# `pub` since Step 6 and that no gate, lint or compiler error objected to.
#
# `verify-toolchain` joined on the same day as the tenth, during the review
# of the 1.95.0 -> 1.98.0 toolchain bump, and for a reason measured on that
# branch rather than assumed: with `rust-toolchain.toml` moved to 1.98.0 and
# `backends/Dockerfile` left on `FROM rust:1.95.0-alpine3.22`, `just verify`
# and `just fmt-check` both exited 0 — and no other recipe in `just ci` reads
# either file, because nothing here compiles the Dockerfile. The toolchain
# file's own header has said "bump both together" since 2026-09-02; that
# sentence was the whole mechanism until this gate, and the first symptom of
# ignoring it would have been a release binary built by a compiler no local
# run and no CI job had ever used. It is appended after `verify-repositories`
# rather than slotted in beside `check-schema` for the same reason those two
# are where they are: the list is chronological, so every ordinal already
# written down elsewhere — "check-schema is the seventh gate", which four
# other files say — stays true when a gate is added.
#
# The ten self-checks, then the advisory verify-docs report.
verify: verify-no-mocks verify-status verify-errors verify-sdk-parity verify-links verify-npm-scope check-schema verify-serde verify-repositories verify-toolchain verify-docs
    @echo "verify: ok — the ten gates above passed; the verify-docs report is advisory"

verify-no-mocks:
    cargo xtask verify-no-mocks

# AGENTS.md rule 2: every `ProviderError::NotImplemented("…")` token in
# shipping code is declared in docs/status.md, and every token declared there
# is still carried by shipping code. Both directions fail the build —
# `verify_status_reports_both_directions_from_the_gate_itself` in `xtask`
# drives the check itself, not a reimplementation of it, through both.
#
# "In shipping code" is lexical, not textual: since 2026-09-05 the scanner
# lexes rather than greps, so a token mentioned in a comment (`//`, `///`,
# `//!`, `/* */` nested or not, leading or trailing), in a `#[doc = "…"]`
# attribute, or inside a string, raw-string or character literal is prose and
# counts for nothing in either direction. Before that, a trailing comment or a
# raw string carrying the token forced a phantom bullet into docs/status.md —
# and the cheapest way to clear one was to delete the honest sentence from
# the adapter's doc comment that explained the gap.
verify-status:
    cargo xtask verify-status

# ADR-0011: every pub error type in backends/crates implements
# vpay_core::error::Classify, and anyhow stays in backends/apps.
verify-errors:
    cargo xtask verify-errors

# ADR-0015: every ✅ in docs/sdks/parity.md names a test that exists in that
# SDK's sources, and every ⛔ carries a dated, owned gap.
verify-sdk-parity:
    cargo xtask verify-sdk-parity

# Every relative link in every tracked *.md resolves to a tracked file or
# directory. `git ls-files`, not a directory walk: a link satisfied by an
# untracked scratch file resolves on the author's machine and nowhere else.
#
# What it does NOT check, so nobody reads a green run as more than it is:
# `#anchor` fragments (agreeing with GitHub's heading-slug algorithm is a
# guess, and a wrong guess fails correct documents), `http(s)` URLs and
# `mailto:` targets. Fenced code blocks, inline code spans and HTML comments
# are masked out first.
#
# Before 2026-09-05 this did not exist and `docs-check` said so in an echo.
# Fail if a doc links to a file this repository does not track.
verify-links:
    cargo xtask verify-links

# Every publishable npm package under `sdks/` is named `@vaam-apps/vpay-*`,
# declares `publishConfig.access: "public"`, names this repository, carries a
# license, and ships a `files` allowlist with an entry point under `dist/`;
# every private one declares no `publishConfig` at all; and no retired
# `@vpay/*` package name survives outside `docs/plans`, `docs/adr` and
# `docs/status.md`.
#
# The `files`/`main` half is not tidiness. `sdks/stripe-compat` has no build,
# no `main` and no `files`, so `pnpm pack` on it produces a tarball of five
# `*.compat.test.ts` files and a `vitest.config.ts` — which is exactly why it
# is the one SDK that stays `"private": true`, and why the gate objects to a
# private package that advertises publish-readiness anyway.
#
# What it does NOT check: that `dist/` exists (gitignored — a gate needing a
# build would fail on a clean checkout for a reason that is not its subject;
# `lint-web` and CI's `web` job build it) and the registry (that needs the
# network, which is `verify-citations`' exception and not this one's).
verify-npm-scope:
    cargo xtask verify-npm-scope

# The CrateStack CLI release this repository verifies its schema against, and
# the ONE place that number lives. `.github/workflows/ci.yml` does not repeat
# it: the `self-checks` job reads it back with `just --evaluate
# cratestack_version` and feeds that to the install action, the same trick the
# jobs there already use to read the compiler channel out of
# `rust-toolchain.toml`. Bump it here and CI follows; there is no second copy
# to forget.
cratestack_version := "0.11.1"

# The floor for `check-schema`'s "is there anything here?" assertion: the
# number of top-level `model`/`enum` declarations schemas/vpay.cstack must
# still carry for a `schema OK` to mean anything. Six models and six enums
# today. A FLOOR, deliberately, not an exact count — adding a model is not a
# reason to fail a gate, and this exists to catch a file that was emptied or
# truncated, not one that grew.
cratestack_min_declarations := "12"

# `schemas/vpay.cstack` parses and type-checks against the real CrateStack
# grammar. Seventh gate in `just verify`, new 2026-09-05.
#
# WHY IT IS A GATE AND NOT A NOTE. Before this recipe existed, the evidence
# that this file is valid was a transcript pasted into docs/status.md by
# whoever last ran the tool by hand ("Verified against CrateStack 0.10.1 …
# schema OK"). That is a claim with a date on it and nothing re-running it:
# the grammar moves fast (crates.io published 29 cratestack-cli releases
# between 0.7.8 on 2026-08-08 and the pinned 0.11.1 on 2026-09-03), and the
# first anyone would have learned that the file had stopped parsing is
# whenever somebody next felt like checking. The schema is still EXCLUDED
# FROM THE BUILD GRAPH — no crate depends on it, nothing generates from it —
# so this gate is the only thing in the repository that reads it at all.
#
# WHAT A GREEN RUN PROVES, AND WHAT IT DOES NOT. It proves the file parses
# and type-checks under the pinned CLI. It proves nothing about a migration,
# a generated server or a running database: `cratestack migrate diff` has
# never been run against a vpay Postgres, and `backends/migrations/*.sql` —
# not this file — is the authoritative schema. See docs/status.md, section
# "CrateStack".
#
# A MISSING BINARY IS A RED GATE. It exits non-zero and prints how to install
# the tool; it never prints "skipped" and exits 0. Same rule as
# `docs-check-citations` without `gh`: a check that downgrades itself reports
# success for a run in which nothing was checked, in a log indistinguishable
# from one in which everything passed. This is why `check-schema` is NOT in
# the offline-safe promise `just ci` makes about the other gates in the same
# way they are — it needs no network to RUN, but it does need a binary that
# is not part of this workspace, and `just install-rust` does not install it.
# That parenthesis used to give a reason — "installing it needs a newer
# compiler than `rust-toolchain.toml` pins" — and the toolchain bump of
# 2026-09-05 (1.95.0 -> 1.98.0) retired it: `cargo install cratestack-cli
# --locked --version 0.11.1` now succeeds from inside this checkout, which is
# what the message below says. `install-rust` still leaves it out, but that
# is now a choice about what a bootstrap recipe should compile rather than
# something the compiler refuses; whether it should install it is a
# maintainer's call, not this comment's.
#
# THE VERSION IS REPORTED, NOT ENFORCED, LOCALLY. The recipe prints the
# version it actually used on every run, and says so loudly when that is not
# `cratestack_version`, so a log never leaves which grammar answered in
# doubt. It does not refuse to run on a mismatch: CI installs the pin exactly
# and CI is the gate of record, and blocking every contributor whose PATH
# carries a newer release is how a gate acquires a local opt-out. That is the
# same division `helm-check` already draws — presence checked locally,
# version pinned in the workflow.
#
# `schema OK` IS NOT ENOUGH ON ITS OWN, WHICH IS WHY THIS RECIPE ALSO CHECKS
# THE SHAPE OF WHAT IT CHECKED (added 2026-09-05 by review, with the two
# measurements that prompted it):
#
#   $ : > empty.cstack && cratestack check --schema empty.cstack
#   schema OK: empty.cstack                                     # exit 0
#
#   $ # schemas/vpay.cstack with the `datasource` block deleted and
#   $ # `tags String[]` added to PaymentIntent:
#   schema OK: ...                                              # exit 0
#
# Both are the failure this gate exists to prevent, wearing the gate's own
# green: an emptied or truncated schema type-checks vacuously, and deleting
# the `datasource` block turns off every database-backed-model rule —
# including the list-arity refusal that is the mutation this gate is proven
# with, and which the CLI's own error message offers "drop the `datasource`
# block" as a way to silence. `cratestack check` is right to accept both (a
# client-only schema is a real thing) and there is no CLI flag that says
# "and it must be a database-backed schema with content in it", so the
# assertion belongs here, next to the claim it protects. The floor is a
# floor, not an exact count, so adding a model does not fail the gate; it is
# `verify-ignored`'s `min_tests` in miniature.
#
# Fail if schemas/vpay.cstack does not parse against the pinned CrateStack.
check-schema:
    #!/usr/bin/env bash
    set -euo pipefail
    schema="schemas/vpay.cstack"
    pinned="{{ cratestack_version }}"

    if ! command -v cratestack >/dev/null 2>&1; then
        echo "check-schema: FAIL — needs the 'cratestack' CLI on PATH, and it is not there." >&2
        echo "check-schema: this is a failure, not a skip: nothing checked $schema in this run." >&2
        echo >&2
        echo "  Install the pinned release:" >&2
        echo >&2
        echo "      cargo install cratestack-cli --locked --version $pinned" >&2
        echo >&2
        echo "  This works from inside the checkout as of 2026-09-05, and did not" >&2
        echo "  before: cratestack-cli $pinned declares rust-version = 1.98.0, and" >&2
        echo "  rust-toolchain.toml pinned 1.95.0 until that date, so cargo run from" >&2
        echo "  inside the worktree refused with an msrv error and the instruction" >&2
        echo "  here was to cd out of the tree first. The pin is 1.98.0 now — that" >&2
        echo "  bump is why. There is also a prebuilt binary for five target triples" >&2
        echo "  (x86_64/aarch64 linux-gnu and apple-darwin, x86_64-pc-windows-msvc) at" >&2
        echo "  https://github.com/cratestack/cratestack/releases/tag/v$pinned —" >&2
        echo "  linux MUSL has none, per https://cratestack.dev/tooling/cli-install" >&2
        exit 1
    fi

    # `cratestack --version` prints "cratestack <semver>".
    found="$(cratestack --version | awk '{print $2}')"
    if [ "$found" != "$pinned" ]; then
        echo "check-schema: WARNING — cratestack $found on PATH, this repository pins $pinned." >&2
        echo "check-schema: the check below still ran in full, but against the $found grammar." >&2
    fi

    # A schema with nothing in it, or with no `datasource` block, passes
    # `cratestack check` — see the comment above for both transcripts. Assert
    # what was actually checked before reporting a green.
    if ! grep -qE '^datasource [A-Za-z_][A-Za-z0-9_]* \{' "$schema"; then
        echo "check-schema: FAIL — $schema declares no 'datasource' block." >&2
        echo "check-schema: without one, cratestack treats it as a client-only schema and" >&2
        echo "check-schema: stops applying every database-backed-model rule — including the" >&2
        echo "check-schema: list-arity refusal this gate is proven with. It would still say" >&2
        echo "check-schema: 'schema OK'. If dropping the datasource is deliberate, change this" >&2
        echo "check-schema: recipe and docs/status.md in the same commit and say what the gate" >&2
        echo "check-schema: still covers." >&2
        exit 1
    fi

    declarations="$(grep -cE '^(model|enum) [A-Za-z]' "$schema" || true)"
    if [ "$declarations" -lt "{{ cratestack_min_declarations }}" ]; then
        echo "check-schema: FAIL — $schema declares $declarations model/enum(s), fewer than the floor of {{ cratestack_min_declarations }}." >&2
        echo "check-schema: an emptied or truncated .cstack file type-checks vacuously and" >&2
        echo "check-schema: cratestack prints 'schema OK' for it, so a green here would mean" >&2
        echo "check-schema: nothing. If declarations were removed on purpose, lower the floor" >&2
        echo "check-schema: (cratestack_min_declarations in this justfile) in the same commit." >&2
        exit 1
    fi

    echo "check-schema: cratestack $found, schema $schema ($declarations model/enum declarations, datasource present)"
    cratestack check --schema "$schema"
    echo "check-schema: ok — $schema type-checks under cratestack $found"

# A REPORT, not a gate: doc-comment lines against code lines per crate, the
# production functions of 80 lines or more, every ```ignore doctest fence and
# every #[allow]/#[expect] in production code. It exits 0 whatever it finds.
#
# Step 7's decision (4), and the reasoning is worth keeping where the recipe
# is: the cheapest way to pass a doc-ratio gate is to delete the `# Errors`
# and `# Panics` sections ADR-0011 and rustdoc depend on. A number that is
# read is worth more here than a number that is enforced. Nothing in `just ci`
# can fail because of it.
#
# Report doc volume, long functions, ```ignore fences and #[allow]s.
# ADR-0016 standard 3: every type deriving Serialize/Deserialize under
# backends/crates/*/src carries #[serde(rename_all = "snake_case")], renames
# every field/variant itself, or is listed in the ADR's exemption table with a
# reason. Visibility is deliberately not part of the rule — both adapters'
# wire modules are pub(crate), and a wire does not care what Rust thinks of a
# type's visibility.
#
# Two-directional, like verify-status: an exemption row naming a type that now
# complies, or a type that no longer exists, fails the build too. A stale
# exemption is a decision the code has already reversed, described in the ADR
# as if it were current.
#
# What it does NOT check: whether a reason is a good one. "models MTN's
# camelCase Collections wire" and "too many to fix" are both non-empty
# strings; the gate refuses only a blank reason, and the table exists to put
# the sentence where a reviewer sees it.
verify-serde:
    cargo xtask verify-serde

# ADR-0016 standard 5: repositories are traits, their implementations are
# private to vpay-db, and a handler names the trait. The set of concrete
# implementations is derived from vpay-db's own source — a declaration holding
# a PgPool/Transaction field, or a type on the right of `impl <a vpay-db
# trait> for …` — rather than listed in the gate, so a store nobody has
# written yet is covered the day it is added.
#
# No exemption mechanism, deliberately: there is no exception today, and an
# escape hatch nobody needs is the one that gets used.
verify-repositories:
    cargo xtask verify-repositories

# `backends/Dockerfile`'s `FROM rust:<version>-alpine…` is the compiler
# `rust-toolchain.toml` pins. That `FROM` line is the one place in this
# repository that names a compiler version and cannot read the toolchain file
# — CI's five Rust jobs all `sed` the channel out of it — so it is the one
# place the pin can drift, and until this gate the only thing stopping it was
# a sentence in a comment. The Alpine base is deliberately NOT checked: it
# moves on its own evidence. See `verify_toolchain` in `.xtask/src/main.rs`
# for the mutation that motivated it and for what this does not cover.
verify-toolchain:
    cargo xtask verify-toolchain

verify-docs:
    cargo xtask verify-docs

# Pins how much of the suite is `#[ignore]`d, how many test binaries exist,
# and how big the suite is, so a new `#[ignore]` cannot quietly shrink
# coverage and a test binary dropping out of the workspace cannot read as
# "fewer tests, still green". The binary count is the one that actually
# catches a dropped binary: many binaries hold only a handful of tests, so a
# global floor alone would let one vanish unnoticed. All three numbers are
# deliberate and must be bumped in the same commit that legitimately changes
# them.
#
# History: 3 ignored (the not-implemented conformance cases) from 2026-09-02;
# 18 for a few hours on 2026-09-03 while the rewritten conformance suite was
# a failing spec ahead of the adapters; 0 since both adapters landed the same
# day and all 26 cases run live against WireMock containers. A test is
# ignored only while its behaviour is unbuilt (AGENTS.md rule 2).
#
# Binaries, all on 2026-09-03: 35 -> 37 (Step 4) for
# `backends/tests/integration/tests/worker_{recovery,e2e}.rs`, the two suites
# that are the *only* proof any job handler works (vpay_worker::handlers' own
# module comment says why it has no unit tests); 37 -> 38 (Step 5) for
# `backends/tests/integration/tests/webhooks.rs`. Step 5b added **no** binary:
# its Rust tests land in files that already existed (`vpay-api`'s own units,
# `backends/tests/integration/tests/payment_intents.rs`,
# `vpay-db`'s `repositories.rs`), and its new suite is TypeScript — see
# `sdks/stripe-compat`, which cargo does not run and this count does not
# cover. `min_tests` is a floor that catches a binary vanishing, not a running
# total — it is set a little under the measured count rather than to it, so it
# is not a number people bump reflexively.
#
# 38 -> 39 (Step 5c) for
# `backends/tests/integration/tests/browser_checkout.rs`, which is the only
# proof the payer-facing `/v1/browser` surface exists at all — and, more to
# the point, the only place the uniform-404 property can be asserted, since
# it is a property of rendered response *bodies* rather than of a function.
#
# Measured 2026-09-03 after Step 5b was rebased onto Steps 4 and 5:
# `886 tests run: 886 passed, 0 skipped`, 38 binaries.
#
# Re-measured 2026-09-03 after Step 6 was squashed and rebased onto that tree
# (Steps 4, 5 and 5b) and Step 6's own webhook-delivery metric assertions
# landed: `just verify-ignored` lists 927 total, still 38 binaries — no new
# binary, because both new assertions (`the_ladder_walks_delivery_delay_and_
# then_succeeds`, `a_delivery_past_the_last_rung_is_exhausted_and_not_
# rescheduled`) extend existing tests in `webhooks.rs` rather than adding
# one.
#
# Re-measured again 2026-09-03 after Step 5c (`browser_checkout.rs`, the
# `@vaam-apps/vpay-stripe-js` unit suite which cargo does not run and this count does
# not cover, and the `track_http_metrics`-on-the-browser-nest test added
# while rebasing 5c onto Step 6) was rebased onto that tree:
# `cargo nextest list --workspace` lists **969 total, 39 test binaries, 0
# ignored** — 39 binaries because `browser_checkout.rs` is a new binary
# (38 -> 39, as the paragraph above already anticipated); 969 rather than
# 927 for the sum of Step 5c's own new tests (including
# `a_browser_get_is_counted_under_its_own_route_pattern`, added while
# resolving this rebase) landing on top of Step 6's 927.
#
# Step 7 Phase A (the `vpay-db` repository seam and the `ProviderError` source
# chain) measured **976** on the same 39 binaries — 969 plus four tests from
# the review remediation and three from the pass itself. That figure lived
# only in docs/status.md and never reached this comment, which is why the
# sequence below is written out in full rather than continued from 969.
#
# Re-measured 2026-09-03 with all five Step 7 lanes landed (doctests, the
# doc externalisation, the shared rail token cache, the tooling in this file):
# `just verify-ignored` lists **999 total, 39 test binaries, 0
# ignored**. Still 39: every test the lanes added landed in a file that
# already existed, and none of them added a `#[ignore]`. The doctests Step 7
# built are **not** in that number and cannot be — `cargo nextest list` does
# not see doctests at all, which is the whole reason `just test-doc` exists as
# a separate recipe and a separate CI step. Their count is in docs/status.md.
#
# 39 -> 40 (Step 8, lane D) for
# `backends/tests/integration/tests/worker_kill9.rs` — the real-`SIGKILL`
# crash test named in `docs/plans/2026-09-03-step8-production-gate.md`
# (`docs/plans/step8-notes/lane-d.md` has the full account). It spawns the
# shipping `vpay-worker-bin`/`vpay-server` as real OS processes and
# `Child::kill()`s them mid-request, so it is a new binary rather than a
# case added to an existing one — its own `mod support;` and its own
# process-spawning harness would not belong inside `worker_recovery.rs`,
# which proves the same recovery table without ever causing a signal.
#
# Re-measured 2026-09-04 on the Step 8 gate branch, with Step 7 merged and
# lanes B and D on top of it: `just verify-ignored` lists **1016
# total, 40 test binaries, 0 ignored** — the 999 above, plus lane D's two
# `worker_kill9` cases (the fortieth binary) and lane B's fifteen (nine
# `vpay_worker::ssrf` unit cases, three `vpay-config` cases, one
# `vpay-provider` pin case and two container-backed `webhooks` cases), all of
# which landed in files that already existed.
#
# 40 -> 41 (Step 8, lane C, the rail callback route), merged onto lanes B and
# D above: `just verify-ignored` on the gate branch lists **1054 total, 41 test
# binaries, 0 ignored**. 41 because `backends/tests/integration/tests/provider_callback.rs`
# is a new binary — the first suite whose subject is a request a *rail* makes
# — and the counts below move with it, in this commit and not a follow-up.
# The 17 new cases are 9 in that binary, 2 conformance cases
# (`the_submit_tells_the_rail_where_to_call_back`, once per rail), 2
# `vpay-api` router cases (a third asserts the 405 a payer's GET now gets), 2
# `vpay-db` repository cases and one unit test in
# `vpay_api::provider_callback`. The rest of the way from 1033 to 1054 is lane
# G (four `vpay-worker` unit cases for the age guard and three
# `worker_recovery` cases, one of them the confirm/worker race run without a
# seam) and lane F (the fourteen `sdk_parity_tests` in `xtask`); lane A adds
# no test, and none of these is a new binary.
#
# Re-measured 2026-09-04 after Step 8's correctness-review remediation (lane
# H): **1059 total, still 41 test binaries, 0 ignored**. Neither counter
# below moves. The five new cases all landed in files that already existed —
# two `vpay-worker` units (the age is the database's, and a `Wait` carries the
# rest of the window), one `vpay-db` unit (the charge read carries Postgres'
# clock), and two `provider_callback` integration cases (the pull-forward
# floor is the ladder's first rung, and a poll already about to run is not
# accelerated) — so `expected_suites` stays 41, and `min_tests` is a floor
# that 1059 clears with the same margin 1054 did.
#
# Re-measured 2026-09-04 on Step 9's lane 2 (the payer's return trip through
# the port): **1068 total, still 41 test binaries, 0 ignored**. Neither
# counter below moves. The nine new cases all landed in files that already
# existed — one `vpay-adapter-mtn-momo` wire unit (a push rail's body carries
# no return URL), two `vpay-adapter-orange-money` units of which one replaced
# the deployment-settings-fallback case it retired, three
# `vpay_api::v1::return_trip` units, two conformance cases
# (`the_submit_tells_the_rail_where_to_send_the_payer_back`, once per rail)
# and two `confirm_rails` integration cases — so `expected_suites` stays 41,
# and `min_tests` is a floor that 1068 clears with the same margin 1059 did.
#
# 41 -> 42 (Step 9, lane 1, the Checkout Session object) for
# `backends/tests/integration/tests/checkout_sessions.rs`. A new binary rather
# than cases added to `browser_checkout.rs`, and deliberately: that suite's
# subject is the two payer routes for a *payment intent*, and its harness
# registers no checkout app and no embedding origins. Folding a second object,
# a second pair of credentials and a `checkout.public_base_url`-less
# deployment into it would have made every one of its existing cases run
# against a configuration none of them is about.
#
# Measured 2026-09-04 on the lane-1 branch: `cargo nextest list --workspace`
# lists **1098 total, 42 test binaries, 0 ignored** — the 1059 above plus
# lane 1's 39 (13 in the new binary, 10 `vpay-api` units across
# `v1::checkout_sessions`/`browser::checkout_sessions`, 4 in `model`, 8
# `vpay-config` units, 2 `vpay-core` `ids` units and 2 `vpay-db` units). The
# four new doctests (`CheckoutConfig` twice, `ids::return_token`,
# `CheckoutSessionRow::return_page_url`) are **not** in that number and cannot
# be — `cargo nextest list` does not see doctests, which is why
# `just test-doc` is a separate recipe. `just test-doc` measures **82**.
#
# Merged 2026-09-04 on the Step 9 gate branch (lanes 5, 2, 3, 2b and 1 in):
# lane 2b's three digits-only MTN steering cases joined lane 2's nine and lane
# 1's thirty-nine: `just verify-ignored` on the merged gate measures **1121
# total, 42 test binaries, 0 ignored**; `expected_suites` is 42 (lane 1's
# `checkout_sessions` binary) and the 1050 floor stands under it.
#
# Re-measured 2026-09-05 landing the session-driven confirm refusal (a confirm
# on a Checkout Session that is no longer `open`): `just verify-ignored` lists
# **1159 total, 42 test binaries, 0 ignored**. Neither counter below moves and
# the 1080 floor stands. The twelve new cases all landed in files that already
# existed — seven `checkout_sessions` integration cases, one `postgres_smoke`
# case for migration `0030`'s index, three `vpay_api::v1::return_trip` units
# and one `vpay_api::error` unit — so no binary was added or dropped. 1159 is
# what `cargo nextest list --workspace` printed on this branch, not a figure
# derived from the 1121 above: that number was measured on the Step 9 gate
# branch and the lanes and follow-ups that landed on `master` after it were
# never added to this comment.
expected_ignored := "0"
expected_suites := "42"
# A floor, not a target — set a little under the measured 1059
# rather than to it, so it is not a number people bump reflexively. Bump it in
# the same commit that legitimately adds tests, never to make a red run green.
#
# 900 -> 950 on 2026-09-03 (Step 7): 900 was set against Step 5c's 969
# and the suite has grown by 30 since, so the old floor had drifted
# far enough below the count that a whole crate's unit tests could vanish
# under it.
#
# 950 -> 990 on 2026-09-04 (Step 8, lanes B and D): 950 was set against Step
# 7's 999 and the suite has grown by 17 since, so the floor is moved with it
# rather than being left to drift far enough below the count that a whole
# crate's unit tests could vanish under it.
# 990 -> 1000 on 2026-09-04 (Step 8, lane C merged), against the measured
# 1054 and on the same terms: still a floor set under the count, not at it.
# Left at 1000 on 2026-09-04 (lane H, measured 1059): five tests is not a
# reason to move a floor, and moving it every time one is added is how a
# floor becomes a number nobody reads.
#
# 1000 -> 1050 on 2026-09-04 (Step 9, lane 1), against the measured 1098 and
# on the same terms: still a floor set under the count, not at it. 1000 was
# set against Step 8's 1059 and the suite has grown by 39 since, so the floor
# is moved with it rather than being left to drift far enough below the count
# that a whole crate's unit tests could vanish under it.
#
# 1050 -> 1080 on 2026-09-04 (Step 9, lane 1b — the integration seams and the
# correctness-review findings F2-F5), against the measured **1129** on the
# gate branch with this lane's eight net cases in it: eleven added (two
# `vpay-config` units for the canonical-origin and display-name rules, one
# `vpay_api::v1::return_trip` unit, one `payer_instrument` unit, three
# `confirm_rails` integration cases and four `checkout_sessions` ones) and
# three retired — `return_url_for_charge`'s precedence units, whose subject
# moved into `payer_instrument` and is tested there against the shipping
# function rather than a stand-in. `expected_suites` does **not** move: every
# case landed in a binary that already existed. Same terms as every bump
# above: a floor set under the count, never at it. Lane E should re-measure
# after the merge.
#
# Re-measured by lane E on the merged gate (`e57e7ff`, every lane in):
# `just verify-ignored` reports **1137 total, 42 test binaries, 0 ignored** —
# 1129 plus lane 5b's eight `sdks/rust` cases for the client-assertion
# audience (three builder units, two wire tests and three `op_conformance`
# ones against the real pinned verifier). The floor **stays at 1080**: eight
# tests is not a reason to move a floor, on the same terms as lane H's
# "left at 1000" above. `expected_suites` stays 42 — lanes 3b, 4, 1b, 5b, r2
# and 6 added no test binary — and `just test-doc` measures **84 passed, 1
# ignored** (the ignored one is `sdks/rust`'s README block and is
# pre-existing).
#
# Re-measured 2026-09-04 for `checkout.session.expired` (the event and webhook
# a Checkout Session emits when the sweep expires it): `just verify-ignored`
# reports **1146 total, 42 test binaries, 0 ignored** — 1137 plus nine cases,
# every one of them in a file that already existed. Six in
# `backends/tests/integration/tests/checkout_sessions.rs` (the event and its
# deliveries, the second sweep, a live charge, a settled session, the
# `/v1/events` read with its tenant boundary, and the transactionality proof),
# one `vpay-api` `model` unit for the rendered snapshot, and two `sdks/rust`
# `resources` cases for the event-type vocabulary. `expected_suites` stays 42:
# no test binary was added, so the checkout suite is where the sweep's own
# case already lives and where a reader looks for it.
#
# The floor **stays at 1080**, on lane E's and lane H's terms: nine tests is
# not a reason to move a floor, and moving it every time one is added is how a
# floor becomes a number nobody reads. `just test-doc` measures **86 passed, 1
# ignored** — 84 plus the two examples this change added
# (`CheckoutSessionObject::expired_snapshot` and `vpay_sdk::KnownEventType`);
# the ignored one is still `sdks/rust`'s README block and still pre-existing.
#
# Re-measured 2026-09-04 after the sabotage review of that change
# (`docs/plans/step9-notes/session-expired-review.md`): **1147 total, 42 test binaries, 0
# ignored**, and `just test-doc` still **86 passed, 1 ignored**. One case
# added — `a_payer_confirming_between_the_read_and_the_write_keeps_the_session`
# — because deleting the live-charge `NOT EXISTS` from `expire_due`'s write
# half left all 23 cases in the checkout suite green. Still no new test
# binary, so `expected_suites` stays 42, and the floor stays 1080.
#
# Re-measured 2026-09-05 on the tree rebased onto `master` for landing:
# `just verify-ignored` reports **1147 total, 42 test binaries, 0 ignored**
# and `just test-doc` **86 passed, 1 ignored** — both unchanged. The rebase
# brought in `61bce45` (assertion messages, test-only) and this pass changed
# four more of the same shape; neither adds or removes a case.
# Re-measured 2026-09-05 for the session-driven confirm refusal (a confirm on
# an intent whose Checkout Session is not `open` is a `409`): `just
# verify-ignored` reports **1158 total, 42 test binaries, 0 ignored** — 1147
# plus eleven cases, every one of them in a file that already existed. Seven
# in `backends/tests/integration/tests/checkout_sessions.rs` (the sweep-expired
# refusal, the merchant-expired one, the unswept horizon with the session row
# proved untouched, the `complete` code, an intent with no session confirming
# as before, the merchant `/v1` refusal with its `Idempotency-Key` replay, and
# a second session after an expiry making the intent payable again) and four
# `vpay-api` units (three in `v1::return_trip` for the verdict, the horizon's
# boundary and an unknown `status`; one in `error` for the two codes).
# `expected_suites` stays 42: no test binary was added.
#
# The floor **stays at 1080**, on the same terms as every bump above: eleven
# tests is not a reason to move a floor. `just test-doc` measures **86 passed,
# 1 ignored** — unchanged, because this change added no example; the ignored
# one is still `sdks/rust`'s README block and still pre-existing.
#
# Re-measured 2026-09-05 during the review of the session-driven confirm
# refusal: **1159 total, 42 test binaries, 0 ignored** — 1158 plus
# `postgres_smoke::the_confirm_paths_session_lookup_is_served_by_an_index`,
# the guard for migration `0030`'s `checkout_sessions_intent_seq_idx`. Still
# no new binary, and the floor still stays at 1080.
#
# Re-measured 2026-09-05 rebasing `claude/exp3-verify-status-opus` (the
# `verify-status` lexer) onto that landed confirm refusal: `just
# verify-ignored` reports **1166 total, 42 test binaries, 0 ignored** — 1159
# plus seven cases, all in `xtask` (83 → 90): the characterising test for the
# defect, five for the lexer's own edge cases, and one — added during this
# branch's own review — that drives `verify_status` itself through both
# directions of the check so neither can go missing unnoticed. Two more
# shapes (a nested block comment and a comment carrying an odd `"`) went into
# an existing case rather than a new one, so they move no counter. No new
# test binary, so `expected_suites` stays 42 and the floor stays 1080.
#
# Re-measured 2026-09-05 with `verify-links` and `verify-citations`: `just
# verify-ignored` reports **1202 total, 42 test binaries, 0 ignored** — 1166
# plus thirty-six, all in `xtask` (90 → 126). Twenty-three for the link
# parser and the gate (nine of them driving `verify_links` end to end over a
# throwaway `git init`ed tree, because "tracked, not merely present" is the
# rule that makes a green run mean anything and only a real index proves it),
# thirteen for the citation patterns offline. No new test binary — both
# commands live in `.xtask/src/main.rs` — so `expected_suites` stays 42, and
# the floor stays 1080: thirty-six tests is not a reason to move a floor.
#
# Re-measured 2026-09-05 after this branch's review, which added four more to
# `xtask` (126 -> 130) and no test binary: **1206 total, 42 test binaries, 0
# ignored**. Three of the four are guards for properties that were prose:
# `verify-citations` fails rather than skips when `gh` is missing, a 403/429
# stops the run instead of reporting the batch missing, and a zero-padded
# eleven-digit number is a timestamp rather than a run id. The fourth is a
# link to the repository root. `expected_suites` stays 42; the floor stays
# 1080.
# Re-measured 2026-09-05 for ADR-0016's two gates (`verify-serde`,
# `verify-repositories`): `just verify-ignored` reports **1260 total, 42 test
# binaries, 0 ignored**, of which 40 are this change's, all in `xtask`
# (144 → 184): 13 driving `verify_serde` and its scanner, 15 driving
# `verify_repositories` and the three signals it unions, 8 on the declaration
# scanner the two share, and 4 on the report lines `verify-docs` gained. Four
# of the 40 are the mutations recorded in `docs/plans/exp10-notes/opus.md`,
# and 3 are the review's: the alias evasion that cleared the first draft of
# `verify_repositories` (see that function's third signal).
#
# The 1206 above was measured on an older base, and the paragraphs between
# were written on branches that did not see each other. `xtask` on this
# branch's base (`master` `2ce13d0`) was measured directly at **144**
# (`cargo test -p xtask` before any of this landed), so the base total is
# 1260 - 40 = 1220 — derived from that one measurement rather than listed
# separately, and stated as derived. No new test binary
# either way, so `expected_suites` stays 42 and the floor stays 1080: 40 tests
# is not a reason to move a floor. `just test-doc` measures **86 passed, 1
# ignored** — unchanged; this change added no example, and the 13
# `#[serde(rename_all)]` attributes it did add appear in no doctest.
#
# Re-measured 2026-09-05 rebasing the toolchain bump (`verify-toolchain`, the
# tenth gate) onto that: `just verify-ignored` reports **1270 total, 42 test
# binaries, 0 ignored** — 1260 plus ten, all in `xtask` (184 → 194), all
# driving `verify_toolchain`: the two files' agreement, a `FROM` line left
# behind, an unreadable `channel`, a `channel` line CI's anchored `sed` could
# not parse, the Alpine suffix being deliberately outside the subject, and the
# repository's own two files. Four of the ten came from mutations of the gate
# itself. The two branches did not see each other — ADR-0016's two gates and
# this one both landed on 2026-09-05 — so the 1260 above and the 1270 here are
# two measurements, not a contradiction. No new test binary, so
# `expected_suites` stays 42 and the floor stays 1080. `just test-doc` measures
# **86 passed, 1 ignored** — unchanged; a toolchain pin appears in no doctest.
min_tests := "1080"

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
#
# `test-doc` sits between `test-rust` and `verify-ignored` here and in the
# `rust` job, because nextest runs no doctests and `verify-ignored`'s counts
# do not cover them: three different questions, three steps.
ci: fmt-check clippy verify test-rust test-doc verify-ignored lint-web test-web deny

# ------------------------------------------------------------------ helm ---

chart := "deploy/helm/vpay"

# Everything CI's `deploy` job runs, in the same order, by calling this recipe.
# CI runs `just helm-check` rather than a copy of these commands, so the gate
# and the local check cannot drift.
#
# NOT part of `just ci`, and that is deliberate: `kubeconform` fetches its
# schemas over HTTPS (the upstream JSON-schema mirror, plus the CRD catalog
# for ServiceMonitor/PrometheusRule, which no `-schema-location default` can
# know about). `just ci` is expected to run on a machine with no network, so
# adding this to it would turn "offline" into "failing". Run it by hand before
# opening a PR that touches the chart; CI runs it on every PR regardless.
#
# What it proves: the chart lints, both value sets render, the seventeen named
# guards are exactly the seventeen on disk and each fires on its own values
# file with a non-zero exit, the default render templates no checkout page and
# `ci/values-full.yaml`'s does, and every rendered object validates against the
# upstream schemas. What it does not prove: anything at all about a cluster.
# Nothing here has ever been applied to one.
helm-check:
    #!/usr/bin/env bash
    set -euo pipefail
    for tool in helm kubeconform; do
        command -v "$tool" >/dev/null 2>&1 || { echo "helm-check: needs '$tool' on PATH" >&2; exit 1; }
    done

    chart="{{ chart }}"
    out="$(mktemp -d)"
    trap 'rm -rf "$out"' EXIT

    echo "==> helm lint (defaults, then ci/values-full.yaml)"
    helm lint "$chart"
    helm lint "$chart" -f "$chart/ci/values-full.yaml"

    echo "==> helm template"
    helm template vpay "$chart" > "$out/default.yaml"
    helm template vpay "$chart" -f "$chart/ci/values-full.yaml" > "$out/full.yaml"

    # Each file under ci/guards/ violates exactly one guard, and the file's
    # basename IS the guard's name. A guard that stops firing — or one whose
    # message stops naming itself — fails here, which is the only thing that
    # keeps these from rotting into decoration.
    #
    # The expected set is written out rather than counted, because "17 files
    # were found and 17 fired" is also what deleting a guard *and* its values
    # file looks like. Adding a guard means adding its name here, its values
    # file under ci/guards/, and the `fail` in templates/_validate.tpl — in
    # one commit.
    expected_guards=(
        checkout-not-templated-by-default
        checkout-templated-when-enabled
        dashboard-not-templated
        database-secret
        extra-env-collision
        grace-period
        image-digest-format
        ingress-host
        networkpolicy-database
        observability-port
        overlay-empty
        pdb-minavailable
        rails-egress-except
        rails-secret
        rate-limit-ordering
        signing-key-secret
        worker-replicas
    )
    echo "==> template guards (each must FAIL, by name)"
    # LC_ALL=C so the comparison does not depend on the runner's collation
    # rules for the hyphens in these names.
    found=($(cd "$chart/ci/guards" && for f in *.yaml; do basename "$f" .yaml; done | LC_ALL=C sort))
    if [ "${expected_guards[*]}" != "${found[*]}" ]; then
        echo "helm-check: FAIL — ci/guards/ holds a different set of guards than this recipe expects." >&2
        echo "  expected: ${expected_guards[*]}" >&2
        echo "  found:    ${found[*]}" >&2
        exit 1
    fi

    guards=0
    for name in "${expected_guards[@]}"; do
        f="$chart/ci/guards/$name.yaml"
        if message="$(helm template vpay "$chart" -f "$f" 2>&1)"; then
            echo "helm-check: FAIL — guard '$name' did not fire; $f rendered successfully" >&2
            exit 1
        fi
        if ! printf '%s' "$message" | grep -qF "guard \"$name\""; then
            echo "helm-check: FAIL — $f failed, but not with guard '$name':" >&2
            printf '%s\n' "$message" >&2
            exit 1
        fi
        echo "    guard \"$name\" fired"
        guards=$((guards + 1))
    done
    echo "    $guards guards, all fired by name (${#expected_guards[@]} expected)"

    # The checkout page, in BOTH directions, over the RENDERED yaml rather
    # than over the values — because "checkout.enabled: false renders nothing"
    # is an absence, and a `fail` guard cannot assert an absence. The
    # "checkout-not-templated-by-default" guard covers the one well-typed way
    # to get half of it; this covers the other half.
    #
    # A payment page templated by default would be a Deployment nobody asked
    # for pulling an image they may not have; a page that stayed absent when
    # enabled would be an Ingress routing to nothing, found by a payer.
    echo "==> checkout page: absent by default, present when enabled"
    if grep -q -- '-checkout' "$out/default.yaml"; then
        echo "helm-check: FAIL — the default render names a checkout object, but checkout.enabled defaults to false:" >&2
        grep -n -- '-checkout' "$out/default.yaml" >&2
        exit 1
    fi
    for kind in Deployment Service Ingress; do
        if ! grep -B20 "^  name: vpay-checkout$" "$out/full.yaml" | grep -q "^kind: $kind$"; then
            echo "helm-check: FAIL — ci/values-full.yaml enables the checkout page but rendered no $kind for it" >&2
            exit 1
        fi
    done
    echo "    default: no checkout object; ci/values-full.yaml: Deployment + Service + Ingress"

    # ADR-0009 assumes a rate limit exists in front of the token endpoint.
    # This is the only thing in the repository that checks one is configured,
    # and it checks the RENDERED yaml, not the values — a template that stops
    # emitting the annotation would otherwise pass every other step here.
    echo "==> ingress rate limit"
    token_only="$(helm template vpay "$chart" -f "$chart/ci/values-full.yaml" --show-only templates/ingress.yaml)"
    printf '%s' "$token_only" | grep -q 'nginx.ingress.kubernetes.io/limit-rps' \
        || { echo "helm-check: FAIL — no limit-rps annotation in the rendered Ingress" >&2; exit 1; }
    rps=($(printf '%s\n' "$token_only" | sed -n 's/.*nginx.ingress.kubernetes.io\/limit-rps: "\([0-9]*\)".*/\1/p'))
    if [ "${#rps[@]}" -ne 2 ]; then
        echo "helm-check: FAIL — expected two Ingress objects each carrying limit-rps, found ${#rps[@]}" >&2
        exit 1
    fi
    # Rendered in template order: the /v1 Ingress first, the token one second.
    if [ "${rps[1]}" -gt "${rps[0]}" ]; then
        echo "helm-check: FAIL — the token Ingress limit-rps (${rps[1]}) is looser than /v1's (${rps[0]})" >&2
        exit 1
    fi
    echo "    /v1 limit-rps=${rps[0]}, /v1/oauth/token limit-rps=${rps[1]} (tighter, as intended)"

    echo "==> kubeconform (downloads schemas — needs network)"
    kubeconform -strict -summary \
        -schema-location default \
        -schema-location 'https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{{{.Group}}/{{{{.ResourceKind}}_{{{{.ResourceAPIVersion}}.json' \
        "$out/default.yaml" "$out/full.yaml"

    echo "helm-check: ok — lint, render, $guards guards, rate limit, kubeconform. No cluster was involved."

# --------------------------------------------------------------- release ---

# The GHCR namespace `release-dry-run` tags into. `release.yml` derives this
# from `github.repository_owner` at run time (its `namespace` job); a local
# recipe has no such context, so it is a variable instead, defaulting to the
# current owner. It was `vaam-store` until the organisation was renamed
# 2026-09-04. `--push=false` below means this default only ever labels a
# local, unpushed image — override with `just --set image_namespace <ns>
# release-dry-run` if that ever stops being true.
image_namespace := "vaam-apps"

# What `.github/workflows/release.yml` does, minus everything that needs a
# registry — and minus one thing that needs a runner it cannot have here.
#
# Builds all four images for the HOST platform ONLY. The published images are
# multi-arch (amd64 + arm64, step-6 decision (8)), but the workflow builds each
# architecture on a NATIVE runner — `ubuntu-latest` and `ubuntu-24.04-arm` —
# because `backends/Dockerfile` compiles the builder's own host triple and the
# whole point of decision (8) is not paying for `ring`'s asm and mimalloc's C
# build under emulation. Reproducing that locally would mean QEMU, i.e. the
# exact path the workflow was designed to avoid, at 10-30x the wall clock. So
# this recipe does not try: a green run here says the Dockerfiles build on
# THIS machine's architecture, and says nothing whatsoever about the other one.
#
# `--provenance=false --sbom=false` for the same class of reason: the
# attestations the workflow attaches need an exporter that can carry them, and
# the local `docker` driver cannot. They are unexercised here by construction.
#
# Also NOT covered, and it is most of the workflow: `push-by-digest`, the
# `imagetools create` manifest merge, the tag set from `docker/metadata-action`,
# GHCR authentication, and `cosign sign`. Nothing local can stand in for those.
# The first evidence for any of them is a real run — see docs/runbooks/release.md.
#
# Build the four release images for the host platform, then check the chart.
release-dry-run:
    #!/usr/bin/env bash
    set -euo pipefail
    image_namespace="{{ image_namespace }}"
    for tool in docker helm kubeconform; do
        command -v "$tool" >/dev/null 2>&1 || { echo "release-dry-run: needs '$tool' on PATH" >&2; exit 1; }
    done
    docker buildx version >/dev/null 2>&1 \
        || { echo "release-dry-run: needs 'docker buildx' (the workflow uses it for every build)" >&2; exit 1; }

    # The host platform, spelled the way buildx spells it. Not read from
    # `docker version --format` because a Go template's braces collide with
    # just's own interpolation syntax.
    case "$(uname -m)" in
        x86_64|amd64)  platform=linux/amd64 ;;
        aarch64|arm64) platform=linux/arm64 ;;
        *) echo "release-dry-run: unknown host arch '$(uname -m)'; the release images are amd64 and arm64 only" >&2; exit 1 ;;
    esac
    echo "==> building for $platform only (see this recipe's comment for why not both)"

    # name:dockerfile:target — the same four the release matrix builds, from
    # the repository root, which is the context BOTH Dockerfiles require:
    # `backends/Dockerfile` COPYs `sdks/rust` and `examples/merchant-demo`
    # because cargo refuses to load a workspace whose `members` list names a
    # missing directory, and `frontends/Dockerfile`'s `checkout` target COPYs
    # `sdks/` so `@vaam-apps/vpay-stripe-js` resolves.
    #
    # No `--build-arg VPAY_GIT_SHA` here, deliberately, and that is a
    # difference from the workflow rather than an omission: a dry run is not
    # a release, its images are never pushed, and stamping one with a real
    # commit would produce a `vpay_build_info` label for an artefact nobody
    # can pull. The default (`unknown`) is the true answer for these.
    for spec in vpay-server:backends/Dockerfile:server \
                vpay-worker:backends/Dockerfile:worker \
                vpay-dashboard:frontends/Dockerfile:runner \
                vpay-checkout:frontends/Dockerfile:checkout; do
        IFS=: read -r name file target <<<"$spec"
        echo "==> $name ($file, target $target)"
        docker buildx build \
            --platform "$platform" \
            --file "$file" \
            --target "$target" \
            --tag "ghcr.io/$image_namespace/$name:dry-run" \
            --provenance=false \
            --sbom=false \
            --push=false \
            .
    done

    echo "==> helm-check"
    just helm-check

    echo "release-dry-run: ok — four images built for $platform, chart checked."
    echo "release-dry-run: NOT covered: the other architecture, provenance/SBOM,"
    echo "release-dry-run: push-by-digest, the manifest merge, and cosign signing."


# -------------------------------------------------------------- dev loop ---

up:
    docker compose up -d
    @echo "postgres :5432 · wiremock-mtn :8081 · wiremock-orange :8082"

# A throwaway RS256 key for the merchant OP in the compose e2e stack
# (compose.e2e.yml mounts it at /secrets/oauth-signing-key.pem). Uses
# openssl rather than `cargo xtask gen-signing-key` so the CI e2e job needs
# no Rust toolchain; the two produce interchangeable PKCS#8 PEMs. 0644 on
# purpose: the scratch image runs as UID 65532 and must read the bind
# mount. Never use this recipe for a real deployment key — `.e2e/` is
# git-ignored and the file is meant to be discarded with the stack.
gen-e2e-signing-key:
    #!/usr/bin/env bash
    set -euo pipefail
    # Checked up front: with `2>/dev/null` on the openssl call below, bash's
    # own "command not found" would be swallowed and this recipe would exit
    # 127 with no output at all (found in review, 2026-09-02).
    for tool in openssl; do
        command -v "$tool" >/dev/null 2>&1 || { echo "gen-e2e-signing-key: needs '$tool' on PATH" >&2; exit 1; }
    done
    mkdir -p .e2e
    if [ -e .e2e/oauth-signing-key.pem ]; then
        echo "gen-e2e-signing-key: .e2e/oauth-signing-key.pem already exists, keeping it"
        exit 0
    fi
    openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 -out .e2e/oauth-signing-key.pem 2>/dev/null
    chmod 0644 .e2e/oauth-signing-key.pem
    echo "gen-e2e-signing-key: wrote .e2e/oauth-signing-key.pem (throwaway, 3072-bit RSA, PKCS#8)"

down:
    docker compose down -v

# ------------------------------------------------------------------ demo ---

# The three files, in the order that makes the last one win. Spelled once so a
# `demo` and a `demo-down` can never disagree about which stack they mean —
# `down -v` against a different file set leaves volumes behind.
demo_compose := "-f compose.yml -f compose.e2e.yml -f compose.demo.yml"

# The eight services `just demo-up` starts, named rather than left to
# compose's "everything in the file set". The ninth is `dashboard`, and
# leaving it out is a statement, not an optimisation: per docs/status.md it
# renders a static scaffold notice and makes no call to `vpay-server`, so
# there is no data source that could show the payments this walkthrough makes
# — see docs/runbooks/demo.md's "what this does not prove".
#
# `vpay-checkout` and `vpay-shop` joined the list in Step 9 and had to: a
# service that is in the file set but not in this list does not start, and the
# hosted session `demo-walk` now prints a URL for is served by the first of
# them.
#
# `just stripe-compat` still brings up its own SIX — it drives `/v1` and needs
# no browser surface, and building two Next.js images for it would cost
# minutes it does not buy anything with.
demo_services := "postgres wiremock-mtn wiremock-orange wiremock-webhook vpay-server vpay-worker vpay-checkout vpay-shop"

# The six `just stripe-compat` brings up. Spelled separately from
# `demo_services` above rather than derived from it, because the difference is
# a decision (no browser surface in a `/v1` conformance run) and not an
# accident of ordering.
compat_services := "postgres wiremock-mtn wiremock-orange wiremock-webhook vpay-server vpay-worker"

# The COMPOSE PROJECT NAME the demo stack lives under, and the variable that
# makes two demos on one machine possible:
#
#     just demo_project=vpay-demo-b demo_port=18088 demo_receiver_port=18089 demo
#
# `compose.demo.yml` reads it as `${VPAY_DEMO_PROJECT}` (that file's `name:`),
# and EVERY recipe below exports it, so `demo-up`, `demo-walk`, `demo-status`
# and `demo-down` all address the same stack — a `demo-down` that derived a
# different project would leave the containers and the volumes of the one you
# started, and report success.
#
# It is a project name, not a port: compose matches containers by project and
# label, so `just demo_project=vpay-demo-b demo-down` needs no port to tear
# down what the line above brought up.
#
# The default is `vpay-demo` and not `vpay`, which is what compose.demo.yml
# hardcoded until Step 8. `vpay` is `just up`'s and CI's e2e project, and
# sharing it meant `just demo` adopted a running development stack and `just
# demo-down` deleted its volumes.
demo_project := "vpay-demo"

# The host port `just demo` publishes `vpay-server` on. Override it per
# invocation when 8080 is taken on your machine:
#
#     just demo_port=18080 demo
#
# `just demo-down` needs no port — see `demo_project` above and that recipe.
#
# Three things have to agree about this number or the demo fails in a way that
# does not name the port, which is why it is one variable and not three:
#
#   1. the published port (`compose.demo.yml` reads `$VPAY_DEMO_PORT`, which
#      the recipes below export). Only the DEMO stack is remapped —
#      `compose.e2e.yml` still publishes 8080:8080 for CI, and compose.demo.yml
#      overrides that mapping rather than adding to it;
#   2. `deployment.public_base_url` in the generated `.e2e/application-demo.yml`
#      overlay. The OP's `issuer` is derived from it (`vpay_api::op::issuer_for`),
#      it is what every access token carries as `iss`, and the SDK derives the
#      same string from its own base URL — a mismatch is an `invalid_client`
#      at the token endpoint with nothing in it pointing at a port;
#   3. `VPAY_BASE_URL` for `examples/merchant-demo`, which runs on the host.
#
# `gen-demo-keys` regenerates the overlay when this changes (its shape check
# covers the URL), so switching ports needs no manual cleanup.
demo_port := "8080"

# The host port `just demo` publishes the merchant webhook receiver
# (`wiremock-webhook`) on. Override it the same way when 8083 is taken:
#
#     just demo_receiver_port=18083 demo
#
# Only two things have to agree about this one, and neither is inside the
# stack: the published port (`compose.demo.yml` reads `$VPAY_DEMO_RECEIVER_PORT`),
# and `VPAY_RECEIVER_URL` for `examples/merchant-demo`'s step 7, which reads
# the receiver's `/__admin/requests` journal from the host. The CONTAINER port
# stays 8080 — that is what `.e2e/application-demo.yml`'s `webhooks[0].url`
# names over the compose network — so unlike `demo_port` this one is not
# baked into any generated file and changing it needs no regeneration.
demo_receiver_port := "8083"

# The host port `just demo` publishes the ORANGE rail stub (`wiremock-orange`)
# on, so a payer's browser can reach the rail's hosted page.
#
#     just demo_orange_port=18082 demo
#
# Orange is a redirect rail. `vpay-server` answers a confirm with
# `next_action.redirect_to_url.url`, that URL is on this container
# (`/stub-hosted-page/{pay_token}`, served by
# `backends/tests/conformance/wiremock/orange/mappings/stub-hosted-page.json`
# since Step 9), and following it is the demo's redirect leg. Until Step 9
# `compose.demo.yml` reset this publication, so the URL the demo printed could
# not be opened at all.
#
# **Any value works, and until Step 9 lane 4 only 8082 did.** The rail's
# `payment_url` comes from a WireMock mapping that spells a literal
# `http://localhost:8082`: WireMock renders a response from the current
# request alone, and vpay's submit reaches the stub over the compose network
# as `wiremock-orange:8080`, so the stub cannot learn what the host published
# it on. Two things therefore have to agree about this number:
#
#   1. the published port (`compose.demo.yml` reads `$VPAY_DEMO_ORANGE_PORT`,
#      which the recipes below export);
#   2. the `payment_url` literal in the mappings that stub is serving.
#
# Lane 2 made (2) a *check*: `gen-demo-keys` read the port out of the
# committed mapping and refused the pair when they disagreed, because that
# file is shared with `compose.yml`, CI's e2e stack and both Rust suites and
# is not a per-run artefact. The honest consequence, which lane 2 wrote down:
# two concurrent demos then collided on 8082 unless one of them edited a
# committed file.
#
# `gen-demo-keys` now COPIES the committed mappings into
# `.e2e/<demo_project>/wiremock-orange/mappings/` with that literal
# substituted, and `compose.demo.yml` mounts the copy. The committed tree is
# untouched and stays the CI/e2e default; the check is gone because the
# substitution makes it unnecessary. `.e2e/` is git-ignored throwaway state,
# and the copy is keyed on `demo_project`, so two demos on two ports have two
# stubs and neither rewrites the other's.
demo_orange_port := "8082"

# The host port `just demo` publishes vpay's own CHECKOUT PAGE
# (`vpay-checkout`, `frontends/apps/checkout`) on — the page a payer is sent
# to by a hosted Checkout Session, and the page a merchant frames for an
# embedded one.
#
#     just demo_checkout_port=13080 demo
#
# Three things have to agree about it, and unlike the Orange stub's port all
# three are things this recipe set can write:
#
#   1. the published port (`compose.demo.yml` reads `$VPAY_DEMO_CHECKOUT_PORT`);
#   2. `checkout.public_base_url` in the generated `.e2e/application-demo.yml`
#      — the origin EVERY payer link vpay mints is built on
#      (`{base}/c/{cs_id}#…`, `{base}/e/{cs_id}?key=…`, and the return page a
#      redirect rail sends the payer back to). A stale value here is a
#      `session.url` that resolves to nothing, and no message names a port;
#   3. `VPAY_CHECKOUT_URL` on `vpay-shop`, which is the iframe `src` of the
#      shop's embedded page.
#
# `gen-demo-keys` regenerates the overlay when this changes.
#
# 3080 and not 3000 or 3001: `compose.e2e.yml` publishes the dashboard on
# 3000 and this stack publishes the shop on 3001.
demo_checkout_port := "3080"

# The host port `just demo` publishes the demo SHOP (`vpay-shop`,
# `examples/shop`) on — the merchant site the end-to-end demo is.
#
#     just demo_shop_port=13001 demo
#
# Three things have to agree about it:
#
#   1. the published port (`compose.demo.yml` reads `$VPAY_DEMO_SHOP_PORT`);
#   2. `SHOP_PUBLIC_URL` on the service, which is what the shop puts in the
#      `success_url`/`cancel_url` it sends vpay — i.e. where a payer lands
#      after paying;
#   3. `shop-merchant`'s `checkout_origins` in the generated overlay, which is
#      what becomes `Content-Security-Policy: frame-ancestors` on vpay's
#      embedded page. Wrong, and the browser refuses to render the iframe with
#      nothing in any server log saying why.
#
# `gen-demo-keys` regenerates the overlay when this changes, and its shape
# check is keyed on this port for exactly reason (3).
#
# 3001, not 3000: the shop's CONTAINER listens on 3000 and so does the
# dashboard's, which `compose.e2e.yml` already publishes on 3000. (It is also
# the port `pnpm --filter @vpay/checkout dev` uses for its own dev server, so
# do not run both at once without moving one.)
demo_shop_port := "3001"

# Everything `just demo` needs on disk before a container starts: the server's
# own OP signing key (the `gen-e2e-signing-key` dependency above) and the demo
# merchant's key pair plus the profile overlay that registers its PUBLIC half.
#
# Idempotent, and deliberately treats the key and the overlay as ONE artefact.
# The overlay carries the public JWK of that exact private key; a leftover key
# with no overlay (or the reverse) authenticates nothing, and the failure is an
# `invalid_client` with nothing in it pointing at a stale file. So: both
# present, keep both; anything else, regenerate both. That is safe because
# `.e2e/` is git-ignored throwaway state by construction — never point this
# recipe at a directory holding a key you cannot lose.
#
# `cargo xtask gen-signing-key` rather than the `openssl` call
# `gen-e2e-signing-key` uses: the JWK has to be extracted, and xtask already
# prints the RFC 7638 `kid` and the JWK JSON that `vpay-api` will compute for
# itself. Reproducing that in shell would be a second implementation of a
# thumbprint. The cost is that this recipe needs the Rust toolchain — fine, so
# does the demo binary it exists to feed.
#
# Generate the demo's throwaway keys and its profile overlay (idempotent).
gen-demo-keys: gen-e2e-signing-key
    #!/usr/bin/env bash
    set -euo pipefail
    for tool in cargo jq; do
        command -v "$tool" >/dev/null 2>&1 || { echo "gen-demo-keys: needs '$tool' on PATH" >&2; exit 1; }
    done

    # The Orange stub's mappings, COPIED into `.e2e/<demo_project>/` with the
    # payer-facing port substituted — BEFORE the overlay's early exit below,
    # because this is not about `.e2e/application-demo.yml` and a stack whose
    # keys are fine can still print a redirect URL nothing serves.
    #
    # Why a copy rather than an edit: `webpayment.json` templates
    # `payment_url` on a literal `http://localhost:8082`, because WireMock
    # renders a response from the current request alone and vpay's submit
    # arrives over the compose network as `wiremock-orange:8080` — the stub
    # cannot learn what the host published it on. That file is committed and
    # shared with `compose.yml`, CI's e2e stack and both Rust suites, so it is
    # not this recipe's to rewrite. `compose.demo.yml` mounts the copy over
    # `/home/wiremock` instead (compose merges `volumes:` by target path), and
    # the copy is keyed on `demo_project` so two concurrent demos have two
    # stubs on two ports. See the `demo_orange_port` variable.
    #
    # Regenerated unconditionally: it is derived, it is cheap (five JSON
    # files), and a staleness check on a directory is more code than the copy
    # it would be guarding.
    orange_src=backends/tests/conformance/wiremock/orange
    orange_gen=.e2e/{{demo_project}}/wiremock-orange
    if ! grep -q 'localhost:[0-9]\{1,\}/stub-hosted-page' "$orange_src/mappings/webpayment.json"; then
        echo "gen-demo-keys: FAIL — no /stub-hosted-page payment_url found in" >&2
        echo "gen-demo-keys:   $orange_src/mappings/webpayment.json" >&2
        echo "gen-demo-keys: the demo's redirect leg would have nothing to point a browser at," >&2
        echo "gen-demo-keys: and this recipe would have nothing to substitute a port into." >&2
        exit 1
    fi
    rm -rf "$orange_gen"
    mkdir -p "$orange_gen"
    cp -R "$orange_src/." "$orange_gen/"
    # Only `localhost:<port>/stub-hosted-page` is rewritten, and nothing else:
    # the rail's own host inside the mappings is `wiremock-orange:8080` and
    # must stay that, and `8080`/`8082` appear in prose in the `metadata`
    # blocks. Anchoring on the path segment is what keeps this from being a
    # blind port substitution over a JSON document.
    find "$orange_gen" -name '*.json' -print0 \
        | xargs -0 sed -i 's|localhost:[0-9]\{1,\}/stub-hosted-page|localhost:{{demo_orange_port}}/stub-hosted-page|g'
    # The post-condition, checked rather than assumed: no OTHER port survives
    # in a payer-facing URL. A `sed` that silently matched nothing would
    # otherwise leave the demo pointing at 8082 with this recipe reporting
    # success.
    if grep -rn 'localhost:[0-9]\{1,\}/stub-hosted-page' "$orange_gen" \
        | grep -v 'localhost:{{demo_orange_port}}/stub-hosted-page' >&2; then
        echo "gen-demo-keys: FAIL — the lines above still name a port other than" >&2
        echo "gen-demo-keys: {{demo_orange_port}} in a payer-facing URL under $orange_gen." >&2
        exit 1
    fi
    substituted=$(grep -rc 'localhost:{{demo_orange_port}}/stub-hosted-page' "$orange_gen" \
        | awk -F: '{n += $2} END {print n + 0}')
    if [ "$substituted" -eq 0 ]; then
        echo "gen-demo-keys: FAIL — wrote $orange_gen with no payer-facing URL in it." >&2
        exit 1
    fi
    # WireMock's container reads this tree; the demo's is throwaway and holds
    # nothing secret.
    chmod -R a+rX "$orange_gen"
    echo "gen-demo-keys: wrote $orange_gen — $substituted payer-facing URLs on localhost:{{demo_orange_port}}"

    key=.e2e/demo-merchant/oauth-signing-key.pem
    # The SHOP's own key pair (D12: `examples/shop` is its own merchant, not
    # `demo-merchant`). Treated as part of the same one artefact as the demo
    # merchant's key and the overlay — all three are written by one heredoc's
    # worth of state, and two of three present is a stack that authenticates
    # one merchant and answers `invalid_client` to the other.
    shop_key=.e2e/shop-merchant/oauth-signing-key.pem
    overlay=.e2e/application-demo.yml

    # The MERCHANT's endpoint list, which is a different thing from the
    # top-level `webhooks:` block and lives at a different indentation:
    #
    #     webhooks:                 <- top level, Step 8's allow_private_targets
    #       allow_private_targets: true
    #     merchant_clients:
    #       - client_id: demo-merchant
    #         webhooks:             <- THIS one, the endpoint list
    #           - id: demo
    #             url: http://wiremock-webhook:8080/webhooks
    #
    # A bare `grep -q '^\s*webhooks:'` matches both, so from the moment Step 8
    # added the top-level block the merchant check could no longer fail.
    # Reproduced by the Step 8 review: delete the indented block, keep the
    # top-level one, and `just gen-demo-keys` answered "already exist, keeping
    # them" — after which the demo's webhook step fails against a receiver
    # nothing points at, which is exactly the failure this check exists to
    # pre-empt. Anchored on the endpoint URL as well as on the indentation,
    # because the URL line cannot appear anywhere but inside that list.
    merchant_webhooks_present() {
        grep -qE '^[[:space:]]+webhooks:[[:space:]]*$' "$overlay" \
            && grep -qE '^[[:space:]]+url: http://wiremock-webhook' "$overlay"
    }

    # Step 9. `checkout.public_base_url` is a TOP-LEVEL block whose one key is
    # spelled exactly like `deployment.public_base_url`, so a bare
    # `grep -q '^\s*public_base_url: …'` cannot tell the two apart — the same
    # trap `merchant_webhooks_present` above documents for `webhooks:`. This
    # one is anchored on the block header and the two lines after it.
    checkout_base_present() {
        grep -A2 '^checkout:$' "$overlay" \
            | grep -qE '^  public_base_url: http://localhost:{{demo_checkout_port}}$'
    }

    # The origin that becomes `Content-Security-Policy: frame-ancestors` on
    # vpay's embedded page for the shop. It has to name the port the shop was
    # actually published on, and it is checked separately from
    # `client_id: shop-merchant` because the two go stale for different
    # reasons: the entry is missing on an overlay generated before Step 9, and
    # the port is wrong the moment somebody passes `demo_shop_port=`.
    # `merchant_clients` is a list, so this entry's key is the FIRST line of a
    # YAML sequence item: `  - client_id: shop-merchant`, not
    # `    client_id: …`. A `^\s*client_id:` pattern misses it entirely — it
    # did, on the first run of this recipe, which reported "regenerating"
    # forever on an overlay that was perfectly fresh.
    shop_client_present() {
        grep -qE '^[[:space:]]*-[[:space:]]+client_id: shop-merchant$' "$overlay"
    }

    shop_origin_present() {
        grep -qF 'checkout_origins: ["http://localhost:{{demo_shop_port}}"]' "$overlay"
    }

    # Step 9, and NARROWED 2026-09-04 (r2 review, finding 6). This used to be
    # a bare `grep -q '^\s*- code: mtn_momo$'` — a PRESENCE proxy, not a
    # currency check. Every overlay that has a `providers:` block at all
    # carries that line, INCLUDING one whose `mtn_momo` entry has been edited
    # back to `currency: EUR`, which is the one state the check exists to
    # catch. Measured before the change: flip the generated overlay's line to
    # EUR and `just gen-demo-keys` answered "already exist, keeping them",
    # after which every confirm of the shop's XAF orders on MTN is refused
    # with `rail 'mtn_momo' settles in EUR; this PaymentIntent is XAF` — true,
    # and naming no file anyone could fix. Keyed on the currency INSIDE the
    # `mtn_momo` sequence item now, the way `merchant_webhooks_present` and
    # `checkout_base_present` above are keyed on shape rather than on a name.
    #
    # The awk range runs from the `- code: mtn_momo` line to the next sequence
    # item or the next top-level key, whichever comes first, so a
    # `currency: XAF` belonging to `orange_money` (which has one) cannot
    # satisfy it.
    mtn_settles_xaf() {
        awk '
            /^[[:space:]]*-[[:space:]]+code:[[:space:]]*mtn_momo[[:space:]]*$/ { in_mtn = 1; next }
            in_mtn && /^[^[:space:]]/ { in_mtn = 0 }
            in_mtn && /^[[:space:]]*-[[:space:]]+code:/ { in_mtn = 0 }
            in_mtn && /^[[:space:]]*currency:[[:space:]]*XAF[[:space:]]*$/ { found = 1 }
            END { exit found ? 0 : 1 }
        ' "$overlay"
    }

    if [ -e "$key" ] && [ -e "$shop_key" ] && [ -e "$overlay" ]; then
        # ...unless the overlay predates a required field. `merchant_id`
        # became required on `merchant_clients` in Step 2, and an overlay
        # generated before that makes the server exit 78 on every restart
        # with `missing field \`merchant_id\`` — while this recipe cheerfully
        # says it kept a file that no longer loads. Measured: `just demo`
        # spent its whole 120 s readiness budget on a crash loop. The check is
        # on the *shape* rather than a version stamp because the overlay is
        # generated, git-ignored, and cheap to rebuild.
        #
        # The same check now covers `deployment.public_base_url`, for the
        # same class of failure: `just demo_port=18080 demo` against an
        # overlay generated for 8080 mints tokens whose `iss` is
        # `http://localhost:8080/v1/oauth` while the SDK, reading its own
        # base URL, signs assertions for `http://localhost:18080/v1/oauth`.
        # The OP answers `invalid_client` and neither message mentions a
        # port. The overlay is generated and git-ignored, so it is rebuilt
        # rather than patched.
        if grep -q '^\s*merchant_id:' "$overlay" \
            && merchant_webhooks_present \
            && grep -q '^\s*publishable_keys:' "$overlay" \
            && grep -q '^\s*allow_private_targets:' "$overlay" \
            && grep -q "^\s*public_base_url: http://localhost:{{demo_port}}$" "$overlay" \
            && shop_client_present \
            && shop_origin_present \
            && checkout_base_present \
            && mtn_settles_xaf; then
            echo "gen-demo-keys: $key, $shop_key and $overlay already exist, keeping them"
            exit 0
        fi
        if ! grep -q '^\s*merchant_id:' "$overlay"; then
            echo "gen-demo-keys: $overlay predates the required \`merchant_id\` field — regenerating the pair"
        elif ! merchant_webhooks_present; then
            # Added 2026-09-03 (Step 5). Not fatal the way a missing
            # `merchant_id` is — the overlay still loads — but the demo's
            # webhook step would then poll a receiver no endpoint points at
            # and fail for a reason that has nothing to do with the worker.
            # Same class of stale-generated-file failure, so the same shape
            # check. Narrowed 2026-09-04: see `merchant_webhooks_present`.
            echo "gen-demo-keys: $overlay is missing the merchant's \`webhooks\` endpoint list — regenerating the pair"
        elif ! grep -q '^\s*allow_private_targets:' "$overlay"; then
            # Added 2026-09-03 (Step 8). The worst of the three to inherit
            # silently: an overlay without it gets the shipping default —
            # private targets refused — and `wiremock-webhook` is a compose
            # service, so every delivery would be recorded `ssrf_blocked` and
            # the demo's webhook step would fail naming a receiver that is
            # working perfectly.
            echo "gen-demo-keys: $overlay predates the \`webhooks.allow_private_targets\` flag — regenerating the pair"
        elif ! grep -q '^\s*publishable_keys:' "$overlay"; then
            # Added 2026-09-03 (Step 5c). Same class again: the overlay still
            # loads without it, but `examples/checkout-browser` and the
            # Cypress spec would then present a key `merchant_id_for_publishable_key`
            # resolves to nothing, and every browser call would be the
            # surface's uniform 404 — a refusal that deliberately names
            # neither the key nor the reason.
            echo "gen-demo-keys: $overlay predates the \`publishable_keys\` block — regenerating the pair"
        elif ! grep -q "^\s*public_base_url: http://localhost:{{demo_port}}$" "$overlay"; then
            echo "gen-demo-keys: $overlay was generated for a different demo_port than {{demo_port}} — regenerating the pair"
        elif ! shop_client_present; then
            # Added 2026-09-04 (Step 9, D12). An overlay generated before this
            # step registers `demo-merchant` and nobody else, so the shop's
            # very first token request answers `invalid_client` — with nothing
            # in it pointing at a stale file, which is the whole class of
            # failure the `merchant_id` and `publishable_keys` checks above
            # exist for.
            echo "gen-demo-keys: $overlay has no \`shop-merchant\` registration — regenerating the pair"
        elif ! shop_origin_present; then
            # Added 2026-09-04 (Step 9, D4/D12). Not fatal at boot: the
            # overlay loads and hosted checkout is unaffected. What breaks is
            # the EMBEDDED page — vpay serves
            # `Content-Security-Policy: frame-ancestors <the old origin>` and
            # the browser refuses to render the iframe, silently, with a
            # console message on the payer's machine and nothing at all in any
            # server log.
            echo "gen-demo-keys: $overlay does not allow http://localhost:{{demo_shop_port}} to frame the checkout page — regenerating the pair"
        elif ! checkout_base_present; then
            # Added 2026-09-04 (Step 9, D3/D6). `checkout.public_base_url` is
            # the origin every payer link vpay mints is built on. Stale, and
            # `POST /v1/checkout/sessions` answers a `url` on a port this
            # stack does not publish — the payer's browser gets a connection
            # refused and no log anywhere names a port.
            echo "gen-demo-keys: $overlay was generated for a different demo_checkout_port than {{demo_checkout_port}} — regenerating the pair"
        else
            # Added 2026-09-04 (Step 9, lane 7 addendum A); keyed on the
            # currency rather than on the block's presence 2026-09-04 (r2
            # review, finding 6 — see `mtn_settles_xaf`). Two overlays land
            # here: one generated before that step, which carries no
            # `providers:` block at all so the base file's stands and
            # `mtn_momo` settles EUR, and one whose `mtn_momo` entry has been
            # edited to EUR by hand. Either way `vpay_api`'s
            # `currencies_agree` refuses every confirm of the shop's XAF
            # orders on MTN with `rail 'mtn_momo' settles in EUR; this
            # PaymentIntent is XAF`. Which is true, and names no file anyone
            # could fix.
            echo "gen-demo-keys: $overlay does not settle \`mtn_momo\` in XAF — regenerating the pair"
        fi
    elif [ -e "$key" ] || [ -e "$shop_key" ] || [ -e "$overlay" ]; then
        echo "gen-demo-keys: $key, $shop_key and $overlay are out of sync — regenerating all three"
    fi
    rm -f "$key" "$shop_key" "$overlay"

    generated=$(cargo xtask gen-signing-key --out .e2e/demo-merchant)
    jwk=$(printf '%s\n' "$generated" | grep -m1 '^{"kty"' || true)
    if [ -z "$jwk" ]; then
        echo "gen-demo-keys: FAIL — could not find the public JWK in xtask's output:" >&2
        printf '%s\n' "$generated" >&2
        exit 1
    fi
    n=$(printf '%s' "$jwk" | jq -er .n)
    e=$(printf '%s' "$jwk" | jq -er .e)
    kid=$(printf '%s' "$jwk" | jq -er .kid)

    # The shop's pair (D12). A second `merchant_clients` entry, not a second
    # overlay: `merchant_clients` is a LIST, and a list in a profile overlay
    # replaces the base list wholesale, so both entries have to be in the one
    # generated document.
    shop_generated=$(cargo xtask gen-signing-key --out .e2e/shop-merchant)
    shop_jwk=$(printf '%s\n' "$shop_generated" | grep -m1 '^{"kty"' || true)
    if [ -z "$shop_jwk" ]; then
        echo "gen-demo-keys: FAIL — could not find the shop's public JWK in xtask's output:" >&2
        printf '%s\n' "$shop_generated" >&2
        exit 1
    fi
    shop_n=$(printf '%s' "$shop_jwk" | jq -er .n)
    shop_e=$(printf '%s' "$shop_jwk" | jq -er .e)
    shop_kid=$(printf '%s' "$shop_jwk" | jq -er .kid)

    # 0644, NOT the 0600 `demo-merchant`'s key gets, and the difference is the
    # whole point: unlike the demo binary — which runs on the HOST, as a
    # merchant's own process would — the shop runs INSIDE compose, so its
    # private key is bind-mounted into a container that runs as uid 1000.
    # Throwaway, git-ignored, regenerated per checkout; never point this
    # recipe at a directory holding a key you cannot lose.
    chmod 0644 "$shop_key"

    # `merchant_clients` is a LIST, and a list in a profile overlay replaces
    # the base list outright (figment merges maps, not sequences). So this one
    # entry is the whole registry under the `demo` profile — `acme-cameroon`
    # from config/application.yml is deliberately gone; its modulus is a
    # placeholder nobody holds a key for.
    cat > "$overlay" <<YAML
    # GENERATED by \`just gen-demo-keys\` — do not edit, do not commit.
    #
    # The \`demo\` profile overlay (VPAY_PROFILE=demo, mounted at
    # /config/application-demo.yml by compose.demo.yml). It registers the one
    # merchant \`examples/merchant-demo\` authenticates as. Only the PUBLIC half
    # of the key is here; the private half stays in $key, mode 0600, and is
    # never mounted into a container.
    #
    # Regenerate with: rm -rf .e2e/demo-merchant .e2e/application-demo.yml && just gen-demo-keys

    deployment:
      name: vpay-demo
      # The port \`just demo\` publishes on ({{demo_port}}), which the OP turns
      # into its \`issuer\` and the SDK independently derives from
      # VPAY_BASE_URL. See the \`demo_port\` variable in the justfile for what
      # goes wrong when the two disagree.
      public_base_url: http://localhost:{{demo_port}}

    # BOTH RAILS SETTLE XAF IN THIS STACK, AND ONLY IN THIS STACK.
    #
    # \`config/application.yml\` puts \`mtn_momo\` on EUR because **MTN's real
    # sandbox rejects XAF** (docs/flows/money.md) — that is the sandbox truth
    # and it is unchanged; the base file still says EUR and so does
    # \`application-sandbox.yml\`. What this stack talks to is not the sandbox:
    # it is a WireMock host that accepts whatever currency the body carries,
    # because none of its mappings matches on one.
    #
    # Why change it at all: the demo shop (\`examples/shop\`) prices its
    # catalogue in XAF, which is Cameroon's currency and docs/flows/money.md's
    # worked example, and offers a payer both rails.
    # \`vpay_api::v1::payment_intents::currencies_agree\` refuses a confirm
    # whose rail settles in a different currency from the intent — so with
    # \`mtn_momo\` on EUR the shop's MTN button is a 400 the payer cannot get
    # past. Configuration, never a code branch (ADR-0003).
    #
    # \`providers\` is a LIST, so this block replaces the base file's outright
    # and both rails have to be written out even though only one line differs
    # from it. The \`\$\` escapes keep the credential placeholders literal in
    # the generated file: they are resolved from compose.e2e.yml's
    # environment at boot, and a literal here would be a secret in a file.
    #
    # **Do not read this block as "MTN takes XAF".** It does not. See
    # docs/flows/money.md and this stack's own README of a mapping,
    # backends/tests/conformance/wiremock/mtn/mappings/requesttopay-status.json.
    providers:
      - code: mtn_momo
        host:
          url: http://wiremock-mtn:8080
          label: mtn-sandbox-wiremock
        # XAF here, EUR in config/application.yml. Read the block comment
        # above before copying this line anywhere.
        currency: XAF
        settings:
          subscription_key_header: Ocp-Apim-Subscription-Key
          target_environment: sandbox
          api_user: \${MTN_API_USER}
        credentials:
          subscription_key: \${MTN_SUBSCRIPTION_KEY}
          api_key: \${MTN_API_KEY}
      - code: orange_money
        host:
          # The \`/orange-money-webpay/{env}\` path prefix is part of the base
          # URL by design (docs/flows/adapter-orange-money.md); the stub's
          # mappings match only under it.
          url: http://wiremock-orange:8080/orange-money-webpay/dev
          label: orange-sandbox-wiremock
        currency: XAF
        settings:
          env: dev
          lang: en
        credentials:
          merchant_key: \${ORANGE_MERCHANT_KEY}
          client_id: \${ORANGE_CLIENT_ID}
          client_secret: \${ORANGE_CLIENT_SECRET}

    # Where vpay's own checkout page is served from (Step 9, D3/D6) — the
    # origin every payer link vpay mints is built on:
    #
    #   hosted    {base}/c/{cs_id}?key={pk}#{client_secret}
    #   embedded  {base}/e/{cs_id}?key={pk}#{client_secret}
    #   return    {base}/c/{cs_id}/return?t={return_token}&key={pk}
    #
    # The port \`just demo\` published \`vpay-checkout\` on
    # ({{demo_checkout_port}}). A value that disagrees with the publication is
    # a \`session.url\` a browser cannot open, and nothing in any log names a
    # port — which is why this recipe's shape check covers it.
    #
    # \`http://\` is legal only because this overlay does not set
    # \`livemode\`, so the base file's \`false\` stands
    # (ConfigError::InsecureCheckoutBaseUrl otherwise).
    checkout:
      public_base_url: http://localhost:{{demo_checkout_port}}

    # The demo's receiver is \`wiremock-webhook\`, a service on the compose
    # network, so every delivery resolves to a private address — and
    # \`vpay_worker::ssrf\` refuses those by default. This one value changes the
    # verdict and nothing else: the guard still resolves the host once and
    # still pins the connection to what it resolved (ADR-0003 — a profile
    # selects a file, never a code path).
    #
    # This overlay does not set \`livemode\`, so the base file's \`false\` stands.
    # It has to: \`livemode: true\` with this flag is a refusal to boot
    # (ConfigError::PrivateWebhookTargetsInLivemode).
    webhooks:
      allow_private_targets: true

    merchant_clients:
      - client_id: demo-merchant
        # The tenant, separate from the credential: every payment intent the
        # demo creates carries this as its merchant_id and every query it
        # makes is filtered by it (vpay_config::MerchantClient::merchant_id).
        merchant_id: demo-merchant-tenant
        jwks:
          keys:
            - kty: RSA
              use: sig
              alg: RS256
              # Informational: the assertion the SDK mints carries no \`kid\`,
              # and the OP's \`select_key\` does not need one while the set holds
              # exactly one key. Adding a second key here without teaching the
              # demo to send a \`kid\` would break authentication.
              kid: "$kid"
              n: "$n"
              e: "$e"
        grant_types: [client_credentials]
        # What the demo is authorised to do. The SDK asks for no scope, so
        # this list is what its token carries (RFC 6749 §3.3 default scope,
        # applied in vpay_api::op::token::token_handler) — and \`payments:write\`
        # is what /v1 requires on the demo's POST /v1/payment_intents. An
        # empty list here would still mint a token and then 403 every call.
        scopes: ["payments:write"]
        # Must contain vpay:v1 — vpay_config::MERCHANT_AUDIENCE. Without it the
        # OP answers invalid_target, and the server refuses to boot.
        allowed_audiences: ["vpay:v1"]
        # What a payer's browser presents on /v1/browser alongside the payment
        # intent's own client_secret (Step 5c, @vaam-apps/vpay-stripe-js). Not secret:
        # it is rendered into the checkout page, it names this tenant, and it
        # authorises nothing on its own.
        #
        # A FIXED value, not generated like the key pair above, and
        # deliberately so: examples/checkout-browser and the Cypress spec
        # both hardcode it, and a per-run value would mean a demo page that
        # has to be regenerated with the overlay. It is a sandbox label for a
        # throwaway stack; there is nothing here to keep secret.
        #
        # \`pk_test_\` because this overlay does not set livemode, so the base
        # config's \`false\` stands — Config::validate_all refuses a \`pk_live_\`
        # key under it (ConfigError::PublishableKeyLivemodeMismatch).
        #
        # Those three backticks are ESCAPED, like every other backtick in this
        # heredoc. The delimiter is unquoted (\`<<YAML\`, so that "\$kid" and
        # "\$n" expand), which means an unescaped backtick pair is COMMAND
        # SUBSTITUTION: until 2026-09-03 these three lines ran \`pk_test_\`,
        # \`false\` and \`pk_live_\` as commands, printed two "command not found"
        # lines into the middle of \`just demo\`, and wrote the comment into the
        # overlay with the backticked words deleted. Harmless as it stood and
        # not harmless as a pattern — a word between backticks here is a
        # command this recipe runs.
        publishable_keys: ["pk_test_demomerchantsandbox01"]
        # Where this merchant's events are POSTed once the worker's job loop
        # delivers them (docs/flows/webhooks.md). \`wiremock-webhook\` is the
        # compose service compose.e2e.yml adds, on its port as seen INSIDE
        # the compose network — the demo binary reads the same container's
        # request journal from the host on \${VPAY_DEMO_RECEIVER_PORT}.
        #
        # The secret is the placeholder both binaries already resolve from
        # compose.e2e.yml's MERCHANT_WEBHOOK_SECRET. The \`\$\` escape is what
        # keeps THIS heredoc from expanding it: the value has to stay a
        # placeholder in the file, or the config would carry a literal and a
        # livemode overlay copied from it would be refused at boot for a
        # reason nobody would connect to this recipe.
        webhooks:
          - id: demo
            url: http://wiremock-webhook:8080/webhooks
            secrets: ["\${MERCHANT_WEBHOOK_SECRET}"]

      # The SHOP is its own merchant (D12), with its own key pair, its own
      # publishable key and its own webhook endpoint. \`demo-merchant\` above
      # and its WireMock receiver are untouched: \`examples/merchant-demo\`'s
      # walkthrough is a different thing from the shop's, and neither should
      # be able to break the other by sharing a credential.
      - client_id: shop-merchant
        merchant_id: shop-merchant-tenant
        jwks:
          keys:
            - kty: RSA
              use: sig
              alg: RS256
              kid: "$shop_kid"
              n: "$shop_n"
              e: "$shop_e"
        grant_types: [client_credentials]
        # \`payments:write\` covers both POST /v1/payment_intents and
        # POST /v1/checkout/sessions, which is the pair the shop makes per
        # order.
        scopes: ["payments:write"]
        allowed_audiences: ["vpay:v1"]
        # Fixed, like demo-merchant's, and for the same reason: the shop's
        # compose environment names it literally (VPAY_PUBLISHABLE_KEY).
        publishable_keys: ["pk_test_shopmerchantsandbox1"]
        # D4 / D12. The shop's own origin, as a BROWSER sees it — this is what
        # becomes \`Content-Security-Policy: frame-ancestors\` on vpay's
        # embedded page. It must name the port \`just demo\` published the shop
        # on ({{demo_shop_port}}); wrong, and the iframe is refused by the
        # browser with nothing in any server log about it.
        #
        # \`http://\` is permitted only because this overlay does not set
        # \`livemode\`, so the base file's \`false\` stands.
        checkout_origins: ["http://localhost:{{demo_shop_port}}"]
        # The shop's own handler, on the compose network. \`vpay-shop\` is a
        # compose service, so this resolves to a private address and rides on
        # the same \`webhooks.allow_private_targets\` flag above that
        # \`wiremock-webhook\` does.
        #
        # The secret must be the SAME BYTES as the shop's own
        # \`VPAY_WEBHOOK_SECRET\` (compose.e2e.yml), or every delivery it
        # verifies is a 400 and no order ever reaches \`paid\`. Left as a
        # placeholder in the file for the reason MERCHANT_WEBHOOK_SECRET is:
        # a literal here would be refused outright by a livemode overlay
        # copied from this one (ConfigError::LiteralSecret).
        webhooks:
          - id: shop
            url: http://vpay-shop:3000/api/vpay/webhook
            secrets: ["\${SHOP_WEBHOOK_SECRET}"]
    YAML
    # The scratch image runs as UID 65532 and must read the bind mount. This
    # file is a public key and a client id; there is nothing secret in it.
    chmod 0644 "$overlay"

    echo "gen-demo-keys: wrote $key (3072-bit RSA, mode 0600, host-only)"
    echo "gen-demo-keys: wrote $shop_key (3072-bit RSA, mode 0644, mounted into vpay-shop)"
    echo "gen-demo-keys: wrote $overlay — client_id=demo-merchant kid=$kid"
    echo "gen-demo-keys: wrote $overlay — client_id=shop-merchant kid=$shop_kid"
    echo "gen-demo-keys: both rails settle XAF in this overlay (config/application.yml keeps mtn_momo on EUR — the sandbox's own currency)"

# Keys, stack, walkthrough — the one command issue #11 asks for.
#
# It is a composition and holds no body of its own, which is the point: `just
# demo` and `just demo-up && just demo-walk` are the SAME two commands, so a
# reader of docs/runbooks/demo.md is never running a path the one-liner does
# not. `demo-walk` prints the URLs and exits with the walkthrough's own
# status, so `just demo` failing still means the demo failed.
#
# Boot the demo stack and run the merchant walkthrough against it.
demo: demo-up demo-walk

# Generate the keys, build the images, bring the EIGHT services of
# `demo_services` up, and return only once the server answers: postgres, both
# WireMock rails, the merchant webhook receiver, `vpay-server`, `vpay-worker`,
# vpay's own checkout page and the demo shop. (It said six until 2026-09-04;
# lane 3 added `vpay-checkout` and lane 7 `vpay-shop` to `demo_services`
# without the sentence following. The dashboard is the one service of the
# file set this recipe deliberately leaves down — see compose.demo.yml.)
#
# Split out of `demo` so the walkthrough is re-runnable against a stack that
# is already up (`just demo-walk`), which is what you want while reading its
# output — and so a stack that will not boot fails HERE, with compose's own
# diagnostics, rather than inside a demo that would blame `/healthz`.
#
# Generate keys, build the images and bring the demo stack up.
demo-up: gen-demo-keys
    #!/usr/bin/env bash
    set -euo pipefail
    # Fail fast with a named tool rather than a 120 s readiness timeout whose
    # message blames /healthz (review finding, 2026-09-02).
    for tool in docker curl; do
        command -v "$tool" >/dev/null 2>&1 || { echo "demo-up: needs '$tool' on PATH" >&2; exit 1; }
    done
    # Read by compose.demo.yml: `${VPAY_DEMO_PROJECT}` is its `name:`, and the
    # five ports are `vpay-server`'s, `wiremock-webhook`'s,
    # `wiremock-orange`'s, `vpay-checkout`'s and `vpay-shop`'s `ports:`.
    # Exported rather than passed per command, because `docker compose`
    # interpolates each file from its own environment.
    export VPAY_DEMO_PROJECT={{demo_project}}
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    export VPAY_DEMO_ORANGE_PORT={{demo_orange_port}}
    export VPAY_DEMO_CHECKOUT_PORT={{demo_checkout_port}}
    export VPAY_DEMO_SHOP_PORT={{demo_shop_port}}
    echo "demo-up: project {{demo_project}}, server :{{demo_port}}, receiver :{{demo_receiver_port}}, orange stub :{{demo_orange_port}}"
    echo "demo-up: checkout page :{{demo_checkout_port}}, shop :{{demo_shop_port}}"

    # `--wait`, not a sleep. Postgres and all three WireMock containers carry
    # healthchecks (compose.yml, compose.e2e.yml), so this returns when the
    # database is accepting connections and each stub has LOADED ITS MAPPINGS
    # — which a TCP probe cannot tell from a JVM that has merely bound its
    # port. Readiness is then a property of the containers, checkable with
    # `docker compose ps`, rather than a number in this file.
    docker compose {{demo_compose}} up -d --build --wait {{demo_services}}

    # The two services `--wait` can only report as *started*: `vpay-server`
    # and `vpay-worker` are `FROM scratch` (ADR-0004) and hold no shell to run
    # a healthcheck in, so neither can have one until the binary grows a
    # `--healthcheck` self-check mode (compose.e2e.yml's own note; not this
    # lane's change). Readiness is therefore observed from outside for the
    # server, exactly as .github/workflows/ci.yml's e2e job does it. This is a
    # poll of a real endpoint, not a fixed wait — it returns as soon as the
    # server answers.
    echo "demo-up: waiting for http://localhost:{{demo_port}}/healthz"
    deadline=$((SECONDS + 120))
    until curl -fsS -o /dev/null http://localhost:{{demo_port}}/healthz; do
        if [ "$SECONDS" -ge "$deadline" ]; then
            echo "demo-up: FAIL — /healthz did not answer within 120s. Last 80 log lines:" >&2
            docker compose {{demo_compose}} ps >&2
            docker compose {{demo_compose}} logs --tail 80 vpay-server >&2
            echo "demo-up: (exit 78 in that log means a config/CLI prerequisite is missing)" >&2
            exit 1
        fi
        sleep 2
    done
    echo "demo-up: /healthz answered"

# Run `examples/merchant-demo` against a stack that is already up.
#
# On the HOST, as a merchant's own process would be, so it needs the
# *published* ports — `demo_port` is the same number the generated overlay's
# `public_base_url` names, or the token exchange is an `invalid_client` that
# mentions no port (see `demo_port`).
#
# Prints the URLs whether or not the walkthrough passed: a failed run is
# exactly when you want to go and read the receiver's journal yourself.
#
# Run the merchant walkthrough against a demo stack that is already up.
demo-walk:
    #!/usr/bin/env bash
    set -uo pipefail
    for tool in cargo curl; do
        command -v "$tool" >/dev/null 2>&1 || { echo "demo-walk: needs '$tool' on PATH" >&2; exit 1; }
    done
    if ! curl -fsS -o /dev/null --max-time 5 http://localhost:{{demo_port}}/healthz; then
        echo "demo-walk: FAIL — nothing answers http://localhost:{{demo_port}}/healthz." >&2
        echo "demo-walk: bring the stack up first: just demo_project={{demo_project}} demo_port={{demo_port}} demo_receiver_port={{demo_receiver_port}} demo-up" >&2
        exit 1
    fi

    VPAY_BASE_URL=http://localhost:{{demo_port}} \
      VPAY_RECEIVER_URL=http://localhost:{{demo_receiver_port}} \
      cargo run -q -p merchant-demo
    status=$?

    echo
    echo "  server      http://localhost:{{demo_port}}"
    echo "  discovery   http://localhost:{{demo_port}}/v1/oauth/.well-known/openid-configuration"
    echo "  receiver    http://localhost:{{demo_receiver_port}}/__admin/requests"
    echo "  checkout    http://localhost:{{demo_checkout_port}}/healthz  (vpay's own payment page)"
    echo "  shop        http://localhost:{{demo_shop_port}}                (the demo merchant's storefront)"
    echo "  orange stub http://localhost:{{demo_orange_port}}/__admin/requests"
    echo "  (an orange_money confirm answers next_action.redirect_to_url.url on that host:"
    echo "   open it for the RAIL's stub hosted page, with a Pay link and a Cancel link)"
    echo "  rail journal  docker compose {{demo_compose}} exec wiremock-mtn curl -s localhost:8080/__admin/requests"
    echo "  (no dashboard: it has no data source to show — docs/runbooks/demo.md)"
    echo
    echo "  tear down with: just demo_project={{demo_project}} demo-down"
    exit $status

# What is running, under which project, on which host ports.
#
# The answer to "is anything of mine still up, and whose is that other stack"
# — which is the question two concurrent demos create. `docker compose ps`
# reports the project this variable set addresses; the `docker ps` line below
# it reports EVERY vpay-ish project on the machine, because a collision is by
# definition something the current variables do not name.
#
# Show what the demo stack is running, and every vpay-ish container.
demo-status:
    #!/usr/bin/env bash
    set -uo pipefail
    export VPAY_DEMO_PROJECT={{demo_project}}
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    export VPAY_DEMO_ORANGE_PORT={{demo_orange_port}}
    export VPAY_DEMO_CHECKOUT_PORT={{demo_checkout_port}}
    export VPAY_DEMO_SHOP_PORT={{demo_shop_port}}
    echo "demo-status: project {{demo_project}}, server :{{demo_port}}, receiver :{{demo_receiver_port}}, orange stub :{{demo_orange_port}}"
    echo "demo-status: checkout page :{{demo_checkout_port}}, shop :{{demo_shop_port}}"
    docker compose {{demo_compose}} ps
    echo
    echo "demo-status: every vpay-ish container on this machine —"
    docker ps -a --filter name=vpay --format 'table {{{{.Names}}\t{{{{.Status}}\t{{{{.Ports}}'

# Removes the containers AND the volumes, so the next `just demo` starts on a
# freshly migrated database rather than one carrying a previous run's rows.
#
# Takes no port, and does not need one whatever `just demo` was run with:
# compose matches containers by project name and label, not by published port,
# and `compose.demo.yml`'s `${VPAY_DEMO_PORT:-8080}` has a default, so an unset
# variable is not even a warning. Measured, not assumed — brought up on 18080,
# torn down with the variable unset, container and volume both gone.
#
# It DOES need the project name, and that is the one thing this recipe cannot
# guess: `just demo_project=vpay-demo-b demo` must be torn down with `just
# demo_project=vpay-demo-b demo-down`, or it tears down the default project
# instead — which is either nothing at all or, worse, somebody else's demo.
# `just demo-status` prints every vpay-ish project on the machine for exactly
# this moment.
#
# Stop the demo stack and delete its volumes.
demo-down:
    #!/usr/bin/env bash
    set -euo pipefail
    export VPAY_DEMO_PROJECT={{demo_project}}
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    export VPAY_DEMO_ORANGE_PORT={{demo_orange_port}}
    export VPAY_DEMO_CHECKOUT_PORT={{demo_checkout_port}}
    export VPAY_DEMO_SHOP_PORT={{demo_shop_port}}
    docker compose {{demo_compose}} down -v
    echo "demo-down: project {{demo_project}} is gone (containers and volumes)"

# Print the demo shop's URL — the end-to-end demo a human clicks through.
#
# A recipe rather than a line in the runbook because the port is a variable:
# `just demo_shop_port=13001 demo-shop` prints the URL that run actually
# published, and a copy in prose would be right only for the default.
#
# It does NOT check that anything is listening. `just demo-status` is the
# question "is it up"; this one is "where is it".
#
# Print the demo shop's URL for the current demo_shop_port.
demo-shop:
    @echo "http://localhost:{{demo_shop_port}}"

# Print vpay's own checkout page's origin — the value the generated overlay's
# `checkout.public_base_url` carries, and the origin every payer link vpay
# mints for this stack is built on.
#
# You do not open this URL directly: `/` is not a route this app serves. A
# payer arrives at `/c/{cs_id}?key=…#{client_secret}`, which is the `url` a
# hosted Checkout Session answers with — `just demo-walk`'s step 5 prints one.
#
# Print vpay's checkout page origin for the current demo_checkout_port.
demo-checkout:
    @echo "http://localhost:{{demo_checkout_port}}"

# ------------------------------------------------------- stripe compat ----

# Run `sdks/stripe-compat` — the official `stripe` package driven against a
# REAL vpay — on the demo stack.
#
# Same stack, same overlay and same `demo_port` as `just demo`, deliberately:
# the suite needs a merchant whose PUBLIC JWK the server actually holds, and
# `gen-demo-keys` is the only thing in this repository that produces one
# (`config/application.yml`'s `acme-cameroon` modulus is a placeholder nobody
# has a key for). So `just demo_port=18080 stripe-compat` and `just
# demo_port=18080 demo` share a stack, and `just demo-down` tears down either.
#
# It brings up SIX services rather than the whole stack: postgres, both
# WireMock rails, the merchant webhook receiver, the server and the worker.
# Neither the dashboard nor vpay's own checkout page nor the demo shop plays
# any part in a `/v1` conformance run, and building three Next.js images for
# it costs minutes. CI's `e2e` job builds them because Cypress needs them.
#
# `vpay-worker` and `wiremock-webhook` are not optional and were added for two
# specific cases: the worker is what drives a confirmed intent to `succeeded`
# (`lifecycle.compat.test.ts`), and the receiver is what records the delivery
# `webhooks.compat.test.ts` hands to the real `stripe` package's
# `constructEvent`. Without either, those two cases fail — deliberately, since
# a suite that skipped them would report a green that proves less than it
# looks like.
#
# `build-sdk-node` is not optional: the suite imports `@vaam-apps/vpay-sdk/stripe`,
# which resolves to `sdks/nodejs/dist/stripe-auth.js`.
#
# The stack is left UP on purpose, exactly as `just demo` leaves it — a failed
# conformance run is when you most want to go and read
# `docker compose logs vpay-server`. Tear down with `just demo-down`.
stripe-compat: gen-demo-keys build-sdk-node
    #!/usr/bin/env bash
    set -euo pipefail
    for tool in docker curl pnpm cargo jq; do
        command -v "$tool" >/dev/null 2>&1 || { echo "stripe-compat: needs '$tool' on PATH" >&2; exit 1; }
    done
    # Same six variables as `just demo-up`, so this recipe addresses the
    # same project and `just demo-down` tears down either. Without the project
    # export it would run under compose.demo.yml's `${VPAY_DEMO_PROJECT:-…}`
    # default while a `just demo_project=… demo-down` addressed another one.
    export VPAY_DEMO_PROJECT={{demo_project}}
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    export VPAY_DEMO_ORANGE_PORT={{demo_orange_port}}
    export VPAY_DEMO_CHECKOUT_PORT={{demo_checkout_port}}
    export VPAY_DEMO_SHOP_PORT={{demo_shop_port}}
    # Six services, not the eight `demo_services` names: this suite drives
    # `/v1` and never opens a browser, so building two Next.js images for it
    # would cost minutes it does not buy anything with. `compat_services` is
    # spelled out beside `demo_services` for exactly that reason.
    docker compose {{demo_compose}} up -d --build --wait {{compat_services}}

    # `vpay-server`'s image is FROM scratch and carries no healthcheck, so
    # readiness is observed from outside — the same way `just demo` and CI do
    # it. The suite's own preflight would fail with a good message anyway;
    # this loop exists so that "the stack is still booting" does not read as
    # "the stack is broken".
    echo "stripe-compat: waiting for http://localhost:{{demo_port}}/healthz"
    deadline=$((SECONDS + 120))
    until curl -fsS -o /dev/null http://localhost:{{demo_port}}/healthz; do
        if [ "$SECONDS" -ge "$deadline" ]; then
            echo "stripe-compat: FAIL — /healthz did not answer within 120s. Last 80 log lines:" >&2
            docker compose {{demo_compose}} ps >&2
            docker compose {{demo_compose}} logs --tail 80 vpay-server >&2
            exit 1
        fi
        sleep 2
    done
    echo "stripe-compat: /healthz answered"
    echo

    set +e
    # VPAY_RECEIVER_URL is the webhook case's view of the merchant side: it
    # reads the receiver's own request journal rather than vpay's tables. The
    # secret is left to the suite's default, which is the placeholder
    # compose.e2e.yml gives both binaries — set MERCHANT_WEBHOOK_SECRET here
    # too if you have changed it there.
    VPAY_BASE_URL=http://localhost:{{demo_port}} \
    VPAY_RECEIVER_URL=http://localhost:{{demo_receiver_port}} \
    VPAY_MERCHANT_CLIENT_ID=demo-merchant \
    VPAY_MERCHANT_PRIVATE_KEY_PATH="$PWD/.e2e/demo-merchant/oauth-signing-key.pem" \
      pnpm --filter @vaam-apps/vpay-stripe-compat compat
    status=$?
    set -e

    echo
    echo "  tear down with: just demo_project={{demo_project}} demo-down"
    exit $status

storybook:
    pnpm --filter @vpay/ui storybook

dev-dashboard:
    pnpm --filter @vpay/dashboard dev

# ------------------------------------------------------------------ docs ---

# The docs gates that need nothing but the checkout. Both are also in
# `just verify`, so `just ci` runs them; this recipe is the short way to run
# only the documentation ones while editing.
#
# Fail if a doc claims an unimplemented item wrongly, or links to a dead path.
docs-check: verify-status verify-links

# NOT part of `just ci`, and deliberately: it needs the network and a GitHub
# token. It resolves every workflow-run id, pull request and issue that a
# tracked *.md cites as evidence — `run 33929374661`, `PR #31`, `Issue #11` —
# against this repository, and fails on one that does not exist. A cited id
# that resolves to nothing is a false claim; fix it with a struck-through,
# dated correction rather than by substituting an id you have not checked.
#
# It FAILS when `gh` is missing or unauthenticated. It does not print
# "skipped" and exit 0, because a check that downgrades itself reports
# success for a run in which nothing was checked, and in a log that is
# indistinguishable from a run in which everything passed.
#
# Fail if a doc cites a CI run, PR or issue id that does not exist (network).
docs-check-citations:
    cargo xtask verify-citations
