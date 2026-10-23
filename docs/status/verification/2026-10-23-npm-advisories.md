# 2026-10-23 — Next 15.5.28 and 16.4.2 clear three Next.js advisories

## What this is

A dependency-only change, carried on the branch of
[#341](https://github.com/vaam-apps/vpay/pull/341). That PR's first CI run,
on `0249da35`, failed `trivy / Trivy` on `pnpm-lock.yaml`, a file it does not
touch. `master` at `8504536f` resolves the same versions, so its next run
fails the same step. No source behaviour changed. Four comments and READMEs
that named the dashboard's resolved Next now also name 15.5.28.

## What the registry said

Measured on the branch before the bump, with pnpm 11.18.0 and Node 22. `pnpm
audit --audit-level=moderate` reported **6 findings: 3 critical, 3 high, 1 of
them ignored**. The ignored one is `braces` GHSA-vfj7-8cjw-p6xm, which
`auditConfig.ignoreGhsas` already carries. Trivy reported the first two rows
below against 15.5.27.

| Advisory                                                                              | Severity | Affected                                  | Patched         |
| ------------------------------------------------------------------------------------- | -------- | ----------------------------------------- | --------------- |
| GHSA-2464-fhr4-w9xw: unauthenticated RCE in Image Optimization (published 2026-10-14) | critical | `>=10.0.0 <=15.5.27`, `>=16.0.0 <=16.4.0` | 15.5.28, 16.4.1 |
| GHSA-p3m5-59mp-jv5r (Trivy: CVE-2026-94482): DoS through Server Actions               | high     | `>=13.0.0 <=15.5.27`, `>=16.0.0 <=16.4.0` | 15.5.28, 16.4.1 |
| GHSA-673q-mjm6-r5xr: unauthenticated RCE in the `use cache` function                  | critical | `>=16.0.0 <=16.4.0`                       | 16.4.1          |

**The bump:**

| Package                                                                       | Before     | After      |
| ----------------------------------------------------------------------------- | ---------- | ---------- |
| `next`, `frontends/apps/checkout` and `frontends/apps/dashboard`              | `^15.5.27` | `^15.5.28` |
| `next`, `examples/shop` (exact)                                               | 16.3.8     | 16.4.2     |
| `@next/eslint-plugin-next`, `frontends/packages/config` (exact, rides `next`) | 16.3.8     | 16.4.2     |

None of them needed `minimumReleaseAgeExclude`. npm dates 15.5.28
2026-10-14T17:36Z and 16.4.2 2026-10-15T20:57Z, both past pnpm's 24-hour
window. 15.5.28 still pins `postcss: 8.4.31`, so the `next>postcss` override
stays load-bearing; `pnpm-workspace.yaml` says so in a dated line.

## Measured after

- `pnpm audit --audit-level=moderate`, whole workspace and `--prod`: **1
  finding each, the ignored `braces` id**. 6 before.
- `pnpm -r typecheck` and `pnpm -r lint`: exit 0.
- `pnpm -r test`, run as CI runs it, with no `storybook-static/` on disk:
  **1661 passed, 2 skipped, 0 failed**. That is the 2026-10-21 page's count.
  The dashboard's `payments-filters.test.tsx` calendar case timed out at
  5 s once, when the two apps' suites ran in parallel. Run alone, the
  dashboard passed 456 of 456 with 1 skipped.
- `next build`: checkout and dashboard on **Next.js 15.5.28**, and
  `examples/shop` on **Next.js 16.4.2 (Turbopack)**. All three exit 0.
- `build-storybook` for both apps: exit 0. Each stylesheet defines
  `--color-base-100` three times.

## Found on the way, and not this change's

**`a11y-gate.test.ts` fails in both apps whenever `storybook-static/`
exists.** Its "the BUILT storybook stylesheet actually defines the theme"
case expects `--color-base-100:\s*#`, a hex value. The `@vaam-apps/ui` 0.5.0
theme (#318, 2026-10-21) defines `--color-base-100:
var(--md-sys-color-surface)`. The case fails the same way with **`master`'s
own dependencies** (re-run with this change stashed: 1 failed, 556 passed in
checkout). CI never sees it: CI runs `pnpm -r test` before `just
build-storybook`, so the case takes its `ctx.skip()` branch. That leaves the
guard a test that does not run in CI and fails locally after any Storybook
build. It is reported, not fixed here.

## Not run

- `test-storybook`, the axe suites, which need a browser; CI's `web` job
  runs them.
- The `e2e (compose)` images; CI builds them.
- Nothing was ignored, and nothing was added to `ignoreGhsas` or
  `minimumReleaseAgeExclude`.
