# 2026-10-23 — the built-stylesheet a11y case follows the theme's `var()` chain, and `just build-storybook` runs it

Branch `claude/beautiful-albattani-b73uab`, from `master` at `8504536f` (#340),
on `@vaam-apps/ui` 0.5.0. The change touches four files outside `docs/`:

- `frontends/apps/{checkout,dashboard}/src/a11y-gate.test.ts`: the case "the
  BUILT storybook stylesheet actually defines the theme" has a new assertion;
- `justfile`: `build-storybook` runs both of those files;
- `.github/workflows/ci.yml`: a comment only.

A second commit on the branch ports the dependency half of
[vaam-apps/vpay#341](https://github.com/vaam-apps/vpay/pull/341)'s `0fb04ec5`
verbatim: `next` `^15.5.28` and 16.4.2, the lockfile, and the version
mentions that moved with them. `master` at `8504536f` fails `just audit-web`
on three Next.js advisories. CI's `web` job runs that audit before
`pnpm -r test` and `just build-storybook`, so without the port this branch's
CI would never reach the change. The evidence for the bump is #341's own page,
`docs/status/verification/2026-10-23-npm-advisories.md`, which lands with #341.

**Later the same day #341 merged, as `c672fffc`, and `master` (then at
`82a68f4e`) was merged into this branch.** The ported files are now identical
to `master`'s, so the port is a no-op. On the merged tree:

- `pnpm install --frozen-lockfile`, `just verify`, `just fmt-check-web`,
  `just lint-web` and `just audit-web` exit 0;
- in the `web` job's order, `pnpm -r test` gives 1662 passed / 2 skipped
  (`master` added one `sdks/nodejs` case), then `just build-storybook` exits
  0 with both apps' `a11y-gate.test.ts` at 6 / 6.

**No Rust, no migration, no route, no `NotImplemented` token and no gate in
`just verify` moved.** [vaam-apps/vpay#341](https://github.com/vaam-apps/vpay/pull/341)
reported the defect in its "Not done" list and its
`docs/status/verification/2026-10-23-npm-advisories.md`.

## What was wrong

The case asserted `--color-base-100:\s*#`, a hex value, against
`storybook-static/assets/*.css`. `@vaam-apps/ui` 0.5.0
([vaam-apps/vpay#318](https://github.com/vaam-apps/vpay/pull/318), 2026-10-21)
maps every daisyUI token onto a Material 3 role. The built sheet, measured on
both apps after `just build-storybook`:

| Property                 | Selector                                                                                | Wrapped in                                           | Value                         |
| ------------------------ | --------------------------------------------------------------------------------------- | ---------------------------------------------------- | ----------------------------- |
| `--color-base-100`       | `:root:not([data-theme])`                                                               | `@media(prefers-color-scheme:dark)` in `@layer base` | `var(--md-sys-color-surface)` |
| `--color-base-100`       | `:where(:root),:root:has(input.theme-controller[value=dark]:checked),[data-theme=dark]` | `@layer base`                                        | `var(--md-sys-color-surface)` |
| `--color-base-100`       | `:root:has(input.theme-controller[value=light]:checked),[data-theme=light]`             | `@layer base`                                        | `var(--md-sys-color-surface)` |
| `--md-sys-color-surface` | `:root,[data-theme=dark]`                                                               | no layer                                             | `#0f0c1b`                     |
| `--md-sys-color-surface` | `[data-theme=light]`                                                                    | no layer                                             | `#fdf7ff`                     |

There is no hex on `--color-base-100` anywhere, so the case failed after
every correct build. Measured here on `master`'s own test files and freshly
built artefacts:

| Suite                                | Result                                                                 |
| ------------------------------------ | ---------------------------------------------------------------------- |
| `pnpm --filter @vpay/checkout test`  | exit 1, **1 failed / 556 passed** (#341 measured the same)             |
| `pnpm --filter @vpay/dashboard test` | exit 1, **1 failed / 456 passed**                                      |
| failing case, both                   | `expected '/*! tailwindcss v4.3.3 …' to match /--color-base-100:\s*#/` |

`just build-storybook` exited 0 throughout: its grep counts
`--color-base-100:` (defined 3×, referenced 24× in the checkout and 25× in the
dashboard) and never looks at the value.

## Why CI never saw it

The `web` job runs `pnpm -r test` before `just build-storybook`. When this case
runs there, `storybook-static/` does not exist, so it takes its `ctx.skip()`
branch. It had never run against a build in CI. **Vitest exits 0 on a skipped
case**: measured with the artefact moved aside, the JSON reporter said
`numPendingTests: 1` and the process exited 0. So nothing in CI could have
reported this.

## The fix

**The assertion.** The case parses the built sheet with `postcss` and hands it
to `themeColours`, the cascade-aware resolver in each app's
`src/testing/contrast.ts`. That resolver drops a declaration pinned to the
other theme. It applies "unlayered beats layered". And it follows `var()` to a
literal, which it learnt to do in #318. For `dark` and for `light`,
`--color-base-100` must resolve to an opaque colour equal to what the same
function resolves from the package's own `@vaam-apps/ui/styles/theme.css`.
Measured with a throwaway probe of the same function, deleted after: dark
`#0f0c1b` and light `#fdf7ff`, for both apps' built sheets and for the package.

The expected value comes from the package. It is not a hex literal, and not a
role name: either would break on the next theme redesign the way the hex broke
on this one, which is the same reason `styling-gate.test.ts` reads the
permitted set from the package. The `ctx.skip()` when no build exists is
unchanged.

**The CI wiring, the maintainer's choice** among four options put to them on
2026-10-23: re-run the test from the recipe; reorder the CI steps; add a CI
step; leave CI as is. `just build-storybook` keeps its grep, which is the
cheap first check and whose counts are the diagnostic. Then it runs each
app's `src/a11y-gate.test.ts` with Vitest's JSON reporter, and fails unless
every case in the file passed. A skip there therefore fails the recipe. CI's
`web` job already calls the recipe, so `ci.yml` gains only a comment. Cost:
the Vitest run is about 2 s per app (`Duration 2.02s`, checkout).

## Mutations, each seen to fail, each reverted

M1 and M2 were built with `pnpm --filter @vpay/<app> build-storybook`, and
the grep column gives the counts measured on that artefact. M3 and M3b edit
a correct artefact directly. R1 and R2 ran the recipe itself.

| #   | Mutation                                                                                                                 | App(s)                            | What the recipe's grep sees                   | The case                                                                                                                     |
| --- | ------------------------------------------------------------------------------------------------------------------------ | --------------------------------- | --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| M1  | theme `@import` deleted from `app/globals.css`                                                                           | checkout, dashboard               | defined 0×, referenced 24× / 25×, so it fails | **fails**: "does not resolve to the package's "dark" ground" (the position case fails too)                                   |
| M2  | theme `@import` moved below `@plugin` (#135's original defect)                                                           | checkout                          | defined 0×, referenced 24×, so it fails       | **fails**, same message; the build log carries Vite's `@import must precede all other statements`                            |
| M3  | every `--md-sys-color-surface:` declaration deleted from the artefact, `var()` mapping kept                              | checkout, dashboard               | **passes**: defined 3×                        | **fails**: "does not resolve to the package's "dark" ground"                                                                 |
| M3b | only the light theme's `--md-sys-color-surface:#fdf7ff` deleted from the artefact                                        | checkout                          | passes                                        | **fails**: "… the package's "light" ground"                                                                                  |
| R1  | the case pointed at `storybook-static/assetz`, so it skips, then `just build-storybook`                                  | checkout                          | passes                                        | skipped; Vitest `5 passed \| 1 skipped`, exit 0; **recipe exits 1**: "passed 5 of 6 cases … a skipped case measured nothing" |
| R2  | both `--md-sys-color-surface` declarations deleted from the installed package's `theme.css`, then `just build-storybook` | checkout (the recipe stops there) | **passes**: defined 3×                        | **fails** ("the package's own theme.css resolves …"); **recipe exits 1**                                                     |

M3 and R2 are the reason for the change, not just a check of it. Each leaves
a sheet the grep accepts, while the role the mapping points at is gone. R2's
package edit used `sed -i`, which writes a new file, so the pnpm store copy
was never touched. The file was restored afterwards and its SHA-256
(`d9c4ce64…`) re-checked.

Restored: `git diff` shows no change to either `globals.css`. A fresh
`just build-storybook` exits 0 and prints, for each app, `--color-base-100
defined 3x`, then `Tests  6 passed (6)`.

## Gates

| Check                                                         | Result                                                                                                                                                    |
| ------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just build-storybook`                                        | exit 0; checkout and dashboard `a11y-gate.test.ts` 6 / 6 each                                                                                             |
| `pnpm -r test`, artefacts present                             | exit 0, **1663 passed, 0 skipped**: checkout 557, dashboard 457, config 66, tokens 12, stripe-js 152, nodejs 236, api-client 4, tauri plugin 71, shop 108 |
| `pnpm -r test`, no artefacts (CI's order)                     | exit 0, **1661 passed, 2 skipped**: the built-stylesheet case in each app                                                                                 |
| `pnpm --filter @vpay/{checkout,dashboard} typecheck` / `lint` | exit 0, all four                                                                                                                                          |
| `just verify`                                                 | exit 0, the fifteen gates (`verify-doc-counts` reads this directory's file count; `verify-links` the four links to this page)                             |
| `just fmt-check`, `just lint-web`                             | exit 0, before and after the port                                                                                                                         |
| `just audit-web`                                              | exit 1 before the port, on `next` (GHSA-2464-fhr4-w9xw, GHSA-p3m5-59mp-jv5r, GHSA-673q-mjm6-r5xr); **exit 0 after it**                                    |
| the `web` job's order, after the port                         | artefacts deleted; `pnpm -r test` exit 0, 1661 passed / 2 skipped; then `just build-storybook` exit 0, both apps' `a11y-gate.test.ts` 6 / 6               |
| `test-storybook`, both apps, after the port                   | exit 0: checkout **26 / 26**, dashboard **36 / 36**. Not through the recipe: see below                                                                    |

One run is recorded rather than dropped. The first `pnpm -r test` on this
branch failed two dashboard cases with `Test timed out in 5000ms`, both in
`src/components/payments-filters.test.tsx`, a file this change does not
touch. A `cargo install` was compiling on the same 4 cores at the time (load
average 8.4). Re-run on an idle machine: that file 12 / 12, the dashboard
suite 457 / 457, and the two full runs above.

`test-storybook` ran through each app's own script, with the recipe's cache
clearing done by hand, and not through `just test-storybook`. That recipe
starts with `playwright install chromium`. Playwright 1.63 wants Chromium
revision 1243, and this container has 1194 (Chromium 141), with a rule
against downloading another. So both apps' `vitest.storybook.config.ts` were
pointed at `/opt/pw-browsers/chromium` for the run and reverted after it. CI
runs the pinned browser.

## In CI

The first CI measurement of the case is run
[40011902343](https://github.com/vaam-apps/vpay/actions/runs/40011902343),
`web` job, on this branch's head `49f7d0a1`. The job passed:

- `pnpm -r test`: `src/a11y-gate.test.ts (6 tests | 1 skipped)` in each app,
  the skip this page expects; checkout 556 passed / 1 skipped, dashboard 456 /
  1;
- `just build-storybook`: `checkout --color-base-100 defined 3x, referenced
24x`, then `a11y-gate.test.ts (6 tests)`, 6 passed; `dashboard … defined
3x, referenced 25x`, then 6 passed. No skip;
- `just test-storybook`: 26 passed and 36 passed, on the pinned browser.

`rust` and `e2e (compose)` passed on the same run. Its `supply chain` job
failed on RUSTSEC-2026-0386 against `tokio 1.53.1` in `Cargo.lock`. This
branch does not change `Cargo.lock`, which is identical to `master`'s, and
`master`'s last green run predates the advisory.

## Not done

- The skip on `pnpm -r test` remains, by design. `just ci` builds no
  Storybook, so a local `just ci` still reports the case skipped. It is
  measured where a build exists: in `just build-storybook`, and in CI's `web`
  job through it.
- The resolver reads tokens and does not lay out a page, so it cannot say
  whether `--color-base-100` resolves on a given element. That remains
  `just test-storybook`'s job; this case only proves the theme is in the
  artefact that suite renders.
- The Rust half of `just ci` was not run: `clippy`, `test-rust`, `test-doc`,
  `verify-ignored` and `deny`. No Rust or Cargo file changed. This container
  has no Docker daemon for the testcontainers suites, and no `cargo-nextest`
  or `cargo-deny`. `just verify`, which builds and runs `xtask`, was run.

## Merged with `master` on 2026-10-24

[vaam-apps/vpay#355](https://github.com/vaam-apps/vpay/pull/355) fixed the
same case first, in parallel, with a one-`var()` regex resolution and a
second run of this file after the recipe's loop with `VPAY_STORYBOOK_BUILT=1`
([2026-10-23-a11y-gate-built-theme.md](2026-10-23-a11y-gate-built-theme.md)).
The maintainer chose to merge this branch as well, so `master` was merged into
it and the three conflicts were resolved like this:

- **Both `a11y-gate.test.ts` files.** This branch's assertion is kept: the
  whole chain per theme, compared with the package's own `theme.css`. #355's
  four one-hop helpers (`definitionsOf`, `COLOUR_LITERAL`, `VAR_REFERENCE`,
  `unresolvedColours`) are removed, since nothing calls them now. #355's skip
  guard is kept: with `VPAY_STORYBOOK_BUILT=1` and no artefact, the case fails
  with its own message.
- **`justfile`.** One run per app, inside the loop. It sets
  `VPAY_STORYBOOK_BUILT=1` and keeps this branch's JSON-report check. #355's
  two runs after the loop are removed, because they ran the same file again.
- **`docs/status/README.md`.** Both lists are kept and the counts are now
  114 files, 91 named. #355's page and its rows in `frontend.md` and the
  dashboard flow are marked superseded rather than rewritten.

Measured on the merge, in this session:

| Check                                                                                       | Result                                                                                  |
| ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `pnpm install --frozen-lockfile`, typecheck and lint for both apps                          | exit 0                                                                                  |
| `just build-storybook`                                                                      | exit 0; each app's `a11y-gate.test.ts` ran once against the fresh artefact, 6 of 6 each |
| Mutation: every `--md-sys-color-surface:` declaration removed from the checkout's built CSS | the case fails (1 failed, 5 passed); restored, 6 of 6                                   |
