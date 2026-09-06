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

FROM node:22-alpine AS builder
# Prisma's engines are dynamically linked against OpenSSL 3 on musl. Prisma
# left the request path at the ZenStack 3 upgrade but not the image: v3's
# migration engine still drives Prisma Migrate.
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
RUN pnpm --filter @vpay-examples/shop build

FROM node:22-alpine AS runner
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
