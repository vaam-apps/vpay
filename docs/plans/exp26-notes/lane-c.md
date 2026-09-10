# exp26 Lane C notes — `examples/shop` on Tailwind 4 + daisyUI 5 (decision D1)

> **Superseded in three places by the review**
> ([lane-c-review.md](lane-c-review.md), 2026-09-07). Read that document
> beside this one; where they disagree it is right and this is wrong.
>
> 1. **The branch was rebased onto `08d9b8e`**, Lane A's _reviewed_ head, so
>    the base SHA below and the commit SHAs below are the pre-rebase ones.
> 2. **`just test-e2e` runs.** The `dashboard` image failure that blocked it
>    is fixed on `08d9b8e`; the full recipe was run to completion in review
>    and passes 11/11. The manual substitute recorded below is no longer
>    the evidence for the two shop specs.
> 3. **`just lint-web`'s failure is caused by this lane, not pre-existing.**
>    The `git stash` reproduction below is invalid — `git stash` does not
>    uninstall a dependency tree, so `node_modules` still held Tailwind 4
>    for the "unmodified base" run. Reproduced properly (revert this lane's
>    two manifest files, reinstall, re-run) the gate passes without this
>    lane and fails with it. See the review's finding 6.
>
> The review also found that the `badge-{tone}` classes this document
> reports as present in the compiled stylesheet were present **only because
> a vitest file contains the strings** — finding 1, fixed.

Branch `claude/exp26-ui-lane-c`, base `177645e` (Lane A's head). Commits, in
order, on top of that base:

1. `e53c13d` — shop deps alone: `tailwindcss@4.3.3`, `daisyui@5.7.28`,
   `@tailwindcss/postcss@4.3.3`, `postcss@^8.5.28` (matching Lane A's exact
   pins for `@vpay/ui`), plus `@testing-library/react@^16.1.0`,
   `@testing-library/jest-dom@^6.6.0`, `jsdom@^25.0.1` for the two new
   component tests. `examples/shop/postcss.config.js` is new — the shop had
   no PostCSS pipeline at all before.
2. `d4b2702` — `globals.css` (229 lines → 6) and `layout.tsx` (the demo
   banner and header on daisyUI classes, `data-theme="bumblebee"`).
3. `62e4799` — every remaining shop page/component on daisyUI classes: the
   catalogue, cart, checkout and four order pages, `cart-table`,
   `checkout-form`, `order-summary`, `order-actions`, `test-numbers-panel`,
   `order-poller`, `embedded-checkout`.
4. `e8d2742` — the two vitest suites for the plan's decisive
   mutations, and the vitest config change (`environment: "node"` stays the
   default; jsdom is opt-in per file, matching
   `frontends/apps/checkout/vitest.config.ts`'s own convention).
5. (this commit) — `docs/status.md`, `docs/flows/hosted-checkout.md`,
   this file, with the manual Cypress run's results filled in below.

Head at report time: run `git rev-parse HEAD` in the worktree — do not trust
a pasted SHA in this file over that command.

## Before / after (`exp26-plan-count.sh`)

Before (`177645e`, Lane A's own head — Lane A does not touch `examples/shop`,
so this is unchanged from the plan's own baseline table):

```
@vpay-examples/shop  styling_files=11  classname_sites=41  class_tokens_distinct=15
                      class_tokens_total=46  css_lines=229  inline_styles=24
```

After (this head):

```
@vpay-examples/shop  styling_files=16  classname_sites=128  class_tokens_distinct=93
                      class_tokens_total=300  css_lines=5  inline_styles=0
```

`css_lines` 229 → 5 and `inline_styles` 24 → 0 both meet Lane C's own
acceptance criteria in plan §5 (`css_lines 229 → ≤ 8`, `inline_styles 24 → 0`).

**`styling_files` rose 11 → 16 instead of falling, and this is measured and
explained rather than smoothed over — see "Where the plan (or the brief) was
wrong" below.** No per-package `styling_files` target for the shop appears
anywhere in the plan itself (§5 Lane C's acceptance section names only
`css_lines` and `inline_styles`; §2's `styling_files` target, `≤ 3`, is the
whole-revamp aggregate across checkout + dashboard + shop combined, read at
the final gate after all four lanes land).

The `OUTSIDE @vpay/ui` aggregate row the whole-revamp gate reads:

```
before (Lane A head, unchanged from plan baseline): styling_files=18  class_tokens_distinct=92
after (Lane A + Lane C only):                       styling_files=23  class_tokens_distinct=136
```

This rose rather than fell, and that is **expected mid-migration**, not a
Lane C regression: `checkout` and `dashboard` still carry their pre-revamp
markup (Lane B and Lane D have not landed), so the union only grows while
the shop's own tokens change and nothing shrinks elsewhere yet. The
aggregate cannot be read meaningfully until all three app lanes have landed.

## Gates, recipe by recipe

| recipe                                                                                                                                   | result                                             | evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ---------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm install` (real)                                                                                                                    | ✅                                                 | 1 new package resolved, 0 peer-dependency errors                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `pnpm install --frozen-lockfile`                                                                                                         | ✅                                                 | exits 0, "Lockfile is up to date"                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `pnpm --filter @vpay-examples/shop typecheck`                                                                                            | ✅                                                 | exit 0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `pnpm --filter @vpay-examples/shop lint` (ESLint + `prettier --check`)                                                                   | ✅                                                 | exit 0 — ESLint clean on the first run; `prettier --check` initially flagged 7 files for attribute wrapping only (class strings untouched), fixed with `prettier --write` scoped to `examples/shop/**`                                                                                                                                                                                                                                                                                                                          |
| `pnpm --filter @vpay-examples/shop test`                                                                                                 | ✅                                                 | **100 vitest cases, 0 skipped** (was 96) — 11 files, all green                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `docker build -f examples/shop/Dockerfile .`                                                                                             | ✅                                                 | full multi-stage build, `next build` compiles the Tailwind/daisyUI CSS pipeline; image built and removed after (no leftover)                                                                                                                                                                                                                                                                                                                                                                                                    |
| `just verify-ui`                                                                                                                         | 🔴 (expected, not a Lane C defect)                 | fails on `frontends/apps/checkout/src/components/screens.tsx`'s `form-control`/`label-text` — Lane B's unmigrated file, exactly as Lane A's own report recorded. All four of `verify-ui`'s checks confirmed to pass scoped to `examples/shop` alone (`git grep` run with each pattern against `-- examples/shop` only, after `git add -A examples/shop` so untracked files are visible to it)                                                                                                                                   |
| `just lint-web`                                                                                                                          | 🔴 (pre-existing, not caused by this lane)         | fails at `frontends/apps/checkout`'s own `typecheck`: `tailwind.config.ts(12,13): error TS2322` — a type mismatch between `tailwindcss@^3.4.17`'s `PluginAPI` and the `tailwindcss@4.3.3` types now resolved elsewhere in the workspace (Lane A's `@vpay/ui` devDependency). **Confirmed pre-existing by reproducing on the unmodified Lane A base (`git stash`, re-run, same failure, `git stash pop`)** — this lane's diff touches nothing under `frontends/`, so it cannot be the cause. Lane B's file                       |
| `just verify-npm-scope`                                                                                                                  | ✅                                                 | unchanged from Lane A                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `just verify-links`                                                                                                                      | ✅                                                 | 894 links, 161 files (was 160 — this lane adds one docs file)                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `just verify-status`                                                                                                                     | ✅                                                 | 1 declared unimplemented item, unchanged                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `just test-e2e` (full recipe, `demo_project=exp26c`, ports `18401-18405`)                                                                | 🔴 (blocked upstream of Lane C, not a shop defect) | fails building the **`dashboard`** image before any container starts or any spec runs: `Module not found: Can't resolve './cn.js'` etc. in `@vpay/ui/src/index.ts`, imported by `frontends/apps/dashboard`'s `next build`. This is Lane D's integration gap (dashboard consuming Lane A's `@vpay/ui` exports through Next's webpack bundler) — nothing in this lane's diff touches `frontends/packages/ui` or `frontends/apps/dashboard`. See "What was NOT done" below for how the two shop specs were actually proven instead |
| the two shop Cypress specs, driven manually (`docker compose … up` on every service but `dashboard`, then `cypress run --spec` per spec) | ✅                                                 | see the dedicated section below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

## What was NOT done

- **`just test-e2e` as a single recipe run to completion.** It builds all
  four app images in one `docker compose … up -d --build`, and the
  `dashboard` image fails to build (see the gates table). Neither
  `shop-hosted.cy.ts` nor `shop-embedded.cy.ts` needs the dashboard
  container — confirmed by reading both specs, neither references
  `dashboard.cy.ts`'s origin or visits it — so the two specs this lane owns
  were driven with `dashboard` left out of the `docker compose up` service
  list and each spec run directly with `cypress run --spec`, rather than
  through `pnpm --filter @vpay/e2e e2e` (which globs every spec, dashboard's
  included). This is **not** the plan's own gate recipe and is recorded as a
  substitute, not a claim that `just test-e2e` passes.
- **`just lint-web` and `just ci` end to end.** Both fail on the checkout
  app's pre-existing `tailwind.config.ts` typecheck (see the gates table);
  neither failure is in this lane's diff and both are reproduced on the
  unmodified Lane A base.
- **An explicit `@source` line in `examples/shop/src/app/globals.css`.**
  The plan's own §4.3 table includes one in its six-line template; measured
  here to be unnecessary and NOT added — see "Where the plan was wrong"
  below.
- **Nothing outside `examples/shop/**` and the two docs files.** `@vpay/ui`,
  `@vpay/tokens`, `@vpay/config`, `frontends/apps/checkout`,
  `frontends/apps/dashboard` — all untouched, as scoped.

## The two shop Cypress specs, proven manually

`docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml up -d
--build --wait` with the service list trimmed to exclude `dashboard`
(`postgres vpay-server vpay-worker wiremock-webhook wiremock-orange
vpay-checkout vpay-shop` — `wiremock-mtn` starts automatically as a
dependency), project `exp26c`, ports `18401` (server) `18402` (receiver)
`18403` (orange stub) `18404` (checkout) `18405` (shop), all confirmed free
before starting so as not to collide with another `vpay-demo` project
already running on this shared Docker host.

`docker compose ... up -d --build --wait` completed clean (all seven
containers `Healthy`); `/healthz` on the server (18401), the shop (18405)
and checkout (18404) all answered before any spec ran.

`cypress run --spec cypress/e2e/shop-hosted.cy.ts` (unframed pass,
`chromeWebSecurity` on): **3 passing, 0 failing, 24s** — MTN push to
`paid`, Orange redirect to `paid`, and a declined MTN charge landing on
`cancel_url` with the order never `paid`.

`VPAY_E2E_FRAMED=1 cypress run --spec cypress/e2e/shop-embedded.cy.ts`
(framed pass, `chromeWebSecurity` off): **4 passing, 0 failing, 16s** —
the frame's exact `src` and CSP, MTN completing inside the frame, Orange
breaking out and returning to `return_url`, and an unregistered framer
refused.

**7 of the plan's 11 `just test-e2e` tests, both specs this lane owns,
0 failing.** The other 4 (`checkout.cy.ts` 1, `dashboard.cy.ts` 3) were not
run — `dashboard` was deliberately left out of the `docker compose up`
service list (see the gates table) and `checkout.cy.ts` is Lane B's own
spec, not this lane's to prove.

One unrelated snag, fixed and noted rather than worked around silently:
the first `cypress run` invocation failed at its `before:run` hook with
`EADDRINUSE: address already in use 127.0.0.1:4181` — an orphaned
`Cypress: Config Manager` process and the `checkout-browser` static
server it had spawned, both still listening from a `before:run` phase
that never got to `after:run` cleanup (unrelated to this lane's own
first, aborted `test-e2e` attempt — that one failed at the Docker build
step, before Cypress ever started). Killed by PID, confirmed the port
free, and the retry ran clean.

Both the `exp26c` compose project (`down -v` — containers, network and
volume all removed, confirmed by an empty `docker ps -a --filter
label=com.docker.compose.project=exp26c`) and the four `exp26c-*` images
`docker compose … --build` produced (`docker rmi`, not part of `down -v`
by default — matching `just demo-down`'s own documented behaviour) were
torn down afterward. The pre-existing `vpay-demo` project on this shared
host was confirmed running, unaffected, both before and after.

## Where the plan (or the brief) was wrong, measured rather than assumed

1. **§4.3's six-line `globals.css` template includes an `@source` line this
   package does not need.** Confirmed by compiling the real `globals.css`
   with `@tailwindcss/cli` at three points: (a) before any markup changed,
   where daisyUI still generated `.card` and `.badge` rules from words that
   appear only in this package's own doc comments (proof that Tailwind's
   automatic content detection already reaches `examples/shop/src` with no
   `@source`); (b) after the full migration, where every daisyUI class
   actually used in the markup (`.btn`, `.badge-{success,warning,error}`,
   `.table-zebra`, `.fieldset-legend`, `.radio`, `.navbar`,
   `.alert-{warning,error,info}`) is present in the compiled output with no
   `@source` line added; (c) `table-zebra` was initially missing after (b) —
   not an `@source` problem, but a markup one: the plan's own §4.3 table
   names `table table-zebra` for `cart-table.tsx` and the first draft here
   used `table` alone. Fixed and reconfirmed present in the compiled
   output. `@vpay/ui`'s `styles.css` needs `@source` because Tailwind does
   not scan across a symlinked workspace package boundary by default (Lane
   A's notes, finding 1); `examples/shop`'s own files are not reached
   through such a boundary, so the same requirement does not carry over.
2. **The per-lane target in this lane's own brief (`styling files 11→≤2`)
   does not appear in the plan itself, and is very likely a
   misapplication of the plan's whole-revamp AGGREGATE target rather than
   a Lane C-specific one.** Measured: plan §5 Lane C's acceptance section
   lists only `css_lines 229 → ≤ 8` and `inline_styles 24 → 0` — no
   `styling_files` number. Plan §2's `styling_files ≤ 3` row is explicitly
   the `OUTSIDE @vpay/ui` **aggregate across checkout + dashboard + shop
   combined**, read once at the final whole-revamp gate (§7 row 1) after
   Lane B and Lane D also land — not a per-package figure, and not
   attributed to any one lane in §5's per-lane breakdown. Taken at face
   value against just the shop, `≤ 2` is also structurally unreachable
   under D1's own design: unlike checkout/dashboard, which have `@vpay/ui`
   to absorb `className` decisions into a handful of wrapper components,
   D1 explicitly forbids the shop from importing `@vpay/ui` — every page
   composes daisyUI classes **directly**, which is the opposite of
   concentrating them into two files. Building a shop-local component
   library to hit the number was considered and rejected: it is not what
   the plan's own §4.3 per-file table shows (every row there is an
   existing file changing its own classes, not a new shared component),
   and it would reintroduce exactly the kind of bespoke abstraction layer
   D1's "a merchant must be able to reproduce it" is arguing against.
3. **The counting script's `styling_files` metric cannot see `style={{…}}`
   at all — only `className=`.** This is stated as a known blind spot in
   the script's own header for a _different_ reason (a class name in a
   lookup table outside `cva`/`cn`/`clsx`/`twMerge`), but it has a second
   consequence the header does not name: a file using **only** inline
   styles, with zero `className`, is invisible to `styling_files` before
   the fact and visible after fixing exactly what the script's `inline_styles`
   column already knows is bad. All four `orders/[id]/*` pages were this
   case — zero `className`, several `style={{color:'var(--muted)'}}` —
   and converting them from bad-and-invisible to fixed-and-counted is what
   most of the `styling_files` rise (11 → 16) actually is.

## For the reviewer

Decisive mutations from plan §5 Lane C, run and reverted (not left in the
tree — confirmed here so the reviewer can re-run them):

- `OrderStatusBadge` collapsed to render one tone for every status
  (`className="badge badge-neutral"` regardless of `status`) →
  `order-summary.test.tsx`'s first case fails:
  `expected 'badge-neutral' to be 'badge-warning'`.
- `TestNumbersPanel`'s Orange-caveat `role="alert"` removed →
  `test-numbers-panel.test.tsx`'s first case fails:
  `Expected the element to have attribute: role="alert" / Received: null`.
- A `data-testid` dropped from the cart table: not run as a permanent
  mutation (it would fail `just test-e2e`, per the plan's own table) — left
  for the reviewer to stage, since this lane's own `just test-e2e` run is
  blocked upstream (see the gates table).

`just verify-ui`'s four checks, scoped to `examples/shop` alone via `git
grep … -- examples/shop` after `git add -A`: all four pass with zero
matches. Not re-run as a whole-tree gate here since `frontends/apps/checkout`
is known-red on check 2 for a file this lane does not own (Lane A's own
report records the same finding on the same base).

Known limitation carried from Lane A's own notes: `git grep` (what
`verify-ui` uses) only searches tracked/staged files — a new, uncommitted
file is invisible to a **local** run until `git add`ed. Every file this lane
touches is committed by the time this document is read, so it does not
apply here, but it is worth restating for whichever lane reads this last.
