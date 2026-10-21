# 2026-10-21 — three advisories against Storybook and `@vitest/mocker` turned `web` red on `master`

Branch `claude/happy-hopper-r3h6rz`, from `master` at `b7cc6c9` (#322). Two
version bumps. Both are in dev tooling, and the lockfile changes with them.
**No source file, route, migration, gate or `NotImplemented` token moved, and
no advisory was added to `auditConfig.ignoreGhsas`.**

| Package                                                                                                           | Before    | After     | Manifests                                                                                                                                                                                                          |
| ----------------------------------------------------------------------------------------------------------------- | --------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `storybook`, `@storybook/addon-a11y`, `@storybook/addon-docs`, `@storybook/addon-vitest`, `@storybook/react-vite` | `10.6.0`  | `10.6.3`  | `frontends/apps/{checkout,dashboard}` (exact pins)                                                                                                                                                                 |
| `vitest`                                                                                                          | `^4.1.11` | `^4.1.12` | all ten that declare it: `examples/shop`, `frontends/apps/{checkout,dashboard}`, `frontends/packages/{api-client,config,tokens}`, `sdks/{nodejs,stripe-compat,stripe-js}`, `sdks/tauri/tauri-plugin-vpay-checkout` |
| `@vitest/browser`, `@vitest/browser-playwright`                                                                   | `4.1.11`  | `4.1.12`  | `frontends/apps/{checkout,dashboard}` (exact pins)                                                                                                                                                                 |
| `@vitest/runner`                                                                                                  | `4.1.11`  | `4.1.12`  | `frontends/apps/checkout` (exact pin)                                                                                                                                                                              |

## What failed

`master` CI run 39668437087 on `b7cc6c9`, job `web`, step `audit-web`, at
20:15 UTC. `rust` and `e2e (compose)` passed in the same run, and so did
every self-check. `web` stopped at its audit, so the run never reached its
format, lint, test and Storybook steps.

```text
audit-web: production dependency graph only (attempt 1 of 4)
1 vulnerabilities found
Severity: 1 high (1 ignored)
audit-web: whole workspace, dev dependencies included (attempt 1 of 4)
│ critical            │ Storybook: Missing origin validation in Storybook      │
│                     │ preview allows code injection from malicious websites  │
│ Package             │ @storybook/addon-vitest                                │
│ Vulnerable versions │ >=10.0.0 <10.6.2                                       │
│ More info           │ https://github.com/advisories/GHSA-f2ph-cmg5-wxc8      │
  … the same advisory against `storybook`, 18 paths …
3 vulnerabilities found
Severity: 1 high (1 ignored) | 2 critical
error: recipe `audit-web` failed with exit code 1
```

#322 changes no JavaScript and no lockfile. The finding is in the lockfile
#321 released as v0.10.1, and it fails any branch cut from it.

**Two more advisories appeared after that run.** At 21:12 UTC, `pnpm audit
--audit-level=moderate` on `master`'s lockfile (`b7cc6c9`, in a clean worktree)
reported five findings rather than three:

```text
5 vulnerabilities found
Severity: 2 moderate | 1 high (1 ignored) | 2 critical
```

The two new findings are both against `@vitest/mocker`, and patched in
`4.1.12`:

- [GHSA-qxg6-89mg-rqv8](https://github.com/advisories/GHSA-qxg6-89mg-rqv8):
  code injection via unauthenticated manual mock registration,
  `>=2.1.0-beta.7 <4.1.12`.
- [GHSA-xp26-3454-mjgp](https://github.com/advisories/GHSA-xp26-3454-mjgp):
  Windows backslash traversal bypasses the redirect allowlist fix,
  `>=4.1.11 <4.1.12`. It is a follow-up to GHSA-82fw-gwwq-j7x9, which exp41's
  bump to `4.1.11` cleared on 2026-09-10.

When these two were published is not measured here. They are absent from
CI's report at 20:15 UTC and present in this one at 21:12 UTC, and that is
all this page claims about them. `audit-web` fails on moderate (issue #103),
so fixing only Storybook would still have left `web` red.

## What the advisories reach

**None of the three reaches the production dependency graph.** The recipe's
`--prod` pass on `master` reports only the ignored `braces` advisory and exits
0, both before and after this change. That pass covers what `frontends/Dockerfile`
ships and what `@vaam-apps/vpay-sdk`/`@vaam-apps/vpay-stripe-js` hand a
merchant. All three advisories are in tooling that runs on a developer's
machine or a CI runner:

- Storybook's dev server and preview;
- vitest's mocker, including the browser mode that `test-storybook` drives.

## The fix

- **Storybook `10.6.3`.** It is the newest patch on the 10.6 line, and
  `10.6.2` is the first that carries the fix. All five packages move
  together because each peers on `storybook` at its own version.
- **vitest `4.1.12`.** It is the first patched release. Every manifest that
  names the vitest family now names `4.1.12`, the same rule exp41 followed:
  no manifest pins a version the audit would flag.

**No `minimumReleaseAgeExclude` entry is needed.** pnpm 11 refuses a version
younger than 24 hours. npm dates the vitest family's `4.1.12` 2026-10-15
(14:40–14:51 UTC). It dates the five Storybook packages' `10.6.3` 2026-10-20,
between 09:47 UTC and 11:11 UTC (`@storybook/addon-docs`, the last). All of
them were past the window before this change was made, and `pnpm install`
resolved them without one.

**The lockfile diff is these packages and one transitive entry.** It has
158 insertions and 167 deletions. Every changed line except one is a
`storybook`, `@storybook/*`, `vitest` or `@vitest/*` entry, or a peer suffix
that names one. The exception: re-resolving the vitest family moved
`magic-string@0.30.21`'s `@jridgewell/sourcemap-codec` from `1.5.5` to
`1.6.0`, a version the lockfile already carried, and `1.5.5` dropped out.
`magic-string` is not in the production graph (`pnpm why -r --prod` finds
nothing), but `@tailwindcss/node` uses it, and Tailwind generates the
stylesheet `next build` ships. So both apps were built (below), and their CSS
compared with `master`'s. One peer range moved with the bump:
`@storybook/addon-vitest@10.6.3` peers on `vitest ^4.0.0 || ^5.0.0` and no
longer on `@vitest/runner`. `frontends/apps/checkout` still declares
`@vitest/runner` directly. That pin is bumped here, but this change does not
examine whether it is still needed.

The lockfile now resolves exactly one `vitest` and one `@vitest/mocker`, both
`4.1.12`. `@vitest/{expect,spy,utils,pretty-format}@3.2.4` remain, through
`storybook@10.6.3`, exactly as exp41 recorded them through `storybook@10.6.0`.
None of the four is named by any of these advisories.

## Measured

`pnpm audit --audit-level=moderate`, the two runs `just audit-web` makes:

| Tree                         | `--prod`                   | whole workspace                               | exit  |
| ---------------------------- | -------------------------- | --------------------------------------------- | ----- |
| `master` `b7cc6c9`           | 1 high (1 ignored), exit 0 | 5: 2 moderate, 1 high (1 ignored), 2 critical | 1     |
| Storybook bumped, vitest not | 1 high (1 ignored)         | 3: 2 moderate, 1 high (1 ignored)             | 1     |
| this branch                  | 1 high (1 ignored)         | **1 high (1 ignored)**                        | **0** |

The one remaining finding is `braces` GHSA-vfj7-8cjw-p6xm, ignored by id since
2026-10-09 ([2026-10-09-npm-advisories.md](2026-10-09-npm-advisories.md)).

The rest of the `web` job's steps, run on this machine against this branch
(Node 22.22.0, pnpm 11.18.0):

- `prettier --check .` (`just fmt-check-web`): all files clean.
- `pnpm --filter @vaam-apps/vpay-sdk build`, then `pnpm -r typecheck` and
  `pnpm -r lint` (`just lint-web`): exit 0.
- `pnpm -r test`: exit 0. **1661 passed, 2 skipped, in 106 files**:

  | Package                                 | Files | Passed | Skipped |
  | --------------------------------------- | ----- | ------ | ------- |
  | `frontends/packages/config`             | 1     | 66     | 0       |
  | `frontends/packages/tokens`             | 1     | 12     | 0       |
  | `frontends/packages/api-client`         | 1     | 4      | 0       |
  | `sdks/stripe-js`                        | 9     | 152    | 0       |
  | `sdks/nodejs`                           | 9     | 236    | 0       |
  | `sdks/tauri/tauri-plugin-vpay-checkout` | 8     | 71     | 0       |
  | `examples/shop`                         | 12    | 108    | 0       |
  | `frontends/apps/checkout`               | 30    | 556    | 1       |
  | `frontends/apps/dashboard`              | 35    | 456    | 1       |

  `master` at `b7cc6c9` measured the same way in a clean worktree gives the
  same figure for every package: 1661 passed, 2 skipped, in 106 files.

- `just build-storybook`'s body: both apps build. Its stylesheet assertion
  printed `checkout --color-base-100 defined 3x, referenced 24x` and
  `dashboard --color-base-100 defined 3x, referenced 25x`.
- `test-storybook`'s commands (dep caches cleared first, as the recipe does):
  checkout **26 of 26**, dashboard **36 of 36**.
- `pnpm --filter @vpay/{checkout,dashboard} build` (`next build`, what the
  images run): exit 0 for both, on this branch and on `master`. **The CSS is
  byte-identical.** `master` and this branch emit the same files with the
  same SHA-256: checkout `bef5683f7be46fda.css` (`c5939002…`), and dashboard
  `4aa3ffa5ded9485b.css` (`d9c3d329…`) and `5354ee8cbba713a2.css`
  (`e9654c01…`).

**One substitution in that last step.** The Chromium was not the one CI
uses. This repository's Playwright expects browser revision 1243. This
container ships revision 1194 and cannot download another. The run pointed
`PLAYWRIGHT_BROWSERS_PATH` at a scratch directory where revision 1194's
headless shell stands in under 1243's path. CI's `web` job installs the real
1243 (`just playwright-browser`), and its `test-storybook` step on this
branch is the measurement on the intended browser.

`cargo xtask`, with this page staged, exit 0 for each of the thirteen:

```text
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 20 error type(s), all classified; …
verify-sdk-parity: ok — 790 proving test(s) named in docs/sdks/parity.md all exist, 44 dated gap(s), …
verify-links: ok — 2566 repository link(s) in 478 tracked markdown file(s) resolve to a tracked path …
verify-npm-scope, verify-serde, verify-repositories, verify-toolchain, verify-migrations: ok
verify-versions: ok — 26 version references all say 0.10.1
verify-privacy-inventory: ok — 314 database column(s) classified …
verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s) agree with what this tree measures
```

## Not run here

- **Rust: nothing.** No Rust file changed. `rust` passed on `b7cc6c9` in the
  run above.
- **`e2e (compose)`.** Its browser specs run Cypress, which neither bump
  touches. Its images build `frontends/apps/{checkout,dashboard}` with
  `next build`, which bundles neither Storybook nor vitest. Both builds were
  run here, with byte-identical CSS (above). It passed on `b7cc6c9` in the
  run above.
- **Windows.** GHSA-xp26-3454-mjgp is specific to Windows paths. Nothing
  here runs on Windows, and the bump is the advisory's own remedy.
