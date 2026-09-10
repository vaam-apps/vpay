# exp26 Lane D — sabotage review

Reviewer's notes on `claude/exp26-ui-lane-d` at `be6f068` (7 commits on
Lane A's reviewed head `08d9b8e`). Written against the lane's own notes
([lane-d.md](lane-d.md)), the plan's §4.2/§5/§7, `docs/flows/dashboard.md`
and CLAUDE.md's "The failure mode to avoid".

The short version: **the lane's measured claims are all true**, and the
review found four things nothing in the lane's own gates could see. Two of
them are rules this repository states in prose and enforces nowhere; one is
an accessibility regression the rewrite introduced; one is that the file
written specifically as the contract for exp24's login page cannot express
that page's form.

## 1. The draft's claims, re-measured

Everything below was re-run by the reviewer on `be6f068`, not carried
forward from the lane's report.

| claim                                                           | verdict                  | evidence                                                                                                                                                                                            |
| --------------------------------------------------------------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `styling_files` 2 → 0, `class_tokens_distinct` 17 → 0           | **true**                 | `exp26-plan-count.sh` on a `git archive` of `08d9b8e` and on the worktree: `@vpay/dashboard 2 → 0` and `17 → 0`                                                                                     |
| `OUTSIDE @vpay/ui` `styling_files` 18 → 16, tokens 92 → 85      | **true**                 | same two runs, `OUTSIDE` row                                                                                                                                                                        |
| `just lint-web` green                                           | **true**                 | exit 0                                                                                                                                                                                              |
| `just test-web` green, dashboard 9/9                            | **true**                 | exit 0; `@vpay/dashboard` 5 files / 9 tests; `@vpay/ui` 60/60, `@vpay/checkout` 448/448, `examples/shop` 96/96 unchanged                                                                            |
| `just verify` red only on Lane B's `checkout`                   | **true**                 | exit 1, and the only output is `screens.tsx:408,409` `form-control`/`label-text`. The other ten gates print `ok`                                                                                    |
| `verify-ui` clean scoped to `frontends/apps/dashboard`          | **true**                 | all **six** current checks run individually with the recipe's own `git grep` patterns: no match                                                                                                     |
| `tailwind: true` is actually live (six rules)                   | **true**, and decisively | a long `className` in this app fails `better-tailwindcss/enforce-consistent-line-wrapping` **and** `enforce-consistent-class-order`; commenting `tailwind: true` out makes the same file lint clean |
| the `next.config.ts` webpack workaround is gone and unnecessary | **true**                 | `git diff 08d9b8e..HEAD -- next.config.ts` is empty; `next build` from a wiped `node_modules` succeeds (§4 below)                                                                                   |
| the theme mutation is caught                                    | **true**                 | `data-theme="corporate"` → `src/layout.test.tsx` fails, 1 of 9                                                                                                                                      |
| the two "Lane B measured" items do not reproduce                | **true**                 | no `tailwind.config.ts` under `frontends/apps/dashboard` on any commit of this branch; `transpilePackages` lists `@vpay/ui` and predates the lane                                                   |
| no fake rows anywhere                                           | **true**                 | the only `amount` in a shipping file is a prop type. The two `pi_example_*` ids live in `payments-table.test.tsx`, which is where a fixture belongs                                                 |
| README's "4 files, 6 tests"                                     | **false**                | it was 5 files / 9 tests when written. Fixed                                                                                                                                                        |

## 2. Findings

### Finding 1 — `<main>` was dropped; axe `region` regressed 0 → 1 _(correctness, a11y)_

The `08d9b8e` scaffold wrapped its whole page in `<main>`
(`git show 08d9b8e:frontends/apps/dashboard/app/page.tsx`). The rewrite onto
`@vpay/ui` replaced it with `PageShell`, which renders a `<div>`, and the
landmark disappeared. Measured with `axe-core@4.13.0` over the real rendered
`<body>`:

```
08d9b8e   []
be6f068   ["region: <section>"]     "All page content should be contained by landmarks"
```

Nothing saw it: 9/9 vitest, `lint`, `typecheck`, `next build` and
`dashboard.cy.ts` are green either way.

The lane's notes decline an app-level axe pass explicitly — "`@vpay/ui`'s own
axe suite already covers every component this app composes; adding a second
one here would test the same components a second time rather than this app's
own markup, which is composition only". The first half is true. The second is
the mistake: a **landmark is not a component**. It is a decision
`app/layout.tsx` makes about the document, and no component's own suite can
hold it.

Fixed in `3b80b0e`: `<main>` restored, and `src/a11y.test.tsx` added — plan
§7 row 5's exact structural rule list, over the real layout+page `<body>` and
over all four recipes including the sign-in form's error and pending states.
Decisive mutation run: remove `<main>` → `expected [ 'region: <section>' ] to
deeply equal []`.

### Finding 2 — the nav-honesty rule was gated by nothing _(gate-hole)_

> "The navigation is only ever allowed to link to slices that exist. A menu
> entry for a page nobody wrote is the same lie as an empty table."
> — `docs/flows/dashboard.md`

That rule is quoted in this app's `README.md`, again in `app/layout.tsx`'s
doc comment, and again in `docs/status.md`. Measured: adding

```tsx
<a href="/payments">Payments</a>
```

to the `<nav>` leaves **9/9 vitest green, `lint` green** and
`dashboard.cy.ts` untouched. The app would ship a link to a page nobody wrote
and every gate in the repository would say fine. Stating a rule three times
and checking it zero times is exactly the shape CLAUDE.md warns about.

Fixed in `8b654ac`: `src/layout.test.tsx` resolves every internal `href` the
layout renders against `app/**/page.{tsx,ts,jsx,js}` on disk, plus a negative
control (`pageExists('/')` true, `pageExists('/payments')` false) so the
first case cannot pass by the helper answering `true` to everything.
Mutation: `expected [ '/payments' ] to deeply equal []`.

### Finding 3 — the sign-in recipe printed its error twice _(correctness)_

`SignInForm` rendered the `error` string through `FieldError match={…}`
**and** through `<Alert tone="error">`. `FieldError` is not invisible —
`@vpay/ui` gives it `text-xs text-error` — so every failed sign-in showed the
same sentence twice. The lane's own test asserted `getByRole('alert')` only,
which the duplicate passes, because `FieldError` carries no `role`.

Measured: the rendered form contained the error string **2** times.

Fixed in `b6193b5`, with a test that counts occurrences rather than trusting
a reading. The alert is the single place a form-level error appears; a
genuinely field-level error is what `FieldError` is for, and `@vpay/ui` still
exports it.

### Finding 4 — the recipe cannot express the form it is the contract for _(correctness / contract-hole)_

The brief's whole reason for the recipes is that exp24's pages should land on
them. So the review built **exp24's actual login form from the recipe
verbatim**, as six probe assertions, against what
`claude/exp24-staff-auth`'s own ADR-0017 and `docs/flows/dashboard-auth.md`
say that page does: `POST /dash/v1/staff/login` with an argon2id + pepper
password, then `POST /dash/v1/staff/totp` with an RFC 6238 code behind a
**compare-and-swap replay guard** on `staff_members.last_totp_step`.

All six failed:

```
× leg 1 needs a password field            recipe renders no password control
× code constrained to six digits          { maxLength: -1, pattern: null }
× submit can be disabled while in flight  expected false to be true
× the alert can carry the request id      'Invalid code.' does not contain 'req_01J8ABC'
× the error is shown once                 expected 2 to be 1
× a double click submits once             expected 2 to be 1
```

The `pending` gap is the one with teeth. The second submission of one TOTP
code is refused **by design** — that is what the replay guard is — so a
double-click on this form would have shown "invalid code" to a staff member
who typed a perfectly good one, and the form gave the caller no way to
prevent it.

Fixed in `b6193b5`. Three of the four controls added and tested: `pending`
(disables every control _and_ makes the submit handler a no-op, so a
`requestSubmit` or an Enter key cannot get around `disabled`), `requestId`
(vpay emits `request-id`/`x-request-id` with one value on every response, and
`vpay-api`'s error envelope carries no field for it deliberately — the header
is the only thing a locked-out staff member can quote to an operator), and
`codeLength` (`maxLength` + `pattern`, six by default).

**The fourth — a password control — was deliberately not added.**
`docs/flows/dashboard.md` on _this_ branch still records "how does a human
staff member prove who they are?" as an open decision, and ADR-0017 is not on
this tree. Answering it here would settle something the plan and the flow doc
both reserve. It is named instead — in the recipe's doc comment, in the
README, and here — with the shape the second leg takes, so exp24 adds it
knowingly rather than discovering the gap.

After the fixes: 5 of the 6 probes pass; the password probe still fails, on
purpose.

### Finding 5 — README's test counts were stale _(misleading-claim, nit)_

`# 4 files, 6 tests` in the README's "Testing this app" block, against an
actual 5 files / 9 tests at `be6f068` (and `docs/status.md`'s own row said 9
in 5 in the same commit). Now `6 files, 22 tests`, matching the suite.

## 3. Mutation table

| #   | mutation                                                 | must                             | as delivered (`be6f068`)                                                                       | after the review                                                           |
| --- | -------------------------------------------------------- | -------------------------------- | ---------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 1   | `data-theme` reverts to `corporate`                      | fail                             | **fails** — `src/layout.test.tsx`, 1 of 9                                                      | fails                                                                      |
| 2   | a `<StatusBadge>` renders a duplicate `data-status`      | fail `dashboard.cy.ts`           | not re-run in the browser (plan §5's own case, and `dashboard.cy.ts` is unchanged from master) | unchanged                                                                  |
| 3   | the scaffold notice loses `role="status"`                | fail `dashboard.cy.ts`           | unchanged spec, unchanged assertion                                                            | unchanged                                                                  |
| 4   | `tailwind: true` commented out, long `className` present | lint must fail **with** the flag | **passes with the flag removed, fails with it** — the six rules are live                       | unchanged                                                                  |
| 5   | `<main>` removed from the layout                         | should fail                      | **nothing failed**                                                                             | **fails** — `region: <section>`                                            |
| 6   | `<a href="/payments">` added to the `<nav>`              | should fail                      | **nothing failed** (9/9, lint clean)                                                           | **fails** — `[ '/payments' ]`                                              |
| 7   | `SignInForm` rendered without `pending`, double-clicked  | should submit once               | **submitted twice**                                                                            | caller can prevent it; `pending` refuses both the click and a raw `submit` |

## 4. Gates, on the reviewed head

Every number below was produced by the reviewer, under the `.nvmrc` Node
(v22.23.2), on this branch. `just fmt` was never run (plan §8.1).

| gate                                                                                                                                                                                                                       | as delivered (`be6f068`)                                                                                                                                  | on the reviewed head                                                                                                                                                                                                                                |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just lint-web`                                                                                                                                                                                                            | exit 0                                                                                                                                                    | exit 0                                                                                                                                                                                                                                              |
| `just test-web`                                                                                                                                                                                                            | exit 0                                                                                                                                                    | exit 0                                                                                                                                                                                                                                              |
| `pnpm --filter @vpay/dashboard typecheck`                                                                                                                                                                                  | clean                                                                                                                                                     | clean                                                                                                                                                                                                                                               |
| `pnpm --filter @vpay/dashboard lint`                                                                                                                                                                                       | clean, six `better-tailwindcss` rules live                                                                                                                | clean                                                                                                                                                                                                                                               |
| `pnpm --filter @vpay/dashboard test`                                                                                                                                                                                       | 5 files / **9** tests                                                                                                                                     | 6 files / **22** tests                                                                                                                                                                                                                              |
| `just verify`                                                                                                                                                                                                              | exit 1 — `verify-ui` only, on `frontends/apps/checkout/src/components/screens.tsx:408,409` (`form-control`, `label-text`). Lane B's tree, not this lane's | unchanged, same two lines                                                                                                                                                                                                                           |
| `verify-ui`'s six checks scoped to `frontends/apps/dashboard`                                                                                                                                                              | all clean                                                                                                                                                 | all clean                                                                                                                                                                                                                                           |
| `pnpm install --frozen-lockfile` from a **wiped** `node_modules` (every `node_modules` in the worktree removed first)                                                                                                      | not run by the lane                                                                                                                                       | exit 0                                                                                                                                                                                                                                              |
| `pnpm --filter @vpay/dashboard build` on that clean install, `.next` removed                                                                                                                                               | claimed                                                                                                                                                   | exit 0 — "Compiled successfully in 11.7s", 4/4 static pages, no `next.config.ts` workaround present                                                                                                                                                 |
| `just test-e2e`, all four specs, isolated project `exp26d-review` on ports 18601–18605 (the dashboard's `3000:3000` is **not** overridable in `compose.e2e.yml`, so this cannot run while another lane's stack holds 3000) | claimed 11/11                                                                                                                                             | exit 0 — **11 tests, 11 passing, 0 failing, 0 skipped**: `checkout.cy.ts` 1/1, `dashboard.cy.ts` 3/3, `shop-hosted.cy.ts` 3/3, `shop-embedded.cy.ts` 4/4. Teardown ran to completion; containers, volume, network and all five ports confirmed gone |
| compiled CSS (plan §6.4)                                                                                                                                                                                                   | claimed                                                                                                                                                   | `bumblebee` ×2, `alert-warning`/`badge-success`/`badge-error`/`badge-warning`/`badge-neutral`/`badge-info` ×1 each; `corporate`, `form-control`, `label-text` **absent**                                                                            |

### Recipe by recipe

| recipe           | compiles | rendered by a test             | `@vpay/ui` exports only                                        | raw utility classes | axe (structural)                            |
| ---------------- | -------- | ------------------------------ | -------------------------------------------------------------- | ------------------- | ------------------------------------------- |
| `SignInForm`     | yes      | yes — 8 cases (2 as delivered) | `Alert`, `Button`, `Field`, `FieldLabel`, `Input`, `Text`      | none                | 0 violations, error and pending states both |
| `PaymentsTable`  | yes      | yes — 1 case                   | `StatusBadge`, `Table` (+ `PaymentStatus` from `@vpay/tokens`) | none                | 0 violations                                |
| `DetailTimeline` | yes      | yes — 2 cases                  | `List`, `Stack`, `Text`                                        | none                | 0 violations, populated and empty           |
| `EmptyState`     | yes      | yes — 1 case                   | `Heading`, `Stack`, `Text`                                     | none                | 0 violations                                |

Zero `className` strings in any recipe or in `app/` — the counting script's
one `classname_sites` hit for this package is the **word** `className` inside
a comment in `eslint.config.js`, not an attribute.

## 5. What exp24 must know to build its pages on this

1. **The sign-in recipe is one leg.** It has no password control, and that
   is deliberate (finding 4). Add the first leg as another `Field` +
   `FieldLabel` + `Input type="password"` in the same shape; do not reach for
   a raw `<input>` or a `className`.
2. **Set `pending` around every submit.** The TOTP step is verified behind a
   compare-and-swap on `staff_members.last_totp_step`, so the second
   submission of one code is refused on purpose. Without `pending`, a
   double-click produces "invalid code" for a code that was fine.
3. **Pass `requestId`.** vpay emits `request-id` and `x-request-id` with one
   value on every response; `vpay-api`'s error envelope carries no field for
   it on purpose. The alert is where a staff member reads it.
4. **A new page means a new nav link, in the same commit** — and
   `src/layout.test.tsx` will fail the moment the layout links to a route
   `app/` has no `page.tsx` for. That is the intended workflow, not an
   obstacle: add the page, then the link.
5. **Wrap page content in a landmark.** `PageShell` is a `<div>`;
   `app/layout.tsx` supplies the `<main>`. If a page introduces its own
   top-level region, `src/a11y.test.tsx` is where it gets checked.
6. **`Tabs`, `Skeleton` and `Toast` are still not built** (plan §3 makes them
   conditional on a screen needing them). A page that needs one asks Lane A's
   package for it rather than writing it locally — `verify-ui` check 4 fails
   on a `cva(` outside `@vpay/ui`.
7. **`data-theme` is `bumblebee` and it is the only theme `@vpay/ui`
   compiles.** Any other value renders the app completely unstyled, with no
   error anywhere. `src/layout.test.tsx` pins it.

## 6. Not checked

- **Contrast.** Nobody has measured `bumblebee`'s colour contrast in a real
  browser. jsdom computes no paint, so the axe suite here excludes
  `color-contrast` on purpose (plan §7 row 6). `cypress-axe` against the
  running stack is still built by no lane.
- **Mutations 2 and 3 in the table above** (`data-status` duplicated, the
  scaffold notice losing `role="status"`) were **not** re-run in a browser.
  `dashboard.cy.ts` is byte-identical to master's and its three assertions
  pass against the live stack, but the review did not mutate the page and
  re-run Cypress to watch them fail — a full `test-e2e` cycle per mutation
  was not affordable alongside the sibling lanes' own stacks.
- **`just ci`** — the whole-revamp gate (plan §7 row 2), for the final merged
  head after all four lanes land. Not this lane's to run, and nothing under
  `backends/` is touched by it.
- **The Docker image** was rebuilt by `test-e2e` and `dashboard.cy.ts` ran
  against it; no separate image audit was done.
- **`@vpay/ui`'s own correctness** is taken from Lane A's review, not
  re-derived here.
- **exp24's branch was read, not run.** `claude/exp24-staff-auth` at `1a29ec7`
  touches no frontend file at all (checked: `git diff --name-only
master...HEAD` has no `frontends/` entry, and its
  `frontends/apps/dashboard` is still master's ten-file scaffold). Its
  ADR-0017 and `docs/flows/dashboard-auth.md` were read to derive finding 4's
  probes; no code from it was executed.

## 7. Verdict

**Not safe as drafted — safe after this review's four fixes.** Nothing the
lane reported was untrue, and the measurements it took were real. What it
missed were the checks nobody had asked for: a landmark it deleted, a rule it
quoted three times and gated zero, an error it printed twice, and a login
form it could not build with the file it wrote to be that form's contract.
Every one of them passed all nine gates the lane ran.

Two of the four are single-line mistakes. The pattern is not: each survived
because the thing that would have caught it was argued away as redundant
("`@vpay/ui`'s axe suite already covers this") or left as prose ("add its
link the day the page lands").
