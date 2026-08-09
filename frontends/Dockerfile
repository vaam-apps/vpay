FROM node:22-alpine AS builder
RUN corepack enable
WORKDIR /build
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json .npmrc ./
COPY frontends ./frontends
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
