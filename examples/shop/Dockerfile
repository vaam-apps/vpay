# The demo shop image. Built from the REPOSITORY ROOT:
#
#     docker build -f examples/shop/Dockerfile .
#
# The root context is not incidental: `@vaam-apps/vpay-sdk` and `@vaam-apps/vpay-stripe-js` are
# workspace dependencies resolved through `pnpm-workspace.yaml`, so the
# lockfile, the workspace file and `sdks/` all have to be inside the context
# for `pnpm install --frozen-lockfile` to reproduce what a developer has.
# `frontends/` is deliberately absent — pnpm resolves a workspace glob with no
# matches to zero packages and carries on (the same property
# `frontends/Dockerfile` relies on for `examples/*`).

# The Node SDK's sealing module, compiled here and nowhere else.
#
# `@vaam-apps/vpay-sdk` does not seal in TypeScript: its signing path is
# `sdks/rust/wasm`, Rust compiled to WebAssembly, and the shop signs every
# `/v1` call through it. The node builder below has no Rust toolchain, no
# `bash` (`scripts/build-wasm.sh` is a bash script) and no `wasm-bindgen`, so
# the module is built in this stage and copied across; the node stage then runs
# only the SDK's `tsc` step (`build:ts`), never its `build:wasm`.
#
# The image is the one `backends/Dockerfile` names (rust-toolchain.toml's
# 1.98.0), and `cargo xtask verify-toolchain` keeps this `FROM` equal to
# that file's `channel`. The `wasm-bindgen` CLI is installed at the version
# `Cargo.lock` pins for the crate, read from it rather than written here a
# second time: the CLI and the crate agree on a private interface, and
# `build-wasm.sh` refuses a mismatch.
FROM rust:1.98.0-alpine3.22 AS sealer
# Alpine package versions are not pinned: the base image's tag pins the
# Alpine release, and an exact apk version disappears from the mirror at the
# next security update, which would break the build rather than secure it.
# hadolint ignore=DL3018
RUN apk add --no-cache bash musl-dev pkgconfig
WORKDIR /build
COPY Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install wasm-bindgen-cli --locked \
        --version "$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/version = |"/, ""); print; exit}' Cargo.lock)"
RUN rustup target add wasm32-unknown-unknown
# Cargo parses every workspace member's manifest before it builds one, so the
# whole workspace's manifests have to be here even though one crate is built.
COPY Cargo.toml clippy.toml rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY .xtask ./.xtask
COPY backends ./backends
COPY schemas ./schemas
COPY sdks/rust ./sdks/rust
COPY examples/merchant-demo ./examples/merchant-demo
COPY sdks/nodejs/scripts ./sdks/nodejs/scripts
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    bash sdks/nodejs/scripts/build-wasm.sh

FROM node:22-alpine AS builder
# Prisma's engines are dynamically linked against OpenSSL 3 on musl. Prisma
# left the request path at the ZenStack 3 upgrade but not the image: v3's
# migration engine still drives Prisma Migrate.
# hadolint ignore=DL3018
RUN apk add --no-cache openssl && corepack enable
WORKDIR /build

COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json .npmrc ./
COPY sdks ./sdks
COPY examples/shop ./examples/shop

# No Cypress binary in an image that never runs it; no telemetry call out of a
# build.
ENV CYPRESS_INSTALL_BINARY=0 \
    DO_NOT_TRACK=1 \
    NEXT_TELEMETRY_DISABLED=1 \
    CHECKPOINT_DISABLE=1

# `--filter @vpay-examples/shop...` pulls in the two SDK packages it depends
# on and nothing else. The shop's own `postinstall` runs `zen generate` here,
# so `zenstack/schema.ts` — which `src/server/db.ts` imports — exists before
# `next build` type-checks against it. It is generated rather than committed,
# for the reason .gitignore gives.
RUN pnpm install --frozen-lockfile --filter @vpay-examples/shop...
# The sealing module, built above. `sdks/nodejs/wasm` is gitignored, so the
# `COPY sdks` earlier did not bring one.
COPY --from=sealer /build/sdks/nodejs/wasm ./sdks/nodejs/wasm
# The shop's own `build` runs `build:deps`, which runs the SDK's `build`, which
# runs `build:wasm` and so wants Rust and bash. Here the module already exists,
# so the three steps `build` is made of are run without it: the SDK's `tsc`,
# stripe-js's `tsc`, then `next build`.
RUN pnpm --filter @vaam-apps/vpay-sdk run build:ts \
    && pnpm --filter @vaam-apps/vpay-stripe-js build \
    && pnpm --filter @vpay-examples/shop exec next build

FROM node:22-alpine AS runner
# hadolint ignore=DL3018
RUN apk add --no-cache openssl
WORKDIR /app

# `HOME` and `XDG_CACHE_HOME` point at /tmp because the root filesystem is
# read-only in the compose service: anything that wants somewhere to write
# gets the tmpfs, and nothing else is writable.
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    CHECKPOINT_DISABLE=1 \
    PRISMA_HIDE_UPDATE_MESSAGE=1 \
    DO_NOT_TRACK=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0 \
    HOME=/tmp \
    XDG_CACHE_HOME=/tmp

# The ZenStack CLI, for `migrate deploy` in the entrypoint.
#
# It replaced a pinned Prisma CLI at the v3 upgrade (2026-09-06) and it still
# carries one: `@zenstackhq/cli` depends on `prisma ^6.19.0` and shells out to
# Prisma Migrate, which is what actually applies the SQL. What changed is the
# schema it works from — the zmodel, with the Prisma schema derived on the fly
# — so there is no committed `prisma/schema.prisma` for a CLI to drift from.
#
# Pinned to the same version the application depends on, for the reason the
# Prisma pin had: a CLI ahead of the runtime is how a migration silently
# rewrites a schema nobody reviewed.
#
# Installed into its own directory with its own manifest rather than
# `npm install --global`, so that it can carry the SAME `deepmerge-ts`
# override the workspace does (GHSA-ggr8-5vv4-36mx, high; the justification
# is in the root package.json — it reaches this tree through Prisma's
# `@prisma/config`). A global install would have quietly shipped the version
# `just audit-web` refuses.
RUN mkdir -p /opt/zen-cli \
    && printf '%s' '{"name":"vpay-shop-zen-cli","private":true,"dependencies":{"@zenstackhq/cli":"3.9.3"},"overrides":{"deepmerge-ts":"^8.0.2"}}' > /opt/zen-cli/package.json \
    && npm --prefix /opt/zen-cli install --no-fund --no-audit \
    && npm cache clean --force

# Next's standalone bundle is rooted at the workspace root
# (`outputFileTracingRoot` in next.config.ts), so the server lives at
# examples/shop/server.js inside it.
COPY --from=builder /build/examples/shop/.next/standalone ./
COPY --from=builder /build/examples/shop/.next/static ./examples/shop/.next/static
# The zmodel and the migrations, for `zen migrate deploy`. Not the generated
# `zenstack/*.ts` — those are bundled into `server.js` by `next build`, and
# the CLI reads the zmodel.
COPY --from=builder /build/examples/shop/zenstack/schema.zmodel ./zenstack/schema.zmodel
COPY --from=builder /build/examples/shop/zenstack/migrations ./zenstack/migrations
COPY examples/shop/docker-entrypoint.sh /usr/local/bin/vpay-shop-entrypoint

# `node`, the image's own unprivileged user (uid 1000). Nothing in this image
# is written at runtime: the compose service runs it with
# `read_only: true` and a tmpfs on /tmp.
USER node

EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=12 \
    CMD node -e "fetch('http://127.0.0.1:3000/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

ENV PATH="/opt/zen-cli/node_modules/.bin:${PATH}"

ENTRYPOINT ["/usr/local/bin/vpay-shop-entrypoint"]
