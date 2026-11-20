# 2026-11-20 — One high and three moderate npm advisories cleared, with nothing ignored

## What this is

A dependency-only change. CI's `web` job failed `audit-web` on `master` at
`853e3600`, on `pnpm-lock.yaml`, and therefore on every open pull request
whatever it touched. No source behaviour changed.

## What the registry said

`pnpm audit --audit-level=moderate` (pnpm 11.18.0) on `master` at `853e3600`
reported **4 vulnerabilities, 3 moderate and 1 high**:

| Advisory            | Severity | Package  | Affected             | Patched     | In the lockfile |
| ------------------- | -------- | -------- | -------------------- | ----------- | --------------- |
| GHSA-7589-6fvf-x4pw | high     | `effect` | `<=3.22.0`           | `>=3.22.1`  | 3.21.0          |
| GHSA-7qx7-33qx-rqx2 | moderate | `next`   | `>=15.0.0 <=15.5.28` | `>=15.5.29` | 15.5.28         |
| GHSA-f3mh-rxg3-f5g2 | moderate | `next`   | `>=15.0.0 <=15.5.28` | `>=15.5.29` | 15.5.28         |
| GHSA-pc58-hwjq-p385 | moderate | `next`   | `<=15.5.28`          | `>=15.5.29` | 15.5.28         |

Where they come from:

- `effect`: `examples/shop > @zenstackhq/cli (devDependency) > prisma >
@prisma/config`, which pins `effect` exactly.
- `next`: `frontends/apps/checkout` and `frontends/apps/dashboard`, both on
  `^15.5.28`, and the dashboard's `@refinedev/nextjs-router` peer.

## The change

| What                                      | How                                                | Before → after    |
| ----------------------------------------- | -------------------------------------------------- | ----------------- |
| `next`, `checkout` and `dashboard` floors | direct bump `^15.5.28` → `^15.5.29`                | 15.5.28 → 15.5.29 |
| `@prisma/config>effect`                   | new scoped override `^3.22.1`, dated, with removal | 3.21.0 → 3.22.4   |

`@zenstackhq/cli` 3.9.3 → 3.10.3 was checked first and is not a way out: the
newest CLI (2026-11-19) still declares `prisma ^6.19.0`, `prisma` 6.19.3 is
the newest 6.x, and `@prisma/config` 6.19.3 declares `effect 3.21.0` exactly.
The registry's `latest` `@prisma/config` (7.10.1) is a major the CLI's range
excludes, and declares `effect 3.20.0`, so it would not clear it either. The
override has its dated paragraph in `pnpm-workspace.yaml`.

`effect` resolves 3.22.4 (2026-10-22); 3.22.5 (2026-11-19) was inside
`minimumReleaseAge` and no exclusion was added. `next` 15.5.29 resolves with
no exclusion. The `next>postcss` override still applies: `next@15.5.29`
resolves `postcss@8.5.28` in the lockfile.

## Measured after

All on this branch, pnpm 11.18.0. The corepack `pnpm` on this machine was
broken, so `just` ran with a wrapper that executes `npx -y pnpm@11.18.0`; the
SDK's wasm build needed `wasm-bindgen` 0.2.128, installed into a scratch
directory outside the repository.

- `pnpm install --frozen-lockfile`: exit 0, "Already up to date".
  `examples/shop`'s `postinstall` (`zen generate`, the command that loads
  `@zenstackhq/cli`) ran and printed `Done` on the install that moved the
  lockfile.
- `just audit-web`, both passes (`--prod` and the whole workspace): **No
  known vulnerabilities found** twice, then `audit-web: ok`, exit 0. 4
  vulnerabilities before. `pnpm why effect -r` shows one version, 3.22.4.
- `@prisma/config` 6.19.3 loaded under effect 3.22.4 and
  `loadConfigFromFile` returned `resolvedPath`, `config` and `diagnostics`
  (the call `prisma` makes, here with no config file present).
- `just lint-web` (`build-sdk-node`, `pnpm -r typecheck`, `pnpm -r lint`):
  exit 0.
- `just test-web` (`pnpm -r test`): exit 0; `@vpay/config` 67, tokens 12,
  `stripe-js` 166, `vpay-sdk` 227, api-client 4, the Tauri plugin 71,
  checkout 585 + 1 skipped, `examples/shop` 109, dashboard 580 + 1 skipped.
- `next build` in `frontends/apps/checkout` and `frontends/apps/dashboard`
  (Next.js 15.5.29): both compiled and generated their static pages, exit 0.

## Not run

- `test-storybook`, the axe suites and the Cypress and compose e2e runs; CI
  runs them on the same lockfile.
- Trivy and `just verify` end to end.
- `prisma migrate` against a real Postgres with `effect` 3.22.4; only the
  config loader was exercised.
- Nothing was added to `ignoreGhsas` or `minimumReleaseAgeExclude`.
