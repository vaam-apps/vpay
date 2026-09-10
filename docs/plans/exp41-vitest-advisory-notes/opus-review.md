# exp41 — sabotage review of the vitest advisory bump

Reviewer pass over `b5be024` (the haiku draft), branch `claude/exp41-vitest-advisory`,
base `970bfe0`. Node `22.23.2` (`.nvmrc`), pnpm `9.15.0`, `CARGO_BUILD_JOBS=4`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`.

**Verdict: NOT safe as drafted.** The version change itself is right and
complete, and no Vitest 4 *configuration* migration is owed — that part of the
draft holds, for reasons it did not check. But the bump breaks a test:
`@vpay/ui` fails **six runs in ten** on vitest 4 and **zero in ten** on vitest 3,
on the same machine with the same jsdom, React and Testing Library. The draft
reported that suite green and reported `just ci` as "still running". One test
file is fixed here; everything else fixed is a claim or a document.

---

## 1. What the change actually is

The advisory (`gh api /advisories/GHSA-82fw-gwwq-j7x9`):

| field | value |
| --- | --- |
| summary | Vitest: Path Traversal / Arbitrary File Read via `@vitest/mocker` Redirect Mock |
| severity | **moderate**, CVSS 5.9 |
| published | 2026-09-08 |
| affected | `vitest` `>= 2.1.0, < 4.1.11` → **4.1.11**; `@vitest/mocker` `>= 2.1.0, < 4.1.11` → **4.1.11** (plus the 5.0.0-beta line → 5.0.0-rc.2) |

The twelve open Dependabot alerts (`gh api 'repos/vaam-apps/vpay/dependabot/alerts?state=open'`),
all one advisory:

- **#25** `@vitest/mocker` — `pnpm-lock.yaml`
- **#33** `vitest` — `pnpm-lock.yaml`
- **#26–#32, #34–#36** `vitest` — the ten manifests
  `examples/shop`, `frontends/apps/{checkout,dashboard}`,
  `frontends/packages/{api-client,config,tokens,ui}`,
  `sdks/{nodejs,stripe-compat,stripe-js}`.

The draft edits exactly those ten `package.json` files, `^3.2.7 → ^4.1.11`, plus
the lockfile. The manifest list matches the alert list one for one — nothing
missed, nothing spurious. After the change the lockfile resolves **one**
`vitest` (`4.1.11`) and **one** `@vitest/mocker` (`4.1.11`), and `pnpm why -r vitest`
reports `4.1.11` in all ten workspace projects.

`@vitest/expect`, `@vitest/spy`, `@vitest/utils` and `@vitest/pretty-format` are
still in the lockfile at **3.2.4** as well as at 4.1.11. That is not a miss:
`pnpm why -r @vitest/expect` shows them arriving through `storybook@10.6.0`
(a peer of `@storybook/react` in `@vpay/checkout` and `@vpay/ui`), and none of
those four packages is named by the advisory. `vitest` itself and
`@vitest/mocker` — the two that are — exist only at 4.1.11.

## 2. Findings

| # | severity | finding |
| --- | --- | --- |
| **F1** | **gate hole / correctness — blocking** | the bump makes `@vpay/ui` fail 6 runs in 10; the draft reported it green and never finished `just ci` |
| F2 | misleading-claim | `just audit-web` is **not** evidence that this advisory is closed |
| F3 | misleading-claim | `just verify` is **twelve** gates, not ten; the draft's list names eight |
| F4 | misleading-claim | the unexplained `sdk-node` 208-vs-207 is not a Vitest 4 effect — `master` is already 208, and `docs/status.md` is stale |
| F5 | correctness (docs) | no `docs/status.md` update, which `CLAUDE.md` requires in the same commit |
| F6 | nit | the migration audit checked 2 of the guide's 14 headings and generalised from them |

### F1 — the bump breaks `@vpay/ui`, six runs in ten

The draft's notes say `just test-web` is green, "ui: 74 tests, 18 test files",
and that `just ci` "was still running" — which is where it would have found out.
`just ci` run to completion on the draft's head `b5be024` exits **1**:

```
frontends/packages/ui test:  FAIL  src/components/select.test.tsx > Select > opens on ArrowDown from the trigger and moves the highlight with the arrows
frontends/packages/ui test: AssertionError: expected <button type="button" …(11)>…(1)</button> to be <div data-selected …(5)>…(1)</div>
 ❯ src/components/select.test.tsx:60:36
frontends/packages/ui test:       Tests  1 failed | 73 passed (74)
error: Recipe `test-web` failed on line 123 with exit code 1
```

It is not load, and it is not a pre-existing flake. Measured, `pnpm --filter
@vpay/ui test` run ten times on each side, same machine, same Node 22.23.2:

| tree | vitest | result |
| --- | --- | --- |
| `b5be024` (the draft) | **4.1.11** | **4 pass, 6 fail** — every failure the same test, the same line |
| `970bfe0` (`master`), `pnpm install --frozen-lockfile` into a throwaway `git archive` copy | **3.2.7** | **10 pass, 0 fail** |

The two trees resolve identical `jsdom@25.0.1`, `@testing-library/react@16.3.2`,
`react@19.2.8` and `react-dom@19.2.8`; vitest is the only difference.

The cause is a synchrony assumption in the test, which vitest 3's schedule
happened to satisfy and vitest 4's does not. Line 60 asserted

```ts
const list = await screen.findByRole('listbox');
…
expect(document.activeElement).toBe(english);
```

`findByRole` resolves as soon as the listbox is in the document; Base UI moves
focus onto the highlighted item after that. Under vitest 4 the assertion often
runs first, and what it finds is the trigger `<button role="combobox">` — which
is exactly what the diff above shows.

**Fixed** by waiting for the condition instead of assuming it, with
`waitFor` — already this package's convention (`dialog.test.tsx:52`) — for both
focus assertions. Nothing about *what* is asserted changed. Proved three ways:

- **20 consecutive runs green** after the fix (`74 passed (74)`, `18 passed (18)`),
  against 4-in-10 before it.
- **Mutation 1** — the first `waitFor` made to assert `french` instead of
  `english`: **fails** (`1 failed | 73 passed`). So `waitFor` is not passing on
  anything that happens to be focused.
- **Mutation 2** — the second `fireEvent.keyDown(list, ArrowDown)` deleted, so
  focus never moves off `english`: **fails**. So the arrow-move is genuinely
  asserted and not merely eventually-true.

The rest of `just test-web` is not affected, and that was measured rather than
assumed — every other suite was run repeatedly on vitest 4 and none failed:
`@vpay/dashboard` 8/8, `@vpay/checkout` 8/8, `@vpay-examples/shop` 8/8,
`@vaam-apps/vpay-stripe-js` 5/5, `@vaam-apps/vpay-sdk` 5/5.

The same *shape* was then looked for by hand rather than left to luck. There are
four `document.activeElement` assertions in the repository:
`dialog.test.tsx:53` was already inside a `waitFor`, `select.test.tsx`'s two are
now, and `checkout-view.test.tsx:285` is synchronous with no `findBy`/`await`
boundary in front of it. The four `findBy` sites in
`frontends/apps/dashboard/src/components/sign-in-form.test.tsx` assert either on
the awaited element itself or on attributes set in the same render commit, not
on a later tick. Nothing else has the racing shape.

### F2 — `just audit-web` never gated this advisory, before or after

The draft's notes say `just audit-web` is "clean — no known vulnerabilities
found; advisory GHSA-82fw-gwwq-j7x9 is no longer present", and offer that as
proof. It proves nothing about this advisory, because the recipe runs
`pnpm audit --audit-level=high` twice and **this advisory is moderate**. The
justfile says so itself, above the recipe: "high and critical fail, moderate does
not… A deliberate ceiling, not an oversight".

Measured in both directions rather than argued. `master`'s lockfile
(`git show 970bfe0:pnpm-lock.yaml`, with `970bfe0:package.json`,
`970bfe0:pnpm-workspace.yaml` and this repository's `.npmrc`, in a scratch
directory), Node 22.23.2:

| command | exit | report |
| --- | --- | --- |
| `pnpm audit --audit-level=high --prod` (audit-web's first run) | **0** | — |
| `pnpm audit --audit-level=high` (audit-web's second run) | **0** | prints `2 vulnerabilities found / Severity: 2 moderate` and still exits 0 |
| `pnpm audit --audit-level=moderate` | **1** | names both `vitest` and `@vitest/mocker`, `>=2.1.0 <4.1.11` → `>=4.1.11` |

So `just audit-web` was green on `master` **with the advisory present**, which is
also why CI stayed green for the two days the twelve alerts were open. On this
branch `pnpm audit --audit-level=moderate` exits **0** with
`No known vulnerabilities found` — that, and the lockfile resolution, not
`audit-web`, is the audit-side evidence the advisory is gone.

Two things follow, and neither is fixed here:

- `just audit-web` is **not part of `just ci`** (it needs the network; the
  justfile says so). `just ci` says nothing at all about this advisory.
- Whether the `--audit-level=high` ceiling should move is **a maintainer
  decision, not a reviewer's**: the justfile reserves it explicitly, with the
  reasoning ("blocking every merge on one trains people to reach for the ignore
  list"). It is recorded in `docs/status.md` as a named consequence — a moderate
  advisory in dev tooling is caught by Dependabot and by nothing this repository
  runs — and left alone.

### F3 — `just verify` is twelve gates

The draft: "All 10 verification gates passed", then lists eight and counts
`verify-docs` as a ninth. Measured, from this branch's `just ci` log:

```
verify: ok — the twelve gates above passed; the verify-docs report is advisory
```

The four the draft's list omits are `verify-no-mocks`, `verify-status`,
`verify-errors` and `verify-ui`. This is the exact miscount `CLAUDE.md` opens
with — it said "three" and was wrong for three days.

### F4 — the `sdk-node` +1 is `master`'s, and `docs/status.md` is stale by one

The draft reported `sdk-node` 208 against a stated baseline of 207 and left it as
"1 more than baseline" with no explanation. A test count that moves on a
dependency bump is exactly the thing that must not be waved through, so it was
traced rather than accepted.

It is not a Vitest 4 collection change. Counting `it(`/`test(` at file scope
across the nine non-live suites, per revision:

| revision | date | static cases |
| --- | --- | --- |
| `f42897f` | 2026-09-08 | 207 |
| `9cb11f8` | 2026-09-10 | 207 |
| `b73543c` | 2026-09-10 | 207 |
| `c4ce32d` | 2026-09-10 | 207 |
| **`49a7063`** | **2026-09-10** | **208** |
| `970bfe0` (this branch's base) | 2026-09-10 | 208 |
| `b5be024` (the draft) | 2026-09-10 | 208 |

`49a7063` ("feat(customers): `customer.created` and `customer.updated`, under the
row's lock (#66)") added one case to `sdks/nodejs/src/webhooks.test.ts`:

```
+  it("customer.created and customer.updated are known event types and their payloads are customers", () => {
```

`master` has been at 208 since that merge. `docs/status.md`'s Node-SDK row still
said "**207 tests across 9 files** … re-measured 2026-09-08", which was true when
written and has been stale since `49a7063`. Corrected in this pass, attributed to
`49a7063` and **not** to the vitest bump, which moves no count anywhere.

The related worry the brief raised — the live suite silently joining
`just test-web` because a default changed — does not happen, and was checked
rather than assumed. `sdks/nodejs` has **ten** `*.test.ts` files; the run collects
**nine**. `invoices.live.test.ts` is excluded by `vitest.config.ts`'s `exclude`,
and the only thing that includes it is `vitest.live.config.ts`, reachable solely
through the `test:live` script, which `pnpm -r test` does not run (`pnpm -r test`
runs `test`). With no `VPAY_BASE_URL`, `pnpm --filter @vaam-apps/vpay-sdk test:live`
still **fails** under Vitest 4 rather than skipping — the `globalSetup` preflight
and `passWithNoTests: false` both behave as before: with no `VPAY_BASE_URL`, `pnpm --filter @vaam-apps/vpay-sdk test:live` exits **1** with `@vaam-apps/vpay-sdk live suite: VPAY_BASE_URL is not set` thrown out of `live-preflight.ts`'s `globalSetup`, before any case is collected.

### F5 — `docs/status.md` was not updated

`CLAUDE.md`: update `docs/status.md` in the same commit, not a follow-up. The
draft did not touch it. Fixed: the "`just audit-web` + CI `web` audit step" row
gains a dated 2026-09-10 paragraph carrying the before/after versions, the twelve
alert numbers, the measured audit exit codes, and the fact that `audit-web` did
not and could not catch this one.

### F6 — the migration audit was two checks wide

The draft checked `test.workspace` and `test.poolOptions`, found neither, and
concluded "all configs compatible with vitest 4.1.11". Those are two of the
fourteen headings in Vitest 4's migration guide. The conclusion happens to be
right; it was not established. §3 is the check that establishes it.

## 3. Vitest 4 migration guide, heading by heading

Source: `docs/guide/migration.md` at tag `v4.1.11` in the vitest repository,
cross-read with context7's `/vitest-dev/vitest/v4.1.6`. Every JS tree
(`frontends/`, `sdks/`, `examples/`) was swept, with `node_modules`, `dist`,
`.next` and `storybook-static` excluded.

| # | guide heading | verdict | evidence |
| --- | --- | --- | --- |
| 1 | Prerequisites: **Vite ≥ 6, Node ≥ 20** | **met** | the lockfile resolves exactly one vite, `vite@6.4.3` (itself an existing deliberate `pnpm.overrides` pin); `.nvmrc` `22.23.2`, root `engines.node >= 22.13.0`, `engine-strict=true` |
| 2 | V8 coverage major changes (AST remapping, `ignoreEmptyLines`, `experimentalAstAwareRemapping`) | **n/a** | no `@vitest/coverage-v8` or `-istanbul` in any manifest or in the lockfile; no `test.coverage` block in any of the ten configs; no recipe or CI step passes `--coverage` |
| 3 | `coverage.all` / `coverage.extensions` removed | **n/a** | same |
| 4 | **Simplified `exclude`** — v4 excludes only `node_modules` and `.git` | **no effect**, checked | nine of the ten configs set `include` explicitly and every one is rooted at `src/` (dashboard `{src,app}/`), so `dist`, `cypress`, `.idea/.cache/.output/.temp` and `*.config.*` — the paths v4 stopped excluding — cannot match. The one package with **no** `vitest.config.ts`, and therefore v4's default `include`, is `frontends/packages/config`: six tracked files, one test (`src/eslint.test.js`), no `dist/`, no `cypress/`. No config spreads `configDefaults.exclude` |
| 5 | `spyOn`/`fn` support constructors (an arrow `mockImplementation` now throws under `new`) | **no effect** | all eight `vi.spyOn` sites spy on plain functions — `console[method]` (checkout `secrets.test.ts`, twice), `globalThis.setInterval`, `globalThis.fetch`, `Math.random` four times — and none is constructed with `new` |
| 6 | Changes to mocking (`getMockName`, `restoreAllMocks` scope, automock isolation, automocked getters, `settledResults`, `invocationCallOrder` base 1) | **no effect**, each sub-item | there are no snapshots anywhere, so the `[MockFunction spy]` → `[MockFunction]` rename cannot bite; the three `vi.restoreAllMocks()` sites (checkout `secrets.test.ts:58`, stripe-js `redirect.test.ts:87` and `polling.test.ts:39`) exist to restore **manual `vi.spyOn` spies**, which is precisely what v4 still restores; there is no automock in the repository — `vi.mock` appears once (`frontends/apps/checkout/src/layout.test.tsx:30`) with an explicit factory and no `spy: true`; `invocationCallOrder`, `getMockName` and `settledResults`: 0 hits |
| 7 | Standalone mode with filename filter | **n/a** | nothing runs `vitest --standalone` |
| 8 | `vite-node` → Module Runner (`deps.optimizer.web` → `client`, `vitest/execute`, custom-environment `transformMode` → `viteEnvironment`, `VITE_NODE_DEPS_MODULE_DIRECTORIES`) | **n/a** | no `deps`, `optimizer`, `server.deps` or custom environment in any config; no `vitest/execute` import; no `VITE_NODE_*` variable |
| 9 | `workspace` → `projects` | **n/a** | no `vitest.workspace.*`, no `defineWorkspace`, no `test.workspace`, no `test.projects`. What this repository calls a "project" is a second **config file** run by a second package script (`sdks/nodejs/vitest.live.config.ts` via `test:live`; `sdks/stripe-compat/vitest.config.ts` via the e2e job's `compat`), which v4 does not touch |
| 10 | Browser provider rework (`@vitest/browser` → `vitest/browser`, provider factory, `browser.instances`) | **n/a** | `@vitest/browser` is in no manifest and in no lockfile entry; no `test.browser` block. Real-browser testing here is Cypress |
| 11 | Pool rework (`poolOptions`, `maxThreads`/`maxForks`, `singleThread`/`singleFork`, `minWorkers`, `threads.useAtomics`, `memoryLimit`) | **n/a** | none of those keys appears anywhere. `fileParallelism: false` (stripe-compat, sdk-node live) is a top-level option, not a `poolOptions` key, and is unchanged in v4 |
| 12 | Reporter updates (`basic` removed, `verbose` now flat, `onCollected`/`onTaskUpdate`/`onFinished` removed) | **n/a** | no `reporters` key in any config, no `--reporter` in any package script, justfile recipe or CI step, and no custom reporter |
| 13 | Snapshots with custom elements print the shadow root | **n/a** | `toMatchSnapshot`, `toMatchInlineSnapshot`, `toMatchFileSnapshot`: 0 hits; no `__snapshots__` directory exists |
| 14 | Deprecated APIs removed (`poolMatchGlobs`, `environmentMatchGlobs`, `deps.external`/`inline`/`fallbackCJS`, `browser.testerScripts`, `minWorkers`, **test options as a third argument**) | **n/a** | the third-argument form is the one that matters, because v4 **ignores** it rather than erroring — a dropped `retry`/`timeout` would be invisible. A sweep of every `*.test.ts`/`*.tsx` in `frontends/`, `sdks/` and `examples/` for an object literal in third position returns nothing; the only options-object-as-second-argument hit in the repository is `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts`'s `describe("the dashboard", { testIsolation: false }, …)`, which is Cypress and which vitest never loads |

Two things outside the guide, checked because they are how this repository would
break quietly:

- **`@vitest-environment` docblocks.** Five test files opt into `jsdom` from a
  `node`-default config
  (`frontends/apps/checkout/src/components/{screens.axe,checkout-view}.test.tsx`,
  `sdks/stripe-js/src/embedded.test.ts`,
  `examples/shop/src/components/{order-summary,test-numbers-panel}.test.tsx`).
  v4 still honours them, and a green run is sufficient proof: a render or an axe
  pass with no `document` throws, it does not skip.
- **Peers.** `.npmrc` sets `strict-peer-dependencies=true`,
  `auto-install-peers=false` and `node-linker=isolated`, so an unmet peer is an
  install failure, not a warning. `pnpm install --frozen-lockfile` passes
  (`Lockfile is up to date, resolution step is skipped`, `Already up to date`,
  exit 0). No package in the lockfile declares a peer on `vitest` at all —
  `@testing-library/jest-dom` reaches vitest through a subpath export, not a
  peer — so there is no `vitest@3` peer floor anywhere to pull a second copy in.
  v4 also stopped pulling `@types/node` in through its deprecated types;
  `pnpm -r typecheck`, inside `just lint-web`, is the gate for that and it
  passes.

## 4. Gates

`just ci` was started six times and reached the end three times. Two runs died in
`test-rust` on the same rootless-Docker container-creation timeout, on a machine
carrying other agents' testcontainers; neither is a result, and both are
recorded rather than dropped, and a fourth (run 5) died on an unrelated
shutdown-timing case that had passed on the identical Rust twenty minutes
earlier — this branch changes no Rust at all, and `test-rust`'s 1658/1658 is
reproduced on both green runs. All six under Node 22.23.2 (`.nvmrc`),
`CARGO_BUILD_JOBS=4`, `DOCKER_HOST=unix:///run/user/1000/docker.sock`, exit code
read from a file rather than from a banner.

| run | head | exit | where it stopped |
| --- | --- | --- | --- |
| 1 | `b5be024` | **100** | `test-rust`, at `checkout_sessions::a_confirm_past_the_horizon_…` — **an environment failure, not a code one**: `Error: the MTN stub container starts / Caused by: failed to create a container: Timeout error` after 255 s, on a box carrying other agents' testcontainers at load ≈ 10. 1411 of 1412 run passed; a `docker run --rm alpine true` smoke immediately afterwards exited 0. Not counted as a result |
| 2 | `b5be024` + this review's doc edits | **1** | **`test-web`, `@vpay/ui`** — finding F1. Everything before it passed; see the numbers below |
| 3 | the F1 fix's head | **100** | `test-rust` again, again on `failed to create a container: Timeout error` (the MTN wiremock stub), this time under `checkout_sessions::the_session_read_stops_handing_out_the_intents_secret_once_it_is_settled` after 245 s. A different test from run 1, the same cause: 95 containers on the daemon, load ≈ 9, other agents' testcontainers alongside. 1437 of 1438 run passed. Not counted as a result |
| 4 | the F1 fix's head | **0** | ran to the end — the gate for the code change |
| 5 | this documentation commit's head | **100** | `test-rust`, `vpay-server::cli` `worker::a_valid_config_lets_the_worker_boot` — `expected exit 0 after SIGTERM, got unix_wait_status(256)`, a shutdown-timing case that had passed on the identical Rust in run 4 minutes earlier. Environment again; 1323 of 1324 run passed. Not counted as a result |
| 6 | this documentation commit's head | **0** | ran to the end, every number identical to run 4 |

Recipe by recipe, from run 6 — the green one on the final head — with run 2's
numbers noted where they differ. Runs 4 and 6 agree on every number below;
the last commit in this branch is this file, so its own row was written from
run 4 and confirmed by run 6.

| recipe | result |
| --- | --- |
| `fmt-check` | ok |
| `clippy` | ok, no warnings |
| `verify` | **the twelve gates**, all ok, plus the advisory `verify-docs` report. `verify-status` 1 unimplemented item; `verify-errors` 19 error types; `verify-sdk-parity` 450 proving tests / 33 dated gaps; `verify-links` **1036 links in 197 tracked markdown files** (1033 in 195 before this review's two notes files were tracked); `verify-npm-scope` 2 publishable packages; `verify-serde` 85 types; `verify-repositories` 4 implementations, no generated schema exported; `verify-toolchain` `1.98.0`; `verify-migrations` 39 files |
| `test-rust` | **1658 run, 1658 passed, 0 skipped** (1513 s on run 4, 1466 s on run 6) — equal to `master`'s, as it must be: this branch touches no Rust |
| `test-doc` | **111 passed, 1 ignored** |
| `verify-ignored` | `0 ignored (expected 0), 45 test binaries (expected 45), 1658 total (minimum 1080)` |
| `lint-web` | ok — `build-sdk-node`, then `pnpm -r typecheck`, then `pnpm -r lint`, 15 of 15 packages |
| `test-web` | run 2: **failed** at `@vpay/ui` (F1). Runs 4 and 6: **1284 passed, 0 skipped, across 96 files** — `@vpay/checkout` 507/24, `@vaam-apps/vpay-sdk` 208/9, `@vpay/dashboard` 172/21, `@vaam-apps/vpay-stripe-js` 146/9, `@vpay-examples/shop` 102/12, `@vpay/ui` 74/18, `@vpay/config` 63/1, `@vpay/tokens` 8/1, `@vpay/api-client` 4/1 |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` (runs 4 and 6 — run 2 never reached it) |

Not in `just ci`, run separately:

| command | result |
| --- | --- |
| `pnpm install --frozen-lockfile` | exit 0, `Lockfile is up to date, resolution step is skipped` |
| `just audit-web` equivalent, `--audit-level=high` (both runs) | exit **0** — and exit 0 on `master` too, with the advisory present. See F2 |
| `pnpm audit --audit-level=moderate` | exit **0**, `No known vulnerabilities found` (on `master`'s lockfile: exit 1, both advisory entries) |
| `pnpm --filter @vpay/ui test` ×20, after the F1 fix | **20/20**, `74 passed (74)` |
| `pnpm --filter @vaam-apps/vpay-sdk test:live` with no `VPAY_BASE_URL` | exit **1**, thrown from `globalSetup` — still fails loudly under vitest 4, does not skip |

## 5. What was not done

- The `--audit-level=high` ceiling was **not** changed (F2) — the justfile
  reserves that choice to the maintainer, with reasoning.
- `just audit-web` was **not** added to `just ci`; it needs the network, and the
  justfile says why it is out.
- Nothing was pushed and no PR was opened.
- The only non-documentation change in this review is
  `frontends/packages/ui/src/components/select.test.tsx` (F1). No shipping
  source file and no configuration file was touched, by the draft or by this
  review — the whole change is ten `package.json` version strings, the lockfile,
  one test's synchrony, and documents.
- **Why** vitest 4's schedule differs was not chased to a mechanism. The
  regression is characterised (6/10 vs 0/10, one assertion, one line) and fixed
  at the assertion; whether the pool rewrite, the Module Runner or something
  else moves the focus tick was not established, and no claim about it is made
  here.
- The two `@testing-library/jest-dom` copies in the lockfile (`6.10.0` and
  `6.9.1`) predate this branch and were left alone.
- `frontends/apps/dashboard`'s `vitest run --passWithNoTests` remains a standing
  way for that suite to report green while collecting nothing. It predates this
  branch and it did not fire here (172 collected) — but it is the one place where
  a Vitest 4 include-glob change *would* have been silent, so it is named rather
  than fixed.
