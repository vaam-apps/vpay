# Verification log — 2026-09-12, Storybook restored in the checkout app

Last verified: 2026-09-12, on branch `claude/checkout-storybook-a11y`, off
`611c1701`. This closes the gap the `@vaam-apps/ui` cutover opened and named:
`@vpay/ui` was deleted with the checkout's Storybook install inside it, and
restoring it was a filed task
([2026-09-12.md](2026-09-12.md), [2026-09-12-browser-a11y.md](2026-09-12-browser-a11y.md)).

## What this note claims, and what it does not

It claims the 22 checkout stories exist again, that a real browser renders
them, that axe returns a **verdict** on them, and that all 22 clear WCAG AA on
`@vaam-apps/ui`'s theme. It also claims a defect was found in both apps'
stylesheets and fixed.

It does **not** claim anything about the served page: this renders stories,
not `compose.e2e.yml`. It does not retire `outcome-contrast.test.ts`'s note
that axe cannot walk the real page's DOM for a background. It does not cover
the dashboard, which has no equivalent gate. And it measures 22 states, not
`@vaam-apps/ui` — that package is published and cannot be patched here.

## The defect this found, which is the important part

**CSS drops an `@import` that follows another at-rule.** Both apps'
`app/globals.css` placed `@import "@vaam-apps/ui/styles/theme.css"` **after**
`@plugin "daisyui" { themes: false }`.

Tailwind's own parser is lenient about the ordering, so everything that
compiles the file through `@tailwindcss/postcss` directly inlined the theme
and passed — `next build`, and `src/styling-gate.test.ts`, the gate the
cutover added for exactly this class of failure. A spec-compliant pipeline did
not. Storybook's vite build emitted a stylesheet containing every
`var(--color-base-100)` and defining `--color-base-100` **nowhere**.

The result was not a crash. Every story rendered completely unstyled on the
browser's default white while the shipped page is `#0a0b0d`, and axe dutifully
measured every foreground against the wrong ground:

| Run                                                       | Result                              |
| --------------------------------------------------------- | ----------------------------------- |
| 22 stories, theme missing                                 | **22 passed**, 0 violations         |
| `#3a3a3a` probe (unreadable on this theme), theme missing | **passed**                          |
| `#3a3a3a` probe, theme present                            | **fails, 1.73:1 against `#0a0b0d`** |

Six consecutive runs passed that way. It was caught by measuring the rendered
background instead of trusting the green: `getComputedStyle(document.body)`
returned `rgba(0, 0, 0, 0)` and `--color-base-100` came back as the empty
string.

Moving one line: **136 360 → 145 903 bytes**, theme present, body background
`rgb(10, 11, 13)`. `frontends/apps/dashboard/app/globals.css` had the
identical ordering and is fixed in the same commit. Both apps'
`styling-gate.test.ts` still pass, so neither was broken by the reorder —
and neither could ever have caught this, because both use the lenient parser.

Three wrong diagnoses were tried and discarded on measurement before the real
one: a `resolve.alias` for the specifier (byte-identical output), the
package's physical `dist` path instead of its export (byte-identical), and
`test.css: true` in the vitest config (no effect). A marker rule appended to
`globals.css` is what proved the file was in the graph at all and narrowed it
to the `@import` alone.

## The gate

`just build-storybook` exit 0: **23 index entries** — 22 stories plus one
autodocs page — and a **145 903-byte** stylesheet with the theme defined.

`just test-storybook` exit 0 from a cold cache: **1 test file, 22 tests, 22
passed, 0 skipped, 0 unhandled errors.** `@storybook/addon-vitest` +
`@vitest/browser` 4.1.11 on Chromium, axe run by `addon-a11y`'s own
`afterEach`, so there is no second axe configuration to keep in step.

It is **not** in `just ci`: it needs a ~115 MB Playwright Chromium the first
time and `just ci` must pass offline, the same reason `helm-check` is
excluded. CI's `web` job runs the recipe, with `~/.cache/ms-playwright` cached.

The stories are the deleted ones, recovered from `72dbfac5^` and unchanged
except for their doc comment and one import (`@storybook/react` →
`@storybook/react-vite`) — `CheckoutView`'s and `ReturnView`'s props did not
change in the cutover, so this is a restoration rather than a rewrite.

## A second silent failure, found by opening the artefact

**`just build-storybook` exited 0 and produced a Storybook in which every
single story rendered the red "React is not defined" panel.**

The cause is the same JSX-runtime trap as the vitest suite's: this app's
`tsconfig.json` says `"jsx": "preserve"` because Next requires it, Vite's
esbuild reads the nearest tsconfig, and the stories import no React. The fix
had been applied to `vitest.storybook.config.ts` alone — so
`just test-storybook` passed all 22 stories while the **other** consumer of
the same stories, the one a human actually opens, rendered none of them.
Compiling is not rendering, and the build exits 0 either way.

Nothing measured it. The browser suite was green, `build-storybook` was
green, `index.json` had its 23 entries and the stylesheet had its theme. It
was found by serving `storybook-static/` and looking at a screen.

The setting lives in `.storybook/main.ts`'s `viteFinal` now, which
`build-storybook` and `addon-vitest` both read through `configDir`, and the
duplicate in the vitest config is deleted. One setting, one place, and the
suite is the guard: deleting it from `main.ts` fails **all 22 tests** with
`React is not defined` (measured, reverted), where before it failed none.

## What keeps it honest

`frontends/apps/checkout/src/a11y-gate.test.ts` — jsdom, so it runs in
`just test-web` and therefore in `just ci`, which never runs the browser
suite. Five cases, each verified against the mutation it exists to catch:

| Mutation                                         | Result   |
| ------------------------------------------------ | -------- |
| theme `@import` moved back after `@plugin`       | 1 failed |
| `bg-base-100` dropped from the preview shell     | 1 failed |
| `test: "error"` → `"todo"`                       | 1 failed |
| `@storybook/addon-vitest` removed from `main.ts` | 1 failed |

The fifth case reads the **built** stylesheet and fails if `--color-base-100`
is generated but never defined. It skips rather than fails when no build is
present, because `just ci` deliberately does not build Storybook and a test
that demanded one would make the standard gate depend on a step that is
deliberately outside it.

Its first draft failed on its own documentation — the prose explaining the
defect names `@plugin`, and an `indexOf` found it there. It strips comments
first now, the same distinction `verify-status` draws by lexing.

## Carried over from PR #133, and it paid

Two findings from the superseded branch were written into `justfile`'s
`build-storybook` slot before this work started, and both were needed:
`@storybook/test-runner` does not compose with Storybook 10 (so
`@storybook/addon-vitest` was chosen directly), and every dependency must be
declared in `optimizeDeps.include` or the suite reports stories passing while
they throw during render. The cache-clearing recipe came from the same place.
