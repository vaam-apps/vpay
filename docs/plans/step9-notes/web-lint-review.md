# exp4 (opus): sabotage review of the `pnpm -r lint` pass

2026-09-05. Reviewing `git diff 33d6c25..ab1e618` on `claude/exp4-web-lint-opus`
(three commits), against `docs/plans/step9-notes/web-lint.md` and the task brief.
Everything below was run in this worktree with
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `CYPRESS_INSTALL_BINARY=0`,
`CARGO_BUILD_JOBS=4`.

## 1. Claims in the notes, checked

| claim | verdict | evidence |
|---|---|---|
| ESLint 10 refused because it needs Node `^22.13` while `.nvmrc` is `22.11.0` under `engine-strict` | ~~**true**~~ → **RETRACTED later the same day (2026-09-05)** — the verdict checked the right fact and drew the wrong conclusion. `eslint@9.39.5`'s own engines are permissive, but its dependency `eslint-visitor-keys@5.0.1` declares the *same* `^20.19.0 \|\| ^22.13.0 \|\| >=24`, and `engine-strict` applies to every installed package — so staying on 9 raised the baseline anyway and CI failed `web`, `rust` and `e2e` at `pnpm install --frozen-lockfile`. This review ran on Node v24.20.0, not CI's, so it could not see it. See `web-lint.md`, "The Node baseline" | `pnpm view eslint@10 engines` → `{ node: '^20.19.0 \|\| ^22.13.0 \|\| >=24' }` on every 10.x through 10.10.0; `.nvmrc` is `22.11.0`; `.npmrc` sets `engine-strict=true`; CI's `web` job uses `node-version-file: .nvmrc` |
| 15 packages, 214 files | **true, exactly** | `eslint . --format json` per package, deduplicated: 214 paths. `git ls-files` finds 217 authored `.ts/.tsx/.js/.jsx/.mjs/.cjs`; the three not linted are the `next-env.d.ts` Next regenerates, which the config ignores by name. **No authored file falls outside the gate** |
| the 233 first-run findings were an unbuilt `dist/`, and `lint` now builds its deps | **true, and it is in the lint path, not a precondition** | `rm -rf sdks/*/dist && pnpm -r lint` → exit 0. The four packages that need it (`examples/shop`, `frontends/apps/checkout`, `frontends/tests/e2e`, `sdks/stripe-compat`) prefix their `lint` with `pnpm run deps` / `build:deps`, and the build lines appear in the run output |
| `--max-warnings 0` is load-bearing | **true, measured both ways** | `<img>` in `frontends/apps/dashboard/app/page.tsx`: `eslint .` exits **0** reporting a `@next/next/no-img-element` warning, `eslint . --max-warnings 0` exits **1** |
| `.storybook/{main,preview}.ts` typechecked by nothing | **true** | `tsc -p tsconfig.json --noEmit --listFiles` in `frontends/packages/ui` lists 7 files, neither of them. Left to a maintainer — correct call, changing that `include` changes a different gate |
| `just test-rust` did not complete (Docker) | **environment, confirmed** | re-run here with `DOCKER_HOST` set: **1159 passed, 0 skipped, 42 binaries, 672 s, no retry consumed**. No Rust file is touched by the diff |
| `just test-web` 660 passed | **true** | 660 before this review; 723 after, the 63 added being the new guard |
| the four "real bug fixes" | **all four are real, none is a behaviour change** | the credential-trace sink (`secrets.test.ts`) recorded `String(input)`, so a `Request` would have entered the sink as `[object Request]` and the leak grep would have passed over it — latent, since the app passes strings today, but the assertion was weaker than it read; `screens.tsx`'s `String(data.get('msisdn') ?? '')` differs from the new `typeof raw === 'string' ? raw : ''` only for a `File` entry, which a text input cannot produce — defensive, not behavioural; the Cypress `ShopOrder.status` union really did collapse to `string` via `\| string`, so `waitForOrderStatus` accepted a misspelling; the shop README prettier violation is real and was hidden by the broken sweep |
| "every rule family was given a deliberate violation and each caught it" | **five of six hold; one was proved in the only place it worked** | see finding 1 |

The 21 suppressions were read one by one. All are line-scoped, all carry a
prose reason immediately above, none is a rule switched off. The 12
`require-await` are on methods whose `async` an interface or a third-party
type demands (`ShopStore`, stripe-node's `Authenticator`, `@stripe/stripe-js`
source compatibility) — genuine false positives under the brief's rule. The 7
react-hooks and 1 `react-hooks/refs` say **"REAL finding, not suppressed as a
false positive"** in the source and say the same in `docs/status.md`; leaving a
payment page's entry state machine unreshaped by a lint pass is the right call
under CLAUDE.md, and the suppression keeps the finding visible where deleting
the rule would not. The `console.info` in the shop's webhook route prints an
event id, an event type and an outcome — no body, no signature, no secret.
Judged acceptable, all 21.

Worth recording, because it changes how much the suppressions can rot: ESLint 9
flat config defaults `linterOptions.reportUnusedDisableDirectives` to `warn`,
and every package runs `--max-warnings 0`. A suppression that stops doing work
therefore **fails the gate**. Measured: with the `no-console` block deleted,
`examples/shop` failed on `Unused eslint-disable directive` rather than passing.

## 2. Mutations

Each applied to a clean tree, the named check run, the tree restored, `git
status --porcelain` empty before the next. "Guard" is the new
`frontends/packages/config/src/eslint.test.js`, which did not exist at
`ab1e618`.

| # | mutation | `pnpm -r lint` at `ab1e618` | caught by |
|---|---|---|---|
| M7 | `console.log` in a Next page, an SDK source file and the shop's server code | **exit 1**, each naming file and rule (`pnpm -r` is fail-fast, so the recursive run reports only the first; per-package runs report all three) | the gate itself ✅ |
| M1 | delete the `no-console` rule block, plants still in place | **exit 0** for the dashboard and the SDK | **nothing** at `ab1e618` → guard (5 failures) |
| M2 | set `@typescript-eslint/no-restricted-imports` to `"off"`, `@/testing/**` and `../testing/**` imports still in place | **exit 0** for both packages | the hand-written vitest guards still caught it (`examples/shop` `no-runtime-imports.test.ts` failed) — the second lock works; and now the guard (2 failures) |
| M3 | drop `--max-warnings 0` from `frontends/apps/dashboard` | **exit 0** with a `@next/next/no-img-element` warning printed | **nothing** → guard (1 failure) |
| M4 | `sdks/nodejs/eslint.config.js` → `export default [];` | **exit 0**, that package linting zero rules | **nothing** → guard (5 failures) |
| M5 | delete `@vpay/tokens`'s `lint` script | **exit 0**, same "Scope: 15 of 16 workspace projects" line | **nothing** → guard (2 failures) |
| M6a | `/* eslint-disable no-console */` at the top of a file, `console.log` under it | **exit 0** | **nothing** → guard (2 failures) |
| M6b | `eslint-disable-next-line` with no reason comment | **exit 0** | **nothing** → guard (1 failure) |
| M8a | `tsconfigRootDir` pointed at a directory with no tsconfig | exit 1 | the gate ✅ |
| M8b | rename `sdks/nodejs/tsconfig.json` away | exit 1, **27 hard parse errors** (`was not found by the project service`) | the gate ✅ — `projectService` degrades loudly, never silently, so the type-aware claim cannot rot into a program-free parse unnoticed |
| F1 | scope `js.configs.recommended` back to `**/*.js` (i.e. revert this review's fix) | exit 0, 42 base rules silent on every `.ts` | **nothing** → guard (1 failure) |

## 3. Findings

| # | severity | where | evidence | state |
|---|---|---|---|---|
| 1 | **correctness** + misleading-claim | `frontends/packages/config/src/eslint.js` | `js.configs.recommended` was scoped to `["**/*.js","**/*.jsx","**/*.mjs","**/*.cjs"]`, and `tseslint.configs.recommendedTypeChecked` does not carry it — its `eslint-recommended` entry only switches the 23 compiler-covered base rules OFF again for `.ts`. `eslint --print-config sdks/nodejs/src/client.ts`: **48 active rules, against 90 after the fix**. The 42 missing include `no-fallthrough`, `no-debugger`, `no-unsafe-optional-chaining`, `no-constant-binary-expression`, `no-async-promise-executor`, `no-sparse-arrays`, `no-useless-escape`, `no-prototype-builtins`, `no-unsafe-finally`, `use-isnan` — none reported by `tsc` either. Planted `[1, , 3]`, `"\a"` and an empty `catch` in a `.ts` file: **not reported**; the same three in a `.mjs`: reported. The notes' proof table plants `no-dupe-keys` in `examples/webhook-receiver/index.mjs` — a `.js` file, the one place the family fired | **fixed**, `67d48b9` |
| 2 | rule-break + robustness | package manifests / `eslint.config.js` (all 15) | M3, M4, M5 above: a package can leave the gate three different ways and `pnpm -r lint` stays green and silent. This is the *original defect one level up* — the brief's "every workspace package gets a `lint` script that runs it" was true on the day and nothing kept it true | **fixed**, `9f4f32a` |
| 3 | rule-break | tree-wide | M6a, M6b: the brief forbids blanket disable files and requires a one-line reason; both were honoured by hand and enforced by nothing. ESLint's `reportUnusedDisableDirectives` catches a *stale* suppression, never one doing work | **fixed**, `9f4f32a` |
| 4 | nit | `examples/shop/src/app/api/vpay/webhook/route.ts` | two overlapping reason comments stacked above the `no-console` suppression, the second starting mid-sentence in lower case — an editing leftover | **fixed**, `1b9e861` |
| 5 | nit | `frontends/packages/config/src/eslint.js` | `NOT_SHIPPING_SOURCE` exempted `**/*.test.ts` and `**/*.test.tsx` from `no-console` but not `**/*.test.js`, so a JavaScript test would have been held to a rule its TypeScript sibling is not | **fixed**, `67d48b9` |
| 6 | misleading-claim | `docs/status.md`, `justfile`, `docs/plans/step9-notes/web-lint.md` | all three described the rule set as including `@eslint/js` recommended, which was true of 7 of the 214 files. Corrected by making the claim true rather than editing it down; the status row now records what the proof missed and the print-config numbers | **fixed**, this commit |
| 7 | — | `just test-rust` | not a finding: re-run with `DOCKER_HOST` set, **1159 passed, 0 skipped**. The previous pass reported it accurately as unmeasured rather than claiming it | n/a |

Nothing was found under money, secret-leak, or hard-coded-success. No
`eslint-disable` file, no rule deleted to make the tree pass, no test asserting
nothing, no mock adapter, no fake row. The 30 source fixes were read
individually; none changes behaviour except the four discussed above, and each
of those is a narrowing.

## 4. What the fix adds

`frontends/packages/config/src/eslint.test.js`, 63 assertions, run by
`just test-web` (`@vpay/config`'s `test` script was `echo 'no tests'`; it is now
`vitest run`, and `vitest ^3.2.7` was already in the lockfile for nine other
packages). It resolves each file's effective rule set the way ESLint itself does
— `ESLint#calculateConfigForFile`, from the consuming package's own directory,
so it exercises the real `eslint.config.js` and the real factory call, not a
copy of the intended configuration.

It asserts, and each assertion has a measured mutation behind it: every package
`git ls-files` finds declares a `lint` script, that script carries
`eslint . --max-warnings 0`, and its `eslint.config.js` reaches the shared
factory; the base rules are on for `.ts`; `no-console` is an error in the five
shipping files the proofs plant into and off in the three that print on purpose;
the `testing/**` ban is on in the checkout app and the shop; `no-floating-promises`
— the rule that cannot fire without a real type checker — is on in three
packages; react-hooks and the Next plugin reach the packages they are claimed
for; and no file in the tree carries a whole-file disable directive or a
suppression with no stated reason.

## 5. Gates, re-run in full

| gate | exit | reported |
|---|---:|---|
| `pnpm install --frozen-lockfile` | 0 | lockfile current (one entry added for `@vpay/config`'s vitest; committed) |
| `pnpm -r lint` | 0 | 15 of 15 `Done`, from `rm -rf sdks/*/dist` |
| `just lint-web` | 0 | `build-sdk-node` → typecheck → lint |
| `pnpm -r typecheck` | 0 | 11 projects |
| `just test-web` | 0 | **723 passed, 0 skipped, 0 todo** (660 before, +63 guard) |
| `just audit-web` | 0 | `No known vulnerabilities found`, both runs, answered on the first attempt — not a registry-unreachable result |
| `just verify` | 0 | no-mocks ok; verify-status ok **both directions**; verify-errors ok; verify-sdk-parity ok |
| `just test-rust` | 0 | **1159 passed, 0 skipped**, 42 binaries |
| `actionlint .github/workflows/ci.yml` | 0 | clean (workflow unchanged by this review) |

## 6. Verdict, and what was not checked

**Would `ab1e618` have been safe to merge without this review? Yes, but it
would have been believed to check more than it did.** Nothing in it is
dangerous, nothing is fake, no gap is hidden — the notes are unusually honest,
including about the gate that did not run and the typecheck hole in
`.storybook/`. But finding 1 means the gate's advertised rule set was
overstated for 97% of the files it covers, and the proof that would have caught
it was constructed in the one file type where the family worked. Findings 2 and
3 mean the same class of defect the task existed to fix — a gate that reports
its own absence as success — was left reachable by any future package.

Not checked: **CI has still never run this** — `actionlint` is clean and
`just lint-web` is what the `web` job invokes, but no job has executed. `just
test-e2e` was not run (Cypress binary skipped, compose stack not brought up), so
the two edited Cypress files are covered by typecheck and lint and by no
browser. The repository is still not prettier-clean tree-wide (~40 files, all
pre-existing, only `examples/shop` gates prettier); this review did not change
that. `frontends/packages/ui/.storybook/{main,preview}.ts` are still typechecked
by nothing — confirmed, and still left to a maintainer, since fixing it means
changing what `pnpm -r typecheck` covers. The guard asserts a *sample* of the 42
base rules rather than all of them, and asserts rule presence rather than
re-linting fixtures, so a rule downgraded from `error` to `warn` inside the
shared config would pass it (though `--max-warnings 0` still fails the lint).
