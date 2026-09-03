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

test-sdk-browser:
    pnpm --filter @vpay/stripe-js test

# `@vpay/stripe-js` is browser ESM: nothing in the workspace imports it as a
# TypeScript source, so `lint-web` does not need it built the way it needs
# `build-sdk-node`. The static checkout example loads `dist/index.js`
# directly, which is what this recipe is for.
build-sdk-browser:
    pnpm --filter @vpay/stripe-js build

# Vendors `@vpay/stripe-js`'s build output into
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

# Depends on `build-sdk-node` because the typecheck does: `sdks/stripe-compat`
# imports `@vpay/sdk/stripe`, whose types resolve through that package's
# `exports` map to `dist/stripe-auth.d.ts`, and `dist/` is gitignored. Without
# the build this recipe fails on a clean checkout with `TS2307: Cannot find
# module '@vpay/sdk/stripe'` — a missing artefact reported as a broken import.
lint-web: build-sdk-node
    pnpm -r typecheck

deny:
    cargo deny check

# `deny`'s counterpart for the JavaScript half of the repo, and CI's `web`
# job runs THIS recipe rather than a copy of its commands, so the gate and
# the local check cannot drift.
#
# Two runs, not one, and the narrower one first. `--prod` walks only the
# production dependency graph — what a merchant would actually receive from
# `@vpay/sdk` / `@vpay/stripe-js`, and what `frontends/Dockerfile` ships —
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
audit-web:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "audit-web: production dependency graph only"
    pnpm audit --audit-level=high --prod
    echo "audit-web: whole workspace, dev dependencies included"
    pnpm audit --audit-level=high
    echo "audit-web: ok — no high or critical advisory in the workspace"

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
# `@vpay/stripe-js` unit suite which cargo does not run and this count does
# not cover, and the `track_http_metrics`-on-the-browser-nest test added
# while rebasing 5c onto Step 6) was rebased onto that tree:
# `cargo nextest list --workspace` lists **969 total, 39 test binaries, 0
# ignored** — 39 binaries because `browser_checkout.rs` is a new binary
# (38 -> 39, as the paragraph above already anticipated); 969 rather than
# 927 for the sum of Step 5c's own new tests (including
# `a_browser_get_is_counted_under_its_own_route_pattern`, added while
# resolving this rebase) landing on top of Step 6's 927.
expected_ignored := "0"
expected_suites := "39"
# A floor, not a target — set a little under the measured 969 rather than to
# it, so it is not a number people bump reflexively. Bump it in the same
# commit that legitimately adds tests, never to make a red run green.
min_tests := "900"

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
# What it proves: the chart lints, both value sets render, the fifteen named
# guards are exactly the fifteen on disk and each fires on its own values file
# with a non-zero exit, and every rendered object validates against the
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
    # The expected set is written out rather than counted, because "13 files
    # were found and 13 fired" is also what deleting a guard *and* its values
    # file looks like. Adding a guard means adding its name here, its values
    # file under ci/guards/, and the `fail` in templates/_validate.tpl — in
    # one commit.
    expected_guards=(
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

# What `.github/workflows/release.yml` does, minus everything that needs a
# registry — and minus one thing that needs a runner it cannot have here.
#
# Builds all three images for the HOST platform ONLY. The published images are
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
# Build the three release images for the host platform, then check the chart.
release-dry-run:
    #!/usr/bin/env bash
    set -euo pipefail
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

    # name:dockerfile:target — the same three the release matrix builds, from
    # the repository root, which is the context BOTH Dockerfiles require:
    # `backends/Dockerfile` COPYs `sdks/rust` and `examples/merchant-demo`
    # because cargo refuses to load a workspace whose `members` list names a
    # missing directory.
    #
    # No `--build-arg VPAY_GIT_SHA` here, deliberately, and that is a
    # difference from the workflow rather than an omission: a dry run is not
    # a release, its images are never pushed, and stamping one with a real
    # commit would produce a `vpay_build_info` label for an artefact nobody
    # can pull. The default (`unknown`) is the true answer for these.
    for spec in vpay-server:backends/Dockerfile:server \
                vpay-worker:backends/Dockerfile:worker \
                vpay-dashboard:frontends/Dockerfile:runner; do
        IFS=: read -r name file target <<<"$spec"
        echo "==> $name ($file, target $target)"
        docker buildx build \
            --platform "$platform" \
            --file "$file" \
            --target "$target" \
            --tag "ghcr.io/vaam-store/$name:dry-run" \
            --provenance=false \
            --sbom=false \
            --push=false \
            .
    done

    echo "==> helm-check"
    just helm-check

    echo "release-dry-run: ok — three images built for $platform, chart checked."
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

# The host port `just demo` publishes `vpay-server` on. Override it per
# invocation when 8080 is taken on your machine:
#
#     just demo_port=18080 demo
#
# `just demo-down` needs no port — see that recipe.
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
    key=.e2e/demo-merchant/oauth-signing-key.pem
    overlay=.e2e/application-demo.yml

    if [ -e "$key" ] && [ -e "$overlay" ]; then
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
            && grep -q '^\s*webhooks:' "$overlay" \
            && grep -q '^\s*publishable_keys:' "$overlay" \
            && grep -q "^\s*public_base_url: http://localhost:{{demo_port}}$" "$overlay"; then
            echo "gen-demo-keys: $key and $overlay already exist, keeping them"
            exit 0
        fi
        if ! grep -q '^\s*merchant_id:' "$overlay"; then
            echo "gen-demo-keys: $overlay predates the required \`merchant_id\` field — regenerating the pair"
        elif ! grep -q '^\s*webhooks:' "$overlay"; then
            # Added 2026-09-03 (Step 5). Not fatal the way a missing
            # `merchant_id` is — the overlay still loads — but the demo's
            # step 7 would then poll a receiver no endpoint points at and
            # fail for a reason that has nothing to do with the worker. Same
            # class of stale-generated-file failure, so the same shape check.
            echo "gen-demo-keys: $overlay predates the \`webhooks\` block — regenerating the pair"
        elif ! grep -q '^\s*publishable_keys:' "$overlay"; then
            # Added 2026-09-03 (Step 5c). Same class again: the overlay still
            # loads without it, but `examples/checkout-browser` and the
            # Cypress spec would then present a key `merchant_id_for_publishable_key`
            # resolves to nothing, and every browser call would be the
            # surface's uniform 404 — a refusal that deliberately names
            # neither the key nor the reason.
            echo "gen-demo-keys: $overlay predates the \`publishable_keys\` block — regenerating the pair"
        else
            echo "gen-demo-keys: $overlay was generated for a different demo_port than {{demo_port}} — regenerating the pair"
        fi
    elif [ -e "$key" ] || [ -e "$overlay" ]; then
        echo "gen-demo-keys: $key and $overlay are out of sync — regenerating the pair"
    fi
    rm -f "$key" "$overlay"

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
        # intent's own client_secret (Step 5c, @vpay/stripe-js). Not secret:
        # it is rendered into the checkout page, it names this tenant, and it
        # authorises nothing on its own.
        #
        # A FIXED value, not generated like the key pair above, and
        # deliberately so: examples/checkout-browser and the Cypress spec
        # both hardcode it, and a per-run value would mean a demo page that
        # has to be regenerated with the overlay. It is a sandbox label for a
        # throwaway stack; there is nothing here to keep secret.
        #
        # `pk_test_` because this overlay does not set livemode, so the base
        # config's `false` stands — Config::validate_all refuses a `pk_live_`
        # key under it (ConfigError::PublishableKeyLivemodeMismatch).
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
    YAML
    # The scratch image runs as UID 65532 and must read the bind mount. This
    # file is a public key and a client id; there is nothing secret in it.
    chmod 0644 "$overlay"

    echo "gen-demo-keys: wrote $key (3072-bit RSA, mode 0600, host-only)"
    echo "gen-demo-keys: wrote $overlay — client_id=demo-merchant kid=$kid"

# Exits with the demo's own status, so `just demo` failing means the demo
# failed — and still prints the URLs, because a failed demo run is exactly
# when you want to go and look at the dashboard and the discovery document
# yourself. (The earlier `/healthz` timeout is the one path that does not
# print them: if the server never answered, there is nothing to visit. It
# dumps `docker compose logs vpay-server` instead.)
#
# Boot the full stack and run the merchant demo against it.
demo: gen-demo-keys
    #!/usr/bin/env bash
    set -euo pipefail
    # Fail fast with a named tool rather than a 120 s readiness timeout whose
    # message blames /healthz (review finding, 2026-09-02).
    for tool in docker curl jq cargo; do
        command -v "$tool" >/dev/null 2>&1 || { echo "demo: needs '$tool' on PATH" >&2; exit 1; }
    done
    # Read by compose.demo.yml's `ports:` for vpay-server. Exported rather
    # than passed per command, because `docker compose` interpolates each
    # file from its own environment.
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    docker compose {{demo_compose}} up -d --build

    # `vpay-server` has no container healthcheck — its image is FROM scratch
    # and holds no shell to run one (compose.e2e.yml's own note). Readiness is
    # observed from outside instead, the same way .github/workflows/ci.yml's
    # e2e job does it.
    echo "demo: waiting for http://localhost:{{demo_port}}/healthz"
    deadline=$((SECONDS + 120))
    until curl -fsS -o /dev/null http://localhost:{{demo_port}}/healthz; do
        if [ "$SECONDS" -ge "$deadline" ]; then
            echo "demo: FAIL — /healthz did not answer within 120s. Last 80 log lines:" >&2
            docker compose {{demo_compose}} ps >&2
            docker compose {{demo_compose}} logs --tail 80 vpay-server >&2
            echo "demo: (exit 78 in that log means a config/CLI prerequisite is missing)" >&2
            exit 1
        fi
        sleep 2
    done
    echo "demo: /healthz answered"
    echo

    set +e
    # The demo runs on the host, so it needs the *published* port — the same
    # one the overlay's `public_base_url` names, or step 2 is an
    # `invalid_client` (see `demo_port`).
    VPAY_BASE_URL=http://localhost:{{demo_port}} \
      VPAY_RECEIVER_URL=http://localhost:{{demo_receiver_port}} \
      cargo run -q -p merchant-demo
    status=$?
    set -e

    echo
    echo "  dashboard   http://localhost:3000"
    echo "  server      http://localhost:{{demo_port}}"
    echo "  discovery   http://localhost:{{demo_port}}/v1/oauth/.well-known/openid-configuration"
    echo "  receiver    http://localhost:{{demo_receiver_port}}/__admin/requests"
    echo
    echo "  tear down with: just demo-down"
    exit $status

# Removes the containers AND the volumes, so the next `just demo` starts on a
# freshly migrated database rather than one carrying a previous run's rows.
#
# Takes no port, and does not need one whatever `just demo` was run with:
# compose matches containers by project name and label, not by published port,
# and `compose.demo.yml`'s `${VPAY_DEMO_PORT:-8080}` has a default, so an unset
# variable is not even a warning. Measured, not assumed — brought up on 18080,
# torn down with the variable unset, container and volume both gone.
#
# Stop the demo stack and delete its volumes.
demo-down:
    docker compose {{demo_compose}} down -v

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
# The dashboard plays no part in a `/v1` conformance run and building
# `frontends/Dockerfile` for it costs minutes. CI's `e2e` job builds it
# because Cypress needs it.
#
# `vpay-worker` and `wiremock-webhook` are not optional and were added for two
# specific cases: the worker is what drives a confirmed intent to `succeeded`
# (`lifecycle.compat.test.ts`), and the receiver is what records the delivery
# `webhooks.compat.test.ts` hands to the real `stripe` package's
# `constructEvent`. Without either, those two cases fail — deliberately, since
# a suite that skipped them would report a green that proves less than it
# looks like.
#
# `build-sdk-node` is not optional: the suite imports `@vpay/sdk/stripe`,
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
    export VPAY_DEMO_PORT={{demo_port}}
    export VPAY_DEMO_RECEIVER_PORT={{demo_receiver_port}}
    docker compose {{demo_compose}} up -d --build \
        postgres wiremock-mtn wiremock-orange wiremock-webhook vpay-server vpay-worker

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
      pnpm --filter @vpay/stripe-compat compat
    status=$?
    set -e

    echo
    echo "  tear down with: just demo-down"
    exit $status

storybook:
    pnpm --filter @vpay/ui storybook

dev-dashboard:
    pnpm --filter @vpay/dashboard dev

# ------------------------------------------------------------------ docs ---

# Fail if a doc links to a file that does not exist.
docs-check:
    cargo xtask verify-status
    @echo "note: link checking is not implemented yet — see docs/status.md"
