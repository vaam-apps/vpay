# exp26 Lane C — the sabotage review

Reviewed 2026-09-07, against `docs/plans/2026-09-07-ui-revamp.md` §4.3, §5
Lane C, §6.3, §6.4 and §7. The lane's own account is
[lane-c.md](lane-c.md); this document is the second pair of eyes on it and
supersedes it where the two disagree.

**Rebased first.** Lane C was written on `177645e`, Lane A's implementation
head. Lane A's own review landed seven more commits, so this branch was
rebased onto **`08d9b8e`** before anything else — clean, no conflicts, five
Lane C commits replayed unchanged. Two consequences, both material to Lane
C's report:

- The `@vpay/ui` `./cn.js` resolution defect that stopped `frontends/apps/
  dashboard`'s `next build` — the reason Lane C could not run `just test-e2e`
  — **is fixed on `08d9b8e`** (`e2a0a09`, plus a `verify-ui` check that
  re-runs the original failure). `just test-e2e` therefore runs here, and did.
- `just lint-web`'s failure on `frontends/apps/checkout/tailwind.config.ts`,
  which Lane C recorded as pre-existing, **is caused by Lane C**. Finding 6
  has the reproduction. This is the one thing in this review that a reader
  should not skim.

Severity vocabulary, as the brief sets it: gate-hole / correctness /
rule-break / misleading-claim / nit.

---

## The counting script, re-run rather than taken on trust

`exp26-plan-count.sh --json`, run by this review on a `git archive` of the
base and on the branch head. **Every figure in `lane-c.md` reproduces
exactly**:

| `@vpay-examples/shop` | `177645e` (base) | Lane C head `892f521` | after this review |
|---|---|---|---|
| `styling_files` | 11 | 16 | 16 |
| `classname_sites` | 41 | 128 | 130 |
| `class_tokens_distinct` | 15 | 93 | 90 |
| `class_tokens_total` | 46 | 300 | 295 |
| `css_lines` | **229** | **5** | **5** |
| `inline_styles` | **24** | **0** | **0** |

`class_tokens_distinct` drops 93 → 90 across this review because the three
`badge-{tone}` strings moved out of a `className` template literal and into a
plain lookup object, which is the script's own documented blind spot (plan
§2). That is a *smaller* number for a *better* file, and it is stated here
rather than claimed as a reduction.

### The `styling_files` argument — Lane C is right on the text, and it is still not the whole answer

Lane C argues that the brief's per-lane target (`styling files 11 → ≤ 2`) is
not in the plan and is a misapplication of an aggregate. **Checked against
the plan, and the argument holds on the text:**

- Plan §5 Lane C's *Acceptance* paragraph names `css_lines 229 → ≤ 8` and
  `inline_styles 24 → 0`, plus the `data-testid`s and `lint`. There is no
  `styling_files` figure in it. Lanes A, B and D each get one; Lane C does
  not.
- Plan §2's `styling_files ≤ 3` is on the row labelled **OUTSIDE
  `@vpay/ui`**, an aggregate over checkout + dashboard + shop, read once at
  §7 row 1 on the final merged head.

So `≤ 2` was not a Lane C acceptance criterion, and Lane C's two acceptance
criteria are both met (229 → 5, 24 → 0). **That verdict does not carry over
to §7 row 1, and Lane C's notes stop one step short of saying so.** §7 row 1
requires "every target in §2's table met". The shop alone now holds 16 of the
23 `styling_files` outside `@vpay/ui`; even if Lane B drives checkout's 5 and
Lane D the dashboard's 2 to zero, the aggregate lands at **16, not ≤ 3**. The
whole-revamp gate as written cannot pass with this lane as designed.

That is not a defect in Lane C's code. It is a genuine conflict between two
things the plan decides in different sections — D1 forbids the shop the
`@vpay/ui` layer that is *how* checkout and the dashboard will concentrate
their class decisions, and §2 then measures the shop as if it had one. It is
**surfaced here, not resolved**: the choice between (a) restating §2's
`styling_files` target as "outside `@vpay/ui` and outside `examples/shop`",
(b) accepting a higher number with the reason recorded, and (c) reopening D1,
is the maintainer's, and picking a defensible default would be taking a
decision the plan reserved. Lane C's own count is honest either way.

---

## Findings

### 1. gate-hole + correctness — the badge tone classes were in the stylesheet by accident

`OrderStatusBadge` and `TestNumbersPanel` rendered ``  `badge badge-${tone}`  ``.
Tailwind 4 decides what CSS to generate by **scanning source text**, so
`badge-success`, `badge-warning` and `badge-error` appear nowhere in the
shop's shipping source — and every order page's badge was coloured anyway,
because `src/components/order-summary.test.tsx` carries the three strings as
expected values and Tailwind's automatic content detection scans `.test.tsx`
files too. The root `.dockerignore` does not exclude test files, so the
shipped image inherited the same accident.

Measured, twice, the same way:

| build | `.badge` | `.badge-success` | `.badge-warning` | `.badge-error` |
|---|---|---|---|---|
| Lane C head, as delivered | 5 | 1 | 1 | 1 |
| Lane C head, `order-summary.test.tsx` moved aside | 2 | **0** | **0** | **0** |
| after the fix, `order-summary.test.tsx` moved aside | 5 | 1 | 1 | 1 |

(`pnpm --filter @vpay-examples/shop build`, then `grep -oF` over
`.next/static/chunks/*.css`.)

This is exactly the silent failure plan §6.3 calls "the highest-risk item in
the plan, because it does not error" and §6.4 says must be checked **on the
built CSS**. Lane C did check the built CSS — its notes record the classes as
present — and drew the wrong conclusion from a true observation, because it
never asked *why* they were present. Recorded as a misleading claim in
`docs/status.md` and `docs/flows/hosted-checkout.md` as well, both corrected.

**Fixed** (`3b91737`): both maps hold the complete class string per case.
**Gated** (same commit): `src/testing/no-dynamic-class-names.test.ts` fails if
any `className` in the shop's shipping source becomes a template literal with
an interpolation. Restoring the old shape fails it
(`expected [ Array(1) ] to deeply equal []`); its second case pins the regex
against the shapes it must and must not reject, and its own examples use
nonsense tone words so the guard cannot repeat the defect it exists to catch.

### 2. gate-hole — the class-string rules ran nowhere in this package

Lane A's review wired `eslint-plugin-better-tailwindcss` into `@vpay/config`
behind a `tailwind` flag each app flips in the commit that moves it to
Tailwind 4. This package moved and the flag was never flipped, so plan §7's
"class-string rules, concretely" — one line per class attribute, deterministic
order, no unknown class — were inert here.

**Fixed** (`a291359`), with both directions proven: a class attribute wrapped
over two lines now fails `enforce-consistent-line-wrapping`, and a
`form-controlx` fails `no-unknown-classes` — the rule §6.3 wanted for the
daisyUI 4 names daisyUI 5 removed. The 13 `enforce-consistent-class-order`
errors it surfaced were fixed by the rule's own autofix; no class added or
removed.

The rules read `@vpay/ui`'s `styles.css` as their class universe. That does
not reopen D1 — D1 governs what the shop *ships*, `src/` still imports neither
`@vpay/ui` nor `@vpay/tokens`, and this is a dev-only lint preset in the same
config file that has taken this package's ESLint rules from `@vpay/config`
since before the revamp. Said so beside the flag.

### 3. correctness — three daisyUI 5 layouts that parse, apply, and do the wrong thing

Found by opening the shop under `bumblebee` and measuring boxes. Fixed in
`ddec874`; the commit message carries the before/after geometry.

- **The e-mail label sat beside its input.** daisyUI 5's `.label` is an
  `inline-flex` for a label *inside* a `.fieldset`; bare above an `<input>` it
  shares the line and the `mb-1` on it is inert. Before: label `x=16 y=347`,
  input `x=178.7 y=341`. The field is now in a `fieldset`, which is the shape
  daisyUI's own upgrade guide gives as the replacement for `form-control` +
  `label-text`. After: label `y=278`, input `y=302`.
- **`OrderFailureNotice` put its heading and its detail side by side.**
  `.alert` is `display: grid; grid-auto-flow: column`, so the `flex-col
  items-start` on it did nothing: both children on one row, in a 290px and a
  498px column. One wrapping `<div>` is one grid cell; they stack, left
  aligned, which `alert-vertical` would not be (it centres).
- **The test-numbers Orange caveat did the same**, squeezing "Read this before
  you try them." into a 132px column beside the sentence continuing it
  (`grid-template-columns: 132.797px 655.203px`; after: `804px`).

No `data-testid`, `id`, `role` or copy changed.

### 4. Surfaced, not taken — the shop's `cancelled` badge is `error` while D4 made `canceled` `warning`

`order-summary.tsx` maps the shop's `cancelled` order status to `badge-error`,
with a comment arguing that a buyer reads `failed` and `cancelled` the same
way. Decision **D4** (plan §9) moved `canceled` to `warning` in
`@vpay/tokens`' `checkoutOutcomeTone`, on the reasoning that a cancellation is
the payer's own action.

The two are not obviously the same thing — D4 is scoped to
`checkoutOutcomeTone`, the shop imports it (or anything else from that
package) nowhere by design, and the shop's `cancelled` is reached by the
*merchant* cancelling its PaymentIntent, not by the payer, and is unreachable
today at all (`payment_intent.canceled` is emitted by nothing; the panel says
so). So D4's rationale does not transfer cleanly.

**Left as it is.** A payer nonetheless sees amber for a cancellation on vpay's
page and red for one on the merchant's, which is the kind of split D4 was
taken to close, so it is written down here for the maintainer rather than
decided by a reviewer. Not a defect; a decision.

### 5. nit — coverage the two new suites do not reach, stated rather than implied

The two suites are real and both decisive mutations fail under them
(table below). What they do **not** cover, checked by grepping every
`data-testid` in the package against every test and spec in the repository:

`order-failure`, `order-failure-{title,detail,code,message}`,
`configured-mode`, `mode-popup`, `order-retry`, `order-cancel`,
`order-action-{error,note}`, `failed-message`, `embedded-{mount,error}`,
`qty-*`, `cart-{total,empty}`, `email-why`, `checkout-{error,note}`,
`order-{id,email,payment-intent,checkout-session,total}` and `test-number-*`
are referenced by **no test and no spec**. That is pre-existing — the base is
the same — and it is not this lane's to close. It is worth stating because
this lane rewrote the markup around all of them, and the only reason that was
safe is checked separately below.

The three-mode switch has no unit test; `shop-hosted.cy.ts` and
`shop-embedded.cy.ts` drive `mode-hosted` and `mode-embedded`, and `mode-popup`
is driven by nothing anywhere (`docs/status.md` already carries "no test
anywhere opens a real popup"). The README cross-check from #64
(`src/lib/test-numbers.test.ts`, 9 cases) still exists and passes.

### 6. gate-hole + misleading-claim — Lane C breaks `just lint-web`, and the "pre-existing" reproduction was invalid

`just lint-web` — which is in `just ci` — fails on this branch with exactly
one error:

```
frontends/apps/checkout/tailwind.config.ts(12,13): error TS2322:
  Type 'PluginWithConfig' is not assignable to type 'PluginCreator | ...'
  Property 'prefix' is missing in type tailwindcss@3.4.19's PluginAPI
  but required in type tailwindcss@4.3.3's
```

Lane C's notes call this pre-existing and say it was "confirmed by
reproducing on the unmodified Lane A base (`git stash`, re-run, same
failure, `git stash pop`)". **`git stash` does not uninstall anything.** The
Tailwind 4 tree was still on disk for that re-run, so the experiment could
not have produced any other answer.

Reproduced properly here — revert the two manifest files this lane changes,
reinstall, run the gate:

| tree | `node_modules/.pnpm/node_modules/tailwindcss` | `pnpm --filter @vpay/checkout typecheck` |
|---|---|---|
| `08d9b8e` + Lane C's `examples/shop/package.json` and `pnpm-lock.yaml` | **4.3.3** | **exit 2**, the TS2322 above |
| `08d9b8e`'s own `examples/shop/package.json` and `pnpm-lock.yaml`, reinstalled with `--frozen-lockfile` | **3.4.19** | **exit 0** |

The mechanism, since "a dependency change broke a typecheck two packages
away" deserves better than a shrug. `daisyui@4.12.24`'s own type
declaration is `import type plugin from "tailwindcss/plugin"`, and that
package directory has no `tailwindcss` of its own, so TypeScript walks up to
the **single** copy pnpm hoists into `node_modules/.pnpm/node_modules/`.
Which copy that is, is a property of the whole workspace: with `@vpay/ui`
alone on Tailwind 4 it is 3.4.19, and adding `examples/shop` flips it to
4.3.3. Checkout's `tailwind.config.ts` then type-checks a Tailwind 3
`satisfies Config` against a Tailwind 4 `PluginAPI` and cannot pass.

**Not fixed here, deliberately, and the reason is scope rather than
difficulty.** The failing file is `frontends/apps/checkout/tailwind.config.ts`
— Lane B's, and a file plan §4.4 **deletes**, because Tailwind 4 has no
`tailwind.config.ts` at all. Measured rather than assumed: with that one file
moved aside, `pnpm --filter @vpay/checkout typecheck` exits 0 on this head.
So Lane B's migration closes this by construction, and the alternatives open
to Lane C — editing another lane's file, or an `.npmrc` hoist rule with
repo-wide blast radius — are both worse than saying so.

**What this means for merge order, plainly: `just ci` is red between Lane C
landing and Lane B landing.** Either they land together, or Lane C waits.
That is a decision for whoever sequences the four lanes; it is surfaced here
because Lane C's own notes said the opposite and nothing downstream would
have caught it — `just lint-web` never runs a `next build`, and every gate
Lane C did run is green.

### Checked and clean

- **Every `data-testid` and every `id` is byte-identical to the base** — 40
  and 40, extracted from both trees and diffed, empty both ways.
  `#vpay-embedded-checkout` included. This is what made rewriting the markup
  around untested selectors safe.
- **D1**: nothing under `examples/shop/src` imports `@vpay/ui`,
  `@vpay/tokens` or `@vpay/config`; `globals.css` is one `@import`, one
  `@plugin` block and a comment; zero `style={{`; zero `cva(`; zero
  `!important`; zero palette colours.
- **daisyUI 4 removals**: zero matches in `examples/shop` for any of the nine
  names `verify-ui`'s check 2 lists. The recipe is still red on the whole
  tree, for Lane B's `screens.tsx` and nothing else.
- **`@source`**: Lane C's finding stands. The classes above were generated
  with no `@source` line in `globals.css`, from a real `next build` — the
  automatic detection reaches `examples/shop/src` because it is not behind a
  workspace symlink. Confirmed by this review's own builds. (What automatic
  detection also reaches, and should not be relied on, is finding 1.)
- **`role`/`aria`**: `role="alert"` 4 → 10, `role="status"` 3 → 3,
  `aria-live` and `aria-describedby` unchanged. Nothing lost.
- **The image**: `docker build` green, and the entrypoint's `zen migrate
  deploy` applied all three migrations from empty in the container
  (`20260904091557_init`, `20260904091600_seed_catalogue`,
  `20260906120000_optional_email_and_failure_columns`), which is what the
  shop reporting `Healthy` on `/healthz` rests on.
- **Mobile**: 375×812, `document.scrollWidth === clientWidth` on `/cart` — no
  horizontal overflow. Focus is visible: `outline: solid 2px` with a 2px
  offset on `order-retry`.

---

## Mutations, all four run by this review

| mutation | expected | observed |
|---|---|---|
| `OrderStatusBadge` renders one tone for every status (`badge badge-neutral`) | fail | **fails** — `expected 'badge-neutral' to be 'badge-warning'` |
| `TestNumbersPanel`'s caveat loses `role="alert"` | fail | **fails** — `Expected the element to have attribute: role="alert" / Received: null` |
| a `data-testid` is dropped from the cart table (`cart-table`) | fail `just test-e2e` | **fails** — image rebuilt with it removed, `shop-hosted.cy.ts` `0 passing, 3 failing`, `Expected to find element: [data-testid="cart-table"], but never found it` |
| `className` goes back to `` `badge badge-${TONE[status]}` `` | fail | **fails** — `no-dynamic-class-names.test.ts`, `expected [ Array(1) ] to deeply equal []` (new gate, finding 1) |
| a class attribute wrapped over two lines | fail `lint` | **fails** — `enforce-consistent-line-wrapping` (new gate, finding 2) |
| an unknown class (`form-controlx`) | fail `lint` | **fails** — `no-unknown-classes` (new gate, finding 2) |

The third was rebuilt into a real image and driven through a real browser, not
reasoned about: Lane C left it for the reviewer because its own `just
test-e2e` was blocked, and it is no longer blocked.

---

## Gates, recipe by recipe, on the final head

Run on `08d9b8e..HEAD` under the `.nvmrc` Node (22.23.2), `pnpm install
--frozen-lockfile` first. Exit codes read from a file.

| recipe | result | evidence |
|---|---|---|
| `pnpm install --frozen-lockfile` | ✅ | exit 0 |
| `pnpm --filter @vpay-examples/shop typecheck` | ✅ | exit 0 |
| `pnpm --filter @vpay-examples/shop lint` | ✅ | exit 0, ESLint (now including the class-string rules) + `prettier --check` |
| `pnpm --filter @vpay-examples/shop test` | ✅ | **102 cases, 12 files, 0 skipped** (Lane C left 100; this review adds the 2-case dynamic-class guard) |
| `just lint-web` | 🔴 | exit 1 — **caused by this lane**, see finding 6. Exactly one error, `frontends/apps/checkout/tailwind.config.ts(12,13): TS2322` |
| `just test-web` | ✅ | exit 0, every package: `@vpay/ui` 60, `@vpay/checkout` 448, `examples/shop` **102**, the two SDKs 190 and 146, `api-client` 4 — 0 failed, 0 skipped |
| `just verify-ui` | 🔴 | exit 1, on `frontends/apps/checkout/src/components/screens.tsx`'s `form-control`/`label-text` — Lane B's unmigrated file, the same two lines Lane A's own review records. Green scoped to `examples/shop`: all five checks, zero matches. One own goal found and fixed here: a JSX comment this review added *named* those two classes in prose, and `verify-ui` is a grep that reads comments, so the comment was reworded rather than a path exemption added |
| `just verify-npm-scope` | ✅ | exit 0 |
| `just verify-links` | ✅ | exit 0 — 899 links across 163 tracked markdown files |
| `just verify-status` | ✅ | exit 0 — 1 declared unimplemented item, unchanged |
| `docker build` of the shop | ✅ | built four times over this review (three source changes plus the mutation), every one green |
| **`just test-e2e`** | ✅ | the plan's own §7 row 3 recipe, run to completion on the final head, `demo_project=exp26c-review`, ports 18501–18505: **11 tests, 11 passing, 0 failing, 0 pending, 0 skipped** — `checkout.cy.ts` 1/1, `dashboard.cy.ts` 3/3, `shop-hosted.cy.ts` 3/3, `shop-embedded.cy.ts` 4/4 (framed pass). Exit code read from a file, `TEST_E2E2=0`. Run twice, and **the first attempt failed** — recorded rather than dropped: `wiremock-orange is unhealthy`, a container this review had left up from an earlier stack while `gen-demo-keys` rewrote the mappings directory under it, alongside `EAI_AGAIN` registry timeouts during the image builds. `docker compose down -v` first, then the recipe from nothing, green. An environment fault, and the reason it can be called one is that it moved when the environment did and not when the code did |

`just test-e2e` is the headline: Lane C could not run it and proved its two
specs by hand instead. On this head it runs as written, all four images build,
and all eleven tests pass — so the substitute recipe, and the caveat attached
to it in `docs/status.md`, are both retired.

### The shop, looked at

Opened under `bumblebee` in a real browser on the running stack and driven
through a real payment (MTN `237600000101`, `insufficient_funds`): catalogue,
cart, checkout, vpay's page, the shop's `cancel_url` and the order page. Three
defects came out of that and are finding 3; everything else read correctly —
the test-numbers badges carry their tones, the runbook `dl` shows the failure
code and what the rail said, the cancelled page says in as many words that
`cancelled` is unreachable, and nothing is clipped or overflows at 375px.

## What this review did NOT do

- **Did not reopen D1, D4 or the `styling_files` conflict.** Findings 4 and
  the `styling_files` verdict are written down for the maintainer.
- **Did not add coverage for the untested selectors in finding 5.** They are
  pre-existing and out of this lane's scope; closing them is a decision about
  where the shop's test budget goes.
- **Did not run `just ci`** — it is the Rust workspace plus the web gates, and
  nothing under `backends/` is touched by this branch. The web half is in the
  table above.
- **Did not run axe** (plan §7 rows 5 and 6). No axe wiring reaches
  `examples/shop`; that is Lane A's gate over `@vpay/ui` and the checkout
  screens, and this lane adds none.
- **Did not regenerate the exp21 screenshots** (§7 row 10) — checkout's, not
  the shop's.
