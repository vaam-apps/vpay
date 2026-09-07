# exp26 Lane D notes — `frontends/apps/dashboard`

Branch `claude/exp26-ui-lane-d`, base `177645e` (Lane A's head — `@vpay/ui`
rebuilt on Base UI 1.8.0 + daisyUI 5 + Tailwind 4, the shared preset in
`@vpay/config`, `just verify-ui`, decision D4 applied). Commits, in order:

1. `8984501` — the toolchain move alone: `tailwindcss` 3.4.17 → 4.3.3,
   `daisyui` 4.12.23 → 5.7.28, `autoprefixer` deleted in favour of
   `@tailwindcss/postcss`; `tailwind.config.ts` deleted; `postcss.config.js`
   rewritten; `next.config.ts` gains a `webpack.resolve.extensionAlias`
   entry (see "Where the plan was wrong", below). `pnpm-lock.yaml` updated
   by a real `pnpm install`, not hand-edited.
2. `ae42afd` — `app/layout.tsx` and `app/page.tsx` rewritten as pure
   `@vpay/ui` composition; `data-theme` `corporate` → `bumblebee`.
3. `a3ca1c3` — four component recipes (`src/recipes/`), this app's first
   jsdom test environment (`vitest.config.ts`, `vitest.setup.ts`), and
   `README.md`.
4. `1cae0ef` — the layout's own test (`src/layout.test.tsx`), pinning
   `data-theme="bumblebee"` and the brand `<h1>` inside the `<nav>`. Added
   after finding, by actually running plan §5 Lane D's own named mutation
   ("the layout's `data-theme` reverts to `corporate`"), that nothing in the
   suite up to that point caught it — see "For the reviewer" below.

Head at report time: run `git rev-parse HEAD` in the worktree — do not trust
a pasted SHA in this file over that command. At the time this was written it
was `1cae0efc6890c2f0616fbf4ae37627df4943de12`.

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

| recipe | result | evidence |
|---|---|---|
| `pnpm install` (real, dashboard's `package.json` changed twice) | ✅ | 0 peer-dependency errors, lockfile updated |
| `pnpm --filter @vpay/dashboard typecheck` (`tsc --noEmit`) | ✅ | clean, both before and after the `next.config.ts` webpack fix |
| `pnpm --filter @vpay/dashboard build` | ✅ | `next build` succeeds; **decisive check per plan §6.4**: `.next/static/css/*.css` inspected directly — `bumblebee`, `.alert-warning`, `.badge-success`, `.badge-error`, `.badge-warning`, `.badge-neutral`, `.badge-info` all present in the compiled stylesheet, `form-control`/`label-text` absent. A component rendering `class="badge badge-success"` against a stylesheet where that rule was never generated looks fine in jsdom and is unstyled in a browser — this is why the check reads the built CSS, not a render |
| `pnpm --filter @vpay/dashboard lint` | ✅ | clean (one `@typescript-eslint/no-unnecessary-type-assertion` in `vitest.setup.ts` found and fixed with `eslint --fix` before landing — see "Where the plan was wrong" below) |
| `pnpm --filter @vpay/dashboard test` | ✅ | 5 files, 9 tests — up from 0 (`--passWithNoTests`) before this lane. Run **five times in a row**, 9/9 green each time, 0 unhandled errors, following Lane A's own finding that a Base UI teardown race can surface roughly once in ten runs |
| `just lint-web` (whole workspace) | ✅ | clean across all 15 buildable packages |
| `just test-web` (whole workspace) | ✅ | `@vpay/checkout` 448/448 (unchanged), `@vpay/ui` 46/46 (unchanged), `@vpay-examples/shop` 96/96 (unchanged), `@vpay/dashboard` **9/9** (was 0), plus every SDK/tokens/config/api-client package unaffected — no Rust, no other frontend package touched by this lane |
| `just verify-ui` | 🔴 (expected, not a Lane D defect) | fails only on `frontends/apps/checkout/src/components/screens.tsx`'s `form-control`/`label-text` — Lane B's unmigrated tree, exactly as Lane A's own report recorded before this lane started. **Scoped to `frontends/apps/dashboard` alone, all four checks pass** — verified directly with the same four `git grep` patterns `verify-ui`'s recipe uses, restricted to this app's path, all four returning nothing |
| `just verify-links` | ✅ | 904 links, 162 files (up from Lane A's 894/160 — `README.md`'s own relative links, all resolving) |
| `just verify-npm-scope` | ✅ | unaffected — no package renamed or removed |
| `just verify-status` | ✅ | `docs/status.md`'s one declared `NotImplemented` item still matches shipping code |
| `just audit-web` | ✅ | "No known vulnerabilities found" on both `--prod` and the whole workspace, after adding `@testing-library/react`, `@testing-library/jest-dom`, `jsdom` as new dashboard devDependencies |
| `dashboard.cy.ts` (real Cypress run against `compose.e2e.yml`) | **not run** | see "What was NOT done" below |
| `just ci` | not run | the whole-revamp gate (plan §7), meant for the final merged head after all four lanes land |

## What was NOT done

- **`dashboard.cy.ts` was not run against a live stack.** `just test-e2e`
  needs `pnpm exec cypress install` on a network that can reach Cypress's
  CDN and a running `compose.e2e.yml` (real Next.js, real vpay-server, real
  Postgres). Every claim about it in this note is: the three assertions it
  makes (`h1` text "vpay dashboard", `[role="status"]` containing
  "Scaffold", `[data-status="…"]` exactly once for each of the five
  statuses) were checked by hand against the actual rendered markup — the
  `h1` is now in `app/layout.tsx`'s `<nav>` rather than `app/page.tsx`, and
  each still resolves — but the spec itself was not executed. Same gap
  Lane A and the plan's own §9 record for this whole revamp.
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
2. **`NextConfig['webpack']` is typed `any` in Next's own types**, so the
   naive fix (`webpack(config) { config.resolve.extensionAlias = …; }`)
   fails `@typescript-eslint/no-unsafe-member-access` /
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
  browser with no error anywhere. `src/layout.test.tsx` (commit `1cae0ef`)
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
