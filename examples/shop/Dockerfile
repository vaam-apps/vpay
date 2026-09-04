# The demo shop image. Built from the REPOSITORY ROOT:
#
#     docker build -f examples/shop/Dockerfile .
#
# The root context is not incidental: `@vpay/sdk` and `@vpay/stripe-js` are
# workspace dependencies resolved through `pnpm-workspace.yaml`, so the
# lockfile, the workspace file and `sdks/` all have to be inside the context
# for `pnpm install --frozen-lockfile` to reproduce what a developer has.
# `frontends/` is deliberately absent — pnpm resolves a workspace glob with no
# matches to zero packages and carries on (the same property
# `frontends/Dockerfile` relies on for `examples/*`).

FROM node:22-alpine AS builder
# Prisma's engines are dynamically linked against OpenSSL 3 on musl.
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
# on and nothing else. The shop's own `postinstall` runs `zenstack generate`
# and `prisma generate` here, so the Prisma client exists before `next build`
# type-checks against it.
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
    PORT=3000 \
    HOSTNAME=0.0.0.0 \
    HOME=/tmp \
    XDG_CACHE_HOME=/tmp

# The Prisma CLI, for `migrate deploy` in the entrypoint.
#
# Pinned to the same version the schema was generated with — a CLI ahead of
# the client is how a migration silently rewrites a schema nobody reviewed.
#
# Installed into its own directory with its own manifest rather than
# `npm install --global`, so that it can carry the SAME `deepmerge-ts`
# override the workspace does (GHSA-ggr8-5vv4-36mx, high; the justification
# is in the root package.json). A global install would have quietly shipped
# the version `just audit-web` refuses.
RUN mkdir -p /opt/prisma-cli \
    && printf '%s' '{"name":"vpay-shop-prisma-cli","private":true,"dependencies":{"prisma":"6.19.3"},"overrides":{"deepmerge-ts":"^8.0.2"}}' > /opt/prisma-cli/package.json \
    && npm --prefix /opt/prisma-cli install --no-fund --no-audit \
    && npm cache clean --force

# Next's standalone bundle is rooted at the workspace root
# (`outputFileTracingRoot` in next.config.ts), so the server lives at
# examples/shop/server.js inside it.
COPY --from=builder /build/examples/shop/.next/standalone ./
COPY --from=builder /build/examples/shop/.next/static ./examples/shop/.next/static
# The migrations and the generated schema, for `prisma migrate deploy`.
COPY --from=builder /build/examples/shop/prisma ./prisma
COPY examples/shop/docker-entrypoint.sh /usr/local/bin/vpay-shop-entrypoint

# `node`, the image's own unprivileged user (uid 1000). Nothing in this image
# is written at runtime: the compose service runs it with
# `read_only: true` and a tmpfs on /tmp.
USER node

EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=12 \
    CMD node -e "fetch('http://127.0.0.1:3000/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

ENV PATH="/opt/prisma-cli/node_modules/.bin:${PATH}"

ENTRYPOINT ["/usr/local/bin/vpay-shop-entrypoint"]
