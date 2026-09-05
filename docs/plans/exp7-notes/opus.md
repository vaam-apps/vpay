# exp7 — renaming the published npm packages to `@vaam-apps/vpay-*`

Sample 7 of the tier experiment, `opus` arm. Base `a81b6b6`, branch
`claude/exp7-npm-scope-opus`, Node `22.23.2` (`.nvmrc`), pnpm 9.15.0,
`engine-strict`. No Docker.

## What changed

| Was | Is | Directory |
|---|---|---|
| `@vpay/sdk` | `@vaam-apps/vpay-sdk` | `sdks/nodejs` |
| `@vpay/stripe-js` | `@vaam-apps/vpay-stripe-js` | `sdks/stripe-js` |
| `@vpay/stripe-compat` | `@vaam-apps/vpay-stripe-compat` | `sdks/stripe-compat` |

Each of the three manifests also gained `publishConfig.access: "public"`
(none had a `publishConfig` at all; a scoped package defaults to
`restricted`, so publishing one without this line fails on a free
organisation), and `repository` (`git+https://github.com/vaam-apps/vpay.git`
plus a `directory`), `homepage` and `bugs` — none of which existed before
either.

## `npm view`: nothing was ever published, under either scope

Run 2026-09-05 against `https://registry.npmjs.org/` (`npm config get
registry`), unauthenticated (`npm whoami` → `E401`), so these are the public
registry's answers and not a private mirror's.

| Package | `npm view <pkg> version` | exit |
|---|---|---|
| `@vpay/sdk` | `npm error code E404` — `'@vpay/sdk@*' is not in this registry.` | 1 |
| `@vpay/stripe-js` | `npm error code E404` — `'@vpay/stripe-js@*' is not in this registry.` | 1 |
| `@vpay/stripe-compat` | `npm error code E404` — `'@vpay/stripe-compat@*' is not in this registry.` | 1 |
| `@vaam-apps/vpay-sdk` | `npm error code E404` — `'@vaam-apps/vpay-sdk@*' is not in this registry.` | 1 |
| `@vaam-apps/vpay-stripe-js` | `npm error code E404` — `'@vaam-apps/vpay-stripe-js@*' is not in this registry.` | 1 |
| `@vaam-apps/vpay-stripe-compat` | `npm error code E404` — `'@vaam-apps/vpay-stripe-compat@*' is not in this registry.` | 1 |

The first three are the check the rename depended on: **no version of any old
name exists, so nothing downstream can break.** The last three were not asked
for and are worth having anyway: the new names are free, so the rename does
not collide with a package somebody else owns.

## The state the brief's premise did not match, stated plainly

The brief calls these "the three **publishable** packages". On `a81b6b6`
they are not publishable, and neither is anything else in the workspace:

```
$ git ls-files '*package.json' | ... (name, private, publishConfig)
examples/checkout-browser      @vpay-examples/checkout-browser   private=true  publishConfig=-
examples/merchant-node         @vpay-examples/merchant-node      private=true  publishConfig=-
examples/merchant-stripe-node  @vpay-examples/merchant-stripe-node private=true publishConfig=-
examples/shop                  @vpay-examples/shop               private=true  publishConfig=-
examples/webhook-receiver      @vpay-examples/webhook-receiver   private=true  publishConfig=-
frontends/apps/checkout        @vpay/checkout                    private=true  publishConfig=-
frontends/apps/dashboard       @vpay/dashboard                   private=true  publishConfig=-
frontends/packages/api-client  @vpay/api-client                  private=true  publishConfig=-
frontends/packages/config      @vpay/config                      private=true  publishConfig=-
frontends/packages/tokens      @vpay/tokens                      private=true  publishConfig=-
frontends/packages/ui          @vpay/ui                          private=true  publishConfig=-
frontends/tests/e2e            @vpay/e2e                         private=true  publishConfig=-
package.json (root)            @vymalo/vpay                      private=true  publishConfig=-
sdks/nodejs                    @vpay/sdk                         private=true  publishConfig=-
sdks/stripe-compat             @vpay/stripe-compat               private=true  publishConfig=-
sdks/stripe-js                 @vpay/stripe-js                   private=true  publishConfig=-
```

**All sixteen workspace packages carry `"private": true`, the three renamed
ones included**, and no workflow in `.github/workflows/` runs `npm publish`
or `pnpm publish` or reads an `NPM_TOKEN` (grepped for all four). So:

- The answer to the brief's "if you find a fourth package that is publishable
  (no `private: true`), list it and rename it the same way" is **there is no
  such package — there is not even a first one.** Nothing was renamed beyond
  the three the brief names.
- `"private": true` was **left in place on all three.** Removing it is what
  would actually make them publishable, it is a one-word change with a real
  consequence (`pnpm publish -r` would start pushing them), and nothing in
  the brief asked for it. It is left to a maintainer, and `docs/status.md`
  says so.
- `sdks/nodejs/README.md` already said "**This package is `private: true` and
  is not published.**" and still does. Its install section still reads "There
  is nothing to install from a registry yet"; only the package name in it
  changed. It was **not** rewritten to `pnpm add @vaam-apps/vpay-sdk` as an
  instruction that works, because it does not work — printing a working
  install command for a package that cannot be installed is exactly the
  failure `CLAUDE.md` describes.

## Reference counts, before and after

`git grep -n -E '@vpay/(sdk|stripe-js|stripe-compat)'` over everything
tracked.

| Pattern | Files before | Lines before | Lines after |
|---|---|---|---|
| `@vpay/sdk` | 75 | 191 | 35 |
| `@vpay/stripe-js` | 78 | 198 | 46 |
| `@vpay/stripe-compat` | 9 | 18 | 4 |
| combined (unique lines) | 120 | 391 | 81 |

The "after" column excludes this notes file, which quotes the old names 13
times on purpose. The 81 are **71** in `docs/plans/**`, **4** in `docs/adr/**`
and **6** in the `docs/status.md` entry that records the rename and therefore
has to spell what it renamed. Those three are the whole of it — checked by
`git grep -n -E '@vpay/(sdk|stripe-js|stripe-compat)' | grep -v -E
'^docs/(plans|adr)/'`, which returns only those six `docs/status.md` lines.
**No source file, manifest, workflow, `justfile` recipe, Dockerfile, config,
lockfile or live document still names an old package.**

**Left deliberately, class 1 — closed, dated planning and step notes
(`docs/plans/**`, 64 lines in 18 files).** Rewriting them would falsify a
record of what was run on a date under a name that was then correct.

```
docs/plans/2026-09-04-step9-hosted-checkout.md          15
docs/plans/step9-notes/lane-5.md                         9
docs/plans/2026-09-03-step5c-stripejs.md                 8
docs/plans/2026-09-03-step5b-stripe-sdk.md               8
docs/plans/step9-notes/lane-3.md                         5
docs/plans/step9-notes/session-expired-review.md         4
docs/plans/step9-notes/session-expired.md                3
docs/plans/step9-notes/lane-2.md                         3
docs/plans/step8-notes/lane-f.md                         3
docs/plans/step9-notes/lane-6.md                         2
docs/plans/step9-notes/lane-5b.md                        2
docs/plans/step9-notes/web-lint.md                       1
docs/plans/step9-notes/lane-4.md                         1
docs/plans/step9-notes/lane-3b.md                        1
docs/plans/step9-notes/expired-session-confirm.md        1
docs/plans/step9-notes/expired-session-confirm-review.md 1
docs/plans/step8-notes/lane-c.md                         1
docs/plans/exp6-notes/opus.md                            1
docs/plans/exp6-notes/opus-review.md                     1
docs/plans/2026-09-03-step8-production-gate.md           1
```

**Left deliberately, class 2 — ADRs, which AGENTS.md makes immutable
("supersede, never edit"), 4 lines in 2 files.**

```
docs/adr/0015-sdk-parity.md                              3
docs/adr/0010-merchant-auth-private-key-jwt.md           1
```

This one is a genuine trade and it is not mine to settle: ADR-0015 describes
`docs/sdks/parity.md`, which *was* renamed, so the record and the document it
governs now spell the same package differently. Superseding two ADRs for a
package rename is a maintainer decision. **Flagged, not taken.**

`docs/status.md` (34 lines) **was** rewritten, and the dated entry added there
says so in as many words, including that a note quoting `pnpm --filter
@vaam-apps/vpay-sdk build` is the current spelling of a command that ran as
`pnpm --filter @vpay/sdk build`. The reasoning: `status.md` is the living
"what works today" contract and is read by grep; a package identifier in it is
a name, not a measurement.

## Things that look like they should have changed and did not

- **The Node SDK `User-Agent`.** It is `vpay-sdk-node/${SDK_VERSION}`
  (`sdks/nodejs/src/auth.ts:488`). It never contained the npm scope, and
  `sdks/rust` pins the header grammar to it byte-for-byte
  (`sdks/rust/README.md`), so changing it would break cross-SDK parity to no
  purpose. The brief listed `User-Agent` under "any user-facing string in the
  SDKs follows"; here nothing follows, because nothing named the package.
- **The Rust crate `vpay-sdk`** (`sdks/rust`, `publish = false`): crates.io
  has no scopes. Left alone, as the brief says. Its *prose* mentions of the
  npm package were updated (`sdks/rust/README.md`, `src/model.rs`,
  `src/validate.rs`).
- **`pnpm-lock.yaml`'s `importers` keys.** The brief expected these to
  change; they did not, and could not — they are workspace *directory paths*
  (`sdks/nodejs`, `sdks/stripe-js`, `sdks/stripe-compat`), not package names.
  What changed is the dependency entries inside them: 10 lines, listed below.
- **`Cargo.toml`'s `repository = "https://github.com/vymalo/vpay"`**, and the
  git remote, which is also `vymalo/vpay`. The brief requires the npm
  `repository` fields to name `github.com/vaam-apps/vpay`, and they do — but
  that leaves the repository disagreeing with itself across three
  organisations (`vaam-apps` in `deploy/helm/vpay/Chart.yaml` and
  `docs/runbooks/demo.md`, `vaam-store` in one planning note, `vymalo` in
  `Cargo.toml` and in `git remote -v`). Out of scope here, and **worth
  someone's attention**: a `repository` URL that 404s is worse than none.

## `pnpm-lock.yaml`

`pnpm install` regenerated it. The whole diff is **10 lines** — six workspace
dependency renames and one alphabetical reordering — with **zero** transitive
resolution churn:

- `examples/checkout-browser`, `examples/merchant-node`,
  `examples/merchant-stripe-node`, `sdks/stripe-compat` → `@vaam-apps/vpay-sdk`
- `examples/shop` → `@vaam-apps/vpay-sdk` and `@vaam-apps/vpay-stripe-js`
- `frontends/apps/checkout` → `@vaam-apps/vpay-stripe-js`
- `frontends/tests/e2e` → `@vaam-apps/vpay-sdk` (moved above `@vpay/config`)

## The gate

Run on the final tree, Node `v22.23.2`, `CARGO_BUILD_JOBS=4`, after
`rm -rf sdks/nodejs/dist sdks/stripe-js/dist` so the SDK builds are cold.

| Command | Exit | What it printed |
|---|---|---|
| `pnpm install --frozen-lockfile` | **0** | `Lockfile is up to date, resolution step is skipped` — on the *regenerated* lockfile |
| `pnpm -r typecheck` (cold, no `dist/`) | **2** | `cypress/tasks/checkoutTasks.ts(18,28): error TS2307: Cannot find module '@vaam-apps/vpay-sdk'`. **Pre-existing, and measured as such** — see below |
| `just lint-web` | **0** | builds the Node SDK first, then `pnpm -r typecheck` and `pnpm -r lint` over 15 projects |
| `pnpm -r typecheck` (warm) | **0** | 15 of 16 projects, all `Done` |
| `just test-web` | **0** | **723 tests, 0 skipped, 45 files**: `frontends/apps/checkout` 302, `sdks/nodejs` 172, `sdks/stripe-js` 119, `frontends/packages/config` 63, `examples/shop` 57, `frontends/packages/api-client` 4, `frontends/packages/tokens` 3, `frontends/packages/ui` 3. Identical to the count on `a81b6b6` |
| `just audit-web` | **0** | `No known vulnerabilities found` for the `--prod` graph and for the whole workspace; one attempt each |
| `just verify` | **0** | `verify-no-mocks` ok; `verify-status` ok (1 unimplemented item); `verify-errors` ok (15 types, 14 `#[from]` variants); `verify-sdk-parity` ok (**342** proving tests, 26 dated gaps); `verify-links` ok (**679** links in **116** files, up from 676/115 — this notes file and its three links) |
| `just docs-check` | **0** | `verify-status` + `verify-links`, both ok |

Not in the brief's list, run anyway because the rename edited Rust doc
comments (including one inside a compiled doctest, `vpay-core/src/ids.rs`):

| Command | Exit | What it printed |
|---|---|---|
| `just fmt-check` (`cargo fmt --all -- --check`) | **0** | silent |
| `cargo clippy --workspace --all-targets -- -D warnings` | **0** | no warnings |
| `just test-doc` (`cargo test --doc --workspace`) | **0** | **86 doctests passed, 1 ignored, 0 failed** |

### The one non-zero, and why it is not this change's

`pnpm -r typecheck` on its own fails on a tree with no `sdks/nodejs/dist` —
`frontends/tests/e2e`'s `typecheck` has no `deps` step, so pnpm can schedule
it before the SDK that its `checkoutTasks.ts` imports has been built. **This
was measured on the base commit `a81b6b6` before any edit**, and it failed
there in exactly the same place with exactly the same code:

```
a81b6b6 (before):  cypress/tasks/checkoutTasks.ts(18,28): error TS2307:
                   Cannot find module '@vpay/sdk' … Exit status 2
this branch:       cypress/tasks/checkoutTasks.ts(18,28): error TS2307:
                   Cannot find module '@vaam-apps/vpay-sdk' … Exit status 2
```

Same file, same line, same column, same error code; only the module name
moved. Re-run after any build of the SDK — which is what `just lint-web`, CI's
`web` job and `just ci` all do first, deliberately — and it is 0, on both
trees. **This is a pre-existing ordering defect in `frontends/tests/e2e`'s
`typecheck` script, not a regression, and it is not fixed here** (adding a
`deps` step to that package is a change to a package the rename had no other
reason to touch).

### One real regression, found by the gate and fixed

The first post-rename run of `just lint-web` **failed**: `examples/shop`'s own
`lint` script ends with `prettier --check "**/*.{ts,tsx,css,json,md,mjs}"`,
and the longer package name pushed that README's stack table out of prettier's
alignment. Fixed by `prettier --write examples/shop/README.md`, which touched
that one file and re-aligned that one table. It is the only package in the
workspace whose `lint` gates prettier (checked across all 16 manifests), which
is why the other 40-odd renamed markdown files did not fail anything.

## Not run

- **`just test-e2e`.** No Docker in this environment, as the brief states. So
  the Cypress specs, `sdks/stripe-compat`'s 25 out-of-process cases against a
  live stack, and every `backends/tests/integration` test that needs a
  container were **not** executed. Those are the suites that would catch a
  rename that broke a running stack rather than a build — in particular
  `frontends/tests/e2e/cypress/tasks/checkoutTasks.ts`'s `@vaam-apps/vpay-sdk`
  import and CI's `pnpm --filter @vaam-apps/vpay-stripe-compat compat` step.
  They are typechecked and linted here, and nothing more.
- **`cargo xtask verify-citations`** (`just docs-check-citations`): needs a
  GitHub token. Not part of `just verify` or `just ci`. This change cites no
  run id, PR or issue.
- **`pnpm exec prettier --write .`** (the second half of `just fmt`). The
  tree is **not** prettier-clean on `master` — `prettier --check .` reports
  dozens of pre-existing files, `README.md`, `package.json` and
  `pnpm-lock.yaml` among them — so running it would have buried a 106-file
  rename under an unrelated reformat. `just fmt-check`, which *is* the gate,
  is `cargo fmt --all -- --check` only and does not cover prose.
