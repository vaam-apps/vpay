# Verification log — 2026-09-13, a Storybook for `frontends/apps/dashboard`

Last verified: 2026-09-13, on `d5e323d3`, Node `22.23.2` (the `.nvmrc` pin,
not the host's 24), pnpm `9.15.0`.

> **Reviewed 2026-09-13, adversarially, after the first draft of this page.**
> Three of this page's own claims were wrong and are corrected in place, each
> marked `[corrected on review]`; the review's own findings, fixes and
> mutations are in [What the review changed](#what-the-review-changed) at the
> end. The headline one: `dark` was **not** checked on all 25 stories. Three
> stories that mount `@vaam-apps/ui`'s `ThemeSwitcher` rendered in whatever
> theme the RUNNER's `prefers-color-scheme` said, overriding the decorator.

## Why this note exists

`frontends/apps/checkout` gained its own Storybook on 2026-09-12 (PR #135,
re-verified in
[2026-09-13-storybook-reverified.md](2026-09-13-storybook-reverified.md))
after the `@vaam-apps/ui` cutover deleted the shared `@vpay/ui` package that
used to host one. **The dashboard did not.** `docs/status/frontend.md` said
so in two places (rows about the instrument-register contrast and about
Storybook coverage). This closes that gap: `frontends/apps/dashboard/
.storybook/` now exists, with its own vitest browser config and its own
`a11y-gate.test.ts` locking the settings inside `just ci`.

## The decisive tests, with real numbers

| #   | Test                                                                        | Result                                                                                                                                                                                                                        |
| --- | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `just build-storybook` exit code; `storybook-static/index.json` story count | exit **0**; **25** entries of type `story`, plus **1** of type `docs` — `[corrected on review]`, this page first said 0: the stories file carries `tags: ["autodocs"]`, so `addon-docs` generates one docs entry for the meta |
| 2   | Theme lands in the built stylesheet                                         | `assets/iframe-*.css`: `var(--color-base-100)` referenced **22×**, `--color-base-100:` defined **3×**                                                                                                                         |
| 3   | `just test-storybook` exit code, story/skip/error counts                    | exit **0** — **25 passed**, 0 skipped, 0 unhandled errors, from a cold cache                                                                                                                                                  |
| 4   | Negative control: `color: #3a3a3a` on this theme's ground                   | axe **FAILS** it — "insufficient color contrast of **1.73** (foreground color: **#3a3a3a**, background color: **#0a0b0d**... Expected contrast ratio of 4.5:1)"; removed after confirming                                     |
| 5   | The `a11y-gate.test.ts` cases                                               | **six** after review (a fifth-and-sixth, see below); all pass, each under its own mutation — see the mutation table below                                                                                                     |

Exit codes were read from files the commands wrote themselves
(`; echo $? > file`), not from a harness banner.

Row 4 matters beyond "axe fires": `#0a0b0d` is this app's own shipped
background (`app/layout.tsx`'s `data-theme="dark"` resolving through
`@vaam-apps/ui`'s theme), so the probe proves the suite is measuring the
REAL ground rather than the browser's default white — the exact way PR #135's
predecessor failed silently on the checkout.

## Gate table

| Command                              | Result                                                                                                                                                                                                                                                  |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just verify`                        | exit **0**, twelve gates, after this page existed to satisfy `verify-links` (see below)                                                                                                                                                                 |
| `just fmt-check-web`                 | exit **0** after `prettier --write` on two files this change touched (`docs/status/frontend.md`, the stories file) — both had been hand-edited without running prettier first                                                                           |
| `pnpm --filter @vpay/dashboard test` | exit **0** — **31 files, 317 tests**. With a Storybook built: **317 passed, 0 skipped**. With none (CI's state, and `just ci`'s): **316 passed, 1 skipped** — the built-stylesheet case, which now says so instead of reporting a pass                  |
| `pnpm --filter @vpay/dashboard lint` | exit **0**                                                                                                                                                                                                                                              |
| `just build-storybook`               | exit **0**, from a cold cache (`storybook-static`, both apps' `node_modules/.vite*` and `node_modules/.cache` removed first); prints `checkout --color-base-100 defined 3x, referenced 22x` and `dashboard --color-base-100 defined 3x, referenced 22x` |
| `just test-storybook`                | exit **0** from a cold cache — checkout **22 passed**, dashboard **25 passed**, 0 skipped, 0 unhandled errors in either                                                                                                                                 |
| `just lint-web` (typecheck + eslint) | exit **0** — run on review because `pnpm --filter @vpay/dashboard lint` is eslint only and does not typecheck; the review's own first draft of `a11y-gate.test.ts` failed `tsc --noEmit` and eslint did not notice                                      |

`just verify`'s own first run in this task failed exactly once, on
`verify-links`, because this very file did not exist yet when
`docs/flows/dashboard/status-built-and-not-built.md` was written to link to
it — an honest failure from writing the pointer before its target, not a
defect in the gate. Writing this file and re-running fixed it. `just ci`
itself was intentionally NOT run (it builds the whole Rust workspace, and
other agents were building concurrently on this host) — the brief for this
task asked for the seven gates above instead.

## The five traps named in the brief, and what actually happened

1. **`@import` ordering.** `app/globals.css` already had the `@import`s
   first; not touched. `a11y-gate.test.ts` case 4 asserts the position and
   was mutated (swap the order) to confirm it fails.
2. **`css: true` in the vitest browser config.** Present from the first
   draft — copied from the checkout's config with the same reasoning.
   `[corrected on review]` — the reasoning given here first ("the theme
   measurement already depends on it end-to-end") was wrong, and row 2
   measures the **build**, which this setting does not touch at all. Actually
   deleting the line and re-running the suite with the `#3a3a3a` probe in a
   story: the probe still **FAILED at 1.73 against #0a0b0d**. On vitest
   4.1.11 browser mode the stylesheet is applied whatever `css` says. The
   line is kept for intent, and `vitest.storybook.config.ts`'s own comment
   now says it is not what guards the unstyled-but-green failure.
3. **`esbuild.jsx: "automatic"` in `main.ts`'s `viteFinal`, not in the
   vitest config.** Placed correctly the first time; without it, every
   story would die with `React is not defined`, and the two-pipeline
   divergence the checkout's comment describes (vitest green, build red) is
   exactly why it lives in the one file both consumers read.
4. **`optimizeDeps.include`.** Measured against this app's own component
   tree rather than copied: `react`, `react-dom`, `react-dom/client`,
   `react/jsx-runtime`, `react/jsx-dev-runtime`, `@vaam-apps/ui`,
   `@vpay/tokens`, `lucide-react`. **`@refinedev/core`, `@tanstack/
react-query` and `@vpay/api-client` — named "at minimum" in the brief —
   are NOT reached by this story set**: `src/dash/resources.ts` imports
   `ResourceProps` from `@refinedev/core` with `import type` only (erased at
   build time), and nothing this suite stories imports the other two at
   runtime. `vitest.storybook.config.ts`'s own doc comment states this and
   says to re-measure if a future story changes that.
5. **No `setupFiles` calling `setProjectAnnotations`.** Not added, for the
   reason the checkout's config states: `@storybook/addon-vitest` applies
   `preview.ts` itself since Storybook 10.3, and a manual call suppresses
   the automatic one.

**A sixth trap this app hit that the checkout's never did:** `next/link`.
See "Two new problems" below.

## Two new problems the checkout's Storybook never had to solve

**The router.** `PaymentsFilters` calls `useRouter()` and `AppShell` calls
`usePathname()`; outside a mounted Next app router both throw `invariant
expected app router to be mounted` the instant they render, and this app is
on `@storybook/react-vite`, not `@storybook/nextjs-vite`. Chosen: a vite
alias for the bare specifier `next/navigation` to a hand-written stub
(`.storybook/next-navigation-mock.ts`) fixing `usePathname()` at `/payments`
and `useRouter().push` as a no-op — not a routing simulator, and its own doc
comment says so. Considered and rejected: an `AppRouterContext.Provider`
decorator, because the context that would satisfy these hooks lives at an
unexported `next/dist/...` path with no contract to hold across a Next
upgrade.

**`next/link` and a real browser's `process`.** `PaymentsTable` and
`PaymentsPager` render `next/link` anchors — the first `next/link` usage in
either app's Storybook. Measured: with no fix, `pnpm --filter @vpay/dashboard
test-storybook` fails the WHOLE suite at import time with `ReferenceError:
process is not defined` inside `next/dist/client/has-base-path.js`
(`process.env.__NEXT_ROUTER_BASEPATH`). Next's own webpack build replaces
every `process.env.__NEXT_*` flag via `DefinePlugin`; nothing does that for
Vite. `build-storybook` does NOT catch this — it bundles the reference
without executing it, and only a real browser loading the module (what
`test-storybook` does) throws. Fixed with two `define` entries in
`viteFinal` matching this app's own `next.config.ts` (no `basePath` set).

## The theme.css alias: measured, and NOT carried over

The checkout's `.storybook/main.ts` hand-aliases
`@vaam-apps/ui/styles/theme.css` because, per its own doc comment, PR #135
found the built stylesheet defined `--color-base-100` **0** times without it.
The brief asked this app's config to carry that alias over "if — and only
if — you measure that vite needs it here too."

**Measured: it does not.** With the alias absent from a copy of this app's
`main.ts` and a cold cache (`rm -rf storybook-static node_modules/.cache`),
`pnpm run build-storybook`'s output still defined `--color-base-100` **3**
times and referenced it **22** times — identical to the build with the alias
present. Both apps share the same `postcss.config.js` plugin list
(`@tailwindcss/postcss`), so this was surprising enough to re-run the
identical method against the checkout's own tree too: removing **its** alias
and rebuilding also still defined the variable 3 times. As measured today, on
this dependency tree, **neither app's build currently reproduces the PR #135
failure with the alias absent** — which may mean a `vite`/`@tailwindcss/
postcss`/`@vaam-apps/ui` version between then and now already fixed the
underlying resolution, or that some other variable in the original repro no
longer holds. This app's `.storybook/main.ts` carries no alias, because one
that changes nothing is dead code, not insurance. `src/a11y-gate.test.ts`'s
built-stylesheet case is what would catch a real regression regardless of
which explanation is right.

**Not fixed here:** the checkout's own comment and `docs/status/frontend.md`
row 51's evidence still describe the alias as necessary. Re-verifying or
correcting that claim in the checkout's own files is outside this task's
scope (a different app), so it is named here rather than edited there.

## A real, third-party accessibility defect, found and NOT hidden

`AppShell`'s `Shell`/`Shell Light` stories fail axe's `landmark-unique` rule
at this suite's actual browser viewport (1200×900 — measured with a
temporary `console.log`, since neither vitest nor this config states one,
and below Tailwind's `xl` breakpoint of 1280px): two elements answer
`nav[aria-label="Primary"]` at once.

`@vaam-apps/ui@0.1.2`'s own `side-nav.js` doc comment asserts "Exactly one
`<nav aria-label="Primary">` exists at any width, in either mode." Measured
directly, not inferred from the failure message
(`axe.run(document, { runOnly: ["landmark-unique"] })` against the built
story, via a throwaway Playwright script): **zero** violations at 1280×800
(above `xl`), **one** at 1200×900 (below it). The in-flow sidebar's
className is `xl:flex xl:h-full xl:shrink-0 … xl:w-64` — every utility
`xl:`-prefixed, none unprefixed — so below `xl` none of them apply and the
element falls back to a `<nav>`'s browser-default `display: block` (visible,
not absent) at the same moment the 1024–1279px floating vertical rail
(`hidden sm:flex xl:hidden`) is also `display: flex`.

This is a defect in a published dependency this repository does not own the
source of, not a gap in this app's own markup: the className comes from
`@vaam-apps/ui@0.1.2`'s `dist/components/primitives/side-nav.js:456`
(`collapsed ? "hidden" : "xl:flex xl:h-full … xl:py-4"`, no unprefixed
`hidden`), and `app-shell.tsx` passes `smallScreen`'s default and writes
none of those classes. It is a real defect the shipped app has at every width
below 1280px, not a Storybook artefact.

`[corrected on review]` — the first draft suppressed it with
`parameters.a11y = { test: "todo" }`, which turns the addon off for the whole
story. Measured on review: a `#3a3a3a`-on-`#0a0b0d` probe placed inside
`Shell` **PASSED** under that setting, so `AppShell` — the chrome around
every screen in the app — had no colour-contrast verdict at all. The two
stories now disable the single rule `landmark-unique` and keep
`test: "error"` in force; the same probe then **FAILS at 1.73 against
#0a0b0d**. `src/a11y-gate.test.ts` case 5 pins which stories may carry a
suppression and which rule id, so a third cannot be added silently and
neither of these two can be widened back to `test: "todo"`.

## Both themes: `dark` is genuinely axe-checked; `light` only partially

`@vaam-apps/ui` 0.1.2 registers `dark` as default and `light` as opt-in;
`app/layout.tsx` pins `data-theme="dark"`. `.storybook/preview.ts` adds a
toolbar **globalType** (`theme`) defaulting to `dark`, with a decorator
writing `data-theme` from it — a human can flip it in the Storybook UI.

`just test-storybook` runs each story exactly once, under whatever
`globals.theme` resolves to at load. **So `dark` is checked by 21 of the 25
stories; `light` is checked by the four that explicitly set
`globals: { theme: "light" }`** — `FiltersLight`, `SignedInLight`,
`DetailWithChargeLight`, `ShellLight`, one per screen family that has an
obvious light-readable variant, not every story. `[corrected on review]`:
this page first said "seven", which contradicted its own list of four and its
own closing section.

**And until the review, none of that was reliably true.**
`@vaam-apps/ui`'s `ThemeSwitcher` owns `data-theme` at runtime — a
module-level store reads `localStorage["vaam-ui:theme"]`, defaults to
`"system"`, resolves that through `matchMedia("(prefers-color-scheme:
dark)")`, and OVERWRITES `document.documentElement`'s `data-theme` on mount.
`AppShell` mounts one in `accountSlot`; `MoreMenu`'s open drawer mounts a
second. Measured on the built Storybook, one fresh browser context per story
at 1200×900:

| story          | before the fix (`prefers-color-scheme: light`) | before the fix (`: dark`) | after the fix, either |
| -------------- | ---------------------------------------------- | ------------------------- | --------------------- |
| `Shell`        | `light`, body `#fcfcfd`                        | `dark`                    | `dark`, `#0a0b0d`     |
| `ShellLight`   | `light`                                        | **`dark`**                | `light`, `#fcfcfd`    |
| `MoreMenuOpen` | `light`                                        | `dark`                    | `dark`, `#0a0b0d`     |
| the other 22   | as declared                                    | as declared               | as declared           |

So three stories' theme was decided by the runner's colour scheme rather than
by the story: `Shell` and `MoreMenuOpen` were never checked against the
shipped dark ground on this suite's own browser, and `ShellLight` was not a
`light` data point. `preview.ts`'s decorator now writes the package's own
storage key and dispatches a `StorageEvent` before setting the attribute (the
store latches after its first read and refreshes only on that event, which
the DOM never fires in the tab that wrote it — and all 25 stories run in ONE
page). Re-measured after the fix: all 25 stories render the theme they
declare under both `prefers-color-scheme` values.

Stated plainly rather than implied: most of this app's `light` theme surface
is reviewable by a human flipping the toolbar control, not covered by the
automated gate.

## CI wiring: the path filter already covers the dashboard, no fix needed

The brief flagged that `d568f689` made CI path-filtered and asked to check
whether the `web` job's filter covers `frontends/apps/dashboard/**`.
**Measured: it does, unchanged.** `.github/workflows/ci.yml`'s `web` filter
includes the `web-sources` anchor, which lists `'frontends/**'` — a glob that
matches every path under `frontends/apps/dashboard/`, `.storybook/` included.
No workflow edit was needed or made.

## The mutation table (`src/a11y-gate.test.ts`)

Each case was broken, observed to fail, then restored — not asserted from
reading the assertion.

Every row below was re-run independently during the review, against the
committed files, with the exit code read from the run — not carried over.

| Case                                                                  | Mutation                                                                        | Result before restore |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------- | --------------------- |
| 1. `preview.ts` still fails on a violation                            | `test: "error"` → `test: "todo"`                                                | FAILS (exit 1)        |
| 2. `preview.ts` paints the real document shell                        | dropped `"bg-base-100"` from the decorator's `classList.add` call               | FAILS (exit 1)        |
| 2b. (same case, other half)                                           | dropped `import "../app/globals.css"`                                           | FAILS (exit 1)        |
| 3. `main.ts` loads the addons and the router alias                    | removed `"@storybook/addon-a11y"` from the addons array                         | FAILS (exit 1)        |
| 3b. (same case)                                                       | removed `"@storybook/addon-vitest"`                                             | FAILS (exit 1)        |
| 3c. (same case, other half)                                           | removed the `next/navigation` alias entry                                       | FAILS (exit 1)        |
| 4. `globals.css` imports the theme first                              | swapped `@import "@vaam-apps/ui/styles/theme.css"` to after `@plugin "daisyui"` | FAILS (exit 1)        |
| 5. Story-level a11y suppressions are the pinned set (added on review) | gave a third story a `parameters` a11y override                                 | FAILS (exit 1)        |
| 5b. (same case)                                                       | widened the two `AppShell` stories back to `test: "todo"`                       | FAILS (exit 1)        |
| 5c. (same case)                                                       | changed the suppressed rule id to `color-contrast`                              | FAILS (exit 1)        |
| 6. Built stylesheet defines the theme                                 | rewrote the built `--color-base-100:` declarations out of the stylesheet        | FAILS (exit 1)        |

**Two mutations that did NOT fail, and what was done about each:**

- **Removing `"@storybook/addon-docs"` from the addons array** leaves the
  gate green (`6 passed`). Left as is: `addon-docs` generates the docs page,
  not the a11y verdict, and case 3 deliberately pins only the two addons that
  make the suite a gate. Named here so it is a choice on the record rather
  than an oversight.
- **Case 6 with no `storybook-static/` present** reported **`passed`** in the
  first draft, because the body did a bare `return`. CI's `web` job runs
  `pnpm -r test` BEFORE `just build-storybook`, so that is CI's every run:
  the one gate standing behind the removed theme alias was green-and-blind
  there. It now calls `ctx.skip()` (measured: `5 passed | 1 skipped (6)`), and
  the assertion that actually runs against a real artefact moved into `just
build-storybook` itself — see below.

## What this note does not claim

- It does not claim `@vaam-apps/ui`'s `SideNav` landmark defect is fixed. It
  is not; it is named, reproduced at four viewports, traced to the exact
  upstream line, and suppressed by rule id on exactly the two stories it
  affects — with the suppression itself pinned by a gate.
- It does not claim `light` theme coverage is complete. Four of 25 stories
  carry it; the rest are `dark`-only in the automated gate, stated above.
- ~~It does not claim the `landmark-unique` defect has been reported
  upstream.~~ **Filed 2026-09-13 as vaam-apps/ui#16.** Re-measured across
  the full width range for the report, in a real Chromium, one fresh
  context per width: two visible `nav[aria-label="Primary"]` at 640,
  1023, 1200 and 1279; one at 1280 and 1440. **The defect is wider than
  this page first recorded** — it is every width below `xl`, not the
  1200-1279 band, and the sidebar paints as a visible 16-40px sliver
  rather than being merely present in the accessibility tree. The
  suppression here is unchanged in scope (one rule id, two stories).
  It has not; that is a maintainer action, not one taken here.
- It does not claim the checkout's own theme.css alias is now unnecessary in
  general — only that, measured today, removing it did not reproduce PR
  #135's failure on either app's current dependency tree. Whether that is a
  fixed upstream resolution or a changed variable in the original repro was
  not chased to a root cause.
- It did not run `just ci` (Rust workspace build; explicitly out of scope
  for this task on a host running other agents' builds concurrently).
- It does not add a Storybook route for any `app/**/page.tsx` — every page
  in this app is an async server component reading cookies and calling
  vpay, which neither jsdom nor a backend-less browser story can run; this
  mirrors `a11y.test.tsx`'s own choice to cover screens through the
  component rather than the page.

## Files touched

- `frontends/apps/dashboard/.storybook/{main.ts,preview.ts,next-navigation-mock.ts}`
- `frontends/apps/dashboard/vitest.storybook.config.ts`
- `frontends/apps/dashboard/src/components/dashboard-screens.stories.tsx`
- `frontends/apps/dashboard/src/a11y-gate.test.ts`
- `frontends/apps/dashboard/package.json` (storybook devDeps + scripts, versions matching the checkout's exactly)
- `frontends/apps/dashboard/eslint.config.js` (`outsideTsconfig` gains `.storybook/**`, measured the same dot-directory gap the checkout's carries)
- `pnpm-lock.yaml`
- `justfile` (`build-storybook`/`test-storybook` extended to both apps; `build-storybook` also asserts each app's built stylesheet DEFINES `--color-base-100`; `test-storybook`'s cache clear corrected; the `lint-web` comment block gains a dated paragraph)
- `docs/status/frontend.md` (two struck-through, dated corrections)
- `docs/status/README.md` (this page listed at the top of the verification log)
- `docs/flows/dashboard/status-built-and-not-built.md` (a new dated block; `docs/flows/dashboard.md`'s own Status section is a summary and was left untouched, per that page's own convention)

## What the review changed

An adversarial review ran on 2026-09-13 against commit `17d15a0e` with one
question: can this gate go RED for the right reason, or does it pass while
measuring nothing? Everything above was re-measured rather than read. Four
things were wrong enough to fix.

1. **Three stories rendered in the wrong theme** — `@vaam-apps/ui`'s
   `ThemeSwitcher` overrode the decorator from `prefers-color-scheme`. Fixed
   in `.storybook/preview.ts`; measured before and after, both colour
   schemes, all 25 stories. See the themes section above.
2. **`AppShell` had no axe coverage at all** — `test: "todo"` switches the
   addon off entirely, not just the one failing rule. Narrowed to
   `landmark-unique`; the `#3a3a3a` probe inside `Shell` now fails at 1.73
   against `#0a0b0d` where it used to pass at 11.09 against white.
3. **Nothing gated a story-level suppression**, in either app. Added as
   `a11y-gate.test.ts` case 5, pinning both the set of stories and the set of
   rule ids, verified under three separate mutations.
4. **The built-stylesheet gate never ran in CI** — a bare `return` reported a
   pass when there was no build, and CI's ordering guarantees there is none.
   Now an explicit skip, with the real assertion moved into `just
build-storybook`, where the artefact exists. Verified by mutation: with
   `app/globals.css`'s `@import` moved below `@plugin`, the recipe exits 1
   and prints `dashboard --color-base-100 defined 0x, referenced 22x` — PR
   #135's exact numbers.

Two claims in the first draft's own comments were also corrected rather than
left standing: `css: true` is not what makes the stories styled on this stack
(trap 2 above), and `just test-storybook`'s `rm -rf
node_modules/.cache/storybook` cleared nothing — that path does not exist in
this workspace; the per-app vite dep caches at
`frontends/apps/<app>/node_modules/.vite` are now named explicitly.

Independently confirmed, not changed:

- The negative control fails at **1.73** against **#0a0b0d**, the real ground.
- The theme lands in **both** apps' built stylesheets with **no** theme.css
  resolve alias, from a genuinely cold cache (`storybook-static`,
  `node_modules/.vite` and `node_modules/.cache` all removed first): defined
  **3×**, referenced **22×**, dashboard and checkout alike. The implementer's
  cross-check against the checkout's own tree reproduces.
- `optimizeDeps.include`'s narrowing is sound: `just test-storybook` from a
  cold dep cache reports **25 passed, 0 skipped, 0 unhandled errors**, and
  the checkout's 22 are unaffected.
- Removing `esbuild.jsx` from `viteFinal` leaves `build-storybook` at **exit
  0** while the built artefact renders "No Preview" with `ReferenceError:
React is not defined` on every story — confirmed by serving
  `storybook-static` and loading `iframe.html` in Playwright, not by reading
  the build log. The vitest suite does catch it: **25 failed**.
- `landmark-unique` on `AppShell` is genuinely upstream, and genuinely
  absent at and above 1280px.
- The CI `web` path filter covers `frontends/**`; no workflow edit needed.

Not proven, and stated as such:

- **Nothing pins the story count.** Deleting a story export leaves every gate
  green. The checkout has the same gap. Not added here: an exact count is
  churn on every legitimate story, and case 5's story-name pin already covers
  the specific case that matters (a story being quietly exempted rather than
  quietly deleted). A maintainer who wants deletion caught should say so.
- **The suite measures 1200×900 only**, below the `xl` breakpoint. Raising it
  above 1280 would make `landmark-unique` disappear — which is why it was NOT
  raised: 1200 is a width the app really ships at, and the defect there is
  real. A second viewport project would be the honest way to cover both
  bands; that is a scope decision left to the maintainer.
