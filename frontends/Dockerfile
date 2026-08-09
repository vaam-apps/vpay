FROM node:22-alpine AS builder
RUN corepack enable
WORKDIR /build
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json .npmrc ./
COPY frontends ./frontends
# pnpm-workspace.yaml also globs `examples/*`, but that directory is
# deliberately NOT copied into this build context. Verified directly
# (reconstructing this exact context with `examples/` absent): pnpm resolves
# the glob to zero matches and proceeds — `--frozen-lockfile` does not
# require a workspace-glob match to exist on disk, only that the lockfile is
# internally consistent with whatever *does* exist. Copying it would only
# add unused files to the image build context.
ENV CYPRESS_INSTALL_BINARY=0
RUN pnpm install --frozen-lockfile --filter @vpay/dashboard...
RUN pnpm --filter @vpay/dashboard build

FROM node:22-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production
COPY --from=builder /build/frontends/apps/dashboard/.next/standalone ./
COPY --from=builder /build/frontends/apps/dashboard/.next/static ./frontends/apps/dashboard/.next/static
EXPOSE 3000
CMD ["node", "frontends/apps/dashboard/server.js"]
