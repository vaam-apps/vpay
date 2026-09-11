# exp53 — sabotage review of `@vpay/ui` (PR #116)

Branch `claude/exp53-ui-package`, reviewed at `7a8786d`, in place. The
implementer never got a review — the host died first — so this is the only
thing between the branch and `master`. Everything below was run, not reasoned
about; where a number is asserted, the command that produced it is named.

The brief's question was whether this is a real refactor or a folder structure
over the same complexity. **It is a real refactor**, and the component code is
better than the thing it replaced. What did not hold up is some of what the
branch says about itself: two of the seven gates could not fail, and three
counts in `docs/status.md` and the notes described work that was not done.
Those are fixed.

## 1. Findings

| #   | Finding                                                                                    | Severity | Fixed                      |
| --- | ------------------------------------------------------------------------------------------ | -------- | -------------------------- |
| 1   | `verify-ui` check 5 (`.js`-import guard) matched no code this repo can write               | High     | yes                        |
| 2   | `verify-ui` check 7 walked around by `cn()`, `clsx()`, a template literal or single quotes | High     | yes                        |
| 3   | Six of eleven bare `<section>`s were never converted; three documents said all eleven were | Medium   | claim corrected, code left |
| 4   | `LiveRegion` spread `{...rest}` after its own `aria-live`/`aria-atomic`                    | Medium   | yes                        |
| 5   | `docs/status.md` said `Section` wires `aria-labelledby` off `useId`; it never did          | Medium   | yes                        |
| 6   | `@vpay/ui` suite reported as 92 cases; it is 96 (97 after this review)                     | Low      | yes                        |
| 7   | `<code>` audit reported 18 across 6 files; it is 19 across 7                               | Low      | yes                        |
| 8   | `verify-ui` header said "Six things"; there are seven numbered checks over nine greps      | Low      | yes                        |
| 9   | justfile header said "Eleven invariants", `verify-migrations` pasted mid-sentence          | Low      | yes                        |
| 10  | The maintainer's "80% reduction" is not this branch's to claim                             | —        | reported, §4               |

### 1 — check 5 could not fail

`verify-ui`'s `.js`-suffixed-import guard is the regression guard for a
`next build` break that `docs/status.md` records as having happened **twice**.
It read:

```text
git grep -nE "from '\.{1,2}/[^']*\.js'" -- 'frontends/packages/ui/src'
```

Single quotes only. `.prettierrc` sets `"singleQuote": false` and `just fmt`
rewrites every import in the repository to double quotes, so the pattern
matched nothing this repository can produce. Measured, both directions:

| Mutation in `components/code/code.tsx` | `just verify-ui` |
| -------------------------------------- | ---------------- |
| `import { cn } from "../../cn.js";`    | **exit 0**       |
| `import { cn } from '../../cn.js';`    | exit 1           |

So the guard against the build break could not have caught either occurrence
of it. Both quote characters now; the double-quoted spelling is the recorded
mutation, and it exits 1.

### 2 — check 7 saw one spelling out of five

Check 7 (new on this branch: no daisyUI component class in an app) required a
double-quoted literal immediately after `className=`. Every one of these, in a
file under `frontends/apps/dashboard`, passed the whole recipe:

| Mutation                                  | `just verify-ui` as landed |
| ----------------------------------------- | -------------------------- |
| `className="btn btn-primary"`             | exit 1, correct            |
| `className={cn("btn", "btn-primary")}`    | **exit 0**                 |
| `className={` + backtick-template + `}`   | **exit 0**                 |
| `className={'btn btn-primary'}`           | **exit 0**                 |
| `className={clsx("card", "bg-base-100")}` | **exit 0**                 |

`cn` is exported from `@vpay/ui`'s own public `src/index.ts`, so the second row
is one import away from any app file — and it is the idiomatic way to write a
class in this codebase. A second Button written that way passes every gate,
which is the exact thing check 7 was added to stop.

It is two checks now. **7a** — no `className` written under `frontends/apps` at
all, outside `*.test.ts(x)`, `*.stories.tsx` and the one documented
`<body className="min-h-screen bg-base-100">`. It does not care what is inside
the attribute, so no spelling walks around it, and it is exactly the rule
`frontends/apps/dashboard/README.md` already claimed ("Zero `className`
strings … not one, under `app/` or `src/`, outside the tests"). **7b** — the
daisyUI component-class list, widened to read inside a `cn()`, a `clsx()`, a
template literal and a single-quoted string, so it still names the class in a
test or a story that 7a exempts.

Seven mutations run against the fixed recipe: all five above (all exit 1), a
plain `className="mt-2"` (7a fires — an app may not write a Tailwind utility
either), and `cn("btn-primary")` inside a `*.test.tsx` (7b fires where 7a is
exempt). Removing each returns exit 0.

### 3 — six bare `<section>`s remain, and three documents said otherwise

The audit found eleven bare, nameless `<section>` elements. The notes, the
dashboard flow doc and `docs/status.md` all said they became `Section`. Five
did — the dashboard's, in `payment-detail.tsx`. The other six are the
checkout's, in `screens.tsx` at lines 283, 450, 524, 568, 659 and 708, and they
are still bare and still nameless:

```text
$ git grep -n '<section' -- frontends/apps
frontends/apps/checkout/src/components/screens.tsx:283:    <section>
frontends/apps/checkout/src/components/screens.tsx:450:    <section>
frontends/apps/checkout/src/components/screens.tsx:524:    <section>
frontends/apps/checkout/src/components/screens.tsx:568:    <section>
frontends/apps/checkout/src/components/screens.tsx:659:    <section data-outcome={kind}>
frontends/apps/checkout/src/components/screens.tsx:708:    <section data-error-code={code}>
```

This is not a regression — it is what `master` has — and it raises no axe
violation, because `checkout-view.tsx` and `return-view.tsx` each wrap the page
in `<main>` and the `region` rule asks only that content sit inside _some_
landmark. What a payer using a screen reader still cannot do is jump to one by
name, which is the whole argument the branch makes for `Section`.

**The code was left alone and the claim was fixed**, deliberately. Each
checkout `<section>` heads itself with `ScreenHeading`, which owns
`data-screen`, `tabIndex` and the focus-on-screen-change behaviour the state
machine depends on; `Section` renders a `Heading` of its own from a required
`title` string. Reconciling those two is a change to the payer-facing app's
focus handling, which is a maintainer's decision, not a reviewer's drive-by.
It is now listed in the notes' §7 as not done.

### 4 — `LiveRegion` did not own its own attributes

```diff
-  return <div aria-live={politeness} aria-atomic="true" {...rest} />;
+  return <div {...rest} aria-live={politeness} aria-atomic="true" />;
```

The doc comment says `aria-atomic` "is not optional and so is not a prop: a
partial announcement of a payment outcome … is worse than none", and the test
comment says "no call site may turn this off". With the spread last, a call
site could: `Omit` stops a _typed_ call site, but a spread of a widened props
object does not go through that check, and the failure is silent — an outcome
that is never announced looks exactly like one that is. `live-region.test.tsx`
now pushes `aria-live="off"` and `aria-atomic="false"` in through a cast and
expects both ignored; restoring the old order fails that case.

### 5 — `Section` and `useId`

`docs/status.md` said `Section` "wires `aria-labelledby` off `useId`". It never
did and could not: `useId` is a hook, it would make `Section` a client
component, and `payment-detail.tsx`, which renders it, is a Server Component —
which is the reasoning the component's own doc comment gives for choosing
`aria-label` instead. The status row now says what the code does.

## 2. What held up under attack

Everything below was attacked and did not move.

- **The 200-line ceiling is real.** A 250-line file added under
  `frontends/packages/ui/src` and `git add`ed: `just verify-ui` exits **1**,
  naming the file and its length. Removed: exit 0. (Untracked, it passes —
  which is what the recipe says, and CI only ever sees tracked files.)
- **The checkout's live region still announces.** `aria-live` removed from
  `LiveRegion`: two cases in `@vpay/ui` and twelve in `checkout-view.test.tsx`
  fail, including "keeps a polite live region mounted on every screen, from
  first render".
- **`Section`'s accessible name is defended.** `aria-label={title}` removed:
  `section.test.tsx > is a region named by its own heading` fails. The
  _dashboard's_ a11y suite does not catch it — `<main>` already satisfies the
  `region` rule — so the package's own case is the only thing holding it. That
  is enough, but it is worth knowing which suite is load-bearing.
- **The failure tone is defended, exactly as the flow doc claims.**
  `error: "alert-error"` changed to `error: ""` in `alert/alert.variants.ts`:
  `checkout-view.test.tsx > does NOT render a failed payment in the neutral
tone a cancelled one beats` fails with
  `a failure must carry a tone: expected 'mt-4 alert' to contain 'alert-error'`.
- **`Link` really is server-safe, and the claim is checkable.** Base UI 1.8.0's
  `internals/useRenderElement.js` guards its one hook call behind
  `if (typeof document !== 'undefined')`. Both production `next build`s succeed
  and the dashboard's `/payments` route, a Server Component rendering `Link`,
  builds and pre-renders.
- **Tailwind still sees the classes through the new folder structure.** The
  risk of a folder-per-component move is that `@source './components'` stops
  finding a `cva` map and a class silently stops being emitted. Checked against
  the dashboard's compiled stylesheet: `break-all`, `cursor-pointer`,
  `table-zebra`, `badge-success`, `alert-error`, `link-hover`, `space-y-1`,
  `opacity-70`, `sr-only`, `max-w-md`, `font-mono` and `bg-base-200` are all
  present.
- **The axe suite has a negative control** that runs first and asserts the
  harness can report a violation at all. It discriminates.
- **The suite counts in the flow docs are right**: dashboard 178 in 19 files,
  checkout 507 in 24 files, both measured.

## 3. Are the primitives Base UI, or hand-rolled divs in a folder?

Nine of the 29 folders wrap a Base UI primitive. The rest are not evasions —
Base UI ships no `Card`, `Badge`, `Stack`, `Text`, `Table`, `Alert`, `Spinner`
or `Timeline`, and there is nothing to wrap.

| Wraps Base UI                                                                                            | Hand-rolled, because Base UI has no such primitive                                                                                                                                                                                |
| -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `button`, `checkbox`, `dialog`, `drawer`, `field`, `input`, `radio`, `select`, `link` (via `use-render`) | `alert`, `badge`, `card`, `code`, `data-list`, `empty-state`, `heading`, `list`, `live-region`, `logo`, `page-shell`, `pagination`, `section`, `spinner`, `stack`, `status-badge`, `table`, `text`, `timeline`, `visually-hidden` |

Every one of the nine is a genuine wrap, not an import for show: `select`
composes eight `BaseSelect.*` parts, `dialog` five, `drawer` six, `field` four.
Base UI primitives that exist and are **not** wrapped — `separator`, `toolbar`,
`progress`, `meter`, `tabs`, `tooltip`, `popover`, `menu`, `accordion`,
`switch`, `toast` and the rest — are named in the notes' §7 as a decision: no
screen renders one. That reasoning is sound but not applied evenly: `dialog`,
`drawer` and `radio` are three primitives **no app consumes today** (they
predate this branch), and the same "a component nothing consumes is scaffold
with a story attached" argument applies to them.

## 4. The 80% reduction, measured

The maintainer asked for "a min 80% reduce in ui components and css classes".
Class tokens written in shipping source, counted across `className` attributes
and `cva` maps alike:

| Tree                                     | `frontends/apps` | `@vpay/ui` | total |
| ---------------------------------------- | ---------------- | ---------- | ----- |
| `30fb8f1` — before the exp26 UI revamp   | **177**          | 32         | 209   |
| `6b1b7d8` — `master`, this branch's base | **2**            | 189        | 191   |
| `7a8786d` — this branch                  | **2**            | 175        | 177   |

**The 98.9% reduction in the apps is real, and it is exp26's, not exp53's.** It
was already on `master` before this branch started: `git grep 'className' --
frontends/apps` returns exactly one line on `master` and exactly one line here,
the checkout layout's `<body>`. There was nothing left for this branch to
remove. Repo-wide, this branch took 191 class tokens to 177 — a 7% reduction,
not 80%.

To the branch's credit, **it does not claim otherwise**: the notes' §4 says only
that the grep "returns one line", which is true and is a statement about the
tree rather than about this change. But nothing in the notes or `docs/status.md`
tells a reader that the 80% was already banked, so a reader counting this branch
against the directive would credit it with work exp26 did. That is what this
section is for.

What this branch actually delivers against the directive is different and worth
having: 29 component folders with a `cva` map, a test, a story and an `index.ts`
each; a 200-line ceiling that fires; eight new primitives from a count of what
the apps render rather than from a design-system checklist; and five pieces of
markup that were accessibility defects (nameless sections, unstyled anchors, a
duplicated live region, hand-repeated `<th scope="row">`, bare `<code>`) turned
into one implementation each.

Line counts, for honesty about relocation:

| Shipping LoC (no tests, no stories) | `master` | this branch |
| ----------------------------------- | -------- | ----------- |
| `frontends/apps/checkout`           | 7591     | 7591        |
| `frontends/apps/dashboard`          | 4748     | 4649        |
| `frontends/packages/ui/src`         | 1226     | 2010        |

Total frontend shipping source is **up 685 lines**. Three dashboard components
(`pager`, `detail-timeline`, `empty-state`, 126 lines between them) became three
`@vpay/ui` primitives (153 lines) plus two app-side adapters (`payments-pager`,
`timeline-gap`, 79 lines). That is relocation plus growth — but it is the
relocation the maintainer asked for, since a primitive living in one app is a
primitive the other app has to copy, and a large part of the growth is doc
comments that did not exist before. It is worth naming rather than presenting
the dashboard's −99 on its own.

## 5. "Both for checkout but also for dashboard"

The directive says the package serves both apps. It does — both import from
`@vpay/ui`, and neither writes a class. But of the **eight primitives this
branch added**, the checkout consumes exactly **one**:

| New primitive                                                                 | checkout | dashboard |
| ----------------------------------------------------------------------------- | -------- | --------- |
| `LiveRegion`                                                                  | yes      | no        |
| `Code`, `DataList`, `EmptyState`, `Link`, `Pagination`, `Section`, `Timeline` | no       | yes       |

This is defensible: the checkout has no table, no cursor paging, no event
timeline and no identifiers to render as chips. It is not "half the job" in the
sense the brief worried about — the checkout was already fully on `@vpay/ui`
from exp26 and consumes 22 primitives in total. But the _new_ work in this
branch is dashboard work, and the shared-package framing should not obscure
that.

## 6. Gates

Run on the review's final head, exit codes read from files rather than from a
banner, under the Node version `.nvmrc` pins (`v22.23.2`, not the host's
v24.20.0) so the toolchain matches CI's.

| Gate                                  | Exit | Numbers                                           |
| ------------------------------------- | ---- | ------------------------------------------------- |
| `just verify`                         | 0    | "the twelve gates above passed"                   |
| `just lint-web`                       | 0    | `pnpm -r typecheck` + `pnpm -r lint`, 15 packages |
| `just test-web`                       | 0    | see below                                         |
| `pnpm --filter @vpay/dashboard build` | 0    | 8 routes, 3 pre-rendered                          |
| `pnpm --filter @vpay/checkout build`  | 0    | 6 routes plus middleware                          |

Web suites: `@vpay/ui` **97** (was 96 — this review added one),
`@vpay/checkout` 507, `@vpay/dashboard` 178, `examples/shop` 102, `sdks/nodejs`
208, `sdks/stripe-js` 146, `@vpay/config` 63, `@vpay/tokens` 8, `api-client` 4.
**Zero skipped, zero ignored** in any of them.

## 7. What this review did NOT check

- **No real browser, and therefore no rendered contrast and no real keyboard
  run.** Everything here is jsdom and a compiled stylesheet. The branch's own
  §7 says the same and the gap is unchanged: `cypress-axe` (plan §7 row 6) is
  still built by nobody. Keyboard behaviour is covered only as far as the unit
  tests go — `Select` (ArrowDown opens and moves the highlight), `Dialog` and
  `Drawer` (Escape), `Checkbox` (which documents, honestly, that jsdom does not
  simulate a native button's Space/Enter and does not pretend otherwise).
  Nothing here proves a tab order.
- **No screenshots.** Nothing in this review is claimed on the strength of one.
- **`KitchenSink` covers 33 of the 44 exported symbols.** `Logo`,
  `VisuallyHidden`, `CheckboxLabel` and `FieldError` are not in it, so the axe
  suite says nothing about them. Each has its own unit test; none has an axe
  case.
- **Storybook was not built.** `pnpm --filter @vpay/ui build-storybook` was not
  run, so "every story renders" is the implementer's claim and is not
  re-measured here.
- **`examples/shop` was not looked at.** It has ~150 raw class strings and no
  `@vpay/ui` dependency, deliberately, and is out of every gate's scope.
- **7a's exemption list is minimal by design.** A README under `frontends/apps`
  that quotes a `className="…"` in a fenced example would fail it. None does
  today; the answer when one needs to is a named exemption with a reason, which
  is the pattern every other check in the recipe uses.

## 8. `just ci`

The implementer's own run (notes §6b) exited 100 at `test-rust` on a Docker
daemon saturated by another worktree, and that branch's "the Rust is
byte-identical to master" claim was re-checked rather than accepted:
`git diff --name-only origin/master...HEAD` outside `frontends/` and `docs/` is
exactly one file, `justfile`, and `git diff --stat origin/master...HEAD --
backends Cargo.toml Cargo.lock rust-toolchain.toml schemas clippy.toml
deny.toml` is empty.
