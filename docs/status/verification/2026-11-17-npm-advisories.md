# 2026-11-17 — Nine moderate npm advisories cleared, with nothing ignored

## What this is

A dependency-only change. CI's `web` job failed `audit-web` on `master` at
`ea469b40`, on `pnpm-lock.yaml`, and therefore on every open pull request
whatever it touched. No source behaviour changed.

## What the registry said

`pnpm audit --audit-level=moderate`, as `just audit-web` runs it (pnpm
11.18.0, Node 24.21.0), on `master` at `ea469b40` reported **9 moderate
findings** and 1 low below the gate's threshold (`hono` GHSA-3gg2-335m-rxc6, `>=3.12.0 <=4.13.15`, which the `hono` floor clears too). The `braces` id that
`auditConfig.ignoreGhsas` carries is not counted.

| Advisory            | Package             | Affected            | Patched     | In the lockfile |
| ------------------- | ------------------- | ------------------- | ----------- | --------------- |
| GHSA-3g3m-2fjr-6pw4 | `glob`              | `>=9.0.0 <=14.0.1`  | `>=14.0.2`  | 13.0.6          |
| GHSA-g3hv-75wg-m249 | `minimatch`         | `>=7.0.0 <11.0.1`   | `>=11.0.1`  | 10.2.6          |
| GHSA-qv5m-9pm6-hqp3 | `@hono/node-server` | `>=2.0.0 <2.1.6`    | `>=2.1.6`   | 2.1.5           |
| GHSA-jpvv-r467-5p4w | `@hono/node-server` | `>=2.0.0 <2.1.6`    | `>=2.1.6`   | 2.1.5           |
| GHSA-4m76-xfj5-7975 | `hono`              | `<4.13.16`          | `>=4.13.16` | 4.13.13         |
| GHSA-h7mg-cq63-vhpq | `hono`              | `>=4.1.4 <=4.13.15` | `>=4.13.16` | 4.13.13         |
| GHSA-46wx-v44x-9hh2 | `hono`              | `<4.13.16`          | `>=4.13.16` | 4.13.13         |
| GHSA-4h7h-rv7c-7m2v | `hono`              | `>=4.5.1 <=4.13.15` | `>=4.13.16` | 4.13.13         |
| GHSA-mfr8-8qww-v76h | `next`              | `>=16.3.0 <=16.4.4` | `>=16.4.5`  | 16.4.2          |

Where they come from:

- `glob` and half of `minimatch`: `frontends/apps/{checkout,dashboard} >
@storybook/react-vite > @joshwooding/vite-plugin-react-docgen-typescript >
glob > minimatch`.
- The other half of `minimatch`: `@vpay/config > typescript-eslint >
@typescript-eslint/* > @typescript-eslint/typescript-estree > minimatch`
  (seven paths).
- `@hono/node-server` and `hono`: `examples/shop > @zenstackhq/cli` (a
  devDependency), directly and through `@zenstackhq/server`.
- `next`: `examples/shop`'s own exact pin, and `@zenstackhq/server`'s peer.

## The change

| What                                                    | How                                          | Before → after    |
| ------------------------------------------------------- | -------------------------------------------- | ----------------- |
| `next`, `examples/shop` (exact)                         | bump                                         | 16.4.2 → 16.4.6   |
| `@next/eslint-plugin-next`, `@vpay/config` (exact)      | bump, rides `next` as on 2026-10-23          | 16.4.2 → 16.4.6   |
| `@zenstackhq/cli>@hono/node-server`                     | existing override raised `^2.1.3` → `^2.1.6` | 2.1.5 → 2.1.8     |
| `hono@<4.13.16`                                         | new floor override, the `qs` class           | 4.13.13 → 4.13.18 |
| `@joshwooding/vite-plugin-react-docgen-typescript>glob` | new scoped override, `^14.0.2`               | 13.0.6 → 14.0.6   |
| `@typescript-eslint/typescript-estree>minimatch`        | new scoped override, `^11.0.1`               | 10.2.6 → 11.0.8   |

Each override has a dated paragraph in `pnpm-workspace.yaml` with its
advisories and its removal condition. Why each is not a direct bump, read
from the registry on 2026-11-17:

- **`glob`.** The whole 13.x line is in the range; npm's bulk advisory
  endpoint still reports GHSA-3g3m-2fjr-6pw4 against 13.0.7 (the
  `v13-legacy` tag, 2026-11-06). Every `@storybook/react-vite` from 10.6.0 to
  10.6.7 (the newest) declares the plugin on `^0.7.0`, and 0.7.0 declares
  `glob ^13.0.1`. The plugin's 0.9.1 drops glob, but `^0.7.0` cannot reach it.
- **`minimatch`.** 10.2.6 is the last 10.x, and minimatch's README says the
  v11 matcher "will not be backported". `typescript-eslint` 8.73.0 declares
  `minimatch ^11.0.1`, so the upstream fix is a direct bump of `@vpay/config`'s
  pinned 8.69.0 — but npm dates 8.73.0 2026-11-16T17:06Z, inside pnpm 11's
  24-hour `minimumReleaseAge` at the time (12:29Z on 2026-11-17), and
  taking it would have meant eleven exact-version `minimumReleaseAgeExclude`
  entries (the lockfile carries eleven `typescript-eslint` packages at
  8.69.0).
- **`@hono/node-server`.** `@zenstackhq/cli` 3.10.2, the newest, still
  declares `^1.13.8`, and 1.x is no way out: the bulk endpoint reports
  GHSA-rmxm-3fg6-px4f against 1.19.19, the newest 1.x.
- **`hono`.** Every dependent takes it on a caret range; the lockfile held
  4.13.13. A floor moves it, as `qs` was moved on 2026-10-27.

**Newest versions skipped by `minimumReleaseAge`, with no exclusion added:**
`@hono/node-server` 2.1.9 (2026-11-17T08:20Z) and `hono` 4.13.19
(2026-11-16T12:57Z). Both resolved versions are already fixed.

## Is crossing a major safe? Measured, not assumed

Two of the overrides cross a major. Each consumer makes exactly one kind of
call, and it was run against both versions side by side
(`node_modules/.pnpm` still held the old ones after the install):

- **glob 13.0.6 vs 14.0.6**, with the plugin's own options
  (`globSync(pattern, { absolute: true, cwd, ignore, nodir: true })`), for
  four include patterns (`**/*.tsx`, `src/**/*.ts`, `src/**/*.{ts,tsx}`,
  `**/*.stories.@(ts|tsx)`) in each of the two apps: **8 cases, 0
  different**. Beyond that, the plugin is never loaded here:
  `@storybook/react-vite`'s preset imports it only when `reactDocgen` is
  `"react-docgen-typescript"`, which neither app's `.storybook/main.ts` sets.
- **minimatch 10.2.6 vs 11.0.8**, called as `typescript-estree` calls it
  (`minimatch(filePath, pattern, { dot: true })`, through `require`), on 7
  paths × 9 patterns including braces, extglobs, negation and dotfiles: **63
  cases, 0 different**. The call is textually identical in
  `typescript-estree` 8.69.0 and 8.73.0, the release that moved to
  minimatch 11 itself. It is reached only for `allowDefaultProject`, which
  this repository's lint config does not set; the module is still loaded by
  every lint run.

The comparison script is not committed; it lived in the session's scratch
directory.

## Measured after

All on this branch, pnpm 11.18.0, Node 24.21.0, a 4-core machine shared
with other builds (load average about 12 during the test runs).

- `pnpm install` moved the lockfile; `pnpm install --frozen-lockfile` then
  passes. `zen generate` (`examples/shop`'s `postinstall`, the one command
  that loads `@zenstackhq/cli` and so hono 4.13.18 and
  `@hono/node-server` 2.1.8) ran and printed `Done`.
- `just audit-web`, both passes (`--prod` and the whole workspace): **No
  known vulnerabilities found**, then `audit-web: ok`, exit 0. 9 moderate
  findings before. A bare `pnpm audit` (every severity) also reports none.
- `pnpm -r typecheck`, after `just build-sdk-node` as `lint-web` runs it:
  exit 0, 12 packages.
- `pnpm -r lint`: exit 0, 16 packages. The type-aware rules load
  `typescript-estree` and with it minimatch 11.0.8.
- `pnpm -r test`, with no `storybook-static/` on disk: **1775 passed, 2
  skipped** across the nine packages that have tests (`@vpay/config` 67,
  tokens 12, `vpay-stripe-js` 152, `vpay-sdk` 227, api-client 4, the Tauri
  plugin 71, checkout 556 + 1 skipped, `examples/shop` 107, dashboard 579 + 1
  skipped). **The dashboard's count is from its own run, not the recursive
  one.** In the recursive run its `payments-filters.test.tsx` calendar cases
  timed out at 5 s (2 failed, 578 passed); run alone the suite failed one of
  them the same way (1 failed, 579 passed), and that file run on its own
  passed **12 of 12, twice**, at about 5 s of test time. This is the timeout
  the 2026-10-23 page recorded under parallel load; the dashboard resolves no
  package this change moved except the Storybook plugin's `glob`, which
  vitest does not load. It is reported, not fixed.
- `just build-storybook`: exit 0. Both apps define `--color-base-100` 3
  times, and each app's `a11y-gate.test.ts` passed 6 of 6 against the
  artefact just built.
- `examples/shop` `next build`: **Next.js 16.4.6 (Turbopack)**, compiled, 3
  of 3 static pages, exit 0.
- `just verify`: exit 0, `verify: ok — the seventeen gates above passed`.
  `verify-doc-counts`, `verify-links`, `verify-ui` and `verify-npm-scope`
  re-run after this page was finished: all pass; `prettier --check .`
  passes.

## Not run

- `test-storybook`, the axe suites, which need a browser; CI's `web` job
  runs them.
- The checkout and dashboard `next build`s: neither resolves a package this
  change moved, except through Storybook.
- Trivy, and the `e2e (compose)` images; CI runs both on the same lockfile.
- `zen proxy`, the only `@zenstackhq/cli` command that starts
  `@hono/node-server`; `examples/shop` never runs it.
- The `typescript-eslint` 8.73.0 bump. It is the real fix for the
  `minimatch` half; once it is a day old it needs no exclusion, and the
  override comes out with it.
- Nothing was added to `ignoreGhsas` or `minimumReleaseAgeExclude`.
