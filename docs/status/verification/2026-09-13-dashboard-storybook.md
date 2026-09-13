# Verification log — 2026-09-13, a Storybook for `frontends/apps/dashboard`

Last verified: 2026-09-13, on `d5e323d3`, Node `22.23.2` (the `.nvmrc` pin,
not the host's 24), pnpm `9.15.0`.

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

| #   | Test                                                                        | Result                                                                                                                                                                                    |
| --- | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `just build-storybook` exit code; `storybook-static/index.json` story count | exit **0**; **25** entries of type `story` (0 of type `docs` — this config has no `.mdx` pages)                                                                                           |
| 2   | Theme lands in the built stylesheet                                         | `assets/iframe-*.css`: `var(--color-base-100)` referenced **22×**, `--color-base-100:` defined **3×**                                                                                     |
| 3   | `just test-storybook` exit code, story/skip/error counts                    | exit **0** — **25 passed**, 0 skipped, 0 unhandled errors, from a cold cache                                                                                                              |
| 4   | Negative control: `color: #3a3a3a` on this theme's ground                   | axe **FAILS** it — "insufficient color contrast of **1.73** (foreground color: **#3a3a3a**, background color: **#0a0b0d**... Expected contrast ratio of 4.5:1)"; removed after confirming |
| 5   | The five `a11y-gate.test.ts` cases                                          | all pass; see the mutation table below                                                                                                                                                    |

Exit codes were read from files the commands wrote themselves
(`; echo $? > file`), not from a harness banner.

Row 4 matters beyond "axe fires": `#0a0b0d` is this app's own shipped
background (`app/layout.tsx`'s `data-theme="dark"` resolving through
`@vaam-apps/ui`'s theme), so the probe proves the suite is measuring the
REAL ground rather than the browser's default white — the exact way PR #135's
predecessor failed silently on the checkout.

## Gate table

| Command                              | Result                                                                                                                                                                        |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just verify`                        | exit **0**, twelve gates, after this page existed to satisfy `verify-links` (see below)                                                                                       |
| `just fmt-check-web`                 | exit **0** after `prettier --write` on two files this change touched (`docs/status/frontend.md`, the stories file) — both had been hand-edited without running prettier first |
| `pnpm --filter @vpay/dashboard test` | exit **0** — **31 files, 316 tests, 316 passed, 0 skipped**                                                                                                                   |
| `pnpm --filter @vpay/dashboard lint` | exit **0**                                                                                                                                                                    |
| `just build-storybook`               | exit **0** (builds both apps; dashboard's own numbers are row 1 above)                                                                                                        |
| `just test-storybook`                | exit **0** (both apps; dashboard's own numbers are row 3 above; checkout unaffected, still 22 passed)                                                                         |

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
   Removing it was not separately re-tested here because the theme
   measurement (row 2 above) already depends on it end-to-end: a stub CSS
   import cannot produce a stylesheet with the variable defined at all.
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
source of, not a gap in this app's own markup. It is not silently
suppressed: `Shell`/`Shell Light` carry a documented `parameters.a11y = {
test: "todo" }` — the same escape hatch the checkout's `preview.ts` names —
with the measurement above in the story file's own comment, and every OTHER
story in the suite still fails on a real violation with no such override.

## Both themes: `dark` is genuinely axe-checked; `light` only partially

`@vaam-apps/ui` 0.1.2 registers `dark` as default and `light` as opt-in;
`app/layout.tsx` pins `data-theme="dark"`. `.storybook/preview.ts` adds a
toolbar **globalType** (`theme`) defaulting to `dark`, with a decorator
writing `data-theme` from it — a human can flip it in the Storybook UI.

`just test-storybook` runs each story exactly once, under whatever
`globals.theme` resolves to at load. **So `dark` is checked by every one of
the 25 stories; `light` is checked only by the seven stories that
explicitly set `globals: { theme: "light" }`** (`FiltersLight`,
`SignedInLight`, `DetailWithChargeLight`, `ShellLight` — one per screen
family that has an obvious light-readable variant, not every story). Stated
plainly rather than implied: most of this app's `light` theme surface is
reviewable by a human flipping the toolbar control, not covered by the
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

| Case                                               | Mutation                                                                        | Result before restore |
| -------------------------------------------------- | ------------------------------------------------------------------------------- | --------------------- |
| 1. `preview.ts` still fails on a violation         | `test: "error"` → `test: "todo"`                                                | 1 failed \| 4 passed  |
| 2. `preview.ts` paints the real document shell     | dropped `"bg-base-100"` from the decorator's `classList.add` call               | 1 failed \| 4 passed  |
| 3. `main.ts` loads the addons and the router alias | removed `"@storybook/addon-a11y"` from the addons array                         | 1 failed \| 4 passed  |
| 3b. (same case, other half)                        | removed the `next/navigation` alias entry                                       | 1 failed \| 4 passed  |
| 4. `globals.css` imports the theme first           | swapped `@import "@vaam-apps/ui/styles/theme.css"` to after `@plugin "daisyui"` | 1 failed \| 4 passed  |
| 5. Built stylesheet defines the theme              | wrote a fake `storybook-static/assets/*.css` with no `--color-base-100`         | 1 failed \| 4 passed  |

All five passed after each restore, `5 passed (5)` overall.

## What this note does not claim

- It does not claim `@vaam-apps/ui`'s `SideNav` landmark defect is fixed. It
  is not; it is named, reproduced with a decisive measurement, and left as a
  documented `test: "todo"` on exactly the two stories it affects.
- It does not claim `light` theme coverage is complete. Four of 25 stories
  carry it; the rest are `dark`-only in the automated gate, stated above.
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
- `justfile` (`build-storybook`/`test-storybook` extended to both apps; the `lint-web` comment block gains a dated paragraph)
- `docs/status/frontend.md` (two struck-through, dated corrections)
- `docs/status/README.md` (this page listed at the top of the verification log)
- `docs/flows/dashboard/status-built-and-not-built.md` (a new dated block; `docs/flows/dashboard.md`'s own Status section is a summary and was left untouched, per that page's own convention)
