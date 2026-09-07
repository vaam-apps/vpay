# exp26 Lane B notes — `frontends/apps/checkout` on `@vpay/ui`

Branch `claude/exp26-ui-lane-b`, base `177645e` (Lane A's first head), rebased
onto `08d9b8e` (Lane A's reviewed head) mid-task after the coordinator
reported the review had landed — see "The rebase" below.

Run `git rev-parse HEAD` in the worktree for the actual head; do not trust a
pasted SHA in this file over that command.

## What this lane did (plan §4.1, §5 Lane B, §6.5, §6.6, §8.3, §8.5)

1. `frontends/apps/checkout/package.json`: `@base-ui-components/react`
   deleted; `@vpay/ui` added (`workspace:*`); `daisyui` `^4.12.23` →
   `5.7.28`; `tailwindcss` `^3.4.17` → `4.3.3`; `@tailwindcss/postcss` added;
   `autoprefixer` deleted.
2. `tailwind.config.ts` deleted; `postcss.config.js` →
   `{ plugins: { '@tailwindcss/postcss': {} } }`; `app/globals.css` → one
   `@import '@vpay/ui/styles.css';` plus the unchanged
   `prefers-reduced-motion` block (the app's only remaining hand-written CSS,
   and still the gate's one `!important` exemption for this app).
3. `next.config.ts` gained `transpilePackages: ['@vpay/tokens', '@vpay/ui']`.
4. `screens.tsx`, `checkout-view.tsx`, `return-view.tsx`, `locale-switch.tsx`
   rewritten against `@vpay/ui`'s `Alert`, `Badge`, `Button`, `Card`/
   `CardBody`, `Checkbox`, `Field`/`FieldLabel`/`FieldDescription`/
   `FieldError`, `Heading`, `Input`, `List`, `PageShell`, `Select`,
   `Spinner`, `Stack`, `Text`. Zero `cva`, zero daisyUI class literal
   anywhere in this app now (`just verify-ui` check 4 and check 2 both
   confirm this on the built tree).
5. `checkout-screens.stories.tsx`: one stale comment corrected (it named
   `tailwind.config.ts`, which no longer exists, as the reason Storybook's
   theme "only approximately" matched this app's; since both now import the
   same `@vpay/ui/styles.css`, the comment's premise was gone).
6. `src/config/theme.ts` collapsed 176 → 97 lines per plan §8.5.
   `theme.test.ts` rewritten: the daisyUI-4-shaped comparison test (20
   cases, one `for` loop of six colours plus one of eight bad values) is
   replaced by 31 cases — one `it` per colour and per refused value rather
   than a loop inside a single `it`, so the suite grew rather than shrank,
   plus a new independent WCAG-contrast check (see "Two things Lane B must
   prove" below).
7. `layout.test.tsx`: the one assertion pinned to daisyUI 4's OKLCh string
   (`--p:84.2251% 0.165456 91.330667`) updated to daisyUI 5's literal-colour
   form (`--color-primary:#f3c623;`). Nothing else in that file changed —
   the hydration-regression assertions (no explicit `<head>`, `href`/
   `precedence` hoisting) are untouched.
8. `vitest.setup.ts`: gained the same jsdom polyfills (`PointerEvent`,
   `hasPointerCapture`/`setPointerCapture`/`releasePointerCapture`,
   `ResizeObserver`) `@vpay/ui`'s own suite carries, guarded behind
   `typeof window !== 'undefined'` — this app's *default* test environment
   is `node` (most of its suite talks to `node:http`), with individual
   files opting into `jsdom`, so the unguarded class-declaration-time
   `extends MouseEvent` in `@vpay/ui`'s copy would throw in every
   `node`-environment file. Caught by running the full suite, not assumed.
9. `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts` and
   `shop-embedded.cy.ts`: the one `button.btn-primary` line in each,
   changed to `[data-testid="continue"]`, with the `data-testid="continue"`
   added to `RedirectPrompt`'s button in `screens.tsx`. Nothing else in
   either spec touched.
10. `frontends/apps/checkout/eslint.config.js`: `tailwind: true` added
    (Lane A review finding 4) — required in the same commit as the Tailwind
    4 migration, since `eslint-plugin-better-tailwindcss` aborts outright
    against a Tailwind-3 package.

## Before / after (`exp26-plan-count.sh`), `@vpay/checkout` only

```
before   styling_files=5  classname_sites=59  class_tokens_distinct=71  class_tokens_total=161  css_lines=13
after    styling_files=2  classname_sites=8   class_tokens_distinct=9   class_tokens_total=10   css_lines=11
target   styling_files≤1  (plan §5 and this lane's brief agree)         class_tokens_distinct≤14
```

**`class_tokens_distinct` target met (9 ≤ 14). `styling_files` target missed
by one (2, not ≤1).** The two remaining files and exactly what each keeps:

- `app/layout.tsx` — `bg-base-100 min-h-screen` on `<body>`, which plan
  §4.1's own per-screen table lists as "unchanged (2 tokens, app chrome)".
  Not touched by this lane on purpose.
- `screens.tsx` keeps five tokens, none a daisyUI component class and none a
  colour: `h-8 w-auto` on `BrandHeader`'s `<img>` (overrides Tailwind's own
  `img{height:auto}` preflight reset — without it the logo has no fixed
  height at all); `text-3xl tabular-nums` on the payment amount (`Text`'s
  largest `size` variant is `text-lg`, and the amount is the single most
  important number on a payment page — this lane chose to keep it legible
  over hitting the metric exactly); `break-all` on the session reference (a
  long id would otherwise overflow its card); `sr-only` on two visually-
  hidden labels; `cursor-pointer` on the memory opt-in's `<label>` (no
  `@vpay/ui` primitive wraps a checkbox-plus-label the way this control
  needs).

Reported plainly rather than reworded to look like a pass: the target reads
"≤1" and this lane delivers 2. Every plan-migration-table entry that names a
"stays" exception (`h-8 w-auto`, layout.tsx's app chrome) already implies at
least these two files would remain styling files; the plan's own numeric
target and its own per-screen table are in mild tension, and this lane chose
to honour the table's named exceptions rather than delete functional CSS to
chase the count.

Repo-wide (all four lanes, only B landed): `styling_files` 18 → 15,
`class_tokens_distinct` (union) 92 → 40, `classname_sites` 107 → 56,
`class_tokens_total` 224 → 73, `css_lines` 245 → 243, `inline_styles` 24 →
24 (shop's, Lane C's job, unchanged). Shop and dashboard are still
untouched, so the 80% repo-wide targets are not expected to be met until
all four lanes land.

## Gates, recipe by recipe

| recipe | result | evidence |
|---|---|---|
| `pnpm install` | ✅ | lockfile up to date after `package.json` changes |
| `pnpm --filter @vpay/checkout typecheck` | ✅ | clean |
| `pnpm --filter @vpay/checkout lint` | ✅ | clean, including the six `eslint-plugin-better-tailwindcss` rules `tailwind: true` turns on |
| `pnpm --filter @vpay/checkout test` | ✅ | **459/459, 23 files, 0 skipped** (was 448) |
| `pnpm --filter @vpay/checkout build` | ✅ | a real `next build` — Lane A review finding 3. Compiled `.next/static/css/*.css` read directly: `.btn-primary` (1), `.badge-ghost` (1), `.alert-error` (1), `.checkbox` (35), `.card{` (1), `.select{` (8), `.fieldset` (17), `--color-primary` (8) all present |
| `just verify-ui` | ✅ | all four checks, including the stricter colour/`className=`-scoping check Lane A's review added |
| `just verify` | ✅ | all eleven gates, on the rebased tree; nothing under `backends/`/`schemas/` touched (`git status` confirmed empty there before running) |
| `just test-web` | ✅ | `@vpay/checkout` 459/459; every other package's count unaffected |
| `just lint-web` | 🔴, **pre-existing, out of scope** | `pnpm -r typecheck` fails on `frontends/apps/dashboard/tailwind.config.ts` (Tailwind 3's `PluginAPI` type against the now-workspace-wide `tailwindcss@4.3.3`). Reproduced identically on Lane A's unmodified reviewed head (`08d9b8e`) with zero Lane B changes applied — this lane did not cause it and cannot fix it without editing `frontends/apps/dashboard`, which is out of scope. Lane D's job (`tailwind.config.ts` deletion, plan §4.2) |
| `just ci` | not run | the whole-revamp gate (plan §7), meant for the final merged head, same reasoning Lane A gave |
| `just test-e2e` | 🔴 as written, 🟢 for the three specs this lane owns | see below |

### `just test-e2e`

The recipe itself could not complete: `docker compose`'s `dashboard` image
build fails for the identical `.js`-suffixed-import reason `@vpay/checkout`'s
did before the Lane A review fix (`e2a0a09`) — `frontends/apps/dashboard`'s
own `next.config.ts` has no `@vpay/ui` in `transpilePackages`, and its
scaffold `app/page.tsx` already imports `@vpay/ui`'s `StatusBadge` from the
original two-component set. Out of scope (`frontends/apps/dashboard/**` is
Lane D's), not attempted.

Brought the other seven `compose.e2e.yml`/`compose.demo.yml` services up by
name instead (`demo_project=exp26b`, ports 23001/23080/28080/28082/28083 —
confirmed free with `docker ps`/`ss -ltn` before use, so as not to collide
with the `exp26d`/default-`vpay-demo` stacks already running on this shared
host), then ran the three specs that do not need `dashboard`:

```
checkout.cy.ts        1/1
shop-hosted.cy.ts      3/3
shop-embedded.cy.ts    4/4  (VPAY_E2E_FRAMED=1 — cypress.config.ts excludes it by default)
——————————————————————————
                       8/8, 0 failing, 0 skipped
```

`dashboard.cy.ts` (3 tests) did not run. Stack torn down
(`docker compose down -v`) after.

**One operator mistake caught and fixed in the process, recorded so it is
not repeated**: the first `gen-demo-keys` run for this stack used the
default `demo_port=8080` rather than the actual `28080` in use, which wrote
`.e2e/application-demo.yml`'s OAuth audience for the wrong port and made
`checkout.cy.ts` fail with `invalid_client: Client authentication failed` /
`InvalidAudience`. Re-running `gen-demo-keys` with the matching
`demo_port=28080` (and restarting the affected containers, since the config
is bind-mounted and re-read on start, not baked into the image) fixed it.
`wiremock-orange`'s Docker healthcheck reported `unhealthy` throughout
("OCI runtime exec failed... possible container breakout detected" — the
documented rootless-Docker shim fault) while the container kept answering
real requests correctly per its own logs; a `docker restart` of that one
container (not the daemon) was enough for the Orange-redirect tests to pass
without retry.

### Decisive mutations run and confirmed (staged, tested, reverted)

| mutation | result |
|---|---|
| `OutcomePanel` hard-codes `tone="neutral"` instead of `checkoutOutcomeTone[kind]` | **fails** — `checkout-view.test.tsx`: `AssertionError: a failure must carry a tone: expected 'mt-4 alert' to contain 'alert-error'` |
| `MsisdnForm`'s submit button loses `type="submit"` | **fails** — `checkout-view.test.tsx`'s "leaves the MSISDN form's only submit button the submit button": `expected +0 to be 1` |
| `theme.ts`'s `themeStyleSheet` skips the `linearRgb` null-check (an unvalidated colour reaches the output) | **fails** — 10 of `theme.test.ts`'s cases, including both XSS strings (`javascript:alert(1)`, `</style><script>`) |

Not re-run here (Lane A's own decisive-mutation table covers them, and
nothing in this lane touches the mechanism): `ScreenHeading` dropping
`tabIndex={-1}` — unchanged from the original file, still asserted by
"moves focus to the new screen's heading". `data-testid="continue"` removed
— its own decisive check is `just test-e2e` itself, which is the 8/8 above.

## Two things §8.5 asked Lane B to prove

1. **Does `color-mix` hold contrast?** Yes, measured. `theme.test.ts`
   re-implements `color-mix(in oklch, colour 20%, white|black)`
   **independently** (Björn Ottosson's OKLab matrices, not imported from
   `theme.ts`) and computes the WCAG contrast ratio between each of six
   colours and its own foreground: `#f3c623` 11.79:1, `#ff0000` 5.08:1,
   `#ffffff` 18.10:1, `#000000` 11.24:1, `#1d4ed8` 4.93:1, `#00a651`
   6.29:1 — all six clear AA's 4.5:1, the closest being `#1d4ed8` at 4.93.
   The `color-mix` approach was kept; the OKLCh-literal fallback §8.5 named
   was not needed.
2. **Does the emitted string still carry nothing but a validated colour?**
   Yes — see the decisive mutation above.

## What was NOT done

- **`dashboard.cy.ts`** — blocked by the out-of-scope dashboard build defect
  above.
- **`cypress-axe` / real-browser contrast** (plan §7 row 6) — not built by
  this lane. Lane A's own report already named it unbuilt by anyone; this
  lane did not change that.
- **`frontends/apps/dashboard` and `examples/shop`** — untouched, as scoped.
- **`frontends/tests/e2e/cypress/e2e/dashboard.cy.ts`** as a file — untouched
  (it is Lane D's, per plan §5's "Owns" list); nothing in it references
  `@vpay/checkout`.
- **The `styling_files ≤1` target** — missed by one file, reasoned above,
  not silently dropped.

## Where the plan turned out imprecise, for the next reader

1. **§4.1's per-screen table implies more raw-utility survival than the
   `styling_files ≤1` target admits.** Read literally, the "deleted" column
   for several rows (`RailSelector`'s `mt-4 flex flex-col gap-2` container,
   `MemoryOptIn`'s wrapping `<label>`) does not list the container's own
   layout classes as deleted — only the daisyUI component classes inside it.
   Taken at face value this would leave far more than 9 tokens in
   `screens.tsx`. This lane resolved the tension by using `@vpay/ui`'s
   `Stack` for every generic flex/gap container it plausibly could (the
   whole reason Lane A built `Stack` ahead of being asked, per Lane A's own
   notes finding 4), and kept only the five raw-utility exceptions §4.1
   explicitly names or that have no `@vpay/ui` primitive at all. The
   resulting `styling_files` count (2) still misses the stated target by
   one; see "Before / after" above for exactly which two files and why.
2. **§6.6's decisive check needs a screen with a `btn-primary`, and
   `select_rail` (an obvious first choice) has none.** Every button on
   that screen is `variant="outline"` (plan §4.1's own table), so
   `branded.png` had to be rendered from `collect_msisdn` instead, whose
   submit button is the default `variant="primary"`.
3. **`@base-ui/react/select`'s trigger is not a native `<select>`,
   which the plan's §4.1 migration table does not call out but which
   changes how a test interacts with it.** `checkout-view.test.tsx`'s
   locale-switch test used `fireEvent.change` on what was a native
   `<select>`; it had to become click-open, `pointerdown`+`click` on the
   option, matching the pattern `@vpay/ui`'s own `select.test.tsx` (added
   by Lane A) already established.
4. **jsdom does not simulate a native `<button>`'s Space/Enter default
   action**, so a test written against decision D2's rc.0-era `<span
   role="checkbox">` (which Base UI's own JS handled keyboard activation
   for) does not carry over unchanged to the native-button rendering:
   `fireEvent.keyDown`/`keyUp` with `key: ' '` on the new `<button>` fires
   nothing. Lane A's own review independently measured and documented the
   identical limitation for `@vpay/ui`'s `checkbox.test.tsx` ("the Space
   key... jsdom does not translate it to a click on a `<button>`,
   measured") — this lane's fix (assert the native-button property and the
   click directly, defer real keyboard activation to `just test-e2e`) is
   the same shape independently arrived at.

## The rebase (mid-task)

Started against Lane A's first head (`177645e`). The coordinator reported
Lane A's own review had landed seven more commits (head `08d9b8e`) partway
through this lane's work, with five points to address. Rebased
(`git stash` — nothing had been committed yet — `git reset --hard 08d9b8e`,
`git stash pop`; the stash applied cleanly, no conflicts) and addressed all
five:

1. `cn()`'s colour/style-class compose fix — nothing in this lane relied on
   the old (broken) merge behaviour; no change needed, confirmed by the
   full suite staying green.
2. `@vpay/ui`'s own relative imports lost their `.js` suffix — this
   directly **removed** the need for the `webpack.resolve.extensionAlias`
   workaround this lane had written in `next.config.ts` before the rebase
   (the root cause moved upstream); the workaround was deleted rather than
   kept as dead code, and `pnpm --filter @vpay/checkout build` re-verified
   clean without it.
3. Ran `pnpm --filter @vpay/checkout build` (a real `next build`) —
   already part of this lane's gate list before the reminder; re-run
   post-rebase, still clean, evidence above.
4. `tailwind: true` added to `eslint.config.js` in the same commit as the
   Tailwind 4 migration.
5. `just verify-ui` re-run on the rebased tree, exit 0.

## For the reviewer

- `git rev-parse HEAD` for the actual head SHA.
- The three decisive mutations above are reproducible: the exact `sed`/
  `python3` edits are in this lane's own history if a commit-by-commit
  replay is wanted, or re-derive them from the table (each is a one-line
  change at the call site named).
- `docs/plans/exp26-notes/lane-b/*.png` were looked at, not merely
  generated — `outcomes-hosted.png` shows succeeded/failed/canceled as
  green/red/**amber** (D4 visibly correct), `branded.png` shows the submit
  button as `#1d4ed8` with a white foreground.
- The checkbox-renders-as-a-circle observation in `docs/status.md`'s row is
  a daisyUI 5 + `bumblebee` characteristic (`--radius-selector: 1rem` on a
  `1.5rem` box, clamped to 50% by the browser), not something this lane's
  code controls — `class="checkbox"` is daisyUI's own compiled CSS,
  unmodified.
