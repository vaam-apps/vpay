# exp40 — the formatting PR: sabotage review, and what it changed

Reviewer: Opus, 2026-09-10. Branch `claude/exp40-formatting`, base
`44e0c80` (master, the merge of #104). The draft under review was
`a603125` — four haiku commits, `7dec365 d7417e2 79b9b5f a603125`, reachable
from this branch's reflog and from nowhere else after the regeneration
described in § 6.

Verdict up front: **not safe as drafted.** The draft's `just ci` was green and
its diff was semantically neutral for every non-markdown file, which is the
part it got right and which this review confirmed rather than took on trust.
It was wrong in two ways that a green gate could not see — a formatter left
free to rewrite the contents of code fences inside documentation that is
mostly pasted evidence, and a "make `just fmt` a no-op" claim with no gate
behind it — and one way that only mattered because of when this lands: an
eight-thousand-line reformat of `pnpm-lock.yaml` in the PR that goes in last.

---

## 1. Findings

| #   | Severity         | Finding                                                                                                                         | Fixed in  |
| --- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------- | --------- |
| 1   | correctness      | `embeddedLanguageFormatting: "auto"` rewrote the contents of **40 code fences in 22 markdown files**, several of them evidence  | `db8fd93` |
| 2   | gate-hole        | nothing enforced prettier: `fmt-check` was Rust-only, so the no-op was true on the day it was measured and nothing kept it true | `71dab68` |
| 3   | blast-radius     | `pnpm-lock.yaml` reformatted, +5444/-2837, in the PR that lands after three branches in flight                                  | `d26268b` |
| 4   | misleading-claim | "markdown verified format-only via `prettier --parser markdown` before/after" is circular and proves nothing                    | § 3       |
| 5   | misleading-claim | the file-type breakdown was `401`, not 405 — the four it dropped included `pnpm-lock.yaml`, the largest file in the diff        | § 6       |
| 6   | misleading-claim | `docs/status.md` said "all 405 tracked files have been formatted"; 405 was the number **changed**                               | _(this)_  |
| 7   | nit              | the justfile comment asserted migrations are not reformatted; true, but only because prettier has no SQL parser                 | `d26268b` |

### 1 — correctness: the formatter edited pasted evidence

Prettier's default `embeddedLanguageFormatting: "auto"` reformats the code
_inside_ fenced blocks in markdown, for every language it can parse. This
repository's markdown is largely transcripts, quoted logs and historical
notes, so that default is not a style choice here — it edits records.

Measured by parsing both sides of the draft's formatting commit with
prettier's own markdown parser and comparing every `code` node's value: **40
of 611 fences changed, in 22 files**, by language `json` 15, `markdown` 9,
`ts` 6, `yaml` 3, `tsx` 2, `css` 2, `jsonc` 1, `console` 1.

The ones that made a document false:

- **`docs/runbooks/demo.md`** — a single-line `tracing` JSON log line, pasted
  from a real run, pretty-printed across ten lines. `tracing`'s JSON layer
  emits one line; the runbook now showed an operator output the server does
  not produce, in the section that tells them what to grep for.
- **`docs/plans/exp9-notes/opus.md`** — a workflow fragment indented to show
  where it sits under `steps:` was dedented to column 0 by prettier's YAML
  printer. The snippet's indentation _was_ the claim.
- **`docs/rfc/README.md`** — the RFC template people copy gained blank lines
  between its headings.
- nine `markdown` fences quoting other documents (`step9-notes/lane-{1b,2,5}`)
  had their `*em*` rewritten to `_em_` _inside the quotation_, and fifteen
  `json` API-response examples in `docs/api/README.md`, `docs/flows/*.md`,
  `examples/merchant-curl/README.md` and `issue-47-notes/impl.md` were
  rewrapped.

**Fix and proof.** `embeddedLanguageFormatting: "off"` in `.prettierrc.json`
(`db8fd93`). Re-measured on a clean extraction of `44e0c80`'s `docs/` and
`examples/merchant-curl/`: 40 changed fences becomes **1** — trailing spaces
stripped from a `docker compose` transcript in `docs/runbooks/demo.md`, which
prettier does to every code block regardless of this option and which changes
no content. The same comparison over the regenerated commit `871045d` reports
`fences scanned: 611, changed: 1`.

### 2 — gate-hole: a no-op nothing kept true

`just fmt-check` was `cargo fmt --all -- --check` and nothing else. The task's
own decisive check — "a deliberately unformatted line in a `.ts` file makes
`just fmt-check` FAIL" — did not hold; the draft reported instead that a
deliberately unformatted `.ts` fails `prettier --check`, which is a different
sentence about a command no gate runs.

The only enforcement anywhere was `examples/shop`'s own `lint` script
(`prettier --check "**/*.{ts,tsx,css,json,md,mjs}"`), which covers one of
sixteen workspace packages and which reaches `just ci` through
`lint-web` -> `pnpm -r lint`. That package agrees with the root config because
prettier resolves `.prettierrc.json` upward from each file — checked, not
assumed: `prettier --find-config-path` from `examples/shop` answers
`../../.prettierrc.json`, no package carries a `prettier` key in its
`package.json` (the only two matches for `"prettier"` are the devDependency
in the root and in the shop), and `pnpm exec prettier --check .` from the
root and `pnpm -r lint` are both green on this head.

There is, however, a **second ignore file**: `examples/shop/.prettierignore`,
tracked, covering the `zen generate` output and `.next/`. It applies to that
package's own `prettier --check` and NOT to `just fmt` or `fmt-check-web`,
which run from the root and read only the root file. They agree today, and
that was measured rather than assumed: every path it names is either already
in the root `.gitignore` (`zenstack/{schema,models,input}.ts`, `.next/`) or a
file prettier cannot parse — the only tracked things under
`examples/shop/zenstack/migrations/` are three `.sql` files and a
`migration_lock.toml`. The root `.prettierignore` header now says so, and
says what would break the agreement: a generated `.ts`, `.json` or `.md`
appearing under one of those paths, which the root run would check and the
package's own lint would not.

`pnpm -r exec prettier --check .` — running prettier once per package, which
no repository script does — reports 48 files in `sdks/nodejs` and
`sdks/stripe-js`, all of them under `dist/`. That is an artefact of the
invocation: prettier resolves `.gitignore` from its working directory, and
those packages have no local one. The root run, which is the one `just fmt`
and `fmt-check-web` perform, excludes `dist/` correctly.

The draft's `docs/status.md` entry stated the hole plainly — "`just fmt-check`
runs only Rust formatting checks and passes" — and left it open. Writing a
gap down is better than hiding it, but this gap is the whole deliverable: a
formatting commit is a snapshot, and a snapshot with no gate behind it is
untrue at the next merge.

**Fix and proof.** `fmt-check` is now `fmt-check-rust` + `fmt-check-web`, and
CI's `web` job runs `just fmt-check-web` — the recipe, not a copy
(`71dab68`). Measured on this tree: `just fmt-check` exits **0** as committed;
with six spaces inserted before `export` in
`frontends/apps/dashboard/src/format.ts` it exits **1** and names the file.

### 3 — blast-radius: the lockfile

`pnpm-lock.yaml`, +5444/-2837. It is not a correctness problem — both sides
parse to the same 258143-character sorted-key JSON, and
`pnpm install --frozen-lockfile` is happy with either — but it is the wrong
file to format for two reasons. pnpm rewrites it in pnpm's layout on every
install that resolves anything, so `just fmt` would stop being a no-op at the
next dependency change; and this PR lands **last**, behind exp43, exp44 and
exp45, so an eight-thousand-line reformat of the one file every branch touches
is a conflict by construction. `.prettierignore` entry, with that reasoning in
it (`d26268b`).

### 4 — misleading-claim: the markdown proof was circular

The draft's proof for 189 markdown files was "`prettier --parser markdown`
before/after". Running prettier on prettier's own output proves prettier is
idempotent. It cannot detect a change prettier itself made, which is why all
40 fence rewrites passed it. The replacement is § 3 below: prettier's own
markdown **parser**, compared as a tree, plus a fence-by-fence comparison.

### 7 — nit: an accidental guarantee

The draft's justfile comment said migrations are not reformatted. True —
prettier has no SQL parser — but nothing in the configuration said so, and
`backends/migrations/README.md` was in scope and was only left alone because
it was already prettier-clean. `.prettierignore` now carries
`backends/migrations/*.sql` with the reason, an entry that is inert today and
exists so the guarantee survives prettier gaining a SQL parser. The README is
deliberately left formatted; nothing hashes it.

---

## 2. What "format-only" was actually proven with

`git diff -w --ignore-blank-lines` is near-useless for a prettier diff: **all
215** non-markdown files in the formatting commit still show a
non-whitespace hunk, because prettier
changes quote style, adds trailing commas, adds parentheses and inserts JSX
whitespace expressions. Each class was checked with a parser instead. All
numbers below are for the regenerated commit `871045d`.

| Class                     | Files | Method                                                                                | Result                                   |
| ------------------------- | ----- | ------------------------------------------------------------------------------------- | ---------------------------------------- |
| `.ts` `.tsx` `.js` `.mjs` | 204   | TypeScript 5.9.3 parse; full node-kind + literal-value stream compared                | **0 differing**                          |
| the same, comments        | 204   | comment trivia scanned on both sides; sets compared; directive-comment order compared | **0 differing** (1 explained false hit)  |
| `.yml` `.yaml` `.json`    | 8     | parsed; values compared with keys sorted                                              | **0 differing**                          |
| `.md`                     | 189   | prettier's own markdown parser; mdast compared with positions dropped                 | **0 differing**                          |
| `.md` code fences         | 611   | every `code` node's value compared                                                    | **1 changed** (trailing spaces, console) |

Two normalisations were applied to the TypeScript comparison, and both are
prettier additions that preserve meaning rather than differences to be waved
away:

- `ParenthesizedExpression` — prettier wraps multi-line JSX and expressions in
  parentheses. 14 files.
- six inserted `{" "}` JSX text nodes, which _preserve_ a rendered space that
  a line break would otherwise eat (`signed-in-bar.tsx`'s
  `merchant{" "}<code>`). **No `{" "}` was removed**, checked separately:
  `grep -c` on the diff gives 6 added, 0 removed.

The one comment "difference" is a limitation of the scanner, not a change:
`frontends/apps/checkout/src/lib/secrets.test.ts` contains
`` `https://shop.example/done?sid=${…}` `` inside a template literal, which a
context-free scanner reads as a `//` line comment on both sides; prettier
split that call across lines, so the pseudo-comment's tail differs. The file's
AST is identical, which is the check that matters.

### The non-whitespace hunks that were read by eye

Every non-markdown file outside the 204 TypeScript ones, in full:

| File                                                | Change                                              | Neutral?                            |
| --------------------------------------------------- | --------------------------------------------------- | ----------------------------------- |
| `.github/workflows/release.yml`                     | `tags: ['v*']` -> `tags: ["v*"]`                    | yes                                 |
| `compose.yml`, `compose.e2e.yml`                    | the `healthcheck.test` arrays broken across lines   | yes; `!override` tags preserved     |
| `…/fixtures/webhook-livemode-resolved-secret.yml`   | `secrets: [...]` moved to the next line             | yes; the config test parses it      |
| `deploy/helm/vpay/values.schema.json`               | reindented                                          | yes; parse-identical                |
| `frontends/apps/{checkout,dashboard}/tsconfig.json` | `"exclude": ["node_modules"]` collapsed to one line | yes                                 |
| `package.json`                                      | `engines` and one `peerDependencies` expanded       | yes                                 |
| `…/postcss.config.js` (×3)                          | single -> double quotes                             | yes                                 |
| `…/globals.css` (×2), `ui/src/styles.css`           | `@import '…'` -> `"…"`, `[data-theme='…']` -> `"…"` | yes; `@plugin 'daisyui'` left alone |
| `examples/webhook-receiver/index.mjs`               | rewrapped, trailing commas                          | yes; AST-identical                  |

### The gates that read documentation as data

Re-run rather than reasoned about:

- **`just verify-links`** — 1041 repository links in 202 tracked markdown
  files resolve. Prettier reflows reference-style links and pads tables; this
  is the gate that would have caught it.
- **`just verify-status`** — 1 unimplemented item, declared and still in
  shipping code. A `NotImplemented("…")` token reflowed across a line break
  in `docs/status.md` would break the scanner; none was.
- **`the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`**
  (`backends/tests/integration/tests/postgres_smoke.rs`) — parses the
  `UPDATE` out of `docs/runbooks/migrations.md` and runs it against a live
  `postgres:16-alpine`. Passes. The `sql` fence in that file is byte-identical
  (prettier has no SQL parser); only its tables and `*em*` markers moved.
- **`just verify-sdk-parity`** — 450 named tests, 33 dated gaps, 35 rows read
  out of `docs/sdks/parity.md`, whose tables prettier repadded. Passes.
- **`just verify-migrations`** — 39 migrations match `MANIFEST.sha256`.
  Nothing under `backends/migrations/` is in the commit.

### What the run did not touch

`git status --porcelain` after `just fmt` lists 404 modifications and no
addition, deletion or rename. No `.rs`, no `.toml`, no `.lock`, no
`pnpm-lock.yaml`, nothing under `backends/migrations/` or
`deploy/helm/**/templates/`, and `malformed.yml` byte-identical. `.e2e/`,
`dist/`, `storybook-static/` and `node_modules/` are out of scope through
`.gitignore`, which prettier 3 reads by default — they are deliberately not
repeated in `.prettierignore`.

---

## 3. Reproducibility

- `just fmt` on the committed tree: exit 0, **0 files changed**.
- `just fmt-check`: exit 0.
- A fresh `git clone` of this branch, with the root `node_modules` symlinked
  in: `prettier --check .` exits 0 and `prettier --write .` leaves the clone
  clean. The committed bytes are what prettier produces from them.
- Prettier is **3.9.6**, resolved from `"prettier": "^3.4.0"`.

**Left to the maintainer.** That caret is the one thing that can make this
untrue without anybody touching a file: `--frozen-lockfile` pins 3.9.6
everywhere today, but the next lockfile refresh may pull a release whose
defaults differ, and `fmt-check-web` would go red on master for a reason
nobody changed. An exact pin — the treatment `cratestack` already gets in
`Cargo.toml` and the justfile — is the obvious remedy, and it is a dependency
decision rather than a formatting one, so it is recorded here and in
`docs/status.md` rather than taken.

---

## 4. Gates, recipe by recipe

`just ci` on the draft (`a603125`) and again on this head. Exit code read out
of a file, not off a banner. Read the `test-rust` row with the paragraph
after the table: on this head that recipe was never reached, six times, for a
reason outside this branch.

| Recipe           | Result                                                             |
| ---------------- | ------------------------------------------------------------------ |
| `fmt-check`      | ok (on this head, prettier included)                               |
| `clippy`         | ok                                                                 |
| `verify`         | ok — the twelve gates                                              |
| `test-rust`      | **1666 passed, 0 skipped** on `a603125`; see below for this head   |
| `test-doc`       | 5 doctests, ok                                                     |
| `verify-ignored` | 0 ignored (expected 0), 46 test binaries (expected 46), 1666 total |
| `lint-web`       | ok — `pnpm -r typecheck`, then `pnpm -r lint` over 15 packages     |
| `test-web`       | **1284 passed** (63 + 8 + 146 + 208 + 4 + 74 + 102 + 172 + 507)    |
| `deny`           | advisories ok, bans ok, licenses ok, sources ok                    |

1666 and 1284 are the numbers on the base commit too, and for the Rust half
that is structural rather than a coincidence to be re-measured: the branch
contains **zero** `.rs`, `.toml` and `.lock` changes
(`git diff --numstat 44e0c80 HEAD -- '*.rs' '*.toml' '*.lock'` is empty), so
no formatting commit can move the Rust count.

### `test-rust` on the final head: reported, not papered over

`just ci` was run **six times** on this head. Every run reached `test-rust`
and every run died inside it, always on a testcontainers failure and never on
an assertion:

| Attempt | Tests run before it died | Test that died                                                    | Error                                     |
| ------- | ------------------------ | ----------------------------------------------------------------- | ----------------------------------------- |
| 2       | 1283/1666                | `vpay-db::repositories reschedule_clears_the_lease…`              | `failed to create a container: Timeout`   |
| 3       | 1489/1666                | `vpay-tests-integration::dashboard_read_surface the_detail_read…` | `failed to create a container: Timeout`   |
| 4       | 1209/1666                | `vpay-db::repositories a_flow_label_the_schema_cannot_name…`      | `failed to create a container: Timeout`   |
| 5       | 1235/1666                | `vpay-db::repositories an_elapsed_rate_limit_window…`             | `container is not ready: startup timeout` |
| 6       | 1504/1666                | `vpay-tests-integration::invoices the_two_terminal_transitions…`  | `lost host port 48612 (address in use)`   |
| 7       | 1504/1666                | `vpay-tests-integration::invoices the_two_terminal_transitions…`  | `failed to create a container: Timeout`   |

Attempt 6's stderr names the cause outright:
`vpay-testkit: postgres:16-alpine start attempt 1/4 lost host port 48612
(address already in use)` — the rootlesskit host-port race that
`.config/nextest.toml`'s `postgres-containers = { max-threads = 1 }` group
exists to remove. That group serialises **this repository's** container
starts; it cannot serialise against the three sibling agents running their
own suites against the same rootless daemon at the same time. Measured on
the host while this was happening: load average 9–17, `docker ps` showing
other agents' `postgres:16-alpine` containers created seconds earlier, and
`docker run -d postgres:16-alpine` taking **9.2 s** on an otherwise idle
invocation. Memory (107 GB free) and disk (134 GB free) were not the
constraint. The daemon was left alone rather than restarted, which is the
documented remedy for this fault, because a restart would have killed the
sibling agents' containers and the `vpay-demo` stack.

What that leaves, stated plainly rather than rounded up to green:

- **Measured on this head**: `fmt-check` (prettier included), `clippy`,
  `verify` (the twelve), `test-doc`, `verify-ignored`, `lint-web`,
  `test-web`, `deny` — all ok, all with the counts in the table above. The
  five recipes that follow `test-rust` were run individually after the sixth
  `just ci` aborted, so nothing in the pipeline is unmeasured except the
  Rust suite's execution.
- **`verify-ignored` is the count, measured on this head**: it reports
  `0 ignored (expected 0), 46 test binaries (expected 46), 1666 total`, and
  it counts what nextest **lists**, not what a run completes. So the claim
  "this branch does not change the Rust test count" is measured here, not
  inferred.
- **Not measured on this head**: 1666 Rust tests actually executing. It was
  measured on `a603125`, whose `.rs`, `.toml`, `.lock` and
  `backends/migrations/` trees are byte-identical to this head's
  (`git diff a603125 HEAD -- '*.rs' '*.toml' '*.lock' backends/migrations/`
  is empty), at 1666 passed / 0 skipped. That is evidence, and it is not the
  same thing as a green run on this SHA. A run on a quiet machine, or CI, is
  what would close it.

The UI revamp's `eslint-plugin-better-tailwindcss` rules were the specific
worry — `enforce-consistent-line-wrapping` with
`{ group: "never", preferSingleLine: true, printWidth: 100 }` requires every
class string on one line, and prettier's `printWidth` is 80. There is no
conflict, because prettier never breaks _inside_ a string literal; it moves
the attribute, not the classes. Measured, not argued: `lint-web` is green and
`pnpm -r lint` reports no `better-tailwindcss` finding.

---

## 5. Not checked

- **`just test-e2e` / Cypress.** Not part of `just ci`, and not run.
  `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts` was reformatted; it is
  one of the 204 files whose AST is identical, so the risk is nil, but no
  Cypress run backs that sentence.
- **`just helm-check`.** Not part of `just ci` (it needs the network).
  `deploy/helm/vpay/values.schema.json` was reindented and is parse-identical;
  no `helm template` was run against it. Nothing under
  `deploy/helm/**/templates/` is in the commit.
- **Rendered markdown.** The mdast comparison is a comparison of structure and
  text, not of GitHub's rendering. No renderer was run over the 189 files.
- **`just ci` on the rebased head.** This branch is deliberately **not**
  rebased: it lands after exp43, exp44 and exp45. The landing re-run of
  `just fmt` is expected to be a no-op for everything already in this commit
  and a pure reformat of whatever those three branches add.
- **`just ci` green end to end on the final SHA.** Six attempts, six
  testcontainers timeouts; see § 4. Everything except the Rust suite's
  execution was measured on this head.
- **The prettier version question in § 3.** Surfaced, not decided.

---

## 6. What was done to the branch

The draft's four commits were regenerated rather than amended, because the
formatting commit itself had to change and a second 400-file commit undoing
part of the first is worse to land and worse to re-run `just fmt` on top of.
The branch now carries six commits on `44e0c80`, one per finding plus the
single formatting commit:

| Commit    | What                                                                                    |
| --------- | --------------------------------------------------------------------------------------- |
| `d26268b` | `.prettierignore` — fixture, `pnpm-lock.yaml`, `backends/migrations/*.sql`, reason each |
| `db8fd93` | `.prettierrc.json` — defaults written out, `embeddedLanguageFormatting: "off"`          |
| `871045d` | **the formatting commit** — one `just fmt`, 404 files                                   |
| `71dab68` | `justfile` + `.github/workflows/ci.yml` — `fmt-check-web` is a gate                     |
| _(this)_  | `docs/status.md` and these notes                                                        |

The draft's file-type breakdown claimed 189 `.md`, 110 `.ts`, 90 `.tsx`, 4
`.json`, 3 `.css`, 3 `.js`, 1 `.yml`, 1 `.mjs` — 401 of the 405 files it
committed. The four it dropped were three more `.yml` and the one `.yaml`,
which is `pnpm-lock.yaml`: the largest file in the diff, invisible in the
summary of it. The regenerated commit is 404 files and the breakdown in its
message adds up.
