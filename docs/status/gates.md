# How `docs/status.md` is machine-checked, and the history of every gate

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

This is the full narrative that stood at the top of `docs/status.md` until 2026-09-11:
what each gate checks, what it used to miss, and the mutation that proved each hole shut.
[docs/status.md](../status.md) now carries the summary; this page carries the record.

**What actually works today.** This page is the contract behind the repo's second
rule: _never advertise a feature as done when it clearly is not._

It is machine-checked, and **since 2026-09-03 the check runs in both
directions**. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is
missing from this file — _and_ fails if this file declares a token that no
shipping code carries any more, so a section that outlives the code it
described cannot sit here unnoticed. The scanner reads _code_, not text:
since 2026-09-05 it lexes, so a `NotImplemented("…")` written in a comment
of any kind (`//`, `///`, `//!`, `/* */` nested or not — leading **or**
trailing), in a `#[doc = "…"]` attribute, or inside any string, raw-string
or character literal is prose, and prose declares nothing. It was
comment-aware from 2026-09-03 (that blind spot is described in the Step 2
note below, where it was found), but only for comments that _began_ a line,
and not at all for string literals; a trailing `// … NotImplemented("x")` or
an `r#"…"#` carrying the token still forced a phantom bullet into this file
— and because the check runs in both directions, the bullet then had to
stay, so the docs→code half could be satisfied by prose alone.
`cargo xtask verify-no-mocks` no longer greps the two app manifests: it
walks `cargo metadata`'s dependency graph from each shipping binary along
non-dev edges only, so a test double reachable through _any_ intermediate
crate is caught, and it additionally refuses a test-only crate listed under
any workspace member's `[dependencies]` — with one narrow, documented
allowlist (`vpay-testkit` → `testcontainers`/`testcontainers-modules`,
because starting a real container is what that crate exists to do and
ADR-0006 says a stub rail **is** a WireMock host reached over HTTP). The
Rust `wiremock` crate — an _in-process_ double — is allowlisted for nobody,
which is how `vpay-testkit`'s unused runtime dependency on it was found and
dropped.
**Extended 2026-09-03 (Step 7):** its `backends/apps` name scan also refuses
`vpay_db::connect_lazy` in non-test code. That function is _not_ a test
double — the pool is the real `sqlx` one — which is exactly why no
dependency rule would ever object to it; what it defeats is the property
`connect` exists to hold, that a process which cannot reach its database
fails at boot rather than at the first payment.
AGENTS.md's claim that `verify-status` "fails in both directions" is,
as of this pass, true. Since 2026-09-02 `cargo xtask
verify-errors` likewise fails the build if an error type in
`backends/crates` is not classified per [ADR-0011](../adr/0011-error-modelling.md),
or if `anyhow` leaks into a library crate. **Extended 2026-09-03 (Step 7):**
it additionally refuses a composite whose `#[from]` leaf is answered for by a
`_ =>` arm — for every `#[from]` variant, each `Classify` method that
_discriminates_ on `self` must name `Self::<Variant>` explicitly.
"Discriminates" is **five** spellings, not one: the first version of this
check searched for the literal `match self`, which made the rule opt-out —
the same ladder written `if let Self::Db(e) = self { … } else { … }`,
`matches!(self, …)` or `match *self` skipped the method entirely, and its
trailing `else` answered for an unnamed leaf exactly as a `_ =>` arm would.
Proven live by deleting `ApiError`'s `Self::Db(e) => e.code()` arm and
watching the check fail, and by
`a_from_variant_swallowed_by_an_if_let_ladder_is_reported`, which fails if
the list of spellings is narrowed back. It now reports `15 error type(s),
all classified; 14 #[from] variant(s) delegate every Classify method they
match on` — and that second number is a real count of `#[from]` variants
that passed, not the arithmetic it was (a per-source-file accumulation with
one subtracted per violation). _(**Corrected 2026-09-04 (Step 8):** this
paragraph read ~~`14 error type(s)`~~ when Step 7 wrote it. Lane B added a
fifteenth, `vpay_worker::ssrf::EgressRefusal`, and no new `#[from]` variant,
so the second number is unchanged. `15` / `14` is what `cargo xtask
verify-errors` printed on the merged gate branch on 2026-09-04.)_

**New 2026-09-03 (Step 7, lane 4): `just verify` is three gates and one
report** — four gates since `verify-sdk-parity`, **five since
`verify-links` (2026-09-05)**, **six since `verify-npm-scope` (2026-09-05)**,
**seven since `check-schema` (2026-09-05, the CrateStack schema gate —
see "CrateStack" below)**, **nine since `verify-serde` and
`verify-repositories` (2026-09-05, [ADR-0016](../adr/0016-engineering-standards.md))**
and **ten since `verify-toolchain` (2026-09-05, the pin-drift gate — see
"Toolchain pin" below)**, all seven below. `cargo xtask
verify-docs` prints, per crate, doc-comment lines
against code lines, **in-file comment lines against code lines, the number of
`#[doc = include_str!]` modules**, every production function of 80 lines or
more, every
` ```ignore ` doctest fence and every `#[allow]`/`#[expect]` in
production code — and **exits 0 whatever it finds**. That is Step 7's
decision (4) and not an oversight: the cheapest way to pass a doc-ratio gate
is to delete the `# Errors` sections ADR-0011 depends on, so this number is
published rather than enforced. It is scoped to `backends/crates` and
`backends/apps` `src/` trees, so `sdks/rust` and `.xtask` are outside every
number it prints. Separately, `just test-doc` (`cargo test --doc
--workspace`) now runs in `just ci` and in CI's `rust` job: `cargo nextest`
runs no doctests, so before this the workspace's doctests — there was
exactly one — had never been compiled by CI at all.

**The measured after-state, on the final Step 7 tree with all five lanes
landed** (`cargo xtask verify-docs`; `prose` excludes doc lines inside a
` ``` ` fence, which are compiled examples rather than comment volume):

```
  crate                        prose       ex     code     ratio
  vpay-adapter-mtn-momo          395       13      625     63.2%
  vpay-adapter-orange-money      389       16      559     69.5%
  vpay-api                      4281      161     3808    112.4%
  vpay-config                   1303       32      844    154.3%
  vpay-core                      781      570      830     94.0%
  vpay-db                       2826       26     2370    119.2%
  vpay-ledger                     37       59       89     41.5%
  vpay-provider                  581       93      493    117.8%
  vpay-testkit                   197        6       89    221.3%
  vpay-worker                   1717      135     2588     66.3%
  vpay-server                    265        0      358     74.0%
  vpay-worker-bin                276        0      285     96.8%
  TOTAL                        13048     1111    12938    100.8%
```

**Against the design's baseline, in the design's own units: 97.1% → 88.6%.**
That is the only comparison that means anything, because the design's table
(`docs/plans/2026-09-03-step7-cleanup-rework.md`, "(a) Re-measured baselines")
counted _every_ doc line as prose and used a looser denominator; the same
convention on this tree reads doc 14 159 / code 15 964 = **88.6%**. The step's
own target was ≤40%, and **it was not met — not close.** What actually moved:
1 111 lines of prose became compiled examples, and about 700 more moved into
`docs/reference/`. Four crates are still over 100%, `vpay-config` is at
154.3%, and `vpay-testkit` at 221.3% was in no lane's
scope at all. Anyone reading this as "the docs were cleaned up" should read
the four crates' numbers instead.

Since 2026-09-03 `cargo xtask verify-sdk-parity` reads the fourth
machine-checked document,
[docs/sdks/parity.md](../sdks/parity.md): the merchant SDKs (`sdks/rust`,
`sdks/nodejs`) must offer the same capabilities with the same wire
semantics — [ADR-0015](../adr/0015-sdk-parity.md) — and every `✅` cell must
name a test that actually exists in that SDK's sources (a renamed or
deleted test fails the build the same way an undeclared `NotImplemented`
does), while every `⛔` cell must carry a dated, owned gap. `@vaam-apps/vpay-stripe-js`
gets its own table in the same document rather than a third column, because
it authenticates a payer's browser, not a merchant. As of this pass the
matrix names **335 proving tests** across the two merchant SDKs and records
**26 dated gaps** (merchant SDKs and the browser surface together) — measured
on 2026-09-04 on the Step 9 gate branch, up from 267 / 24 after Step 8: lane 5
added the checkout-session rows and two gaps saying that nothing in either SDK
had then spoken to a real checkout session, and lane 5b added the
client-assertion audience row. See the "Merchant SDKs" section below and the
matrix's own "Gap ledger" for the list.

**Updated 2026-09-06: that gate was one-directional, and now is not.** It read
the matrix and checked whether what the matrix said was true, which cannot
notice what the matrix does not say. Two measured consequences, both on this
repository's own tree: deleting a whole capability row passed — 350 proving
tests fell to 347 and `just verify` stayed green — and an SDK method with no
row at all was invisible, so ADR-0015's rule ("every SDK ships every feature
or a dated gap") had no enforcement in the code→doc direction. The gate now
enumerates every `<resource>.<method>` both SDKs declare, by scanning their
sources the way the other `verify-*` gates do (no compiling), and fails if a
method has no row, naming the `file:line` it is declared on, or if a row names
a method neither SDK declares, naming the row's line — unless every cell of
that row is a dated `⛔`, which is how a capability written down before it
exists is recorded. `docs/sdks/parity.md`'s header states the naming
convention. **Run on this tree, it found nothing to fix**: every shipped
capability already had a row and every distinct capability row already
named a shipped method or was a dated gap, so the two directions are recorded
as _newly enforced_, not as _newly discovered defects_. **Re-measured on
2026-09-06 after this branch was rebased onto `bb8de92`** (which merged issue
#45 and added `refunds.retrieve` to both SDKs, the server route and the
matrix), its success line reads `354 proving test(s) named in
docs/sdks/parity.md all exist, 28 dated gap(s), 14 SDK method(s) enumerated
across 17 row(s)` — 14 shipped capabilities, all with rows, and 15 distinct
capabilities named by those rows, the fifteenth being the `events.retrieve`
dated ⛔. Still nothing to fix. (Before the rebase the same line read `350
proving test(s) … 13 SDK method(s) enumerated across 16 row(s)`; the numbers
moved because #45 landed, not because the gate changed.) The method count is
printed so that an enumerator that silently found nothing — which would
satisfy both new directions vacuously — is visible, and a unit test asserts
the 14 capabilities **by name, per SDK**, for the same reason. That
by-name assertion is not decoration: it is what caught the rebase. Git
reported no conflict — #45 touched the SDK sources and the matrix, this
branch touched the gate — and a count-only guard would have stayed green
while the meaning of "enumerated everything" widened underneath it. Instead
the test failed naming `sdks/rust` and printing both lists. Proven by five mutations, each applied, run and reverted on
2026-09-06: a deleted `refunds.create` row, a `pub async fn frobnicate(` added
to a Rust resource, the same added to a Node one, a `payments.teleport` row
naming no method, and a renamed proving test (the pre-existing direction,
still failing) — four fail with exit 1 and the fifth, the same
`payments.teleport` row rewritten as `⛔`/`⛔` with a date, passes.
**Three of them were re-run on the rebased tree on 2026-09-06 and all three
still fail with exit 1**: deleting the (new) `refunds.retrieve` row fails
naming `sdks/rust/src/resources.rs:726`; `pub async fn frobnicate(` added to
`PaymentIntentsResource` fails naming `sdks/rust/src/resources.rs:512`; and
the same method preceded by a `b'}'` byte literal in the same `impl` — the
shape the lexer fix exists for — fails naming
`sdks/rust/src/resources.rs:516` rather than being swallowed. See
[plans/exp15-notes/C.md](../plans/exp15-notes/C.md).

**Reviewed the same day, and the enumerator had three silent holes — all
three closed** ([plans/exp15-notes/C-review.md](../plans/exp15-notes/C-review.md)).
The review re-ran all five mutations above (all still correct) and added its
own; three of them passed when they should have failed, and each was a case
where the _other_ SDK still backed the row, so the printed method count did
not move and nothing was said:

- a Rust **character or byte literal holding a brace** — `b'}'`, the shape
  `sdks/rust/src/webhooks.rs:321` has shipped since it was written —
  truncated the enclosing `impl …Resource` and dropped every method after
  it. Measured: adding an unrecorded `pub async fn` to
  `PaymentIntentsResource` failed with exit 1 on its own, and passed with
  exit 0 once one such literal preceded it. The gate's own lexer had claimed
  in a comment that "neither SDK contains one"; that was false.
- a **nested Rust block comment** ended at the first `*/`, so a method parked
  inside `/* … /* … */ … */` was enumerated as shipped and the gate demanded
  a parity row for a method that does not exist — a false positive whose
  cheapest cure is deleting the honest comment.
- a **TypeScript method with a type parameter** (`async listAll<T>(…)`) was
  read as a field and never enumerated.

The first two are fixed by deleting the second Rust lexer: `code_only` now
hands Rust literals to `end_of_literal`, the one `verify-status`,
`verify-serde` and `verify-docs` already share, which knows `r#"…"#`, `b'…'`
and the lifetime-versus-character-literal ambiguity. Each fix carries a
regression test measured to fail against the behaviour it replaced. **The
enumeration of this tree is unchanged** — the same capabilities across the
same rows, 13 across 16 when measured, 14 across 17 after the rebase onto
`bb8de92` — so these closed holes, and did not correct a miscount.
`cargo test -p xtask` 208 → **211 passed, 0 failed, 0 ignored**. Still not
checked, and recorded rather than fixed: a Node resource declared as an
object literal or with arrow-function properties remains invisible (neither
shape exists in `sdks/nodejs` today, checked module by module). ~~and no rule
compares a row's per-column `✅`/`⛔` cell against whether _that_ SDK
declares the method~~ **— closed 2026-09-08 (exp33 review).** That hole was
measured on the exp33 head before it was closed: deleting `invoices.void`
from `sdks/rust` alone left `verify-sdk-parity` at **exit 0**, reporting
"443 proving test(s) … 32 SDK method(s)", because the doc→code direction was
satisfied by `sdks/nodejs` still declaring the method and the ✅ cell's named
test went on existing as _that_ SDK's source text. The gate now has a sixth
rule — a ✅ on a capability row may only appear in a column whose own tree
declares the method — and the same mutation exits 1 naming `sdks/rust`.
`a_tick_for_a_method_only_the_other_sdk_declares_fails` is the regression
test, with `a_dated_gap_is_how_the_sdk_that_lacks_the_method_answers` and
`a_behaviour_row_is_untouched_by_the_per_column_rule` for the two shapes that
must keep passing.

**New 2026-09-05: the three publishable npm packages are renamed
`@vpay/*` → `@vaam-apps/vpay-*`.** The organisation was renamed
`vaam-store` → `vaam-apps` on 2026-09-04, and the scope now matches it while
the package name keeps `vpay` so the name still says what it is:

| Was                   | Is                              | Directory            |
| --------------------- | ------------------------------- | -------------------- |
| `@vpay/sdk`           | `@vaam-apps/vpay-sdk`           | `sdks/nodejs`        |
| `@vpay/stripe-js`     | `@vaam-apps/vpay-stripe-js`     | `sdks/stripe-js`     |
| `@vpay/stripe-compat` | `@vaam-apps/vpay-stripe-compat` | `sdks/stripe-compat` |

**Nothing had ever been published under the old names, so the rename cost
nothing and would have been expensive after a first publish.** Verified
rather than assumed, 2026-09-05, against `https://registry.npmjs.org/`:
`npm view @vpay/sdk version`, `npm view @vpay/stripe-js version` and
`npm view @vpay/stripe-compat version` each exit 1 with `E404 … is not in
this registry`, and so do all three _new_ names, so none of them is taken by
anybody else either. The outputs are in
[docs/plans/exp7-notes/opus.md](../plans/exp7-notes/opus.md).

Three things this rename did **not** change, stated because each could be
read into it:

- **Two of the three stopped being `"private": true` on 2026-09-05, at the
  maintainer's direction** ("change packages to be published from `@vpay/*`
  to `@vaam-apps/*`"): `@vaam-apps/vpay-sdk` and `@vaam-apps/vpay-stripe-js`.
  Each carries `publishConfig.access: "public"` (a scoped package defaults to
  `restricted`), `repository`/`homepage`/`bugs` on `github.com/vaam-apps/vpay`,
  a `license`, a `description`, and a `prepack` that builds. **Nothing is
  published, and nothing can be until a release workflow exists** — no
  workflow in `.github/workflows/` runs `npm publish` or reads an `NPM_TOKEN`,
  and `npm view` answered `E404` for both names on 2026-09-05. Removing the
  flag makes a release possible without editing a manifest; it does not
  perform one.
- **`@vaam-apps/vpay-stripe-compat` stays `"private": true`, and its
  `publishConfig` was removed rather than kept.** It is a conformance suite,
  not a library — its own README says so — with no build, no `main`, no
  `exports` and no `files`, so `pnpm pack` on it produces a tarball of five
  `*.compat.test.ts` files, a `vitest.config.ts` and an `eslint.config.js`
  and no JavaScript. A package that cannot be packed honestly does not get to
  look one word away from shipping.
- **The tarballs were empty before `prepack`, and that was the real
  publish-break.** `dist/` is gitignored, so on a clean clone `pnpm pack`
  produced a 14 934-byte `@vaam-apps/vpay-sdk` containing `LICENSE`,
  `package.json`, `README.md` and `scripts/mint-assertion.mjs` — no
  JavaScript at all, with `main`, `types` and `exports` all pointing at files
  that were not in it. `"prepack": "pnpm run build"` on both packages closes
  it; re-measured with `dist/` deleted, the tarballs carry 32 and 12 built
  files. `cargo xtask verify-npm-scope` fails the build if either package
  loses that script.
- **The Node SDK's `User-Agent` is unchanged**, and deliberately: it is
  `vpay-sdk-node/<version>`, which never carried the npm scope. It is a wire
  contract `sdks/rust` holds itself to byte-for-byte, so moving it would have
  broken parity to no purpose.
- **The Rust crate is still `vpay-sdk`** (`sdks/rust`, `publish = false`);
  crates.io has no scopes, so nothing about it follows from an npm rename.

Every occurrence of the three old names in _this_ file was rewritten
mechanically, including inside dated notes recording commands that really
were run under the old spelling — so a Step 5b note reading `pnpm --filter
@vaam-apps/vpay-sdk build` is the current spelling of a command that ran as
`pnpm --filter @vpay/sdk build`. The old names survive verbatim in
`docs/plans/**` (closed, dated step and design notes) and in
[ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md) and
[ADR-0015](../adr/0015-sdk-parity.md), which AGENTS.md makes immutable —
superseding those two is a maintainer decision, not this change's.

**New 2026-09-05: `cargo xtask verify-npm-scope` is the sixth gate.** It
asserts what the rename above depends on and what nothing else checked:
every publishable package under `sdks/` is named `@vaam-apps/vpay-*`, says
`publishConfig.access: "public"`, names this repository, carries a `license`,
ships a `files` allowlist with an entry point under `dist/` and a `prepack`
that builds it; every private one under `sdks/` declares no `publishConfig`;
no package outside `sdks/` wears the publishable scope; and no retired
`@vpay/{sdk,stripe-js,stripe-compat}` name survives outside `docs/plans`,
`docs/adr` and `docs/status.md`. Measured before it existed, on the branch
that performed the rename: deleting `publishConfig.access` from
`sdks/nodejs/package.json` — the one line between `npm publish` and a scoped
package's default `restricted` — passed `pnpm install --frozen-lockfile`,
`pnpm -r typecheck`, `just lint-web`, `just test-web`, `just audit-web` and
all five gates. Reverting a package's own `name` to its retired spelling
passed the lockfile too: pnpm keys `importers` by directory, so a workspace
package's own name never reaches `pnpm-lock.yaml`. Fourteen unit tests in
`.xtask` drive the gate itself through each rule. What it does **not** check:
that `dist/` exists (gitignored) and the registry (needs the network).

**New 2026-09-05: `cargo xtask verify-links` is the fifth gate,
`cargo xtask verify-npm-scope` the sixth, `just check-schema` the seventh,
`cargo xtask verify-serde` the eighth and `cargo xtask verify-repositories`
the ninth (both [ADR-0016](../adr/0016-engineering-standards.md)),
`cargo xtask verify-toolchain` the tenth (added later the same day, in the
review of the toolchain bump), and `cargo xtask verify-citations` an eleventh
that is opt-in and in no CI job.** Until this
landed, `just docs-check` ran `verify-status` and printed `note: link
checking is not implemented yet`; the one class of claim this repository
makes about itself most often — _the document you are reading points at the
file it names_ — was the one class no gate protected. The Step 9
release-claims review had recorded both holes by mutation and left them as a
maintainer decision (`docs/plans/step9-notes/release-claims-review.md`,
findings F3 and F4, mutations M2 and M3); both rows now carry a dated
correction.

- **`verify-links` — a gate, in `just verify`, `just docs-check`, `just ci`
  and CI's `self-checks` job.** Over every `*.md` that `git ls-files`
  reports: inline links, image links and reference definitions, with fenced
  code blocks, inline code spans and HTML comments masked out first. It skips
  `http(s)://`, `mailto:` and bare `#anchor` targets; for everything else it
  strips a `#fragment` and a trailing `:line` (or `:line:column`), decodes
  percent escapes, resolves against the linking file's own directory, and
  requires the result to be a **tracked** file or directory — a link
  satisfied by an untracked scratch file resolves on one machine and nowhere
  else. **Measured 2026-09-05: 115 files scanned (113 on `master`, plus this
  change's notes and its review's), 673 repository links checked, 5 broken,
  5 fixed** — **re-measured 2026-09-05 on the tree that carries both this
  gate and `check-schema`: 119 files, 684 links, 0 broken**, the file count
  larger only because two branches' notes landed on top of the first
  measurement. The 5 broken are the count on `master`, and an independent
  oracle built on markdown-it-py 3.0.0 agrees with both numbers exactly (this branch's
  review, `docs/plans/exp6-notes/opus-review.md`). All five were in `docs/plans/step8-notes/`
  (`lane-c.md` ×3, `lane-h.md` ×2), and ~~four were links copied verbatim out
  of a `docs/flows/` document into a blockquote, keeping the quoted file's own
  relative path, and one was a `../` one level short~~ **all five are the same
  thing (corrected 2026-09-05, in this branch's review):** each is inside a
  blockquote quoting text that belongs in another directory, and each
  destination was _correct where that text lives_ — `docs/flows/`
  (`crash-safety.md:320`, `reconciler.md:95`, `reconciler.md:173`) or
  `docs/status.md:1449`. Nothing was deleted — every one named a file that
  exists, and each was repointed so it resolves from the notes file. **The
  cost, stated rather than hidden: those five blockquotes are no longer
  verbatim quotations.** `lane-c.md` and `lane-h.md` each carry a dated note
  at the quote saying so and naming the applied text, because a planning note
  whose quoted patch would introduce a broken link if pasted is a worse defect
  than the one the gate found.
  **Deliberately not checked, and the gate's own doc comment says so:**
  `#anchor` fragments (agreeing with GitHub's heading-slug algorithm is a
  guess, and a wrong guess fails correct documents), `http(s)` URLs, and
  reference _usages_ with no definition (which render as literal text, so a
  reader sees them).
- **`check-schema` — a gate, in `just verify`, `just ci` and CI's
  `self-checks` job.** `cratestack check --schema schemas/vpay.cstack` at the
  version this repository pins. It is the first gate here that shells out to
  a tool this repository does not build, and it was the first thing in the
  repository that read `schemas/vpay.cstack` at all — ~~the file is excluded
  from the build graph, so no compiler has ever looked at it~~ **corrected
  2026-09-06: `vpay-db` compiles the file now** (`mod schema` →
  `include_server_schema!`), so `cargo build` is a second reader and a
  syntax error there is a build failure. The gate is not redundant: it runs
  the _CLI_ at the pinned version, and the CLI is what `migrate baseline`
  and the drift test also use, so it is what keeps the library and the tool
  answering about one grammar. Until this landed, the evidence that it
  parses was a transcript someone pasted into
  the "CrateStack" section below after running the CLI by hand; crates.io
  published **29 `cratestack-cli` releases between 0.7.8 (2026-08-08) and the
  then-pinned 0.11.1 (2026-09-03)** — 26 days, not the "four times in five weeks"
  an earlier draft of this bullet said — and nothing would have noticed the
  file going stale across any of them. **A missing binary is a red gate**,
  printing the install command rather than "skipped" — same rule as
  `verify-citations` without `gh`. **A vacuous green is a red gate too**: an
  emptied schema, or one with no `datasource` block, gets `schema OK` and
  exit 0 out of the CLI, so the recipe asserts a `datasource` block and a
  floor of 13 `model`/`enum` declarations (12 until 2026-09-06) before it
  believes the result. See
  "CrateStack" below for the version, the six mutations that prove the gate
  fires, and what a green run does _not_ prove.
- **`verify-toolchain` — a gate, in `just verify`, `just ci` and CI's
  `self-checks` job. New 2026-09-05, in the review of the toolchain bump.**
  `backends/Dockerfile`'s `FROM rust:<version>-alpine…` must name the version
  `rust-toolchain.toml`'s `channel` pins. That `FROM` line is the only place
  in the repository that names a compiler and cannot read the toolchain file
  (CI's five Rust jobs `sed` the channel out of it), so it is the only place
  the pin can drift — and drift was measured to be free: with `channel` on
  1.98.0 and the `FROM` line left on 1.95.0, `just verify` and `just fmt-check`
  both exited **0**, and no other `just ci` recipe reads either file, because
  nothing here compiles the Dockerfile. Ten `xtask` tests drive it, four of
  them written from mutations of the gate itself — including one that caught a
  test of this review's own passing on the wrong error. It checks the compiler
  version only: the Alpine suffix moves on its own evidence, and a test pins
  that it may. What it does **not** check: that the tag exists upstream (needs
  the network; the image build proves it), Dockerfile comments (the header
  deliberately names a tag it did not take), or `Cargo.toml`'s `rust-version`,
  which is a graph-derived floor rather than a copy of this number. See
  "Toolchain pin" below.
- **`verify-citations` — a gate that needs the network, so it is opt-in
  (`just docs-check-citations`) and is in no CI job.** It resolves every
  workflow-run id, pull request and issue a tracked `*.md` cites as evidence
  against this repository with `gh api -i`, deduped so one id costs one call
  however often it is written. **Measured 2026-09-05: 39 unique ids across those 115
  files — 24 workflow runs, 14 pull requests, 1 issue — all resolve against
  `vaam-apps/vpay`; 0 false citations found.** It **fails** when `gh` is
  missing or unauthenticated and never prints "skipped", because a check that
  downgrades itself reports success for a run in which nothing was checked;
  a 403 or 429 stops the whole run rather than reporting the remaining ids as
  missing, so a rate limit can never send somebody to delete true claims.
  Three `(file, id)` pairs are exempt in `CITATIONS_THAT_ARE_NOT_CLAIMS`, all
  of them the same invented eleven-digit id in a mutation record, because a
  document reporting that substituting a nonexistent run id left every gate
  green has to be able to print that id. The exemption is scoped per file, so
  those digits anywhere else are still checked. **What it is not:** the `#n` rule needs a `PR`/`pull
request`/`issue` cue, so an id cited _only_ without one is not checked —
  today that set is empty, because every bold `**#17**` in `docs/roadmap.md`
  is also written `PR #17` in the same file. Cross-repository references
  (`authkestra#287`) are out of scope, and so is anything under a
  `github.com/<someone else>/` URL. **And an eleven-digit number that is not a
  run id is a false failure, not a false pass:** a zero-padded webhook
  timestamp is refused since this branch's review (a run id is never
  zero-padded), but an eleven-digit phone number written in a document would
  still be looked up and reported missing. That is the price of matching every
  id in a list rather than only the one after the word `run`, and it is paid
  in the direction that fails a correct document.

**New 2026-09-05: `cargo xtask verify-serde` is the eighth gate and
`cargo xtask verify-repositories` the ninth**, both from
[ADR-0016](../adr/0016-engineering-standards.md), the ADR that finally wrote down
the six engineering standards this repository had been applying in prose. Both
are in `just verify`, `just ci` and CI's `self-checks` job; neither needs a
network, a database or a binary this workspace does not build.

- **`verify-serde` — a gate.** Every type deriving `Serialize`/`Deserialize`
  under `backends/crates/*/src` (`#[cfg(test)]` items and comments stripped,
  line numbers preserved) carries `#[serde(rename_all = "snake_case")]`,
  renames every field/variant itself, or is listed in ADR-0016's exemption
  table with a reason. **Measured on `2ce13d0` before the rule existed: 64
  serialisable types, 28 without the attribute.** After: **49 comply, 15
  are exempted, 0 violations** — 13 were fixed by adding the attribute
  (`SessionCredential`, `ReturnCredential`, `OriginsQuery`, `Origins`,
  `PayerCredential`, `BrowserConfirmParams`, `CheckoutMerchantObject`,
  `CheckoutSessionForPayer`, `RawClaims`, `WebhookPolicy`,
  `PollChargePayload`, `ResubmitPayload`, `DeliverWebhookPayload`), **and the
  wire did not move for any of them**: every field of all thirteen is already
  snake_case in Rust, so `rename_all = "snake_case"` is the identity on each
  one, which is why no fixture, no SDK type and no `.sql` changed. The 15
  exemptions are the two adapters' `wire.rs`/`token.rs` types (a rail's casing
  is the rail's), the four `#[serde(untagged)]` unions whose variant names
  never reach a wire, and `vpay_core::Currency` (`UPPERCASE`, ISO-4217). The
  count of 28 is not a count of _defects_: the convention was already written
  down in `docs/reference/rails.md`, in both adapters' module docs and in a
  comment above `Currency` — in four places, checked by nobody. **The table is
  read in both directions:** a row naming a type that now complies, or a type
  that no longer exists, fails the build. **What it does not check:** whether
  a reason is a good one; "too many to fix" is a non-empty string and the gate
  cannot tell it from "models MTN's camelCase Collections wire".
- **`verify-repositories` — a gate.** Nothing outside `vpay-db` names a
  concrete repository implementation. The set of concrete types is _derived_
  from `vpay-db`'s own source — a declaration holding a `PgPool`/`Transaction`
  field, a type on the right of `impl <a vpay-db pub trait> for …` (minus a
  blanket impl's own type parameter), or a name `vpay-db` publishes for one of
  those (`pub use … as`, `pub type`, to a fixpoint) — rather than listed in the
  gate, so a store nobody has written yet is covered the day it is added. The
  third signal was added on review, against a measured evasion: the gate
  matches names textually, so `pub use repository::PgRepositories as Repos;`
  in `vpay-db` plus `use vpay_db::Repos;` in `vpay-api` cleared it. Both
  spellings now fail; the tree's own set is unchanged at 3, because `vpay-db`
  publishes no such alias today. **Measured on
  `2ce13d0`: 3 implementations (`PgRepositories`, `PendingTransaction`,
  `SqlClientAssertionStore`) and 2 violations, both in
  `backends/crates/vpay-api/src/op/mod.rs`** — `SqlClientAssertionStore` was
  `pub` and `vpay-api` constructed it by name. Fixed rather than exempted: the
  type is now `pub(crate)` and `vpay_db::client_assertion_store(pool)` returns
  `impl ClientAssertionStore`, so a caller gets the behaviour and no way to
  spell the type. **After: 0 violations across 65 source files outside
  `vpay-db`.** The gate has **no exemption mechanism**, deliberately — there is
  no exception today, and `Repositories::op_store_pool` (Step 7 decision 9) is
  a decision about a _pool_, not a licence to name an implementation type.
- **`verify-docs` gained two report lines, both advisory** (ADR-0016 standard
  6 asks for fewer in-file comments and for long reasoning to live in
  `docs/reference/<crate>.md`, and neither had a baseline). **Measured
  2026-09-05: 1789 in-file `//` comment lines against 15157 code lines
  (11.8%) across `backends/crates` and `backends/apps`, and 0
  `#[doc = include_str!]` modules.** Zero is the honest number: the
  externalised-module-doc habit does not exist in this tree yet. Neither
  number can fail a build, and ADR-0016 records why — the cheapest way to pass
  a comment-volume gate is to delete the sentence that said why, and an
  `include_str!` gate is passed by moving a paragraph into a file nobody links
  to. **Nothing was moved in the change that added the measurement.**
- **`cargo test -p xtask`: 144 before, 184 after.** 36 for the two gates and
  the declaration scanner they share, 4 for the two report lines. Four of them
  are the mutations recorded in
  [`docs/plans/exp10-notes/opus.md`](../plans/exp10-notes/opus.md).

<!-- Appended 2026-09-12, below the archived text rather than inside it: the
     note at the top of this page says its body is the original and unedited. -->

## 2026-09-12 — a gate that is not one of the twelve

`just test-storybook` renders every Storybook story in a real Chromium and
fails on an axe violation. It is **not** part of `just verify` and **not**
part of `just ci` — it needs a ~115 MB Playwright Chromium download the first
time, and `just ci` is expected to pass offline, which is the same reason
`helm-check` is excluded. CI's `web` job runs it, next to `build-storybook`,
which is likewise absent from `just ci`. So the count in
[../status.md](../status.md)'s gate table is unchanged at twelve, and
correctly so.

That leaves three tiers of check in this repository, which is worth stating
once: the twelve in `just verify`; `verify-citations`, a gate that needs the
network and a GitHub token and is therefore opt-in; and now `build-storybook`
and `test-storybook`, gates that run in CI but not in the local `just ci`.

**What keeps it a gate, given that `just ci` never runs it:**
`frontends/packages/ui/src/testing/a11y-gate.test.ts`, a plain jsdom test in
the suite `just test-web` — and therefore `just ci` — does run. It fails if
`.storybook/preview.ts` stops saying `a11y: { test: "error" }`, if
`.storybook/main.ts` stops loading `@storybook/addon-vitest` or
`@storybook/addon-a11y`, or if the set of stories carrying the
`a11y: { test: "todo" }` escape hatch is ever anything but the four declared,
measured ones. Each of its assertions was run against the mutation it exists
to catch; the numbers are in
[verification/2026-09-12.md](verification/2026-09-12.md), together with the
false green that the first version of the suite produced and how it was
found. The rows are on [frontend.md](frontend.md).
