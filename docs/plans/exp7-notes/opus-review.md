# exp7 — sabotage review of the `opus` arm, and what it changed

Sample 7 of the tier experiment, reviewing `git diff a81b6b6..52ba4b5` on
`claude/exp7-npm-scope-opus` — the rename of the published npm packages from
`@vpay/*` to `@vaam-apps/vpay-*`. The implementer's own report is
[`opus.md`](opus.md) and is left as it was written; the corrections are here.
Node `22.23.2` (`.nvmrc`), pnpm 9.15.0, `engine-strict`, `CARGO_BUILD_JOBS=4`,
no Docker.

**The one-line answer: the rename itself is correct and complete, and the
packages it renamed could not have been published.** On a clean clone
`pnpm pack` produced a tarball with no JavaScript in it — `dist/` is
gitignored and nothing built it — while `main`, `types` and `exports` all
pointed inside it. That is fixed here, along with a conformance suite that
had been dressed up as publish-ready, and the whole thing is now held by a
sixth `just verify` gate, because **nothing in this repository caught any of
it**.

---

## 1. Findings

| # | Severity | Finding | Where | Status |
|---|---|---|---|---|
| F1 | **publish-break** | `dist/` is gitignored and there is no `prepack`/`prepare`/`prepublishOnly` anywhere in `sdks/*`. With `dist/` absent — the state of every fresh clone — `pnpm pack` in `sdks/nodejs` produced **14 934 bytes / 4 files**: `LICENSE`, `package.json`, `README.md`, `scripts/mint-assertion.mjs`. No JavaScript, and `main`/`types`/`exports` all naming files that were not in the tarball. `sdks/stripe-js`: **8 588 bytes / 3 files**, same defect. Pre-existing at `a81b6b6`; the rename neither caused nor noticed it, but it is what "publishable" was asserting. | `sdks/nodejs`, `sdks/stripe-js` | **fixed** — `"prepack": "pnpm run build"` on both, and `verify-npm-scope` fails without it |
| F2 | **publish-break** | `sdks/stripe-compat` was given `publishConfig.access: "public"`, `repository`, `homepage` and `bugs` — the full publish-ready manifest — although its own README says "This is a conformance suite, not a library. It ships nothing and is `private: true`." It has no `build`, no `main`, no `exports` and no `files`, so `pnpm pack` on it yields **14 files including five `*.compat.test.ts`**, `vitest.config.ts`, `eslint.config.js` and `tsconfig.json`, and zero JavaScript. | `sdks/stripe-compat/package.json` | **fixed** — `publishConfig` removed, package stays private, `repository`/`homepage`/`bugs` kept (they are true) |
| F3 | **correctness (missing guard)** | Nothing catches the deletion of `publishConfig.access` — the one line between `npm publish` and a scoped package's default `restricted`. Mutation M3 below survived `pnpm install --frozen-lockfile`, `pnpm -r typecheck`, `just lint-web`, `just test-web`, `just audit-web` **and all five `just verify` gates**. | repository-wide | **fixed** — `cargo xtask verify-npm-scope`, 14 unit tests |
| F4 | **misleading-claim** | The brief and the notes both treat `pnpm install --frozen-lockfile` as the guard on the rename. It is not: reverting `sdks/stripe-js`'s own `name` to `@vpay/stripe-js` left the lockfile check at **exit 0** (`Lockfile is up to date, resolution step is skipped`), because pnpm keys `importers` by *directory* and a workspace package's own name never reaches `pnpm-lock.yaml`. Only a **dependent's** dependency key is checked there (M2b). | measurement | **recorded**, and covered by `verify-npm-scope`'s scope rule |
| F5 | **misleading-claim** | [`opus.md`](opus.md) says "`Cargo.toml`'s `repository = "https://github.com/vymalo/vpay"`, **and the git remote, which is also `vymalo/vpay`**". The git remote is not: `git remote -v` answers `git@github.com:vaam-apps/vpay.git`. `.xtask/src/main.rs:2894` already documents that `vymalo/vpay` resolves to `vaam-apps/vpay`. | notes | **corrected here**; the `Cargo.toml` URL is fixed |
| F6 | **misleading-claim (nit)** | The reference-count table's "Files before" column reads 120 for the combined pattern. `git grep -l -E '@vpay/(sdk\|stripe-js\|stripe-compat)\b' a81b6b6 \| sort -u \| wc -l` answers **128** (identical with and without `\b`). The line counts — 191 / 198 / 18 / **391** — are all exact. | notes | **corrected here** |
| F7 | **misleading-claim (nit)** | "The 'after' column excludes this notes file, which quotes the old names **13** times." It is **14** lines. The implementer noticed this itself and corrected it in an uncommitted edit at 09:42 that never reached a commit; the count in the committed file is still 13. Every other number in that section is exact: 81 residual lines = 71 `docs/plans/**` + 4 `docs/adr/**` + 6 `docs/status.md`, verified line by line. | notes | **corrected here** |
| F8 | **correctness** | `Cargo.toml`'s `repository = "https://github.com/vymalo/vpay"` was stale. `github.com/vymalo/vpay` 301-redirects to `vaam-apps/vpay`, which is why it survived; crates.io and tooling that does not follow redirects read the field literally. | `Cargo.toml` | **fixed** at the maintainer's direction |
| F9 | **nit** | Three files went from prettier-clean at `a81b6b6` to prettier-dirty at `52ba4b5` because the longer names moved a wrap: `examples/checkout-browser/README.md`, `examples/checkout-browser/index.html`, `sdks/rust/README.md`. The notes claim `examples/shop` was "the only package in the workspace whose `lint` gates prettier" — that is **true** (checked across all 16 manifests), which is why nothing failed; but "the other 40-odd renamed markdown files did not fail anything" is not the same as "did not regress". Measured: 35 of the 91 touched files were already prettier-dirty at BASE, 38 at HEAD. | 3 files | **fixed** — `prettier --write` on exactly those three |
| F10 | **rule-break** | Two ADRs — `docs/adr/0010` and `docs/adr/0015` — were amended in the working tree of this branch, and a commit `efc33cb` carrying them existed on the branch ref between 09:38 and 09:40 before being reset away. AGENTS.md makes ADRs immutable ("supersede, never edit"), the branch's own `docs/status.md` entry says superseding them "is a maintainer decision, not this change's", and `efc33cb`'s message reasons about `docs/plans/exp7-notes/haiku.md` — the *other arm's* notes — and lists residue (`examples/checkout-browser/index.html:92`, `examples/shop/.env.example:68`, the two Dockerfiles) that this tree had already fixed in `479ecfe`. It was the other arm's reviewer working in the wrong worktree. | working tree | **reverted**; ADR-0010 and ADR-0015 are byte-identical to `a81b6b6` |
| F11 | *not a finding* | The `frontends/tests/e2e/package.json` change was suspected of being an out-of-brief edit to its `typecheck` script. It is not: the two changed lines are the `deps` script's `pnpm --filter` target and the `@vpay/sdk` → `@vaam-apps/vpay-sdk` devDependency key. Both are the rename. | — | — |
| F12 | *not a finding* | Every `.rs`, `.sql` and `.yml` file the rename touched was checked line by line. All 40-odd edits are comments, doc comments, `///` prose, a compiled doctest line in `vpay-core/src/ids.rs`, one `ApiError::invalid_param` message string and one `assert!` message. `backends/migrations/0028_create-checkout-sessions.sql`'s single edit is a `--` comment explaining `ui_mode`. No behaviour changed; `cargo test --doc` and `clippy` agree. | — | — |
| F13 | **flagged, not taken** | Three live references still name the old organisation, and all three are attribution rather than addressing: root `package.json`'s `"name": "@vymalo/vpay"` (private, never published), `Cargo.toml`'s `authors = ["Vymalo"]`, and `LICENSE`'s `Copyright 2026 Vymalo`. Whom a project is copyrighted to is not a rename. **Maintainer's call.** Separately, `LICENSE` says of itself: "This file is an abridged pointer; replace it with the full text before publishing the repository" — that is a blocker on the first `npm publish`, not on this merge. | maintainer | open |
| F14 | **flagged, not taken** | ADR-0015 governs `docs/sdks/parity.md`, which *was* renamed, so the immutable record and the machine-checked document it governs now spell the same package differently. The implementer flagged this and left it; so do I. Superseding two ADRs for a package rename is a maintainer decision. | maintainer | open |

Everything the brief asked to be verified about the rename itself checked out:
`publishConfig.access: "public"` present on all three (F2 changes that
deliberately for `stripe-compat`); `repository`/`homepage`/`bugs` on
`github.com/vaam-apps/vpay` on all three; `files`/`exports`/`main`/`types`
resolving; `npm view` answering `E404` for all six names (three old, three
new) on 2026-09-05; `actionlint` exit 0; no `CHANGELOG` in the repository; the
Node `User-Agent` genuinely never contained the scope
(`vpay-sdk-node/${SDK_VERSION}`, `sdks/nodejs/src/auth.ts:488`); and the
residue at HEAD is exactly the 81 lines the notes claim, in exactly the three
document classes they name.

---

## 2. Mutations

Each applied to the working tree, the named check run, the result recorded,
then `git checkout --` and `git status --porcelain` empty before the next.

| # | Mutation | Check | Result |
|---|---|---|---|
| M1a | `examples/shop/src/server/vpay.ts` imports `@vpay/sdk` again | `pnpm --filter @vpay-examples/shop typecheck` | **caught** — `src/server/vpay.ts(13,28): error TS2307`, exit 2 |
| M1b | `frontends/apps/checkout/src/lib/types.ts` imports `@vpay/stripe-js` again | `pnpm --filter @vpay/checkout typecheck` | **caught** — `src/lib/types.ts(18,36): error TS2307`, exit 2 |
| M2 | `sdks/stripe-js/package.json`'s own `"name"` reverted to `@vpay/stripe-js`, nothing else | `pnpm install --frozen-lockfile` | **SURVIVED — exit 0**, `Lockfile is up to date, resolution step is skipped`. The stale `node_modules/@vaam-apps/vpay-stripe-js` symlink was not even repaired. Caught downstream only by `pnpm --filter @vaam-apps/vpay-stripe-js build` reporting `No projects matched the filters`, which is what `frontends/apps/checkout`'s `deps` script runs |
| M2b | The same revert applied to the **dependent's** key in `frontends/apps/checkout/package.json` | `pnpm install --frozen-lockfile` | **caught** — `ERR_PNPM_OUTDATED_LOCKFILE`, with the specifier diff printed. This is the half the lockfile actually guards |
| M3 | `publishConfig` deleted from `sdks/nodejs/package.json` | `pnpm install --frozen-lockfile`; `pnpm --filter @vaam-apps/vpay-sdk typecheck`; `cargo xtask verify-all` | **SURVIVED all three — 0, 0, 0.** Nothing in the repository saw it. This is why `verify-npm-scope` exists; `a_publishable_package_without_publish_access_fails` is the same mutation as a unit test |
| M4 | Private `@vpay/tokens` renamed to `@vaam-apps/vpay-tokens`, producer only | `pnpm install --frozen-lockfile`; `pnpm -r typecheck`; `verify-npm-scope` as first written | **survived all three (0, 0, 0)** — decided and documented: the publishable scope is now **reserved for `sdks/`**, so `verify-npm-scope` rejects any package outside `sdks/` wearing it, and M4 is caught. The converse rule — "the prefix implies publishable" — is deliberately **not** asserted, because `sdks/stripe-compat` is renamed *and* private by design and a gate saying otherwise would fail the tree the maintainer asked for |
| M5 | `sdks/nodejs/dist` and `sdks/stripe-js/dist` deleted, then `pnpm pack` | inspect the tarballs | **the F1 publish-break**, before the fix: 4 and 3 files, no JavaScript. After `prepack`: 36 files / 32 in `dist/` and 15 files / 12 in `dist/`, from the same cold tree |
| M6 | `prepack` changed to `echo hi` (unit test) | `verify-npm-scope` | **caught** — "which does not build" |
| M7 | A `"private": true` nested one level down inside a publishable manifest (unit test) | `verify-npm-scope` | **caught** after the helpers were rewritten to track brace depth. As first written the gate matched `"private"` by indentation-insensitive prefix and a nested key exempted the whole package — found by my own test, fixed before commit |

---

## 3. Final gate

Run on the finished tree, Node `v22.23.2`, `CARGO_BUILD_JOBS=4`, no Docker.

| Command | Exit | What it printed |
|---|---|---|
| `pnpm install --frozen-lockfile` | 0 | lockfile untouched by this review — no dependency edge changed |
| `pnpm -r typecheck` (warm) | 0 | 15 of 16 projects |
| `just lint-web` | 0 | build, `pnpm -r typecheck`, `pnpm -r lint` over 15 projects |
| `just test-web` | 0 | **723 tests, 45 files, 0 skipped** — checkout 302, nodejs 172, stripe-js 119, config 63, shop 57, api-client 4, tokens 3, ui 3. Identical to `a81b6b6` |
| `just audit-web` | 0 | `No known vulnerabilities found`, `--prod` graph and whole workspace |
| `just verify` | 0 | six gates: no-mocks; status (1 unimplemented, declared); errors (15 types, 14 `#[from]`); sdk-parity (**342** proving tests, 26 dated gaps); links (**682** links in **117** files); **npm-scope (2 publishable, 1 private, no retired name)** |
| `just docs-check` | 0 | verify-status + verify-links |
| `just fmt-check` | 0 | silent |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | no warnings |
| `cargo test -p xtask` | 0 | **144 tests**, of which **14** are `npm_scope_tests` |
| `cargo test --doc --workspace` | 0 | 86 passed, 0 failed |
| `actionlint` | 0 | silent |
| `pnpm pack` ×3, from a tree with no `dist/` | 0 | listings below |

### The three tarballs

Packed after `rm -rf sdks/nodejs/dist sdks/stripe-js/dist`, so `prepack` is
what put the JavaScript in them.

**`@vaam-apps/vpay-sdk` 0.1.0 — 46 736 bytes, 36 files, 0 test files.**
`dist/{auth,client,errors,form,http,index,stripe-auth,types,validate,version,webhooks}.{js,d.ts}`,
`dist/resources/{balance,checkout-sessions,events,payment-intents,refunds}.{js,d.ts}`
(32 files), plus `LICENSE`, `package.json`, `README.md`,
`scripts/mint-assertion.mjs`.

**`@vaam-apps/vpay-stripe-js` 0.1.0 — 30 873 bytes, 15 files, 0 test files.**
`dist/{client,embedded,errors,form,index,types}.{js,d.ts}` (12 files), plus
`LICENSE`, `package.json`, `README.md`.

**`@vaam-apps/vpay-stripe-compat` 0.0.0 — 21 477 bytes, 14 files, 5 of them
tests.** `src/{client,env,preflight}.ts`, `src/{errors,headers,idempotency,lifecycle,webhooks}.compat.test.ts`,
`eslint.config.js`, `vitest.config.ts`, `tsconfig.json`, `LICENSE`,
`package.json`, `README.md`. **No `dist/`, no JavaScript.** This is the
listing that decides it stays private, and it is why its `publishConfig` is
gone.

---

## 4. What was NOT run, and what is NOT checked

- **`just test-e2e`** — no Docker. So the Cypress specs, `sdks/stripe-compat`'s
  25 out-of-process cases against a live stack, and every
  `backends/tests/integration` test needing a container were not executed.
  Those are the suites that would catch a rename breaking a *running* stack
  rather than a build — in particular
  `frontends/tests/e2e/cypress/tasks/checkoutTasks.ts` and CI's
  `pnpm --filter @vaam-apps/vpay-stripe-compat compat` step. Typechecked and
  linted only.
- **`cargo nextest run --workspace`** — the container-backed suites need
  Docker; not run, and no Rust behaviour changed (F12).
- **`cargo xtask verify-citations`** — needs a GitHub token. This change cites
  no run id, PR or issue.
- **`npm publish --dry-run`** — needs a registry credential. The tarball
  listings above are what was measured; whether the `vaam-apps` npm
  organisation exists and grants publish rights is **not** verified here.
- **`verify-npm-scope` does not check** that `dist/` exists (gitignored — a
  gate needing a build would fail on a clean checkout for a reason that is
  not its subject) or the registry (needs the network). It also cannot see a
  retired name inside `.xtask/src/main.rs`, which is on its own allowlist
  because the check cannot name what it forbids without containing it.
- **`pnpm exec prettier --write .`** was not run. The tree is not
  prettier-clean on `master` (35 of the 91 files this change touched were
  already dirty at `a81b6b6`), so a blanket run would bury the change under an
  unrelated reformat. Only the three files F9 identifies were formatted.

---

## 5. A note on the environment, because it corrupted two measurements

This session's scratchpad directory is **shared between the agents reviewing
both arms**, and a helper script named `env.sh` was overwritten at 09:41:35 by
the other arm's reviewer with one that `cd`s into `.claude/worktrees/exp7-haiku`.
Every command of mine that sourced it after that ran in the *other* worktree.
Two Phase 1 measurements were taken there — the prettier BASE-vs-HEAD
comparison and mutation M4 — and both were **re-run in this worktree** before
being recorded above; the numbers in this file are the re-runs. The reverse
also happened: `efc33cb` (F10) was the other arm's reviewer committing to
*this* branch. Nothing was lost on either side — `git reflog show
claude/exp7-npm-scope-opus` accounts for every ref move, and no commit of this
review was ever reset — but for the tier-experiment ledger the lesson is
structural rather than about any model: **give each arm its own scratchpad,
or name every helper file after its arm.**

---

## 6. Verdict

**Safe to merge, with the fixes in this branch.** As delivered at `52ba4b5`
it was not: the rename was correct, but it renamed three packages toward a
registry that would have received a tarball containing no code (F1), and it
labelled a suite of five test files as publish-ready (F2). Neither would have
been caught by anything in the repository, on any machine, at any time — that
is what F3 measures and what the sixth gate now prevents.

Nothing is published, and nothing can be until someone writes a release
workflow; `LICENSE` must be replaced with the full Apache-2.0 text before that
happens (F13). ADR-0010 and ADR-0015 are untouched and byte-identical to
`a81b6b6` — the amendments that briefly existed were reverted (F10), and
superseding them stays a maintainer decision (F14).
