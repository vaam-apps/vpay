# 2026-10-23 — the a11y gate's built-theme case resolves `var()`, and runs in CI

## What this is

The maintainer said "Fix it" about a finding first recorded on
[2026-10-23-npm-advisories.md](2026-10-23-npm-advisories.md) § "Found on the
way". In both `frontends/apps/checkout` and `frontends/apps/dashboard`,
`src/a11y-gate.test.ts`'s case "the BUILT storybook stylesheet actually
defines the theme" asserted `--color-base-100:\s*#`, a hex literal. The
`@vaam-apps/ui` 0.5.0 theme (#318, 2026-10-21) writes `--color-base-100:
var(--md-sys-color-surface)` and gives `--md-sys-color-surface` a hex value
per scheme, so the case failed on every local build. CI never saw it: the
`web` job runs `pnpm -r test` before `just build-storybook`, so there the case
always took its `ctx.skip()` branch. A guard that fails wherever it runs and
runs nowhere that matters was this repository's named failure mode.

## Before

On `master` at `9499f09d`, after `just build-storybook` (exit 0; the recipe's
own count printed `checkout --color-base-100 defined 3x, referenced 24x` and
`dashboard … defined 3x, referenced 25x`), the built stylesheets held:

| Declaration                                    | Checkout | Dashboard |
| ---------------------------------------------- | -------- | --------- |
| `--color-base-100:var(--md-sys-color-surface)` | 3×       | 3×        |
| `--md-sys-color-surface:#0f0c1b`               | 1×       | 1×        |
| `--md-sys-color-surface:#fdf7ff`               | 1×       | 1×        |

and `pnpm exec vitest run src/a11y-gate.test.ts` failed that case in both
apps, its message carrying the whole minified stylesheet (about 200 KB).

## The change

- **The assertion.** Each copy now resolves the token one `var()` deep:
  every definition of `--color-base-100` must be a colour literal, or a
  `var()` of a custom property every definition of which is one. It returns
  the reasons it does not, one short line each, and the case asserts that
  list is empty. A deeper chain is reported rather than followed, so a change
  in the theme's shape is looked at rather than silently accepted.
- **The run.** `just build-storybook` now runs the case in each app straight
  after its existing count, with `VPAY_STORYBOOK_BUILT=1`. With that set, a
  missing `storybook-static/` fails the case instead of skipping it. CI's
  `web` job already runs `just build-storybook`, so the case runs in CI for
  the first time, with no workflow change. Without the variable, the case
  still skips when there is no build, which is what `pnpm -r test` before a
  build needs.

## Proof

**After the change, build present:** `vitest run src/a11y-gate.test.ts` 6 of
6 in each app. Through the recipe: `just build-storybook` exit 0, and each
app's run reports `Tests 6 passed (6)`, none skipped.

**Mutations of the built checkout stylesheet**, one at a time, restored
from a copy after each (the restored file compared byte-identical):

| Mutation                                                   | Result                                                             |
| ---------------------------------------------------------- | ------------------------------------------------------------------ |
| `--md-sys-color-surface` defined nowhere                   | failed: "… names --md-sys-color-surface, which is defined nowhere" |
| `--color-base-100:var(--nope)`                             | failed: "… names --nope, which is defined nowhere"                 |
| `--color-base-100` defined nowhere (PR #135's own failure) | failed: "--color-base-100 is defined nowhere"                      |
| `--md-sys-color-surface:inherit` for one scheme            | failed: "… --md-sys-color-surface: `inherit` is not a colour"      |
| `--color-base-100:#0a0b0d` (the pre-0.5.0 literal shape)   | passed, as it should                                               |

**Without a build:** 5 passed and 1 skipped in each app. With
`VPAY_STORYBOOK_BUILT=1` and no build: the case fails, "…/storybook-static/assets
does not exist, yet `just build-storybook` says it just built it".

## Gates

| Check                                                      | Result                                                                                                                      |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `just lint-web` (`pnpm -r typecheck`, then `pnpm -r lint`) | exit 0                                                                                                                      |
| `pnpm -r test`, no `storybook-static/` (as CI runs it)     | exit 0: checkout 556 passed and 1 skipped, dashboard 456 passed and 1 skipped; every other package passed with none skipped |
| `pnpm -r test`, after `just build-storybook`               | exit 0: checkout **557 of 557**, dashboard **457 of 457** — the case that failed locally on `master` passes                 |
| `just build-storybook`                                     | exit 0, both counts and both runs of the case                                                                               |
| `just verify`                                              | ok, all fifteen: `verify-links` 2797 links, `verify-doc-counts` 13 counts                                                   |

## Not run

- `just test-storybook`, the axe suites in a real Chromium: unchanged by this,
  and CI's `web` job runs it.
- The `e2e (compose)` job.
