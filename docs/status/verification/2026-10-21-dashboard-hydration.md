# 2026-10-21 — the dashboard failed hydration under Cypress, and `dashboard.cy.ts` went 17 of 18 red

Branch `claude/happy-hopper-r3h6rz`, from `master` at `0706cd5` (#319). Two
changes. In `frontends/apps/dashboard`, the theme script moves from `<head>`
to the first child of `<body>`, and one unit case pins it there. In
`dashboard.cy.ts`, one stale assertion that #318 missed is corrected; that
second failure is described below. **No route, no server code, no migration
and no `NotImplemented` token moved.**

## What failed

`master` CI run 39610100683 on `0706cd5`, job `e2e (compose)`, step "the
browser specs". It was the first run on `master` to reach that step since
#318 merged. Every `master` run from `e28292d` on had been cancelled by the
next merge, and #318's own run had not finished when it merged. Its
verification page ([2026-10-21-dashboard-md3e.md](2026-10-21-dashboard-md3e.md))
says `dashboard.cy.ts` was "updated in this change and not run".

```text
  Running:  dashboard.cy.ts                                                                 (2 of 3)
  1 passing (2m)
  17 failing
  ✔  checkout.cy.ts       1 passing
  ✖  dashboard.cy.ts     18 tests, 1 passing, 17 failing
  ✔  shop-hosted.cy.ts    4 passing
```

The first failure, and most of the rest:

```text
  1) the dashboard
       sends an unauthenticated visitor to the sign-in form, and no further:
     Error: The following error originated from your application code, not from Cypress.
  > Minified React error #418; visit https://react.dev/errors/418?args[]=HTML&args[]= …
```

The tests that did not fail on #418 failed on its consequence: the spec runs
with `testIsolation: false` in order, and the sign-in in its second test never
completed. One example is "the session cookie" missing in test 3.

**v0.10.0 (`e393116`) carries the same layout.** Its `vpay-dashboard` images
were built from it between 13:38 and 13:40 UTC. That is the code this run failed on: #319
changes no dashboard file.

## Why

The dashboard served in development mode under the same Cypress (15.21.1)
gives React's full message and the diff:

```text
  > Hydration failed because the server rendered HTML didn't match the client. …
                        <html lang="en" data-theme="dark" suppressHydrationWarning={true}>
                          <head>
+                           <script dangerouslySetInnerHTML={{__html:"(function(..."}}>
-                           {" "}
```

#318 added `themeInitScript` as the one child React itself renders in
`<head>`. React therefore hydrated `<head>` expecting that `<script>` as its
first node. In the document Cypress serves, the first node of `<head>` is a
whitespace text node, because Cypress rewrites the document it drives.
`suppressHydrationWarning` on `<html>` covers that element's attributes, not
its children's structure.

**Without Cypress, the same build hydrates cleanly.** Headless Chromium
(Playwright's `chromium-1194`), loading the production build's `/login`,
`/` and `/payments` in both `prefers-color-scheme` values, logged no
hydration error. Neither did a run that injected a `<script>` with no
whitespace at the start of `<head>`. So what is measured here is that the
layout fails under any agent that puts a node at the start of `<head>`, and
that Cypress is one. No real user's browser was measured. A hydration failure
in a React 19 production build is recoverable: the document is re-rendered on
the client, and Cypress fails the test only because the error is uncaught.

## The fix, and what it was measured against

`app/layout.tsx` renders no `<head>` children. The theme script is the
first child of `<body>`. A parser reaches it before any of the body's
content, so it still runs before anything paints.

Cypress 15.21.1 (its bundled Electron), one test copied from
`dashboard.cy.ts`'s first case (`cy.clearCookies()`, `cy.visit("/payments")`,
expect `/login` and the "Sign in" `<h2>`). The dashboard ran on this machine,
configured with the e2e stack's five `VPAY_DASH*` variables and no vpay
behind them, which is enough for the sign-in form:

| Dashboard                   | Before (`0706cd5`)                           | After (this branch) |
| --------------------------- | -------------------------------------------- | ------------------- |
| `next build` + `next start` | 1 failing (`uncaught exception`, 3 attempts) | **1 passing**       |
| `next dev`                  | 1 failing (the diff above)                   | **1 passing**       |

The theme is still applied before React runs. Headless Chromium on the
production build, with `vaam-ui:theme` preset in `localStorage`, reads
`data-theme` at `DOMContentLoaded`:

```text
light   theme@DCL light   theme now light
system  theme@DCL dark    theme now dark     (colorScheme: dark)
```

Each load logged one `404`, the same before and after, and no hydration
error.

## Tests and gates

- `src/layout.test.tsx`, new case "runs the theme script as the body's first
  child, and puts nothing in `<head>`". It passes on this branch. **With
  `app/layout.tsx` restored to `0706cd5`'s it fails**, and only it does
  (1 failed, 10 passed).
- `pnpm --filter @vpay/dashboard test`: 35 files, **456 passed, 1 skipped**.
  The skipped case is unchanged by this branch; the README's latest figure
  was 455 passed, 1 skipped.
- `pnpm --filter @vpay/dashboard typecheck` and `lint`: clean.
  `prettier --check` on every changed file: clean.
- `cargo xtask`, with this page staged, exit 0 for each:

  ```text
  verify-no-mocks: ok — no test double reachable from a shipping binary
  verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md and all still in shipping code
  verify-errors: ok — 20 error type(s), all classified; …
  verify-sdk-parity: ok — 790 proving test(s) named in docs/sdks/parity.md all exist, 44 dated gap(s), …
  verify-links: ok — 2558 repository link(s) in 476 tracked markdown file(s) resolve to a tracked path …
  verify-npm-scope, verify-serde, verify-repositories, verify-toolchain, verify-migrations: ok
  verify-versions: ok — 26 version references all say 0.10.0
  verify-privacy-inventory: ok — 314 database column(s) classified …
  verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s) agree with what this tree measures
  ```

  `verify-ui` and `check-schema` were not run here (the first is not an
  `xtask` command and the second needs the CrateStack CLI); CI's
  `self-checks` runs both.

**Not run here:** the whole of `dashboard.cy.ts` against the compose stack.
It needs vpay, its OP and a staff member, and only CI's `e2e (compose)` runs
it.

## The second failure, which the first one hid

This branch's first CI run (`1b65619`, run 39620276267) passed **17 of
`dashboard.cy.ts`'s 18** (`checkout.cy.ts` 1/1 and `shop-hosted.cy.ts` 4/4
passed too). Only the last case still failed:

```text
  1) the dashboard
       signs the same staff member back in with the password they set:
     AssertionError: Timed out retrying after 4000ms: Expected to find element: `#dashboard-signin-email`, but never found it.
      at Context.eval (webpack://@vpay/e2e/./cypress/e2e/dashboard.cy.ts:1193:7)
```

That error is the third attempt's; Cypress prints only the last one. #318
moved `HOME_PATH` from `/payments` to `/overview` and updated the first
sign-in's landing assertion (leg 5) to match. It did not update this case's
last line, which still expected `/payments` after the re-sign-in. Here is
what the run is consistent with:

1. Attempt 1 signed in, landed on `/overview`, and failed on that line.
2. Attempts 2 and 3 opened `/login` while signed in. `alreadySignedIn()`
   redirected them to `HOME_PATH`, which has no email field.

**This is inferred, not observed.** Attempt 1's message was not printed and
no screenshot was uploaded. What supports it: this is the only landing
assertion after a sign-in that #318 left at `/payments` (the others in the
spec follow explicit visits to `/payments`). Each attempt took about 30 s,
which is the TOTP wait plus a sign-in, not a 4 s miss. And the redirect for
a signed-in `/login` is `app/login/page.tsx`'s own. The line now expects
`/overview`, with a comment saying why.

**Confirmed by the next run.** On `b6889fb` (run 39626241491), `e2e (compose)`
passed on the first attempt of every case, with no retry:

```text
  ✔  checkout.cy.ts       1 of 1
  ✔  dashboard.cy.ts     18 of 18
  ✔  shop-hosted.cy.ts    4 of 4
  ✔  All specs passed!   23 of 23
  ✔  shop-embedded.cy.ts  6 of 6   (the framed run)
```
