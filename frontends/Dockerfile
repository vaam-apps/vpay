# The two Next.js images this repository publishes, built from the REPOSITORY
# ROOT (`docker build -f frontends/Dockerfile .`):
#
#   --target runner    ghcr.io/vaam-apps/vpay-dashboard   (unchanged)
#   --target checkout  ghcr.io/vaam-apps/vpay-checkout    (Step 9, lane 4)
#
# **Every consumer names its target explicitly** — `.github/workflows/release.yml`
# and both `dashboard` and `vpay-checkout` in `compose.e2e.yml`. Nothing here
# relies on "the last stage wins", because adding a stage would then silently
# change what an unqualified build produces. The dashboard's stage keeps the
# name `runner` it has always had, so the release workflow's existing matrix
# entry still resolves.
#
# The two builders are separate stages on purpose. They install different
# subsets (`--filter <app>...`) from different contexts, and a shared install
# would put the dashboard's dependency tree inside the checkout image and the
# other way round.

FROM node:22-alpine AS base
RUN corepack enable
WORKDIR /build
# No Cypress binary in an image that never runs it, and no telemetry call out
# of a build.
ENV CYPRESS_INSTALL_BINARY=0 \
    DO_NOT_TRACK=1 \
    NEXT_TELEMETRY_DISABLED=1

# ------------------------------------------------------------- dashboard ---

FROM base AS dashboard-builder
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json .npmrc ./
COPY frontends ./frontends
# `pnpm-workspace.yaml` also globs `examples/*` and `sdks/*`, and neither is
# copied into this stage. Verified directly (reconstructing this exact context
# with both absent): pnpm resolves the glob to zero matches and proceeds —
# `--frozen-lockfile` does not require a workspace-glob match to exist on
# disk, only that the lockfile is internally consistent with whatever *does*
# exist. `@vpay/dashboard` depends on no package under either directory.
RUN pnpm install --frozen-lockfile --filter @vpay/dashboard...
RUN pnpm --filter @vpay/dashboard build

FROM node:22-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production
COPY --from=dashboard-builder /build/frontends/apps/dashboard/.next/standalone ./
COPY --from=dashboard-builder /build/frontends/apps/dashboard/.next/static ./frontends/apps/dashboard/.next/static
EXPOSE 3000
CMD ["node", "frontends/apps/dashboard/server.js"]

# -------------------------------------------------------------- checkout ---

FROM base AS checkout-builder
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml tsconfig.base.json .npmrc ./
# `sdks/` as well as `frontends/`, and it is load-bearing rather than tidy:
# `@vpay/checkout` depends on `@vaam-apps/vpay-stripe-js` (`workspace:*`), whose
# `exports` resolve to `dist/` — which is gitignored. Its own `build`,
# `typecheck` and `test` scripts each run `pnpm --filter @vaam-apps/vpay-stripe-js
# build` first for exactly that reason (lane 3's note §5), so the SDK's
# SOURCE has to be in this context or `next build` fails with TS2307 on the
# first import. `examples/` stays out, as above.
COPY sdks ./sdks
COPY frontends ./frontends
RUN pnpm install --frozen-lockfile --filter @vpay/checkout...
RUN pnpm --filter @vpay/checkout build

FROM node:22-alpine AS checkout
WORKDIR /app

# `HOME` and `XDG_CACHE_HOME` point at /tmp because this image is run with a
# read-only root filesystem (`readOnlyRootFilesystem: true` in the chart,
# `read_only: true` in compose): anything that wants somewhere to write gets
# the tmpfs, and nothing else is writable. Next's standalone server writes
# nothing outside those two under `NODE_ENV=production`.
#
# `PORT`/`HOSTNAME` are the standalone server's own knobs. 3000 is the
# CONTAINER port; the host mapping is `demo_checkout_port` (3080 by default),
# and in the cluster it is the Service's. The app's `package.json` says
# `next start -p 3001` — that is the DEV server, and it is a different
# process from this one.
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0 \
    HOME=/tmp \
    XDG_CACHE_HOME=/tmp

# `output: 'standalone'` with `outputFileTracingRoot` at the repository root
# emits `.next/standalone/frontends/apps/checkout/server.js` plus a
# `node_modules/` and a top-level `package.json`. Next does NOT copy the
# static assets into it — `.next/static` has to come across separately, as
# Next's own documentation says (lane 3's note §4d).
COPY --from=checkout-builder /build/frontends/apps/checkout/.next/standalone ./
COPY --from=checkout-builder /build/frontends/apps/checkout/.next/static ./frontends/apps/checkout/.next/static

# `node`, the image's own unprivileged user (uid 1000). The same discipline
# `examples/shop/Dockerfile` uses, and the reason the chart can set
# `runAsNonRoot: true` without a numeric `runAsUser` guess: this is a
# `node:22-alpine` image, so /etc/passwd exists and the name resolves.
USER node

EXPOSE 3000

# `/healthz` is a route handler in this app (`app/healthz/route.ts`) that takes
# no dependency: it answers "Next is serving", never "vpay is reachable". Used
# by the compose healthcheck and by the chart's probes.
#
# `node -e` rather than curl or wget: neither is in `node:22-alpine`, and
# adding one to answer a healthcheck is a package in the image for the sake of
# a probe. Node 22's global `fetch` is already there.
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=12 \
    CMD node -e "fetch('http://127.0.0.1:3000/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

CMD ["node", "frontends/apps/checkout/server.js"]
