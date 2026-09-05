# exp4 (opus): making `pnpm -r lint` real

2026-09-05. Branch `claude/exp4-web-lint-opus`, base `master` `33d6c25`.

## The defect, measured

On `master`, from a clean `pnpm install --frozen-lockfile`:

```
$ pnpm -r lint
Scope: 15 of 16 workspace projects
frontends/packages/tokens lint$ eslint src
frontends/packages/tokens lint: sh: 1: eslint: not found
 ERR_PNPM_RECURSIVE_RUN_FIRST_FAIL  @vpay/tokens@0.0.0 lint: `eslint src`
spawn ENOENT
$ echo $?
1
```

Five of the fifteen packages declared a `lint` script. Four named a tool that
was installed nowhere in the workspace — `eslint src` in `@vpay/tokens`,
`@vpay/api-client` and `@vpay/ui`, `next lint` in `@vpay/dashboard` — so the
sweep died on the first of them before a single rule ran. The fifth,
`@vpay-examples/shop`, ran `prettier --check` and was never reached. The other
ten declared no `lint` script at all, and `pnpm -r` skips a missing script
silently.

`just lint-web` was `build-sdk-node` then `pnpm -r typecheck` — it never
invoked `pnpm -r lint`, which is the only reason the `web` gate was green
while claiming a lint. `@vpay/config/package.json` had declared an
`"./eslint": "./src/eslint.js"` export since the package was created, at a
file that did not exist.

## What landed

**One shared flat config**, `frontends/packages/config/src/eslint.js`, exported
as `@vpay/config/eslint` — the export that was already declared. It is a
factory (`vpayEslintConfig({ tsconfigRootDir, react, next, forbidTestingImports,
scripts, browser, outsideTsconfig })`) because `projectService` needs the
consuming package's own directory, which the shared file cannot know. Every
package's `eslint.config.js` is a three-line call into it.

**Versions pinned exactly** (in `@vpay/config`; `eslint` also as a peer):

| package | version |
|---|---|
| `eslint` | `9.39.5` |
| `@eslint/js` | `9.39.5` |
| `typescript-eslint` | `8.69.0` |
| `eslint-plugin-react-hooks` | `7.1.1` |
| `@next/eslint-plugin-next` | `16.3.4` |
| `globals` | `17.12.0` |

**ESLint 9, not 10, and that is a deliberate refusal.** 10.10.0 is the current
stable and every one of the four plugins accepts it (`typescript-eslint@8.69.0`
peers on `^8.57.0 || ^9.0.0 || ^10.0.0`). But ESLint 10 and `@eslint/js@10`
declare `engines.node: ^20.19.0 || ^22.13.0 || >=24`, `.nvmrc` is `22.11.0`,
CI's `web` job installs Node from `node-version-file: .nvmrc`, and `.npmrc`
sets `engine-strict=true`. Pinning 10 would mean moving the whole repository's
Node baseline as a side effect of a lint pass. 9.39.5's engines are
`^18.18.0 || ^20.9.0 || >=21.1.0`, which `22.11.0` satisfies. **Re-check when
`.nvmrc` next moves.**

> **Retracted later the same day (2026-09-05).** This paragraph is wrong and
> CI proved it: ESLint
> 9.39.5's *own* `engines` are permissive, but its dependency tree is not —
> `eslint-visitor-keys@5.0.1` declares `^20.19.0 || ^22.13.0 || >=24`, so
> pinning 9 moved the Node baseline just as pinning 10 would have. It is kept
> above rather than edited away, because the argument as written is what the
> pass believed and what its review confirmed. See
> **"The Node baseline"** at the end of this file.

**The rule set.** `@eslint/js` recommended; `typescript-eslint`
**recommended-type-checked** (`projectService: true` against each package's own
tsconfig, so `strict`, `noUncheckedIndexedAccess` and
`exactOptionalPropertyTypes` from `tsconfig.base.json` are what the rules
reason about); `eslint-plugin-react-hooks` `configs.flat['recommended-latest']`
on the four React packages; `@next/eslint-plugin-next` recommended +
core-web-vitals on the three Next apps (`@vpay/checkout`, `@vpay/dashboard`,
`@vpay-examples/shop` — the brief said "the two Next apps"; there are three);
`no-console` as an **error** in shipping source; and
`@typescript-eslint/no-restricted-imports` refusing `testing/**` from shipping
files in `frontends/apps/checkout` and `examples/shop`. The TS variant of that
rule, not the base one, so `import type` is caught too. The hand-written vitest
guards in both packages are untouched — this is a second lock, not a
replacement.

`no-unused-vars` is configured to honour a leading `_` (args, vars, caught
errors, destructured array elements). That is the repository's existing mark
for a deliberate discard — `machine.ts`'s `_sessionSecret`, `fixtures.ts`'s
`_secret` — and configuring the rule to respect a convention is not the same as
switching the rule off.

**Every package linted.** All 15 workspace packages run `eslint . --max-warnings 0`.
No package got an `echo no lint` escape: every one has at least one authored
`.ts`/`.tsx`/`.js`/`.mjs` file, `@vpay/tokens` included (it is not data-only —
`src/index.ts`, `src/index.test.ts`, `vitest.config.ts`). `--max-warnings 0`
on all of them, and that flag is load-bearing rather than decorative:
`@next/eslint-plugin-next`'s recommended set puts several rules at `warn`
(`no-img-element` among them), so without it a Next finding would pass the
gate silently. Measured — an `<img>` added to the dashboard's `page.tsx`:
`eslint app/page.tsx` exits **0**, `eslint app/page.tsx --max-warnings 0`
exits **1**. No finding of any severity is reported on the tree as it stands.

**Files actually linted, per package** (`eslint . --format json | length`):

| package | files | | package | files |
|---|---:|---|---|---:|
| `frontends/apps/checkout` | 58 | | `sdks/nodejs` | 30 |
| `examples/shop` | 48 | | `sdks/stripe-js` | 17 |
| `frontends/packages/ui` | 13 | | `frontends/tests/e2e` | 11 |
| `sdks/stripe-compat` | 10 | | `frontends/apps/dashboard` | 6 |
| `frontends/packages/tokens` | 4 | | `frontends/packages/api-client` | 4 |
| `examples/checkout-browser` | 4 | | `frontends/packages/config` | 3 |
| `examples/merchant-node` | 2 | | `examples/merchant-stripe-node` | 2 |
| `examples/webhook-receiver` | 2 | | **total** | **214** |

**`just lint-web`** is now `build-sdk-node`, `pnpm -r typecheck`, `pnpm -r lint`.
CI's `web` job keeps its name and now runs `just lint-web` rather than a copy
of its commands — the same reasoning the `audit-web` step already carried, so
the gate and the local check cannot drift. `actionlint` exit 0.

## The finding that was not a finding

The first full run reported **281 errors**. **233 of them were one missing
build artefact.** `@vpay/sdk` and `@vpay/stripe-js` publish their types through
an `exports` map into a gitignored `dist/`; unbuilt, TypeScript resolves those
imports to `any`, and `recommendedTypeChecked` reports the absence as a storm
of `no-unsafe-assignment` / `no-unsafe-member-access` / `no-unsafe-call` in
`examples/shop`, `frontends/apps/checkout`, `sdks/stripe-compat` and
`frontends/tests/e2e`. `frontends/apps/checkout` alone went **143 → 16** once
the two SDKs were built.

`typecheck` already knew this — `@vpay/checkout`'s runs `pnpm run deps` first,
`@vpay-examples/shop`'s runs `build:deps` — and the justfile already documents
it for `lint-web`. So `lint` was given the same prerequisite in all four
packages that need it (`sdks/stripe-compat` and `frontends/tests/e2e` gained a
`deps` script of their own). Not one line of source was changed to make those
233 go away.

## Findings: 51 raised in source, 30 fixed, 21 suppressed

Counted after the workspace is built as the gate builds it.

### Fixed (30)

- **3** `@typescript-eslint/no-require-imports` — `require('daisyui')` in the
  three `tailwind.config.ts` files, in `type: module` packages. daisyui 4.12.24
  ships `types`, so these became real `import daisyui from 'daisyui'`.
- **9** `no-unnecessary-type-assertion` — assertions that changed nothing,
  removed. One of them, `sdks/nodejs/src/stripe-auth.test.ts`, had a comment
  above it claiming "the cast is the documented divergence": stripe-node types
  `payment_method_types` as `Array<string>`, so the comment was wrong as well
  as the cast. The comment was corrected rather than deleted.
- **4** `no-base-to-string` — `String(input)` where `input` is
  `RequestInfo | URL`. A `Request` stringifies to `[object Request]`. One of
  these is `frontends/apps/checkout/src/lib/secrets.test.ts`'s credential-trace
  sink: it greps recorded URLs for a leaked secret, and a `Request` would have
  been recorded as a string containing no secret — the assertion would have
  passed over a real leak. And one is shipping source:
  `screens.tsx` fed `String(data.get('msisdn') ?? '')` to the MSISDN validator,
  where `FormData.get` is `string | File | null`.
- **4** `no-redundant-type-constituents` —
  `frontends/tests/e2e/cypress/support/shop.ts` declared
  `status: "unpaid" | "paid" | "failed" | "cancelled" | string`, which collapses
  to `string`. `waitForOrderStatus(orderId, expected)` takes that type, so a
  misspelled status was accepted. Narrowed to the four values
  `examples/shop`'s own `OrderStatus` holds.
- **3** `no-unused-vars` (genuine dead code) —
  `checkout-view.tsx` destructured `destination` and `secondsLeft` and then
  read `props.destination`/`props.secondsLeft`; `sdks/stripe-js/src/client.ts`
  imported `originOf` and never used it.
- **4** `no-unused-vars` on `_`-prefixed bindings — resolved by configuring the
  rule, not by editing source.
- **2** `no-duplicate-type-constituents` — `| undefined` on optional
  parameters in `sdks/nodejs/src/http.ts`.
- **1** `no-unsafe-member-access` — a field read off `JSON.parse`'s `any` in
  `sdks/nodejs/src/client.test.ts`; the round-trip is now typed.

### Suppressed (21), each `eslint-disable-next-line`, each with a reason

No blanket disable file, no `.eslintignore`, no rule removed.

| n | rule | where |
|---:|---|---|
| 12 | `@typescript-eslint/require-await` | `async` demanded by a contract with nothing to await: 7 `MemoryShopStore` methods implementing `ShopStore` (whose `PrismaShopStore` sibling does await), `api-client`'s `listPayments` (a `NotImplementedError` stub), `stripe-js`'s `loadStripe` (async purely for `@stripe/stripe-js` source compatibility — its own doc comment says so), `stripe-compat`'s bad-credential `authenticator` (stripe-node's `Authenticator` type is promise-returning), and two `fetch` stand-ins in tests |
| 6 | `react-hooks/set-state-in-effect` | `checkout-client.tsx` ×2, `return-client.tsx` ×2, shop's `cart-table.tsx`, `checkout-form.tsx` |
| 1 | `react-hooks/refs` | shop's `order-poller.tsx` |
| 1 | `no-console` | shop's `/api/vpay/webhook` route — one deliberate `console.info` of an event id and type, in a demo merchant with no logger |
| 1 | `@typescript-eslint/no-base-to-string` | `shop-embedded.cy.ts` — `.should('have.attr', 'src')` yields the attribute string at runtime; Cypress's types leave the subject typed as the element |

**The 7 react-hooks suppressions are real findings I did not fix, and the
comments in the source say so in those words.** `eslint-plugin-react-hooks` v7
folds the React Compiler lints into `recommended`, and these seven are its
verdict on state that is settled in an effect because only a browser can read
its source (`window.location`, `document.referrer`, `localStorage`) and on one
ref written during render. Fixing them properly means reshaping the checkout
page's entry state machine and the demo shop's cart — a behaviour change to a
payment page, which is not a lint pass's change to make. Suppressing at line
scope leaves the finding visible in the source forever; deleting the rule would
not have. `rules-of-hooks` and `exhaustive-deps` — the two rules the plugin
carried before v7 — report **zero** findings on this tree.

## Proofs

Deliberate violation planted in a Next app, an SDK and the shop, all three at
once. `pnpm -r lint` exit **1**:

```
sdks/nodejs lint: sdks/nodejs/src/client.ts
sdks/nodejs lint:   107:1  error  Unexpected console statement  no-console
examples/shop lint: examples/shop/src/server/context.ts
examples/shop lint:   1:1  error  '@/testing/memory-store' import is restricted from being
  used by a pattern. testing/** holds test doubles. AGENTS.md: no test double may be
  reachable from a shipping process  @typescript-eslint/no-restricted-imports
frontends/apps/checkout lint: frontends/apps/checkout/src/lib/money.ts
frontends/apps/checkout lint:   92:1  error  Unexpected console statement  no-console
```

The relative-path form of the testing ban, separately, in the checkout app:

```
frontends/apps/checkout/src/lib/entry.ts
   1:1  error  '../testing/fixtures' import is restricted from being used by a pattern.
   testing/** holds test doubles. …  @typescript-eslint/no-restricted-imports
```

All four files restored byte-identically (`git status --porcelain` empty for
them), and `pnpm -r lint` exit **0**, 15 of 15 `Done`.

### Every rule family fires — the config does not report nothing

CLAUDE.md's failure mode for this task is a lint config that reports nothing.
Each family was given a deliberate violation and each caught it; every file
restored from the index afterwards, tree clean.

| family | planted in | reported |
|---|---|---|
| `@eslint/js` recommended | `examples/webhook-receiver/index.mjs` | `Duplicate key 'a'` — `no-dupe-keys` |
| typescript-eslint **type-aware** | `sdks/nodejs/src/version.ts` | `Promises must be awaited …` — `@typescript-eslint/no-floating-promises` |
| react-hooks | `frontends/packages/ui/src/cn.ts` | `React Hook "useState" is called in function "notAHook" …` — `react-hooks/rules-of-hooks` |
| `@next/next` | `frontends/apps/dashboard/app/page.tsx` | `Using <img> could result in slower LCP …` — `@next/next/no-img-element` |
| `no-console` | a Next app, an SDK, the payer page `checkout.js` | `Unexpected console statement` |
| `no-restricted-imports` | the shop and the checkout app, `@/` and `../` forms | the AGENTS.md message |

The `no-floating-promises` case is the one that matters most: it is a rule
that cannot fire without a real type checker, so it is the evidence that
`projectService` is genuinely resolving each package's tsconfig rather than
falling back to a program-free parse.

The exemptions were checked in the same direction — a `console.log` appended
to `examples/merchant-node/index.mjs` (a command-line example) and to
`frontends/apps/checkout/src/lib/money.test.ts` (a test) each exits **0**,
while the same line in `examples/checkout-browser/checkout.js` (the payer
page, not exempt) exits 1.

## Gates run, with their real results

All on the authoring machine, 2026-09-05, `CYPRESS_INSTALL_BINARY=0`, from a
`rm -rf node_modules sdks/*/dist` + `pnpm install --frozen-lockfile`.

| gate | exit | what it reported |
|---|---:|---|
| `pnpm install --frozen-lockfile` | 0 | lockfile up to date; committed |
| `pnpm -r lint` | **0** | 15 of 15 `Done`, **from a tree with no `dist/` at all** — the lint scripts build what they need |
| `just lint-web` | 0 | `build-sdk-node` → typecheck → lint |
| `pnpm -r typecheck` | 0 | 11 projects |
| `just test-web` | 0 | **660 passed, 0 skipped, 0 todo** — checkout 302, sdk 172, stripe-js 119, shop 57, api-client 4, tokens 3, ui 3 (dashboard `--passWithNoTests`). Test-case counts in the two suites this pass edited are unchanged from `HEAD` (`client.test.ts` 78, `stripe-auth.test.ts` 21, both before and after) |
| `just audit-web` | 0 | `No known vulnerabilities found` on both runs; the 79 packages added introduce no advisory. Not a registry-unreachable result — both attempts answered on attempt 1 of 4 |
| `just verify` | 0 | no-mocks ok; **verify-status ok, both directions** — 1 unimplemented item, declared and still in shipping code; verify-errors ok (15 types); verify-sdk-parity ok (342 proving tests, 26 dated gaps) |
| `actionlint .github/workflows/ci.yml` | 0 | clean |
| `just fmt-check` | 0 | (as part of `just ci`) |
| `just clippy` | 0 | `--workspace --all-targets -D warnings` |
| `just test-doc` | 0 | workspace doctests |
| `just verify-ignored` | 0 | `0 ignored (expected 0), 42 test binaries (expected 42), 1159 total` |
| `just deny` | 0 | `advisories ok, bans ok, licenses ok, sources ok` |
| `just test-rust` | **100 — NOT PASSED** | see below |

**`just test-rust` did not complete, and this pass cannot claim it.** It failed
on `vpay-db::postgres an_abandoned_transaction_survives_a_rollback_it_cannot_send`
with `failed to create a container: Error in the hyper legacy client: client
error (Connect)` — testcontainers could not reach a Docker daemon. This brief
was run without Docker, and `DOCKER_HOST` is unset on this host (the daemon is
rootless, on `/run/user/1000/docker.sock`, which testcontainers reads only from
that variable). It is an environment result, not a code result: **no Rust file
was changed by this pass at all**, `cargo fmt --check`, `cargo clippy
--workspace --all-targets -D warnings` and all four `verify-*` gates are green,
and nextest reported `623 passed, 1 failed, 0 skipped` before `--fail-fast`
cancelled the remaining 535. Whether those 535 pass here is **unmeasured**.

So `just ci` as a whole is **not green on this machine**, and nothing in this
pass should be read as saying it is.

## What this pass did NOT do

- **No CI run exists.** `just lint-web` and the reworked `web` job were
  exercised on the authoring machine only. `actionlint` reports the workflow
  clean; that is not the same as the job having run.
- **`just test-e2e` was not run** — it needs Docker and a compose stack, which
  this pass was told not to use. Two Cypress files were edited
  (`support/shop.ts`'s `ShopOrder.status`, a disable comment in
  `shop-embedded.cy.ts`); `pnpm -r typecheck` covers them, no browser has.
- **`frontends/packages/ui/.storybook/{main,preview}.ts` are typechecked by
  nothing, and this pass did not fix that.** That package's tsconfig says
  `include: ["src", ".storybook"]`, but TypeScript's include-glob expansion
  skips dot-directories: `tsc -p tsconfig.json --listFiles` lists six files and
  neither of those two. So `pnpm -r typecheck` has never covered them. ESLint
  lints them minus the type-aware rules (`outsideTsconfig`). Changing the
  tsconfig is a change to a different gate and was left to a maintainer.
- **The repository is not prettier-clean and this pass did not make it so.**
  `pnpm exec prettier --check .` warns on ~40 files (docs, workflows, compose,
  `frontends/apps/checkout`'s single-quote style). Only `examples/shop` gates
  prettier, through its own `lint` script. Files this pass touched were
  reformatted **only where the file was already prettier-clean at `HEAD`**, so
  no package's existing style was churned. The one exception is deliberate:
  `examples/shop/README.md` had a pre-existing violation (`*does not prove*`
  where prettier wants `_does not prove_`) that the broken `pnpm -r lint` had
  been hiding since the shop landed. It is fixed, because the shop's own lint
  script now actually runs.
- **No rule was tuned to make a package pass.** The only rule-level
  configuration is `no-unused-vars`'s `^_` patterns.
- The `sandbox`/`demo` profile question, the Storybook 9 migration and the
  dependency overrides in root `package.json` were not touched.

## docs/status.md

The web-tooling row as this pass first committed it. It has been superseded
twice since — by the review pass (2026-09-05) and by the Node-baseline pass
(2026-09-05, below) — so read `docs/status.md` for the current text; this
block is kept as the snapshot of what the implementing pass claimed:

> | pnpm workspace, TS strict, `pnpm -r lint` | ✅ | `pnpm -r typecheck` clean. **`pnpm -r lint` became real on 2026-09-05 and was broken until then**: five of fifteen packages declared a `lint` script, four of them for an ESLint that was installed nowhere in the workspace, so the sweep exited 1 on the first one (`@vpay/tokens`, `eslint: not found`) before any rule ran — and `just lint-web` never invoked it, so the `web` gate claimed a lint it did not perform. Now **all 15 packages** run `eslint . --max-warnings 0` over one shared flat config exported from `@vpay/config/eslint` (the `./eslint` export that package had declared since it was created, at a file that did not exist): ESLint `9.39.5`, `typescript-eslint` `8.69.0` **recommended-type-checked** against each package's own tsconfig, `eslint-plugin-react-hooks` `7.1.1` on the four React packages, `@next/eslint-plugin-next` `16.3.4` on the three Next apps, `no-console` as an error in shipping source (tests, stories, Cypress specs and command-line examples exempt), and `no-restricted-imports` refusing `testing/**` from shipping code in `frontends/apps/checkout` and `examples/shop` — a second lock beside the hand-written vitest guards, which are unchanged. **214 files linted**, measured by `eslint . --format json`. `just lint-web` is now `build-sdk-node` → `pnpm -r typecheck` → `pnpm -r lint`, and CI's `web` job runs that recipe rather than a copy of its commands. **ESLint 9, not the current 10**, deliberately: ESLint 10 requires Node `^22.13.0` and `.nvmrc` (which CI reads) is `22.11.0` — re-check when `.nvmrc` moves. Proven in both directions on the authoring machine: a `console.log` in `frontends/apps/checkout/src/lib/money.ts` and in `sdks/nodejs/src/client.ts`, and an `@/testing/memory-store` import in `examples/shop/src/server/context.ts`, each make `pnpm -r lint` exit 1 naming the file and rule; removed, it exits 0 across 15 of 15. **51 findings were raised on the tree; 30 were fixed in source and 21 suppressed at line scope with a reason each** (12 `require-await` where an interface demands `async`, 7 React-Compiler advisories that need a component redesign and are marked as unfixed rather than as false positives, 1 deliberate `console.info` in the demo shop's webhook route, 1 Cypress typing gap) — no rule removed, no blanket disable file. A separate 233 findings on the first run were **an unbuilt `dist/` resolving to `any`**, not defects: `lint` now builds its workspace dependencies exactly as `typecheck` already did. **Every rule family was given a deliberate violation and each caught it** — `no-dupe-keys`, `no-floating-promises` (the one that cannot fire without a real type checker, so it is the evidence `projectService` resolves each package's tsconfig), `rules-of-hooks`, `@next/next/no-img-element`, `no-console` and the `testing/**` ban; see the notes. `--max-warnings 0` is load-bearing, not decorative: the Next plugin sets several rules to `warn`, and without the flag such a finding exits 0 (measured both ways). **No CI run of this change exists**, and `just test-rust` did not complete on the authoring machine either — it aborted on a testcontainers Docker connect failure (no Docker in this pass; no Rust file was changed by it). `fmt-check`, `clippy`, all four `verify-*`, `test-doc`, `verify-ignored`, `deny`, `lint-web`, `test-web` (660 passed, 0 skipped) and `audit-web` are green; see `docs/plans/step9-notes/web-lint.md`. |

---

## Correction, added by the review pass (2026-09-05)

Two claims above did not survive review. They are corrected here rather than
edited out, and in full in `docs/plans/step9-notes/web-lint-review.md`.

1. **"The rule set: `@eslint/js` recommended; …"** and the proof-table row
   "`@eslint/js` recommended | `examples/webhook-receiver/index.mjs` |
   `Duplicate key 'a'`" were true of `.js` files only. The config scoped
   `js.configs.recommended` to `**/*.js`/`.jsx`/`.mjs`/`.cjs`, and
   `tseslint.configs.recommendedTypeChecked` does not carry the base rules —
   its `eslint-recommended` entry only switches the compiler-covered ones OFF
   again for `.ts`. So the family reported nothing on 207 of the 214 files, and
   the one proof of it was planted in the one file type where it fired.
   `eslint --print-config` on `sdks/nodejs/src/client.ts`: **48 active rules,
   90 after the fix**; the 42 that were missing include `no-fallthrough`,
   `no-debugger`, `no-unsafe-optional-chaining`,
   `no-constant-binary-expression`, `no-async-promise-executor`,
   `no-sparse-arrays`, `use-isnan`. Fixed in `67d48b9`; no source file needed
   a change.

2. **`just test-rust` is green.** It was reported here, correctly, as
   unmeasured. Re-run with `DOCKER_HOST=unix:///run/user/1000/docker.sock`:
   **1159 passed, 0 skipped, 42 binaries, no retry consumed**. An environment
   result, as this file said it was.

The review also found that nothing kept a package inside the gate — a deleted
`lint` script, an `eslint.config.js` rewritten to `export default []`, or a
dropped `--max-warnings 0` each left `pnpm -r lint` at exit 0 with 15 of 15
`Done`, as did a whole-file disable directive and a reasonless line
suppression. `frontends/packages/config/src/eslint.test.js` now fails on each.

---

## The Node baseline — the refusal above was wrong (2026-09-05, later)

### What happened

The first CI run of this branch failed **three** jobs — `web`, `rust` and
`e2e` — at the same step, `pnpm install --frozen-lockfile`:

```
 ERR_PNPM_UNSUPPORTED_ENGINE  Unsupported environment (bad pnpm and/or Node.js version)

Your Node version is incompatible with "eslint-visitor-keys@5.0.1".

Expected version: ^20.19.0 || ^22.13.0 || >=24
Got: v22.11.0
```

All three jobs install Node with `node-version-file: .nvmrc`, `.nvmrc` was
`22.11.0`, and `.npmrc` sets `engine-strict=true`, which makes an unsatisfied
`engines` field an error rather than a warning.

### Why the argument this file made was wrong

The section **"ESLint 9, not 10, and that is a deliberate refusal"** above
reasoned about the wrong thing. It checked the `engines` field of `eslint` and
`@eslint/js` themselves — 9.39.5 declares `^18.18.0 || ^20.9.0 || >=21.1.0`,
which `22.11.0` satisfies — and concluded that staying on 9 kept the Node
baseline where it was. It did not check the **dependency tree**.
`eslint@9.39.5` depends on `eslint-visitor-keys@5.0.1`, which declares
`^20.19.0 || ^22.13.0 || >=24`, the *same* floor ESLint 10 declares. Under
`engine-strict=true` pnpm enforces the engines of every package it installs,
not only the ones named in `package.json`. The refusal therefore bought
nothing: the pass moved the Node baseline as a side effect of a lint pass
exactly as it said it would not, and merely failed to notice.

The review pass checked the claim and recorded it as **true**
(`web-lint-review.md`, row 1) — because it verified `pnpm view eslint@10
engines` against `.nvmrc`, which is the same partial check the implementing
pass made.

### The ruling

Raise the baseline deliberately rather than pin around it. Pinning
`eslint-visitor-keys` down, or relaxing `engine-strict`, would have kept a
number in `.nvmrc` that no longer describes what the repository can be built
with.

- **`.nvmrc`: `22.11.0` → `22.23.2`** — the current Node 22 LTS release
  (`curl -s https://nodejs.org/dist/index.json | jq -r '[.[] | select(.lts and
  (.version|startswith("v22")))][0].version'` on 2026-09-05).
- **`.npmrc`'s `engine-strict=true` is kept.** It is the reason this was a
  loud failure at install rather than a mystery at runtime.
- **Root `package.json` `engines.node`: `>=22.11.0` → `>=22.13.0`.** So the
  floor is stated where the repository states its own toolchain, and a stale
  Node fails naming *this repository* instead of a transitive package the
  reader has never heard of. `22.13.0` and not `22.23.2`, because `22.13.0` is
  the actual floor the dependency tree imposes; `.nvmrc` says which release to
  use, `engines` says what will work.
- **The Dockerfiles needed no change.** `frontends/Dockerfile` (three stages)
  and `examples/shop/Dockerfile` (two) build on `node:22-alpine`, a floating
  tag with no minor pin — `docker run --rm node:22-alpine node -v` is
  `v22.23.2`, the same release. `backends/Dockerfile` has no Node at all.
- **ESLint stays on 9.39.5, but as an unmade upgrade rather than a refusal.**
  ESLint 10's `^20.19.0 || ^22.13.0 || >=24` is now satisfied, every plugin
  already peers on `^10.0.0`, and nothing in this repository argues against
  it. The argument recorded above is retired; a future pass that wants 10 has
  no baseline objection to answer.

Recorded in the `justfile`'s `install-node` recipe (the toolchain comment),
`README.md`'s prerequisites, and `docs/status.md`'s web-tooling row.

### Proof, both directions, on the exact CI Node

Node installed with `nvm`; `pnpm@9.15.0` from `corepack`, as
`packageManager` pins it. Run on the rebased tree with every change above
applied.

| Node | command | exit | what it printed |
|---|---|---|---|
| `v22.23.2` | `pnpm install --frozen-lockfile` | **0** | `Done in 9s`; `pnpm-lock.yaml` unmodified afterwards (`git status --short` empty) |
| `v22.11.0` | `pnpm install --frozen-lockfile` | **1** | `ERR_PNPM_UNSUPPORTED_ENGINE … incompatible with "/…/exp4-opus". Expected version: >=22.13.0. Got: v22.11.0` |
| `v22.11.0`, root `engines.node` reverted to `>=22.11.0` | `pnpm install --frozen-lockfile` | **1** | `ERR_PNPM_UNSUPPORTED_ENGINE … incompatible with "eslint-visitor-keys@5.0.1". Expected version: ^20.19.0 \|\| ^22.13.0 \|\| >=24. Got: v22.11.0` |

The third row is the one that matters for honesty: it shows the raised root
`engines.node` is **not** masking the real constraint. Revert that one field
and the original CI failure comes back verbatim, from the dependency tree
itself.

### The review miss

Both the implementing pass and the review pass ran a Node newer than CI's
(`v24.20.0` on the authoring machine). Under it `pnpm install
--frozen-lockfile` succeeds, so **every local gate was green over a break that
only the runner could see** — `lint-web`, `test-web`, `audit-web`,
`typecheck`, all four `verify-*`, `test-doc`, `deny`, `clippy`. This is an
environment-parity gap, not a code one: no gate was weak, they were all run on
the wrong machine. A local `pnpm` gate is only evidence about CI if it runs on
the Node `.nvmrc` names. `nvm use` (or `docker run --rm -v "$PWD":/w -w /w
node:$(cat .nvmrc)-alpine …`) before believing a web gate.

### The outcome, on a runner

Run `33935680386` on `9cf3df0`, 2026-09-05 — **all six jobs green**. That is
the same tree as the commit this file ships in, apart from the documentation
lines below that record the result:

| job | result |
|---|---|
| `web` | success. `actions/setup-node@v4` → `Found in cache @ /opt/hostedtoolcache/node/22.23.2/x64`, `node: v22.23.2` — the release `.nvmrc` now names. `pnpm install --frozen-lockfile`, `audit-web`, `lint-web`, `pnpm -r test`, `build-storybook` all pass |
| `rust` | success. `1166 tests run: 1166 passed, 0 skipped` in 765 s; `verify-ignored: 0 ignored (expected 0), 42 test binaries (expected 42), 1166 total` |
| `e2e (compose)` | success. `All specs passed!` on both spec files — 7 tests and 4 tests |
| `supply chain`, `deploy (helm chart)`, `self-checks (no-mocks, status)` | success |

For contrast, the run this section is about — `33934371223` on the
pre-baseline commit — failed `web`, `rust` **and** `e2e`, each at
`pnpm install --frozen-lockfile`, each with
`Your Node version is incompatible with "eslint-visitor-keys@5.0.1". Expected
version: ^20.19.0 || ^22.13.0 || >=24. Got: v22.11.0`. That is the guard
failure and its repair, measured on the same machine CI runs on.

