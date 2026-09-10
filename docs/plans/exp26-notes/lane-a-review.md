# exp26 Lane A — sabotage review

Reviewer pass over `claude/exp26-ui-lane-a` at `177645e` (base `7d52421`, eight
implementing commits). Every claim below is a measurement made in this
worktree; where something could not be measured, it says so instead of
inferring. Lanes B, C and D branched from `177645e`, so every fix here is
additive and renames no export.

## Verdict

**Not safe as drafted.** Two of the three gates the lane reports as delivered
were not doing the work they were reported as doing; two shipped components
carried defects of the exact class the plan is written to prevent — a class
that still parses, still renders and quietly stops styling anything; and the
package could not be built by any consumer at all (finding 9), which every
gate in the lane's own list was blind to.

The numbers in `lane-a.md` reproduce, the component set is real, and the gate
that _is_ wired catches what it says it catches. Mostly the problem was
coverage rather than honesty — with two exceptions, findings 8 and 9, where a
sentence in `docs/status.md` outlived the code it described.

Seven findings fixed, one reported defect not reproduced (finding 10), one
open question surfaced rather than taken.

### Were Lane A's gate claims true on `177645e`?

Recipe by recipe, re-run here on that commit:

| claim                                                                                                 | true?                                                                                                                                                        |
| ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pnpm install --frozen-lockfile` green                                                                | **yes**, exit 0                                                                                                                                              |
| `just lint-web` green                                                                                 | **yes**, exit 0 — including `frontends/apps/checkout`, contrary to the report in finding 10                                                                  |
| `just test-web` green (checkout 448, shop 96, tokens 8, ui 46)                                        | **yes**, every figure exact                                                                                                                                  |
| `just audit-web` clean                                                                                | **yes**, exit 0                                                                                                                                              |
| `build-storybook` green                                                                               | **yes**, exit 0                                                                                                                                              |
| `just verify-ui` red only on checkout's `form-control`                                                | **yes**, and that is the only failing line                                                                                                                   |
| the `verify-ui` gate's four checks each proven by mutation                                            | **yes** for the three it kept; the colour check had four holes (finding 4)                                                                                   |
| `cn()` knows daisyUI's conflict groups                                                                | **yes** for every group it declares; two of them conflated dimensions that compose (finding 5)                                                               |
| the `@source` correction (three `../`, not four)                                                      | **yes**, proved decisively on the compiled CSS                                                                                                               |
| the class-string / one-line-`className` rules are part of the gate                                    | **NO** — the plugin was never wired (finding 1)                                                                                                              |
| "`pnpm -r build` now compiles all packages including the dashboard's `next build`" (`docs/status.md`) | **NO** — that build was broken (finding 9); the lane did not run it, and did not claim to in `lane-a.md`, but left the sentence standing in `docs/status.md` |
| the intermittent-failure rate, and "unmount every Base UI render"                                     | **NO** — did not reproduce in 12 runs, and one render had no `unmount()` (finding 7)                                                                         |
| the axe harness was sanity-checked                                                                    | done once by hand; nothing in the tree (finding 8)                                                                                                           |

## Findings

| #   | severity         | finding                                                                                  |
| --- | ---------------- | ---------------------------------------------------------------------------------------- |
| 1   | gate-hole        | `eslint-plugin-better-tailwindcss` installed, never wired                                |
| 2   | correctness      | `Select`'s popup width compiled to invalid CSS                                           |
| 3   | rule-break       | `Drawer`'s backdrop was a raw `bg-black/40`                                              |
| 4   | gate-hole        | `verify-ui`'s colour check had three measured holes                                      |
| 5   | correctness      | `cn()` dropped a daisyUI colour when a style class followed it                           |
| 6   | rule-break       | a11y properties the components claim and never assert                                    |
| 7   | misleading-claim | the intermittent failure could not be reproduced, and its fix is incomplete              |
| 8   | misleading-claim | the axe harness's negative control existed only in prose                                 |
| 9   | correctness      | `@vpay/ui`'s `.js` import suffixes broke every consumer's `next build` — the second time |
| 10  | not reproduced   | the reported `tailwind.config.ts` lint failure in `frontends/apps/checkout`              |

### 1 — `eslint-plugin-better-tailwindcss` was installed and never configured (gate-hole)

Plan §5 Lane A step 7 asks for it; plan §3 rule 1 ("no `className` string
longer than one line") is the maintainer's own requirement it carries; plan §5's
mutation table says a two-line class attribute must fail `just lint-web`.

Measured at `177645e`:

- `git grep better-tailwindcss` → `frontends/packages/config/package.json`,
  `pnpm-lock.yaml`, and the plan document. No config file.
- `eslint --print-config frontends/packages/ui/src/components/button.tsx` → 133
  active rules, **0** with "tailwind" in the name.
- A deliberately six-line class attribute in `badge.tsx` →
  `pnpm --filter @vpay/ui lint` **exit 0**.

It was also absent from `lane-a.md`'s "What was NOT done", which is the half
that matters: an unwired gate nobody has recorded reads as a gate that runs.

Fixed in `a6e2951`. A `tailwind` option on `vpayEslintConfig`, off by default,
on for `@vpay/ui`. **Not keyed to the existing `react` flag** — measured: the
plugin compiles the Tailwind entry point through the _linted_ package's own
`tailwindcss`, so enabling it wherever React renders aborts ESLint outright in
`@vpay/checkout` and `@vpay/dashboard`, which are on Tailwind 3 until lanes B
and D migrate them. **Each app flips `tailwind: true` in the commit that moves
it to Tailwind 4** — see "For lanes B, C and D" below.

Plan §7 names the rule `no-unregistered-classes`; under the pinned `4.7.0` that
rule is `no-unknown-classes`. The plan is wrong on the name only, and it was
right about its value: it catches daisyUI-5-removed classes inside `@vpay/ui`
independently of `verify-ui`'s grep.

Six mutations, each staged in `badge.tsx`, run, reverted:

| mutation                                    | rule that fired                      |
| ------------------------------------------- | ------------------------------------ |
| a two-line class attribute                  | `enforce-consistent-line-wrapping`   |
| a one-line class attribute over 100 columns | `enforce-consistent-line-wrapping`   |
| `form-control`                              | `no-unknown-classes`                 |
| Tailwind 3 bare-variable arbitrary value    | `enforce-consistent-variable-syntax` |
| `mt-1 mt-1`                                 | `no-duplicate-classes`               |
| `text-xs mt-1`                              | `enforce-consistent-class-order`     |

The rule set's first real run reported ten findings inside `@vpay/ui`, all
fixed in the same commit.

### 2 — `Select`'s popup width compiled to invalid CSS (correctness)

`select.tsx` carried `w-[--anchor-width]` — Tailwind **3**'s shorthand for a
bare CSS variable as an arbitrary value. Tailwind 4 replaced it with
parentheses and does not reject the old form; it compiles it anyway. Compiling
`frontends/packages/ui/src/styles.css` with `@tailwindcss/postcss@4.3.3`:

```css
/* before */ .w-\[--anchor-width\] { width: --anchor-width; }
/* after  */ .w-\(--anchor-width\) { width: var(--anchor-width); }
```

`width: --anchor-width` is not a valid declaration and every browser drops it,
so the locale-switch popup sized to its own content instead of matching its
trigger — silently, on the one page a payer sees. No test could catch it (jsdom
computes no layout) and `verify-ui`'s four greps look for something else
entirely. Plan §6.3's failure mode in Tailwind's own syntax rather than
daisyUI's.

Fixed in `0bb4030`; gated by finding 1's `enforce-consistent-variable-syntax`,
which reports it independently.

One incidental measurement worth carrying forward: **Tailwind's scanner reads
comments as content.** The first draft of the fix's doc comment spelled the
wrong class verbatim and re-emitted the dead rule from the comment itself. Both
that comment and the `no-unknown-classes` comment in `eslint.js` are worded to
avoid naming a class the gates ban — the same reason `verify-ui` already
carries a path exemption for `field.tsx`.

### 3 — `Drawer`'s backdrop was a raw palette colour (rule-break)

`bg-black/40`. Plan §3 says it twice: "No component in this list may contain a
colour literal, a hex value, or an `!important`", and, of what may be written
inside `frontends/packages/ui/src/`, "every colour utility without exception is
a bug report against this list". Now `bg-base-content/40` — the theme's own ink,
so a future dark theme dims with its own colour rather than a hard black.

`git grep` over `frontends/packages/ui/src` for every
`(bg|text|border|ring|fill|…)-(black|white|red|…)` and for `#rrggbb`: exactly
one hit, this line, and no hex anywhere. Fixed in `31b8bc8`.

### 4 — `verify-ui`'s colour check had three measured holes (gate-hole)

The check was `className=.*\b(bg|text|border)-<palette>-[0-9]` over
`frontends/apps` and `examples/shop`. Mutations, staged in
`frontends/apps/dashboard/app/page.tsx`, run, reverted:

| mutation                                                  | before     | after                         |
| --------------------------------------------------------- | ---------- | ----------------------------- |
| `const TONE_CLASS = { failed: 'bg-red-500 text-white' };` | **PASSED** | fails                         |
| `<div className="bg-black/40" />`                         | **PASSED** | fails                         |
| `<div className="text-[#ff0000]" />`                      | **PASSED** | fails                         |
| `<div className="bg-[rgb(255,0,0)]" />`                   | **PASSED** | fails                         |
| `<div className="border-emerald-600" />`                  | fails      | fails                         |
| `const PALETTE = { bad: 'bg-red-600' };` in `@vpay/ui`    | **PASSED** | fails                         |
| `<span className="text-white" />` in `@vpay/ui`           | **PASSED** | fails                         |
| `<div className="bg-base-200 text-error" />`              | passes     | passes — the negative control |

Why the first one matters most: requiring `className=` on the same line made a
class held in a lookup object invisible, and that is not hypothetical —
`TONE_CLASS` in the checkout's `screens.tsx` is exactly that shape today, and
the plan's own counting script (§2) names it as its one known blind spot. The
gate had inherited the measurement's blind spot.

Fixed in `0c72c0d`: the `className=` prefix dropped, `black`/`white` added,
arbitrary colour values added as a second check, and `frontends/packages/ui/src`
brought in scope. One path exemption, in the gate's existing style with the
reason inline: `cn.ts`, whose doc comment uses `bg-red-500 bg-blue-500` in prose
as the example of a Tailwind conflict group — measured to be the only file in
the tree that matches without an offending class.

### 5 — `cn()` dropped a daisyUI colour when a style class followed it (correctness)

Measured at `177645e`:

```
cn('btn-primary', 'btn-outline')            -> 'btn-outline'
cn('badge badge-outline', 'badge-success')  -> 'badge badge-success'
```

Both drop a class the caller wrote. Read against daisyUI 5.7.28's own compiled
CSS, that is wrong — colour and style are orthogonal dimensions, and the style
classes read the variable the colour classes set:

```css
.btn-primary   { --btn-color: var(--color-primary); … }
.btn-outline,
.btn-dash      { --btn-bg:#0000; --btn-border: var(--btn-color, …); … }
.badge-primary { --badge-color: var(--color-primary); … }
.badge-outline { color: var(--badge-color); --badge-bg:#0000; … }
```

`btn-primary btn-outline` is how daisyUI 5 spells "a primary outline button".
Plan §3's illustrative `classGroups` snippet put them in one group, which
contradicts the principle stated two lines above it in the same document —
"one group per daisyUI component whose modifiers are mutually exclusive **by
design**". They are not.

Fixed in `1cca924`: `daisy-btn-style` and `daisy-badge-style` split out.
**No existing assertion was changed or removed** — all 18 cases still pass
verbatim; four were added, including the two that pin composition. Mutations:
emptying `daisy-btn-style` fails its own named case; moving `outline` back into
the colour group fails two.

**Open question, surfaced rather than taken.** `ghost` and `link` deliberately
stay grouped with the colours. daisyUI's CSS would support the same argument
for them (`.btn-ghost` also reads `--btn-color`, for its text), but `Button`'s
own `cva` map offers `ghost` as an alternative to `primary`, and plan §3 pins
`cn('btn btn-primary', 'btn-ghost') === 'btn btn-ghost'` as this file's named
acceptance case. Whether a _ghost primary_ button should be expressible is a
product decision about the component's API, not a review's.

### 6 — a11y properties the components claim and never assert (rule-break)

Fixed in `1c47d1d`; 46 → 60 cases.

- **Checkbox.** `docs/status.md` records of the checkbox D2 replaced: "the label
  click and space key both _measured_". Neither was re-asserted after the swap
  — and a `<button role="checkbox">` is not a form control, so a label does not
  associate with it the way it does with an `<input>`, which is precisely what
  D2 could have broken. Three cases now: a wrapping `<label>`, a `htmlFor`
  label, and the native-button assertion. **Both label forms work** — D2 is
  sound, and now it is measured rather than argued.
- **The Space key is deliberately NOT asserted**, and the test says so.
  Measured: `fireEvent.keyDown/keyUp` with `' '` — and with `Enter` — leaves
  `aria-checked` at `false`, because a real browser turns those into a `click`
  on a `<button>` and jsdom does not implement that. Asserting it would be
  asserting jsdom, which is the failure mode `CLAUDE.md` names. What is
  asserted is the two halves the browser's behaviour is made of: a real
  `<button type="button">` with `tabindex="0"`, and a click that toggles.
- **Dialog.** Focus trap and Escape, neither previously touched. Both fail if
  the Base UI primitive is swapped for daisyUI's CSS-only `modal` markup.
- **Select.** Keyboard navigation, on a component that is a button trigger plus
  a portalled listbox rather than a native `<select>`. Committing with Enter is
  **not** asserted: measured, Base UI does not commit on a synthetic keydown on
  the list, the option, the trigger or `document.activeElement`, and a case that
  fires Enter then asserts nothing changed would be asserting the harness. That
  is real-browser work, plan §7 row 6.
- **Input** had no test file at all, against plan §5's "a case per component".

### 7 — the intermittent failure did not reproduce, and its fix was incomplete (misleading-claim)

Commit `177645e` reports "the failure had surfaced at least once in roughly ten
runs before" the `unmount()` fix. **It could not be reproduced here.** The
parent commit `ef39ec2`'s `frontends/packages/ui/src` was restored into the
worktree and `pnpm --filter @vpay/ui test` run **twelve** consecutive times:
12/12 exit 0, zero `window is not defined`, zero unhandled errors.

That does not make the fix wrong — cancelling scheduled work is correct
discipline and cost nothing — but the evidence for it is one observation that
does not reproduce on this machine, and the commit message presents it as a
characterised rate.

The fix is also **not complete as its message describes it**. It says "unmount
every Base UI render in @vpay/ui's own tests"; `drawer.test.tsx`'s Escape case
mounts a Base UI `Drawer` — focus guards and transition timers, the
highest-risk primitive in the set — and never called `unmount()`. That file
does not appear in the commit's own enumeration either. Fixed in `1c47d1d`.

After every change here: `pnpm --filter @vpay/ui test` run five times in a row,
**60/60 each time, 0 unhandled errors**.

### 8 — the axe harness's negative control existed only in prose (misleading-claim)

`docs/status.md` reads: "sanity-checked against a deliberately unlabelled
`<button>` before being trusted". That was done once, by hand, and left nothing
in the tree. "Zero violations" is evidence only if the harness can report a
violation at all — a mis-scoped container, a stale rule id in `runOnly`, or an
`axeViolations` that swallowed its result would each produce a green run over a
broken page.

It is a test now (`1c47d1d`), and it runs first. Proved by mutation: making
`axeViolations` return `[]` unconditionally fails it and leaves the
kitchen-sink case green.

### 9 — `@vpay/ui`'s `.js` import suffixes broke every consumer's build (correctness)

Reported by Lane C, reproduced here on the unmodified `177645e` before fixing.
`pnpm --filter @vpay/dashboard build`:

```
../../packages/ui/src/index.ts
Module not found: Can't resolve './cn'
Module not found: Can't resolve './components/button.js'
Module not found: Can't resolve './components/card.js'
Build failed because of webpack errors
```

`@vpay/ui` ships TypeScript source (`main: ./src/index.ts`) and its tsconfig
sets `moduleResolution: "bundler"`, under which `tsc` and Vitest resolve
`'./cn.js'` back to `cn.ts` and pass. Next's webpack resolver takes the suffix
literally. Typecheck, lint and all 60 vitest cases were green over an app that
could not be built at all.

**This is the second occurrence, and the documentation had already been
corrected past it.** `docs/status.md`'s "`@vpay/ui` production build
(`next build`)" row records the first by name and mechanism and states
"Suffixes were dropped from `frontends/packages/ui/src/index.ts` and every
component file". `88b2808` put them back on all 48 files; `dc0e243` then edited
that very row and left the sentence standing. That is the one place in this
lane where a claim and the code disagreed rather than a claim simply reaching
further than its evidence.

**Why no gate saw it.** Neither `just lint-web` nor `just test-web` runs a
`next build`; `pnpm -r build` and `just test-e2e` do, and neither is in
`lane-a.md`'s gate table or in `just ci`. The lane did not claim to have run
them — but `docs/status.md` claimed the outcome of one.

Fixed in `e2a0a09`, and **gated**, because a comment had not held it:
`verify-ui` check 5 greps for a `.js`-suffixed relative import under
`frontends/packages/ui/src`. It re-runs the original failure's cause rather
than a proxy. Mutation: restoring the suffix on `index.ts`'s first line alone
makes the gate exit non-zero naming file and line.

After: `pnpm --filter @vpay/dashboard build` exit 0 ("Compiled successfully",
4/4 static pages) and `pnpm -r build` exit 0 across the workspace.

### 10 — the reported checkout `tailwind.config.ts` lint failure did NOT reproduce

Also reported by Lane C: `just lint-web` failing on
`frontends/apps/checkout/tailwind.config.ts`. **Measured four times in this
worktree — at `177645e` and at this head — `just lint-web` exits 0**, and
`pnpm --filter @vpay/checkout typecheck` exits 0 on its own.

The mechanism is real and worth writing down even though the defect is not
present here. `tailwind.config.ts` opens `import daisyui from 'daisyui'`, and
daisyUI **5** ships no type declarations for that import shape, so the moment
`daisyui` resolves to 5.x in that package the file cannot typecheck. Forced
here to confirm, by pointing the import at a 5.7.28 symlink:

```
tailwind.config.ts(1,21): error TS2307: Cannot find module … or its
corresponding type declarations.
```

On this branch checkout still declares `daisyui ^4.12.23` / `tailwindcss
^3.4.17` and resolves **4.12.24 / 3.4.19** — read from
`frontends/apps/checkout/node_modules/daisyui/package.json`, and pinned that
way in `pnpm-lock.yaml`, so a clean `pnpm install --frozen-lockfile` cannot
produce anything else. A worktree that installed from a _modified_ manifest and
then stashed only its source diff would keep the modified `node_modules`, which
is the most likely origin of the report.

**Deliberately not "fixed" here, and the reason is not laziness.** There is
nothing broken on this base to fix, and the correct fix is plan §4.1's —
delete `tailwind.config.ts` — which belongs to Lane B and cannot be done alone:
the app still uses `@tailwind base/components/utilities` and needs that file's
`content` globs and daisyUI plugin registration, so deleting it before
migrating the app to Tailwind 4 would ship an unstyled payment page. Pinning
checkout's `daisyui`/`tailwindcss` to exact versions was considered and
rejected: it rewrites `pnpm-lock.yaml`, which three lanes are about to rebase
across, to defend against a resolution the lockfile already prevents.

**Lane B must expect this**: the first thing that happens when checkout gains
`daisyui@5` is TS2307 on `tailwind.config.ts`, and the answer is to delete the
file in the same commit, per §4.1, not to type around the import.

## What the sabotage pass checked and found sound

- **`cn()`'s daisyUI conflict groups are real, not decoration.** Every required
  case measured directly: `cn('btn-primary','btn-secondary')` → `btn-secondary`;
  `cn('btn','btn-sm','btn-lg')` → `btn btn-lg`; the same for badge, alert,
  input, select, checkbox, radio, card, table and loading, in both the variant
  and the size dimension. Non-conflicting daisyUI classes are **not** dropped:
  `cn('btn btn-primary','btn-block','btn-wide')`, `cn('card','card-body',
'bg-base-200')`, `cn('table table-zebra','table-pin-rows')`,
  `cn('modal modal-box','modal-open')`, `cn('join','join-item')` all keep every
  class. Deleting `'ghost'` from `daisy-btn-variant` fails exactly its own named
  case and nothing else; emptying `daisy-alert-variant` likewise.
- **The `@source` correction in `lane-a.md` finding 1 is right, and decisively
  so.** Unique marker classes were planted in `frontends/apps/checkout/src` and
  `frontends/apps/dashboard/app`, `styles.css` compiled with
  `@tailwindcss/postcss`, and the `@layer utilities` block read: with **three**
  `../` both markers are generated (281 utilities); with the plan's **four**
  they are not (225 utilities). The apps really are scanned.
- **`verify-ui`'s other three checks all fire**, each proved by a staged
  mutation in an app and reverted: `form-control`, a bare `cva(`, and an
  `!important` outside the three named exemptions. The exemption list names
  every exempted file with its reason inline.
- **The `verify-ui` failure the lane reports is the only one**, and it is
  `frontends/apps/checkout/src/components/screens.tsx:408-409`'s
  `form-control`/`label-text` — Lane B's migration, not a Lane A defect.
- **Storybook 10 builds** (`build-storybook` exit 0), and **every component has
  a story**: 15 story files for 15 component modules, 0 missing.
- **`lane-a.md`'s numbers reproduce.** `exp26-plan-count.sh` at `177645e` gives
  `OUTSIDE @vpay/ui` = 96 / 29 / 45 / 18 / 107 / 92 / 224 / 3 / 245 / 0 / 24 —
  unchanged from the baseline, exactly as claimed — and `@vpay/ui`'s own
  `class_tokens_distinct` = 130, the figure the notes record along with the
  reason it exceeds the plan's 80–110 estimate.
- **`pnpm install --frozen-lockfile` exit 0; `just audit-web` exit 0** ("no high
  or critical advisory in the workspace", both `--prod` and full), after five
  majors and two deleted overrides.
- **`just test-web` reproduces every figure**: checkout 448/448 in 23 files,
  shop 96/96, tokens 8/8, config 63/63, api-client 4/4, sdks 146 + 190.
- The two deleted `pnpm.overrides` really are gone from the root `package.json`,
  with the measured reason recorded in its own note block.

## Not checked

- **`color-contrast` under `bumblebee`.** Storybook's a11y addon runs in the
  Storybook _UI_; `build-storybook` does not run axe, and no
  `@storybook/test-runner` is installed. The structural axe pass is vitest's and
  excludes contrast on purpose. So the numbers this review can report are
  structural only: **0 violations across every component, and a negative control
  proving the harness can report one.** Contrast remains unverified by anything,
  exactly as `lane-a.md` and `docs/status.md` both say. Plan §7 row 6
  (`cypress-axe` in a real browser) is still owed by whichever lane lands last.
- **`just test-e2e`.** Not run: it needs the compose stack and a Cypress binary
  from a CDN, and no lane has migrated an app yet, so nothing it covers has
  changed.
- **`just ci` in full.** Rust is untouched by this lane (`git diff --stat
7d52421..HEAD` names no file under `backends/`), so `fmt-check`, `clippy`,
  `test-rust`, `test-doc`, `verify-ignored` and `deny` were not re-run. Per plan
  §7 row 2, a _changed_ Rust number would itself be a finding; there is no
  mechanism here by which one could change.
- **A real browser.** No page in this repository has been rendered by one on
  this branch. `@vpay/ui` has no consumer yet — the three apps are still on
  Tailwind 3 — so there is nothing to screenshot that a lane has not yet built.

## For lanes B, C and D

1. **No export was renamed, added or removed.** `frontends/packages/ui/src/index.ts`
   is byte-identical to `177645e`.
2. **`cn()` changed behaviour in one direction only, and only additively**: a
   daisyUI colour class and a style class (`outline`/`soft`/`dash`) now both
   survive a merge instead of the colour being dropped. Nothing that used to
   survive is dropped now. Component output is unchanged — a `cva` map emits one
   class per dimension either way.
3. **Turn on the class-string lint in the same commit that moves your app to
   Tailwind 4**: add `tailwind: true` beside `react: true` in the app's
   `eslint.config.js`. It is off today because the plugin cannot load in a
   Tailwind 3 package, and leaving it off after the migration means shipping the
   app without the maintainer's one-line rule. Expect it to report class-order
   findings; `eslint --fix` handles those. Do **not** accept its line-wrapping
   autofix — the remedy for a too-long class attribute is a `cva` variant or a
   layout primitive, not a wrap. `select.tsx` shows the pattern for a static
   string that cannot fit: hoist it to a module constant.
4. **`verify-ui`'s colour check is stricter**: `black`/`white`, arbitrary colour
   values (`bg-[#…]`, `text-[rgb(…)]`), and classes outside a `className=`
   attribute now all fail, and `frontends/packages/ui/src` is in scope. daisyUI
   theme tokens are unaffected. If you hit a false positive on prose, add a path
   exemption with the reason inline — that is the gate's established style — and
   prefer rewording, because each exemption is a hole.
5. **`@vpay/ui`'s relative imports no longer carry a `.js` suffix**, and
   `verify-ui` check 5 fails the build if one comes back. If you add a file to
   that package, import it as `'./thing'`. `moduleResolution: "bundler"` makes
   the suffixed form typecheck and test green while breaking every consumer's
   `next build`, which is how it survived a whole lane twice.
6. **Run `pnpm --filter <your app> build` before you report.** Neither
   `just lint-web` nor `just test-web` runs a `next build`, and that is the gap
   finding 9 lived in.
7. **`verify-ui` is still red on the tree**, on `screens.tsx`'s
   `form-control`/`label-text`. That is Lane B's, and it is the last thing
   standing between this branch and a green `just verify`.
