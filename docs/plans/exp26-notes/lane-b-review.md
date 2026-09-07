# exp26 Lane B — sabotage review

Reviewer pass over `claude/exp26-ui-lane-b` at `c105884` (base `08d9b8e`,
Lane A's reviewed head; three Lane B commits on top). Every number below is a
measurement made in this worktree; where something could not be measured it
says so instead of inferring.

Run `git rev-parse HEAD` for the actual head; do not trust a SHA pasted here
over that command.

## Verdict

**Not safe as drafted.** The migration itself is real and the component
composition is good work — but three of the things it reported are not what
happened, and two of them are on the payer's screen:

- the payment-failure alert stopped clearing WCAG AA (3.53:1, down from
  6.82:1 under daisyUI 4) and nobody measured it;
- the language switch lost its visible label and the page header stopped
  being a `<header>`, both unreported, both invisible to every gate;
- the `just ci` failure was attributed to Lane D on the strength of a
  comparison that was run in a differently-placed worktree; re-run properly,
  it is Lane B's own regression.

The counts reproduce. The decisive mutations the lane claimed all fire. The
`theme.ts` collapse and its independent contrast test are good, and the
hydration guard is untouched. Mostly the problem is that a "deleted" column
listing CLASS NAMES was read as licence to delete ELEMENTS, and that nothing
in the gate list looks at a rendered pixel.

Nine findings, all fixed. One claimed mutation that does not fire (finding 9),
one open question surfaced rather than taken (the checkbox's shape).

### Were Lane B's own gate claims true at `c105884`?

Recipe by recipe, re-run here on that commit:

| claim | true? |
|---|---|
| `pnpm --filter @vpay/checkout typecheck` clean | **yes**, exit 0 |
| `pnpm --filter @vpay/checkout lint` clean | **yes**, exit 0 |
| `pnpm --filter @vpay/checkout test` 459/459, 23 files, 0 skipped | **yes**, exact |
| `pnpm --filter @vpay/checkout build` a real `next build`, compiled CSS read | **yes** — and the CSS does carry every class claimed |
| `just verify-ui` exit 0 | **yes** |
| `just verify` all eleven gates exit 0 | **yes** — confirmed inside `just ci`, and every Rust number equals master's: **1466 tests run, 1466 passed, 0 skipped** |
| the three decisive mutations | **yes**, all three fire; re-run below, plus three more |
| screenshots regenerated and looked at; canceled amber; branded `#1d4ed8` white-on-blue | **yes** for what they show; **no** for what they were not looked at for (finding 1) |
| `just lint-web` red "pre-existing, reproduced identically on Lane A's own unmodified head `08d9b8e` with zero Lane B changes applied — Lane D's job" | **NO** (finding 3). It is Lane B's regression |
| `just test-e2e` "could not complete — the `dashboard` image build fails" | **NO** (finding 8). It completes: 11/11 across four specs |
| "a real browser's native activation is what `just test-e2e` now proves instead" (`docs/status.md`) | **NO** (finding 6). No Cypress spec touches the checkbox |
| `styling_files` 2, "target missed by one", the target and plan §4.1 "in mild tension" | number true, reasoning **NO** (finding 4). The target is reachable and is now met |

## Findings

| # | severity | finding |
|---|---|---|
| 1 | correctness | the payment-failure alert fails WCAG AA — 3.53:1, a regression from daisyUI 4's 6.82:1 |
| 2 | correctness | the language switch lost its visible label |
| 3 | misleading-claim | `just ci`'s failure is Lane B's, not pre-existing; the comparison was run in a worktree at a different filesystem depth |
| 4 | rule-break | seven raw utilities left in `screens.tsx` against plan §3, and the `styling_files ≤1` target argued away rather than met |
| 5 | rule-break | the `<header>` banner landmark deleted from both views |
| 6 | misleading-claim | "a real browser's native activation is what `just test-e2e` now proves" — no spec touches the checkbox |
| 7 | gate-hole | plan §7 row 5's axe run over "every checkout screen" was never built, and row 6 (contrast) is what finding 1 slipped through |
| 8 | misleading-claim | `just test-e2e` was reported as unable to complete; it completes, 11/11 |
| 9 | nit | `transpilePackages: ['@vpay/ui']` is not load-bearing; no mutation of it fails |

### 1 — the payment-failure alert fails WCAG AA (correctness)

daisyUI 5's bumblebee pairs `--color-error-content` oklch(39% .141 25.723)
with `--color-error` oklch(70% .191 22.216) — **#801518 on #ff6266, 3.53:1**,
against AA's 4.5:1 for body text. `.alert-error` is what `OutcomePanel` renders
for `failed`, so the one screen in this product that shows it is the one
telling a payer their money did not move.

Measured three ways rather than argued:

| source | result |
|---|---|
| the compiled theme block in `.next/static/css/*.css` | #801518 on #ff6266, 3.53:1 |
| the pixels of the lane's own committed `outcomes-hosted.png` | bg rgb(255,98,102), darkest glyph rgb(128,21,24), **3.53:1** |
| the pixels of daisyUI 4's `docs/plans/exp21-checkout-page-notes/outcomes-hosted.png` | black on #ff5861, **6.82:1** |

So it is a **regression of this revamp**, not an inherited defect: daisyUI 5
changed how a theme's `*-content` is derived. `.alert-info` is 4.27:1 by the
same measurement. Nothing errors and nothing warns — the failure mode plan
§6.3 is written to catch, arriving through the theme rather than through a
class name. The screenshot was regenerated and looked at, and this is what
looking at a screenshot does not tell you.

Fixed in `bfa3f40`. Two theme tokens at daisyUI's own hue and chroma,
darkened only as far as AA needs (4.61:1 and 4.63:1), declared UNLAYERED in
`@vpay/ui/src/styles.css` so they beat daisyUI's `@layer base` block — the
same mechanism `theme.ts` already uses at runtime for `--color-primary`.
Colour in a theme token is where plan §3 says colour belongs; no component
and no app gains a colour literal, and `just verify-ui` still exits 0.

`frontends/packages/ui/src/theme-contrast.test.ts` is the gate. It compiles
`styles.css` with the real `@tailwindcss/postcss` and the real daisyUI,
resolves each `--color-*` **through the layer cascade**, and computes the WCAG
ratio for every tone a component can actually render. Two decisive mutations:
delete the override (three cases fail, 3.54 and 4.27), and move the override
INSIDE `@layer base` (the same three fail — which is what proves the gate
models the cascade rather than source order).

Real-browser confirmation, Chrome, computed styles rasterised to sRGB from the
app's own `next build` stylesheet — the evidence plan §7 row 6 asks for and
says jsdom cannot give:

```
.alert-error    rgb(101,0,9)   on rgb(255,98,102)   4.62:1
.alert-success  rgb(0,76,57)   on rgb(0,211,144)    5.12:1
.alert-warning  rgb(121,50,5)  on rgb(252,183,0)    5.24:1
.btn-primary    rgb(115,62,10) on rgb(253,199,0)    5.51:1
```

`secondary` is 4.09:1 and `accent` untested — neither is reachable through any
component's `cva` map, so neither is asserted, and a case fails if one becomes
reachable. Surfaced rather than silently included.

### 2 — the language switch lost its visible label (correctness)

`locale-switch.tsx` was `<label className="text-sm opacity-70">` +
`<select>`. It became `<Select aria-label={t('locale.label')}>`. The
accessible name survived; the visible word did not — a sighted payer is now
looking at a bare "English"/"Français" combobox beside the page title with
nothing saying what it does, on the page where the wrong language means
approving a payment you cannot read.

Plan §4.1's row for this file deletes the label's CLASSES (`text-sm
opacity-70`, alongside `flex items-center gap-2` and `select
select-bordered select-sm`). It does not delete the label. The change is
visible on the committed screenshots and was reported nowhere.

Why nothing caught it: `getByLabelText` is satisfied by an `aria-label`. The
existing case went on passing.

Fixed in `1a47a0e`. `Select` gains `aria-labelledby` (preferred over
`aria-label`, said so in its prop doc); the app names the control with a
visible `<Text>` and still writes no class of its own. The new case asserts
the name is RENDERED TEXT — an element carrying the dictionary string, not
`sr-only`, referenced by `aria-labelledby`, with no `aria-label` standing in.
Decisive mutation: restore the `aria-label` and drop the `<Text>` — fails with
`the combobox is named by a visible element: expected null to be truthy`.

### 3 — the `just ci` failure is Lane B's own (misleading-claim)

`docs/status.md` and `lane-b.md` both say `frontends/apps/dashboard/
tailwind.config.ts`'s typecheck failure was "reproduced identically on Lane
A's own unmodified head (`08d9b8e`) with zero Lane B changes applied — Lane
D's job". It was not.

Measured in ONE worktree, only the commit changing, fresh
`pnpm install --frozen-lockfile` each time:

```
git checkout 08d9b8e && pnpm --filter @vpay/dashboard typecheck   -> exit 0
git checkout c105884 && pnpm --filter @vpay/dashboard typecheck   -> exit 2
```

The earlier comparison was run in a worktree at a different filesystem depth.
Node's resolution algorithm walks up the directory tree, so where a worktree
sits changes where it stops — which is exactly what made a Lane B regression
look pre-existing. This review's own first attempt made the same mistake and
got the same wrong answer; it is recorded here because it is a trap for
anyone comparing two heads in this repository.

Mechanism: daisyUI 4's `src/index.d.ts` opens with `import type plugin from
"tailwindcss/plugin"` and declares **no** `tailwindcss` peer. Under
`node-linker=isolated` there is nothing to resolve inside daisyui's own
directory, so TypeScript walks up into pnpm's hidden hoist directory
(`node_modules/.pnpm/node_modules/`) and takes whichever version pnpm hoisted
there. While two workspace packages were on Tailwind 3 that was 3.4.19; moving
`@vpay/checkout` to Tailwind 4 made 4.3.3 the hoisted one, and daisyUI 4's
plugin arrived typed against Tailwind 4's `PluginAPI` to be checked against
Tailwind 3's `Config`.

Fixed in `104ade7`, a `pnpm.packageExtensions` entry declaring the peer
daisyUI 4 always had — `.npmrc`'s own policy ("undeclared imports fail loudly
rather than working by accident through hoisting") applied to the package
slipping through it. Seven lines of lockfile. No `frontends/apps/dashboard`
file is touched; that app stays Lane D's, and this entry is removed when Lane
D deletes the workspace's last daisyUI 4 dependent.

### 4 — seven raw utilities against plan §3, and a target argued rather than met (rule-break)

`screens.tsx` kept `h-8 w-auto`, `text-3xl`, `tabular-nums`, `break-all`,
`sr-only` and `cursor-pointer`, argued as functional rather than decorative.
They are functional; that is an argument for owning them in the package, not
for writing them at a call site. Plan §3: "Raw utilities are allowed **only
inside** `frontends/packages/ui/src/`". `cursor-pointer` is not in §3's
permitted set at any location.

The lane read the `styling_files ≤1` target and §4.1's table as "in mild
tension". They are not: §4.1 lists `app/layout.tsx` as "unchanged (2 tokens,
app chrome)", which is exactly one styling file, so ≤1 means `screens.tsx`
keeps none.

Fixed in `aa3c585`: `Text` gains `size="3xl"`, `numeric` and
`wrap="anywhere"`; `VisuallyHidden`, `Logo` and `CheckboxLabel` are new. This
also removed a dead prop — the amount was `<Text size="lg"
className="text-3xl">`, where `cn()` drops `text-lg` and `size="lg"` styled
nothing. `screens.tsx` now contains the string `className` zero times, in code
or in prose.

Proof the move did not silently stop styling anything: a real `next build`,
compiled CSS read directly — `text-3xl` 5, `tabular-nums` 3, `break-all` 2,
`cursor-pointer` 1, `.h-8` 1, `w-auto` 1, `sr-only` 1. Every utility still
emitted from its new home under `@source './components'`.

§3's exhaustive permitted list needs `cursor-*` added and is amended in the
plan with a dated note: `select.tsx`'s `ITEM_CLASS` already relied on it
before this review, so the list was already incomplete.

### 5 — the `<header>` landmark deleted from both views (rule-break)

`checkout-view.tsx` and `return-view.tsx` both had `<header className="flex
items-center justify-between gap-4">`. Both became `<Stack>`, which renders a
`<div>` — the `banner` landmark left the accessibility tree on both of the
app's two pages. §4.1 deletes that header's CLASSES, which are duplicated
verbatim across the two files and are exactly what `PageShell`/`Stack` exist
to absorb; it does not delete the element. Reported nowhere.

Fixed in `de2937c`: `Stack` gains `as`, defaulting to `div`, restricted to the
grouping elements a stack can honestly be. Decisive mutation: drop `as` in both
views — fails with `Unable to find an accessible element with the role
"banner"`. Also re-indents `return-view.tsx`'s live region, left two columns
out by the `PageShell` wrap; `just ci`'s `fmt-check` is `cargo fmt` and does
not read TSX, so nothing would have said so.

### 6 — the keyboard claim (misleading-claim)

`docs/status.md`: "a real browser's native activation is what `just test-e2e`
now proves instead". `lane-b.md`: "defer real keyboard activation to
`just test-e2e`".

Measured: `git grep -n 'remember\|checkbox' frontends/tests/e2e/cypress/e2e/`
returns nothing. **No Cypress spec touches the memory opt-in at all**, by
keyboard or otherwise. The rewrite was honest about jsdom's limitation — a
real `<button>` answers Space through the browser's own default action, which
`fireEvent` does not simulate, and Lane A's review measured the same thing —
but the coverage it deferred to does not exist. Space/Enter activation of the
memory opt-in is tested nowhere.

The vitest assertions are kept and are the right ones for jsdom (native
`<button>`, `tabindex="0"`, the click path). The claim is corrected in
`docs/status.md` and here, and the gap is named as a gap rather than as
coverage. Not closed: `checkout.cy.ts` is not Lane B's file (plan §5's "Owns"
list), and adding a spec to it is a scope decision for the coordinator.

The label-click half IS now measured rather than asserted — `CheckboxLabel`'s
own case clicks the sentence and expects the handler, which is what proves a
`<button role="checkbox">` is a labelable element in practice and not just in
the spec text.

### 7 — axe over the checkout screens was never built (gate-hole)

Plan §7 row 5: `axe-core@4.13.0` in vitest over "every `@vpay/ui` component
**and every checkout screen**". Lane A built the first half and the harness;
nothing ran it over the screens those components compose into, and that is
where findings 2 and 5 live — a `Field` that passes in isolation can still be
mounted without a label, and a `Stack` that passes in isolation can still be
the `<header>` that stopped being one.

Fixed in `666ead8`: 46 cases, every state in `CHECKOUT_SCREENS` and
`RETURN_SCREENS`, both locales, zero violations on this head. `@vpay/ui` gains
a `./testing` export so the fifteen structural rules live in one file and the
two suites cannot drift. Contrast is deliberately not in it (plan §7 row 6:
"a green jsdom contrast run is not evidence"); the theme's contrast is
finding 1's gate. Decisive mutation: delete `MsisdnForm`'s `<FieldLabel>` —
four cases fail with `expected [ 'label: 1 node(s)' ] to deeply equal []`.

Plan §7 row 6's `cypress-axe` is still not built, by this review either. Its
question — does the theme hold contrast in a real browser — is now answered
for this head by direct measurement (finding 1) and gated in vitest; the
general harness is not.

### 8 — `just test-e2e` completes (misleading-claim)

Reported as unable to complete, with the `dashboard` image build named as the
blocker, and `dashboard.cy.ts` recorded as not run. On this head it completes:

```
just demo_project=exp26b-review demo_port=29080 demo_receiver_port=29083 \
     demo_orange_port=29082 demo_checkout_port=29081 demo_shop_port=29001 \
     test-e2e                                            -> exit 0

checkout.cy.ts        1/1
dashboard.cy.ts       3/3
shop-hosted.cy.ts     3/3
shop-embedded.cy.ts   4/4   (VPAY_E2E_FRAMED=1)
———————————————————————————
                     11/11, 0 failing, 0 pending, 0 skipped
```

That is plan §7 row 3's pass condition exactly. The dashboard image builds and
its container reports healthy. Stack torn down (`down -v`) by the recipe.

Whether the earlier failure was the `.js`-suffix defect Lane A's review fixed
upstream, or finding 3's resolution, is not established here — only that the
recipe passes as written, so the report should not stand.

The lane also recorded an operator mistake (running `gen-demo-keys` with a
`demo_port` that did not match the stack's). The recipe already guards it:
`gen-demo-keys: .e2e/application-demo.yml was generated for a different
demo_port than 29080 — regenerating the pair`, observed in this run.

### 9 — `transpilePackages: ['@vpay/ui']` is not load-bearing (nit)

The lane's own comment presents it as required. Measured: remove `@vpay/ui`
from the list, `rm -rf .next`, `pnpm --filter @vpay/checkout build` — **exit
0**. So the mutation this review proposed for it does not fire, and nothing
gates the line.

Keeping it is still right — the package genuinely ships `.ts`, and the
alternative is relying on a resolution that happens to work. The comment is
corrected to say which of those two things is measured.

## The checkbox's shape — surfaced, not decided

`Checkbox` renders as a full circle rather than a rounded square, which reads
as a radio button. Measured in Chrome: `border-radius: 16px` computed on a
24px control, because bumblebee sets `--radius-selector: 1rem` on a `1.5rem`
box and a browser clamps a radius at 50%. Under daisyUI 4 the same control was
a rounded square (`docs/plans/exp21-checkout-page-notes/entry-screens.png`).

It is a theme value, not a defect in any code this branch owns: the app
renders `class="checkbox"` and nothing else, and daisyUI's own compiled CSS
does the rest. Two things argue it is tolerable — daisyUI 5 ships it for every
bumblebee site, and the checked state is unambiguous (daisyUI 5.7.28 styles
`.checkbox[aria-checked=true]::before` explicitly, so decision D2's native
`<button role="checkbox">` does get the tick; measured in the compiled CSS,
not assumed). One argues it is not: on a payment page a circle beside a single
opt-in invites a payer to read it as "choose one of".

Overriding `--radius-selector` would be one line in the same block finding 1
added. It is a visual design decision and it belongs to the maintainer, so it
is recorded here rather than taken.

## Mutations, run on the reviewed head

Each was applied to the shipping source, the suite (or the build) was run, and
the source was restored. `git diff --stat` empty after each. A mutation that
fails nothing is a test that is not testing.

| # | mutation | result |
|---|---|---|
| M1 | `OutcomePanel` renders `tone="neutral"` for every outcome | **15 failed** / 507 |
| M2 | the MSISDN submit loses `type="submit"` | **1 failed** — "leaves the MSISDN form's only submit button the submit button" |
| M3 | `themeStyleSheet` skips the `linearRgb` null-check | **10 failed**, including both XSS strings |
| M4 | `ScreenHeading` drops `tabIndex={-1}` | **13 failed** |
| M5 | the theme `<style>` goes back inside an explicit `<head>` (plan §6.5) | **1 failed** — "renders NO explicit `<head>` element" |
| M6 | `transpilePackages` loses `@vpay/ui` | **nothing failed**, `next build` exit 0 — finding 9 |
| M7 | the language switch goes back to `aria-label` only | **1 failed** — "names the language switch on the screen" |
| M8 | `<Stack as="header">` -> `<Stack>` in both views | **1 failed** — `Unable to find an accessible element with the role "banner"` |
| M9 | `MsisdnForm` loses its `<FieldLabel>` | **4 failed** — `[ 'label: 1 node(s) ]` in both locales |
| M10 | delete the theme-contrast override from `styles.css` | **3 failed** — error 3.54:1, info 4.27:1 |
| M11 | move that override INSIDE `@layer base` | **3 failed** — the gate models the cascade, not source order |

M1-M5 are the lane's own table (M4 and M5 it deferred rather than ran); M6-M11
are this review's.

## Gates on the reviewed head

See `docs/status.md`'s Lane B row for the numbers, recipe by recipe.

## What this review did NOT do

- **`cypress-axe`** (plan §7 row 6's general harness). Not built. Its specific
  question is answered and gated for the tones this product renders; a reusable
  browser-side axe run is not there.
- **Space/Enter activation of the memory opt-in in a real browser** — finding
  6. Named as a gap; not closed, because `checkout.cy.ts` is not this lane's
  file.
- **`examples/shop` and `frontends/apps/dashboard`** — untouched as source.
  The only change reaching either is finding 3's root `package.json` entry and
  finding 1's theme tokens in `@vpay/ui`, both of which apply to every consumer
  by design.
- **The `<head>` mutation as a CYPRESS check** (plan §6.5's own wording). Run
  as a vitest mutation instead — `layout.test.tsx`'s "renders NO explicit
  `<head>` element" fails — because a full `just test-e2e` cycle per mutation
  was not spent. The Cypress side is covered by `shop-hosted.cy.ts` passing
  3/3 on this head, which is what §6.5 names as the decisive check for the
  unmutated tree.
- **`--radius-selector`** — surfaced above, not decided.
