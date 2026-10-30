# 2026-10-27 — A `qs` floor clears GHSA-6pq8-jhm2-2vcw

## What this is

A dependency-only change, carried on the branch of
[#385](https://github.com/vaam-apps/vpay/pull/385), a docs-only PR. That PR's
first CI run, on `8d51fe49`, failed the `web` job's `audit-web` step on
`pnpm-lock.yaml`, a file it does not touch. `master` at `f885170c` resolves
the same version. Its last `ci` run (14:40Z) was green only because the
advisory was not yet in the registry's feed, so its next run fails the same
step. No source behaviour changed.

## What the registry said

`pnpm audit --audit-level=moderate`, as `just audit-web` runs it, reported
**1 finding**:

| Advisory            | Severity | Affected           | Patched    | In the lockfile |
| ------------------- | -------- | ------------------ | ---------- | --------------- |
| GHSA-6pq8-jhm2-2vcw | moderate | `>=6.7.0 <=6.16.0` | `>=6.16.1` | 6.16.0          |

It is reached through the dashboard only:
`frontends/apps/dashboard > @cratestack/refine > @refinedev/core > qs`,
`@refinedev/nextjs-router > qs`, and the Cypress dev dependency
`@cypress/request > qs`.

## The change

One entry under `overrides` in `pnpm-workspace.yaml`, `"qs@<6.16.1": ^6.16.1`,
dated there. It is a floor, not a pin, which is the 2026-10-09 class, not the
`next>postcss` class: every dependent takes `qs` on a caret range
(`@refinedev/core` 5.0.12 and `@refinedev/nextjs-router` 7.0.5 on `^6.10.1`,
`@cypress/request` 4.0.1 on `^6.15.2`, read from the registry). A bump of a
direct dependency would not move it, because the lockfile is what held
6.16.0.

`pnpm install` (pnpm 11.18.0, Node 24.21.0) moved the lockfile: the override
line, and `qs` 6.16.0 → **6.16.1** in its `packages` entry, its own
`snapshots` entry and the three dependents' `snapshots` references. Nothing
else moved.

**Why 6.16.1 and not the newest.** npm dates 6.16.1 2026-10-14T22:06Z, 6.17.0
2026-10-27T06:23Z and 6.16.2 2026-10-27T19:27Z. The last two were inside pnpm
11's 24-hour `minimumReleaseAge` when this was written, so the resolver
skipped them. No `minimumReleaseAgeExclude` entry was added, because 6.16.1
is already fixed.

## Measured after

- `just audit-web`, both runs (`--prod` and the whole workspace): **No known
  vulnerabilities found**, then `audit-web: ok`. 1 finding before.
- `pnpm --filter @vpay/dashboard typecheck`: exit 0.
- `pnpm --filter @vpay/dashboard test`: **150 passed, 1 skipped**, 16 files.
- `just verify`: the seventeen gates passed.

## Not done

- `pnpm -r test` across the whole workspace was not run. The checkout app and
  `examples/shop` do not resolve `qs`.
- Trivy was not run locally. CI's `trivy` job reads the same lockfile.
- The override has no removal condition beyond "re-check on the next audit
  pass", like the 2026-10-09 floors. Once a fresh resolution lands past
  6.16.0 for good, it constrains nothing and can go.
