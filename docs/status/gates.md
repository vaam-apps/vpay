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
as of this pass, true — and **since 2026-09-15 there is a third direction**,
added on review of the Orange refund flip. The two directions compare _sets of
token strings_; neither knows which file a token was written in, so an adapter
answering another rail's token satisfied both. Measured on the flip's own tree
before the rule existed: with `NotImplemented("orange_money::refund")` in the
Orange adapter replaced by `NotImplemented("mtn_momo::refund")` — the adapters'
`refund` bodies are one line apart — the gate failed the docs→code way, whose
message invites the wrong repair ("delete the bullet"); with the bullet then
deleted it printed **"ok — 1 unimplemented item(s)"**, and a whole rail's gap
had left `docs/status.md` with a green build. A token whose prefix names a rail
this workspace ships a `vpay-adapter-*` crate for must now be carried by that
crate. Prefixes naming no rail are deliberately unconstrained: there is no
convention in this repository saying where a `worker::…` token may live, and a
gate that invented one would be a rule about characters.
`a_token_naming_another_rail_is_refused_however_well_the_page_matches` in
`.xtask` pins it and asserts the passing half too, so the rule cannot degrade
into "refuse every rail-prefixed token". Since 2026-09-02 `cargo xtask
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

```text
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
  a `license`, a `description`, and a `prepack` that builds. ~~**Nothing is
  published, and nothing can be until a release workflow exists** — no
  workflow in `.github/workflows/` runs `npm publish` or reads an
  `NPM_TOKEN`, and `npm view` answered `E404` for both names on
  2026-09-05.~~ **Corrected 2026-09-20: both packages are live.**
  `release.yml` now has a `publish-node-sdk` job and a
  `publish-stripe-js-sdk` job, each running
  `pnpm publish --access public --provenance --no-git-checks`; re-run today,
  `npm view @vaam-apps/vpay-sdk version` and
  `npm view @vaam-apps/vpay-stripe-js version` both return `0.3.0`. Removing
  the flag made a release possible without editing a manifest; the workflow
  now performs one.
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

---

## `verify-ui` — added after this archive, documented here for the first time

**A pre-existing gap, not a new one.** `verify-ui` (`justfile`, a frontend
`just` recipe rather than an `xtask` gate — the only entry on this page that
is) was added by the 2026-09-07 exp26 UI revamp, after this page's own
2026-09-11 archival snapshot was frozen. It has carried a row in
`docs/status.md`'s gate table since, but never a history entry here — this
is that entry, written on 2026-09-12 because the gate itself just changed
shape, not merely narrowed.

**What it checked, 2026-09-07 to 2026-09-12.** Seven numbered checks (nine
greps) over `frontends/apps` and `frontends/packages/ui/src`, refusing a raw
palette colour, a hard-coded colour value, a daisyUI-4 class daisyUI 5
removed, a stray `!important`, a `.js`-suffixed relative import inside the
shared package, any file in that package over 200 lines, and — the
centrepiece — any `className=` written directly in an app at all. That last
rule's own remedy was "add the missing primitive to `@vpay/ui`," a package
every app composed for its layout and typography as well as its daisyUI
components.

**2026-09-12: `frontends/packages/ui` (`@vpay/ui`) was deleted.** Both apps
now compose the published `@vaam-apps/ui`, which ships components but no
layout or typography primitives at all — no `PageShell`, `Stack`, `Heading`,
`Text`, `List`, `Link`, `Section`, `VisuallyHidden` or `Logo`. The old rule's
escape hatch stopped existing, so a blanket "no `className` in an app" would
have refused the layout classes both apps must now write to render at all.
Two of the seven checks became **vacuous** rather than merely wrong: the
200-line ceiling's `git ls-files` and the `.js`-import guard's `git grep`
were both path-scoped to the deleted package, and a `git grep`/`git
ls-files` against a pathspec matching no tracked file exits non-zero (empty
result) inside `if …; then`, which reads as "no offence found" — the check
would have gone on reporting success having measured nothing. Both were
**removed**, not left: a check that cannot fail reads as coverage it no
longer provides, which is worse than no check at all. The 200-line ceiling's
own directive (the maintainer's 2026-09-11 "very few lines per component,
max 200") is recorded as ungated rather than silently dropped — whether it
transfers from a shared primitive package to app screens is undecided, and
three shipping files already exceed it while this same change is still
rewriting them (measured: 718, 336 and 279 lines the day the package was
deleted).

The `.js`-import guard was **replaced**, not merely deleted: its slot became
the permanent regression guard for the deletion itself — nothing may import
`@vpay/ui` by any spelling (`from`, `require`, `resolve`, a bare `@import`
in CSS, or a `"@vpay/ui": "…"` manifest entry) and nothing may reach into
`frontends/packages/ui` by filesystem path either, which is the one that
would have caught `frontends/packages/config/src/eslint.js`'s Tailwind
entry-point constant — a path, not a package name, so no import-specifier
grep would ever have found it.

The blanket `className=` ban became **three narrower rules**, each banning
one of the three things the old rule was actually written to stop, now that
layout classes are legal: a _computed_ class string (`cn(...)`, a template
literal, a ternary — the signature of a hand-rolled variant system, which a
literal string is not); a raw `text-state-<hue>-fg`/`bg-state-<hue>-bg`/
`text-destructive` status-colour token, which `@vaam-apps/ui`'s theme
exposes and the old palette-colour check has never been able to see because
it is a theme token, not a palette one; and a class attribute over 60
characters, which is the class-budget half of the same 2026-09-11 directive
the 200-line ceiling carried the component-size half of. The daisyUI-4 and
daisyUI-component-class checks survived with their pathspecs narrowed to
drop the deleted package, and gained one narrow, named, dated exemption
(`frontends/apps/checkout/src/components/locale-switch.tsx`) for the one
control in either app with no `@vaam-apps/ui` primitive it could legally
compose into — a native `<select>`, forced by decision 8, that must remain
themed in daisyUI's own classes or render completely unstyled.

**Ordering, and why the deletion could not be gated by anyone but the group
that ran it last.** `frontends/packages/ui`'s presence or absence changes
what several of these greps refuse, so the checks that assume it is gone had
to land in the same change as the deletion, verified against a tree with the
package present (checks 1/1b/2/3/4/7a-ii/7a-iii unaffected either way) and a
tree without it (checks 5/5b/6/7a-i's replacement all begin refusing, or
stop being vacuous, only once the directory is actually gone). Both states
were measured before this entry was written.

**Every mutation named above was run by hand, its exit code recorded, and
reverted**, per this recipe's own header discipline
(`justfile`, immediately above `verify-ui`'s definition) — a rule nobody has
mutated is a claim, not a check.

<!-- Appended 2026-09-12, below the archived text rather than inside it: the
     note at the top of this page says its body is the original and unedited. -->

## 2026-09-12 — a thirteenth gate that existed for one day

For a few hours on 2026-09-12 this repository had a gate that ran axe over
every Storybook story in a real Chromium — `just test-storybook`, built on
branch `claude/recursing-darwin-204853` ([PR #133](https://github.com/vaam-apps/vpay/pull/133)).
**It is gone**, because the `@vaam-apps/ui` cutover that landed the same day
deleted `@vpay/ui`, and that package was where Storybook, the stories' host
and the gate all lived. It is recorded here rather than dropped, because what
it measured is still true of this repository and the next attempt should not
pay for it twice.

It was never one of the twelve: it needed a ~115 MB Playwright Chromium the
first time and `just ci` must pass offline, the same reason `helm-check` is
excluded, so it ran in CI's `web` job beside `build-storybook`. The count in
[../status.md](../status.md)'s gate table was unchanged at twelve then and is
unchanged now.

**What it found, in one run, none of it planted:** four WCAG AA contrast
violations, one of them the checkout's MSISDN rejection message — the
`role="alert"` line telling a payer their number was refused — at 2.92:1
against AA's 4.5:1. Every other gate in this repository was green on that
tree. They were fixed, and the fix is gone with the package; the two causes
are general and are written down in `justfile`'s `build-storybook` slot for
whoever rebuilds.

**What it proved about gates generally**, which outlives it:

- A CI-only gate needs a lock that runs inside `just ci`, or nothing anyone
  runs locally notices it being switched off. That branch used a plain jsdom
  test asserting the addon was still loaded, that a violation still **failed**
  rather than warned, and that the set of stories opting out was exactly the
  declared, measured ones.
- **A story that throws while rendering still counts as a passing test.** The
  suite reported "93 passed" with 24 unhandled errors and axe had run on none
  of those 24. A green test count is not evidence that anything rendered.
- That failure reproduced from a cold dependency cache only, so it passed
  locally and failed on a fresh runner — CI caught it, this machine never
  would have.

The full record is
[verification/2026-09-12-browser-a11y.md](verification/2026-09-12-browser-a11y.md).

**Restored later the same day**, in `frontends/apps/checkout` this time, where
the stories live and no package deletion can take it away again. The rebuild
inherited all four lessons above and immediately earned a fifth: a gate can
run, pass, and be measuring the wrong thing entirely. Six consecutive runs
passed all 22 checkout stories while every one of them rendered **unstyled**,
because both apps' `globals.css` placed the theme `@import` after another
at-rule and CSS drops it there. Tailwind's own parser is lenient, so every
existing gate — including the styling gate the cutover added for exactly this
failure — inlined the theme and passed. **A green accessibility run against
the wrong background is worse than no run: it is a claim nobody re-checks.**
The lock is now a position assertion on the `@import` plus a read of the built
stylesheet, in
[verification/2026-09-12-storybook-restored.md](verification/2026-09-12-storybook-restored.md).

## 2026-09-17 — `verify-versions`, the thirteenth gate (PR #201)

Recorded here on 2026-09-17 by [#187](https://github.com/vaam-apps/vpay/pull/187),
not by the change that added it. [#201](https://github.com/vaam-apps/vpay/pull/201)
moved the `verify` recipe, its echo, `verify_all` and a CI step, and wrote no
row on this page, no row in [../status.md](../status.md)'s gate table and no
bullet in `justfile`'s own header list. `docs/status.md` § "Where a new row
goes" says a new gate lands in both places; this is the second half, written
where the two thirteenth gates met.

It refuses two things: a version release-please owns that disagrees with the
others, and a file listed in `release-please-config.json`'s `extra-files` that
carries no `x-release-please-version` comment — because release-please's
`generic` updater rewrites only annotated lines, so an unannotated
`extra-files` entry is a version that silently stops being bumped. Nothing
else checks it: `release.yml` derives its Docker tag from the git ref and never
compares it against any manifest.

**It went red one commit after it landed, and what turned it red is the thing
it was built to catch.** The release pull request
[#203](https://github.com/vaam-apps/vpay/pull/203) bumped 0.1.0 → 0.1.1, and
release-please's YAML updater **re-serialised**
`deploy/helm/vpay/Chart.yaml` and
`sdks/flutter/vpay_checkout_flutter/pubspec.yaml` instead of rewriting one
line of each. Re-serialising a YAML document drops its comments — including
the `x-release-please-version` annotations that are the only reason those two
files were bumpable. At `a475a2d8` the pubspec read
`version: 0.1.0 # x-release-please-version`; at `eb078020` it reads
`version: 0.1.1`, and the Chart has lost its entire header comment block.

**The tool destroyed its own instructions on its first run**, so the second
release would have left both versions behind at 0.1.1 with nothing anywhere
saying so — which is exactly the quiet regression #201's own doc comment says
it exists for. Without the gate the symptom would have been a Helm chart
version that stopped matching the image tag, some releases later.
`master`'s CI run
[35275177452](https://github.com/vaam-apps/vpay/actions/runs/35275177452)
fails on the `verify-versions (release-please's bump is complete)` step.

~~**The gate is right and the tree is wrong**, so it is recorded rather than
worked around. A separate pull request owns the repair; nothing on this page
or in #187 touches either file, and #187 is therefore red on this gate too,
inherited rather than caused.~~

**Repaired 2026-09-18, and the section below this one is the record.**
[#204](https://github.com/vaam-apps/vpay/pull/204) restored both files and made
every `extra-files` entry `{"type": "generic"}`;
[#206](https://github.com/vaam-apps/vpay/pull/206) gave the gate its first
tests and cleared the rest of #203's fallout. #187 merged both, so it is no
longer red on this gate, and the two sentences struck above were true only
between 2026-09-17 and that merge. **This section is the discovery; § 2026-09-18
below is the diagnosis**, and it is the one to read: what re-serialised those
two files was not the YAML updater being careless but a **bare-string**
`extra-files` entry routing to `GenericYaml` instead of the annotation-only
updater — a root cause this section did not have.

_(It is worth reading beside `justfile`'s `verify-migrations` note and the
2026-09-12 Storybook section above: three gates now, each of which went from
"reasonable precaution" to "caught a real regression" inside a week of
landing, and in all three cases the regression was invisible to every other
check in `just ci`.)_

## 2026-09-17 — `verify-privacy-inventory`, the fourteenth gate (issue #144)

New 2026-09-16 with the personal-data inventory of issue #144, and the
**fourteenth** gate rather than the thirteenth: `verify-versions` landed on
`master` hours earlier, from a branch this one had not seen, the same way
`verify-npm-scope` and `check-schema` collided on 2026-09-05. _(This section
was headed "the thirteenth gate" until the merge of 2026-09-17.)_ It comes from
([ADR-0020](../adr/0020-privacy-controls-and-evidence.md),
[RFC-0002](../rfc/0002-gdpr-policy-and-operator-decisions.md)). It reads
`schemas/privacy-inventory.yaml` and derives the authoritative database
column set by parsing `backends/migrations` itself — not
`schemas/vpay.cstack`, which deliberately models less than the whole database
(ADR-0020). It fails in **both** directions, like `verify-status`:

- a migrated column with no element in the inventory fails (a privacy-relevant
  column cannot land unclassified);
- an inventory column copy naming no live column fails (a stale or misspelled
  row cannot survive the column it named).

It also validates that each element's six string fields are non-empty — the
ADR-0020 §1 classification is eight fields; `necessary` is a boolean and
`recipients` a list, so "present" is all either can be — and that every
registered non-database surface has a unique id and a description.

**Why it parses the migrations rather than reading a manifest:** a manifest is
a second artifact that can itself drift, and `schemas/vpay.cstack` is a
_projection_ — RFC-0002 PR 2 requires the check be against "a fully migrated
database or the migrations that create it". Parsing meant modelling SQL DDL
`CREATE TABLE` and `ALTER TABLE ... ADD/DROP/RENAME COLUMN`, string-aware, so
the parser reflects the **final** schema: it must apply migration 0010's
`DROP COLUMN private_key_pem` / `RENAME COLUMN id TO kid`, 0014's
`DROP COLUMN last_payment_error`, and 0044's five dropped `staff_members`
credential columns. Three of those would otherwise have shown up in the
inventory as stale rows (the initial hand-built inventory did, and the gate
caught every one). The string-awareness matters for a real migration too:
0007's `CHECK (private_key_pem LIKE '-----BEGIN%KEY-----%')` begins with
`--`, and a comment stripper that is not string-aware eats the expression and
leaves the `CHECK (` unclosed.

**And `DROP TABLE`, which the first draft did not model — the hole this gate
was reviewed into having.** Added 2026-09-17 on the review of this PR.
Migration `0009` drops `merchant_api_keys` outright (`0008` created it; the
merchant-auth model moved to `private_key_jwt` before either shipped, ADR-0010).
A parser that models `DROP COLUMN` but not `DROP TABLE` leaves all eight of
that table's columns in the derived set, and then **both** directions agree
about a table no database has: direction A cannot report them unclassified
because the inventory classifies them, and direction B cannot report the
inventory rows as stale because the parser still derives the columns. The
inventory did classify all eight, and
[../reference/personal-data-inventory.md](../reference/personal-data-inventory.md)
published `merchant_api_keys.key_digest`/`key_prefix` as stored merchant
credentials — a GDPR artifact naming a table that does not exist. With
`DROP TABLE` modelled, the gate named all eight on the real tree in one run.
`a_dropped_table_leaves_no_columns_behind` and
`a_stale_row_naming_a_dropped_tables_column_fails_direction_b` pin it.

**What it proved about this gate generally**, and it is the same lesson
`verify-status` learned twice: a drift gate whose two directions read the
_same_ derived set can be wrong in both at once. Neither direction is a check
on the other; **the parser is**, and that is why the parser is where the next
review looked.

**A second review pass, 2026-09-18, found the same shape four more times, and
closed the class rather than the instances.** The paragraph above used to end
by naming `ALTER TABLE … RENAME TO`, `CREATE TABLE … (LIKE …)` and
`PARTITION OF` as unmodelled, and observing that the last two "would produce a
table with no columns and fail it **silently**, which is the shape to watch
for". That was correct and it was left as an observation; an observation is not
a gate. What the second pass measured, by driving `migrations_db_columns` over
temp trees:

1. **The keyword match was one literal space.** `stmt.to_uppercase().find("CREATE TABLE")`.
   `CREATE  TABLE t (…)` with two spaces, a tab, or a line break after `CREATE`
   derived **nothing at all** — the whole table, every column of it, invisible
   to both directions. `ALTER  TABLE t ADD COLUMN email` silently lost `email`.
   `DROP  TABLE t` silently left `t` standing, which re-opens the `0009` hole
   above through a second space. `RENAME  COLUMN a TO b` left the old name
   live. Nothing in this repository reformats SQL, so the only thing keeping
   the gate honest was that all 48 migrations happen to be typed with exactly
   one space.
2. **No word boundary** — `RECREATE TABLE` contains `CREATE TABLE` — and **no
   string-literal awareness**. These migrations' `COMMENT ON` bodies are essays
   that discuss migrations (45 of them already embed a `;` inside the quotes),
   so one containing the words `DROP TABLE customers` would have removed the
   real `customers` table from the derived set.
3. **Offsets from an uppercased copy indexed the original.**
   `str::to_uppercase` is not length-preserving for every input.
4. **`strip_sql_comments` pushed `byte as char`**, re-encoding every byte of a
   multi-byte character as its own Latin-1 code point. These migrations are
   full of `§`, `—` and `±`, so its output was mojibake **and longer than the
   input** while its doc comment claimed length was preserved. Nothing indexed
   back into the source, so nothing misparsed: the false claim was the whole of
   the defect, which in this repository is the part that counts.

`kw_end`/`words_at` replace the matching entirely — in order,
ASCII-case-insensitive, whitespace-tolerant, boundary-anchored, string-aware,
on the bytes of the statement itself. **And the forms the parser cannot read
are now refused rather than skipped:** `create_table_parts` returns a
three-armed `CreateTable`, so `AS SELECT`, `PARTITION OF`, `INHERITS`, a
`(LIKE …)` body and `ALTER TABLE … RENAME TO` each fail the gate **by name**,
telling the author to model the form before landing the migration. None of the
five appears in `backends/migrations` today, which is now a fact the gate holds
rather than one a reviewer re-establishes. Two smaller, fail-**closed** defects
went with them: `ADD`/`DROP` had no leading word boundary
(`ADD COLUMN c my_add TEXT` derived a phantom column `text`) and
`alter_drop_columns` lacked the depth guard `alter_add_columns` already had.
The gate derives the same 295 columns before and after, which is what a
hardening pass should do.

**Measured on this tree, 2026-09-17:** 295 database columns classified across
25 elements (16 personal-data, 17 necessary), checked in both directions
against the 295 the migrations derive; 10 non-database surfaces registered,
6 of them not yet statically enumerable — recorded as unmet criteria that keep
#144 open rather than faked (RFC-0002 PR 2: "record any surface that cannot be
enumerated as an unmet criterion rather than manufacturing a self-check").
_(It read 303/303 on 2026-09-16, before `DROP TABLE` was modelled.)_

**Six classifications the 2026-09-18 pass could not settle, and did not
quietly re-decide.** It read ten `subject: payer|staff|merchant` columns and
ten `subject: none|system` columns against the migration that defines each one,
and found no fabricated compliance claim, no invented lawful basis and no
retention period stated as a commitment — but six classifications the
migrations' own comments contradict, among them `checkout_sessions.publishable_key`
marked `control: forbid` where `0028` says "**NOT a secret**", and three
payer-facing session credentials filed under `subject: merchant` where `0028`
and `0026` call them "payer credentials". Four of the six are one defect
wearing four hats — an element carries one classification, and
`oauth_client_secret` holds both merchant secrets and payer credentials — which
is ADR-0020 §1's missing per-copy control with a line number attached. Each is
listed with the contradicting line in
[../reference/personal-data-inventory.md](../reference/personal-data-inventory.md)
§ "Classifications the review could not settle" and left for a maintainer:
re-deciding a GDPR artifact in a review pass, on a question RFC-0002 D1 has not
answered, is the failure this directory exists to prevent. Two outright false
statements on that page **were** fixed, because they were claims and not
judgements: its field table listed four `subject` values where the gate
enforces five, and it credited `provider_requests.error_kind` with "a closed
operator-label vocabulary" that `0016` says does not exist ("Free text on
purpose").

**Three more unmet criteria, found by the 2026-09-17 review and fixed in no code**,
because each would be a claim rather than a check: five columns carry
`control: redact` and no statement in this repository redacts them;
`jobs.last_error` holds the rail's own message under a `subject: none`
element; and ADR-0020 §1's per-copy necessity/control reference is absent from
the schema. All three are written down in
[../reference/personal-data-inventory.md](../reference/personal-data-inventory.md)
and keep #144 open alongside the six surfaces. The `enumerable` flag on the
other four surfaces is **counted, not derived** — nothing re-checks them.

**Mutations that prove each direction bites** (`privacy_inventory_tests` in
`.xtask/src/main.rs`, **26** cases as of 2026-09-18; 16 before it): adding a
`phone` column to a migration with no element fails direction A; adding a
`ghost` column to the inventory with no migration fails direction B; a dropped
table's column named by the inventory fails direction B; a duplicate surface
id, a surface id colliding with an element name, an out-of-vocabulary `control`
or `subject`, an unsupported copy kind and an inventory of another `version`
each fail; the DDL parser's handling of `DROP`/`RENAME`/`DROP TABLE`, its
constraint-keyword word boundary and the string-aware comment stripper are
pinned by their own tests. The ten added on 2026-09-18 pin one hole each:
`a_keyword_pair_survives_any_whitespace_between_its_words`,
`a_keyword_inside_an_identifier_is_not_a_keyword`,
`a_ddl_keyword_inside_a_string_literal_is_data`,
`a_create_table_the_parser_cannot_read_is_refused_not_skipped`,
`a_table_rename_is_refused_rather_than_silently_ignored`,
`an_alter_clause_inside_parentheses_is_not_a_column_operation` and
`strip_comments_preserves_byte_length_and_multibyte_text`. Every one of them
drives the real `migrations_db_columns` or `verify_privacy_inventory` over a
temp tree — no shipped migration is touched, because `verify-migrations`
checksums them.

## 2026-09-18 — `verify-versions`, the release that armed it, and the tests it went two pull requests without

`verify-versions` is the thirteenth gate in `just verify`. It was added on
2026-09-17 by [#201](https://github.com/vaam-apps/vpay/pull/201) and it was red
on `master` the same night — correctly: the first release release-please ever
cut here ([#203](https://github.com/vaam-apps/vpay/pull/203), merged 23:10) had
destroyed two of the files it was watching.

### What #203 did, and why

`release-please-config.json` listed `deploy/helm/vpay/Chart.yaml` and
`sdks/flutter/vpay_checkout_flutter/pubspec.yaml` in `extra-files` as **bare
strings**, on the assumption that a bare string gets the annotation-only
`Generic` updater. It does not. release-please infers an updater from the file
extension (`src/strategies/base.ts`, `extraFileUpdates`, byte-identical from
`17.6.0` — which `googleapis/release-please-action@v5` resolves to — through
`17.11.2`):

```text
.json        -> CompositeUpdater(GenericJson('$.version'),  Generic)
.yaml/.yml   -> CompositeUpdater(GenericYaml('$.version'),  Generic)
.toml        -> CompositeUpdater(GenericToml('$.version'),  Generic)
.xml         -> CompositeUpdater(GenericXml('/*/version'),  Generic)
anything else-> Generic
```

`CompositeUpdater` runs them in order and the document updater is first.
`GenericYaml` is js-yaml's `loadAll` followed by `dump`; its own doc comment
says the parser "does reformat the document and removes all comments". So the
annotation was gone before the half that reads it was handed the text.

| File                          | Lines     | Version effect                                                                                                                                                     |
| ----------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `deploy/helm/vpay/Chart.yaml` | 48 → 13   | chart `version:` **downgraded** `0.2.0` → `0.1.1` by `$.version` — the one field the config deliberately excluded — and the annotated `appVersion` left at `0.1.0` |
| `sdks/flutter/…/pubspec.yaml` | 60 → 45   | `version:` correct only by accident; `$.version` happens to be the right key for a pubspec                                                                         |
| four `Cargo.toml`             | unchanged | only the annotated lines moved                                                                                                                                     |
| `sdks/nodejs/src/version.ts`  | unchanged | only the annotated line moved                                                                                                                                      |

**The survivors are the interesting ones**, because neither is safe by virtue of
being a manifest. A Cargo manifest has no _top-level_ `version` key — it is
under `[workspace.package]` or `[package]` — so `GenericToml`'s `$.version`
matched nothing and it returned the bytes unchanged. `.ts` buys no updater at
all. `vsms` escaped the identical configuration by the same accident: its two
`.yaml` extra-files are compose files, which carry no top-level `version:`.

### What [#204](https://github.com/vaam-apps/vpay/pull/204) did

Restored both files verbatim from `a475a2d8`, moved `appVersion` and the plugin
to `0.1.1`, rewrote **every** `extra-files` entry as
`{"type": "generic", "path": …}` — which routes to release-please's
`case 'generic'` and runs the annotation-only updater whatever the extension —
and taught the gate to refuse a bare string outright, naming the incident. A
follow-up commit taught the parser the compact one-line object spelling as well
as the multi-line one.

### What this change adds

**Tests.** Two pull requests moved this gate and neither landed one; #204 proved
its new rule by editing the real config by hand and putting it back, which works
exactly once. `version_tests` in `.xtask/src/main.rs` now pins nine behaviours,
and the load-bearing one is `a_bare_string_entry_is_refused_and_the_message_names_the_repair`:
**that config was this repository's own state two commits ago, and every gate
here was green on it.**

**One hole, named rather than closed.** The compact-object branch is entered
only when a line carries both `"type"` and `"path"`, so a single-line object
with a `"path"` and **no** `"type"` matches no branch and is skipped in
silence — where the multi-line spelling of the same mistake is a hard error. It
is caught today only by the "found no `type: generic` entries" tripwire, and
only when it is the sole entry of its kind.
`a_single_line_object_with_no_type_is_skipped_silently_and_that_is_a_known_hole`
pins it. It is left as it is deliberately: #204 states that this parser is kept
identical to `vsms`' copy so the two cannot drift, and closing it needs the same
edit in both repositories. Written down here so it is a known gap rather than an
unknown one.

**And three things #203's fallout left behind**, none of them the gate:

- `CHANGELOG.md` — created by the release, in release-please's own style (`*`
  bullets, a double blank line before each `###`), and rejected by
  `prettier --check`. `master`'s `web` job was red on it and on `AGENTS.md` from
  2026-09-17 until this change. The changelog is now in `.prettierignore` with
  the reason: release-please splices a new section into that file at every
  release, so formatting it buys one clean run and makes every future release
  pull request red.
- `deploy/helm/vpay/Chart.yaml`'s own `version:`, `0.2.0` → `0.2.1`. `#204`
  moved `appVersion` to `0.1.1` and left the chart version behind, and the
  file's own rule two lines below says why that is half an edit:
  `values.yaml`'s `images.*.tag` defaults to `appVersion`, so a chart left at
  `0.2.0` now resolves a different image than the `0.2.0` anyone already has.
- the gate count. `AGENTS.md` and [../status.md](../status.md) both said
  **twelve** from 2026-09-17, when the recipe grew its thirteenth entry, until
  2026-09-18 — and `docs/status.md`'s gate table had no `verify-versions` row at
  all. AGENTS.md's own paragraph asks the next change to fix exactly this; it
  took three.

### What is still not checked

Named rather than left to look like an oversight: **nothing here runs
release-please.** This gate reads two JSON files as text. It can say that the
config is in a shape whose behaviour is known and that every version it names
agrees today; it cannot say what release-please will write. The first real
confirmation is the next release pull request's own diff, and it should be read
before it is merged.

The full record, with the mutation outputs, is in
[verification/2026-09-17-release-please-yaml-rewrite.md](verification/2026-09-17-release-please-yaml-rewrite.md).
