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
# `backends/tests/integration/tests/webhooks.rs`. `min_tests` is a floor that
# catches a binary vanishing, not a running total — it is set a little under
# the measured count rather than to it, so it is not a number people bump
# reflexively.
expected_ignored := "0"
expected_suites := "38"
min_tests := "840"

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

storybook:
    pnpm --filter @vpay/ui storybook

dev-dashboard:
    pnpm --filter @vpay/dashboard dev

# ------------------------------------------------------------------ docs ---

# Fail if a doc links to a file that does not exist.
docs-check:
    cargo xtask verify-status
    @echo "note: link checking is not implemented yet — see docs/status.md"
