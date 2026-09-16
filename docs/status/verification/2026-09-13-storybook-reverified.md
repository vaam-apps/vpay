# Verification log — 2026-09-13, the Storybook restoration re-verified, and the docs that still said it was missing

Last verified: 2026-09-13, on `7730284c` (`origin/master`), Node `22.23.2`
(the `.nvmrc` pin, not the host's 24.20.0), pnpm `9.15.0`.

## Why this note exists

A task was filed to restore Storybook in `frontends/apps/checkout` after the
`@vaam-apps/ui` cutover deleted `@vpay/ui` and the install it hosted. **That
work was already done** — it landed on 2026-09-12 as PR #135 and is recorded in
[2026-09-12-storybook-restored.md](2026-09-12-storybook-restored.md). This note
does two things: it re-runs that work's gates on `origin/master` rather than
taking the earlier log's word for it, and it closes out six places in the
repository that still asserted the gap was open.

Nothing was re-implemented. If this note claimed to have built Storybook it
would be describing somebody else's commit.

## Re-run on `7730284c`, every number measured here

| Command                                       | Result                                                                                                           |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `just build-storybook`                        | exit 0; `frontends/apps/checkout/storybook-static/` 5.5 MB                                                       |
| `storybook-static/index.json`                 | **22** entries of type `story`, **1** of type `docs`                                                             |
| built stylesheet `assets/iframe-CHAomViR.css` | `--color-base-100` **defined 3×**, referenced **22×**                                                            |
| `storybook-static/sb-addons/`                 | `a11y-1`, `docs-2`, `vitest-3`, `storybook-core-server-presets-0`                                                |
| `just test-storybook`                         | exit 0 — **22 passed**, 0 skipped, 0 unhandled errors, from a cold cache (the recipe `rm -rf`s vite's dep cache) |

The exit codes above were read from a file the command wrote itself, not from a
harness banner. The stylesheet row is the one that matters most: it is the
direct check that the defect PR #135 found — a dropped `@import` leaving every
`var(--color-base-100)` referenced and the variable defined nowhere — has not
come back. A theme-less build is the failure mode that passes 22 stories and a
deliberately unreadable probe with them.

## The open question the justfile left, now answered

`justfile`'s lint paragraph asked whoever restored Storybook to re-measure
whether the new location's tsconfig actually reaches it, rather than assuming
`@vpay/ui`'s dot-directory finding still held for a different directory.

**It holds, unchanged.** The app's `include` is `**/*.ts` — broader than
`@vpay/ui`'s `[".storybook"]` was — and TypeScript's include-glob expansion
still skips dot-directories:

- `tsc -p tsconfig.json --noEmit --listFiles` lists **76** files under
  `frontends/apps/checkout/` and **0** under `.storybook/`.
- The decisive test, not a reading of the glob rules: append
  `const DELIBERATE_TYPE_ERROR: number = "not a number";` to
  `.storybook/main.ts`, and **both** `tsc --noEmit` and
  `eslint .storybook --max-warnings 0` exit **0**. Zero `error TS` lines.

So nothing in `just ci` type-checks the Storybook config. `eslint.config.js`
already names `".storybook/**"` in `outsideTsconfig` with this reasoning
written out, so the absence of type-aware rules there is by design. What
actually fails when the config breaks is `just build-storybook` and
`just test-storybook`, neither of which is in `just ci` — CI's `web` job runs
both. `src/a11y-gate.test.ts` runs inside `just ci` and locks the config's
load-bearing settings instead of its types, which is the property worth
holding: a config that type-checks and configures the wrong thing is the
failure this repository actually met.

## The mutation behind one comment edit

`src/testing/no-runtime-imports.test.ts`'s doc comment said no `*.stories.tsx`
file existed in the app and the suffix exclusion was dormant. Both halves are
now false. Measured before rewriting it:

- Baseline: **3 cases, 3 passed**, exit 0.
- Delete `stories` from the suffix pattern: exit 1, **1 failed | 2 passed**,
  failing on `src/components/checkout-screens.stories.tsx`.

The exclusion is load-bearing rather than decorative — it is the only reason
that file, which imports `../testing/fixtures` and `../testing/screen-states`,
does not fail the no-test-doubles-in-shipping-code rule.

## The six places that still said the gap was open

Each carried a dated correction rather than being edited away, per the
convention the status pages already use.

| File                                                                       | What it said                                               |
| -------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `docs/status/frontend.md` row 51                                           | "a chip is filed … and nothing has landed yet"             |
| `docs/status/frontend.md` row 112                                          | "A chip is filed to restore it; nothing has landed"        |
| `docs/flows/hosted-checkout/not-built-and-not-proven.md` (contrast bullet) | "A chip is filed … nothing has landed"                     |
| `docs/flows/hosted-checkout/not-built-and-not-proven.md` (stories bullet)  | "no visual-review surface and no browser-level a11y addon" |
| `frontends/apps/checkout/src/testing/no-runtime-imports.test.ts`           | "no `*.stories.tsx` file exists in this app today"         |
| `frontends/apps/checkout/src/testing/fixtures.ts`                          | "a chip is filed to restore Storybook inside this app"     |

`justfile`'s lint paragraph and `docs/status/verification/2026-09-12.md`'s
"what this group did not do" list were also updated — the first with the
measurement it asked for, the second with a forward pointer only, because that
group's record of its own scope is accurate and stays.

`docs/flows/hosted-checkout.md` needed nothing: its Status section already
carries a later "Storybook is back" entry that supersedes the two earlier
paragraphs, which is how that file is meant to read.

## What this note does not claim

- It does not claim the **dashboard** has a visual-review surface. It does not.
  There is no Storybook and no equivalent browser-level a11y gate for it, and
  the restoration was scoped to the checkout.
- It does not claim anything about the served page. This renders stories, not
  `compose.e2e.yml`.
- It does not retire `outcome-contrast.test.ts`. That remains the no-browser
  measurement, and issue #73's contrast half is unaffected by this note.
- It did not run the full `just ci`; the changes made here are doc comments, a
  justfile comment and markdown — no shipping code was touched. The gates that
  could be affected were run, and are quoted in the table below rather than
  summarised.

## The gates run on the edits in this note

| Command                                                           | Result                                                             |
| ----------------------------------------------------------------- | ------------------------------------------------------------------ |
| `just fmt-check-web` (`prettier --check .`)                       | exit 0 — "All matched files use Prettier code style!"              |
| `pnpm --filter @vpay/checkout test`                               | exit 0 — **27 files, 523 tests, 523 passed, 0 skipped, 0 ignored** |
| `pnpm --filter @vpay/checkout lint` (`eslint . --max-warnings 0`) | exit 0                                                             |
| `just build-storybook`                                            | exit 0 (see the table above)                                       |
| `just test-storybook`                                             | exit 0, 22 passed (see the table above)                            |

`just ci` itself was not run: it builds and clippies the whole Rust workspace,
and nothing in this change is Rust. `cargo fmt`/`clippy` inputs are untouched —
`git diff --stat` covers markdown, two TypeScript doc comments and a block of
justfile comments only.
