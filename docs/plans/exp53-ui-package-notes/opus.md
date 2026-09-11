# exp53 — `@vpay/ui` becomes the one primitive set both apps compose from

Branch `claude/exp53-ui-package`, base `6b1b7d8`. Opus implementation, one
pass. What follows is what was built, what the audit found, what is enforced,
and — at the end — what was **not** done.

## The directive

> "Using refine, we can also define our own components. And that is what we
> will do. We'll go base-ui + daisyui. I expect from you a @vpay/ui package
> with all UI primitives that we'll use both for checkout but also for
> dashboard. Therefore, I expect a real good structure with components in
> folders, the least class possible, base-ui components as basis, cva for
> structured variants, tw-merge + clsx for applying css classes, and daisyui
> to build our design system => we MUST have components folder with very few
> lines, max 200 LoC, including imports and comments."
> — the maintainer, 2026-09-11

## 1. A folder per component

`src/components/` was sixteen flat files. `layout.tsx` alone was **221 lines**
and held five primitives; the other fifteen each held a component and its `cva`
map in one file, and a component's test and story sat beside them in the same
flat directory as everybody else's.

It is **28 folders** now. Each holds the component, its `cva` variant map in
its own file, its test, its story, and an `index.ts`:

```
src/components/button/
  button.tsx           41   the component, and nothing that is not a component
  button.variants.ts   23   the cva map
  button.test.tsx      50
  button.stories.tsx   38
  index.ts              2   re-exports both
```

`layout.tsx`'s five primitives — and the two the plan's own count forgot —
became seven folders: `stack`, `text`, `visually-hidden`, `logo`, `heading`,
`list`, `page-shell`. Each now has a test file and a story of its own rather
than a seventh of one.

Splitting the `cva` map out is what keeps a component file small without
anything being squeezed. `alert.tsx` is 28 lines because its twenty lines of
variant map are in `alert.variants.ts`, where a reader looking for "what
colours can an alert be" finds them without reading JSX.

The package's public surface is `src/index.ts`, and it lost nothing: every
symbol both apps import still resolves (proved by `pnpm -r typecheck` and by
two production `next build`s, below). The `cva` maps are exported too —
`button`, `badge`, `text`, `stack`, `code`, `link`, `alert`, `card`,
`checkbox`, `input`, `radio`, `spinner`, `table`, `selectTrigger` — because a
consumer that wants to know what `variant="danger"` resolves to should be able
to read it rather than guess.

## 2. The 200-line limit, enforced

`just verify-ui` check 6, new. No **tracked** file under
`frontends/packages/ui/src` may exceed 200 lines, imports and comments
included. Every extension, not only `.tsx`: a 300-line `.css` or a 300-line
test is the same problem as a 300-line component.

**Mutation, run:** filler appended to
`src/components/button/button.tsx` until it reached 201 lines →
`just verify-ui` exits **1**, printing
`verify-ui: frontends/packages/ui/src/components/button/button.tsx is 201
lines, over the 200-line limit`. Filler removed → exit **0**.

The gate caught one file on its first run: `theme-contrast.test.ts`, 232
lines. Its colour maths (oklch → linear sRGB, WCAG luminance, contrast ratio,
and the compiled sheet's resolved `--color-*` values) moved to
`src/testing/contrast.ts`; the test is now the file that says what a
measurement has to clear, and is 150 lines. Nothing it asserts changed.

The longest file in the package is now `src/testing/kitchen-sink.tsx` at 158;
the longest component file is `select/select.tsx` at 101.

That same restructure broke a check that had been silently vacuous —
`theme-contrast.test.ts`'s scan for a component starting to use an unmeasured
tone did a **non-recursive** `readdirSync` over `components/`, which in a tree
of folders returns directory names, reads nothing, and passes on an empty
string. It is recursive now and includes `*.variants.ts`, which is where every
`cva` map lives.

## 3. The audit, and what it made me build

I counted what both apps actually render, rather than working from a list of
components a design system usually has.

| Found in the apps                                             | Count                        | Built                        |
| ------------------------------------------------------------- | ---------------------------- | ---------------------------- |
| Bare `<code>`                                                 | 18, across 6 dashboard files | `Code`                       |
| Unstyled `next/link` anchors                                  | 5                            | `Link` (Base UI `useRender`) |
| `<div aria-live="polite" aria-atomic="true">`, byte-identical | 2                            | `LiveRegion`                 |
| Bare `<section>`, none with an accessible name                | 11                           | `Section`                    |
| `<table><tbody><tr><th scope="row">` key/value markup         | 2 blocks, 20 rows            | `DataList` / `DataListRow`   |
| `EmptyState` — a primitive living in an app                   | 1                            | moved into `@vpay/ui`        |
| `Pager` — a primitive living in an app                        | 1                            | `Pagination`                 |
| `DetailTimeline` — a primitive living in an app               | 1                            | `Timeline`                   |

Three things the audit found that are worth naming individually.

**A `<section>` with no accessible name is not a landmark.** axe-core's
`region` rule does not count one, and a heading merely sitting inside a
`<section>` does not name it. All eleven of the dashboard's were `<div>`s with
extra steps. `Section` takes the heading as a **required** prop and wires it
with `aria-labelledby` off `useId`, so the labelled form is the only form;
`section.test.tsx` asserts `getByRole('region', { name: 'Charge' })`, which
only matches when the wiring is there.

**Three of the dashboard's own components were primitives.** `EmptyState`,
`Pager` and `DetailTimeline` had nothing dashboard-specific in them — the
checkout could not have used one without copying it, which is the definition
of the thing this package exists to prevent. They are `@vpay/ui`'s now.

**`Pagination` takes elements, not hrefs.** This package cannot depend on
`next/link`: it is consumed by two Next apps and by Storybook, and only two of
those three have a router. So `Link` uses Base UI's `useRender` and
`Pagination` takes the two navigating elements from its caller. The dashboard's
`PaymentsPager` is what is left in the app — the `next/link` elements and
nothing else.

`payment-detail.tsx` fell from 264 lines to 207 with nothing removed from the
screen.

## 4. The least class possible

`git grep 'className=' -- frontends/apps` returns **one** line in shipping
source: `frontends/apps/checkout/app/layout.tsx`'s
`<body className="min-h-screen bg-base-100">` — layout plus a daisyUI theme
token on the document itself, which no component owns.

`verify-ui` gained a second check for this, number 7: **no daisyUI COMPONENT
class anywhere under `frontends/apps`**. Checks 1 and 4 catch a palette colour
and a stray `cva`; neither sees `className="btn btn-primary"`, which is how a
second Button gets written with every gate green. Measured: that exact string
in the dashboard's `nav.tsx` passed checks 1 through 6.

**Mutations, both run:**

- `className="btn btn-primary"` on a `Stack` in
  `dashboard/src/components/signed-in-bar.tsx` → `verify-ui` exits **1** on
  check 7. Removed → exit **0**.
- `className="bg-red-500"` in the same place → exits **1** on check 1.
  Removed → exit **0**.

`examples/shop` is deliberately **out of that scope**, and the recipe says why
rather than leaving it to be re-derived: it does not depend on `@vpay/ui` at
all — its `package.json` takes `@vpay/config` and nothing else from this
repository — because it is a _merchant's_ storefront. Holding a third party's
demo to vpay's design system would mean shipping a demo no real merchant
would have.

## 5. A variant a page uses is a variant a page's test defends

**Mutation, run:** `error: "alert-error"` changed to `error: ""` in
`alert/alert.variants.ts` →
`checkout-view.test.tsx > does NOT render a failed payment in the neutral tone
a cancelled one beats` **fails** with
`a failure must carry a tone: expected 'mt-4 alert' to contain 'alert-error'`.
Restored → 116/116 in that file.

That case is the one that matters: the defect it was written for was a payer
whose payment **failed** reading a grey box while a payer who cancelled read a
red one.

## 6. Numbers

Web suites, before → after:

| Package                                  | Before         | After          |
| ---------------------------------------- | -------------- | -------------- |
| `@vpay/ui`                               | 74 (18 files)  | 92 (32 files)  |
| `@vpay/dashboard`                        | 182 (21 files) | 178 (19 files) |
| `@vpay/checkout`                         | 507 (24 files) | 507 (24 files) |
| `@vpay-examples/shop`                    | 102            | 102            |
| `@vpay/config` / `tokens` / `api-client` | 63 / 8 / 4     | 63 / 8 / 4     |
| `@vaam-apps/vpay-sdk` / `vpay-stripe-js` | 208 / 146      | 208 / 146      |

The dashboard's four are not a loss of coverage and are accounted for
individually: `empty-state.test.tsx` (1 case) and `detail-timeline.test.tsx`
(2) are deleted because both components moved to `@vpay/ui`, where they have
**3** and **2** cases respectively; the app's a11y suite dropped its timeline
and empty-state cases (2) for the same reason and gained one for the pager
composition it did not have before. The package's +18 covers the eight new
primitives.

## 7. What I did NOT do

- **No sortable table headers, and deliberately.** The brief's illustrative
  list named them. `GET /dash/v1/payment_intents` has no sort parameter —
  cursor paging over `created DESC` is the whole of it — so a sortable header
  would either sort one page of rows against the impression that it sorted the
  result set, or be a control that does nothing. Either is the failure mode
  `CLAUDE.md` names. It is a `/dash/v1` change first.
- **No `Toast`, `Tabs`, `Skeleton`, `Tooltip`, `Progress`, `Accordion`,
  `DateRangeField`, `CopyToClipboard` or `ConfirmDialog`.** Base UI ships a
  primitive for most of them and each would be perhaps forty lines — but no
  screen in either app renders one today, and a component nothing consumes is
  scaffold with a story attached. Named here so the list is a decision rather
  than an oversight. `DateRangeField` is the closest to earning its place:
  `payments-filters.tsx` has two `<Input type="date">` that are conceptually
  one control, at one call site.
- **`examples/shop` is untouched.** ~150 raw class strings, no `@vpay/ui`
  dependency; see §4 for why that is the right answer rather than a gap.
- **The `<body className="min-h-screen bg-base-100">` in the checkout's
  layout stays.** It is one line, both utilities are permitted (layout, and a
  theme token), and the alternatives — hand-written CSS in an app whose own
  comment says it has only one rule, or a `@vpay/ui` component that renders
  `<body>` — are both worse than the thing they would replace.
- **No visual check in a real browser.** Storybook builds and every story
  renders under bumblebee with the a11y addon configured, and the axe pass is
  jsdom's structural rules. Contrast is still measured from the compiled
  stylesheet (`theme-contrast.test.ts`) and not from a rendered glyph; plan §7
  row 6's `cypress-axe` is still built by nobody, and this branch did not
  build it.
- **No screenshots committed.** Previous lanes committed before/after PNGs;
  this pass did not take any, so nothing here is claimed on the strength of
  one.
