# exp26 Lane D notes — `frontends/apps/dashboard`

Branch `claude/exp26-ui-lane-d`. **Rebased once**, onto Lane A's own review
fixes — see "The rebase onto Lane A's review (08d9b8e)" below. Original base
`177645e` (Lane A's first head); current base `08d9b8e` (Lane A's head after
its own Opus review landed 7 fix/docs commits, including the exact defect
that broke this app's `next build`). Commits, in order, on the current base:

1. `0aed570` — the toolchain move: `tailwindcss` 3.4.17 → 4.3.3, `daisyui`
   4.12.23 → 5.7.28, `autoprefixer` deleted in favour of
   `@tailwindcss/postcss`; `tailwind.config.ts` deleted; `postcss.config.js`
   rewritten; `eslint.config.js` gains `tailwind: true` (added on rebase —
   see below). `pnpm-lock.yaml` updated by a real `pnpm install`, not
   hand-edited.
2. `940bb07` — `app/layout.tsx` and `app/page.tsx` rewritten as pure
   `@vpay/ui` composition; `data-theme` `corporate` → `bumblebee`.
3. `c386915` — four component recipes (`src/recipes/`), this app's first
   jsdom test environment (`vitest.config.ts`, `vitest.setup.ts`), and
   `README.md`.
4. `68e1b18` — the layout's own test (`src/layout.test.tsx`), pinning
   `data-theme="bumblebee"` and the brand `<h1>` inside the `<nav>`. Added
   after finding, by actually running plan §5 Lane D's own named mutation
   ("the layout's `data-theme` reverts to `corporate`"), that nothing in the
   suite up to that point caught it — see "For the reviewer" below.
5. `ee6fa29` — docs (this file, `docs/status.md`, `docs/flows/dashboard.md`).

Head at report time: run `git rev-parse HEAD` in the worktree — do not trust
a pasted SHA in this file over that command.

## The rebase onto Lane A's review (`08d9b8e`)

The coordinator reported, mid-report, that Lane A's own Opus review had
landed 7 commits fixing (among other things) **the exact defect this note's
"Where the plan was wrong" section below documents independently**: `@vpay/
ui`'s relative imports carried a `.js` suffix that Next's webpack resolver
takes literally, so `pnpm --filter @vpay/dashboard build` failed with
`Module not found: Can't resolve './cn.js'` on `177645e`. This lane's own
`next.config.ts` `webpack.resolve.extensionAlias` entry was a workaround for
that defect written **before** Lane A's fix existed. `git rebase 08d9b8e`
(no conflicts — the only file both lanes touched was `docs/status.md`, on
non-overlapping lines) picked up Lane A's real fix, which made the
workaround provably redundant:

  - `pnpm --filter @vpay/dashboard build` was re-run with the workaround
    removed entirely — exit 0, "Compiled successfully", 4/4 static pages —
    before it was taken out for good. `next.config.ts` now reads exactly as
    it did in the untouched scaffold; `git diff 08d9b8e..HEAD --
    frontends/apps/dashboard/next.config.ts` is empty.
  - This is recorded so the workaround's absence isn't mistaken for an
    oversight: it existed, it worked, and it was removed once the fix it
    worked around no longer needed one.

Three more actions taken on the rebased tree, all at the coordinator's
direction:

1. **`tailwind: true` added to `frontends/apps/dashboard/eslint.config.js`**,
   beside `react: true`, folded into commit 1 (the toolchain-move commit) by
   `git commit --fixup` + `git -c sequence.editor=true rebase -i
   --autosquash` (this repository has no network TTY for a real interactive
   rebase; the no-op sequence editor makes `--autosquash` run
   non-interactively). Confirmed active with `eslint --print-config`
   (`better-tailwindcss/*` present, six rules) rather than assumed from the
   flag alone — the same sanity check Lane A's own review used to catch it
   silently not running the first time. **Zero findings**: this app's
   `app/layout.tsx`/`app/page.tsx`/`src/recipes/*` carry zero raw
   `className` strings, so there is nothing for the class-string rules to
   flag — consistent with, not merely compatible with, this lane's own
   "styling_files 2 → 0" measurement.
2. **`just verify-ui` re-run on the rebased tree.** Still exactly one
   failure, unchanged: `frontends/apps/checkout/src/components/
   screens.tsx`'s `form-control`/`label-text` (Lane B's unmigrated tree).
   The check itself is stricter now (Lane A's review closed three measured
   holes: no `black`/`white`, no arbitrary colour value, no lookup-object
   class was being checked) — re-run **scoped to `frontends/apps/dashboard`
   alone** against all six current checks (the original four plus the split
   arbitrary-colour check and the `.js`-suffix regression guard, the latter
   scoped by the gate itself to `frontends/packages/ui/src` and not this
   app), all six clean.
3. **`.js`-suffixed relative imports removed from this app's own 5 test
   files** (`src/layout.test.tsx` and the four `src/recipes/*.test.tsx`
   files), for consistency with the house style Lane A's fix established —
   not because `verify-ui`'s check 5 reaches this directory (it doesn't;
   these files are never built by Next's webpack, only transpiled by
   esbuild under Vitest, which resolves the suffix either way) but because
   leaving a mix invites the exact confusion the coordinator's message was
   about. `pnpm --filter @vpay/dashboard typecheck`/`lint`/`test` re-run
   clean after (9/9 tests, 5 files).

`cn()`'s fix (Lane A's review, commit `1cca924` — a daisyUI colour and style
class now compose instead of the style class silently dropping the colour)
was read and is informational only for this lane: none of the four recipes
or the layout/page composes two conflicting `cn()` inputs of that shape —
each uses a component's own `tone`/`variant` prop (`StatusBadge`,
`Alert tone=`, `Button variant=`), never a raw `cn('btn-primary',
'btn-outline')` call. Nothing here needed the fix, and nothing here regresses
without it.

## Coordination with exp24

Checked again before starting, per the plan's own instruction (§5 Lane D):
`claude/exp24-staff-auth` was not queried directly (no access to that
worktree from here), but nothing under `frontends/apps/dashboard` in *this*
worktree carried any trace of it — the scaffold was still the plan's
described ten-file baseline before this lane started (`git log -- frontends/
apps/dashboard` on the base commit shows no exp24 commits). Lane D proceeded
against the scaffold as the plan expects, and no rebase or amendment was
needed.

## Before / after (`exp26-plan-count.sh`)

Before (`177645e`, Lane A's head — matches Lane A's own "unchanged" row):

```
@vpay/dashboard   styling_files=2   class_tokens_distinct=17   classname_sites=7
                  class_tokens_total=17   css_lines=3
```

After (this head):

```
@vpay/dashboard   styling_files=0   class_tokens_distinct=0   classname_sites=0
                  class_tokens_total=0   css_lines=1
```

Brief's targets: `styling_files` 2 → ≤ 1, `class_tokens_distinct` 17 → ≤ 4.
Plan §5 Lane D's own acceptance: `styling_files` 2 → **0**. Both exceeded —
**zero** raw `className` strings in either `app/layout.tsx` or
`app/page.tsx`; every visual decision now lives behind `@vpay/ui`.
`css_lines` 3 → 1 is the one line left in `globals.css`, the `@import
'@vpay/ui/styles.css';` per plan §4.4.

`OUTSIDE @vpay/ui` (the row the repo-wide 80% gate reads) moved from
`styling_files=18` to `styling_files=16` and `class_tokens_distinct=92` to
`85` — exactly this lane's contribution; `checkout` and `shop` are
untouched, as scoped.

## Gates, recipe by recipe

All re-run on the rebased tree (`08d9b8e`..`HEAD`), not carried forward from
before the rebase.

| recipe | result | evidence |
|---|---|---|
| `pnpm install` (real, dashboard's `package.json` changed twice, `pnpm-lock.yaml` re-resolved after the rebase) | ✅ | 0 peer-dependency errors, "Lockfile is up to date" after rebase |
| `pnpm --filter @vpay/dashboard typecheck` (`tsc --noEmit`) | ✅ | clean, on the rebased tree with `next.config.ts`'s webpack workaround removed |
| `pnpm --filter @vpay/dashboard build` | ✅ | `next build` succeeds **without the webpack workaround** (Lane A's real fix supersedes it) — re-verified by removing the workaround, rebuilding, and only then deleting it for good. **Decisive check per plan §6.4**: `.next/static/css/*.css` inspected directly — `bumblebee`, `.alert-warning`, `.badge-success`, `.badge-error`, `.badge-warning`, `.badge-neutral`, `.badge-info` all present, `corporate`/`form-control`/`label-text` absent |
| `pnpm --filter @vpay/dashboard lint` | ✅ | clean, **including the six `better-tailwindcss` rules now wired** (`tailwind: true`, confirmed active via `eslint --print-config`) — zero findings, because this app's shipping files carry zero raw `className` strings |
| `pnpm --filter @vpay/dashboard test` | ✅ | 5 files, 9 tests. Re-run after the `.js`-suffix cleanup in the app's own test files, still 9/9 |
| `just lint-web` (whole workspace) | ✅ | clean across all 15 buildable packages, on the rebased tree |
| `just test-web` (whole workspace) | ✅ | `@vpay/checkout` 448/448 (unchanged), `@vpay/ui` **60/60** (Lane A's review added 14 cases — was 46/46 before the rebase), `@vpay-examples/shop` 96/96 (unchanged), `@vpay/dashboard` **9/9**, plus every SDK/tokens/config/api-client package unaffected |
| `just verify-ui` (whole tree) | 🔴 (expected, not a Lane D defect) | fails only on `frontends/apps/checkout/src/components/screens.tsx`'s `form-control`/`label-text` — Lane B's unmigrated tree, unchanged by the rebase. The check itself is **stricter now** (Lane A's review closed three measured holes in the colour check and split it into a lookup-object-aware palette check plus a new arbitrary-colour-value check; added a `.js`-suffix regression guard scoped to `frontends/packages/ui/src`). **Scoped to `frontends/apps/dashboard` alone, all six current checks pass** — the palette+black/white check, the arbitrary-value check, the daisyUI-4-class check, the `!important` check, the `.js`-suffix check (not scoped here by the gate itself, but run anyway — clean after this lane's own cleanup) and the `cva` check, each run directly with the gate's own `git grep` pattern |
| `just verify-links` | ✅ | 909 links, 164 files |
| `just verify-npm-scope` | ✅ | unaffected — no package renamed or removed |
| `just verify-status` | ✅ | `docs/status.md`'s one declared `NotImplemented` item still matches shipping code |
| `just audit-web` | ✅ | "No known vulnerabilities found" on both `--prod` and the whole workspace, re-run after the rebase |
| `just test-e2e` (real Cypress, full recipe, against `compose.e2e.yml` + `compose.demo.yml`) | ✅ | **Run twice, both for real**, isolated (`demo_project=exp26d`, five overridden ports) from the sibling lanes' `vpay-demo` stack. **First run** (pre-rebase tree): `checkout.cy.ts` 1/1, `dashboard.cy.ts` 3/3, `shop-hosted.cy.ts` 3/3 — `shop-embedded.cy.ts` (`e2e:framed`) did not finish inside this report's own time budget; measured, not assumed, that the stall was host contention rather than a defect (20 Cypress/Electron processes system-wide, load average 24–36 on 24 cores, five sibling lanes' own `test-e2e` runs); the stack was killed and manually torn down (containers, volume, network all confirmed removed) rather than left orphaned. **Second run, on the final rebased head, prompted by the coordinator's demand for a repo-wide green** — the exact same recipe, run to completion this time despite even worse measured contention (load average peaked at 82.97): **all 11 tests, all four specs, 0 failing** — `checkout.cy.ts` 1/1, `dashboard.cy.ts` **3/3**, `shop-hosted.cy.ts` 3/3, `shop-embedded.cy.ts` 4/4. `just test-e2e`'s own teardown ran to completion this time (`docker compose … down -v`, exit 0); containers, volume, network and all five overridden ports confirmed gone afterward |
| `just ci` | not run | the whole-revamp gate (plan §7), meant for the final merged head after all four lanes land |

## What was NOT done

- **Nothing, as it turns out — `shop-embedded.cy.ts` completed on the
  second run.** The first `test-e2e` run (pre-rebase) genuinely did not
  finish `e2e:framed` within a reasonable window, and this bullet
  originally recorded that as host contention rather than a defect (20
  Cypress/Electron processes system-wide, load average 24–36). That
  diagnosis is now confirmed rather than merely argued: the coordinator's
  demand for a repo-wide-green re-run, on the final rebased head, completed
  the exact same recipe end to end — **11/11 tests, all four specs** —
  under measurably *worse* contention (load average peaking at 82.97
  mid-run) than the first attempt that stalled. The difference was patience
  and Docker's warm build cache, not a code change to anything
  `shop-embedded.cy.ts` exercises. Left for the record because a reader of
  an earlier draft of this note would reasonably have concluded the app
  had an untested embedded-mode interaction; it does not — see the Gates
  table's `just test-e2e` row for the final numbers.
- **No real-browser contrast check** (`cypress-axe`, plan §7 row 6) — not
  built by this lane; Lane A's own note already scopes it to "whichever
  lane lands last, or a dedicated pass".
- **No structural axe pass added for this app specifically.** `@vpay/ui`'s
  own axe suite (Lane A) already covers every component this app composes;
  adding a second one here would test the same components' accessibility a
  second time rather than this app's own markup, which is composition only.
- **`Tabs`, `Skeleton`, `Toast` — still not built.** None of the four
  recipes needed them. Plan §3's own instruction ("build them in Lane A
  only if Lane D's dashboard screens actually need them… if Lane D does not
  need them, they are not built and this document is edited to say so") is
  satisfied by this sentence.
- **No page in `app/` uses the four recipes.** They exist in `src/recipes/`
  for exp24 (or whoever lands slice 1) to build on, per the brief
  ("so exp24's pages land on your components rather than raw classes") —
  wiring them into a live route would mean fabricating a data source, which
  is out of scope for a UI-only lane and is exactly the failure mode
  `AGENTS.md` warns against.
- **The `@base-ui/react/otp-field` subpath was considered and not used.**
  Plan §0.1 lists it among 1.8.0's public subpaths; a segmented one-time-
  code input would be a natural fit for `SignInForm`. Not built because it
  has no `@vpay/ui` wrapper yet (out of this lane's owned files) and a raw
  Base UI primitive styled ad hoc in an app file would be the "no cva
  outside `@vpay/ui`" rule's spirit violated even where its letter (no
  literal `cva(` call) would pass. `SignInForm`'s code field stays a plain
  `Input` with `inputMode="numeric"`.

## Where the plan was wrong, measured rather than assumed

1. **`next build` fails on a fresh `@vpay/ui` import, and nothing in the
   plan or Lane A's notes catches it.** `@vpay/ui`'s own source imports its
   siblings with a `.js` specifier that resolves to a `.ts` file
   (`tsconfig.base.json`'s `moduleResolution: "bundler"` — the pattern
   Vite/esbuild resolve natively). Lane A's own gates never hit this:
   `pnpm --filter @vpay/ui build` is `tsc --noEmit` (no bundling),
   `build-storybook` uses Vite (resolves it natively), and no app had
   imported `@vpay/ui` yet when Lane A reported. This lane is the first
   consumer, and `next build` failed immediately with `Module not found:
   Can't resolve './cn.js'` until `next.config.ts` gained
   `webpack.resolve.extensionAlias: { '.js': ['.ts', '.tsx', '.js'] }`.
   **Lane B will hit the exact same failure the moment `frontends/apps/
   checkout` imports `@vpay/ui`** — recorded here and in `docs/status.md` so
   it is not rediscovered from scratch.

   **Superseded, 2026-09-07:** Lane A's own Opus review found and fixed the
   actual defect at its source — the `.js` suffix on every relative import
   under `frontends/packages/ui/src` (48 files), not a per-consumer
   workaround — and gated it (`just verify-ui` check 5, scoped to that
   directory). This lane rebased onto that fix (`08d9b8e`) and removed its
   own `next.config.ts` workaround once `pnpm --filter @vpay/dashboard
   build` was confirmed to still pass without it. What follows (item 2,
   about the workaround's own typing) is kept as history — the workaround
   no longer exists — because the same shape of typing problem
   (`NextConfig['webpack']` being `any`) will recur for any future
   `next.config.ts` customisation in this workspace.
2. **(Historical — the workaround this item describes was removed on
   rebase; kept for the next person who reaches for a webpack config
   override.) `NextConfig['webpack']` is typed `any` in Next's own types**,
   so the naive fix (`webpack(config) { config.resolve.extensionAlias =
   …; }`) fails `@typescript-eslint/no-unsafe-member-access` /
   `no-unsafe-return` under this repo's type-aware lint config. Worked
   around with a small local interface (`WebpackConfigWithResolve`)
   narrowing only the one field touched, rather than reaching into the
   `any` or suppressing the rule.
3. **The plan's own `verify-ui` snippet (§7) would have passed silently
   over a real bug in this lane's first draft of `vitest.setup.ts`.**
   Copying `frontends/packages/ui/vitest.setup.ts` verbatim landed a
   `PointerEventPolyfill as unknown as typeof PointerEvent` cast that is
   unnecessary here — not because the code is wrong, but because
   `@vpay/ui`'s own `eslint.config.js` lists `*.setup.ts` under
   `outsideTsconfig` (so that file is never in `@vpay/ui`'s own type-aware
   lint program, and `tsc -p tsconfig.json` there doesn't include it
   either — its `include` is only `["src", ".storybook"]`), while this
   app's `tsconfig.json` includes `**/*.ts` at the root, so its own
   `vitest.setup.ts` **is** type-checked. `eslint --fix` removed the now-
   provably-unnecessary cast; the polyfill logic itself is unchanged.
   Recorded because "copy the sibling package's setup file" is the kind of
   thing a future lane will reach for again, and the same file reads
   differently depending on which `tsconfig.json` it lands under.

## For the reviewer

Decisive checks run and confirmed, not merely read:

- **Compiled CSS, not a render** (plan §6.4): `.next/static/css/*.css`
  after `pnpm --filter @vpay/dashboard build` was grepped directly for
  `bumblebee`, `.alert-warning`, `.badge-success`, `.badge-error`,
  `.badge-warning`, `.badge-neutral`, `.badge-info` — all present, count 1
  each — and for `form-control`/`label-text` — absent. `rm -rf .next` was
  run between builds so a stale artefact could not be mistaken for a fresh
  one.
- **`verify-ui`'s four checks, scoped to this app alone**: the same four
  `git grep -nE` patterns the `justfile` recipe uses, run individually
  against `frontends/apps/dashboard` only (not the whole tree, which fails
  on Lane B's unmigrated `checkout`) — all four returned nothing.
- **Flake check**: `pnpm --filter @vpay/dashboard test` run five times in a
  row after the recipes landed — 9/9 every time, 0 unhandled errors —
  following Lane A's own finding that an unmounted Base UI render can
  surface a stray jsdom teardown error roughly once in ten runs. Every test
  here calls `unmount()` explicitly.
- **Mutation named in plan §5 Lane D's own table, in full — this is the one
  finding in this note that changed what shipped.** "the layout's
  `data-theme` reverts to `corporate`" must fail. It didn't: at the point
  commits 1–3 landed, nothing in the suite asserted the theme attribute at
  all (`dashboard.cy.ts` checks `h1` text and status badges, not
  `data-theme`). Run for real: `app/layout.tsx` hand-edited to
  `data-theme="corporate"`, then `pnpm --filter @vpay/dashboard build`
  re-run — the build **succeeded** and the compiled CSS carried **zero**
  `corporate` rules (`frontends/packages/ui/src/styles.css` configures
  `bumblebee` only), so the page would render completely unthemed in a real
  browser with no error anywhere. `src/layout.test.tsx` (commit `68e1b18`)
  was added in response and confirmed, by re-running the same mutation, to
  fail with the exact expected/received values before the fix was reverted
  and the test landed for real. This is the plan's own §6.3 warning
  ("daisyUI 5 class renames — the silent failure… does not error") in a
  different shape: a *theme name* that silently stops styling anything is
  the same failure mode as a *class name* that does.
- **What was read but not independently re-derived**: Lane A's own report
  (`docs/plans/exp26-notes/lane-a.md`) that `checkout`/`shop`/`dashboard`
  were untouched by Lane A and that `verify-ui` was already red on the
  unmigrated checkout tree before this lane started — taken as given
  rather than re-audited file-by-file, since Lane A's own decisive
  mutations for that claim are already recorded there.

## A coordinator report that did not reproduce on this tree

Mid-report, the coordinator relayed two findings "Lane B measured that are
yours": (1) `frontends/apps/dashboard/tailwind.config.ts` fails typecheck
under `just lint-web` on the Tailwind 4 base (daisyUI 5 ships no types for
the default import); (2) the dashboard's Docker image fails to build until
`next.config.ts` lists `@vpay/ui` in `transpilePackages`.

**Neither reproduces on this branch, checked directly rather than assumed:**

1. `frontends/apps/dashboard/tailwind.config.ts` does not exist on this
   tree — `ls` reports "No such file or directory". It was deleted in this
   lane's first commit (`0aed570`, plan §4.2's own instruction) and has
   never existed on any commit since. `find . -iname tailwind.config.ts`
   across the whole repository on this branch finds exactly one file:
   `frontends/apps/checkout/tailwind.config.ts` — Lane B's own, not owned
   by this lane and not touched here. `just lint-web` (which runs `pnpm -r
   typecheck` across every workspace package, so a failure anywhere shows
   up as a repo-wide failure) was re-run on the final head after this
   report and is clean.
2. `frontends/apps/dashboard/next.config.ts`'s `transpilePackages` has
   listed `@vpay/ui` since before this lane touched the file at all — it
   was already there in the untouched ten-file scaffold this lane started
   from. **And it is empirically proven to build**: the `exp26d-dashboard`
   Docker image was built twice from this branch (once before the rebase
   in this report, once after, on the final head) and both times
   `docker compose … up -d --build --wait` reported `exp26d-dashboard-1
   Healthy`, and `dashboard.cy.ts` passed 3/3 against it both times.

Most likely explanation, not confirmed: the coordinator's message describes
what Lane B found combining lanes in a way this report has not — a merge of
A+B+C+D, or an earlier snapshot of this branch before its first commit
landed the deletions above. Recorded here, with the direct checks, rather
than silently "fixed" (there is nothing on this tree to fix) or silently
ignored (the report deserves an answer). `just lint-web` and a second full
`just test-e2e` were both re-run on the final head specifically because the
coordinator asked for repo-wide green on this lane's head, landing last —
see the Gates table above and its `dashboard.cy.ts` row for the second run.
