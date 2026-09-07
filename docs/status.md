# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked, and **since 2026-09-03 the check runs in both
directions**. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is
missing from this file — *and* fails if this file declares a token that no
shipping code carries any more, so a section that outlives the code it
described cannot sit here unnoticed. The scanner reads *code*, not text:
since 2026-09-05 it lexes, so a `NotImplemented("…")` written in a comment
of any kind (`//`, `///`, `//!`, `/* */` nested or not — leading **or**
trailing), in a `#[doc = "…"]` attribute, or inside any string, raw-string
or character literal is prose, and prose declares nothing. It was
comment-aware from 2026-09-03 (that blind spot is described in the Step 2
note below, where it was found), but only for comments that *began* a line,
and not at all for string literals; a trailing `// … NotImplemented("x")` or
an `r#"…"#` carrying the token still forced a phantom bullet into this file
— and because the check runs in both directions, the bullet then had to
stay, so the docs→code half could be satisfied by prose alone.
`cargo xtask verify-no-mocks` no longer greps the two app manifests: it
walks `cargo metadata`'s dependency graph from each shipping binary along
non-dev edges only, so a test double reachable through *any* intermediate
crate is caught, and it additionally refuses a test-only crate listed under
any workspace member's `[dependencies]` — with one narrow, documented
allowlist (`vpay-testkit` → `testcontainers`/`testcontainers-modules`,
because starting a real container is what that crate exists to do and
ADR-0006 says a stub rail **is** a WireMock host reached over HTTP). The
Rust `wiremock` crate — an *in-process* double — is allowlisted for nobody,
which is how `vpay-testkit`'s unused runtime dependency on it was found and
dropped.
**Extended 2026-09-03 (Step 7):** its `backends/apps` name scan also refuses
`vpay_db::connect_lazy` in non-test code. That function is *not* a test
double — the pool is the real `sqlx` one — which is exactly why no
dependency rule would ever object to it; what it defeats is the property
`connect` exists to hold, that a process which cannot reach its database
fails at boot rather than at the first payment.
AGENTS.md's claim that `verify-status` "fails in both directions" is,
as of this pass, true. Since 2026-09-02 `cargo xtask
verify-errors` likewise fails the build if an error type in
`backends/crates` is not classified per [ADR-0011](adr/0011-error-modelling.md),
or if `anyhow` leaks into a library crate. **Extended 2026-09-03 (Step 7):**
it additionally refuses a composite whose `#[from]` leaf is answered for by a
`_ =>` arm — for every `#[from]` variant, each `Classify` method that
*discriminates* on `self` must name `Self::<Variant>` explicitly.
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
one subtracted per violation). *(**Corrected 2026-09-04 (Step 8):** this
paragraph read ~~`14 error type(s)`~~ when Step 7 wrote it. Lane B added a
fifteenth, `vpay_worker::ssrf::EgressRefusal`, and no new `#[from]` variant,
so the second number is unchanged. `15` / `14` is what `cargo xtask
verify-errors` printed on the merged gate branch on 2026-09-04.)*

**New 2026-09-03 (Step 7, lane 4): `just verify` is three gates and one
report** — four gates since `verify-sdk-parity`, **five since
`verify-links` (2026-09-05)**, **six since `verify-npm-scope` (2026-09-05)**,
**seven since `check-schema` (2026-09-05, the CrateStack schema gate —
see "CrateStack" below)**, **nine since `verify-serde` and
`verify-repositories` (2026-09-05, [ADR-0016](adr/0016-engineering-standards.md))**
and **ten since `verify-toolchain` (2026-09-05, the pin-drift gate — see
"Toolchain pin" below)**, all seven below. `cargo xtask
verify-docs` prints, per crate, doc-comment lines
against code lines, **in-file comment lines against code lines, the number of
`#[doc = include_str!]` modules**, every production function of 80 lines or
more, every
```` ```ignore ```` doctest fence and every `#[allow]`/`#[expect]` in
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
```` ``` ```` fence, which are compiled examples rather than comment volume):

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
counted *every* doc line as prose and used a looser denominator; the same
convention on this tree reads doc 14 159 / code 15 964 = **88.6%**. The step's
own target was ≤40%, and **it was not met — not close.** What actually moved:
1 111 lines of prose became compiled examples, and about 700 more moved into
`docs/reference/`. Four crates are still over 100%, `vpay-config` is at
154.3%, and `vpay-testkit` at 221.3% was in no lane's
scope at all. Anyone reading this as "the docs were cleaned up" should read
the four crates' numbers instead.

Since 2026-09-03 `cargo xtask verify-sdk-parity` reads the fourth
machine-checked document,
[docs/sdks/parity.md](sdks/parity.md): the merchant SDKs (`sdks/rust`,
`sdks/nodejs`) must offer the same capabilities with the same wire
semantics — [ADR-0015](adr/0015-sdk-parity.md) — and every `✅` cell must
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
as *newly enforced*, not as *newly discovered defects*. **Re-measured on
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
[plans/exp15-notes/C.md](plans/exp15-notes/C.md).

**Reviewed the same day, and the enumerator had three silent holes — all
three closed** ([plans/exp15-notes/C-review.md](plans/exp15-notes/C-review.md)).
The review re-ran all five mutations above (all still correct) and added its
own; three of them passed when they should have failed, and each was a case
where the *other* SDK still backed the row, so the printed method count did
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
shape exists in `sdks/nodejs` today, checked module by module), and no rule
compares a row's per-column `✅`/`⛔` cell against whether *that* SDK
declares the method.

**New 2026-09-05: the three publishable npm packages are renamed
`@vpay/*` → `@vaam-apps/vpay-*`.** The organisation was renamed
`vaam-store` → `vaam-apps` on 2026-09-04, and the scope now matches it while
the package name keeps `vpay` so the name still says what it is:

| Was | Is | Directory |
|---|---|---|
| `@vpay/sdk` | `@vaam-apps/vpay-sdk` | `sdks/nodejs` |
| `@vpay/stripe-js` | `@vaam-apps/vpay-stripe-js` | `sdks/stripe-js` |
| `@vpay/stripe-compat` | `@vaam-apps/vpay-stripe-compat` | `sdks/stripe-compat` |

**Nothing had ever been published under the old names, so the rename cost
nothing and would have been expensive after a first publish.** Verified
rather than assumed, 2026-09-05, against `https://registry.npmjs.org/`:
`npm view @vpay/sdk version`, `npm view @vpay/stripe-js version` and
`npm view @vpay/stripe-compat version` each exit 1 with `E404 … is not in
this registry`, and so do all three *new* names, so none of them is taken by
anybody else either. The outputs are in
[docs/plans/exp7-notes/opus.md](plans/exp7-notes/opus.md).

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

Every occurrence of the three old names in *this* file was rewritten
mechanically, including inside dated notes recording commands that really
were run under the old spelling — so a Step 5b note reading `pnpm --filter
@vaam-apps/vpay-sdk build` is the current spelling of a command that ran as
`pnpm --filter @vpay/sdk build`. The old names survive verbatim in
`docs/plans/**` (closed, dated step and design notes) and in
[ADR-0010](adr/0010-merchant-auth-private-key-jwt.md) and
[ADR-0015](adr/0015-sdk-parity.md), which AGENTS.md makes immutable —
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
the ninth (both [ADR-0016](adr/0016-engineering-standards.md)),
`cargo xtask verify-toolchain` the tenth (added later the same day, in the
review of the toolchain bump), and `cargo xtask verify-citations` an eleventh
that is opt-in and in no CI job.** Until this
landed, `just docs-check` ran `verify-status` and printed `note: link
checking is not implemented yet`; the one class of claim this repository
makes about itself most often — *the document you are reading points at the
file it names* — was the one class no gate protected. The Step 9
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
  destination was *correct where that text lives* — `docs/flows/`
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
  reference *usages* with no definition (which render as literal text, so a
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
  the *CLI* at the pinned version, and the CLI is what `migrate baseline`
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
  fires, and what a green run does *not* prove.
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
  request`/`issue` cue, so an id cited *only* without one is not checked —
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
[ADR-0016](adr/0016-engineering-standards.md), the ADR that finally wrote down
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
  count of 28 is not a count of *defects*: the convention was already written
  down in `docs/reference/rails.md`, in both adapters' module docs and in a
  comment above `Currency` — in four places, checked by nobody. **The table is
  read in both directions:** a row naming a type that now complies, or a type
  that no longer exists, fails the build. **What it does not check:** whether
  a reason is a good one; "too many to fix" is a non-empty string and the gate
  cannot tell it from "models MTN's camelCase Collections wire".
- **`verify-repositories` — a gate.** Nothing outside `vpay-db` names a
  concrete repository implementation. The set of concrete types is *derived*
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
  a decision about a *pool*, not a licence to name an implementation type.
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
  [`docs/plans/exp10-notes/opus.md`](plans/exp10-notes/opus.md).


Last verified: 2026-09-07, on branch `claude/exp29-migration-manifest` at
commit `02445b8` — **the migration manifest gate (issue #76), and the runbook
repair it shipped with corrected**. `just ci` **exit 0**, recipe by recipe,
exit code read from a file rather than a banner. (`02445b8` is the head this
was measured at; the only commit after it is the one that writes this
paragraph, which no gate here reads differently.)

- `fmt-check`; `clippy` `-D warnings`.
- `verify`, **all eleven gates**: `verify-no-mocks`; `verify-status` 1 declared
  unimplemented item; `verify-errors` 18 error types, 16 `#[from]` variants
  delegating; `verify-sdk-parity` 407 proving tests, 35 dated gaps, 19 methods
  over 23 rows; `verify-links` **920 links in 167 tracked files**;
  `verify-npm-scope` 2 publishable packages; `check-schema` 19 declarations;
  `verify-serde` 73 types, 16 exempted; `verify-repositories` 4 concrete
  implementations named by none of 80 outside files; `verify-toolchain`
  1.98.0; **`verify-migrations` 35 migration files all matching the
  manifest**. `verify-docs` advisory.
- `test-rust` **1563 tests run, 1563 passed, 0 skipped** in 936.9 s across 46
  binaries against a real Postgres and real WireMock rails; `test-doc` **99
  passed, 1 ignored**; `verify-ignored` **0 ignored (expected 0), 46 binaries
  (expected 46), 1563 total (floor 1080)**.
- `lint-web`; `test-web` (checkout 448, nodejs SDK 190, stripe-js SDK 146,
  shop 96, api-client 4, ui 3); `deny` — advisories, bans, licenses, sources
  all ok.

**One caveat about this run, stated because a warning is not a pass:**
`check-schema` printed `WARNING — cratestack 0.11.1 on PATH, this repository
pins 0.12.0` and type-checked against the 0.11.1 grammar. That is this
machine's PATH, not this branch: CI's `self-checks` job installs the pinned
0.12.0 from the justfile. Nothing in this change touches `schemas/vpay.cstack`.

**An earlier run of the same branch was also exit 0 but took 2251 s**, because
`the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`
took **1201 s** on its own — `sqlx::migrate!` returns `VersionMismatch` before
its `conn.unlock()` (sqlx-core 0.9.0 `src/migrate/migrator.rs`), so a refused
migration hands its connection back to the pool holding the advisory lock and
the next `run()` blocks until that connection is reaped ten minutes later.
Running the refused migration on its own closed pool took it to **1.619 s**.
The test now asserts the leaked lock is there (`pg_locks`, `locktype =
'advisory' AND granted` = 1) so the workaround cannot outlive its reason.

**Superseded by the run above, kept for the record.**

Last verified: 2026-09-07, on branch `claude/exp21-checkout-page` at the head
of the sabotage review, rebased onto `origin/master` (PRs #55 #60 #62 #64) —
**the checkout page restyled and made runtime-configurable, and then fixed**.
`just ci` **exit 0**, recipe by recipe, read from a file rather than a banner:
`fmt-check`; `clippy` `-D warnings`; `verify`, all ten gates (`verify-links`
over **854 links in 153 tracked files**, `verify-status` 1 declared
unimplemented item, `verify-toolchain` 1.98.0); `test-rust` **1401 tests run,
1401 passed, 0 skipped** in 1022.8 s across 43 binaries against a real Postgres
and real WireMock rails; `test-doc` **96 passed, 1 ignored**; `verify-ignored`
**0 ignored (expected 0), 43 binaries (expected 43), 1401 total (floor 1080)**;
`lint-web`; `test-web`, of which `@vpay/checkout` is **448 cases in 23 files, 0
skipped** (was 302 in 17) and `@vpay/tokens` **7 in 1**; `deny` (advisories,
bans, licenses, sources all ok). **Nothing under `backends/` was touched**, so
every Rust number is `master`'s.

**And, for the first time on this branch, a real browser.** `just test-e2e`
against a compose stack of the review's own — **11 Cypress tests, 11 passing, 0
failing, 0 skipped**: `checkout.cy.ts` (1), `dashboard.cy.ts` (3),
`shop-hosted.cy.ts` (3), `shop-embedded.cy.ts` (4), through a stack that mounts
both runtime YAML files. **The first run of that command, before any fix, was
3 failing**: the runtime theme override was emitted inside an explicitly
written `<head>`, and React threw #418 — *hydration failed because the server
rendered HTML did not match the client* — uncaught, on the hosted payment page.
Every detail, including the three-way measurement that isolated it, is in
[`plans/exp21-checkout-page-notes/opus-review.md`](plans/exp21-checkout-page-notes/opus-review.md).

*Superseded, kept for the record — 2026-09-06, at `19274df`, before the
rebase and before any Cypress run:* **the checkout page restyled and made
runtime-configurable**
(`frontends/apps/checkout`; the maintainer's requirements of 2026-09-05, plus
a popup peer requested by the `examples/shop` track and a return-trip rule the
maintainer decided the same day). `just ci` **exit 0**, recipe by recipe:
`fmt-check`, `clippy`, all ten `verify-*` gates (`verify-links` over 806 links
in 146 tracked files; `verify-toolchain` on 1.98.0), `test-rust` **1382 tests
run, 1382 passed, 0 skipped** in 1084 s across 43 binaries against a real
Postgres and real WireMock rails, `test-doc` **91 passed, 1 ignored**,
`verify-ignored` **0 ignored (expected 0), 43 binaries (expected 43), 1382
total**, `lint-web`, `test-web` — of which `@vpay/checkout` is **442 cases in
22 files, 0 skipped**, up from 302 in 17 — and `deny` (advisories, bans,
licenses, sources all ok). **Nothing under `backends/` was touched**, so every
Rust number here is `master`'s. **What this run does NOT cover, and it is the
part that matters most: neither Cypress spec was run** — `just ci` never runs
them, and the binary could not be fetched here. See the
`frontends/apps/checkout` row for the four other things that remain unproven.
*That paragraph's "neither Cypress spec was run" was accurate, and the specs
were not passing; see the 2026-09-07 block above.*

Last verified: 2026-09-04, on branch `claude/step9-hosted-checkout` at
`e57e7ff` — **Step 9, hosted checkout**: twelve lanes merged (5 the SDKs, 2 the
return trip through the port, 3 the page, 2b the digits-only steering MSISDNs,
1 the Checkout Session object and both surfaces, 7 the demo shop, 3b the page's
three correctness fixes, 4 build/image/deploy/demo, 1b the integration seams
and the review's server-side findings, 5b the client-assertion audience, r2 the
second review round's remediation, 6 the Cypress proof), plus two commits of
the integrator's own: the `merchant` member is **absent** from a browser
session read when the deployment configured no display name (`6abbaa0`,
superseding lane 1b's tenant-id fallback), and the demo runbook's §4 walkthrough
re-pasted from the merged branch's own green VM run (`74f761f`). **The defect
this step found is worth naming here rather than in a row:** no merchant server
that reaches vpay by an internal URL — a compose service name, a private DNS
name, a mesh address — could authenticate at all, because both SDKs signed the
client assertion's `aud` with the URL they were POSTing to while vpay's OP
accepts only its own `deployment.public_base_url`. It was found by lane 6
running the demo shop inside the compose network, having survived three lanes
that never did (`docs/plans/step9-notes/lane-6.md` §2), and fixed by lane 5b.
**The review trail behind this note:** two adversarial reviews of the merged
gate (correctness/money-and-secrets, and conventions/blast radius), a second
round after the first remediation, and the remediations themselves reviewed —
lanes 1b, 3b, 5b and r2 are what came out of them.

**Every number in this bullet list was measured for this note on `e57e7ff`**,
in the gate worktree with `CARGO_BUILD_JOBS=4`, except the bullets that name
the integrator's `vpay-ci` VM as the measurer.

- `just verify`: **ok**, four gates and the advisory report — `verify-no-mocks:
  ok`; `verify-status: ok — 1 unimplemented item(s), all declared in
  docs/status.md and all still in shipping code`; `verify-errors: ok — 15 error
  type(s), all classified; 14 #[from] variant(s) delegate every Classify method
  they match on; anyhow confined to binaries`; `verify-sdk-parity: ok — 335
  proving test(s) named in docs/sdks/parity.md all exist, 26 dated gap(s)`.
- `just verify-ignored`: **`0 ignored (expected 0), 42 test binaries (expected
  42), 1137 total (minimum 1080)`** — up from Step 8's 1059 / 41 / 0. The
  forty-second binary is lane 1's `checkout_sessions.rs`; `expected_suites`
  moved 41 → 42 in that lane's commit and `min_tests` 1000 → 1050 → 1080 across
  lanes 1 and 1b, each in the commit that earned it. **`verify-ignored` *lists*
  those 1137 without executing them.**
- `just test-doc`: **84 passed, 0 failed, 1 ignored** — the one ignored doctest
  is `sdks/rust`'s README block and is pre-existing. Four new ones landed with
  lane 1 (`CheckoutConfig` twice, `ids::return_token`,
  `CheckoutSessionRow::return_page_url`).
- `just docs-check`: ~~`verify-status` ok; **link checking is still not
  implemented** and that recipe still says so.~~ **Corrected 2026-09-05:** it
  runs `verify-status` **and `verify-links`**, and the echo is gone — see the
  `verify-links` bullet above for the counts.
- `cargo nextest list` per binary, on this branch: **33** conformance cases (was
  28 — lane 2's return-URL case once per rail, lane 2b's digits-only twin ×3),
  **17** `checkout_sessions` (new), **12** `confirm_rails` (was 7), **11**
  `provider_callback`, **10** `browser_checkout`, **25** `payment_intents`,
  **23** `worker_recovery`, **17** `webhooks`, **14** `postgres_smoke`, **3**
  `worker_e2e`, **2** `worker_kill9`; **229** `vpay-api` unit tests (was 214),
  **106** `vpay-config`, **75** `vpay-worker`, **70** `vpay-core`, **69**
  `vpay-db::repositories`.
- `cargo xtask verify-docs` (advisory, never a gate): TOTAL **prose 16 126 /
  examples 1 208 / code 14 928 = 108.0%**, against 104.1% after Step 8. Nine
  production functions of 80 lines or more, two of them Step 9's —
  `browser/checkout_sessions.rs:206 fn authenticate` at 106 and
  `v1/checkout_sessions.rs:418 fn validate_create` at 92 — both recorded as
  decisions rather than suppressed (`docs/plans/step9-notes/lane-1b.md` §3,
  `lane-r2.md` finding 11). Zero ```` ```ignore ```` fences; four
  `#[allow]`/`#[expect]`, unchanged.
- **Measured by the integrator in the `vpay-ci` VM, on `551ec80`** — the gate
  one merge earlier, which lane 6 followed with Cypress specs, the `test-e2e`
  recipe, CI's `e2e` job, two `.gitignore` patterns and `@vpay/e2e`'s own
  scripts — **no Rust, and no package `pnpm -r test` runs** — so these counts
  are the same ones re-measured above:
  **`just ci` green from a clean build — 1137/1137 tests across 42 binaries, 0
  ignored; 84 doctests passed, 1 ignored; `@vaam-apps/vpay-stripe-js` 119,
  `@vaam-apps/vpay-sdk` 168, `@vpay-examples/shop` 57, `@vpay/checkout` 302,
  `@vpay/tokens` 3, `@vpay/ui` 3, `@vpay/api-client` 4; `cargo deny`
  advisories/bans/licenses/sources ok.** Earlier VM attempts that failed did so
  on a WireMock container-start timeout under host I/O load (943/944 before it),
  then on the VM's own disk filling with three build trees and the corrupt rlib
  that left behind — **none of them on code**, and none of them is a flake this
  branch owns.
- **`just demo` from nothing, in the VM on `551ec80`: three consecutive green
  runs**, six outcomes for six each, **XAF on both rails**, step 5 minting one
  hosted and one embedded Checkout Session; `write_matched_no_row` in no run's
  logs. **The paste in `docs/runbooks/demo.md` §4 is no longer one of them**:
  it was re-captured the same day from a fourth run, on the same tree plus a
  one-line correction to the `frame-ancestors` sentence step 5 prints, which
  claimed a browser refusal nothing here has observed. One earlier
  attempt failed before `demo-up` on a Docker Hub token fetch — the network,
  not the stack. **Step 8's bar of three from nothing is met, and consecutively
  this time.**
- **Cypress, in the VM, `just test-e2e` from nothing and unpatched: exit 0, 11
  tests across four specs, 0 failing, 0 skipped.** Pass 1 — `checkout.cy.ts`
  13 s (1), `dashboard.cy.ts` 0.4 s (3), `shop-hosted.cy.ts` 1 m 18 s (3);
  pass 2 (`VPAY_E2E_FRAMED=1`) — `shop-embedded.cy.ts` 13 s (4). Two runner
  facts bound what those specs can claim, and both are recorded on the rows
  rather than rounded off: Cypress rewrites `window.top`/`window.parent`, which
  is why the two page modes need two passes; and **Cypress strips
  `Content-Security-Policy`, so vpay's `frame-ancestors` is asserted as the
  server sends it and no browser has been observed enforcing it** — what a
  browser *was* seen enforcing is the checkout app's own origin check refusing
  an unregistered framer, proven origin-driven by registering that origin and
  watching the same page render.
- **The "do not deploy" banner stays, and the reason is unchanged. Nothing in
  Step 9 called a real rail.** A payer's browser now walks a whole checkout —
  shop, vpay's page, the rail's page, back to the shop, `paid` from a verified
  webhook — and every rail in that walk is a `wiremock/wiremock` container.
  **No HTTP call to a real rail has ever been made, no merchant endpoint
  outside this repository has ever been POSTed to, and no money has moved.**

The Step 8 note follows, unchanged:

Last verified: 2026-09-04, on branch `claude/step8-production-gate` —
**Step 8, the production gate**: six lanes merged at `ef19991` (D the real
`SIGKILL` test, B the runtime egress guard, C the rail callback route, G the
confirm/worker race fix, A the demo on both rails, F the SDK parity matrix),
and a seventh at `1c742a4` — **lane H, the four findings Step 8's own
correctness review confirmed against lanes B, C, D and G**: the recovery age
stopped being the worker host's clock minus Postgres', `RecoveryAction::Wait`
stopped spending six ladder rungs proving a charge was young, the callback
route's pull-forward gained a floor, and four reserved prefixes stopped being
deliverable. **The clock defect was found by the correctness review reading
lane G's fix — not by lane G's own tests**, which passed with the defect in
place because the authoring host's clock agreed with the database's; that is
the argument for reviewing a remediation rather than trusting it. **The review
trail behind this note: three reviews of the merged gate — correctness,
documentation accuracy, blast radius — and two remediation reviews, one per
remediation.** The documentation remediation is the `docs(…)` run from
`0c9f767` to `4900740`; the correctness remediation is lane H (`5ba6b11`,
`605f4da`, `6987e31`, `8508b31`).
**Every number in this bullet list was measured for this note** — on `ef19991`
unless the bullet names `1c742a4` — with `CARGO_BUILD_JOBS=4` and
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, except the bullets that say
who measured them instead.

- `just verify`: **ok**, four gates and the advisory report — `verify-no-mocks:
  ok`; `verify-status: ok — 1 unimplemented item(s), all declared in
  docs/status.md and all still in shipping code`; `verify-errors: ok — 15 error
  type(s), all classified; 14 #[from] variant(s) delegate every Classify method
  they match on; anyhow confined to binaries`; `verify-sdk-parity: ok — 267
  proving test(s) named in docs/sdks/parity.md all exist, 24 dated gap(s)`.
- `just verify-ignored`: **`0 ignored (expected 0), 41 test binaries (expected
  41), 1059 total (minimum 1000)`** — re-measured on `1c742a4` with `cargo
  nextest list --workspace` through the recipe's own `jq`, up from the 1054 of
  `ef19991`. 41 binaries because lanes D and C each added one
  (`worker_kill9.rs`, `provider_callback.rs`); lane H's five new cases all
  landed in files that already existed, so neither counter in the `justfile`
  moves and its own comment accounts for the 1059 lane by lane.
  **`verify-ignored` *lists* those 1059 without executing them** — what it
  proves is that every test binary still builds and no test vanished, not that
  all 1059 pass.
- `just test-doc`: **77 passed, 0 failed, 1 ignored** — the one ignored doctest
  is `sdks/rust`'s and is pre-existing, unchanged by this step.
- `just docs-check`: ~~`verify-status` ok; **link checking is still not
  implemented** and that recipe still says so.~~ (True of the Step 8 tree this
  block measures; **link checking landed 2026-09-05**, above.)
- `cargo nextest list` (re-measured on `1c742a4`): **28** conformance cases
  (was 26 — lane C's `the_submit_tells_the_rail_where_to_call_back`, once per
  rail), **11** `provider_callback` cases (was 9 — lane H's floor join and the
  poll it refuses to accelerate), **2** `worker_kill9` cases, **23**
  `worker_recovery`, **7** `confirm_rails`, **3** `worker_e2e`, **17**
  `webhooks`, **75** `vpay-worker` unit tests (was 73 — lane H's age case and
  wait case), **82** `vpay-db` (was 81 — lane H's clock case), **214**
  `vpay-api`.
- `cargo xtask verify-docs` (advisory, never a gate; re-measured on `1c742a4`):
  TOTAL **prose 14 072 / examples 1 138 / code 13 510 = 104.1%**, against
  103.2% on `ef19991` and Step 7's 100.8% — every lane that shipped code
  shipped prose with it, lane H included. In the Step 7 design's own looser
  convention: doc 15 210 / code 16 704 = **91.0%**, against Step 7's 88.6% and
  a target of ≤40% that is still not met and still moving the wrong way.
  `poll_charge` is on the long-function list at **115** lines, three longer
  than before lane H (the `ChargeAsOf` destructuring and the comment saying why
  the clock is Postgres'), and `vpay_worker::recovery` is on the prose-ratio
  list at 292.9%.
- **Measured by the integrator on `ef19991`, not re-run for this note:**
  `cargo nextest run -p vpay-tests-integration -E 'binary(worker_kill9) |
  binary(provider_callback)'` — **11 passed**; and `-E 'binary(worker_recovery)
  | binary(confirm_rails) | binary(worker_e2e) | binary(worker_kill9)'` —
  **35 passed**, after the ageing fix described in the `SIGKILL` row below.
  Both are superseded as counts by lane H, which added two `provider_callback`
  cases: **measured by lane H on its own branch `claude/step8-review-r1`**,
  `-p vpay-worker -p vpay-db -p vpay-api` — **371 passed, 0 skipped, 0
  ignored**, and `-E 'binary(worker_recovery) | binary(worker_kill9) |
  binary(provider_callback) | binary(confirm_rails)'` — **43 passed, 0
  skipped**, neither re-run on `1c742a4` for this note.
- ~~**Not run for this note, and not claimed:** the full `cargo nextest run
  --workspace`, `just ci`, Cypress, `just helm-check`, `sdks/stripe-compat`,
  and **`just demo`**.~~ **Run by the integrator on 2026-09-04 in the `vpay-ci`
  VM on the merged code (`1c742a4`, lane H in): `just ci` green — `cargo
  nextest run --workspace` 1059/1059 across 41 binaries, 0 ignored, 77 doctests,
  `lint-web`/`test-web` and `deny` green; `sdks/stripe-compat` 25/25.** Still
  not run: Cypress and `just helm-check` (both are CI jobs; the PR's checks are
  their evidence). The demo's own state is in the "Local demo" row.
  ~~A green six-outcome run from nothing was observed on lane A's rebased
  branch (which carried lane G's fix).~~ **Corrected 2026-09-04:** one green
  run from nothing exists (lane A's rebased branch, 2026-09-04, **without**
  lane G; the race is timing-dependent and did not fire), lane A's own earlier
  count was two greens in six attempts and zero for three from nothing, lane G
  did not re-run the demo. **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing *after* the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both.
- **The "do not deploy" banner stays, and the reason is unchanged.** Nothing in
  Step 8 called a real rail. The egress guard was proven against a container on
  a compose network, the callback route against bodies transcribed from
  `docs/flows/adapter-*.md`, the kill test against a WireMock rail, and the
  demo against two WireMock rails. **No HTTP call to a real rail has ever been
  made, and no merchant endpoint has ever been POSTed to.**

The pre-Step-8 note follows, unchanged:

Last verified: 2026-09-03, on branch `claude/step7-cleanup` — **Step 7 Phase A**
(the `vpay-db` repository layer and the `ProviderError` source chain) on top of
the pending Step 5c tree, plus the **Phase A review remediation** (the
`connect_lazy` guard, the broadened delegation check, `TxOutcome::Abandon`'s
swallowed rollback). **Every number below was measured directly for this
note, on the authoring machine, with the toolchain pinned to `1.95.0` and
`DOCKER_HOST=unix:///run/user/1000/docker.sock`** — except where a bullet says
which pass it was measured in.

- `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets --
  -D warnings`: clean. `cargo deny check`: `advisories ok, bans ok, licenses
  ok, sources ok`. `just verify`: ok — `verify-no-mocks` clean,
  `verify-status` `1 unimplemented item(s)`, `verify-errors` `13 error
  type(s), all classified; 14 #[from] variant(s) delegate every Classify
  method they match on`.
- `just verify-ignored`, **Phase A remediation figure, superseded by the
  final lanes-landed bullet below**: **`0 ignored (expected 0), 39 test
  binaries (expected 39), 976 total (minimum 900)`** — 976 rather than the 972 the
  Phase A pass measured, for four new tests and no new binary:
  `an_abandoned_transaction_survives_a_rollback_it_cannot_send`
  (`vpay-db/tests/postgres.rs`) and three `xtask` units
  (`a_from_variant_swallowed_by_an_if_let_ladder_is_reported`,
  `a_binary_that_opens_its_pool_lazily_is_a_violation`,
  `a_stub_adapter_named_anywhere_in_a_binary_is_a_violation`). The Phase A
  pass's own 972 was 969 plus `a_transport_failures_source_chain_reaches_the_
  reqwest_error` (`vpay-adapter-mtn-momo`) and two `xtask` units.
  `expected_suites` and `min_tests` are unchanged and were not touched **as
  of this Phase A remediation measurement** — both moved later, to 950, in
  the lane-4 pass the final bullet below reports.
- **`VPAY_REQUIRE_NODE=1 cargo nextest run --workspace --no-fail-fast
  --retries 2`: 972 tests run, 972 passed, 0 skipped** (540.0 s), against
  real `postgres:16-alpine` and `wiremock/wiremock` containers, **zero
  retries consumed**. An earlier run on the same tree, before `pnpm install`
  had been run in this worktree, failed
  `the_delivered_signature_verifies_with_the_shipping_node_sdk` three times
  with `tsc: not found` — a missing toolchain in the worktree, not a defect;
  recorded rather than quietly re-run away. **This figure is from the Phase A
  pass and has not been re-measured since the remediation**; what the
  remediation pass ran instead is the next bullet.
- Remediation pass, measured on this commit: `cargo nextest run -p vpay-db -p
  vpay-api -p vpay-worker -p xtask` — **400 tests run, 400 passed, 0
  skipped** (122.1 s); `cargo nextest run -p vpay-tests-integration -E
  'binary(confirm_rails) | binary(payment_intents)'` — **32 tests run, 32
  passed, 0 skipped** (90.3 s). The remaining 544 tests in the workspace were
  **not** re-run on this commit.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean. It was
  **not** clean the first time: moving 60 free functions into trait methods
  broke 120 intra-doc links in `vpay-db`, `vpay-api` and `vpay-worker`, which
  is exactly the failure mode this flag exists to catch. Every link was
  repointed at the method's new home; none was deleted or downgraded to a
  code span except `PgRepositories`, which is now private.
- `just lint-web`: clean across all fourteen TypeScript workspace projects.
  `pnpm -r test`: 248 tests passed (`sdks/nodejs` 150, `sdks/stripe-js` 88,
  `frontends/packages/api-client` 4, `frontends/packages/tokens` 3,
  `frontends/packages/ui` 3).
- `just demo_port=18085 demo`: **all seven steps passed on the first
  attempt**, including settlement (`succeeded`) and the delivered webhook
  verified with the shipping SDK. `docker ps --filter name=vpay-` was empty
  before starting and after `just demo_port=18085 demo-down`.
  `just demo_port=18085 stripe-compat`: **25 tests passed, 5 files**, through
  the real `stripe` npm package against the running stack.
- **Not run for this note**: Cypress (`just test-e2e`), `just helm-check`,
  and CI. No CI run of this exact tree exists — which now matters more than it
  did, because Step 7 adds a step to CI's `rust` job (`just test-doc`) and one
  to its `self-checks` job (`cargo xtask verify-docs`) that no CI run has ever
  executed. `actionlint` **was** run for the lane-4 pass over the edited
  workflow and is clean; that is a syntax check, not a run.
- **Measured after all five Step 7 lanes landed, on a clean working tree:**
  `cargo fmt --all -- --check` clean, `just verify` ok (three gates plus the
  advisory report), `just verify-ignored` `0 ignored (expected 0), 39 test
  binaries (expected 39), 999 total (minimum 950)`, `just test-doc` 77 passed
  / 1 ignored / 0 failed, `cargo nextest run -p vpay-worker-bin` 12/12.
  **Not re-run for this note: the full `cargo nextest run --workspace`.**
  `verify-ignored` *lists* all 999 without executing them, so what is proven
  here is that every test binary still builds and no test vanished — not that
  all 999 pass on this tree. The last full execution was the Phase A pass
  (972 passed), before four of the five lanes existed.

The pre-Step-7 note follows, unchanged:

Last verified: 2026-09-03, on branch `claude/step5c-stripejs` at commit
`b57e0ce` — Step 5c (`/v1/browser`, `@vaam-apps/vpay-stripe-js`, `examples/checkout-browser`)
squashed and rebased onto `master` after Step 6 landed there
(`git reset --soft` to the pre-rebase base, then `git rebase --onto
origin/master`). **Every number below was measured directly for this note, on
the authoring machine, with the toolchain pinned to `1.95.0` and
`DOCKER_HOST=unix:///run/user/1000/docker.sock`.**

- Two real rebase conflicts (`backends/crates/vpay-api/src/lib.rs`'s import
  list and one doc-comment paragraph; `justfile`'s `expected_suites` history
  comment) — both resolved keeping both sides' intent; no conflict markers
  remain anywhere (`grep -rn '<<<<<<<' --exclude-dir=target
  --exclude-dir=node_modules .` is empty). Beyond the textual conflicts, the
  merge left the `/v1/browser` nest without the `track_http_metrics` layer
  Step 6 added to the other two nests — nothing textual to conflict on, since
  neither side's diff touched the same lines, but the result was silently
  wrong. Fixed by adding the layer to the browser nest, extending
  `every_mounted_group_is_counted_exactly_once` (`vpay-api` unit test) to
  cover it as a fourth group, and adding
  `a_browser_get_is_counted_under_its_own_route_pattern` — a new integration
  test in `browser_checkout.rs` that drives a real browser `GET` through
  Postgres and WireMock and scrapes a real `/metrics` listener for
  `vpay_http_requests_total{route="/v1/browser/payment_intents/{id}",method="GET",status="200"}`.
- `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets --
  -D warnings`: clean. `cargo deny check`: `advisories ok, bans ok, licenses
  ok, sources ok`. `just verify`: ok (`verify-no-mocks` clean, `verify-status`
  `1 unimplemented item(s)`, `verify-errors` `12 error type(s), all
  classified`). `just helm-check`: 15 guards, all fired by name; kubeconform
  `20 resources found in 2 files - Valid: 20, Invalid: 0, Errors: 0, Skipped:
  0`. `actionlint`: clean.
- `just verify-ignored`: **`0 ignored (expected 0), 39 test binaries (expected
  39), 969 total (minimum 900)`** — 969 rather than the pre-rebase 927
  because Step 5c's own suite (`browser_checkout.rs`, a new binary, 38 → 39)
  landed on top of Step 6's count, plus the metrics test added above.
- **`VPAY_REQUIRE_NODE=1 cargo nextest run --workspace --no-fail-fast
  --retries 2`, the whole suite: 969 tests run, 969 passed, 0 skipped**
  (542.1 s), against real `postgres:16-alpine` and `wiremock/wiremock`
  containers, **zero retries consumed on the passing run.** A first attempt
  on this tree — before the fix described in the next paragraph — genuinely
  failed twice each, all three tries, on two tests; that was a real defect
  this pass found and fixed, not flakiness, and is recorded rather than
  quietly re-run away.
- **A real, pre-existing gap this pass found and fixed, not a rebase
  artefact**: `backends/tests/integration/tests/confirm_rails.rs`'s
  `a_push_confirm_the_rail_accepts_moves_the_intent_to_processing` and
  `redirect_confirm_commits_the_rails_material_before_it_answers` compared a
  `confirm` response and a later `retrieve` for full `PaymentIntent`
  equality — an invariant that predates Step 5c's D2
  (`vpay_api::model::PaymentIntentWithSecret`: `client_secret` is rendered by
  `create`, `retrieve` and the two browser routes, **never** by `confirm`).
  Neither file is touched by Step 5c's own commits; the SDK typing fix from
  the concurrent security-hardening pass (`c40a137`, see this step's own
  plan Outcome section) added `sdks/rust`'s `PaymentIntent.client_secret`
  field, which made this pre-existing equality assertion observe the
  documented asymmetry for the first time once both landed on the same tree.
  Not a real bug — `confirm` is `SecretRendering::Omit` by design and
  `retrieve` is `Include` — so both tests were corrected to assert the
  asymmetry explicitly and then compare the rest of the object with
  `client_secret` normalised out, rather than weakening the invariant they
  exist to prove. `examples/merchant-demo`'s step 5 had the **identical**
  comparison and the identical failure — found by actually running `just
  demo`, not by inspection (the confirm succeeded, the retrieve succeeded,
  and the demo still exited 1 at "the confirm's response and a later
  retrieve are different objects") — and was fixed the same way.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean.
  `just lint-web`: clean across all fourteen TypeScript workspace projects.
  `pnpm -r test`: clean — 247 tests (`sdks/nodejs` 150, `sdks/stripe-js` 87,
  `frontends/packages/tokens` 3, `frontends/packages/api-client` 4,
  `frontends/packages/ui` 3, `frontends/apps/dashboard` 0
  `--passWithNoTests`; `frontends/packages/config` and `sdks/stripe-compat`
  carry no `test` script).
- `just demo_port=18084 demo`: **failed at step 5 on the first attempt**,
  with the exact `confirm`/`retrieve` comparison bug described above —
  `docker ps --filter name=vpay-` was empty before starting, so this was the
  code, not a contended compose project. Fixed, rebuilt, and re-run: all
  seven steps passed, including settlement (`succeeded` after 7 polls) and
  webhook delivery (`Vpay-Signature` verified with the shipping SDK).
  `just demo_port=18084 demo-down` afterward; `docker ps --filter
  name=vpay-` empty again.
  `just build-checkout-browser` (vendors `@vaam-apps/vpay-stripe-js`'s build into
  `examples/checkout-browser/dist/stripe-js/`) was run first, then
  `pnpm --filter @vpay/e2e e2e` against the running stack
  (`VPAY_BASE_URL=http://localhost:18084`,
  `VPAY_MERCHANT_CLIENT_ID=demo-merchant`,
  `VPAY_MERCHANT_PRIVATE_KEY_PATH=.e2e/demo-merchant/oauth-signing-key.pem`):
  **both specs passed, 4/4** — `checkout.cy.ts` (2321 ms) and
  `dashboard.cy.ts`'s existing three.
- **Not re-run for this note**: no fresh Docker-daemon inotify exhaustion was
  hit this pass (eleven dead `created` testcontainers from an earlier,
  unrelated run were cleared before the nextest run below; that was
  environment hygiene, not a defect this tree has). **No CI run of this
  exact tree exists yet** — same caveat the pre-rebase Step 5c note below
  already states.

The pre-rebase "Step 6 rebase" pass's own note follows, unchanged:

Last verified: 2026-09-03, on branch `claude/step6-deployment` at commit
`9354e91` — Step 6 rebased onto `master` after Steps 4, 5 and 5b landed there,
plus the webhook-delivery metric, two CodeQL cleartext-logging fixes and the
seven-variable `rails-secret` guard/README update this pass added. **Every
number below was measured directly for this note, on the authoring machine,
with the toolchain pinned to `1.95.0` and
`DOCKER_HOST=unix:///run/user/1000/docker.sock`.**

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `just verify`: ok — `verify-no-mocks` clean, `verify-status` `1
  unimplemented item(s), all declared in docs/status.md and all still in
  shipping code`, `verify-errors` `12 error type(s), all classified; anyhow
  confined to binaries`.
- `just helm-check`: 15 guards, all fired by name; `/v1` `limit-rps=20` and
  `/v1/oauth/token` `limit-rps=5`; kubeconform `20 resources found in 2 files
  - Valid: 20, Invalid: 0, Errors: 0, Skipped: 0`.
- `actionlint` over `.github/workflows/`: clean (no `shellcheck` installed,
  so the `run:` bodies are not separately linted).
- `just verify-ignored`: `0 ignored (expected 0), 38 test binaries (expected
  38), 927 total (minimum 900)`. 38 binaries, unchanged from the pre-rebase
  count — the two new metric assertions extend existing tests in
  `webhooks.rs` rather than adding a binary; the floor moved from 840 to 900,
  a little under the measured 927.
- **`VPAY_REQUIRE_NODE=1 cargo nextest run --workspace --no-fail-fast
  --retries 2`, the whole suite including every container-backed one: 927
  tests run, 927 passed, 0 skipped** (458–487 s across two clean runs),
  against real `postgres:16-alpine` and `wiremock/wiremock` containers, no
  retries consumed. *A first attempt on this tree, before `pnpm install` had
  ever been run in this worktree, failed
  `the_delivered_signature_verifies_with_the_shipping_node_sdk` on all three
  tries with "`pnpm --filter @vaam-apps/vpay-sdk build` failed: `tsc: not found`" — a
  missing `node_modules`, not a code defect. Running `pnpm install` and
  re-running that one test, then the whole suite again, confirmed it: 927/927
  clean both times after. Recorded so the next person hitting the same
  message knows it is an environment gap, not a regression.* Also cleared
  before both runs: ~90 dead `created` containers left by earlier failed
  starts elsewhere on this machine, the same `fs.inotify.max_user_instances`
  exhaustion the pre-rebase Step 6 note below describes; removing them left
  both runs clean with zero retries.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean.
- `just lint-web` (`pnpm --filter @vaam-apps/vpay-sdk build` then `pnpm -r
  typecheck`): clean across all eight TypeScript packages.
- `pnpm -r test`: clean — `frontends/packages/tokens` (3), `sdks/nodejs`
  (148), `frontends/packages/api-client` (4), `frontends/packages/ui` (3),
  `frontends/apps/dashboard` (0, `--passWithNoTests`, no test files exist
  yet); `frontends/packages/config` and `sdks/stripe-compat` carry no `test`
  script (`sdks/stripe-compat`'s conformance suite is deliberately excluded
  from this recipe — see `.github/workflows/ci.yml`'s `web` job).
- `just demo_port=18083 demo`: all seven steps passed, including the
  webhook-delivery step (`vpay-sdk` verified the delivered
  `payment_intent.succeeded`). `docker ps --filter name=vpay-` was empty
  before this run, so nothing was contending for the compose project name
  this time (contrast the pre-rebase note below, where a second worktree's
  stack was still up). `/metrics` was then scraped from **inside** the
  compose network (`docker run --rm --network vpay_default curlimages/curl
  ... http://vpay-server:9090/metrics` and the same against `vpay-worker`,
  the observability port not being published to the host): the server
  rendered `vpay_error_events_total`, `vpay_http_requests_total`,
  `vpay_provider_requests_total`, `vpay_charge_transitions_total`,
  `vpay_build_info`, `vpay_http_request_duration_seconds` and
  `vpay_provider_request_duration_seconds`; the worker rendered
  `vpay_jobs_claimed_total`, `vpay_charge_transitions_total`,
  `vpay_provider_requests_total`, **`vpay_webhook_deliveries_total`**,
  `vpay_jobs_completed_total`, `vpay_build_info` and
  `vpay_provider_request_duration_seconds`. `vpay_webhook_deliveries_total`
  read `{outcome="succeeded"} 1` — the demo's one delivery, counted at the
  seam this pass added. `vpay_jobs_oldest_claimable_age_seconds` was absent
  on the first scrape (the loop's 60 s gauge task had not ticked yet) and
  present and negative (`-0.543394915`) on a second scrape ~75 s in, matching
  the documented "unwritten until the gauge task runs" behaviour. `GET
  /livez` answered `200` on both processes over the same network. Torn down
  with `just demo_port=18083 demo-down`; `docker ps --filter name=vpay-` was
  empty afterward.
- `just ci` (`fmt-check clippy verify test-rust verify-ignored lint-web
  test-web deny`, plain `cargo nextest run --workspace` rather than the
  `VPAY_REQUIRE_NODE=1 --retries 2` invocation above): clean end to end,
  `927 tests run, 927 passed, 0 skipped`.

**Rebase conflicts**, one line each, resolved keeping both sides' intent:
`.github/workflows/ci.yml` (master's `compose.demo.yml` teardown line kept,
Step 6's new `deploy` job appended after it — master had no such job yet);
`Cargo.lock` (`vpay-worker`'s dependency list, both sides' additive entries
kept, then verified `--locked`-consistent); `backends/apps/vpay-worker-bin/src/main.rs`
(module doc comment only — combined master's webhook-outbox sentence with
Step 6's observability-listener sentence; the code itself had already merged
without a conflict, webhook wiring and the observability listener coexisting
cleanly); `backends/crates/vpay-api/src/lib.rs` (import list only — kept both
`HeaderName` (master, Step 5's `request-id` header) and `MatchedPath` (Step 6,
route-pattern metrics labels), both used elsewhere in the file unconflicted);
`backends/tests/integration/tests/worker_e2e.rs` (a comment about
`fanout_state` determinism from master reordered around Step 6's metrics
scrape block — both kept, in the order that reads correctly); `docs/runbooks/
README.md` (master's `webhook-delivery-failures.md` row and its own "Status"
paragraph merged into Step 6's table-with-Alert-column and its longer
evidence bullets); `docs/status.md` (three separate regions: this note now
prepended ahead of the unconflicted Step 5 and Step 4 notes below it; a
"Webhooks (signing, outbox, delivery)" row kept from master in full — Step 6's
side of that conflict was a stub predating Step 5's landing; and the HTTP
surface row's two tails — master's `/v1/events` mount sentence and Step 6's
probe-split/`track_http_metrics` sentence — concatenated into one row);
`justfile` (`expected_suites`/`min_tests` — master's `38`/`870` kept as the
starting point, then both re-measured for this note per the history comment
above). No conflict markers remain anywhere in the tree — a grep for the
seven-angle-bracket rebase marker across the tree, excluding `target` and
`node_modules`, returns nothing.

The pre-rebase "Step 6" pass's own note follows, unchanged:

Last verified: 2026-09-03, on branch `claude/step6-deployment`
(pre-rebase, before Steps 4/5/5b were rebased in) at the squashed pre-rebase
Step 6 commit (the numbers below were measured on the remediated tree, before
this pass rebased it). **Everything in this
paragraph was measured directly for this note, on the authoring machine, with
the toolchain pinned to `1.95.0` and
`DOCKER_HOST=unix:///run/user/1000/docker.sock`.**

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `just verify`: ok — `verify-no-mocks` clean, `verify-status` `1
  unimplemented item(s), all declared in docs/status.md and all still in
  shipping code`, `verify-errors` `12 error type(s), all classified; anyhow
  confined to binaries`.
- `just verify-ignored`: `0 ignored (expected 0), 37 test binaries (expected
  37), 846 total (minimum 840)`. The floor moved from 790 to 840 in this pass,
  a few below the measured total.
- **`cargo nextest run --workspace`, the whole suite including every
  container-backed one: 846 tests run, 846 passed, 0 skipped** (592 s),
  against real `postgres:16-alpine` and `wiremock/wiremock` containers.
  Step 4's note recorded 806; the difference is Step 6's own tests plus three
  added by the review remediation (the bounded `method` label, the bounded
  `method` label's spellings, and the committed-vs-rolled-back charge
  transition).
  *Two earlier attempts at that same command on the same tree did not
  finish, and neither was a test defect: each stopped on a different
  container-backed test with testcontainers' `Timeout error` /
  `WaitContainer(StartupTimeout)` while the rootless Docker daemon was
  logging `failed to create inotify fd: too many open files`
  (`fs.inotify.max_user_instances` is 128 on this machine and was
  saturated) with ~100 dead `created` containers left by earlier failed
  starts. Clearing those made container starts work again and the third run
  was clean. Recorded because "846/846" on a machine that needed two
  retries to get there is a weaker statement than it looks, and the next
  person seeing that timeout should know where to look.*
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean.
- `just helm-check`: 15 guards, all fired by name; `/v1` `limit-rps=20` and
  `/v1/oauth/token` `limit-rps=5`; kubeconform `20 resources found in 2 files
  - Valid: 20, Invalid: 0, Errors: 0, Skipped: 0`. Not part of `just ci` (it
  needs the network for its schemas).
- `actionlint` over `.github/workflows/`: clean (with `-shellcheck=` too — no
  `shellcheck` was installed to lint the `run:` bodies).

**What this note does *not* claim.** `pnpm -r typecheck` and `pnpm -r test`
were **not** re-run for it. **No CI run exists for this branch.** No cluster
has ever run the Helm chart, no Prometheus has ever scraped a vpay process,
`release.yml` has never run and no image has been published. No real rail has
been called by anything, ever.

**`just demo` was not completed for this note, and the reason is not a
defect in this branch.** `just demo_port=18083 demo` built all three images,
brought the stack up and passed steps 1-4 of six (discovery and JWKS, the
`private_key_jwt` token exchange, the unauthenticated 401, and
create-then-retrieve through the SDK). Step 5 — the confirm that reaches the
rail — answered `502 provider_unavailable`, because a **different worktree on
the same machine** (`vpay-step5b-stripe`) ran its own `docker compose up` on
the same compose project name `vpay` while this run was in flight, replacing
the WireMock rail stub underneath it. That stack is still up and was left
alone rather than torn down: its `vpay-server` publishes 18080 (not this
run's 18083), its issuer is `http://localhost:18080/v1/oauth`, and its
startup banner still says "No rail adapter implements `submit`" — the
pre-Step-3 capability set, so it is demonstrably not this branch's binary.
`just demo` on this branch therefore has evidence for steps 1-4 only, and
nobody should read the step-5 failure as a statement about the rail path;
re-run it when no other `vpay` compose project is up.

**What this pass adds is a deployment surface and the instrumentation under
it — and no evidence that any of it has been operated.** A Helm chart with
fifteen named template guards, a release workflow, an observability listener
on a second port, twelve metric names with one seam each, five alert rules and
eight runbooks. Every one of those is rendered, linted, schema-validated or
unit-tested; **not one of them has been applied to a cluster, scraped by a
Prometheus, or run as a release.** The rows below say, per artefact, which
half of that sentence applies. The review remediation on top of it closed two
cardinality-and-correctness defects in the instrumentation itself (an
unbounded `method` label on `vpay_http_requests_total`; a charge transition
counted before its transaction committed), widened
`VpayProviderErrorRateHigh` so a rail outage can fire it, narrowed
`release.yml`'s permissions, and retired the "this listener does not exist
yet" claims that block A had made false the same day.

A later pass on the same branch cleared the GitHub code-scanning alerts CodeQL
raises on `master`: `ci.yml` and `docs.yml` now declare a workflow-level
`permissions: contents: read` (no job overrides it — none of them write to
GitHub), and the `vpay-config` test that proves `Config`'s `Debug` redacts
credentials no longer prints that `Debug` string in its own failure message,
which is the one moment the string is known to hold a credential in cleartext.
The assertions are unchanged; only the messages are, and each now names the
credential's path in `config/application.yml`. The two remaining
`rust/cleartext-logging` alerts are in Step 5 files that do not exist on this
branch (`vpay-worker/src/signing.rs`, `tests/integration/tests/webhooks.rs`)
and are fixed after the rebase onto `master`.

The "Step 5" pass's own note follows, unchanged:

Last verified: 2026-09-03, on branch `claude/step5-webhooks` (the "Step 5"
webhooks pass) at commit `608ce96` — Step 4 plus the rebased Step 5 work and its
binary wiring. **The gate numbers in this paragraph were measured on the
authoring machine with the toolchain pinned to `1.95.0` and
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, and the workspace run was
measured by the pass that wired the binary, not re-run for this note — which is
said again below.** `just verify`: ok — `verify-no-mocks` clean,
`verify-status` `1 unimplemented item(s), all declared in docs/status.md and
all still in shipping code`, `verify-errors` `12 error type(s), all classified;
anyhow confined to binaries`. `just verify-ignored`: **`0 ignored (expected 0),
38 test binaries (expected 38), 865 total (minimum 840)`** — re-run directly for
this note, after the security remediation. **`VPAY_REQUIRE_NODE=1 cargo nextest
run --workspace`: 855 run, 855 passed, 0 skipped**, against real
`postgres:16-alpine` and `wiremock/wiremock` containers — that run is the wiring
pass's, and the remediation has since added ten tests it did not execute (see
the paragraph below). The `VPAY_REQUIRE_NODE=1` is load-bearing and not decoration: it is
what makes `the_delivered_signature_verifies_with_the_shipping_node_sdk` fail
rather than skip on a machine without `node`, so a green run means the Node
verifier a merchant installs actually accepted a header this server emitted.
The suite grew by 59 over Step 4's 806 and by one binary (`vpay-tests-integration`
gained `webhooks.rs`, which holds **13** of the integration suite's **88**);
`verify-ignored`'s three pins were bumped in the same commit that earned them. `just demo` was run **by the wiring pass, not for this note**, and its
**seventh step passed**: a signed `payment_intent.succeeded` read out of the
WireMock receiver's own request journal, `Stripe-Signature` byte-identical to
`Vpay-Signature`, and the recorded bytes verified with
`vpay_sdk::webhooks::verify`. That is cited on that pass's authority, on the
same footing as the workspace run.

**What the security remediation changed, and what was re-measured after it.**
It added migration `0023` (the `scan:deliveries` backstop), the boot-time
endpoint bounds and the livemode secret floor, fan-out page isolation, and tests
for each. `just verify` and `just verify-ignored` above **were re-run on the
remediated tree** and are current: 865 listed, 38 binaries, 0 ignored. The
`855 run / 855 passed` workspace execution was **not** re-run — it is the
wiring pass's, ten tests behind the listing, and is cited as such rather than
restated as if it covered them. `just ci` is what would close that gap.

**What this note does *not* claim.** The workspace run above is **the wiring
pass's run, cited rather than re-measured** — this documentation pass re-ran
`just verify` and `just verify-ignored` only, and states the workspace numbers
on that pass's authority. `cargo fmt`, `cargo clippy`, `cargo deny`,
`pnpm -r typecheck`, `pnpm -r test` and `cargo doc` were **not** re-run for this
note. **No CI run exists for this branch.** No merchant endpoint has ever been
POSTed to and no real rail has ever been called: every delivery and every
settlement observed anywhere in this repository went to a WireMock host on a
compose network.

**What this pass adds is the outbox drain, signing and delivery — the last
functional phase.** `vpay_worker::webhooks` turns the `events` backlog Step 4
writes into `webhook_deliveries` rows (migration `0022`) and `deliver_webhook`
jobs, then renders, signs and POSTs each one; `GET /v1/events` and
`GET /v1/events/{id}` are mounted as the merchant's fallback, rendering through
the *same* `EventObject` the deliverer signs. **A third sentence from an earlier
note is hereby retired: "webhooks are not implemented".** What replaces it is
narrower and is the reason the Webhooks row below is 🟡 and not ✅: the receiver
is a host in configuration, exactly as the rails are; there was **no runtime SSRF
filtering** at the time of that pass (boot-time `validate_webhook_url` only) —
closed in Step 8 by `vpay_worker::ssrf`, see this file's own row;
replaying an `exhausted` delivery is a hand-written transaction against `psql`,
now written down in
[runbooks/webhook-delivery-failures.md](runbooks/webhook-delivery-failures.md);
and the ladder's 1 h, 6 h and 24 h rungs are asserted as values by a unit test
and have never elapsed in a running deployment. The "Step 4" pass's own note
follows, with the one Step 5 sentence it had acquired moved up into this one:

Last verified: 2026-09-03, on branch `claude/step4-worker` (the "Step 4"
worker pass) at commit `905597c`. **Everything in this paragraph was measured
directly for this note, on the authoring machine, with the toolchain pinned to
`1.95.0` and `DOCKER_HOST=unix:///run/user/1000/docker.sock`.** `just verify`:
ok — `verify-no-mocks` clean, `verify-status` `1 unimplemented item(s), all
declared in docs/status.md and all still in shipping code`, `verify-errors`
`12 error type(s), all classified; anyhow confined to binaries`. `just
verify-ignored`: `0 ignored (expected 0), 37 test binaries (expected 37), 806
total (minimum 790)`. **`cargo nextest run --workspace`, the whole suite
including every container-backed one: 806 tests run, 806 passed, 0 skipped**,
in the run cited at the end of that note, against real `postgres:16-alpine` and
`wiremock/wiremock` containers. That was the first run in this repository's
history in which nothing was skipped, listed-not-run, or reported from
elsewhere. *(The implementation pass's own gate run reported 796 and the docs
pass 797; the second and third remediations added nine tests — a late success
past the horizon, a resubmit past the horizon, a poisoned job past the horizon,
idempotent re-escalation, a decline past the horizon, real-interval horizon
unit tests, and the `redirect_url` merge.)* That note also recorded that
`cargo fmt`, `cargo clippy`, `cargo deny`, `pnpm -r typecheck`, `pnpm -r test`
and `cargo doc` were **not** re-run for it, and that `just demo` was not run
either — so the demo row's sixth step was claimed on the strength of the code
and of `worker_e2e.rs`, not of an observed demo.

**What that pass added was the worker, and with it the first `succeeded`
payment.** `vpay-worker-bin` no longer logs a heartbeat saying the loop is not
implemented: it boots, reconciles configuration, reaps stranded leases, seeds
its housekeeping jobs and runs a real claim/settle loop over a `jobs` table
(migration `0021`). A confirmed intent is now driven to `succeeded` — charge,
intent, `amount_received` and one `payment_intent.succeeded` event in a single
transaction — or to `failed`/`requires_payment_method`, or to `unresolved` with
an alert after 24 hours. The recovery table in
[flows/crash-safety.md](flows/crash-safety.md) is executed rather than merely
written down. **Two sentences from the Step 3 note are hereby retired:
"nothing polls a `submitted` charge" and "`succeeded` has still never
happened".** What replaces them is narrower: every settlement so far was a
WireMock host answering, and the crash tests write the state a crash leaves
rather than killing a process. *(**Narrowed 2026-09-04 (Step 8, lane D):** that
second clause is now true of **kill point 1 only**.
`backends/tests/integration/tests/worker_kill9.rs` `SIGKILL`s the shipping
`vpay-worker-bin` mid-status-query and the shipping `vpay-server`
mid-`requesttopay`, so two of the three kill points are now *caused* rather
than written — see the "Real `SIGKILL` crash test" row below.)*
**Step 5 (2026-09-03) retires the third:
webhooks are delivered.** `just demo` now ends with a signed
`payment_intent.succeeded` POSTed to a WireMock *receiver* and verified with
the shipping SDK — which means the receiver, like the rails, is a host in
configuration and not a merchant. The rows below say which is which. The "Step 3" pass's own note follows, unchanged:

Last verified: 2026-09-03, on branch `claude/step3-rails` (the "Step 3" rails
pass) against `master` at `036b30c` — the merge of #16, which is Step 2's
`06a4280` on top of `0ac2a7f`. **Everything in this paragraph was measured
directly for this note, on the authoring machine, with the toolchain pinned
to `1.95.0` and `DOCKER_HOST=unix:///run/user/1000/docker.sock`.** `just
verify`: ok — `verify-no-mocks` clean, `verify-status` `1 unimplemented
item(s), all declared in docs/status.md and all still in shipping code`,
`verify-errors` `12 error type(s), all classified; anyhow confined to
binaries`. `just verify-ignored`: `0 ignored (expected 0), 35 test binaries
(expected 35), 718 total (minimum 640)`. `cargo nextest run -p vpay-provider
-p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-core`:
**156 tests run, 156 passed, 0 skipped** (11 `vpay-provider`, 48
`vpay-adapter-mtn-momo`, 53 `vpay-adapter-orange-money`, 44 `vpay-core`),
none of which needs Docker. `cargo nextest run -p vpay-config`: **70 run, 70
passed, 0 skipped**.

**The container-backed runs, which are what every 🟡 in the rail and confirm
rows below rests on.** `cargo nextest run -p vpay-tests-integration -p
vpay-db -p vpay-tests-conformance`: **115 tests run, 115 passed, 0 skipped**,
in 199 s, against real `postgres:16-alpine` and `wiremock/wiremock`
containers — 26 `vpay-tests-conformance` (4 capability cases plus 11 port
cases parameterised over both rails, each against a container started by
`vpay_testkit::containers::start_wiremock`), 37 `vpay-db`, and 52
`vpay-tests-integration`, of which **7** are the new `confirm_rails` suite
and 17 `payment_intents`.

`cargo nextest run -p vpay-api`: **165 run, 165 passed, 0 skipped**.

*Re-measured on this branch for Step 5b, 2026-09-03:* `cargo nextest run -p
vpay-api` is now **172 run, 172 passed, 0 skipped** — seven more than the
Step-3 figure above, added by the `request-id` mirror, the
`stripe-should-retry` derivation, the refused money-moving parameters and the
replayed-advisory gap. The other Rust counts in this paragraph were **not**
re-measured for Step 5b and stand as the Step-3 note left them.

**What this note does *not* claim.** It measured 506 of the 718 listed tests.
`vpay-server`'s and `vpay-worker-bin`'s subprocess CLI suites, the Rust
SDK's, `xtask`, `vpay-worker`, `vpay-testkit` and `vpay-ledger` were
**listed, not run** for this note; the counts for them below are reported
from the pass's own gate run, not re-measured here. `cargo fmt`, `cargo
clippy`, `cargo deny`, `pnpm -r test` and `cargo doc` were likewise not
re-run here — `just ci` on this branch is the thing that would refute the
previous pass's results for them. **No CI run exists for this branch**, and
`just demo` was not run for this note; the demo row below says what was and
was not observed there.

**What this pass adds is the rails — and the thing to be clear about is what
"the rails" means here.** Both adapters make real HTTP calls: MTN's
`requesttopay` and status query, Orange's `webpayment` and
`transactionstatus`, each with a token cache, a documented failure mapping
and a callback parser. `POST /v1/payment_intents/{id}/confirm` now reaches
one of them over the network and moves the intent — to `processing` on a
push rail, to `requires_action` with a `next_action.redirect_to_url` on a
redirect rail, or back to `requires_payment_method` with a
`last_payment_error` and a `409` when the rail declines. **Every one of
those observations is against a WireMock host. Neither MTN's nor Orange's
real sandbox has ever been called by this code**, so what is proven is that
the adapters speak the protocol these documents describe — not that the
documents are right about the rails. Nothing polls a `submitted` charge, so
a confirmed intent stops at `processing`/`requires_action` forever and
**`succeeded` has still never happened**. The rows below say which is which.
The "Step 2" pass's own note follows, unchanged: on branch
`claude/step2-payment-intents` (the
"Step 2" payment-intents pass) against `master` at `0ac2a7f`. **Everything in
this paragraph was measured directly for this note, on the authoring machine,
with the toolchain pinned to `1.95.0`; nothing in it is reported from
elsewhere.** `just verify`: ok — `verify-no-mocks` clean, `verify-status`
`8 unimplemented item(s), all declared in docs/status.md`, `verify-errors`
`11 error type(s), all classified; anyhow confined to binaries`. **A scanner
blind spot found on this pass, stated rather than hidden:** for a while the
count read `9`, because `scan_not_implemented` in `.xtask/src/main.rs` does
not distinguish a doc comment from code and a doc comment in
`backends/crates/vpay-api/src/v1/boot.rs` spelled out `NotImplemented("…")`
while explaining why a different error is used there — and the check still
passed only because this page's own prose happens to contain an ellipsis.
The comment was reworded (the eight adapter tokens are unchanged — this pass
added none and removed none); making the scanner comment-aware, and making it
refuse a "declaration" that is not in the token list, is a follow-up.
`just verify-ignored`: `3 ignored (expected 3), 34 test
binaries (expected 34), 546 total (minimum 500)` at 01:20 UTC+2 and `548
total` at 01:33 — **and that drift is worth stating rather than smoothing
over.** This note was written while a concurrent remediation pass was still
editing `vpay-db::idempotency`, migration `0015` and the two
`payment_intents` suites on the same branch (it is adding a `claim_id`
column so an expired-then-reclaimed idempotency row cannot be overwritten by
the stale claim — an ABA fix), so every count in this paragraph is a
measurement of a moving tree. **The number to trust is whatever `just
verify-ignored` and `just ci` print on the commit**, not the ones written
here; they are recorded so a later reader can tell whether anything
*shrank*. `expected_suites` moved 33 → 34 for the new `payment_intents`
integration binary and `min_tests` 320 → 500 in the same change. `cargo nextest run -p vpay-api -p vpay-core -p
vpay-config`: **258 tests run, 258 passed, 0 skipped** (160 `vpay-api`, 41
`vpay-core`, 57 `vpay-config`), none of which needs Docker.

**And, for the first time in this repository's history, the container-backed
payment tests were observed passing on the authoring machine.** With
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `cargo nextest run -p
vpay-db -p vpay-tests-integration`: **74 tests run, 74 passed, 0 failed, 0
skipped**, in 125 s, against real `postgres:16-alpine` containers — measured
at 01:20, i.e. *before* the concurrent ABA fix described above; that fix
touches exactly these two packages, so this run must be repeated on the
commit. That is 32
in `vpay-db` (24 of them in `tests/repositories.rs`) and 42 in
`vpay-tests-integration` — 16 `payment_intents` (new this pass), 14
`postgres_smoke`, 7 `merchant_token_flow`, 3 `authkestra_op_smoke`, 2
`client_store`. Every 🟡 and ✅ in the payment-intent, idempotency and
reconciliation rows below rests on that run.

**What this note does *not* claim.** It measured 332 of the 546 listed tests.
The other 214 — `vpay-server`'s and `vpay-worker-bin`'s subprocess CLI
suites, the Rust SDK's 107, `xtask`, `vpay-worker`, `vpay-provider`,
`vpay-ledger`, and the adapter-conformance suite that holds all 3 `#[ignore]`s
— were **listed, not run** for this note. `cargo fmt`, `cargo clippy`,
`cargo deny`, `pnpm -r test` and `cargo doc` were likewise not re-run here;
the previous pass's results for them stand and `just ci` on this branch is
the thing that would refute them. **No CI run exists for this branch**, and
`just demo` was not run for this note either — the demo row below says what
was and was not observed there.

**What this pass adds is the first `/v1` business resource, and it stops at
the rail on purpose.** `POST/GET /v1/payment_intents`,
`GET /v1/payment_intents/{id}`, `POST …/confirm` and `POST …/cancel` are
served; create, retrieve, list and cancel return real objects from real rows,
and **`confirm` reaches the rail adapter and answers `501 not_implemented`**,
because no adapter implements `submit`. Nothing here has taken a payment,
nothing has called a rail, and no payment intent has ever reached
`processing`, `requires_action` or `succeeded`. The rows below say which is
which. The "Step 1" pass's own note follows, unchanged: on branch
`claude/step1-merchant-tokens` (the
"Step 1" merchant-token pass) against `master` at `8ace988`. **For the first
time, every suite ran on the authoring machine**: the rootless Docker daemon
that could not start containers was repaired mid-pass (a stale in-daemon
containerd talking to a newer shim; restarted with the other projects'
containers restored), so `cargo nextest run --workspace` here means the
container-backed suites too. With the toolchain pinned to `1.95.0`: `cargo
fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `just verify` (`verify-no-mocks` clean, `verify-status` `8
unimplemented item(s)`, `verify-errors` `11 error type(s), all classified` —
`SigningKeyError` and `HttpClientError` are new), `just verify-ignored` (`3
ignored, 33 test binaries, 421 total`), `cargo deny check` (`advisories ok,
bans ok, licenses ok, sources ok`), `pnpm -r typecheck`, `pnpm -r test` (136
passed), `RUSTDOCFLAGS="-D warnings" cargo doc` for `vpay-api`, `vpay-core`,
`vpay-worker`, `vpay-sdk`, `vpay-config` and `vpay-db` all clean. `cargo
nextest run --workspace` with `DOCKER_HOST` set: **418 passed, 0 failed, 3
skipped** (the three `#[ignore]`d conformance cases), run four times in a
row after every container-starting package was moved into the serialised
nextest group and `vpay_testkit::containers::start_postgres_with_retry`
landed — the earlier runs had lost one to three random tests per run to
rootlesskit's `bind: address already in use` on a host running ~24
unrelated containers. `just demo` ran green end to end against the
containerised stack (`compose.yml` + `compose.e2e.yml` + `compose.demo.yml`):
discovery and JWKS, a token whose claims are `iss={base}/v1/oauth`,
`aud=vpay:v1`, `sub=demo-merchant`, `exp=+900 s`, the 401 envelope without a
bearer, and the authenticated 404 `unknown_route`. The CI run for this
branch is the other half of the evidence and is named in the pull request.
**Everything else in this paragraph was measured directly for this note,
not reported from elsewhere.** `just verify`: ok — `verify-no-mocks` clean,
`verify-status` `8 unimplemented item(s)` (unchanged; this pass added no
`NotImplemented` token and removed none), `verify-errors` `10 error type(s),
all classified` (up from 9 — the new one is `vpay_api::op::keys::SigningKeyError`).
`just verify-ignored`: `3 ignored (expected 3), 32 test binaries (expected
32), 396 total (minimum 320)` — `expected_suites` moved 30 → 32 in this
pass, for the two new `vpay-tests-integration` binaries `client_store` and
`merchant_token_flow`. `cargo nextest run -p vpay-api -p vpay-config -p
xtask`: **161 tests run, 161 passed, 0 skipped** (74 `vpay-api`, 53 `vpay-config`, 34 `xtask`) — that is the whole
of this pass's unit coverage, and none of it needs Docker. `cargo nextest
list -p vpay-tests-integration -p vpay-db` enumerates **36 container-backed
tests** (25 in `vpay-tests-integration`, of which 6 are the new
`merchant_token_flow` suite and 2 the new `client_store` suite; 11 in
`vpay-db`, of which 5 are the new `ensure_active_signing_key` cases) —
**listed, not run**: the testcontainers bootstrap still cannot start a
container on this machine (same rootless-daemon fault the Step 0 note below
describes), so this note claims none of the 36 as passing.

**The evidence behind the merchant token flow, stated exactly, because every
🟡 in the merchant rows below rests on it.** The implementer ran the six
`merchant_token_flow` tests against a scratch database on an
already-running Postgres, bypassing the testcontainers bootstrap this
machine cannot run; **all six passed**. **The Docker-backed form of those
tests has not yet run in CI. The two `client_store` tests and the five new
`vpay-db` signing-key tests have not been run anywhere at all** — not here,
not in CI. So: the code is written and the tests that would fail if it broke
exist, but the only observation of the whole handshake working end to end is
one manual run, on one machine, outside the harness CI will use.

**What this pass adds is the merchant half of Phase 2 — a token a merchant
can actually obtain, and an authentication boundary in front of `/v1`:** the
merchant OP at `/v1/oauth` (token, discovery, JWKS), RS256 signing-key
generation/loading/activation, the `disabled_clients` kill switch enforced
on the one interception point every token request passes through, replay
protection wired into the live store, and every `/v1` path other than
`/v1/oauth` nested behind `AuthenticatedMerchant`. **It adds no `/v1`
business resource** — an authenticated `/v1` request gets the honest 404,
deliberately — **and no `/dash/v1` anything**: there is still no login, no
session store, and no dashboard route. The rows say which is which. The
"Step 0" operability pass's own note follows, unchanged: on branch
`claude/vpay-production-readiness-56b122` (the "Step 0" operability pass)
against `master` at `03d34cc`. On the authoring machine, with the toolchain
now pinned to `1.95.0`: `cargo fmt --all -- --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `just verify` (`verify-no-mocks`,
`verify-status` — `8 unimplemented item(s)` — and `verify-errors` — `9
error type(s)`), the new `just verify-ignored` (`3 ignored, 30 test binaries, 332 total`,
proven to fail at `expected_ignored=4`, `expected_suites=31` and `min_tests=400`), `cargo deny
check`, and `RUSTDOCFLAGS="-D warnings" cargo doc -p vpay-api` all clean.
`cargo nextest run` for `vpay-config` (48 passed, the one that loads the
real `config/application.yml`), `vpay-api` + `vpay-core` (64 passed) and
the two binaries (15 passed; **9 failed for environmental reasons** — every
one at the testcontainers `start_postgres` call, before a vpay binary is
spawned, because this machine's rootless Docker daemon cannot start a
container: `failed to start shim … unsupported protocol`; none was marked
`#[ignore]`). **The container-backed suites are therefore not claimed to
pass here.** The CI `rust` job on `ubuntu-latest` is the evidence for them
— it runs the whole workspace with a working daemon and last reported `320
passed, 3 skipped` on run `33626567174` — and the `e2e (compose)` job is
now the evidence for the images and the stack; see "GitHub Actions" under
Infrastructure for what this pass changed there and what is still pending
a green run. **What this pass adds is operability plumbing, not features:**
a CI workflow that can actually go green, a compose stack and image that
can actually boot, the rustls provider install both binaries were missing,
request ids on every API response, the pinned toolchain, and the
`verify-ignored` coverage guard — no route, no rail call, no job loop. The
error-modelling pass's own note follows, unchanged: on branch
`claude/error-modelling` (the
error-modelling pass, [ADR-0011](adr/0011-error-modelling.md)) rebased on
`master` at `985bd96` (the merge of the SDK/authkestra pass below).
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `just verify` (`verify-no-mocks`, `verify-status`, and the new
`verify-errors` — `9 error type(s), all classified`), `cargo deny check`,
`pnpm -r typecheck`, and `RUSTDOCFLAGS="-D warnings" cargo doc` for
`vpay-core`/`vpay-api`/`vpay-worker`/`vpay-sdk` all clean. `cargo nextest
run` over every crate that needs no container (`vpay-core`, `vpay-api`,
`vpay-worker`, `vpay-provider`, `vpay-ledger`, `vpay-config`, `xtask`,
`vpay-sdk`): **262 passed, 0 skipped**; the six new exit-code CLI tests in
the two binaries pass without Docker. The container-backed suites still
cannot run on the authoring machine (same daemon fault as the previous
pass, described below) and are not claimed to pass here — the previous
pass's CI run on `master` is the last evidence for them, and this pass
changed no code in `vpay-db` or the migrations. **What this pass adds is
the error model: the `Classify` seam and policy table in `vpay-core`, a
`Classify` impl on every leaf error, the `ApiError` and `JobError`
composites, exit codes from `Category` in both binaries, and the
`verify-errors` self-check — see the six "Error …" rows in the Backend
table and [docs/flows/errors.md](flows/errors.md); none of it serves a
request or moves money, and the rows say so.** The SDK/authkestra pass's
own note follows, unchanged: on branch `claude/sdk-rust-nodejs-0c1ecf`
against `8c0760e`, `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo xtask verify-no-mocks`, `cargo xtask
verify-status`, `cargo deny check` (`advisories ok, bans ok, licenses ok,
sources ok`), `pnpm -r typecheck`, `pnpm -r test` (136 passed) and
`RUSTDOCFLAGS="-D warnings" cargo doc -p vpay-sdk` all clean. `cargo nextest run --workspace`:
**248 tests, 214 passed, 34 failed, 3 skipped** — and the 34 failures must be
read exactly: every one is in a suite that starts a `postgres:16-alpine`
testcontainer (`vpay-db`, `vpay-server::cli`, `vpay-worker-bin::cli`,
`vpay-tests-integration`), and every one failed before its first assertion
with the Docker daemon on the authoring machine refusing to start *any*
container ("failed to start shim … unsupported protocol: Yunix", a
containerd fault; 24 unrelated containers were running, so the daemon was
not restarted). None of those 34 tests could be run here; none is claimed
to pass. The 3 skipped are the pre-existing `#[ignore]`d adapter-conformance
cases. What landed this pass: (1) **`authkestra-*` 0.5.4 → 0.7.1** (latest on
crates.io that day), with the DDL re-diff the OP-tables row demanded done for
real and its additive delta transcribed as migration `0013` — **proven
against a real Postgres despite the daemon fault**: a throwaway harness
(outside the repo) applied migrations 0001–0013 to a fresh database on an
already-running Postgres 18 container and drove the real 0.7.1
`SqlxOpStore<Postgres>` through `find_client` (decoding the two new
columns), `store_code`/`consume_code`, `store_token`/`get_token` (`jkt`),
and `check_and_record_dpop_jti`, all passing, plus a negative control on a
second database migrated only to 0012 where the same store's `store_token`
and DPoP writes fail as expected — the repo's own three
`authkestra_op_smoke.rs` tests encode exactly those checks and remain unrun
here for the daemon reason above; (2) **CrateStack re-verified at 0.10.1**
(`schema OK`); (3) two `cargo deny` advisory regressions on `master` fixed by
upgrade (`h2`, `chacha20`); (4) **two merchant SDKs** and the wire contract
they implement — see the new "Merchant SDKs" section and
`docs/flows/merchant-auth.md`; the Rust SDK adds 107 tests to the workspace
count (248 = 141 pre-existing + 107). The previous pass's own note is
unchanged below, describing the state before this one:
`cargo nextest run --workspace` (139 passed, 3 skipped), `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo xtask verify-status`, `just verify`, and
`cargo deny check` (`advisories ok, bans ok, licenses ok, sources ok`), all
run against the working tree of three things landing together this pass,
labelled OP-1/OP-2/OP-3 below: (1) **OP-1** — `vpay_config::oauth` now models
both kinds of OAuth2 client ADR-0010/`docs/flows/dashboard-auth.md` need
(`MerchantClient`, `DashboardClient`), with seven boot-time validation rules,
and — the actually load-bearing change — **`Config::load` is now called by
both binaries**, ordered before the database connection, with `--config` /
`VPAY_CONFIG` treated as required at the binary level and proven so by
subprocess tests (`a_missing_config_is_exit_78_naming_the_problem`,
`a_bad_config_is_exit_78_naming_the_problem`,
`a_valid_config_lets_the_server_boot_and_serve_healthz` /
`a_valid_config_lets_the_worker_boot`, in each binary's `tests/cli.rs`); (2)
**OP-2** — a new repository layer in `vpay-db`
(`SqlClientAssertionStore`, `is_client_disabled`/`disable_client`/
`enable_client`, `publishable_signing_keys`/`active_signing_key_kid`/
`rotate_signing_key`), each tested against a real Postgres, the replay store
additionally proven race-safe by a 10-way concurrent test; (3) **OP-3** — a
`JwtValidator`/`AuthenticatedMerchant`/`AuthenticatedDashboard` extractor pair
in `vpay-api::resource_auth`, validating against a cached JWKS, with a real
`jsonwebtoken` audience-validation gap found and closed
(`set_required_spec_claims(&["exp","aud","iss"])` — see that row below).
**None of this makes login or merchant authentication work.** The router is
still `/healthz` plus the Stripe-shaped 404: no `/v1/*` route, no `/dash/v1/*`
route, no OP endpoints (`/authorize`, `/token`, `/jwks.json`, discovery), no
`ClientStore` converting a configured client into
`authkestra_op::client::ClientRegistration`, and no shipping binary ever
constructs `SqlClientAssertionStore`, calls the kill-switch functions, or
calls `rotate_signing_key` — the stores above are proven correct in
isolation, not proven wired into anything that serves traffic. No signing key
has ever been generated; `rotate_signing_key` rotates *to* a key its caller
already has, it does not create one. See the rows below for exactly what each
piece proves and does not prove, and the "Resource-server JWT validation" and
"rustls `CryptoProvider` process default" rows in particular for a landmine
this pass surfaced but did not fix: `authkestra_resource::jwt::Jwks::fetch`
panics without a process-wide default TLS crypto provider installed, and
nothing in a shipping binary installs one. **A dependency-graph fact the
previous pass's own note got wrong going forward, caught while verifying this
pass, not claimed by whoever wrote OP-1/OP-2/OP-3:** the "`cargo deny`"
infrastructure row below used to say the `rsa` advisory's only path was
`vpay-tests-integration`'s dev-dependencies, "no shipping binary pulls it
in." That stopped being true the moment `vpay-db` added `authkestra-op` as a
*production* dependency for OP-2 — `cargo tree -i rsa` now shows
`rsa → authkestra-engine → authkestra-op → vpay-db → vpay-server` /
`vpay-worker-bin` with no `(dev)` marker anywhere on that path. `cargo deny
check` still exits 0 (an `ignore`d advisory is a note, not an error), so nothing
here is a CI regression, but the row's own narrowing of the exposure to
"dev-only" is now false and is corrected below. The Rust count moved from 105
passed / 3 skipped to **139 passed / 3 skipped in this pass**: 34 new tests,
counted directly against `932d8a4` (the commit `docs/status.md` last verified
against): `vpay-config` gains 12 (5 in the new `oauth.rs`, 7 new
OAuth-client validation-fixture tests in `config.rs`), `vpay-db` gains 5 (all
of `tests/repositories.rs`, listed above), `vpay-api` gains 11 (the entire
new `resource_auth.rs` test module: signature/expiry/audience/issuer/`kid`
coverage plus the extractor-and-error-envelope tests), and both binaries'
`tests/cli.rs` gain 3 apiece (6 total) proving the config-required-at-startup
behaviour end to end. 12 + 5 + 11 + 6 = 34. The previous pass's own note is
unchanged below, describing the state before
this one: the Rust count moved from 80 passed / 3 skipped to **105 passed / 3
config loader (`vpay_config::Config::load`, Figment + hand-rolled `${ENV}`
resolution + the existing guard rules), a library with tests but **not wired
into either binary**; (2) a new `vpay-db` crate — `connect`, `run_migrations`,
`check_connection` — that both binaries now require at boot, with `/healthz`
performing a real `SELECT 1`; (3) migrations `0009`-`0012`, a schema cutover
that drops `merchant_api_keys`, reshapes `oauth_signing_keys` to hold no
private key material, and adds `oauth_client_assertion_jtis` and
`disabled_clients`; (4) [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md),
moving `/v1` merchant auth from Stripe-shaped API keys to `client_credentials`
+ `private_key_jwt`, with README/API docs/examples corrected to say a Stripe
SDK cannot authenticate against vpay *(that correction was itself too strong,
and was retracted on 2026-09-03 — see the "Stripe-SDK compatibility **on
`/v1`**" row in the Backend section, which is the one that retracts it; the
Merchant SDKs section carries a second, differently-scoped row also called
"Stripe-SDK compatibility", covering the client half. And ADR-0010's
amendment)*. **None of this makes authentication
work.** The router is still `/healthz` plus the Stripe-shaped 404: no `/v1/*`
route, no `/dash/v1` route, no client store, no `ClientAssertionStore`, no
kill-switch check, no signing-key generation or rotation exist anywhere in
this repository. The schema and the config loader are real and tested; the
auth is not built. See the rows below for exactly what each new piece proves
and does not prove. The Rust count moved from 80 passed / 3 skipped to **105
passed / 3 skipped in this pass**: 31 new tests, split roughly across
`vpay-config` (5 → 36, covering `Config::load`'s YAML layering, `${ENV}`
resolution, and validation), `vpay-db` (a new crate: pool construction,
migration idempotency, and both the success and failure path of the
healthcheck query, each against a real `postgres:16-alpine` via
testcontainers), `vpay-api` (router wiring), and
`backends/tests/integration/tests/postgres_smoke.rs` (six new tests proving
migrations `0009`-`0012`'s constraints — see the schema rows below). The
previous pass's own note is unchanged below, describing the state before this
one: the Rust count moved from 78 passed / 3 skipped to **80 passed / 3
skipped in this pass**: two new regression tests,
one per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`
in `backends/apps/vpay-server/tests/cli.rs` and
`backends/apps/vpay-worker-bin/tests/cli.rs`), covering a real startup race
where a SIGTERM delivered immediately after process start bypassed graceful
shutdown entirely — see the "Process lifecycle" row for the mechanism, the
fix, and this pass's own honest accounting of the regression test's
statistical (not deterministic) nature and its reduced sensitivity when run
as part of the full workspace suite versus scoped/alone. Both new tests were
verified to fail against the pre-fix code before this pass landed. The prior
pass's own note is unchanged below: the Rust count moved from 64 passed / 5
skipped to 71 passed / 3 skipped (database schema and migrations 0001–0005),
then to 78 passed / 3 skipped (three more migrations —
`0006_create-authkestra-op-tables.sql`, `0007_create-oauth-signing-keys.sql`,
`0008_create-merchant-api-keys.sql` — see the "Authkestra OP tables", "OAuth
signing keys" and "Merchant API keys" rows below), adding six new
per-constraint tests to `backends/tests/integration/tests/postgres_smoke.rs`
plus one new test file, `backends/tests/integration/tests/authkestra_op_smoke.rs`,
whose single test,
`sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes`, drives the
real `authkestra_op::sqlx_store::SqlxOpStore<Postgres>` against migration
0006: it inserts a client, calls `find_client` (proving the JSONB columns
decode through the store's own type), `store_code`, then `consume_code`
twice and asserts the second call returns `None` — the store's single-use
enforcement actually firing against this schema, not merely SQL that parses.
The remaining 3 skipped are unrelated: the adapter conformance suite's
`#[ignore]`d cases in `backends/tests/conformance/tests/adapter_conformance.rs`,
gated on rail wire calls that are still `ProviderError::NotImplemented`.

**Re-verified 2026-09-03 for Step 5c (block C), on top of Steps 5b and the
security-hardening commit that landed on this branch while this pass was
running.** Everything below was actually run on this machine, in this
order, not inferred: `just verify` — ok (`verify-no-mocks` clean,
`verify-status` `1 unimplemented item(s)`, `verify-errors` `12 error
type(s), all classified`); `just verify-ignored` — **`0 ignored (expected
0), 39 test binaries (expected 39), 921 total (minimum 900)`**;
`VPAY_REQUIRE_NODE=1 cargo nextest run --workspace` — **`921 tests run: 921
passed, 0 skipped`**, against real `postgres:16-alpine` and
`wiremock/wiremock` containers, `DOCKER_HOST=unix:///run/user/1000/docker.sock`
— all 9 of `backends/tests/integration/tests/browser_checkout.rs`'s tests
passed as part of that run, not in isolation. `pnpm -r test`: **245 tests,
all passing** (breakdown in "Frontend" below). `just docs-check` and `just
lint-web`: both clean. `just demo_port=18084 demo`: all 7 steps passed; the
same stack then drove `examples/checkout-browser` by hand through a real
browser session to `succeeded`, and `pnpm --filter @vpay/e2e e2e` against it
passed both specs, 4/4 (`checkout.cy.ts` new, `dashboard.cy.ts` unchanged).
**Not re-run for this note:** `cargo fmt --check`, `cargo clippy`, `cargo
deny`, `cargo doc`. **No CI run of this exact tree exists yet** — the `e2e`
job gained the two steps and three env vars `checkout.cy.ts` needs
(`.github/workflows/ci.yml`), but nothing has confirmed they work inside an
actual GitHub Actions runner, only on the authoring machine.

---

## Legend

| Marker | Meaning |
|---|---|
| ✅ **Done** | Implemented, tested, and the tests actually assert behaviour |
| 🟡 **Partial** | Some of it is real. The rest is listed explicitly below |
| ⛔ **Not started** | No implementation. Calls return `NotImplemented`; tests are `#[ignore]`d with a reason |

Nothing in this repo is ✅ unless a test would fail if it broke.

---

## Overall

> **vpay is a scaffold.** It compiles, lints clean, and its tests pass — but it
> cannot take a payment. Do not deploy it.
>
> *2026-09-03 (Step 2): `/v1/payment_intents` now exists and answers real
> requests with real rows.*
>
> *2026-09-03 (Step 3): the sentence "no HTTP call to any rail has ever been
> made by this code" is **retired** — both adapters now make real HTTP calls,
> and `confirm` moves an intent to `processing` or `requires_action` on the
> strength of one. **Its replacement is narrower and just as load-bearing:
> no HTTP call to a real rail has ever been made.** Every call has gone to a
> `wiremock/wiremock` host. No payer has been prompted, no money has moved,
> nothing polls a submitted charge, and no intent has ever reached
> `succeeded`. Do not deploy it.*
>
> *2026-09-03 (Step 4): the worker runs, and an intent reaches `succeeded`
> without anyone touching it. **The load-bearing sentence is unchanged: no
> HTTP call to a real rail has ever been made.** Every payment this code has
> settled was settled against a `wiremock/wiremock` host that answered the way
> these documents say a rail answers. No payer has been prompted, no money has
> moved, and nothing has ever run outside a developer machine. Webhooks are
> delivered as of Step 5 — to a WireMock receiver on a compose network, never
> to a merchant. Do not deploy it.*
>
> *2026-09-03 (Step 5c): a payer's own browser can now confirm a push payment
> directly — `/v1/browser` plus `@vaam-apps/vpay-stripe-js`, with no merchant
> credential anywhere near the browser. **The load-bearing sentence is still
> unchanged: no HTTP call to a real rail has ever been made.** The MTN push
> `examples/checkout-browser` confirms goes to `wiremock-mtn`, the same as
> every other payment in this repository's history. The redirect (Orange)
> half of a browser checkout has no return-trip route and must not be
> shipped on this package yet — see [flows/browser-checkout.md](flows/browser-checkout.md).
> Do not deploy it.*
>
> *2026-09-03 (Step 7, Phase A): a cleanup pass, and it built nothing.
> `vpay-db` grew a repository seam and `ProviderError` grew a real
> `#[source]`; no feature moved from ⛔ or 🟡 to ✅ because of it, and **the
> load-bearing sentence is unchanged: no HTTP call to a real rail has ever
> been made.** The proof that nothing changed is the guard suites, which were
> not rewritten to fit: 26 conformance cases, the integration binaries and
> the SDK suites all still pass, and the only assertion lines that moved are
> the three the escalation named. Do not deploy it.*
>
> *2026-09-03 (Step 7, the four parallel lanes): the same sentence, once
> more, for the rest of the step. Step 7 is error surfaces, a repository
> seam, docs moved out of module headers into `docs/reference/`, doctests
> that make the examples real, and the tooling that runs them. **It moved no
> capability.** Not one row below changed from ⛔ or 🟡 to ✅ because of it,
> nothing new is reachable over HTTP, and no rail was called. What did change
> is what the repository checks about itself: the workspace's doctests run in
> CI for the first time, and `just verify` prints a doc-volume report it
> cannot fail on. If a row below looks better than it did before this step,
> that is a defect — say so.*
>
> *2026-09-04 (Step 8, the production gate): this step **did** move capability,
> and the banner still stands. What moved: a runtime egress guard on every
> webhook delivery (the last ⛔ on a shipping path); `POST
> /provider/{code}/callback`, so a rail that tries to tell us about a charge is
> heard; a real `SIGKILL` of the shipping worker and the shipping server, so
> two of the three crash-safety kill points are caused rather than written; a
> demo that walks six payments across both rails; a fix for a `500` on confirm
> that this step's own demo found. **The load-bearing sentence is unchanged:
> no HTTP call to a real rail has ever been made.** The guard has never refused
> a real merchant's endpoint, the callback route has never been called by MTN
> or Orange — its test bodies are transcribed from this repository's own flow
> documents, so a document that is wrong about a rail would pass — and every
> payment in the demo settled against a `wiremock/wiremock` host. No payer has
> been prompted and no money has moved. Do not deploy it.*
>
> *2026-09-04 (Step 9, hosted checkout): this step moved the most visible
> capability so far, and the banner is unchanged. What moved: vpay serves its
> own payment page, hosted and embedded — a `checkout.session` object, three
> new browser reads, `frame-ancestors` from a per-merchant origin list, the
> return trip a redirect rail needs, `initEmbeddedCheckout` in
> `@vaam-apps/vpay-stripe-js`, `checkout.sessions` in both merchant SDKs, a fourth image
> and a Helm workload, and a demo shop a human can buy from. **A real browser
> has walked the whole thing**: add to cart, checkout, vpay's page, an MTN
> prompt or Orange's own page, back to the shop, and `paid` written only by the
> shop's webhook handler after it verified vpay's signature. **The
> load-bearing sentence is unchanged: no HTTP call to a real rail has ever
> been made.** Every rail in that walk is a `wiremock/wiremock` host; no payer
> has been prompted on a real handset; no money has moved; and no merchant
> endpoint outside this repository has ever been POSTed to. Two more things
> this step is not: **no browser has been observed enforcing vpay's
> `frame-ancestors`** — Cypress strips the header, so it is proven sent and the
> refusal a browser was seen performing is the page's own origin check — and
> **no pod has ever run** the page. Do not deploy it.*

---

## Backend

| Area | Status | Notes |
|---|---|---|
| Workspace, edition 2024, resolver 3 | ✅ | `cargo check --workspace --all-targets` clean |
| Lint policy (no `unwrap`/`expect`/`panic`/float in prod) | ✅ | `cargo clippy -- -D warnings` clean; tests exempted via `clippy.toml` |
| Error classification seam (`vpay_core::error`, [ADR-0011](adr/0011-error-modelling.md), [docs/flows/errors.md](flows/errors.md)) | ✅ | `Category` (12 variants), `Retry`, `Severity`, the `Classify` trait and `find_in_chain`. The whole policy table — HTTP status, Stripe `type`, default `code`, retry, severity, public message, exit code — is one set of exhaustive `match`es on `Category`, pinned two ways: invariant tests over every category (caller categories are 4xx and system categories 5xx, only `Rail`/`Storage`/`RateLimited` retry after backoff, only `Internal` pages, every `type` is in Stripe's closed vocabulary, no generic message names anything internal, exit codes follow `sysexits`) **and** a literal transcription of [docs/flows/errors.md](flows/errors.md)'s twelve-row table, so the document and the code fail together; `Category::ALL` is proven complete by an exhaustive index function, so a thirteenth variant fails to compile there and fails the test if left out of `ALL`. 28 test functions in `vpay-core`. ✅ for what this row claims — the seam exists and its policy is proven — not for any request being answered through it (see the `ApiError` row) |
| Leaf errors classified (`MoneyError`, `UnknownCurrency`, `LedgerError`, `ConfigError`, `DbError`, `ProviderError`, `AuthRejection`) | ✅ | Each has an `impl Classify` next to its definition, with a comment per non-obvious choice (`DbError::Migrate` is `Configuration` not `Storage`; `ProviderError::Rejected` is `Conflict` with `Retry::NewAttempt`; `LedgerError::Unbalanced` pages). `ProviderError::Rejected` is the one that carries policy of its own: its envelope `code` is the constant `charge_declined` (an earlier draft reused the `FailureCode` string, which collided with `Transport`'s `provider_unavailable` at a different status), the `FailureCode` is in the public message, and its severity follows [docs/flows/failures.md](flows/failures.md)'s own table — `provider_account_blocked` pages, `provider_unavailable`/`provider_error` warn — proven exhaustively over all eleven codes. **Machine-checked**: `cargo xtask verify-errors` fails `just verify` and CI if any `pub` type under `backends/crates` that derives `thiserror::Error` **or** is named `*Error`/`*Rejection` lacks an `impl Classify` outside test code, or if a library crate lists `anyhow` under `[dependencies]` — proven live by deleting `UnknownCurrency`'s impl, by moving an impl into a `#[cfg(test)]` module, and by injecting `anyhow`, all three refused; it currently counts 9 types. Scope, stated plainly: `pub` types only, `backends/crates` only, `tests/` directories and `#[cfg(test)]` blocks excluded, line and block comments stripped; the SDKs and `backends/apps` are outside it by design. **Changed 2026-09-03 (Step 7, Phase A): `ProviderError::{Transport, Malformed}` are struct variants carrying a real `#[source]`.** They were `Transport(String)`/`Malformed(String)`, so both adapters `format!`ed `reqwest`'s error into the message — and `reqwest`'s `Display` for a timeout is "error sending request for url (…)", with the word *timeout* one link further down. Orange had noticed and hand-walked `Error::source()` into a `String`; that walk is deleted, and the two adapters now attach the error itself as `RailFailure::{Http, Body}` (a closed enum, not `Box<dyn Error>`, because ADR-0011 forbids the box). The chain is rendered once, at the boundary that logs it: `vpay_core::error::source_chain` (new; `ApiError::log` already did this privately and now shares it, and `vpay_worker`'s job settlement uses it so `jobs.last_error` keeps the leaf). `Display` still renders the adapter's own sentence, which is why the body-cap message still names `MAX_RAIL_BODY_BYTES`. Pinned by `a_transport_failures_source_chain_reaches_the_reqwest_error` (`vpay-adapter-mtn-momo`), which fails if anyone goes back to `format!` — verified by doing exactly that and watching it fail. `verify-errors` counts **13** types now (`RailFailure` classifies itself as `Category::Rail`) and additionally checks `#[from]` delegation. **No wire change**: the 26 conformance cases still pass; three of their assertions were adapted to the struct shape under Step 7's decision (14) without changing what they assert |
| `vpay_api::ApiError` (HTTP composite) | 🟡 | `#[from]` every leaf the HTTP layer can meet (`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`, `ConfigError`, `AuthRejection`) plus `UnknownRoute`/`InvalidParam`/`IdempotencyKeyReused`/`Internal`; axum's `Form`/`Json`/`Path`/`Query` rejections convert into `InvalidParam` with a curated sentence (a real `Form` extractor failing over a router yields the 400 envelope with `param: "body"`, never axum's plain text); no blanket `From<serde_json::Error>`, by design. `Classify` delegates all five methods to the leaf, pinned by a test asserting a wrapped leaf answers exactly as the bare leaf for `retry`/`severity`/`public_message` too. `IntoResponse` derives status, `type`, `code`, `message` and optional `param` from the classification and logs the full `Display` **and** source chain at the mapped `tracing` level (`alert = true` on `Page`) — a `DbError` carrying `host-secret-xyz` reaches the log and never the body (tested). `InvalidParam.param` must look like a field name (else `request`) and `message` is capped at 200 chars at render time; a 1 MB input yields a body under 1 KiB (tested). **The two envelope renderers are `pub(crate)`**, so a handler cannot build one by hand — one renderer is structural; `error_envelope` itself is now test-only (its pinned-shape test remains), and `IntoResponse` calls `error_envelope_with_param`. `AuthRejection` is classified and rendered through it; the 404 fallback is an `ApiError`; the pre-existing 404 and 401 envelope bytes are pinned unchanged. 29 test functions in `vpay-api`, 0 ignored. **Changed 2026-09-02: the 401 envelope is now reachable in a running `vpay-server`.** `AuthenticatedMerchant` is mounted in front of the `/v1` nest (see "HTTP surface"), so `ApiError::Auth` is produced by real traffic, not only by this module's own tests: `an_unauthenticated_v1_request_is_401_not_404` and `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope` (`lib.rs`) drive it over the real router, and `a_v1_request_with_no_bearer_token_is_the_401_envelope` (`backends/tests/integration/tests/merchant_token_flow.rs`) does it over a socket against a booted server. Two reachable envelopes now, then: the 401 and the 404. `the_404_fallback_is_byte_for_byte_what_it_was_before_api_error` had to move its URI off `/v1` to keep testing the fallback at all — a `/v1` path with no token is a 401 now and never reaches it — and the pinned bytes are unchanged, because the envelope never echoed the path. **Still 🟡, for what is left rather than for what was**: every other variant (`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`, `IdempotencyKeyReused`, `InvalidParam` from a body extractor) is still produced by no shipping handler, because no `/v1` business resource exists to produce one. `vpay-api` gained `vpay-config` and `vpay-ledger` as runtime dependencies for variants no handler can produce today. `vpay-config` was already in both binaries' graphs; `vpay-ledger` is a workspace crate that **neither binary linked before** and now both do (via `vpay-api` and `vpay-worker`). No third-party package is new to either binary, but `vpay-api`'s own graph now includes `clap`/`figment`/`garde`/`serde_yaml_ng`; `cargo deny check` still clean. **Changed 2026-09-03 (Step 2), and the "produced by no shipping handler" clause above is now false for most of them.** Four variants are new — `NotFound { resource, id }`, `Conflict { message }`, `Forbidden`, and `IdempotencyKeyInFlight { key_hint }` — and `/v1/payment_intents`'s handlers return them, along with `InvalidParam`, `Db`, `Provider`, `Currency` and `Money`, from a shipping request path. `IdempotencyKeyInFlight` is its own variant rather than a `Conflict` because the two are different advice ("your intent moved on" versus "your own earlier call is still running"), and it classifies as `Category::Idempotency` → `400` `idempotency_error`/`idempotency_key_in_flight`; the `409`-versus-`400` question is in the Idempotency row and is a maintainer decision. `NotFound` is what a *foreign* id answers as well as a missing one, byte for byte (`a_foreign_object_and_a_missing_object_are_byte_identical`), so the API cannot be used to discover which ids exist under another tenant — that is why `Forbidden` is reserved for a missing *scope*. `error.rs` now holds 20 tests, including `the_step_2_variants_say_what_they_should_and_no_more`, `every_variant_answers_with_the_classification_its_leaf_chose`, `every_variant_renders_that_classification_over_a_real_router`, `a_key_still_in_flight_is_a_different_code_from_a_key_reused_and_from_a_conflict`, `an_idempotency_key_is_never_echoed_past_its_hint` and `a_storage_errors_leaf_text_reaches_the_log_and_never_the_body`. **Also new: `vpay_api::form`'s `VpayForm`/`VpayQuery` replaced axum's `Form`/`Query` on `/v1`.** axum's own `FormRejection` renders plain text, which would have put a non-envelope body on the one surface whose error contract is the product; the replacements render the Stripe envelope and name the part of the request the rejection came from (`a_form_rejection_is_answered_with_the_envelope_not_axums_plain_text`, `every_extractor_rejection_names_the_part_of_the_request_it_came_from`, `a_json_body_is_told_to_send_a_form`, `a_missing_required_field_is_a_400_naming_the_body`). **Still 🟡:** `LedgerError` and `ProviderError::Rejected` are still produced by nothing, and `Category::Rail`'s `502` has never been produced by an actual rail, because no rail has ever been called |
| `vpay_worker::JobError` (job-loop composite) | ✅ | `Db`/`Provider`/`Money`/`Ledger` wrapped with all-five-method delegation, plus `Poisoned` (`Internal`) and `Exhausted` — the reconciler's `unresolved` state, which [docs/flows/reconciler.md](flows/reconciler.md) defines as "still polled, once an hour, and now raising an alert": `Rail`, `Retry::AfterBackoff`, severity `Error`, code `charge_unresolved`. `decision(attempt)` is a wildcard-free `match` on `Classify::retry` alone: `AfterBackoff → RetryAfter { delay, alert }` with `delay = poll_delay(attempt)` (or the documented hour, `UNRESOLVED_POLL_INTERVAL`, for `Exhausted`) and `alert = severity ≥ Error`; `NewAttempt → Terminal`; `Never → DeadLetter`. 12 test functions: a declined charge is `Terminal`, `NotImplemented` dead-letters, `Db::Connect` rides the ladder *and* alerts (Storage is severity `Error`), `Transport` rides it and wakes nobody, `Exhausted` retries hourly with `alert: true` at every attempt and is never a `DeadLetter`. **Changed 2026-09-03 (Step 4): was 🟡 "nothing calls `decision()`". It is now the only thing that ends a lease.** `vpay_worker::run_loop::settle` asks `decision(attempt_index(job))` and nothing else — no `match` on the error's variants anywhere in the loop — and maps its three answers onto exactly three writes: `RetryAfter` → `jobs::reschedule` (with `alert` carried onto the disposition log line), `Terminal` → `jobs::finish`, `DeadLetter` → `jobs::dead_letter` (`run_at = 'infinity'`, always `alert = true`, because a parked job is work nothing will ever do again). Proven end to end, not just at the unit level: `a_poisoned_job_is_parked_with_its_lease_cleared_and_its_reason_recorded` (a `Poisoned` row really is parked at `'infinity'` with its lease cleared and its reason in `last_error`) and `a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked` (an `Exhausted` job really is rescheduled ~1 h later and **not** parked), both in `backends/tests/integration/tests/worker_recovery.rs` against a real Postgres |
| Binary exit codes (`vpay-server`, `vpay-worker-bin`) | ✅ | `main` returns `ExitCode`: on a startup error the full `anyhow` chain is printed to stderr and the code comes from the first classifiable leaf in that chain (`ConfigError` looked up before `DbError`, since a config naming a dead database is still a config problem), `Internal`/1 if nothing matched. Proven by subprocess tests that need no Docker: missing `--config` → 78, invalid config → 78, a closed Postgres port → 69 (the `sqlx` acquire timeout makes that test take ~5 s, documented on the constant). A mutation forcing `1` fails all six. The drain-timeout `exit(1)` on `vpay-server` is unchanged |
| `Money` — integer minor units, XAF zero-decimal | ✅ | 6 tests incl. cross-currency and over-refund rejection |
| Canonical failure taxonomy | ✅ | 3 tests |
| Charge / intent state + `ProviderFlow` | ✅ | 3 tests incl. live-xor-terminal exhaustiveness. **Extended 2026-09-03 (Step 2):** `vpay_core::state` gained `Transition` (`Confirm(ProviderFlow)`, `Cancel`, …) and `next_status`, the single answer to "is this move legal, and where does it land". The table is proven *total* rather than spot-checked — `the_transition_table_covers_every_status_and_verb` and `next_status_answers_the_lifecycle_diagram_for_every_pair` enumerate every (status, verb) pair against [docs/flows/payment-lifecycle.md](flows/payment-lifecycle.md)'s diagram, with `cancel_is_legal_only_from_requires_payment_method`, `confirm_routes_through_the_flows_own_answer` and `a_new_intent_starts_where_the_diagram_says` naming the individual rules. `/v1`'s handlers ask this module rather than testing a status literal, which is why `confirm_legality_does_not_depend_on_the_rails_flow` can hold. 41 tests in `vpay-core`, of which the new `ids` module contributes 6: `pi_`/`ch_`/`re_`/`evt_` prefixes plus 24 Crockford base-32 characters, `is_well_formed`, and `percent_encoding_an_id_is_the_identity` — so an id can go in a URL path unescaped |
| Ledger balancing invariant | 🟡 | Types and `validate()` done + 3 tests. **Persistence not started** |
| Config guard rails (stub host, literal secret) | 🟡 | The two rules (`validate_host`, `validate_secret`) are unchanged and still directly unit-tested (the original 5 tests). **A third joined them 2026-09-03: `validate_webhook_url`**, a sibling of `validate_host` for webhook endpoints, which are whole URLs rather than the bare origins `validate_host`'s substring tests were written for — see the Webhook URL validation row; `validate_host` itself was deliberately left untouched so the rail path could not be weakened by that fix, and `a_webhook_urls_scheme_is_case_insensitive_and_only_its_host_is_searched` is the sibling's own table test. They are now also exercised through real YAML loading: `a_livemode_config_with_an_http_host_is_rejected` and `a_livemode_config_with_a_literal_secret_is_rejected` in `vpay-config`'s `config.rs` drive them through `Config::load_with_env` against fixture files, not just as bare function calls. **Changed 2026-09-03 (Step 2): DB reconciliation — boot-sequence step 4 — is now started.** `vpay_db::ConfigReconcile::reconcile` makes `currencies` and `providers` match the deployment's configuration in one transaction whose first statement takes `pg_advisory_xact_lock(lock_keys::CONFIG_RECONCILE)`, and both binaries call it at boot. A rail absent from the seed is set `enabled = false`, never deleted, because a rail that has ever taken money must stay nameable. Proven against a real Postgres by `reconcile_is_idempotent_and_disables_a_dropped_provider_code`, `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` (which is what proves the lock is actually taken, rather than merely written down) and `two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge`. `lock_keys` is a new module holding both advisory-lock constants with `every_advisory_lock_key_is_distinct_and_positive` and `each_key_decodes_to_its_documented_mnemonic`. **Changed 2026-09-03 (Step 3), and one item is a bug this pass found rather than a feature it added.** (1) **Livemode had never been bootable.** `validate_secret`'s rule — "a credential must be *written* as a `${VAR}` placeholder, not a literal" — is a question about the file's text, and it was being asked of the *resolved* values, where a correctly written `${MTN_API_KEY}` and a literal `hunter2` are the same string. It therefore enforced nothing and refused every correct livemode config; the literal fixture passed only because a literal is also not a placeholder. The pre-resolution text of each `providers[].credentials` value is now captured before resolution and checked against that (`RawProviderSecrets`, private to the module; a credential the map cannot account for fails closed). The "an unresolved placeholder is fatal" rule stays where it was, so the two now answer the two different questions they were always meant to (`a_livemode_config_with_a_literal_secret_is_rejected`, `a_livemode_config_whose_placeholders_resolve_loads`, `a_livemode_placeholder_that_does_not_resolve_is_still_the_unresolved_error`, `a_sandbox_config_with_a_literal_secret_loads`). (2) **`REQUIRED_RAIL_KEYS`** refuses to boot a rail missing a key its adapter cannot work without — MTN `settings.{target_environment, api_user}` + `credentials.{subscription_key, api_key}`, Orange `credentials.{merchant_key, client_id, client_secret}` — with a present-but-empty value counting as missing (`a_rail_missing_a_required_setting_is_rejected`, `a_rail_missing_a_required_credential_is_rejected`, `a_required_key_present_but_empty_is_treated_as_missing`, `a_rail_this_crate_has_no_key_table_for_is_not_refused_here`). It is a provider-code match outside an adapter crate, which ADR-0002 forbids; it is a deliberate, recorded interim — [ADR-0012](adr/0012-rail-configuration-requirements-in-config.md) — that selects a *refusal to start*, never behaviour, and moves behind the port the day the port grows a `required_settings()` hook. (3) **`callback_url` and `currency`** are new on `ProviderHost`: `currency` is required and must be in the canonical table (`a_rail_currency_outside_the_canonical_table_is_rejected`); `callback_url` defaults to `{public_base_url}/provider/{code}/callback`, and the *effective* value — derived or overridden — goes through `validate_host`, so a livemode deployment cannot hand a live rail a plaintext or stub callback host (`a_livemode_callback_url_that_is_not_https_is_rejected`, `a_livemode_deployment_cannot_derive_a_plaintext_callback_url`, `a_derived_callback_url_survives_a_trailing_slash_and_an_override_wins`). (4) **`ProviderHost::to_provider_config(&Deployment)`** is the single place a `vpay_provider::ProviderConfig` is built from YAML, so server and worker cannot disagree about a rail's callback URL, currency or deadlines (`to_provider_config_projects_the_example_config_onto_the_port`, which asserts the whole projected value rather than field by field, and `to_provider_config_names_a_currency_it_cannot_parse`). **Still 🟡 for this row's own claim, and step 4 is still not complete: nothing records or compares a config hash**, so nothing detects a replica booted from a different config file — see "YAML config loading" below and [docs/flows/configuration.md](flows/configuration.md) |
| YAML config loading (`vpay-config::Config::load`) | ✅ | Figment layers `application.yml` with an optional `application-{profile}.yml` overlay (same directory, `<stem>-<profile>.<ext>`); `${VAR}` placeholders are resolved by hand-rolled string scanning (figment's own `Env` provider does not interpolate inside YAML scalars) before typed deserialization, so an unresolved placeholder is a named, fatal error, never an empty string; validation runs `garde`'s structural derive, then the existing `validate_host`/`validate_secret` guard rules over every provider, then a currency-exponent-vs-canonical-table check, duplicate-code checks, and (new this pass) the OAuth-client rules below. 23 dedicated tests in `vpay-config/src/config.rs` cover all of that plus (see "Secret redaction" below) that neither `ProviderHost`'s nor the whole `Config`'s `Debug` output ever contains a credential value. **Upgraded from 🟡 to ✅ this pass, for the two reasons the previous note gave for withholding it — both are now closed and both are proven by an end-to-end subprocess test, not just a library-level one:** (1) **now wired into both binaries.** `vpay-server` and `vpay-worker-bin` both call `Config::load` before opening a database connection, and `--config`/`VPAY_CONFIG` is now required at the binary level (still `Option<PathBuf>` at the `clap` type level, exactly like `--database-url`) — proven by three subprocess tests per binary in each `tests/cli.rs`: a missing config is a non-zero exit naming `--config`/`VPAY_CONFIG` (`a_missing_config_is_exit_78_naming_the_problem`), a config that fails validation is a non-zero exit (`a_bad_config_is_exit_78_naming_the_problem`), and a valid config lets the process boot and (for `vpay-server`) actually serve `/healthz` (`a_valid_config_lets_the_server_boot_and_serve_healthz` / `a_valid_config_lets_the_worker_boot`). (2) **merchant and dashboard OAuth clients are now modelled** — see `crate::oauth` (new this pass: `MerchantClient`, `DashboardClient`) and the new "Merchant OAuth clients" notes folded into this row below. **What is still explicitly out of scope, unchanged from before and stated in the module's own doc comment:** two boot-guard rules from [docs/flows/configuration.md](flows/configuration.md)'s table remain unimplemented on purpose, because they need a *payment-routing* `merchants` concept this config shape does not have — "every merchant's rail host is in the allowlist" and "every referenced provider exists and is enabled." An OAuth `MerchantClient`'s `client_id` is not that merchant concept and has no rail host to check. Boot-sequence step 4 (reconciling into the database in one transaction) is also still out of scope here. Neither gap weakens the claim this row actually makes — that `Config::load` loads, validates, and is used — so ✅ stands for that claim specifically. **Updated 2026-09-03 (Step 2): 57 tests (up from 53), and two new rules that refuse to boot.** `MerchantClient::merchant_id` is now **required and unique** across `merchant_clients` — required rather than defaulted to `client_id` because a default would let a config that forgot it boot and silently invent the one boundary `/v1` has no second line of defence for, and unique because two credentials sharing a tenant could read each other's objects (`a_merchant_client_without_a_merchant_id_does_not_load`, `two_merchant_clients_sharing_a_merchant_id_are_rejected`, fixture `oauth-duplicate-merchant-id.yml`; `ConfigError::DuplicateMerchantId`). `ProviderHost::enabled` is new and defaults to enabled when the line is absent (`a_provider_with_no_enabled_line_is_enabled`, `an_explicitly_disabled_provider_stays_disabled`). **One of the two "structurally impossible" gaps this row records is now half-closed:** "every referenced provider exists and is enabled" is enforced at boot in the direction that *is* expressible — a YAML rail with no linked adapter is `ConfigError::ProviderWithoutAdapter` and exit `78`, before the port is bound (`a_provider_code_with_no_linked_adapter_is_exit_78` in `backends/apps/vpay-server/tests/cli.rs`, with `the_repositorys_own_configuration_passes_the_adapter_join` asserting the shipped `config/application.yml` satisfies it). The *merchant*-facing half is still impossible for the same reason as before: an OAuth `MerchantClient` names no rails, and there is no merchant→rail routing concept to check. **Updated 2026-09-03 (Step 3): 70 tests (up from 57), measured** (`cargo nextest run -p vpay-config`: 70 run, 70 passed, 0 skipped). The new rules are in the "Config guard rails" row above; the shipped `config/application.yml` is itself loaded by `a_valid_config_loads_and_produces_the_expected_typed_values` and projected onto the port by `to_provider_config_projects_the_example_config_onto_the_port`, so a `${VAR}` added to that file without a matching entry in `compose.e2e.yml` is an exit-`78` boot failure and not a silent empty string — six rail variables are now referenced (`MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `MTN_API_USER`, `ORANGE_MERCHANT_KEY`, `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET`) |
| `checkout.public_base_url` and `merchant_clients[].checkout_origins` (`vpay_config`) | ✅ | **New 2026-09-04 (Step 9 lane 1).** `checkout.public_base_url` is the origin (optionally with a path prefix) every payer link is built on; **absent is a complete answer** — a deployment that omits it serves no checkout page and `POST /v1/checkout/sessions` answers `checkout_not_configured` rather than minting a `url` that resolves to nothing. `checkout_origins` is the per-merchant list `frame-ancestors` is derived from; an empty list (the default) means no site may embed. Six named `ConfigError` variants with one fixture each under `backends/crates/vpay-config/tests/fixtures/checkout-*.yml`: `MalformedCheckoutBaseUrl`, `InsecureCheckoutBaseUrl`, `MalformedCheckoutOrigin`, `InsecureCheckoutOrigin`, `DuplicateCheckoutOrigin`, `CheckoutOriginsWithoutBaseUrl`. Both are `https`-only under `deployment.livemode`, checked at boot. `config/application.yml` carries a worked example of each. **Tightened 2026-09-04 (lane 1b):** an entry must also be the *canonical* spelling a browser compares against — `parsed.origin().ascii_serialization()`, i.e. lower-cased host, IDNA-encoded to ASCII, default port elided. `https://Shop.example`, `https://shop.example:443` and `https://shöp.example` all passed every earlier rule and were all dropped **silently** by the checkout app's own filter (`frontends/apps/checkout/src/lib/origins.ts`), leaving the merchant with `frame-ancestors 'none'` and no diagnostic anywhere. `ConfigError::NonCanonicalCheckoutOrigin` names what to write instead rather than which rule was broken, because the useful part of that message is a value. Refused rather than normalised, so the file and the running policy stay the same document. One fixture, and a unit case that also accepts the three canonical spellings so it cannot pass by refusing everything |
| `merchant_clients[].display_name` (`vpay_config`) | ✅ | **New 2026-09-04 (Step 9, lane 1b).** What a payer is told they are paying, on vpay's own checkout page. Optional, non-blank, at most 80 **characters** — a rendering bound, not a storage one: it is painted into "Pay {merchant}" in a heading on a phone-sized page, and it is refused at boot (`ConfigError::MalformedDisplayName`) rather than truncated at render time. Not secret: it is rendered to every payer of this merchant by construction, so it prints in `MerchantClient`'s `Debug`. Rendered as `merchant: { name }` on both `/v1/browser/checkout/sessions/{id}` and `…/return` — the member name `frontends/apps/checkout`'s own envelope reads, so a server that rendered `display_name` there would show a payer no name at all. **A merchant with none has the member absent from the body entirely** (the integrator's `6abbaa0`, superseding lane 1b's tenant-id fallback): `ResourceConfig::merchant_display_name` answers `Option<&str>`, `CheckoutSessionForPayer.merchant` is `Option<_>`, and the page paints a neutral heading (`page.pay_to_unnamed` and its three siblings, lane 3b) rather than a stand-in that reads like data. `both_browser_reads_carry_the_merchants_display_name` asserts both branches in one deployment, including that the tenant id appears nowhere in the body. Sample in `config/application.yml`, one fixture. **A deployment serving hosted checkout should set this for every merchant**: without it a payer is shown an amount and no name |
| Merchant/dashboard OAuth client modelling (`vpay-config::oauth`, ADR-0010) | ✅ | New this pass, folded into the row above operationally but broken out here because it is a distinct piece of new modelling: `MerchantClient` (public JWK set, `client_credentials` only) and `DashboardClient` (redirect URIs, a single `scope` — enforced by the type being a `String`, not a `Vec<String>`), plus a closed local `GrantType` enum whose serde wire form matches `authkestra_op::client::GrantType`'s. Both carry a `client_secret: Option<String>` trap field that must always be `None`, with hand-written redacting `Debug` impls (5 tests in `oauth.rs`, including one proving a populated `client_secret` never appears in `{:?}` output). Seven boot-time validation rules run from `Config::validate_all`, each with a dedicated fixture-driven test asserting the *specific* `ConfigError` variant: duplicate `client_id` across merchants and the dashboard, an empty/keyless merchant JWKS, a merchant declaring a grant other than `client_credentials`, a dashboard client with no redirect URI, a non-`https` livemode dashboard redirect URI (reusing `validate_host`), and a client secret present anywhere (merchant or dashboard, tested separately). **This is authentication-client modelling only, not merchant *payment routing*** — see the row above's "still out of scope" note for exactly what that distinction means and does not cover |
| Secret redaction (`ProviderHost`/`CommonArgs` hand-written `Debug`) | ✅ | `ProviderHost` (rail credentials) and `CommonArgs` (`--database-url`, which routinely embeds a plaintext password) both hand-write `fmt::Debug` to redact secret values while keeping every other field, and credential *keys*, visible. `Config`, `ServerArgs` and `WorkerArgs` keep `#[derive(Debug)]` — safe because a derive formats each field via *that field's own* `Debug` impl, so the redaction composes upward without a second hand-written impl at every level. Six dedicated tests prove this holds, not just for the leaf types but through the composition: `provider_host_debug_output_never_contains_a_credential_value`, `a_whole_config_debug_output_never_contains_a_credential_value` (`vpay-config/src/config.rs`), `common_args_debug_output_never_contains_the_database_password`, `server_and_worker_args_debug_output_never_contains_the_database_password` (`vpay-config/src/cli.rs`), plus two more asserting the non-secret fields (rail code, host, `[redacted]` marker itself, `database_url: None` when unset) stay visible so the redaction does not silently swallow useful debugging signal. Marked ✅ because a re-derived `Debug` on either type — the exact regression these tests exist to catch — fails the build. **Residual risk stated, not hidden:** `ProviderHost::settings` and `::credentials` are both plain `BTreeMap<String, String>` and only `credentials` is redacted; a value accidentally placed in `settings` instead would leak in plaintext, and no test (and no type) can catch a value merely misclassified between the two maps — the boundary is enforced by convention, not by the type system. **That risk got concrete on 2026-09-03 (Step 3), and the classification was made deliberately:** `ProviderHost` gained `callback_url` and `currency` (both printed, both non-secret), and the rails' new `settings` keys — MTN's `target_environment` and `api_user`, Orange's `env` and `lang` — **print in full** in `ProviderHost`'s `Debug`. That is intended: `api_user` is a UUID identifier, not a bearer secret, and knowing which target environment loaded is exactly the debugging need after a boot failure. Orange's `merchant_key` was considered for `settings` on the same reasoning and **kept in `credentials`** (Step 3, decision 4), so it stays redacted. Every actual secret — `subscription_key`, `api_key`, `client_id`, `client_secret`, `merchant_key` — is in `credentials` and prints as `[redacted]` with its key still visible (`provider_host_debug_output_never_contains_a_credential_value`, `provider_host_debug_output_still_contains_the_non_secret_fields`). The adapters carry the same discipline into their own types: `debugging_the_adapter_does_not_print_the_token`, `debugging_credentials_does_not_print_them`, `debugging_a_token_does_not_print_it` (MTN), `the_adapters_debug_carries_no_credentials`, `debug_never_prints_the_bearer` (Orange) |
| CLI / env configuration (`vpay-config::cli`) | 🟡 | `--version` reports `0.1.0`. Every option auto-resolves from an env var with an explicit flag winning, shared between both binaries via a flattened `CommonArgs`, covered by unit tests on the built `clap::Command` plus subprocess tests that set real env vars on a child process. **`--database-url` is no longer inert** — both binaries now treat it as required at runtime and use it to open a real connection pool and run migrations before serving (see "Database connectivity" below); it stays `Option<String>` at the clap type level, so the CLI itself does not enforce presence, only the two binaries' own startup logic does. **`--config` is no longer inert either, as of this pass** — both binaries now treat it as required at runtime too (same `Option<PathBuf>`-at-the-clap-level, required-in-`main.rs` pattern as `--database-url`), calling `vpay_config::Config::load` and refusing to start on a missing or invalid file; see the "YAML config loading" row above for the three subprocess tests per binary that prove this. **New 2026-09-02 (Step 1): `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`**, on `vpay-server` only — the worker issues no tokens, so mounting the Secret into it would widen its blast radius for no capability. Same `Option<PathBuf>`-at-clap, required-in-`main` pattern as `--config`, and required *before* the database connection, so all three failure modes exit `78` with no Docker needed: `a_missing_signing_key_flag_is_exit_78_naming_the_problem`, `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`, `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents`. The *path* is deliberately not redacted from `Debug` (a path is not a secret, and "which file did it try" is the first thing an operator needs); the file's contents never enter `ServerArgs` at all. Three unit tests in `cli.rs` pin that shape: `the_signing_key_file_flag_parses_to_the_path_it_was_given`, `the_signing_key_path_stays_visible_in_debug_output`, and `the_worker_is_not_handed_the_signing_key`. **`--public-base-url` is gone as of 2026-09-03 (Step 6, block A; step-6 decision (7)).** It had been accepted and parsed and consumed by nothing since it was added — the issuer is `vpay_api::op::issuer_for(&config)`, which reads **`deployment.public_base_url` from the YAML config**, never the CLI flag — so two spellings of one idea existed with the shorter one inert. The flag, its `VPAY_PUBLIC_BASE_URL` variable and its row in the env-name table are deleted; `ServerArgs` no longer has the field. **What this costs, stated because it is a breaking change to an interface:** a deployment that set `--public-base-url` now fails to start on an unknown argument rather than ignoring it. Nothing in this repository set it (`compose*.yml`, the chart, `.env.example` and every fixture grepped), and failing loudly is the right answer for an operator who believed the flag did something — but a downstream chart that set it will break on upgrade. `deployment.public_base_url` in the YAML is unchanged and is still the only source of the issuer. **A pre-existing gap this pass looked at and left alone:** a missing `--database-url` still exits `1`, not `78`, because `main` produces a bare `anyhow` error there with nothing for `exit_code_for` to classify — the `StartupError` introduced for the signing key covers only the signing key. Out of scope, and stated so nobody reads the new `78`s as meaning every missing flag is classified. **New 2026-09-03 (Step 2): one more `78`, and it is a *configuration* failure rather than a flag one.** Both binaries now derive their reconcile seeds at boot from `vpay_api::v1::boot::boot_seeds` — the same function in both, so they cannot disagree about which rails exist — and a YAML provider code with no linked adapter in *that* binary is `ConfigError::ProviderWithoutAdapter`, which classifies as `Category::Configuration` and exits `78` before Postgres is contacted. `a_provider_code_with_no_linked_adapter_is_exit_78` is a subprocess test against the fixture `backends/apps/vpay-server/tests/fixtures/provider-without-adapter.yml`, so it needs no Docker |
| Database connectivity (`vpay-db`: pool, migrations, healthcheck) | 🟡 | New crate this pass: `connect()` (a `PgPoolOptions` pool, max 10 connections, 5s acquire/connect timeout, eager — it does not return until at least one connection succeeds or the timeout elapses), `run_migrations()` (`sqlx::migrate!` against `backends/migrations`, idempotent by construction), and `check_connection()` (`SELECT 1`). All three are tested against a real `postgres:16-alpine` via testcontainers in `vpay-db/tests/postgres.rs`: `run_migrations_applies_cleanly_and_is_idempotent`, `check_connection_succeeds_against_a_live_database`, and `check_connection_fails_against_a_dead_database` (the container is stopped mid-test to prove the failure path, not just asserted by reading the code). Both `vpay-server` and `vpay-worker-bin` now call `connect()` then `run_migrations()` before doing anything else observable, and this happy path is proven end-to-end, not just at the crate level: `backends/apps/vpay-server/tests/cli.rs` spawns the real binary against a real testcontainers Postgres and polls `GET /healthz` until it returns **200** (`bind_and_log_format_env_vars_are_actually_applied` and others); `vpay-worker-bin`'s equivalent tests prove the same connect-then-migrate sequence via its startup log lines. **Marked 🟡, not ✅, because two specific claims this pass makes are implemented but not proven by any test:** (1) **"a missing `--database-url` is a hard startup failure"** — true by reading `main.rs` in both binaries (`args.common.database_url.as_deref().context(...)?`), but every subprocess test in both `tests/cli.rs` files always supplies `DATABASE_URL`; no test spawns either binary without it and asserts a non-zero exit. (2) **"`/healthz` returns 503 when the database is unreachable"** — true by reading `vpay-api/src/lib.rs`'s `healthz` handler, which maps a `check_connection` error to `StatusCode::SERVICE_UNAVAILABLE`, and `check_connection`'s own failure path is unit-tested in `vpay-db` (above) — but nothing kills the database mid-request and polls the real HTTP endpoint to observe a 503; the handler's status-code mapping itself is unexercised by any test. **Extended 2026-09-03 (Step 2): `vpay-db` is no longer only a pool.** Five repository modules landed — `payment_intents` (insert, merchant-scoped get, keyset `list_page`, `transition` as a compare-and-swap on the expected status, `cancel` with its second `NOT EXISTS` guard), `charges` (`insert_for_intent`, taking a `PgConnection` so the *caller* decides the commit point, which is what lets `confirm` commit before the network), `idempotency` (`claim`/`store`/`release`/reclaim-expired/`sweep_expired`), `provider_requests` (`insert_pending`/`record_response`), and `config_reconcile` — plus `lock_keys` for the advisory-lock constants. `DbError` gained `UniqueViolation` (SQLSTATE `23505` → `Category::Conflict`, code `resource_conflict`, deliberately *not* `invalid_state`: the object is not in a forbidden state, it already exists) and `ForeignKeyViolation` (`23503` → `InvalidRequest`), classified by `integrity_violations_are_the_callers_problem_not_a_storage_outage`. **`vpay-db` now runs 32 tests, 24 of them container-backed in `tests/repositories.rs`, and all 32 passed on 2026-09-03** — including `a_transition_from_a_stale_expected_status_changes_nothing` and `an_intent_in_an_unseeded_currency_is_a_named_foreign_key_violation`. Still 🟡 for this row's own claim, unchanged: nothing observes `/healthz` returning a real 503 |
| Repository layer (`vpay_db::Repositories`, `UnitOfWork`) | ✅ | **New 2026-09-03 (Step 7, Phase A).** One `#[async_trait]` trait per table family — `PaymentIntents`, `Charges`, `Idempotency`, `ProviderRequests`, `Events`, `Jobs`, `WebhookDeliveries`, `Settlement`, `SigningKeys`, `DisabledClients`, `ClientAssertions`, `ConfigReconcile`, `Health`, plus `Migrations` for `run_migrations`, which is not a table family but had nowhere else to go once `PgPool` stopped leaving the crate — under one umbrella `Repositories`. **`vpay-db` exposes no `pub fn` that runs a statement any more** — 60 of them became 51 table-family trait methods plus the 9 on `TxRepositories`, keeping their names: every query moved into the trait impl, its SQL and its doc comment. `PgRepositories` is the sole implementation and is **private**; `vpay_db::connect(url)` returns `Arc<dyn Repositories>` and is the only way to obtain one, so no test double can be substituted (ADR-0006 untouched — every suite that exercises this crate still runs against a real `postgres:16-alpine`). Transactions are a closure: `UnitOfWork::transaction(f)` hands `&mut dyn TxRepositories` to `f` and commits or rolls back from what it returns, so "forgot to commit" is not expressible; `TxOutcome::{Commit, Abandon}` exists because a successful closure has two endings — `vpay_worker::webhooks::fan_out_one` loses a race to another drain and rolls back *without* that being an error, and the confirm path's duplicate-charge recovery abandons and re-reads outside the transaction. The closure's error type is a parameter (`E: From<DbError>`) rather than `DbError`: three call sites raise their own layer's error from inside the unit of work (`ApiError` for the confirm invariant, `JobError` for a payload that will not encode), and pinning it to `DbError` would have forced them to mislabel it as storage. Consumers hold `&dyn Repositories`: `vpay-api`'s router state (`RouterDeps.repositories`), both binaries, and every `vpay-worker` handler. **`vpay-worker`'s no-`sqlx` rule is now structural** — its `Cargo.toml` said "and that is a rule and not an oversight" while seven call sites spelled `pool.begin()`; `sqlx` is unnameable there now (`vpay_db` no longer re-exports `PgPool`, and an open transaction is an opaque `PendingTransaction`). **One documented exception:** `Repositories::op_store_pool()` still hands out the raw pool for authkestra's `SqlxOpStore<sqlx::Postgres>` and `SqlClientAssertionStore` (ADR-0010) — foreign trait implementations over a pool whose queries vpay does not own and cannot express as repository methods. That is why `vpay-api/Cargo.toml` still lists `sqlx` under `[dependencies]` after a step that spent itself removing it; it is Step 7's decision (9), not an oversight. `vpay_db::connect_lazy` is `#[doc(hidden)]` and has **no production caller**: it exists because `connect` is deliberately eager, which makes "a handle whose queries fail" unobtainable, and that is exactly what `vpay-api`'s own unit tests need to prove an unreachable database produces a refusal rather than an admission. It is not a double — the pool is the real `sqlx` one — and because it is not, no dependency rule would ever have caught a binary reaching for it, so **`verify-no-mocks` now fails the build if it appears in non-test code under `backends/apps`** (proven by putting the call into `vpay-server`'s `run`). A `TxOutcome::Abandon` does **not** surface a failed rollback: `ROLLBACK` is best-effort by construction — the transaction is aborted whether the statement lands or the connection dies first — so a failure there changes nothing about the database and only about what the caller may say, and both abandoning call sites have an answer that must survive (the confirm path's `409`, `persist_submitted`'s `Internal` alert saying a rail may hold a live payment). It is logged at `warn!` and swallowed; `an_abandoned_transaction_survives_a_rollback_it_cannot_send` (`vpay-db/tests/postgres.rs`) stages it against a real Postgres by terminating the backend that holds the open transaction, and asserts that the commit path under the same staging still fails — the control that makes the abandon half mean something. `TxOutcome::is_commit` was removed in the same pass: it had no callers. **Proven by**: the whole suite green (972 tests, 39 binaries, 0 ignored), and by pointing one consumer back at a removed free function (`vpay_db::settlement::live_charges_stale_since`) and watching `vpay-worker` fail to compile |
| Provider port trait (`vpay-provider`) | ✅ | **Changed 2026-09-03 (Step 3): the trait is now `#[async_trait]`** — `submit`, `query_status` and `refund` are `async`; `parse_callback` stays synchronous on purpose, so an adapter cannot make a network call while "parsing" an unauthenticated hint. `async_trait` rather than a native `async fn` because a trait with one is not dyn-safe and this port is only ever held as `Box<dyn ProviderAdapter>` — which is what makes `if provider == "mtn_momo"` structurally impossible outside an adapter crate (ADR-0002). `refund`'s default is `ProviderError::Unsupported`, a permanent capability answer, **not** `NotImplemented`. `ProviderConfig` gained `connect_timeout`/`request_timeout` (5 s / 20 s constants, on the config rather than the client because one `reqwest::Client` is shared by every rail). **The vendored-roots HTTP client moved here from `vpay-api` as `vpay_provider::http`** — `vpay_api::http_client` is a re-export, so no call site changed. It refuses redirects, ignores `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`, and caps any rail body at `MAX_RAIL_BODY_BYTES` (256 KiB) via `bounded_body` rather than reading to end of stream. **Decision 2's cost, stated rather than hidden: `vpay-provider` is no longer a pure interface crate** — it links reqwest, rustls and webpki-roots, so a future non-HTTP rail (a USSD gateway, a file drop) compiles a TLS stack it never uses; no *binary* grew, because both already resolved all three. 11 tests in the crate, measured 2026-09-03 — including `a_redirect_is_returned_rather_than_followed`, `a_request_timeout_actually_fires_against_a_silent_peer` and the `Classify` table for `ProviderError`. See [docs/flows/provider-port.md](flows/provider-port.md) |
| Process lifecycle (SIGINT/SIGTERM) | ✅ | `vpay-server` shuts down via `axum::serve(...).with_graceful_shutdown(...)` on SIGINT or SIGTERM instead of requiring `docker compose down` to SIGKILL it. `vpay-worker-bin` stays up and answers the same signals. **Changed 2026-09-03 (Step 4): the startup WARN banner and the 60-second "job loop is not implemented" heartbeat are gone**, because the loop exists — what the process logs every 60 seconds now is the job-loop gauge line (see the Worker job loop row). **Startup race fixed this pass:** both binaries used to construct their shutdown-signal future late (inside `with_graceful_shutdown`'s argument, or just before the worker's select loop) — `tokio::signal::unix::signal(..)` and `tokio::signal::ctrl_c()` both install their OS-level handler on first *poll*, not at construction, so a SIGTERM delivered before that first poll (CLI parsing, tracing init, adapter-registry logging, `TcpListener::bind` all had to complete first) kept its default disposition and killed the process outright, skipping graceful shutdown and dropping any in-flight request. Confirmed by reproduction (`kill -TERM` sent tens of milliseconds after spawn reliably produced exit 143 with no shutdown log line) and by reading `tokio`'s own source (`signal_hook_registry::register` runs synchronously inside `tokio::signal::unix::signal`'s function body, not inside the future it returns). Fixed by `vpay_config::signal::ShutdownSignals`, a new type in `backends/crates/vpay-config/src/signal.rs` shared by both binaries (precedented by `CommonArgs` living in the same crate): `ShutdownSignals::install()` is now the first thing either binary's `main` does, before tracing init, registering SIGTERM/SIGINT handlers before any slower startup work can run. On Unix, SIGINT is now handled via `signal(SignalKind::interrupt())` rather than `tokio::signal::ctrl_c()` specifically because `ctrl_c()` is an `async fn` and would reintroduce the same late-installation race; non-Unix platforms still fall back to `ctrl_c()` inside `ShutdownSignals::wait()`, unchanged from before. A failure to install a handler is now a **hard startup failure** (`main` returns `Err`), not a logged warning that lets the process run its whole life with no graceful-shutdown path — deliberately stricter than before, since silently continuing would reintroduce the exact bug for the entire process lifetime rather than a brief window. Both binaries are exercised by subprocess tests that send a real `SIGTERM` and assert a clean exit (`backends/apps/vpay-server/tests/cli.rs`, `backends/apps/vpay-worker-bin/tests/cli.rs`), including a new regression test per binary (`sigterm_immediately_after_startup_still_triggers_graceful_shutdown`) that sends SIGTERM almost immediately after spawn and asserts both exit 0 and the graceful-shutdown log line. **That regression test's own limits, stated plainly:** it is a statistical majority-vote test (`ATTEMPTS`/`MIN_SUCCESSES` spawn-signal-wait trials), not a deterministic one, because the actual race window on modern hardware is on the order of a millisecond once other confounds (binary cold-start, CPU frequency ramp-up) are controlled for — verified in isolation to reliably fail against the pre-fix code and pass against the fix (repeated hundreds of times across macOS and a Linux container). But `cargo nextest run --workspace`'s real contention from ~20 concurrently running test binaries widens that window for *both* fixed and unfixed code enough that no single delay was both safe against the fixed binary and sensitive to the bug under full-suite load; the delay actually shipped (`DELAY = 50ms`) was chosen to never fail the full suite on correctly fixed code, at the cost of not reliably catching the bug when run as part of the full suite — its demonstrated sensitivity is strongest when run scoped/alone. This is disclosed in the test's own doc comment, not hidden. |
| `--shutdown-grace-seconds` bounded drain | 🟡 | On `vpay-server` this is now wired in: `serve_with_bounded_drain` in `backends/apps/vpay-server/src/main.rs` races the axum drain against a `shutdown_grace_seconds`-long clock and exits non-zero if the clock wins, logging that in-flight work was cut off. ~~**No test exercises the timeout path itself** — the existing SIGTERM tests never have in-flight work to drain, so they would pass identically with the grace clock deleted~~ (retired below, in the Step 6 block A note; kept struck through so the two later corrections read in order); nothing here proves the bound actually holds under load. **Changed 2026-09-03 (Step 4): the worker half is real, and it is the half that has a test.** `vpay_worker::run_loop` starts the grace clock **when the shutdown signal arrives**, not at boot (racing `timeout(grace, drain)` from the start would abort every in-flight job `grace` seconds after startup, forever). Tasks stop claiming at the top of an iteration, so a clean drain settles every claimed job and leaves no lease to hand back — `LoopReport::released` is 0 on one by construction. On timeout the remaining tasks are aborted, `jobs::release_all` hands back every lease this worker still holds, and the binary exits **1**. `a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds` (`backends/tests/integration/tests/worker_e2e.rs`) exercises exactly the timeout path against a real Postgres: jobs in flight, a signal, a grace period too short, and afterwards **zero rows still leased**. Delete the `release_all` call and it fails. **Changed again 2026-09-03 (Step 6, block A): the server half now has the test this row said it lacked.** `an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so` (`backends/apps/vpay-server/tests/cli.rs`) holds a real request open against the running binary, sends SIGTERM while it is genuinely in flight, and asserts exit **1** plus the forced-cutoff WARN in the log. The load is a *stalled request body* — the client promises `Content-Length: 200` on `POST /v1/oauth/token` and sends 29 bytes, so the handler's `Form` extractor is genuinely pending inside the router — rather than the delayed rail response `docs/plans/2026-09-03-step6-deployment.md` §5 sketched. **A deliberate deviation, and what it costs:** the sketch's version would have needed a WireMock container, a merchant registration, a minted `private_key_jwt` and a token exchange rebuilt inside a crate that has none of them, to make one request slow; a slow client is slow for free and is not a stub of anything, so `verify-no-mocks` is untouched. What it does *not* prove is that a slow **rail** produces the same outcome — it does not need to, because the drain is a property of the connection rather than of what the handler awaits, but that is a distinction worth not glossing. Both halves of `--shutdown-grace-seconds` now have a timeout-path test; this row is ✅ on its own terms, and stays 🟡 only because no *deployment* has ever drained one |
| Poll ladder | ✅ | **Changed 2026-09-03 (Step 4): was 🟡 "job loop not started".** `vpay_worker::poll_delay` (10s, 20s, 30s, 45s, 60s, 90s, then 120s to the 30-minute mark, then 15 min out to 24 h) is now *driven*: a rung is one `UPDATE jobs SET run_at = now() + delay`, indexed by `jobs.attempts - 1` because the claim itself increments the counter. Four unit tests pin the ladder (fast start, never stops, monotonically non-decreasing, and the hourly `UNRESOLVED_POLL_INTERVAL` is slower than every rung); `the_ladder_rung_is_one_below_the_attempt_count` pins the off-by-one that would otherwise make every retry wait one rung too long forever. The ladder is exercised for real by the worker suites — `three_not_founds_over_the_window_resubmit_and_two_do_not` runs three rungs with a 50 ms window and no sleeps, `a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it` runs the first rung against a WireMock rail that answers `PENDING` then `SUCCESSFUL`. The loop that consumes it has its own row (Worker job loop) |
| HTTP surface | 🟡 | **Changed 2026-09-02 (Step 1): the surface is no longer `/healthz` plus a 404.** `router(RouterDeps)` now serves, unauthenticated by necessity rather than omission, `GET /healthz`, `POST /v1/oauth/token`, `GET /v1/oauth/.well-known/openid-configuration` and `GET /v1/oauth/jwks.json` (`the_oauth_routes_are_reachable_without_a_token`); **every other path under `/v1` is nested behind the `AuthenticatedMerchant` extractor, and that nest's only route is the honest 404.** That pair of answers is the whole observable boundary: `GET /v1/payment_intents/pi_x` with no bearer token is a 401 envelope (`an_unauthenticated_v1_request_is_401_not_404`, `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope`), and the same request with a valid merchant token is a 404 `unknown_route` (`an_sdk_client_authenticates_and_reaches_the_honest_404`, integration). **A 200 there would mean someone invented a resource, which is the failure this repo's `CLAUDE.md` names first — so `/v1` still implements no business resource at all, and this row stays 🟡 for exactly that reason.** The auth layer is `Router::layer`, not `route_layer`: `route_layer` does not wrap a fallback, and since this nest's only route *is* its fallback, axum refuses the build — verified by making the swap and watching every router test panic. That protection disappears the moment a real `/v1` route lands, which is why the choice is written down in `router`'s own doc comment. **Two known edges, both documented in code and neither fixed:** `GET /v1/oauth/token` (right path, wrong method) gets axum's bare `405` with an empty body instead of the Stripe envelope — the status is correct and turning it into the 404 envelope would be worse, so it waits for a `method_not_allowed` renderer for the whole surface; and `GET /v1/` (the bare trailing-slash form) falls through to the *outer* 404 rather than the nest's 401 (`the_bare_trailing_slash_form_of_v1_falls_through_to_the_outer_404` pins this as the current behaviour, not as desirable). What the previous pass changed, unchanged since: `/healthz` is no longer a static `"ok"` string. It runs `vpay_db::check_connection` (a real `SELECT 1`) and returns `200`/`"ok"` or `503`/`"database unreachable"` depending on the result — see "Database connectivity" above for exactly what is and is not tested about that mapping. The router's constructor argument changed with Step 1 from a bare `PgPool` to `RouterDeps { pool, merchant_op, merchant_validator }` — so a router cannot exist without a database connection *or* without the OP and the validator that guard `/v1`; there is no way to build a partially-wired one. **New 2026-09-02: request ids and a per-request span.** `router()` mounts, in this order, a guard that drops a caller-supplied `x-request-id` unless it is 1–64 bytes of ASCII `[A-Za-z0-9._-]` (removed, never rejected: a bad diagnostic header must not block a payment request — the caller merely loses the right to choose the id; one unusable value drops the whole header), tower-http's `SetRequestIdLayer` (mints a v4 UUID `x-request-id` unless the caller sent one the guard kept), a `TraceLayer` whose span records `method`, `path` and `request_id`, and `PropagateRequestIdLayer` (copies the id onto the response). Seven of `lib.rs`'s fourteen tests (the other seven cover the route tree above): a request with no id gets a UUID back, a caller-supplied id comes back unchanged, the `api error` line `ApiError` logs while serving a 404 carries the request id, a 4 KB id / an id with a space, `/`, `"` or a non-ASCII byte / a 65-byte id are each replaced by a minted UUID while a 64-byte one survives, and one unusable value among several drops the header — each proven decisive by disabling the guard, the span field or the propagate layer and watching the relevant tests fail. This is what makes `Category::Internal`'s "Contact support with the request id" a sentence a merchant can act on; `error.rs`'s "No `request_id` field here" section now points at the mounted layers instead of saying they are not mounted. `/healthz`'s plain-text body and the 404 envelope bytes are unchanged (pinned). **Security review the same day:** the `/v1/oauth` nest has its own explicit 404 fallback — measured, an unmatched path under it used to fall into the *authenticated* `/v1/{*rest}` route and answer 401, telling an integrator who mistyped an OP path to present a bearer token on the one subtree that issues them; `the_oauth_nest_answers_its_own_404` pins the 404. **Changed again 2026-09-03 (Step 2): the nest is no longer one 404.** `vpay_api::v1::V1_ROUTES` is now the router's *source* — a `&[V1Route]` of path, methods and mount function that `routes()` folds into the `Router`, so a route cannot exist without appearing in the table a test can walk. Four paths, five methods: `POST`/`GET /payment_intents`, `GET /payment_intents/{id}`, `POST /payment_intents/{id}/confirm`, `POST /payment_intents/{id}/cancel`. `every_registered_v1_path_answers_401_without_a_token` (integration) walks `V1_ROUTES` itself rather than a hand-kept copy, so a new route is covered the day it lands. ~~`/v1/balance` and `/v1/refunds` are deliberately **not** mounted and still answer the nest's 404 — both SDKs can call them and vpay implements neither.~~ **Corrected 2026-09-05 (issue #45): `GET /v1/refunds/{id}` is mounted** (its own row below). `/v1/balance` and **`POST /v1/refunds`** are deliberately not mounted and still answer the nest's 404 — both SDKs can call them and vpay implements neither, because there is no ledger read path and no rail that can refund. **`/v1/events` was on that list until 2026-09-03 and is now mounted** (Step 5): `GET /v1/events` and `GET /v1/events/{id}`, read-only, merchant-scoped, cursor-paged, rendering through the same `EventObject` the webhook deliverer signs. Its `?type=` filter is documented in `docs/api/README.md` and **not** implemented — unknown query parameters are ignored on this whole surface, so `?type=…` returns an unfiltered page rather than a `400`. The auth layer is now a middleware (D3) that validates the bearer token **once**, resolves the token's `client_id` to a `MerchantScope` through the YAML, puts that scope in request extensions, and checks the method's required scope before axum matches a route: `payments:write` for anything that is not a read, `payments:read` or `payments:write` for `GET`/`HEAD`, `403` otherwise (`only_a_write_scope_authorises_a_method_that_is_not_a_read`, `a_clients_tenant_comes_from_config_not_from_the_client_id`, `a_token_without_the_required_scope_is_403_not_401`, and `a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not` over real HTTP). `MerchantScope`'s field is `pub(crate)` and the extractor is the only public way to obtain one, so a handler cannot invent a tenant; there is no `merchants` table and therefore no foreign key that would catch a query missing its filter, which is the whole reason it is built this way. A `RequestBodyLimitLayer` of **64 KiB** is mounted on the nest (`a_body_over_the_limit_is_refused_by_the_layer`). The `route_layer`-versus-`layer` note above no longer applies the way it did: real routes now exist beside the fallback **Changed 2026-09-03 (Step 6).** (a) The probe split the `healthz` doc comment had pre-authorised since Step 1 exists: `/healthz` stays here, unauthenticated, still running a real `SELECT 1`, and is the **readiness** probe; **`/livez` is not on this router at all** — it is on the observability listener (row in Infrastructure), because a liveness probe that fails on a database outage restarts every pod in the deployment and a restart cannot fix a database. `neither_livez_nor_metrics_is_reachable_on_the_traffic_router` fails if either path ever appears here. (b) A fifth middleware, `track_http_metrics`, counts and times every response under the **matched route pattern** — mounted three times when this was written for Step 6 (outer routes, `/v1/oauth`, `/v1`); **corrected 2026-09-03 rebasing Step 5c onto this tree: mounted a fourth time, inside the `/v1/browser` nest, so a browser GET/POST is labelled `/v1/browser/payment_intents/{id}` rather than left uncounted — see the Browser checkout surface row and `every_mounted_group_is_counted_exactly_once` (unit) / `a_browser_get_is_counted_under_its_own_route_pattern` (integration, real Postgres)** — because axum exposes `MatchedPath` only to a layer on the router that matched, and mounted on `/v1` *outside* the auth layer so a `401` is counted against the route it was aimed at rather than as `unmatched`. It records no caller-supplied string, which is what keeps an anonymous caller from minting a time series per request |
| Payment intents (`/v1/payment_intents`) — `vpay_api::v1::payment_intents` | 🟡 | **New 2026-09-03 (Step 2). The first `/v1` business resource, and it stops at the rail on purpose.** Evidence for every claim below is `backends/tests/integration/tests/payment_intents.rs` (16 tests) unless another file is named, and all 16 passed against a real `postgres:16-alpine` on the authoring machine on 2026-09-03 (header). **✅ create**: `POST /v1/payment_intents` decodes a Stripe-bracket form body, validates `amount` (1 … 2^53−1, the same bound both SDKs enforce client-side), `currency` against the deployment's configured set, each `payment_method_types[]` against the *enabled* rails, `metadata` (50 keys / 40-char keys / 500-char values) and `description` (1000), writes a row in `requires_payment_method`, and renders the wire object (`create_then_retrieve_round_trips_through_the_sdk`, plus the unit rules `a_currency_the_deployment_did_not_configure_is_refused_by_name`, `a_disabled_rail_cannot_be_named_on_a_new_intent`, `the_amount_bounds_are_the_ones_both_sdks_enforce`, `metadata_and_description_bounds_name_their_own_parameter`). **✅ retrieve**, including the tenancy property that matters: another merchant's id answers the *identical* 404 (`merchant_b_cannot_read_merchant_as_intent`). **✅ list**: keyset pagination over `seq`, newest first, `limit` defaulting to 10 and capped at 100; both cursors together is a `400`, a malformed cursor is a `400`, and a well-formed cursor belonging to another merchant is an **empty page** rather than an error, because saying "no such id" would leak (`list_pages_forward_and_backward_with_cursors`, `a_list_refuses_two_cursors_and_a_malformed_one`, `a_cursor_is_checked_for_shape_and_not_for_existence`, and `list_page_walks_forward_and_backward_over_twenty_five_intents` in `vpay-db`). **✅ cancel**: a compare-and-swap carrying *two* guards — the expected status and `NOT EXISTS` a live charge — so an intent whose confirm already handed a charge to a rail cannot be cancelled out from under the payer; the ambiguous zero-row answer is disambiguated by one re-read into a 404 or one of two distinct 409s (`cancel_is_legal_only_from_requires_payment_method`, `a_confirmed_intent_cannot_be_canceled`, `cancel_refuses_an_intent_with_a_live_charge_and_allows_one_with_a_terminal_charge` in `vpay-db`). **🟡 confirm — see the "Charge submission" row directly below, which is where this pass's work landed.** A second confirm is a `409` before any insert, and the `one_charge_per_intent` unique index catches the race the read cannot (`a_second_confirm_cannot_produce_a_second_charge`, `a_second_charge_for_one_intent_is_refused_as_a_named_unique_violation` in `vpay-db`). The rail is chosen by `payment_method_data[type]` and branched on by `Capabilities::flow` only, never by code (ADR-0002) — `confirm_legality_does_not_depend_on_the_rails_flow`, `an_intent_only_allows_the_rails_it_was_created_with`. **Still 🟡, and the reason moved again on 2026-09-03 (Step 4):** the intent now does reach `succeeded` — the worker's poll job drives it there, and back to `requires_payment_method` on a post-submission decline (Settlement row) — so what keeps this row 🟡 is narrower: `next_action` is only ever a `redirect_to_url`, `amount_received` is written but not rendered on the object, and every settlement so far was a WireMock rail answering. The integration suite is now **17 tests** (`payment_intents.rs`), all passing against a real Postgres on 2026-09-03. **Updated 2026-09-03 (Step 7): those 17 now have CI evidence and not only an authoring machine's.** `master` run **`33792230584`** (`ci`, commit `ca94eac`) is green on all six jobs including `rust`, which runs `cargo nextest run --workspace` with a working Docker daemon — so `payment_intents.rs` ran there against a real `postgres:16-alpine`. Verified with `gh run view`, not taken on report. **The row stays 🟡, and a CI run was never what was holding it:** `next_action` is only ever a `redirect_to_url`, `amount_received` is written but not rendered on the object, and every settlement in this repository's history was a WireMock rail answering |
| Charge submission (`confirm` → rail) — `vpay_api::v1::payment_intents::confirm` | 🟡 | **New 2026-09-03 (Step 3). `confirm` reaches a rail over HTTP and moves the intent.** Evidence is `backends/tests/integration/tests/confirm_rails.rs` (7 tests, Postgres **and** WireMock containers), measured passing on 2026-09-03. The write-first ordering [docs/flows/crash-safety.md](flows/crash-safety.md) requires is unchanged — mint the reference, **commit** the charge in `submitting` (with the merchant's `return_url` on a redirect rail), insert a `provider_requests` row with no status, `await` `adapter.submit(..)`, record what came back — and step 6 now has four shapes, chosen by the *error's* own classification rather than by anything the handler knows about rails: **(1) push accepted** → charge `submitted`, intent **`processing`**, `200`, `next_action: null` (`a_push_confirm_the_rail_accepts_moves_the_intent_to_processing`); **(2) redirect accepted** → charge `submitted` with the rail's `pay_token` and `redirect_url`, intent **`requires_action`**, `200` with `next_action.redirect_to_url`. **The commit is the gate on the redirect, by construction:** the rail's material, the `return_url` and both statuses move in **one** transaction, and `next_action` is then built **only from the committed charge row**, never from the adapter's return value — so no code path can emit a URL the database does not already hold (`redirect_confirm_commits_the_rails_material_before_it_answers` asserts the URL handed to the merchant equals the one on the row). **(3) declined at submit** (`ProviderError::Rejected`) → charge `failed` with its `failure_code` + `failure_raw`, intent **keeps `requires_payment_method`** now carrying `last_payment_error` (the public message; the rail's raw words are stored and logged, never sent), merchant gets `409 charge_declined`. Two cases, on purpose: `a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` steers the rail's refusal with the one field of the outgoing request a merchant controls (the MSISDN) and lands `invalid_payer`, and `credentials_the_rail_refuses_are_a_page_and_a_terminal_charge` lands `provider_account_blocked` — a different code, a different severity and a different on-call answer, asserted separately so the two cannot silently become one test. **(4) transport / malformed / misconfigured** → **nothing moves**: the charge stays `submitting`, the attempt keeps `status_code IS NULL`, and the merchant gets a `502`. Retrying under the same idempotency key then meets that live charge and is refused with a `409` whose text is **"A charge for this PaymentIntent is being resolved with the rail; poll GET /v1/payment_intents/{id} — do not create a new PaymentIntent."** — the advice follows `ChargeState::is_live`, and only a *terminal* charge gets the old "create a new payment intent" wording. That distinction came out of the Step 3 security review: telling a merchant whose submit timed out to open a second intent is telling them to prompt the payer's handset twice (`an_unreachable_rail_leaves_the_charge_where_recovery_expects_it`). **Two refusals happen before any charge exists**, so the intent stays confirmable on another rail: an intent whose currency is not the rail's settlement currency is a `400` naming `payment_method_data[type]` (`a_rail_that_settles_in_another_currency_is_refused_before_any_charge`), and a `return_url` that is not `http(s)` or is over 2048 characters is a `400` naming `return_url` (`a_return_url_that_is_not_a_bounded_web_url_is_refused_before_any_charge`) — the same bound the `charges.return_url` CHECK carries (migration `0019`), so a merchant gets a named `400` rather than the `503` an unguarded constraint violation becomes. **A third refusal joined them on 2026-09-05, and it is not about the rail at all:** an intent whose **newest** Checkout Session is not `open` is refused with `409 checkout_session_expired` — or `409 checkout_session_complete` — before any charge exists, on this route *and* on `POST /v1/browser/payment_intents/{id}/confirm`, because the gate lives in the `confirm_once` the two share (`vpay_api::v1::return_trip::SessionGate::admit_confirm`, after `load_confirmable_intent` and before `open_attempt`). See the Checkout Sessions row for what it reads and what it deliberately does not write. **`provider_requests.status_code = 0` is a sentinel, documented by migration `0020`:** it means the rail *answered* but the port carries no HTTP status (`Submitted`, `Rejected`). `NULL` keeps its meaning — no answer received — paired with `NULL responded_at` by the `response_is_paired` CHECK, and that pair is what the recovery table reads. `error_kind` is `Classify::code(&ProviderError)`, so it is the same vocabulary the merchant's envelope uses (`the_recorded_error_kind_is_the_errors_own_code`). **Not done, plainly:** every rail response above came from a **WireMock** container, never from MTN or Orange; and ~~there is still no callback route~~ — retired 2026-09-04 (Step 8, lane C): `POST /provider/{code}/callback` exists (its own row below), and a callback only pulls this charge's poll forward; settlement still comes from the worker's authenticated `query_status`. **Two items here were closed on 2026-09-03 (Step 4)** and are struck rather than deleted, because what replaced them is narrower: ~~nothing reads the rows a lost submit leaves~~ — the recovery table now reads them (`vpay_worker::recovery`, Worker job loop row), and the confirm additionally commits the `poll_charge` job **in the charge's own transaction**, so all three crash-safety kill points leave work behind; ~~a push confirm ends at `processing` forever~~ — the worker drives it to `succeeded`, `failed` or `unresolved`. **Updated 2026-09-03 (Step 7): the seven `confirm_rails.rs` tests now have CI evidence.** `master` run **`33792230584`** is green on all six jobs including `rust`, which starts the WireMock and Postgres containers through testcontainers — so these ran on a GitHub runner, not only on an authoring machine. Verified with `gh run view`. **The row stays 🟡 for the reason it always said:** every rail response above came from a `wiremock/wiremock` container, never from MTN or Orange — and, since 2026-09-04, no longer because a callback route is missing, but because the route that exists has never been called by a real rail |
| The payer's return trip through the port — `vpay_provider::ChargeRef::return_url`, `vpay_api::v1::return_trip` | 🟡 | **New 2026-09-04 (Step 9, lane 2).** The provider port carries a **per-charge** `return_url` and `vpay-adapter-orange-money` sends it as both `return_url` and `cancel_url`. Until this landed the adapter answered that question out of **deployment** settings, falling back to the notification endpoint — so the merchant's own `return_url` was validated, written to `charges.return_url`, echoed back as `next_action.redirect_to_url.return_url`, and **never sent to the rail that would act on it**, while every conformance and integration case passed. `mtn_momo` ignores it: a push rail has no browser. **Proven:** `the_submit_tells_the_rail_where_to_send_the_payer_back` (conformance, ×2 rails — Orange's exact value pinned over WireMock's request journal, MTN's *absence* asserted against the stub's whole journal, body, headers and query alike, with the push charge deliberately carrying a URL so the case is not vacuous); `a_direct_confirm_sends_the_merchants_return_url_to_the_rail` and `the_stub_hosted_page_links_to_the_return_url_the_submit_carried` (`backends/tests/integration/tests/confirm_rails.rs`, real Postgres + real WireMock, through the shipping confirm path). `webpayment.json` **requires** an `http(s)` `return_url` and `cancel_url` on every accepted submit — measured 2026-09-04: `#[serde(skip)]` on those two fields makes the stub answer 404 and the Orange conformance case fail on `ProviderError::Config`; restored. The matcher is a URL and deliberately **not** a prefix under vpay's own origin, because for a direct confirm the correct value is the merchant's own site. **Closed 2026-09-04 (Step 9, lane 1b):** the session branch is wired. `SessionReturnPage` holds the repositories *and* `checkout.public_base_url`, and a charge driven by an open session is submitted with `{checkout.public_base_url}/c/{cs_id}/return?t={return_token}&key={pk}` — the URL `CheckoutSessionRow::return_page_url` builds, byte for byte, proven over the Orange stub's request journal by `a_session_driven_confirm_sends_vpays_return_page_to_the_rail` (which also asserts the merchant's own `return_url`, sent on the same confirm, is **not** what reached the rail). The value is written into `charges.return_url` **before** the charge is committed and read back from that row at submit, so what the rail is told, what `next_action.redirect_to_url.return_url` renders on every later read, and what the worker would resubmit under are one column. A session on a deployment with no `checkout.public_base_url` is refused (`checkout_not_configured`) rather than silently sent to the merchant's URL — unreachable by any merchant request, since `create` refuses first, and proven by `a_session_driven_confirm_is_refused_when_the_checkout_app_is_gone` against a second server over the same database. Still 🟡, because no payer's browser has ever been redirected by a real rail: every assertion above is against a `wiremock/wiremock` container, never Orange |
| Checkout Sessions — `/v1/checkout/sessions` and `/v1/browser/checkout/*` (`vpay_api::v1::checkout_sessions`, `vpay_api::browser::checkout_sessions`) | 🟡 | **New 2026-09-04 (Step 9 lane 1).** A `checkout.session` (`cs_…`, migration `0028`) a merchant creates from its server against an existing `pi_…` — `create`/`retrieve`/`list`/`expire` on `/v1` (token-authenticated, `Idempotency-Key`, tenant-scoped), and three `GET`s on `/v1/browser/checkout` for the page. **Two payer credentials, not one** (D6): `client_secret` rides in the hosted `url`'s *fragment* and buys the intent's own `client_secret`; `return_token` rides in the return page's *query string* — it must, a fragment does not survive a rail's redirect — and buys the session and its intent **without** that credential. Every failure on both browser reads is the byte-identical uniform 404, including the tenancy case, and neither read renders the `url` (it carries the stronger credential in its fragment). `create` refuses an intent that is not `requires_payment_method`, one that already has a charge, and one that already has an open session — the last enforced by a **partial unique index**, not only by the pre-check. `expire` is a compare-and-swap with a `NOT EXISTS` live-charge guard in the same statement, so a session cannot be marked abandoned while a rail may still take the payment. The settlement transaction (`vpay_db::settlement`) flips `payment_status`/`status` in the **same commit** as the intent — `paid`/`complete` on success, `failed`/`expired` on a terminal decline. Proven by `backends/tests/integration/tests/checkout_sessions.rs` — **31 container-backed cases** against real Postgres, the real WireMock MTN rail and the shipping worker loop, with twelve recorded guard-failure proofs across lanes 1 and 1b (`docs/plans/step9-notes/lane-1.md` §5, `lane-1b.md` §5). Since **2026-09-04 (lane 1b)** the return trip is wired, sessions expire on their own, and both browser reads carry `merchant: { name }` — the member vpay's own checkout page reads when it is there. Two rules that were missing from those reads are now on the **read** itself and not on any sweep: past `expires_at` both answer the uniform 404 whatever the `status` (the `return_token` travels in a query string and therefore lands in a rail's logs, so the 24-hour horizon has to bound it), and the intent's `client_secret` is rendered only while `status = 'open'` (after settlement there is nothing left to confirm). A redirect confirm on an intent an open session drives no longer requires a `return_url` and ignores one that is sent — the page has none to send, and until this it answered `400`, so the hosted Orange flow could not complete at all. **Changed 2026-09-05: a session that is not `open` refuses the confirm.** Both `POST /v1/payment_intents/{id}/confirm` and `POST /v1/browser/payment_intents/{id}/confirm` ask `CheckoutSessions::find_latest_by_intent` — the **newest** session on the intent, whatever its status — exactly once, in the `confirm_once` the two surfaces share, and answer `409 checkout_session_expired` or `409 checkout_session_complete` (`ApiError::CheckoutSessionNotOpen`, classified under `Category::Conflict`; two codes rather than one, and both distinct from `invalid_state`, because "your payer walked away and we told you an hour ago" and "this intent is already processing" need different handling). The refusal is taken **after** the tenant-scoped intent read and **before** `open_attempt`, so it is not an id oracle and it writes nothing — no charge, no `provider_requests` row, no job. An `open` session past its `expires_at` that no sweep has reached is read as expired **by the read**, which writes nothing back: the same rule the browser reads already carry, for the same reason — a deployment whose worker is down must not decide whether a payer can pay, and a confirm is the wrong place to repair a row (it would emit no `checkout.session.expired` and would skip the sweep's `NOT EXISTS` live-charge guard). The **newest** row decides, so the ordinary "expire the abandoned checkout, offer a fresh link" flow still pays. `checkout_session_complete` is **defence in depth and unreachable through the shipping API today** — `complete` is written only by the settlement transaction, in the same commit as the intent reaching `succeeded`, which `load_confirmable_intent` refuses one step earlier — and its case stages the row with an `UPDATE` and says so in its own doc comment. Migration `0030` adds `checkout_sessions_intent_seq_idx` on `(payment_intent_id, seq DESC)`, **total** rather than partial: the query carries no `status` predicate, so `0028`'s partial `checkout_sessions_open_by_intent_idx` cannot serve it, and without an index the plan measured at 200,000 rows was a parallel sequential scan on **every** confirm. `postgres_smoke::the_confirm_paths_session_lookup_is_served_by_an_index` asserts the plan rather than a timing. Seven new cases here and one in `postgres_smoke` — 17 → **31** in this file. Still 🟡 and not ✅: **no real rail has taken a payment through a session** — every session ever driven end to end (lane 6's Cypress specs) settled against a WireMock host, and nothing proves the refusal in a browser: `frontends/apps/checkout` already paints an expired screen from the session read's 404, so no Cypress spec drives a confirm on a dead session and none was added |
| Checkout session expiry sweep (`vpay_db::CheckoutSessions::{due_for_expiry,expire_due}`, `vpay_worker::handlers::sweep_expired`) | ✅ | **New 2026-09-04 (Step 9, lane 1b).** `checkout_sessions.expires_at` was written at create (24 h, D10) and read by nothing; a session past its horizon reported `status: open` until a merchant expired it by hand or the intent settled. The worker's existing hourly housekeeping job now expires them — `open` and past `expires_at` and **no live charge** → `expired`, `payment_status` untouched — and logs its count as `checkout_sessions` beside `idempotency_keys`, `client_assertion_jtis` and `expired_leases`. **No new `jobs.kind`**: it is the same schedule as the other three, and a fifth kind would have said nothing this one does not. The live-charge guard is a `NOT EXISTS` inside the `UPDATE`, over the same `LIVE_CHARGE_STATES` `expire` and `cancel` use, because a session whose payer confirmed seconds before the horizon has a rail holding a live payment and a background job that expired it would be contradicted by the settlement transaction minutes later. Proven by `the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one` (`backends/tests/integration/tests/checkout_sessions.rs`): two sessions created through `POST /v1/checkout/sessions`, one with a live charge opened through the browser confirm the page uses, both moved past their horizon, swept by the shipping `vpay_worker::run_once` over the shipping `seed_singletons` job. Measured 2026-09-04: deleting the `NOT EXISTS` clause expires the paying session. **Changed 2026-09-04: it emits an event, and it is no longer one statement** — see the `checkout.session.expired` row below. Because the event's `data` is the rendered wire object, which only `vpay-api` can shape, the sweep now reads a page (`due_for_expiry(now, limit)`, `EXPIRY_PAGE` = 100, ordered by `expires_at`), renders each session, and runs one transaction per session (`expire_due(id, now, event_id, event_data)`); a full page that moved something reschedules the sweep immediately rather than in an hour, the device `webhooks::handle_fan_out` already used. One session's failure is a `WARN` naming it and its merchant and no credential, and the pass moves on — that session stays `open` and the next pass retries it; only a failure to read the page is a `JobError`. **`due_for_expiry` sorts without an index for it** — `0028` gives `checkout_sessions` no index on `expires_at`, so the planner restricts to the open set and then sorts all of it to take a hundred, once an hour, where the bulk `UPDATE` this replaced never sorted. The open set is bounded by a day's creates, so this is a cost rather than a defect, and **nothing measured it**; a partial index on `(expires_at) WHERE status = 'open'` is the obvious answer and is **left to the maintainer**, because adding an index to a payments table on reasoning alone is exactly the plausible addition `CLAUDE.md` warns about. The live-charge guard is now evaluated **twice**, once in each half, and both copies are pinned by their own case (`a_payer_confirming_between_the_read_and_the_write_keeps_the_session` for the write's, an assertion inside `a_session_with_a_live_charge_is_neither_expired_nor_evented` for the read's) — the 2026-09-04 review measured that deleting either one on its own left all 23 cases in the file green. **What it is not:** the sweep is not what refuses an expired session's payer credential — the browser reads check `expires_at` themselves, because the sweep leaves live-charge sessions `open` on purpose and a deployment whose worker is down must not keep answering. **A named limitation this change makes louder, pre-existing and filed separately:** `POST /v1/browser/payment_intents/{id}/confirm` authenticates on the *intent's* `client_secret` and consults no session's `expires_at` — only the session reads do (`vpay_api::browser::checkout_sessions`, `checkout_sessions.rs:278`). So a payer holding a page loaded before the horizon can confirm *after* the sweep has expired the session: the charge succeeds, `settle_for_intent`'s `WHERE status = 'open'` no longer matches, and the session sits `expired`/`unpaid` under a `succeeded` intent. Step 9 created that desync; this change turns it into an outbound `checkout.session.expired` the merchant then has to reconcile. ~~**Not fixed here** — both candidate fixes (refuse the confirm, or widen the settlement's guard so a swept session can still be completed) are behaviour changes reserved for the maintainer~~ — **fixed 2026-09-05** by the first of those two: the confirm now reads the intent's newest session and answers a `409` when it is not `open` (Checkout Sessions row above). **The choice between them was recorded here as reserved for the maintainer, and the maintainer did not make it.** It was **chosen by the integrator on 2026-09-05**, on the ground that widening the settlement's guard would let a session whose merchant has already been sent `checkout.session.expired` turn `paid` afterwards — which leaves the event a notification the settlement can contradict rather than a promise. That is a reason, not an authority: the decision is stated in the pull request that lands this so the maintainer can veto it, and the other fix is still available — reverting to it is this gate plus `settle_for_intent`'s `WHERE status = 'open'`, and nothing else depends on which one is chosen |
| `checkout.session.expired` (migration `0029`, `vpay_db::CheckoutSessions::expire_due`) | 🟡 | **New 2026-09-04.** The eighth documented event type, and the first whose `data.object` is neither a `payment_intent` nor a `refund`. Migration `0029` reopens `type_is_a_documented_event` for it; it is Stripe's own spelling, so [flows/webhooks.md](flows/webhooks.md)'s "only real Stripe event types" rule holds and a Stripe-shaped handler already has a branch. **Written inside the same transaction as the `open` → `expired` compare-and-swap** — the settlement's shape, and the argument is sharper here: a session that says `expired` with no event is invisible, because no sweep looks for one, no fan-out backlog names it, and the merchant simply never hears. `a_failed_event_insert_leaves_the_session_open` proves the rollback against a real `data_is_object` CHECK violation, and the reverse was measured on 2026-09-04: committing the flip before the insert makes it fail with the session `expired`. `data.object` is the **thirteen documented keys** rendered by `vpay_api::model::CheckoutSessionObject::expired_snapshot` — `status` already `expired`, `payment_status` untouched, and **`url: null`**, because a hosted session's `url` carries its `client_secret` in the fragment (D6) and an event body is stored, signed, delivered at-least-once and replayed; `client_secret` is absent entirely and `return_token` is on no wire object at all. Both the unit case and the integration case assert that on the **serialised string**, not the parsed object. From there it is an ordinary event: one `webhook_deliveries` row and one `deliver_webhook` job per configured endpoint, the same signing, the same egress guard, the same seven-rung ladder, and readable at `GET /v1/events` and `GET /v1/events/{id}` merchant-scoped. Seven container-backed cases in `backends/tests/integration/tests/checkout_sessions.rs`: `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint`, `a_second_sweep_writes_no_second_expiry_event`, `a_session_with_a_live_charge_is_neither_expired_nor_evented`, `a_session_finished_by_settlement_emits_no_expiry_event`, `an_expiry_event_is_listable_and_retrievable_within_its_tenant`, `a_failed_event_insert_leaves_the_session_open` — all six driving the shipping `vpay_worker::run_once` over the shipping `seed_singletons` — and, added by the 2026-09-04 review, `a_payer_confirming_between_the_read_and_the_write_keeps_the_session`, which stages the window between `due_for_expiry` and `expire_due` and proves the live-charge `NOT EXISTS` in the *write* is load-bearing: without it all 23 cases in the file were green. Measured 2026-09-04: deleting the event insert makes the first fail while the session still expires. Both merchant SDKs carry the type — `vpay_sdk::KnownEventType::CheckoutSessionExpired` with `Event::checkout_session()`, `@vaam-apps/vpay-sdk`'s `KnownEventType` with `isCheckoutSessionEvent` — and both keep `type` a plain string, so an event type either predates is still deliverable ([sdks/parity.md](sdks/parity.md)). **🟡, for three reasons.** (1) **No merchant endpoint has ever received one.** The endpoints in the proving case are URLs nothing resolves, because what it asserts is what the fan-out *created*; the delivery half is the same code every other event walks and has been observed against a WireMock receiver, but not for this type. (2) **A merchant expiring its own session through `POST /v1/checkout/sessions/{id}/expire` emits nothing.** The caller already knows — but a merchant with several services does not necessarily, and Stripe emits it for both paths. Deliberate and scoped to the sweep, which is the path nobody is watching; **left to the maintainer**, because "one transition, one event" is a contract merchants build dedupe logic on and widening it later is cheaper than narrowing it. (3) **No `checkout.session.completed`**, and there should not be one: a session reaching `complete` already produces `payment_intent.succeeded` from the same commit |
| Customers — `/v1/customers` (`vpay_api::v1::customers`, `vpay_db::customers`, migration `0034`) | ✅ | **New 2026-09-06 (S4a).** `cus_…` — the merchant-owned record of a payer, five routes (`create`/`retrieve`/`update`/`list`/`del`), both SDKs, and a twelve-month retention sweep that is **built and runs**, not `NotImplemented`. [flows/customers.md](flows/customers.md) is the whole document; the load-bearing facts: the maintainer's three decisions of 2026-09-05 are in the schema and in tests (`phone` stored by default with no opt-in; **a phone number alone is a complete customer** — `at_least_one_identifier` requires *one* of name/email/phone, proven in both directions against a real Postgres; retention is twelve months, `CUSTOMER_IDLE_AFTER`, deliberately not configurable because it is a promise to a payer). `phone` is canonicalised through the **same** `canonical_msisdn` the account-holder lookup uses, so vpay never holds two spellings of one payer. `DELETE` is a **hard** delete (no `deleted_at`, no `@@soft_delete` — pinned by a `preview_sql` test, because adding the attribute is a one-line schema edit that changes no Rust and would leave the API answering `{deleted: true}` for data it kept), and a customer any intent or session references **cannot** be deleted at all: the foreign keys are `NO ACTION`, turned into a `409` that says so. That is a stated trade — "delete this customer" is *not* a complete erasure of the payer, because the payment record survives; clearing the three identifiers with an update is the other half, which is why an update can clear a field and why clearing the *last* one is a `400` decided above the statement (the database's own CHECK is a `23514` → `Category::Storage` → a `503` telling a merchant to wait for a healthy database). `customer` on `POST /v1/payment_intents` **stopped being accepted-and-dropped** in the same migration and is stored and rendered; the intent object went 12 keys → 13. Proven by **15 container-backed cases** in `backends/tests/integration/tests/customers.rs` (the 44th test binary) driving the shipping router, the shipping SDK and the shipping worker loop, 3 in `postgres_smoke.rs` at the schema, 7 in `vpay-db` with no container, 2 in `vpay_api::model`, 8 in `sdks/rust` and 8 in `sdks/nodejs`. **✅ rather than 🟡, and the bar it is measured against:** every claim in this row fails a named test if it breaks, including the sweep's three cases (deleted / kept-because-recent / kept-because-referenced) and the confirm-path retention stamp. **Five of those cases were added by the sabotage review on 2026-09-07 and each closed a measured hole**, which is why the count moved after the row was first written: the object's key set was guarded by nothing (`the_customer_object_is_the_documented_eight_keys` — rendering the internal `last_used_at` on the wire was green across `vpay-api`, `vpay-sdk`, the integration suite and the Node SDK, and two documents named a tripwire of that name that did not exist; the count in them was also wrong, seven for eight); the list's tenancy and its **cursor** were guarded by nothing (`a_list_is_tenant_scoped_and_a_foreign_cursor_pages_nothing` — dropping the tenant filter from `list_page`'s cursor subquery passed all twelve delivered cases, and the forged cursor then pages another merchant's rows); and `POST /v1/customers`'s idempotency was guarded by nothing (`a_replayed_create_answers_the_stored_customer_and_a_reused_key_is_refused` — releasing the key instead of storing the response passed all twelve, and a retried create then mints a second `cus_…` for one payer). A fifth case, `a_sessions_customer_is_inherited_supplied_or_a_refused_contradiction`, covers the **checkout session** half, which S4a documented in four sentences and tested in none: inherited from the intent, supplied directly, refused when the two disagree, the same uniform `400` for another merchant's `cus_…`, and a customer that only a *session* references pinned against `DELETE` — deleting the contradiction check left the entire thirty-one-case `checkout_sessions.rs` suite green. **A gap that review also found and did not close: neither SDK models a session's `customer` in either direction** though the server accepts and renders it — dated ⛔/⛔ rows in [sdks/parity.md](sdks/parity.md), owned by the SDK maintainers. What it does **not** claim, and the flow doc lists in full: no `address` on the object; `customer.created`/`customer.updated` are not in the event vocabulary because nothing writes them; no `email` filter on the list; neither SDK has run this resource against a live vpay; and **no deployment has ever run the retention sweep**, because no vpay has been up for twelve months |
| Account-holder lookup — `GET /v1/account_holders` (`vpay_api::v1::account_holders`), `ProviderAdapter::account_holder_name`, `Capabilities::supports_account_holder_lookup` | 🟡 | **New 2026-09-05 (issue #47).** A merchant-facing, **stateless** read of a rail: `?msisdn=…&payment_method_type=…` → `{ object: "account_holder", payment_method_type, name: "…"\|null, verified }`. The distinction it exists for is that `name: null` means **the rail answered and has no record**, while a rail that could not be *asked* is a classified error (502/500) and never a `200` with nulls — `a_rail_that_cannot_be_reached_is_a_502_and_never_a_null_name` and the conformance suite's `a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder` are what hold that. Refused on the **capability value** and never on a rail code (ADR-0002): an unknown rail, a *disabled* rail and an incapable rail all get the byte-identical `400` naming `payment_method_type`. `msisdn` is validated server-side against Cameroon E.164 — **the first server-side copy of that rule**; `frontends/apps/checkout/src/lib/msisdn.ts` had been the only one, and a page can be bypassed. **Nothing is persisted** — not the name, not the number, not the fact that the question was asked — and the one log line carries `+2376••••200` and no name (`a_lookup_logs_a_masked_number_and_never_a_name`, against captured `tracing` output). `vpay_account_holder_lookups_total{outcome}` counts all four outcomes and **no label carries the number or the name**. **🟡 and not ✅ for two reasons, both of them about what has not happened:** no rail but a WireMock container has ever answered this call (see the Adapters section), and issue #47 §3's rate limit, audit log and dedicated scope are **not built** — they are reserved decisions recorded in [flows/account-holder-lookup.md](flows/account-holder-lookup.md), which means a merchant credential can today turn a list of phone numbers into a list of names at whatever rate it can make requests. Proven by 6 container-backed cases in `backends/tests/integration/tests/account_holders.rs`, 13 unit cases in `vpay_api::v1::account_holders`, and 10 conformance cases (5 parameterised over both rails) |
| Rail callbacks — `POST /provider/{code}/callback` (`vpay_api::provider_callback`) | 🟡 | **New 2026-09-04 (Step 8, lane C); was ⛔ "no callback route exists".** The route is mounted, unauthenticated by necessity (neither rail signs a callback or sends a shared secret), and it **never writes charge or intent state**: it resolves the adapter by `code`, `parse_callback`s the body into identifiers, finds the charge with `Charges::get_by_provider_reference` (scoped by rail, served by migration `0027`'s index), and pulls that charge's poll job forward. ~~It performs exactly one write.~~ **Corrected 2026-09-04: two statements, in one transaction** (`provider_callback.rs:286-305`) — `TxRepositories::enqueue_in_tx` (`ON CONFLICT (dedupe_key) DO NOTHING`, a no-op in the ordinary case, but it inserts a fresh poll job when the charge's job was deleted or already finished) and then `TxRepositories::pull_forward_in_tx`, an `UPDATE jobs SET run_at = now()` on that `poll:<charge id>` job, refusing a leased, dead-lettered, already-due **or already-due-within-ten-seconds** one. **What bounds an anonymous caller is the `dedupe_key`'s unique index and `poll_charge`'s terminal guard (`vpay-worker/src/handlers.rs:239`), not the absence of an insert.** Four answers: unknown rail code → `404` byte-identical to the router's fallback; unparseable body → `400` + a `warn`; unknown reference → `202` (a rail must not be told to retry forever, and a different answer would be an oracle); accepted → `202`. Body bounded at 16 KiB by `RequestBodyLimitLayer`, `track_http_metrics` on the nest, its own `.fallback`, no CORS, `request-id` on the way out like every other route. `X-Callback-Url` (MTN) and `notif_url` (Orange) were **already** being sent — `ProviderHost::effective_callback_url` has derived `{public_base_url}/provider/{code}/callback` since Step 3 — and are now *asserted* to be sent, by `the_submit_tells_the_rail_where_to_call_back` once per rail, with the shared WireMock mappings refusing a submit that omits them. Proven by `backends/tests/integration/tests/provider_callback.rs` — 9 container-backed cases over both rails, 11 since Step 8's review, including the headline one: the poll job is parked at a later rung, `run_once` then finds **nothing runnable**, the rail's documented body is POSTed to the URL read back off the rail's own WireMock journal, and the next `run_once` settles the charge — and its sibling `a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run`, which is the new bound: a poll already due within `PULL_FORWARD_FLOOR` (ten seconds, the ladder's fastest rung) is left exactly where the ladder put it, and the rail is still answered `202`. Ten of the eleven are container-backed; `the_pull_forward_floor_is_the_poll_ladders_first_rung` is not, because it is the join between `vpay_api::provider_callback::PULL_FORWARD_FLOOR` and `vpay_worker::poll_delay(0)` and needs only both crates linked. Still 🟡 and not ✅ for two reasons: **no real rail has ever called it** (the bodies are transcribed from `docs/flows/adapter-*.md`, so a document that is wrong about the rail would pass), and Orange's `notif_token` is **not** compared against the stored one — see the row below. A `GET` on the path is axum's bare `405`, not the Stripe envelope; that is left as-is, following the precedent `docs/reference/vpay-api.md` records for `GET /v1/oauth/token`. **What it also does, and nothing said until 2026-09-04 (Step 8 review, finding 4): it is the first unauthenticated route this repo publishes to a network** — `compose.demo.yml` maps the whole `vpay-server` port to the host (`${VPAY_DEMO_PORT:-8080}:8080`), ~~bound on `0.0.0.0`, so anyone on the same LAN as a demo or e2e stack can post a rail notification at it without a credential~~ — **narrowed 2026-09-04 (Step 9, lane r2): every publication in `compose.demo.yml` is now bound to `127.0.0.1`, measured against a LAN address rather than reasoned about. `compose.e2e.yml` was deliberately left on `0.0.0.0`, so a CI or bare-e2e stack is still LAN-reachable, and none of what follows about the route itself changes: a bind address is not an authorisation.** **What that buys them was overstated and is now measured.** The module used to claim it was "bounded by what the ladder was going to do anyway"; it was not, and since 2026-09-04 (lane H) the pull-forward refuses a job due within the ladder's own fastest rung, so a POST about a charge the queue was about to ask about anyway changes no row and causes no rail request at all. Past that rung it is **not** bounded: the ladder's rungs grow (20 s, 30 s, 45 s …) while the floor stays at ten, so a caller repeating against one live charge can hold it at roughly one authenticated `query_status` per worker claim. **There is no rate limit, per charge or per source.** What is left standing is that the caller must know a v4 `provider_reference_id` for a live charge *on this deployment*, that each accepted POST buys exactly one authenticated status query — which settles the charge the rail actually names or nothing at all — that the body is bounded at 16 KiB, and that no charge or intent state is ever written ([flows/provider-port.md](flows/provider-port.md): the route is a hint that never moves state). It remains unauthenticated write access to a job's `run_at`, and **a real deployment must front this path** (rate limit, IP allowlist, or a reverse proxy) rather than publish it as the demo does. The floor also has a cost, stated: a rail calling back while the charge sits on the ladder's first rung no longer settles it early — it settles at that rung, up to ten seconds later than before |
| Orange `notif_token` verification, and `ref_extra` repair from a callback | ⛔ | **Still not built, and now explicitly so (2026-09-04, Step 8, lane C).** `vpay_adapter_orange_money::Adapter::parse_callback` returns the received `notif_token` in `CallbackRef::ref_extra` and fails closed when there is none, and `docs/flows/adapter-orange-money.md` names comparing it against the stored one as "the callback route's job". The route **discards `ref_extra` entirely** rather than doing half of it: merging rail key material taken from an unauthenticated request onto the charge would corrupt the `pay_token` the next status query is addressed by, and the comparison that would make it safe needs a stored-token read nothing implements. `a_callback_writes_no_charge_or_intent_state` in `backends/tests/integration/tests/provider_callback.rs` holds it there, on both rails, with a body carrying tokens no rail ever issued. Repairing a lost `ref_extra` write from a callback therefore remains unavailable; the crash-safety answer for that case is unchanged (`ProviderError::Config`, a human) |
| Confirm vs. the worker's first poll (`write_matched_no_row`) | ✅ | **Found 2026-09-04 by Step 8's demo, fixed the same day (lane G).** `insert_charge` commits the `submitting` charge and its `poll_charge` job in one transaction with `run_at = now()` (`vpay-api/src/v1/payment_intents.rs:1506`), and the worker may claim that job before the confirm finishes its own `submitting → submitted` compare-and-swap (`vpay-db/src/charges.rs:313`; `IDLE_SLEEP` is 1 s, `vpay-worker/src/run_loop.rs:52`). It then applied the **crash-recovery** table to a charge whose process had not crashed, and either branch moved the charge out from under the confirm — observed four times in six walkthrough runs on a loaded machine (confirm latency 3.7 s): on a push rail the merchant was told the confirm failed and was then delivered a `payment_intent.succeeded` webhook; on a redirect rail `FailDeadOrder` killed a live order as `provider_unavailable` while the confirm held the very redirect URL its `failure_raw` said the payer had never been given. **The fix is a minimum charge age**: `recovery_step` answers `RecoveryAction::Wait` — write nothing, ask nothing, come back once when the charge is old enough — for any `submitting` charge younger than `RecoveryPolicy::not_found_window` (60 s, three times the 20 s rail request timeout), measured from `charges.created_at` because the `SubmitAttempt::Never` branch has no `provider_requests` row to measure from. One predicate in the pure function, reached by both callers through the `recovery_action` helper they already shared. **Two defects in that first shape were found by Step 8's own correctness review and fixed the same day (lane H).** The age was `OffsetDateTime::now_utc() - charges.created_at` — the worker *host's* clock minus Postgres' — so a worker sixty seconds fast measured every charge as a minute older than it was and the guard became a silent no-op, exactly on the deployment whose fleet clocks had drifted; injecting `+61 s` at the old site failed `a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window` with the identical line deleting the guard produces (`left: Finished  right: Rescheduled(10s)`). The age now comes from `Charges::get_by_id_as_of`, which selects `now()` on the same statement that reads the row, and `recovery_step` takes durations rather than instants so no caller can supply the wrong clock; with the fix, +61 s of host skew injected into every remaining host-clock read in `handlers.rs` leaves that case passing. `past_the_horizon` took the same subtraction and now takes the same age. And `Wait` rescheduled at `poll_delay(0)`: every reschedule is re-claimed, `Jobs::claim` increments `attempts`, and `poll_delay` is indexed by it, so a genuinely crashed charge burned six rungs waiting the window out and started its real recovery at `poll_delay(6)`. `Wait` now carries `window - age` (clamped into `[0, window]`) and reschedules **once**, so the wait costs one claim and the first real rung after a crash is `poll_delay(1)`, twenty seconds. **The cost is stated:** a charge orphaned by a genuine crash waits up to a minute for its first recovery pass, and that wait costs it one rung of the poll ladder rather than six; it stays live and queued throughout, and 60 s is not the 24-hour horizon. Proven by a unit table over all five branches at four ages, and by three cases in `worker_recovery.rs` — the young push charge is not advanced and the aged one is, the young redirect charge is not dead-lettered and the aged one is, and `a_confirms_compare_and_swap_wins_against_the_worker_that_claimed_its_poll_job` runs the shipping `run_loop` against the confirm's own `mark_submitted` and asserts the confirm wins. Deleting the guard fails all three, the last with the merchant's own error text. **What is not proven end to end:** the HTTP confirm handler is not in that suite, so the racing write is `persist_submitted`'s compare-and-swap called directly. **The guard has one measured consequence beyond its own suite**, recorded here rather than left to be rediscovered: it made lane D's `a_server_killed_mid_submit_…` fail on the gate branch (TRY 3 FAIL, 63 s) — the killed server's charge is `submitting` and younger than 60 s, so the worker correctly waited — and `worker_kill9.rs` gained `age_the_crashed_charge`, a second simulated clock beside `age_the_dead_workers_lease`. **The demo has not closed that loop, and this row does not rest on it — it rests on lane G's tests.** ~~The `just demo` re-run that would close the loop end to end went green on lane A's rebased branch.~~ **Corrected 2026-09-04:** one green run from nothing exists (lane A's rebased branch, 2026-09-04, **without** lane G — it was rebased onto `068d8b7`, master plus lanes B and D, and lane G merged later as `53f7a7e`; the race is timing-dependent and did not fire), lane A's own earlier count was two greens in six attempts and zero for three from nothing, lane G did not re-run the demo. Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in): `just demo` from nothing six times, **four green** (six outcomes for six each, exit 0); the two failures were the VM's Postgres answering single statements in 14–36 s under host I/O pressure, with the settlement and the webhook both landing in the worker's log after the demo's budgets; `write_matched_no_row` appeared in no run. Three from nothing is met in count, not consecutively. |
| Merchant OP (`/v1/oauth`) — `vpay_api::op` | 🟡 | New 2026-09-02. `MerchantOp` (`op/mod.rs`) assembles an `authkestra_op` provider whose issuer is `{deployment.public_base_url}/v1/oauth` (`issuer_for`, the single derivation in the workspace — `main` and `MerchantOp::new` both call it, so the `iss` a token is stamped with and the `iss` the validator pins cannot drift; `the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url` and `a_trailing_slash_on_the_public_base_url_does_not_change_the_issuer`). `grant_types_supported` is `["client_credentials"]` and nothing else. The store is a `CompositeOpStore` over `YamlClientStore` (row below), `SqlClientAssertionStore` for replay, and **three slots that serve no `/v1` grant** — they exist because `OpStore` is a supertrait of the code/refresh/device stores. **Changed 2026-09-05:** those three held `SqlxOpStore<Postgres>` until then and now hold `vpay_api::op::refusing_stores`' `Refusing{AuthorizationCode,RefreshToken,DeviceCode}Store`, twelve methods that all return `Err` naming the grant. That is **not** the "always empty" stub the previous sentence here ruled out and AGENTS.md rule 1 forbids: an empty store answers `Ok(None)`, which every `authkestra_op` handler reads as "your code was wrong" and would become a silent lie the day another grant is mounted; a refusing store cannot produce that answer, and the first request on a newly mounted grant fails loudly. Why they changed, and what it cost, is the "The three OP stores that pinned sqlx 0.8" section under Infrastructure. `op/token.rs` is thin: `token_handler` delegates to `authkestra_op::handlers::token::handle_token` and maps its RFC 6749 error JSON to a status (`invalid_client` → 401, everything else → 400); the discovery document is hand-built so it advertises only what this deployment serves (`discovery_publishes_the_endpoints_the_sdk_would_have_guessed`, `discovery_advertises_no_endpoint_this_deployment_does_not_serve`, `discovery_advertises_only_private_key_jwt`). **`authkestra-axum` is deliberately not a dependency** — its bundled router mounts `/authorize`, `/userinfo` and a one-key JWKS handler this deployment must not serve; see the JWKS row below. `op/jwks.rs` serves `/v1/oauth/jwks.json` from `vpay_db::publishable_signing_keys` (the whole rotation window, not just the active key), `Cache-Control: public, max-age=300` (`the_response_is_publicly_cacheable_for_the_documented_window`), skipping and warning about a row whose `public_jwk` is unusable rather than failing the whole document (`a_jwk_without_a_kid_is_skipped_and_warned_about`, `a_public_jwk_that_is_not_an_object_is_skipped_and_warned_about`, `a_kid_that_disagrees_with_its_row_is_published_but_warned_about`). 33 unit tests across `op::{mod,clients,keys,jwks,token}`. **Two numbers in here are defaults this pass chose, not decisions anyone recorded:** `ACCESS_TOKEN_TTL_SECS = 900` and `keys::ROTATION_OVERLAP = 24 h`. `docs/roadmap.md` lists both the access-token TTL and the rotation-overlap window as open questions and this pass does not close either; the only property that is *tested* is the relationship between them (`the_access_token_ttl_fits_inside_the_key_rotation_overlap`, `the_rotation_overlap_dwarfs_the_access_token_ttl_it_has_to_cover`), not that either value is right. **🟡 and not ✅** because the end-to-end proof — `the_jwks_and_discovery_documents_describe_this_process` and the rest of `merchant_token_flow` — has run exactly once, manually, against a scratch database, and never under Docker or in CI; see the header paragraph. **Not done here:** no rate limit on `/token` (left to ingress per [ADR-0009](adr/0009-dashboard-oidc-provider.md), and nothing in this repo checks that ingress actually does it), and no `/v1` resource for a minted token to reach.. **Known limitation, recorded not fixed:** no rate limit exists anywhere in this repository in front of `/v1/oauth/token` (one `disabled_clients` `SELECT` per request for any public `client_id`, before any signature check) — ADR-0009 leaves it to the ingress, which nothing here verifies. **Two changes 2026-09-03 (Step 2).** (1) **A token request that names no `scope` is now granted the client's registered `scopes:`** — RFC 6749 §3.3's "locally defined default", defined here as the registration itself. Both SDKs omit `scope`, so before this, every SDK-minted token carried none and would have been `403`ed by every `/v1` call; `handle_client_credentials` treats an absent `scope` as "grant none" and offers no seam, so the default is applied in `token_handler` before the grant runs. It only ever *fills in* an omitted value: a request naming a narrower scope keeps it, and anything outside the registration is still `invalid_scope` (`the_default_scope_is_the_clients_own_registration_and_nothing_wider`). An empty `scopes:` list is legal and means the client can mint a token and be `403`ed by everything, which is what `a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not` exercises over real HTTP. (2) **A minted token now has somewhere to go** — the "no `/v1` resource for a minted token to reach" clause above is no longer true; see the "Payment intents" row |
| Database schema / migrations (core) | ✅ | Five migrations exist in `backends/migrations/` (`0001_create-currencies.sql` … `0005_create-ledger.sql`), applied via `sqlx::migrate!` to a real `postgres:16-alpine` (testcontainers) and asserted against in `backends/tests/integration/tests/postgres_smoke.rs`: a clean migration run on an empty database, the `one_charge_per_intent` unique index, two cross-column `CHECK` constraints firing (`partial_refunds_imply_refunds` on `providers`, `no_over_refund` on `payment_intents`), a plain `amount >= 0` check, an FK violation, and an out-of-range currency exponent. Marked ✅ and not 🟡 because the claim this row makes — "the schema and migrations exist, apply cleanly, and their constraints actually fire" — is fully implemented and tested; a broken migration or a dead constraint would fail a real test. **This is narrower than "the database works."** No route reads or writes an application row through this schema yet — that gap is now tracked by "HTTP surface" and "Database connectivity" above (a connection pool and a migration runner now exist and are wired into both binaries, closing the exact gap this row used to describe — "there is no connection pool" is no longer true, see those rows for what is and is not proven), the same way "Provider port trait" being ✅ above does not imply the adapters' wire calls work. **This repository now has thirty-one migrations in total** (`0001`–`0031`; **`0031_refunds-fee` landed 2026-09-05 with [issue #46](https://github.com/vaam-apps/vpay/issues/46)** — one nullable `BIGINT` on `refunds`, with no `DEFAULT` and a `fee_non_negative` CHECK, behind the `refund` object's tenth field. Nullable and defaultless on purpose: `NULL` is "the rail reported no fee" and `0` is "the movement was free", and a `DEFAULT 0` would erase the distinction the field exists for. **Nothing writes it** — no rail reports a refund fee, so every stored value is `NULL` — but it *is* read, as of the rebase onto master on 2026-09-06: `vpay_db::refunds`' `COLUMNS` projection selects it and `GET /v1/refunds/{id}` renders it as the object's tenth key. Issue #45 landed the repository and the route while this branch was open; the branch's own claim that the column was unread was true when it was written and is not any more. Migration `0031`'s own header and `COMMENT ON COLUMN` were rewritten to say so — editable, unlike `0017`'s, because `0031` has never been applied anywhere: it is a new file on an open branch, so `sqlx::migrate!`'s checksum has nothing to disagree with yet. `postgres_smoke.rs`'s migration-count assertion moved 30 → 31 in the same commit, and two new cases there prove the column rather than the SQL merely parsing — `an_unreported_refund_fee_stays_null_and_never_becomes_zero` and `a_negative_refund_fee_is_rejected_by_the_database`. The `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` constant did **not** move and that is not an oversight: `refunds` is reported as one whole undeclared table, so a column added to it changes no count; measured 2026-09-05; **`0030_checkout-sessions-latest-by-intent-idx` landed 2026-09-05 with the session-driven confirm refusal** — a plain, total index on `checkout_sessions (payment_intent_id, seq DESC)` serving `CheckoutSessions::find_latest_by_intent`, which the confirm path asks once per confirm. Total rather than partial on purpose: the query carries no `status` predicate, so `0028`'s partial `checkout_sessions_open_by_intent_idx` cannot serve it and the plan was a parallel sequential scan — measured on 2026-09-05 at 200,000 sessions, `11.685 ms` against `0.047 ms` with the index. `postgres_smoke.rs`'s migration-count assertion moved 29 → 30 in the same commit, and `the_confirm_paths_session_lookup_is_served_by_an_index` asserts the plan rather than the timing; measured with the migration removed, both fail; **`0029_events-checkout-session-expired` landed 2026-09-04 with PR #31** — it reopens `events.type_is_a_documented_event` for `checkout.session.expired`; `postgres_smoke.rs`'s migration-count assertion moved 28 → 29 in the fix-forward that followed, because PR #31 was merged before its checks finished and neither its gate list nor its review ran `postgres_smoke` — master's first `rust` job after it failed on exactly that assertion; **`0028_create-checkout-sessions` landed 2026-09-04 with Step 9 (lane 1)** — the `checkout_sessions` table: three closed vocabularies (`ui_mode`, `status`, `payment_status`), a `urls_match_ui_mode` CHECK pinning "hosted has `success_url`+`cancel_url` and no `return_url`, embedded the reverse" in the schema and not only in the handler, both credential-length CHECKs mirroring `0019`/`0026`, a **partial unique index** (`checkout_sessions_one_open_per_intent`) so one intent can never have two open sessions and two live payer links, three read indexes, and a stored `publishable_key` column so a key rotation cannot strand a payer already on a rail's page. `postgres_smoke.rs`'s migration-count assertion moved 27 → 28 in the same commit and `checkout_sessions` joined its queryable-tables list; that binary is 14 run, 14 passed on this branch. Measured 2026-09-04: turning that unique index into a plain index lets a second open session insert; **`0027_charges-provider-reference-idx` landed 2026-09-04 with Step 8 (lane C)** — a plain, non-`UNIQUE` index on `charges (provider_code, provider_reference_id)` that serves `Charges::get_by_provider_reference`, the lookup behind the **unauthenticated** `POST /provider/{code}/callback`; without it that read is a sequential scan anyone who can reach the callback URL can provoke once per request. It is deliberately not `UNIQUE`: every insert path mints the reference with `Uuid::new_v4()` before committing, so "one charge per rail reference" looks true by construction, but it is a schema-level invariant this repository has never claimed and **the decision to assert it is left to the maintainer** — the lookup is written to be total without it (`ORDER BY created_at DESC, id DESC LIMIT 1`). `postgres_smoke.rs`'s migration-count assertion moved 26 → 27 in that commit; **`0026_payment-intent-client-secret` landed 2026-09-03 with Step 5c** — adds `payment_intents.client_secret_suffix TEXT NOT NULL`, backfilled for pre-existing rows from two concatenated `gen_random_uuid()` draws, with a `client_secret_suffix_length CHECK` between 32 and 128 characters; `vpay_core::ids::client_secret`/`client_secret_suffix` derive the full `pi_…_secret_…` credential from it and it is never stored twice. Proven applied against a real Postgres alongside every other Step 5c claim in `backends/tests/integration/tests/browser_checkout.rs` — see the Browser checkout surface row below; **`0025_idempotency-keys-retry-advice` landed 2026-09-03 with Step 5b** — `idempotency_keys.response_retry`, the column that lets a replayed response re-emit the `stripe-should-retry` the original carried instead of dropping it, with a `CHECK (response_retry IN ('true','false'))` proven firing by `the_retry_advisory_round_trips_and_0025_refuses_anything_else`; **three landed 2026-09-03 with Step 5**: `0022_create-webhook-deliveries` re-opens `jobs.kind_is_known` for `fan_out_events` and `deliver_webhook` and creates `webhook_deliveries`, both asserted against a real Postgres by `migration_0022_reopens_the_job_kinds_and_closes_the_delivery_states`; `0023_jobs-scan-deliveries` re-opens it once more for `scan_deliveries`, the backstop scan that re-enqueues a delivery whose job was **deleted or lost** — not one that was dead-lettered, whose `dedupe_key` the parked row still holds; `0024_events-fanout-attempts` adds `events.fanout_attempts` and a third `fanout_state`, `failed`, so an event the drain can never fan out is abandoned after five passes instead of heading every page and re-alerting forever (`a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`), and re-issues `0022`'s `payload_sha256` comment, which said "the first attempt" where it is the first attempt that *rendered and signed a body*; it said twelve until 2026-09-02, thirteen and then eighteen on 2026-09-03, and this count was one the Step 3 pass left stale — `0019` and `0020` landed with the rails and were never added to it); this row covers only the first five — see the rows below for `0006`–`0013`, and this paragraph for `0014`–`0021`. **`0019_charges-return-url`** adds `charges.return_url` with the 2048-character bounded-web-URL CHECK the confirm path refuses ahead of, and **`0020_provider-requests-status-code-comment`** is comment-only: it documents the `0` sentinel (*the rail answered; the port carries no HTTP status*) that `NULL` must not be confused with. **`0021_create-jobs` landed 2026-09-03 (Step 4)** — the `jobs` table (its own row below) plus `charges.provider_txn_id TEXT` with a 1–128-character CHECK, the rail's own identifier for a settled payment, written **only** by `vpay_db::Settlement::apply_succeeded` and deliberately *not* stuffed into `provider_ref_extra`, which a callback repair is allowed to overwrite wholesale. Both are asserted against a real Postgres in `backends/crates/vpay-db/tests/repositories.rs` — the table by every `jobs` test in that file (a claim, a lease, a reap, a dedupe, a park), and the column by `provider_txn_id_round_trips_and_its_check_refuses_empty_and_over_long`, which proves the CHECK refuses `''` and a 129-character value. **Five migrations landed on 2026-09-03 (Step 2), and all five are proven applied against a real Postgres by `migration_0014_replaces_last_payment_error_and_0015_to_0018_create_their_tables` in `backends/crates/vpay-db/tests/repositories.rs`** — which asserts the dropped column is gone, the new ones are present, and each new table exists. `0014_payment-intent-api-fields` adds to `payment_intents` a `seq BIGINT GENERATED ALWAYS AS IDENTITY` (the only pagination order; `created_at` cannot be it, because two intents in the same microsecond tie and a cursor over a non-unique ordering skips or repeats rows), `metadata JSONB`, `description`, `updated_at`, and the pair `last_payment_error_code failure_code` + `last_payment_error_message`, with `lpe_paired`, `lpe_message_length`, `description_length`, `metadata_is_object` and `pmt_is_array` CHECKs, a unique index on `seq`, a `(merchant_id, seq DESC)` index, `charges.updated_at`, and a partial index over the four live charge states. **It is a hard cutover, and the migration says so in its own header:** the free-text `last_payment_error` column added by `0003` is **DROPPED, not backfilled**. That is defensible only because nothing in this repository had ever written it — no SQLx query referenced `payment_intents` at all before this step — so a backfill would have been inventing values; a deployment that somehow held rows with a non-NULL `last_payment_error` loses them. `updated_at` is maintained by the repository layer, deliberately not by a trigger. `0015_create-idempotency-keys` creates the ledger the Idempotency row describes, including a `claim_id UUID DEFAULT gen_random_uuid()` that identifies which claim owns a row: the primary key alone is not enough once a row is reclaimable after expiry, and without it a stalled request waking up after its key was taken over would overwrite a live claim's stored response with its own. `0016_create-provider-requests` creates one row per *attempt* to call a rail (never one per charge), with `response_is_paired` keeping `status_code` and `responded_at` in step, so a NULL-status row is an unanswered attempt — exactly what a recovery sweep will look for (`provider_requests_record_attempts_and_keep_status_and_responded_at_in_step`). **`0017_create-refunds` and `0018_create-events` are schema only, and their own `COMMENT ON TABLE` says so:** that was true of both when they landed. ~~It is still true of `0017_create-refunds` — no refunds repository, no `/v1/refunds` route, nothing writes a refund.~~ **Corrected 2026-09-05 (issue #45): half of it is no longer true.** There is now a refunds repository (`vpay_db::refunds`, one read and no write) and a `/v1` route (`GET /v1/refunds/{id}`). **Nothing writes a refund**, which is the half that still stands, and is why `0017`'s own `COMMENT ON TABLE` was left unedited (`0031`'s header and `COMMENT ON COLUMN` were rewritten on 2026-09-06 for the same reason in reverse: they said no repository and no route existed, and both now do): it records the schema-only posture, and a comment rewritten to say "read-only" would be a claim about a create nobody has written. It is **no longer true of `0018_create-events`**: Step 4 writes those rows and Step 5 drains and serves them (the Events and Webhooks rows below). `0017` alone is now declared the way the ledger tables in `0005` were, and carries the same warning |
| Authkestra OP tables (`0006_create-authkestra-op-tables.sql`, extended by `0013_add-authkestra-op-0-7-columns.sql`) | ✅ | `CREATE SCHEMA authkestra` plus `oauth_clients`, `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes` — a byte-faithful transcription of the `CREATE TABLE` string literal hardcoded inside `authkestra-op` `=0.3.4`'s own `SqlxOpStore::migrate()` (not a vpay design; table/column names and types are not configurable — see the migration's header comment). **Upgraded to `authkestra-op = "=0.7.1"` this pass (from `=0.5.4`), and the re-diff the previous note demanded was done, not assumed:** `diff` over the extracted 0.3.4 and 0.7.1 crate sources shows the four tables 0006 creates are byte-identical, and 0.7.1's `migrate()` adds exactly one table (`authkestra.oauth_dpop_jti`, RFC 9449 DPoP replay tracking, authkestra#291) and three columns (`oauth_refresh_tokens.jkt`, `oauth_clients.token_endpoint_auth_method JSONB`, `oauth_clients.jwks JSONB`, authkestra#287). Migration `0013` transcribes those additions; it is **not optional** at this pin — `get_token`/`consume_token` now `SELECT … jkt` unconditionally and would fail at runtime against 0006's table alone. Proven compatible, not just transcribed correctly by eye: `backends/tests/integration/tests/authkestra_op_smoke.rs`'s `sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes` drives the real `SqlxOpStore<Postgres>` against this schema end to end — inserts a client, `find_client` (JSONB columns decode through the store's own type, **now including `token_endpoint_auth_method` decoding to `TokenEndpointAuthMethod::PrivateKeyJwt` and `jwks` round-tripping as raw JSON**), `store_code`, `consume_code`, and asserts a second `consume_code` of the same code returns `None`, proving the crate's single-use `UPDATE … WHERE used = FALSE` actually fires here. Two new tests in the same file cover 0013's other additions through the store's own SQL: `sqlx_op_store_round_trips_a_refresh_token_with_its_jkt_column` (`store_token`/`get_token` round-trip `jkt`) and `sqlx_op_store_records_a_dpop_jti_once_against_migration_0013s_table` (`check_and_record_dpop_jti` accepts a fresh `jti` and refuses its unexpired replay). Neither refresh tokens nor DPoP are features vpay offers — see `docs/flows/dashboard-auth.md` — these prove schema compatibility with the pinned crate, nothing more. **Two API breaks absorbed in the same test file:** `AuthorizationCode` is `#[non_exhaustive]` since 0.6.0 (constructed via `AuthorizationCode::new` now), and `ClientRegistration::require_pkce` is deprecated since 0.7.0 because PKCE is unconditional on the authorization-code grant (authkestra#273) — the test no longer asserts on a field nothing reads. A second test in `postgres_smoke.rs` proves the `oauth_codes → oauth_clients` FK fires. `oauth_device_codes` is created even though vpay's login flow (PKCE only) never uses the device grant, because `SqlxOpStore` implements `DeviceCodeStore` unconditionally. **Marked ✅ for what this row claims — the DDL exists, matches the pinned crate, and is proven compatible against a real store — not for dashboard auth working.** No shipping binary constructs a `SqlxOpStore` or uses these tables — see "Dashboard auth" below. **Correcting a claim this row used to make, which this pass's dependency-graph check found stale:** it used to say `authkestra-op`/`authkestra-engine` were dev-dependencies of `vpay-tests-integration` only, with neither `vpay-server` nor `vpay-worker-bin` depending on `authkestra*` at all. That second half is no longer true — `vpay-db` added `authkestra-op` as a **production** dependency this pass (for `SqlClientAssertionStore`, OP-2), and both binaries depend on `vpay-db`, so `authkestra-op` (and, transitively, `authkestra-engine`) is now in both binaries' production dependency graph. `vpay-server`/`vpay-worker-bin` still do not name `authkestra*` directly in their own `Cargo.toml`s, but "depend on neither" is no longer an accurate description of the resolved graph — see the "cargo deny" infrastructure row for the concrete consequence (the `rsa` advisory's exposure is narrower than "dev-only" now claims). **Coupling risk:** this migration pair is pinned to `authkestra-op = "=0.7.1"` (root `Cargo.toml`) and must move in lockstep with it — the crate hand-builds SQL against these exact table/column names as string literals, so nothing type-checks a mismatch. Any future version bump of `authkestra-op` requires re-reading `sqlx_store.rs`'s `migrate()` block at the new version and re-diffing against this file before assuming compatibility still holds; the migration's own header comment says the same and this is not to be treated as a routine dependency bump. **Correction, 2026-09-05 — this row's ✅ no longer rests on what it says it rests on.** `authkestra_op_smoke.rs` was deleted when `vpay-api` stopped constructing a `SqlxOpStore` (see the "The three OP stores that pinned sqlx 0.8" section under Infrastructure), so the three tests named above no longer exist and **nothing in this repository now exercises `SqlxOpStore`'s own SQL against these tables**. What still holds them: `postgres_smoke.rs` asserts all four tables exist and that `oauth_codes.client_id`'s foreign key fires — **and, since the review of that same branch on 2026-09-05, `authkestra.oauth_dpop_jti` and 0013's three added columns (`oauth_refresh_tokens.jkt`, `oauth_clients.token_endpoint_auth_method`, `oauth_clients.jwks`) too, which the deletion had left named by no test at all.** What no longer does: the `find_client` JSONB decoding, the `jkt` round trip and the DPoP `jti` single-use case. The tables are **unread and unwritten by any vpay code path** as of this date. Dropping them is a real option and is **left to the maintainer** — it needs a new migration, and this pass did not take it. The header comments inside `0006` and `0013` still cite the deleted file; they were left alone deliberately, because `sqlx::migrate!` checksums each migration's entire file content and editing an applied one turns the next boot into a version mismatch |
| OAuth signing keys (`0007_create-oauth-signing-keys.sql`, reshaped by `0010_reshape-oauth-signing-keys.sql`) | 🟡 | vpay-owned table (authkestra ships no signing-key type, store, or rotation logic at any published version — confirmed by grepping `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` source for `struct SigningKey`, `trait KeyStore` and `fn rotate`, with no hits). **Reshaped this pass: `private_key_pem TEXT` is dropped entirely and replaced with `public_jwk JSONB`; `id` is renamed to `kid`.** The decision (migration `0010`'s own header comment) is that the RS256 private key comes from a Kubernetes Secret via env at process boot and is parsed once by `authkestra_engine::TokenManager::new_asymmetric`, never persisted — so this table now stores only what `/jwks.json` needs to publish across a rotation window: the public half, its `kid`, and the validity window. **This corrects last pass's own note, which said the private key PEM was stored in plaintext and readable by anyone who could `SELECT` the column — that is no longer true; no private key material exists in this table or this repository at all.** The three constraints (partial unique index `one_active_signing_key`, `active_key_has_no_expiry`, `expiry_after_creation`, the last two renamed alongside the column) are proven to still fire *after* the reshape by the same dedicated tests in `postgres_smoke.rs`, updated to insert `kid`/`public_jwk` rather than `id`/`private_key_pem`. **New this pass: a Rust repository layer exists** (`vpay_db::signing_keys` — `publishable_signing_keys`, `active_signing_key_kid`, `rotate_signing_key`), tested against a real Postgres in `vpay-db/tests/repositories.rs` — `publishable_signing_keys_includes_active_and_unexpired_retired_but_excludes_expired` proves the `WHERE active OR expires_at > now()` overlap-window query keeps a just-retired key publishable and drops a long-expired one, and `rotate_signing_key_leaves_exactly_one_active_key` proves the one-transaction retire-then-insert both bootstraps cleanly (no prior active key) and rotates cleanly (an active key already exists), leaving `one_active_signing_key` intact either way. **Both of this row's previous reasons for 🟡 are now closed, 2026-09-02 (Step 1).** (1) **Key generation exists**: `cargo xtask gen-signing-key --out <dir>` writes a 3072-bit RSA PKCS#8 PEM, `0600`, refusing to overwrite — `a_generated_key_parses_back_off_disk_with_the_same_kid`, `the_key_file_is_only_readable_by_its_owner`, `it_refuses_to_overwrite_an_existing_key_file` (`.xtask`). `just gen-e2e-signing-key` is the openssl equivalent for the compose stack, so the CI e2e job needs no Rust toolchain. (2) **A shipping binary now calls this module.** `vpay_api::op::keys::LoadedSigningKey::from_file` parses the PEM into `authkestra_engine::TokenManager`, derives the `kid` as the RFC 7638 thumbprint of the public JWK — a function of the key, not of the file or the process (`the_kid_is_a_function_of_the_key_and_not_of_the_encoding_or_the_process`) — and cross-checks the JWK it publishes against `TokenManager::public_jwk`, so the key announced and the key signed with cannot diverge (`the_published_jwk_is_the_key_authkestra_signs_with`, `the_published_jwk_has_the_six_members_a_verifier_needs_and_a_self_consistent_kid`). Anything that is not an RSA private key, and anything under 2048 bits, is refused (`anything_that_is_not_an_rsa_private_key_is_refused`), and no error message or source chain echoes the PEM (`no_error_message_or_source_chain_echoes_the_pem`). `vpay-server` loads it **before** connecting to Postgres, so the three failure modes are testable without Docker and all exit `78`: `a_missing_signing_key_flag_is_exit_78_naming_the_problem`, `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`, `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents` (`backends/apps/vpay-server/tests/cli.rs`, subprocess). Activation goes through the new `vpay_db::ensure_active_signing_key`, which takes a Postgres advisory lock and does the whole read-decide-write in one transaction, so N replicas booting on the same Secret rotate once between them (`ensure_active_signing_key_bootstraps_is_idempotent_then_rotates_once`, `concurrent_ensure_active_signing_key_calls_with_the_same_kid_rotate_exactly_once`, `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` — a rollback to a retired `kid` is refused rather than silently resurrecting a key). **Still 🟡, for three new and smaller reasons, none of them "nothing calls it":** (a) **there is no rotation at runtime** — `TokenManager` holds exactly one key for the life of the process, so rotating means restarting with a new Secret; nothing re-reads the file, and no operator runbook describes the sequence; (b) the five `ensure_active_signing_key` tests are Docker-backed and **have not been run on any machine yet** (see the header paragraph) — the code is written and the tests exist, nothing has observed them pass; (c) the PEM is **not zeroized** — `LoadedSigningKey::from_file` reads it into a `String` that is dropped normally, so key bytes may linger in freed heap. That is a deliberate, stated limitation, not an oversight (`op/keys.rs`'s own module docs say so), and it is not fixed here. **Rollback to a retired key (security review 2026-09-02):** `ensure_active_signing_key` now refuses it with `DbError::SigningKeyRetired { kid, retired_at }` (`Category::Configuration`, so `vpay-server` exits 78 naming the kid and the retirement instant) instead of a raw duplicate-key SQL error — proven against a real Postgres by `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` and by `a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69` in `vpay-server`'s `tests/cli.rs`. Re-activating a still-publishable retired key is deliberately *not* done — that is the rotation-policy decision [docs/roadmap.md](roadmap.md) leaves open; the operational consequence, that `kubectl rollout undo` after a rotation is a clean exit 78 rather than a degraded boot, is stated here on purpose. The `bootstraps_is_idempotent_then_rotates_once` test had never executed before this pass and failed on its first real run — it compared a nanosecond `OffsetDateTime` with the microsecond `TIMESTAMPTZ` read back; fixed by building the expected instant at microsecond precision, and all 12 `vpay-db` tests now pass on a real container on the authoring machine |
| Merchant API keys — dropped (`0008_create-merchant-api-keys.sql`, dropped by `0009_drop-merchant-api-keys.sql`) | ⛔ | The Stripe-shaped `sk_live_`/`sk_test_` bearer-key design this table backed is reversed by [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md): `authkestra_op::sqlx_store::SqlxOpStore::find_client` hardcoded `token_endpoint_auth_method: None`/`jwks: None` on every row at the then-pinned `authkestra-op = "=0.3.4"`, so an OP-backed client registry could not serve `private_key_jwt`. **That premise is no longer true at the current pin (`=0.7.1`): both columns are persisted and read back (authkestra#287), proven here by migration `0013` and the `find_client` assertions in `authkestra_op_smoke.rs`.** ADR-0010's *decision* — merchant clients in YAML, no database-stored merchant identity — is unchanged; an ADR is superseded, never edited, and whether the now-available OP-backed registry should replace YAML is a maintainer question this pass raises and does not answer. Per this repo's hard-cutover rule, `0009` is a straight `DROP TABLE`, not a deprecation — nothing had ever read or written a row here (last pass's own note said so), and the two tests that proved this table's constraints were deleted in the same migration rather than left passing against a table that no longer exists. **A reader must not infer from ADR-0010's continued reference to this migration number, or from this row remaining in the table for historical clarity, that `merchant_api_keys` still exists — it does not.** See "Merchant auth" below for the model that replaces it |
| Merchant auth (`/v1`: `client_credentials` + `private_key_jwt`, [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md)) | ✅ | **The server half of this flow now exists.** A merchant is a statically registered OAuth2 client with a `client_id` and **public** JWK in YAML, authenticating with a signed `private_key_jwt` assertion; `vpay_api::op::clients::registration_for` is the conversion into `authkestra_op::client::ClientRegistration` this row spent two passes calling "the missing piece", and it is mechanical by design — `token_endpoint_auth_method: Some(PrivateKeyJwt)`, `client_secret_hash: None`, `redirect_uris: []`, and `grant_types` mapped from the config enum rather than hardcoded so `ConfigError::DisallowedMerchantGrant` stays observable (`the_conversion_maps_every_field_the_op_reads`, `the_conversion_maps_grants_it_is_given_rather_than_hardcoding_one`). **The registration is proven to be one the real verifier accepts, not one that merely type-checks:** `an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds` mints an assertion with the shipping `vpay-sdk` (the merchant SDK itself, added as a `[dev-dependency]` — not a test double) and feeds it to `authkestra_op::client_assertion::verify_client_assertion` at the pinned `=0.7.1`; `an_assertion_signed_by_a_key_this_merchant_did_not_register_is_refused` is the negative control. The `vpay:v1` audience the three parties must agree on is now one constant, `vpay_config::MERCHANT_AUDIENCE`, returned by `Surface::Merchant.audience()`, and a deployment whose merchant cannot target it **refuses to boot** (`ConfigError::MerchantMissingV1Audience`, proven by `a_merchant_client_that_cannot_target_the_v1_audience_is_rejected` against a fixture that is verbatim what `config/application.yml` shipped until this pass, plus `the_example_config_registers_its_merchant_for_the_v1_audience` on the real file). End to end, over a booted server on a real database, `backends/tests/integration/tests/merchant_token_flow.rs` covers all six claims this row makes: a token is obtained by the SDK and reaches the authenticated 404 (`an_sdk_client_authenticates_and_reaches_the_honest_404`), no bearer is a 401 envelope (`a_v1_request_with_no_bearer_token_is_the_401_envelope`), a disabled client is `invalid_client`/401 with no restart (`a_disabled_client_is_refused_with_invalid_client_and_401`), a dashboard-audience token this same server signed is refused on `/v1` (`a_dashboard_audience_token_is_refused_on_v1`), JWKS lists exactly the active `kid` and discovery matches the URLs the SDK derived independently (`the_jwks_and_discovery_documents_describe_this_process`), and one assertion cannot be spent twice (`the_same_client_assertion_cannot_be_spent_twice`). **🟡 and not ✅, for one reason and it is about evidence, not code:** ~~those six tests **have never run under Docker, here or in CI** — the only observation of them passing is the implementer's single manual run against a scratch database on an already-running Postgres (header paragraph). When the CI `rust` job runs them green, this row is ✅ and should say which run.~~ **Corrected 2026-09-05: they have run under Docker in CI.** CI run `33929374663` (2026-09-04, `master`, head `33d6c25`) ran `cargo nextest run --workspace` on `ubuntu-latest` to **1159 tests run, 1159 passed, 0 skipped**, and all seven `vpay-tests-integration::merchant_token_flow` tests are `PASS` in that log by name (163 `vpay-tests-integration` and 86 `vpay-db` tests in the run); `ci.yml` fails loudly without a Docker daemon rather than skipping, so green means they executed under testcontainers. **This row's own promotion criterion is therefore met and the row is still 🟡 anyway** — flipping it is a call for whoever owns this page, not for the documentation pass that measured the run; it is left 🟡 so the decision is visible rather than made in passing. **Separately not done, and not blocked on that:** there is no `/v1` business resource for a valid token to reach — an authenticated request gets the honest 404, deliberately — and no rate limit on `/token` (ADR-0009 leaves it to ingress; nothing here verifies ingress does it). One property that is real and easy to misread as a bug: **an access token already issued to a client stays valid for its remaining TTL after that client is disabled** — the kill switch acts on token *issuance*, which is what a stateless bearer token means, and `a_disabled_client_is_refused_with_invalid_client_and_401` builds a fresh SDK client precisely so it tests the endpoint rather than a cache. The *client* side of the flow — the two merchant SDKs, `sdks/rust` and `sdks/nodejs` — is unchanged this pass and described in the "Merchant SDKs" section below; what changed is that the contract they were written against is now served by something. **Updated 2026-09-03 (Step 2), on two points.** (1) **The evidence is no longer one manual run.** All 7 `merchant_token_flow` tests (a seventh, `a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1`, joined the six listed above) ran under testcontainers on the authoring machine on 2026-09-03 as part of a 74-test container-backed run that passed clean — see the header. ~~They have still never run in CI.~~ **Retired 2026-09-05: they run in CI — run `33929374663`, see the correction above.** (2) **Authentication now carries a tenancy decision, not only an identity one.** The middleware resolves the token's `client_id` to the YAML registration's `merchant_id` and puts a `MerchantScope` in request extensions; every `/v1` query filters by it, and a merchant asking for another merchant's `pi_…` gets the same 404 as for one that never existed (`merchant_b_cannot_read_merchant_as_intent`, integration). The scope check (`payments:write` / `payments:read`) is part of the same single validation. **🟡 → ✅ on 2026-09-03 (Step 7), on the condition this row itself registered two passes ago — “when the CI `rust` job runs them green, this row is ✅ and should say which run”.** The run is **`33792230584`** on `master` (`ci`, commit `ca94eac`, the merge of PR #24): all six jobs green, `rust` among them, which is the first time the seven `merchant_token_flow` tests have run under Docker anywhere but an authoring machine. Verified for this note with `gh run view 33792230584`, not taken on report. **What ✅ does not claim, and these are unchanged:** no deployed vpay has ever completed this handshake for a real merchant, there is no rate limit on `/token` (ADR-0009 leaves it to ingress and nothing here verifies ingress does it), and an access token already issued stays valid for its remaining TTL after its client is disabled — a property of stateless bearer tokens, not a gap. |
| Client-assertion replay protection (`oauth_client_assertion_jtis`, `0011_create-oauth-client-assertion-jtis.sql`) | 🟡 | Backs `authkestra_op::client_assertion::ClientAssertionStore::record_jti`, which neither of `authkestra-op`'s two shipped implementations can satisfy for vpay's deployment: `NoClientAssertionStore` fails closed unconditionally, and `MemoryClientAssertionStore` is single-process only (its own doc comment names exactly vpay's situation — multiple replicas — as needing "something shared... instead"). This table's `jti TEXT PRIMARY KEY` is the atomic single-use guard, meant to be used as `INSERT ... ON CONFLICT (jti) DO NOTHING` read via `rows_affected()`, never check-then-insert (the migration's own header comment explains the TOCTOU race a separate SELECT would reintroduce). Two dedicated tests in `postgres_smoke.rs` prove the constraint at the database level (`a_duplicate_client_assertion_jti_is_rejected_by_the_database`, `on_conflict_do_nothing_reports_zero_rows_affected_for_a_replayed_jti`). **New this pass: a real Rust implementation exists — `vpay_db::SqlClientAssertionStore`**, implementing `authkestra_op::client_assertion::ClientAssertionStore::record_jti` with exactly that `INSERT ... ON CONFLICT DO NOTHING` pattern, converting `authkestra-op`'s `chrono::DateTime<Utc>` boundary type to vpay's own `time::OffsetDateTime` convention explicitly at the crossing (`chrono_to_offset_date_time`, `client_assertion.rs`). **Proven race-safe, not just correct when called sequentially**: `concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result` fires 10 concurrent `record_jti` calls with the same `jti` against a real Postgres and asserts exactly 1 reports fresh and 9 report replayed — the same shape of proof `authkestra-op`'s own `sqlx_store` tests use for `consume_code`. **Wired 2026-09-02 (Step 1): `MerchantOp::new` passes a `SqlClientAssertionStore` to `CompositeOpStore::with_client_assertion_store`, so every `/v1` token request goes through it.** That it is genuinely wired — rather than merely constructed — is what `the_same_client_assertion_cannot_be_spent_twice` proves: one assertion is sent by hand twice (the SDK correctly mints a fresh one per request, which is exactly why the SDK cannot reach this case), the first exchange succeeds, and the second is refused `invalid_client`/401 while the assertion is still well inside its own lifetime and would verify perfectly on its own. Drop `with_client_assertion_store` and that test fails. **Still 🟡, for two reasons.** (1) ~~That test is Docker-backed and has never run under Docker — one manual scratch-database run is all the evidence there is (header paragraph).~~ **Corrected 2026-09-05:** `the_same_client_assertion_cannot_be_spent_twice` is `PASS` in CI run `33929374663` (2026-09-04, `ubuntu-latest`, 1159/1159, 0 skipped), so it has run under Docker in CI. What is still true is that nothing outside a test has ever spent an assertion. (2) ~~**There is still no cleanup job.**~~ **Closed 2026-09-03 (Step 4): `vpay_db::delete_expired_client_assertion_jtis` now runs on a timer.** It is one of the three statements of the worker's hourly `sweep:expired` job (`vpay_worker::handlers::sweep_expired`), seeded at worker boot with `ON CONFLICT (dedupe_key) DO NOTHING` and rescheduling itself for as long as the deployment lives; `the_housekeeping_jobs_are_seeded_once_and_reschedule_themselves` proves the seed is a singleton and that a run puts the job an hour out. A long-lived process no longer grows this table monotonically — **provided a worker is running**, which is a new operational dependency and is stated as one: with no `vpay-worker-bin` in the deployment, nothing sweeps at all, because the boot-time stopgap moved out of `vpay-server` in the same change. The sweep's own correctness is proven by reading rows back rather than trusting a count (`expired_client_assertion_jtis_are_swept_and_live_ones_are_kept`, `backends/tests/integration/tests/client_store.rs`) — **a test that has not been run anywhere yet**. **Known limitation, recorded not fixed (security review 2026-09-02):** the replay namespace is global — `jti` alone is the primary key and the upstream `record_jti` seam carries no `client_id` — so a merchant using low-entropy `jti`s could collide with or pre-spend another merchant's; [docs/flows/merchant-auth.md](flows/merchant-auth.md) now states `jti` MUST be a UUID v4 (both SDKs comply) and leaves the `(client_id, jti)` re-keying as a maintainer decision |
| Disabled-clients kill switch (`disabled_clients`, `0012_create-disabled-clients.sql`) | 🟡 | An operator revocation mechanism for an OAuth client (dashboard or merchant `client_credentials`) that takes effect without a deploy — `client_id` plus a disable flag/reason, no credential and no identity of its own (YAML stays authoritative for identity; this table only ever *subtracts* access). Its uniqueness is proven by two tests in `postgres_smoke.rs`: `disabled_clients_accepts_an_insert` and `a_duplicate_disabled_client_id_is_rejected_by_the_database` (rejected specifically on the `client_id` primary key). **New this pass: query functions exist — `vpay_db::is_client_disabled`/`disable_client`/`enable_client`** (`vpay-db/src/disabled_clients.rs`), deliberately uncached (the module's own doc comment argues a cache would reintroduce the revocation delay this table exists to remove). `disabled_client_lookup_reflects_disable_and_enable` in `vpay-db/tests/repositories.rs` proves all three functions observe the same underlying table consistently against a real Postgres, including that `disable_client` is idempotent (a second disable of an already-disabled client updates `reason` without erroring) and `enable_client` is a no-op on a client that was never disabled. **Enforced 2026-09-02 (Step 1), and in the one place where enforcing it is sufficient.** `vpay_api::op::clients::YamlClientStore::find_client` consults `is_client_disabled` — and `find_client` is step 1 of `authkestra_op`'s `handle_token_request`, the single point every token request passes through for every grant. That is not a convenience: reading the pinned `authkestra-op-0.7.1/src/handlers/token.rs`, `handle_client_credentials` takes the already-resolved registration and mints straight through `TokenManager`, consulting no store afterwards, so a kill switch enforced anywhere else would not be enforced at all on the one grant `/v1` uses. Three properties, three tests. A disabled client is reported as `Ok(None)` — "no such client" — so the token endpoint cannot be used as an oracle for whether a merchant exists but is suspended (`find_client_reflects_the_disabled_clients_kill_switch`, integration, which also proves disable and re-enable take effect on the next lookup with no restart). An unknown `client_id` — the shape every credential-stuffing attempt has — is answered from the in-memory YAML index and never reaches Postgres (`an_unknown_client_id_is_refused_without_touching_the_database`). **And a failed lookup fails closed:** a database error becomes `OpError::Storage`, which `handle_token_request` maps to `server_error`, so an outage produces no token rather than a token for a client that may have been revoked (`a_failed_kill_switch_lookup_refuses_a_known_client_rather_than_admitting_it` — returning `Ok(None)` there would have rendered as `invalid_client` and pointed an operator at the merchant instead of at Postgres). End to end: `a_disabled_client_is_refused_with_invalid_client_and_401`. **Still 🟡, for two reasons.** (1) Evidence: both integration tests are Docker-backed and neither has run under Docker (header paragraph). (2) The switch acts on **issuance only** — an already-issued token remains valid for the rest of its TTL, which is what a stateless bearer token means and what ADR-0009's revocation-gap open question is about; nothing in this repo shortens that window. `disable_client`/`enable_client` are still called by no shipping code — an operator flips the row by hand, and **no runbook documents the `disabled_clients`-plus-YAML dual authority yet** |
| Dashboard auth (`/dash/v1` as an Authkestra OP) | ✅ | Decision recorded in [ADR-0009](adr/0009-dashboard-oidc-provider.md), design in [docs/flows/dashboard-auth.md](flows/dashboard-auth.md). **Upgraded from ⛔ this pass, on the strength of the same three prerequisites "Merchant auth" above lists** — the dashboard client is now modelled and validated in config (`vpay_config::oauth::DashboardClient`), and `vpay_api::resource_auth::JwtValidator`/`AuthenticatedDashboard` pinned to `Surface::Dashboard` is proven to validate a correctly-audienced token and reject a merchant-audienced one on this surface specifically (`a_dashboard_audience_token_is_accepted_by_the_dashboard_validator`, `a_merchant_audience_token_is_rejected_by_the_dashboard_validator`, in `resource_auth.rs`). **Still no `/dash/v1` route, and a reader must not conclude login works from any of this**: no login has ever been performed, no token has ever been issued by this code, and no key has ever been rotated — `rotate_signing_key` (OP-2, row above) rotates to a key it is handed, it does not generate one. `authkestra-op`/`authkestra-engine`/`authkestra-axum`/`authkestra-resource` are pinned in the root `Cargo.toml`; `authkestra-resource` is now a genuine production dependency of `vpay-api` (for `JwtValidator`), and `authkestra-op`/`authkestra-engine` are production dependencies of `vpay-db` (for `SqlClientAssertionStore`, OP-2) — **so, unlike what this row used to say, `authkestra-*` is no longer dev-dependency-only; it is in both shipping binaries' resolved graph** (see the "Authkestra OP tables" row above and the "cargo deny" infrastructure row for the concrete consequence). **Status unchanged 2026-09-02 (Step 1) — still 🟡, and the reader must not infer otherwise from the merchant rows above: no login has ever been performed and no `/dash/v1` route exists.** Two of this row's stated prerequisites did close, and they are worth naming precisely because they are the ones most easily mistaken for the feature. (1) **Signing keys and JWKS are real now**: a key is generated, loaded, announced in `oauth_signing_keys` and published at `/v1/oauth/jwks.json` across a rotation window — see the "OAuth signing keys" and "Merchant OP" rows. (2) A shipping binary does now construct `SqlxOpStore<Postgres>` — but as three slots the `OpStore` supertrait demands and **no `/v1` grant reaches**, not as anything serving `/dash/v1`. What is still missing, and it is the whole feature: **no `/login` route, no `/authorize`, no `/dash/v1` anything**; **no `SessionStore`** — `authkestra-engine` is pinned with `features = ["rustls-no-provider", "token", "session"]` and **without `sql-postgres`**, so no SQL-backed session store is even compiled in; and a design problem this pass surfaced but did not solve — `authkestra-op`'s `default_handle_authorization_code` mints the access token with `Some(client_id)` as the audience (`authkestra-op-0.7.1/src/handlers/token.rs`, step 7), with **no requested-audience path at all**, so a token from that grant would carry `aud = <client_id>` and `Surface::Dashboard.audience()` (`vpay:dash/v1`) would reject every one of them. Whoever builds `/dash/v1` has to resolve that first; the merchant surface does not hit it because `handle_client_credentials` *does* honour a requested audience. Rotation is also restart-based (the "OAuth signing keys" row), so "rotating a signing key at least once" — this flow's own definition of done — has still never happened. **Amended 2026-09-06 (exp23), and the amendment cuts both ways.** ~~"no `/dash/v1` anything"~~ is no longer true: two `GET` routes are mounted (see the "`/dash/v1` read surface" row below). Everything else in this row still is — **no login has ever been performed, no `/login` or `/authorize` route exists, no `SessionStore` is compiled in, no key has ever been rotated, and the audience problem is unresolved.** exp23 also found and recorded a *second* blocker this row never named, which is larger than the audience one: `authkestra_op::handlers::authorize::handle_authorize` takes an already-authenticated `authkestra_engine::auth::state::Identity` **as a parameter** — it authenticates nobody — and vpay has no staff table, no credential store, no password hashing and no `AuthenticationStrategy` implementation to produce one. **How a human staff member proves who they are has never been decided anywhere in this repository**, and picking one (staff table + hashes, WebAuthn, TOTP, or federating the human step to an external IdP in front of vpay's own OP) is an ADR that touches ADR-0009's central claim, not a default to take in passing. It is now written down in [flows/dashboard-auth.md](flows/dashboard-auth.md)'s blocker 1 | **AMENDED 2026-09-07 (exp24, [ADR-0017](adr/0017-staff-authentication.md)), and the amendment is the whole row: ~~no login has ever been performed~~ is no longer true, which is why this is ✅ and not 🟡.** `backends/tests/integration/tests/staff_sign_in.rs` — 13 cases, all passing on 2026-09-07 — drives the real `vpay_api::router` on a real socket over a real Postgres and **mints no token of its own**: every token it presents came out of `POST /dash/v1/oauth/token` after a password, a TOTP code and a PKCE exchange, and the server verified it through its own published JWKS over the same socket. What closed each blocker this row named. (1) ~~How a human staff member proves who they are has never been decided~~ — decided, as ADR-0017 decision 1: a vpay-owned `staff_members` table (migration `0035`), argon2id at OWASP's first parameter set with a deployment pepper as argon2's *secret input*, and mandatory RFC 6238 TOTP whose secret is AES-256-GCM-sealed under a second deployment key. The codes are pinned against RFC 6238 Appendix B's own vectors and the hand-written base32 against RFC 4648 §10's. (2) ~~No `SessionStore` is compiled in~~ — still not, and it is **not** a blocker: `staff_sessions` is vpay's own table through vpay's own data layer, with an absolute 12 h bound that is never extended and an idle 30 min bound that every accepted request moves. Enabling `authkestra-engine`'s `sql-postgres` would pull `sqlx/chrono` and `sqlx/json` into the graph for a store this deployment does not use. (3) ~~The audience problem is unresolved~~ — resolved by changing the **validator** rather than the grant (ADR-0017 decision 3). `Surface::audience()` is gone; `JwtValidator::new` takes the audience as a value, and it is the registered `dashboard_client.client_id`, because that is what `default_handle_authorization_code` actually mints. `vpay:dash/v1` is retired and ADR-0017 supersedes that part of ADR-0009. (4) **Key rotation has still never happened**, and that is the one blocker this row named that is untouched. `TokenManager` holds one key for the life of the process, rotation is restart-based, nothing re-reads the key file, and a rollback to a retired `kid` is refused. **Three further things this row must not be read as claiming.** There are **no pages** — `frontends/apps/dashboard` is byte-identical to the scaffold, `dashboard.cy.ts` still asserts the scaffold notice, and every route above is reachable over HTTP and by nothing a person can click. There is **no sweep** of `staff_sessions` or `oauth_authorization_codes` — expired rows are refused on read and removed by the sign-out cascade, and the indexes a sweep would need exist while the sweep does not. And the **rate limit is per replica**: three replicas admit three times the attempts one does, which is the stated cost of in-process fixed-window limiting and is recorded in ADR-0017's Consequences rather than hidden.
| `/dash/v1` read surface — `vpay_api::dash` | 🟡 | **New 2026-09-06 (exp23).** Two `GET` routes — `/dash/v1/payment_intents` (cursor-paged, `?status=`/`?created_gte=`/`?created_lte=`) and `/dash/v1/payment_intents/{id}` (the intent, its charge, its refunds, its event timeline) — behind `require_dashboard_token`, which validates for `Surface::Dashboard`, requires the token's `client_id` to be the registered dashboard client's, requires the registration's single scope, refuses any method but `GET`/`HEAD` before the router matches, and inserts a `MerchantScope` built from `dashboard_client.merchant_id` rather than from anything the token said. **The sentence that governs this row: no client of this deployment can obtain a token for it.** `client_credentials` is the only grant vpay serves, it is registered for merchant clients alone, and (since this pass) a merchant registration listing `vpay:dash/v1` is refused at boot. So this is a resource server with **no issuer** — real, tested, fail-closed and unreachable. It is 🟡 and not ✅ for exactly that reason, and it must not be read as "the dashboard works". `backends/tests/integration/tests/dashboard_read_surface.rs` proves thirteen things over a real booted server on a real Postgres (13 passed, 0 ignored, measured 2026-09-06): the bound merchant's rows come back and the other merchant's do not; another merchant's id is byte-for-byte the same `404` as an id that never existed; a merchant-audience token is refused (**the partner `a_dashboard_audience_token_is_refused_on_v1` has had none of since it was written**); a dashboard-audience token naming an unregistered client is refused; a token without the registered scope is refused; an expired token is a `401`; no route in `DASH_ROUTES` answers without a token; the detail read carries no `client_secret` where `/v1`'s `retrieve` does; the `status` filter narrows the page and an unknown status is a `400` naming `status` rather than an empty list; and a deployment with no `dashboard_client` mounts no nest at all; and — the eleventh, added by the 2026-09-06 sabotage review — the detail read renders the events and the refunds an intent *has*, both the intent's and the charge's events, oldest first, and **no other tenant's event**. That last one closes a gap a mutation found: every other assertion about `refunds` and `events` was `== []` over fixtures that had neither, so `Events::list_for_objects` and `Refunds::list_for_intent` could both `return Ok(vec![])` unconditionally with all ten original tests green — the empty-list shape AGENTS.md rule 2 names by hand. `events.object_id` carries no foreign key (migration `0018`), so that read's `merchant_id` predicate is the only thing keeping another tenant's event out of this timeline, and it is now pinned by an event row written for the other merchant against this merchant's intent. The twelfth, from the same review, pins the **list cursor's** tenancy predicate: `PaymentIntents::list_page_filtered` resolves `starting_after`/`ending_before` through subqueries carrying `AND merchant_id = $1`, so another tenant's id resolves to `NULL` and answers an empty page — and with that predicate removed from both subqueries, all 35 tests in `dashboard_read_surface` and `payment_intents` passed, which made `starting_after` an existence oracle for ids the caller may not read. The predicate is pre-existing and byte-identical to `3694e34`; `/v1` had no test for it either. The thirteenth pins the method refusal itself: a `POST`/`PUT`/`PATCH`/`DELETE` carrying a credential valid in every other respect is answered `403` by `require_dashboard_token` — the boundary's answer — rather than the `405` the route table would give, which is the distinction that makes read-only structural; making `dash::required_scope` answer the read scope for every method had left all twelve other cases green. Eleven mutations have been run against this surface across the pass and its review, and **ten are caught**: tenant from the token instead of the binding → 3 fail; delete the `client_id` check → 1 fail; the dashboard audience equal to the merchant one → 8 fail; either boot refusal deleted → 1 config test each; both detail reads returning `Ok(vec![])` → 1 fail; the events read's tenant predicate neutralised → 1 fail; either cursor subquery's tenant predicate deleted → 1 fail; `required_scope` answering the read scope for every method → 1 fail; the list rendering `PaymentIntentWithSecret` → 1 fail. **The eleventh is not caught and is recorded rather than smoothed over**: mounting the nest unconditionally changes nothing, because `require_dashboard_token` answers `404` on an absent validator or binding too — only removing *both* guards is observable. Both are kept. **`charges.payer_ref_masked` is rendered and is `null` on every row this system has ever written** (nothing populates the column — `open_attempt` stores `None`), so the payments list has no payer column with anything in it and there is deliberately no search by phone: a filter over an always-`NULL` column answers "no results" for every payer who ever paid, which reads as an answer. **A forward-compatibility finding from the 2026-09-06 review, recorded and deliberately not fixed:** the `client_id` check reads the token's `sub`, which is the OAuth2 client under `client_credentials` (`TokenManager::issue_client_token` sets `sub` to it) but is the **staff member** under the authorization-code grant this surface is blocked on (`default_handle_authorization_code` issues a *user* token and puts the client id in `aud`). As written the check would refuse every token a working dashboard login produced. Which claim identifies the dashboard credential once a human is in the loop is part of the same maintainer decision as the audience one, and is now written down in [flows/dashboard.md](flows/dashboard.md) and in `require_dashboard_token`'s own doc rather than guessed at. Nothing else in this pass makes that track harder — one `vpay_config::DASHBOARD_AUDIENCE` constant makes the audience half a single edit. Docs: [flows/dashboard.md](flows/dashboard.md) (new), [reference/vpay-api.md](reference/vpay-api.md) § the dashboard surface. **Not a merchant SDK surface** — `docs/sdks/parity.md` is untouched and must stay so | **AMENDED 2026-09-07 (exp24, ADR-0017), and the governing sentence of this row is withdrawn.** ~~The sentence that governs this row: no client of this deployment can obtain a token for it.~~ One can: see the "Dashboard auth" row above. Two consequences for what this row asserts. (1) **A `client_credentials` token is now REFUSED here**, which is the one behaviour change ADR-0017 makes to a surface that already worked, and it is deliberate — a token of exactly that shape was previously the only thing `/dash/v1` accepted. The refusal is a property of the mint rather than of a list: nothing but the staff authorization-code grant stamps the `vpay_merchant_id` claim `require_dashboard_token` now requires. `a_client_credentials_token_is_refused_on_dash_v1` pins it, and the decisive mutation is deleting the `claims.merchant` arm, which makes it answer `200` with the bound merchant's payments. (2) **The forward-compatibility finding this row recorded is resolved.** ~~the `client_id` check reads the token's `sub` … As written the check would refuse every token a working dashboard login produced~~ — exp23's F7. ADR-0017 decision 3 takes the maintainer decision it reserved: the credential is `aud` (checked by the validator, and equal to the registered client id) plus the merchant claim; `sub` names the staff row and authorises nothing. `ResourceClaims::client_id` is renamed to `subject` so nothing can read it as a client id again without saying so. The suite is **13 -> 16 cases** (measured 2026-09-07): the three added are the `client_credentials` refusal, a staff token whose merchant claim names another tenant, and — from the exp24 review — a token whose `sub` names **no staff member at all**, each refused `403` with the bound merchant's rows still absent from the body. That last one records something the suite had been doing silently: every token it minted named a `stf_…` nobody created, which the shipping grant cannot produce (`oauth_authorization_codes.staff_id` is a foreign key), and nothing noticed until `require_dashboard_token` started reading the row. The harness now seeds the staff member its tokens name. Its harness now mints what the grant mints — `issue_user_token_with_extra`, a `stf_…` `sub`, the client id as `aud`, the merchant claim — rather than `issue_client_token`. **Still 🟡**, and for a reason that has not changed: `charges.payer_ref_masked` is `null` on every row this system has ever written, so the payments list has no payer column with anything in it — and there are still no pages.
| Staff sign-in — `vpay_api::staff`, `vpay_api::staff_auth`, `vpay_db::{staff, staff_sessions, authorization_codes}` | ✅ | **New 2026-09-07 (exp24, [ADR-0017](adr/0017-staff-authentication.md)).** Seven unauthenticated routes under `/dash/v1` — `POST /staff/login`, `/staff/totp`, `/staff/password`, `GET /staff/session`, `POST /staff/logout`, `GET /oauth/authorize`, `POST /oauth/token` — merged **beside** the protected reads rather than inside the bearer-token layer, because they exist to produce the credential that layer checks. Credentials: argon2id (19 MiB, t=2, p=1 — OWASP's first listed set) with a deployment pepper as argon2's secret input, and RFC 6238 TOTP (HMAC-SHA1, 30 s, ±1 step) whose secret is AES-256-GCM-sealed under a second deployment key. Sessions are server-side rows keyed by the **SHA-256** of an opaque 256-bit token, with an absolute 12 h bound and an idle 30 min bound, and signing out **deletes** the row — which is the server-side deny-list ADR-0009's Consequences left undecided. **Three security properties are compare-and-swaps in SQL rather than checks in Rust**, because in each the thing being prevented is a race: the TOTP replay guard (`record_totp_step`'s `last_totp_step < step`), the authorization code's single use (`consume_code`'s `consumed_at IS NULL`), and enrolment happening once (`enrol_totp`'s `totp_enrolled_at IS NULL`). **Every refusal on the sign-in path is one answer** — no such address, wrong password, disabled account, wrong code, replayed code, expired session, idle session, forged session, session at the wrong stage: one `401`, one sentence, the step in the log and never the body. The timing half is `verify_absent_account`, which costs a real argon2id verification for an address that has no account, so the two answers take the same time as well as the same shape. Rate limiting is in-process, fixed-window, per email **and** per IP, refused before any credential work, with both counters moved on every attempt including a refused one; the per-IP half was **dead as first delivered** and was fixed by the exp24 review (finding F2) — the peer address reaches a handler only through axum's `ConnectInfo`, neither `vpay-server` nor the test harness built its service with `into_make_service_with_connect_info`, so every attempt in the process shared one `ip:unknown` bucket and ten unauthenticated requests locked the whole deployment out of the dashboard for five minutes (`the_sign_in_rate_limit_is_per_source_address` drives two loopback source addresses through a real socket and is what would fail again). Behind a reverse proxy the peer is the proxy, so the per-IP budget bounds the deployment rather than the caller — no `X-Forwarded-For` is trusted, and ADR-0017's Consequences says why. Proof: 16 container-backed cases in `staff_sign_in.rs` (13 as delivered, plus the three the exp24 review added below) (the happy path; identical answers for a wrong password and an unknown address; a disabled account refused at once on **both** credentials a sign-in produces — the session and the already-minted bearer token — and a staff member **moved to another merchant** refused on the token they already hold, both of which the exp24 review had to add (findings F1 and F6: as first delivered `require_dashboard_token` never read the staff row at all, so everything it checked was a statement about the *token* and none of it was a statement about the *person*; a disabled staff member kept listing payment intents, and a reassigned one kept reading their old merchant's, for the rest of the 15-minute access-token TTL, while `staff_members.status` was documented as the per-person kill switch); a replayed TOTP code refused inside its own step while the next step's works; a PKCE verifier mismatch; a code exchanged twice; a code redeemed against another redirect URI, refused **and spent**; a staff member of another merchant refused a code at all; idle and absolute expiry; sign-out; a password-only session refused at `/authorize`; the printed password unable to reach `/dash/v1`; a deployment with no `staff_auth` serving no login; and the sign-in rate limit binding per **source address**), plus 26 unit cases in `vpay_api::staff_auth` pinned against RFC 6238 Appendix B, RFC 4648 §10 and NIST's own SHA-256 vector, and RFC 7636 Appendix B for the PKCE check. **What is not built:** no pages (see the row above), no sweep of expired sessions or codes, no `audit_log` (ADR-0008 wants one per dashboard *write* and none is mounted), no way to disable the dashboard *client* short of removing it from YAML, and no key rotation. **Read as a residual, not a claim:** `staff_sessions.access_token` holds a live bearer token for the length of its TTL, so a database dump yields usable `/dash/v1` tokens until they expire. Hashing it does not work — the dashboard's own server has to present it — and ADR-0017's Consequences says so plainly. **A second residual, left open on purpose:** signing out makes the token *unobtainable* (the row it is read from is gone) but not *invalid* — the JWT verifies until it expires, `signing_out_deletes_the_session_and_with_it_the_access_token` asserts exactly that, and closing it means binding every `/dash/v1` read to a live session row, which makes the surface stateful. **That is a maintainer decision the exp24 review surfaced rather than took.** |
| `vpay-server staff add` | ✅ | **New 2026-09-07 (exp24).** The **only** way a staff member is created: no HTTP endpoint creates one and there is no self-service sign-up (ADR-0017 decision 1). It lives in the shipped binary rather than in `cargo xtask` because creating the first staff member happens against a production database from a `FROM scratch` image that has no source tree and no toolchain. It needs no signing key and binds no listener. Checks in order: the configuration, then `--merchant` against `merchant_clients` **before the database is opened** (there is no merchants table, so no foreign key could refuse it, and an unregistered tenant would be an account that signs in and sees nothing), then the insert — a `create` and not an `upsert`, so a second `staff add` for an existing address **fails** rather than quietly rewriting that person's password hash to one an operator just printed. **A defect this feature's own test found and fixed:** the one-time password went to stdout and so did the tracing subscriber, so it arrived as the fifth line of a JSON log and `> password.txt` would have written the log to the file and left the password on the terminal. A subcommand's logs now go to **stderr**; the server's stay on stdout, where a container log collector reads them. Four subprocess cases in `backends/apps/vpay-server/tests/cli.rs`. |
| `dashboard_client.merchant_id`, and the two boot refusals around it (`vpay_config`) | ✅ | **New 2026-09-06 (exp23).** `DashboardClient` gained a required `merchant_id`: the one tenant `/dash/v1` reads. Two boot-time refusals, both fatal, both with their own fixture and both proven by deleting the check and watching the test fail. (1) `ConfigError::DashboardUnknownMerchant` — a `merchant_id` no `merchant_clients` entry registers. It matters because the failure has **no runtime symptom**: every `/dash/v1` query would filter by a tenant no row carries, so the list is empty and the detail read is a `404`, which is exactly what a merchant with no payments looks like. (2) `ConfigError::MerchantClaimsDashboardAudience` — a **merchant** registration listing `vpay:dash/v1` in `allowed_audiences`. **This was an open hole, not a hypothetical**: `handle_client_credentials` mints a token for any requested audience a registration permits, and nothing restricted what a merchant could list beside `vpay:v1`, so one YAML line would have handed a merchant credential a token the `/dash/v1` validator accepts. It was harmless only for as long as `/dash/v1` mounted nothing, and it was closed on the day it stopped being. `Surface::Dashboard.audience()` also stopped being a local literal and now returns the new `vpay_config::DASHBOARD_AUDIENCE`, beside `MERCHANT_AUDIENCE`, because there are now three parties that must agree on the string |
| Resource-server JWT validation (`vpay-api::resource_auth`, OP-3) | 🟡 | New this pass: `JwtValidator`, pinned per `Surface` (`Merchant` or `Dashboard`, distinguished by required `aud`), backed by `authkestra_resource::jwt::JwksCache` — fetched once and cached for `jwks_refresh_interval`, not a network round trip per request (confirmed by reading `authkestra-resource-0.3.4`'s own source, cited in the module doc, and re-confirmed unchanged at `0.7.1`: `JwksCache::get_key` still refreshes only on a cache miss or once the TTL has elapsed). `AuthenticatedMerchant`/`AuthenticatedDashboard` are axum extractors that pull a bearer token, validate it, and hand a handler `ResourceClaims { client_id, scope }`. **A real vulnerability class found and fixed, not merely inherited from the library:** `jsonwebtoken::Validation::validate_aud` defaults to `true` but its own doc comment says the check "only happens if `aud` claim is present" — a token minted with no `aud` claim at all would sail through unchecked. Fixed with `set_required_spec_claims(&["exp", "aud", "iss"])`, which makes the claim's mere presence mandatory before the membership check runs, and proven by `a_token_with_no_audience_claim_at_all_is_rejected`. 11 tests in `resource_auth.rs` cover this plus: a validly-signed token round-trips its claims and scopes; a token signed by a different key (same advertised `kid`) is rejected; an expired token is rejected; a merchant-audience token is rejected by the dashboard validator and vice versa (both directions proven, not assumed from one); an unrecognized `kid` is rejected rather than falling back to any available key; and, over a real axum router, a missing/malformed `Authorization` header and a valid bearer token each produce the right status and Stripe-shaped envelope. Every failure mode collapses to the same generic `invalid_token` response (`AuthRejection::InvalidToken`), deliberately, so the endpoint cannot be used as an oracle for *which* check tripped. **Mounted 2026-09-02 (Step 1).** `AuthenticatedMerchant` is now the layer in front of the whole `/v1` nest (`vpay_api::router`, "HTTP surface" above), so this module is on the path of every merchant request, not only its own tests: `an_unauthenticated_v1_request_is_401_not_404` and `the_unauthenticated_v1_401_is_the_stripe_shaped_envelope` drive it over the real router, and `an_sdk_client_authenticates_and_reaches_the_honest_404` / `a_dashboard_audience_token_is_refused_on_v1` drive it over a socket against a booted `vpay-server`. The provisional `vpay:v1` string is gone: `Surface::Merchant.audience()` returns `vpay_config::MERCHANT_AUDIENCE`, so the validator and the config validation rule cannot disagree about the spelling. ~~`AuthenticatedDashboard` remains mounted on nothing, because `/dash/v1` does not exist.~~ **Half-corrected 2026-09-06 (exp23):** the extractor is indeed still mounted on nothing and stays that way, but the reason given was wrong within a day of being read — `/dash/v1` now exists (the row below), and `require_dashboard_token` validates through a `DashboardJwtValidator` in middleware rather than through the extractor, for the reason the merchant boundary stopped being an extractor in Step 2. This row and the "Dashboard auth" row above it disagreed until this amendment. **Still 🟡, for two reasons.** (1) The router-level tests that cover the mounted path are unit-level for the 401 and Docker-backed for everything past it, and the Docker-backed ones have not run under Docker (header paragraph). (2) **The validator fetches its JWKS over an HTTP round trip to this same process's own loopback port** — `vpay-server` binds first, then builds the validator with `loopback_jwks_url(bound)` (`the_validators_jwks_url_is_always_loopback_on_the_bound_port`, `the_validators_jwks_url_ends_at_the_route_the_router_mounts`, unit tests in the binary). It is always loopback, never the public URL, so no external dependency is introduced — but a process validating its own tokens by asking itself over TCP is a seam that exists because `authkestra_resource` offers no in-process key source, not because it is desirable. It also means the row below is no longer hypothetical. **Two findings from the security review, both fixed and pinned without Docker:** (1) an unauthenticated caller could force one loopback JWKS fetch (a Postgres `SELECT`) per request and hold the cache's write lock across it by presenting junk tokens with random `kid`s — `authkestra_resource`'s `JwksCache` refreshes on every miss; `JwtValidator` now decodes the header first (no `kid` → refused with zero cache access), delegates immediately for a `kid` already in the cached JWKS, and otherwise grants at most one refresh per `UNKNOWN_KID_REFRESH_INTERVAL` (30 s) per process — `a_hundred_unknown_kids_force_at_most_two_jwks_fetches` asserts wiremock saw ≤ 2 fetches for 100 junk tokens (101 with the throttle disabled), and `a_refused_token_does_not_spend_the_permit_for_a_good_one_on_the_same_key` pins that the predicate is membership of the published key set, not "validated before" — the stated cost is that a token signed by a key this replica has not yet fetched can be refused for up to 30 s during a junk burst; (2) a JWKS fetch failure (our own endpoint down because Postgres is down) was rendered as `401 invalid_token`, which the SDKs answer by re-authenticating — an outage amplifier; it is now `AuthRejection::KeysUnavailable`, `Category::Storage`, a 503 `service_unavailable` envelope with `Retry::AfterBackoff` (`a_jwks_that_cannot_be_fetched_is_keys_unavailable_not_invalid_token`, `a_jwks_outage_is_a_503_envelope_over_the_router`, with `a_bad_signature_is_still_a_401_over_the_router` as the control); every claim/signature/unknown-key failure still collapses to the oracle-free 401. Also pinned: a token the OP mints with no requested audience (`aud = client_id`) is refused on `/v1` (`a_token_whose_audience_is_the_client_id_is_refused_on_the_merchant_surface`, and the Docker-backed `a_token_minted_with_no_audience_is_addressed_to_the_client_and_refused_on_v1`) — the decisive mutation is *widening* `set_audience`, not deleting it, since `jsonwebtoken` 11 fails closed on a missing audience list. **Found by the first `just demo` run, not by any test (2026-09-02):** inside the `FROM scratch` image `vpay-server` panicked at boot — `JwksCache::new` builds `reqwest::Client::new()`, which on the workspace's reqwest 0.13 pin loads trust roots from the OS store the image does not have (`No CA certificates were loaded from the system`), exactly the failure the root `Cargo.toml`'s comment on that pin predicted. The prescribed fix (`JwksCache::with_client`) was **not sufficient**: `with_client` replaces a client `new` has already constructed, and 0.7.1 (the latest release) has no other constructor. So `vpay_api::jwks_cache` is a deliberate, narrowed port of `authkestra_resource::jwt::JwksCache` + `validate_jwt_generic` (~15 lines of refresh policy; every cryptographic step still calls authkestra's `Jwks::fetch_with`, `Jwk::to_decoding_key` and `jsonwebtoken::decode`), taking the client as a constructor argument, and `vpay_api::http_client::client()` builds that client on vendored `webpki-roots` + `ring` via `tls_backend_preconfigured` — the twin of `sdks/rust`'s `rustls_client_config`. The module doc lists the deviations and the re-diff obligation on an authkestra bump; the clean answer is an upstream constructor that takes a client, after which the port can be deleted. **Proven three ways:** `a_server_with_no_os_trust_store_boots_and_still_validates_tokens` in `vpay-server`'s `tests/cli.rs` spawns the real binary with `SSL_CERT_FILE`/`SSL_CERT_DIR` pointing at nothing and asserts `/healthz` 200 and a bogus-`kid` `/v1` request answering the 401 envelope (a 503 would mean the fetch failed) — it fails with the original panic when `http_client::client()` is replaced by `reqwest::Client::new()`; the real image booted and answered the same two requests under `docker compose` on the authoring machine; and the CI `e2e (compose)` job exercises the same path. **Latent, stated:** `authkestra-engine` still writes `reqwest::Client::new()` in its device-flow, client-credentials-flow and captcha modules — none reachable from vpay today; if one ever becomes reachable it panics in the image the same way, and `install_crypto_provider` does not prevent that. **Remediation review, later the same day:** the port's `get_jwks` refresh re-checks the entry under the write guard (`refresh_if_stale`), so waiters that queued behind the first refresh at a TTL boundary reuse its result instead of fetching again — a fifth deviation from upstream, documented in the module. Measured before claiming: with the re-check deleted, 32 callers released on a barrier produced 1 extra fetch on 17 of 20 boundaries and 2 on 3, never 32 — `tokio`'s write-preferring `RwLock` was already doing most of the coalescing — so this removes an occasional redundant `SELECT` taken while every validation is queued, not an N-fold amplification. The concurrent form of the test passes with the bug present most of the time and was deliberately **not** shipped; `a_caller_that_reaches_the_refresh_with_a_fresh_entry_does_not_fetch_again` pins the property deterministically (1 fetch with the re-check, 2 without). **Changed 2026-09-03 (Step 2): validation happens exactly once per request (D3).** The `/v1` boundary was an *extractor* (`AuthenticatedMerchant`), which meant every handler that wanted claims paid for a validation, and a handler that forgot to ask for it was unauthenticated by omission. It is now a **middleware** on the nest: it validates the bearer token once, checks the method's required scope, resolves the tenant, and puts a `MerchantScope` in request extensions. `MerchantScope`'s only public constructor is the middleware, and its `FromRequestParts` fails closed with a `500` rather than falling back to any tenant if the middleware is not mounted — reaching a handler with no scope means the layer is missing, and the safe answer to "whose rows may I read" is none of them. The extractor's own test suite is unchanged and still passes (`a_valid_bearer_token_reaches_the_handler_with_claims_attached`, `a_token_without_the_required_scope_is_403_not_401`, and the JWKS-throttle cases above); what is new is that a real resource path depends on it |
| rustls `CryptoProvider` process default, for `authkestra_resource::jwt::Jwks::fetch` | ✅ | **Closed 2026-09-02.** Both `vpay-server` and `vpay-worker-bin` now call `rustls::crypto::ring::default_provider().install_default()` (`install_crypto_provider()` in each `main.rs`) as the second thing in `run()`, after the signal handlers and before tracing init, so no client construction can precede it. The result is `.ok()`-dropped on purpose — `Err` means a default already exists, which is the wanted state — per the root `Cargo.toml`'s own note on the `authkestra-*` pins; no `unwrap`/`expect`. **What the ✅ rests on:** a unit test per binary (`installing_the_crypto_provider_leaves_a_process_default_and_is_idempotent`) asserts `CryptoProvider::get_default()` is `Some` afterwards and that a second call does not panic — emptying the function's body fails both. The existing exit-69 subprocess tests spawn the real binaries through this call and on to the database stage, so it is exercised on a real startup. **Updated 2026-09-02 (Step 1): the path that used to panic now runs in a shipping process.** `vpay-server` builds a `JwtValidator` at startup, and the first authenticated `/v1` request makes it fetch its own loopback JWKS — a real `Jwks::fetch`, not a test one. **What proves it, and how strongly:** `an_sdk_client_authenticates_and_reaches_the_honest_404` and the rest of `backends/tests/integration/tests/merchant_token_flow.rs` boot the real router in-process and complete that fetch; the test binary installs the provider itself at the top of `harness()` for exactly the reason this row exists, and its own comment says so. That is an in-process exercise of the fetch, and it has run once, manually, against a scratch database — **never under Docker or in CI** (header paragraph). It is therefore stronger evidence than this row had before and weaker than "a shipping `vpay-server` container has served an authenticated request": the CI `e2e (compose)` job boots `vpay-server` but its Cypress spec only touches the dashboard, so no containerised `/v1` request has ever been made. The rail adapters are still `NotImplemented`, so no rail client has ever been built. The previous row's analysis — that `vpay-db` never needed this because sqlx builds its own provider inline, and that `sdks/rust` sidesteps it with a pre-built `ClientConfig` — is unchanged and still correct. **Scope narrowed 2026-09-02:** the JWKS client this row was written for is now built by `vpay_api::http_client` from a pre-configured rustls `ClientConfig`, so it no longer consults the process default at all (see the row above); the `install_default()` call stays in both binaries because `authkestra-engine`'s own `reqwest::Client::new()` call sites would need it if ever reached, and because it costs nothing |
| Webhooks (signing, outbox, delivery) | 🟡 | **Changed 2026-09-03 (Step 5): both transactions of [flows/webhooks.md](flows/webhooks.md)'s outbox run, and a signed event has been delivered and verified.** TX 1 was Step 4's (Events row below). TX 2 is `vpay_worker::webhooks::handle_fan_out`, the `fan_out_events` job — a singleton `fanout:events`, seeded by `run_loop::seed_singletons` beside `sweep:expired`, `scan:live` and `scan:deliveries` and rescheduled every 5 s — which per event, in **one transaction**, inserts a `webhook_deliveries` row per configured endpoint (migration `0022`), enqueues one `deliver_webhook` job per row and flips `fanout_state` to `done`. Re-running it creates nothing: `webhook_deliveries_event_endpoint` and `jobs_dedupe_key` absorb the replay (`fan_out_creates_one_delivery_and_one_job_per_endpoint_and_is_idempotent`), and a merchant with **zero** endpoints is still flipped to `done` or the partial index `events_pending_idx` grows forever (`an_event_for_a_merchant_with_no_endpoints_is_still_fanned_out`). `handle_deliver` renders through `vpay_api::model::EventObject` — the *same* renderer `GET /v1/events` returns — signs those exact bytes with HMAC-SHA256 over `"{t}.{body}"` and POSTs them with `Vpay-Signature`, `Stripe-Signature` and `Vpay-Event-Id`. **What is proven about `Stripe-Signature` is byte-identity, grammar and — since Step 5b — verification by the real `stripe` package**: an integration test asserts the two headers carry the same string and that it matches Stripe's documented `t=…,v1=…` shape, `just demo`'s step 7 fails hard if they ever differ, and `sdks/stripe-compat`'s `webhooks.compat.test.ts` takes a delivery out of the WireMock receiver's own request journal and puts the recorded bytes and `Stripe-Signature` through `stripe.webhooks.constructEvent`, requiring `StripeSignatureVerificationError` for a body with one byte flipped and for the right body with the wrong secret. So "a Stripe-shaped handler works unmodified" is an observation now rather than an argument from the scheme being identical. The body is not stored; `payload_sha256` is written by the first attempt that **rendered and signed a body** and compared on every later one, and a mismatch is `JobError::Poisoned`. Not "the first attempt", which is what `0022`'s `COMMENT` said and what `0024` corrects: the unconfigured-endpoint branch records a failed attempt having signed nothing and stores no digest, so a delivery can reach `attempt > 0` with the column still `NULL`. A transport failure does store one — those bytes were signed before the socket was ever opened. Retries walk `vpay_worker::delivery_delay` (10s → 30s → 2m → 10m → 1h → 6h → 24h) and **never** `JobError::decision` — a merchant's 500 is not a `ProviderError`; the eighth failure is `state = 'exhausted'` with `alert = true` and no further rung (`the_ladder_walks_delivery_delay_and_then_succeeds`, `a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled`). **The signature is proven against the SDKs a merchant actually installs, not against a second copy of the HMAC:** the emitted header is fed to `vpay_sdk::webhooks::verify_at` (`the_delivered_signature_verifies_with_the_shipping_rust_sdk`, which also flips one byte of the recorded body and requires `SignatureMismatch`) and to the built `@vaam-apps/vpay-sdk` in a `node` subprocess (`the_delivered_signature_verifies_with_the_shipping_node_sdk`, which **fails rather than skips** when `node` is missing; CI sets `VPAY_REQUIRE_NODE=1`). Two secrets produce two `v1=` values and either verifies (`a_rotation_signs_with_both_secrets_and_either_one_verifies`). Endpoints are YAML (`merchant_clients[].webhooks[]`), keyed for fan-out on `merchant_id` and not on `client_id`, with an operator-authored `id` refused as a duplicate at boot, and secrets under both the livemode literal-secret rule and a 32-byte livemode floor. **URL validation is `vpay_config::validate_webhook_url` (2026-09-03), not the rails' `validate_host`:** the URL is parsed once, must name a host and carry no userinfo **in both deployments** (a sandbox `mailto:x` used to boot and fail as a delivery walking the ladder), and under livemode the scheme must be `https` — compared as a scheme, so `HTTPS://Hooks.Example/x` is accepted where `starts_with("https://")` refused it — and the four stub substrings are searched in the **host only**, so a merchant's own `https://hooks.example/mockups` is accepted and `https://mock.example/x` is not. `validate_host` is unchanged for rail hosts. Four fixtures pin the corrected cases (`a_livemode_endpoint_may_be_uppercase_and_may_have_a_stub_word_in_its_path` plus three rows of `every_webhook_endpoint_rule_refuses_its_own_fixture`), and the length/shape bounds are still boot-time. `just demo`'s step 7 ends by verifying a real delivery out of the WireMock receiver's own request journal, and `the_real_run_loop_delivers_a_backlog_event_to_the_receiver` drives a delivery through the **real `run_loop`** — seed, claim, dispatch, send, settle — rather than calling the two handlers directly, so "a handler no loop calls" is no longer the gap it was when these tests were written. It begins from an inserted `events` row; `worker_e2e.rs` and the demo are what join a real confirm to it. **🟡, not ✅, and for six reasons.** (1) Every delivery so far was to a WireMock host on a compose network; **no merchant endpoint has ever been POSTed to.** (2) **The runtime egress guard landed in Step 8 and this reason is retired** — `vpay_worker::ssrf` resolves each endpoint's host once, refuses every loopback, private, link-local, CGNAT, multicast or otherwise non-public address (both families, mapped forms included) and pins the connection to the addresses it classified, so the TOCTOU that made a resolve-then-connect check worthless is closed without a custom connector. What is left is a *scope* limit, not an absence: the guard is on webhook delivery only, a NAT64 receiver is refused fail-closed, and the pin costs the shared connection pool. See this file's own row for it. (3) There is **no replay endpoint and no CLI** — re-sending an `exhausted` delivery is a hand-written transaction against `psql` (flip the row to `pending` *and* re-insert the `deliver_webhook` job; the row alone would sit until the next backstop pass). Written down 2026-09-03 in [runbooks/webhook-delivery-failures.md](runbooks/webhook-delivery-failures.md), whose SQL — including the replay transaction, run twice to confirm the second run is a no-op — was executed against a `postgres:16-alpine` with every migration through `0022` applied. **The runbook now exists; nobody has followed it against a deployment, and no replayed delivery has been observed reaching a receiver.** (4) The delivery ladder's later rungs (1h, 6h, 24h) are asserted as *values* by a unit test and have never elapsed in a running deployment. (5) **Recovery of a lost delivery job takes up to ten minutes, and an `exhausted` row is recovered by nothing.** `scan:deliveries` (`JobKind::ScanDeliveries`, migration `0023`) runs every 10 minutes over up to 500 rows and re-enqueues a `deliver_webhook` job for each `pending` delivery it finds — both arms: `next_attempt_at` in the past, **and** never-attempted rows older than `RecoveryPolicy::lease`, the lease being what stops the scan racing the queue on a row whose job was written in the same transaction and simply has not been claimed. So a delivery job that was **deleted**, or lost to a `jobs` truncation, no longer strands its row (`the_backstop_re_enqueues_a_delivery_whose_job_vanished`, `pending_due_returns_the_deliveries_nothing_is_driving`). **It does not recover a *dead-lettered* delivery job, and will not try** — `jobs::dead_letter` parks the row at `run_at = 'infinity'` and keeps its `dedupe_key`, so the scan's `ON CONFLICT DO NOTHING` insert is a no-op for exactly those deliveries; a `deliver_webhook` job is parked only for a `Poisoned` reason that retrying cannot fix, and un-parking on a timer is the hot loop parking exists to prevent. What the scan does instead is emit one `WARN` per pass naming those deliveries, so the state has an observer other than an operator running `SELECT * FROM jobs WHERE run_at = 'infinity'`; un-parking is a manual `UPDATE` (runbook). `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan` pins all three halves — the job stays parked, the delivery stays `pending`, the `WARN` names it — and it was the documentation, in six places, that claimed otherwise until 2026-09-03. A pass that *fails* now logs `alert = true` before returning the error, because a backstop nobody notices has stopped is a backstop that is not there. It also cannot touch an `exhausted` row, which is not `pending` — that is still the runbook's manual transaction — and it is a *backstop*: in a healthy deployment it finds nothing, and a steady stream from it means the enqueue is broken. **(6) A permanently unfannable event is abandoned after five passes, and the merchant is never told.** A per-event fan-out failure now increments `events.fanout_attempts` in its own statement (migration `0024`; the event's own transaction has rolled back, so a counter inside it would roll back too) and the fifth — `vpay_worker::FANOUT_MAX_ATTEMPTS` — sets `fanout_state = 'failed'`, which leaves `events_pending_idx` and `events::pending_page`. Before that each failure is a `WARN` with **no** `alert`; the transition is exactly one `ERROR … alert = true`. That is the whole point: a `pending` event heads every subsequent page, so the old shape alerted every five seconds per poisoned event and held one of the drain's hundred slots forever — a hundred of them stopped webhooks for every merchant. A page of 99 poisoned events now costs 99 alerts in total. Proven end to end against a real Postgres by `a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once` (five passes over one event whose 65-character `endpoint_id` a real CHECK refuses: `fanout_attempts = 5`, `fanout_state = 'failed'`, gone from `pending_page`, exactly one `alert=true` in the whole capture and four warnings before it) and, for the single-failure end of the same ladder, by `one_merchants_unfannable_event_does_not_block_another_merchants` (one pass, `fanout_attempts = 1`, still `pending`, **no** alert). **Nothing re-arms a `failed` event** — that is a deliberate `UPDATE` after the cause is fixed, in the runbook, and no deployment has ever produced one |
| Webhook signing (`Vpay-Signature`, `Stripe-Signature`) — `vpay_worker::signing` | ✅ | **New 2026-09-03 (Step 5).** `signature_header(body, now, secrets)` writes `t=<decimal unix seconds>,v1=<hex>[,v1=<hex>]` — HMAC-SHA256 over `t_text || "." || body`, lowercase hex, one `v1=` per configured secret in configuration order. ✅ because the proof is not a second copy of the HMAC: the emitted header is handed to `vpay_sdk::webhooks::verify_at`, the verifier a merchant installs (`one_secret_verifies_through_the_sdk`), and separately to the built `@vaam-apps/vpay-sdk` over a real delivery (see the Node parity row). **`t` is signed as the literal text written into the header**, never a re-rendered number — `the_t_written_is_the_t_signed` pins it, because a sender whose `t` did not round-trip would produce genuine deliveries every merchant silently rejects. Decisive negatives, all present: a tampered body does not verify (`a_tampered_body_is_refused`), a secret that was never configured does not (`each_of_two_secrets_is_independently_accepted`), every signature is 64 lowercase hex characters (`every_signature_is_64_lowercase_hex_characters`), and an endpoint with no secrets yields `t=…` alone, which every verifier calls malformed (`no_secrets_yields_a_header_carrying_no_signature`) — `handle_deliver` refuses to send in that case rather than relying on it. The same string goes out under **both** header names, and **since Step 5b the Stripe-SDK half of that claim is observed rather than argued**: `sdks/stripe-compat/src/webhooks.compat.test.ts` takes a real delivery out of the WireMock receiver's request journal and hands its recorded body and `Stripe-Signature` to the real `stripe` package's `stripe.webhooks.constructEvent`, then requires a `StripeSignatureVerificationError` for a body with one byte flipped and for the right body with the wrong secret. No constant-time comparison here and none needed: this module produces a value and never compares one, which is why the worker does not depend on `subtle` while the SDK does |
| Webhook delivery ladder (`vpay_worker::delivery_delay`) | 🟡 | **New 2026-09-03 (Step 5).** `[10, 30, 120, 600, 3_600, 21_600, 86_400]` seconds — 10s, 30s, 2m, 10m, 1h, 6h, 24h — returned as `Option<Duration>`, `None` past the seventh rung. `Option` and not `Duration` on purpose: "the ladder ran out" is the `exhausted` transition and must not be expressible as another rung. Transcribed rung by rung from [flows/webhooks.md](flows/webhooks.md) **in the document's units** rather than reused from the implementation's array, so the test asserts the document (`the_delivery_ladder_is_the_documented_one`); `the_delivery_ladder_ends_after_seven_rungs` pins `delivery_delay(7) == delivery_delay(u32::MAX) == None` and that rung 6 is still a real delay, so a lost last rung fails. It is **not** [`poll_delay`] and **not** `JobError::decision`: delivery has no rail vocabulary, and pushing a merchant's `500` through the poll ladder's decision table would give a receiver the *rail's* escalation policy. Driven end to end by `the_ladder_walks_delivery_delay_and_then_succeeds` (WireMock answers `500` three times then `200`; `attempt` counts up and each `next_attempt_at` delta is `delivery_delay(attempt_before)`). The whole ladder is **8 POSTs over about 31 hours** — the first attempt plus seven retries, 112,360 seconds of waiting — and **every non-2xx walks all of it, `4xx` included**: a receiver answering `410 Gone` is retried for 31 hours exactly as a `500` is, which is Stripe's behaviour and is deliberate (a `404` from a receiver mid-deploy is indistinguishable from one that means "stop"). **🟡: the 1h, 6h and 24h rungs are asserted as values only.** No deployment has waited one out, so the alert this ladder exists to produce has never fired outside a test that pre-sets `attempt` |
| `webhook_deliveries` (migration `0022`) — `vpay_db::webhook_deliveries` | ✅ | **New 2026-09-03 (Step 5).** One row per `(event, endpoint)`, unique on `webhook_deliveries_event_endpoint` — which is what makes the fan-out's `ON CONFLICT DO NOTHING` legal and "one delivery per event per endpoint, forever" a property of the schema rather than of whichever pass ran first (`a_second_delivery_for_one_event_and_endpoint_is_not_created`). `endpoint_id` is the operator-authored YAML id stored verbatim and **references no table**: endpoints are configuration (ADR-0003), and a corrected URL must keep its delivery history. `state` is `pending`/`succeeded`/`failed`/`exhausted` at the database (`state_is_known`), and **`failed` is in the vocabulary and nothing writes it** — a failure with a rung left stays `pending`, because that is what says another attempt is owed; a `failed` row means something outside the worker wrote it. `exhausted` is written only by `record_attempt(..., exhausted = true)` together with `next_attempt_at = NULL`, so the row is findable in a runbook and invisible to `pending_due` (`record_attempt_bounds_the_excerpt_moves_the_ladder_and_then_exhausts`). Every write is a compare-and-swap guarded on `state = 'pending'`, so a replayed job cannot walk `attempt` past the end of the ladder and a second `record_success` writes nothing (`record_success_settles_a_delivery_once_and_a_second_call_writes_nothing`). `payload_sha256` is `COALESCE`d, so the **first** attempt's digest survives — and `record_attempt` takes it as an `Option`, because a failure on the unconfigured-endpoint branch **sent nothing** and there is no "the bytes we sent" for the column to be the digest of. Stamping it there would leave every later mismatch check comparing against a body no receiver ever saw (`a_delivery_with_no_configured_endpoint_records_a_failure_and_no_digest`). There is deliberately **no** CHECK pairing `status_code`/`responded_at` with `sent_at`: a transport failure has a `sent_at` and neither of the others, and that is a real, distinct state the row must not blur. The migration is applied to a real Postgres and its CHECKs are fired by `migration_0022_reopens_the_job_kinds_and_closes_the_delivery_states` |
| Outbox drain — the `fanout:events` singleton | 🟡 | **New 2026-09-03 (Step 5).** `JobKind::FanOutEvents` with `dedupe_key = "fanout:events"` (pinned as a literal, `jobs.rs`), seeded by `run_loop::seed_singletons` **in one transaction with `sweep:expired`, `scan:live` and `scan:deliveries`** at every worker boot, `ON CONFLICT DO NOTHING` — so restarting a worker re-seeds a lost singleton, and a partial seed is impossible. `run_at = now()` and not a delay, because an event written before this process started is already waiting. Each pass reads `events::pending_page(pool, 100)` in `seq` order, and reschedules itself **immediately** when the page came back full, otherwise after 5 s — so a backlog drains at full speed without any single pass being unbounded. **One bad event no longer stops the page:** a failure on a single event is logged at `ERROR` with `alert = true`, naming the event, its merchant and its type and no secret, and the pass continues; the page closes with a `WARN` counting drained against failed. The failing event keeps `fanout_state = 'pending'` and the next pass retries it, and a pass that drained **nothing** waits the idle interval instead of rescheduling immediately, so a page of failures cannot spin. The earlier shape returned the error and abandoned the rest of the page, which let one merchant's unfannable event hold every other merchant's webhooks behind it (`one_merchants_unfannable_event_does_not_block_another_merchants`). **🟡 for two reasons.** The seeding itself is asserted only through its consequences: `worker_e2e.rs`'s `wait_for_fanout` proves the loop that settles a charge is the loop that drains its outbox, `the_real_run_loop_delivers_a_backlog_event_to_the_receiver` drives the **shipping `run_loop`** — the same function `vpay-worker-bin`'s `main` calls — from an unfanned-out `events` row to a POST in the receiver's journal, which is the seam nothing else covers, and the webhook suite claims the seeded job by its dedupe key (`claim_fanout_job`). **That test starts from an inserted `events` row, not from a real confirm**: `worker_e2e.rs` and `just demo`'s step 7 are what cover settlement-to-webhook end to end. And no test asserts that `seed_singletons` writes *four* rows, so a fifth singleton dropped from it would be caught only by whatever end-to-end test happened to notice the consequence. And the drain is a 5-second **poll**, not `LISTEN/NOTIFY`: that is deliberate (it must also pick up events written by a process that has since died, which a notification would not deliver), but it means the floor on webhook latency is five seconds plus the send, and nothing measures it |
| Events read API (`GET /v1/events`, `GET /v1/events/{id}`) — `vpay_api::v1::events` | 🟡 | **New 2026-09-03 (Step 5); `/v1/events` was on the "deliberately not mounted" list until this pass.** Read-only, merchant-scoped, newest first, using `payment_intents`' cursor machinery to the letter (`limit` default 10 capped at 100, `starting_after`/`ending_before`, not both) — deliberately the same `ListPage` type and not a second copy, because two list endpoints whose paging disagrees is a difference a merchant discovers by writing a loop that silently skips objects (`events_list_page_walks_forward_and_backward_over_twenty_five_events`, `events_are_listed_newest_first_scoped_to_the_merchant`). Scope is authorisation: `vpay_db::Events::list_page` takes the merchant id as a required argument and **has no unscoped variant**, so an unfiltered read of every merchant's payment history does not compile rather than merely being unwritten; a foreign id is the same `404` a nonexistent one gets, byte for byte (`events_get_by_id_is_merchant_scoped`), and a token without a scope is refused before a handler runs (`reading_events_requires_a_scope`). It renders through the **same** `EventObject` the deliverer signs, which is the whole point of the row: this list is the documented fallback for a missed webhook, and two renderers would let it answer a different question. The filter is **`merchant_id` only** — `livemode` is not part of the query. That is correct today because `livemode` is a *deployment* setting (`vpay_config::Deployment`) and one deployment is one livemode, so there is nothing to separate; a deployment that ever served both would leak test events into a live listing, and this row is where that is written down before it happens. There is no `POST /v1/events` and there will not be one — a merchant who could create an event could forge their own history. **🟡: `?type=` is documented in [api/README.md](api/README.md) and not implemented** — it is *ignored*, not refused, exactly as every handler on this surface ignores unknown query parameters, so a caller who filters gets an unfiltered page and no error (decision 5 of the Step 5 plan: a filter has to be computed into `has_more` and the `seq` window or paging silently skips rows, and half of that is worse than none) |
| Webhook endpoints in YAML (`merchant_clients[].webhooks[]`) — `vpay_config::oauth::WebhookEndpoint` | ✅ | **New 2026-09-03 (Step 5).** `{ id, url, secrets: [...] }` per merchant client, `#[serde(default)]` so a merchant with none is valid (`a_merchant_with_no_webhooks_configured_is_valid`). Five rules, each refused at boot with its own `ConfigError` and each covered by its own fixture (`every_webhook_endpoint_rule_refuses_its_own_fixture`): a missing/blank `id` (`WebhookEndpointMissingId`), a duplicate `id` **within one merchant** (`DuplicateWebhookEndpointId` — fatal rather than deduplicated, because two endpoints sharing an id collide on `webhook_deliveries_event_endpoint` and exactly one would ever be delivered to), a URL that names no host in either mode or fails `validate_webhook_url` under livemode, zero or three-plus secrets (`WebhookSecretCount`, `1..=2`), and a resolved secret that is empty (`EmptyWebhookSecret`) or, under `livemode`, shorter than **32 bytes** once resolved (`WeakWebhookSecret`, `a_livemode_webhook_secret_below_the_floor_is_refused`) — an HMAC-SHA256 key below the hash's own 32-byte output adds nothing over one at it and makes offline guessing cheap. That floor and the literal-secret rule are a **pair**, and neither substitutes for the other: the first reads the file's text and says the secret came from the environment, the second reads the resolved value and says the environment holds something worth having — `${MERCHANT_WEBHOOK_SECRET}` is a placeholder of fixed length whatever it holds, so only the resolved value can answer it. In sandbox both are off and the rule is "not blank", which means a rotation rehearsed in sandbox with a short value is refused in livemode. **`secrets` is covered by the livemode literal-secret rule**, which is the gap S3 of the Step 5 plan named: `RawSecrets` now walks the **unresolved** document for `merchant_clients[].webhooks[].secrets` as well as `providers[].credentials`, and `validate_secret` is asked about the value *as written* — because after resolution a correct `${VAR}` and a literal are the same string. A position the table cannot account for fails closed as `""`. `${MERCHANT_WEBHOOK_SECRET}` is now a required variable for **both** binaries (exit `78` unset; `vpay-server` never delivers a webhook but validates the same document — see [flows/configuration.md](flows/configuration.md)). `WebhookEndpoint` hand-writes `Debug` to redact `secrets` to a count while keeping the id and URL, and so does the worker's own `Endpoint`/`EndpointRegistry` — both because the table is held for the process's whole life and a secret in a log is a forged webhook (`a_webhook_endpoints_debug_output_never_contains_a_secret`, `debug_never_prints_a_secret`). The registry is keyed on `merchant_id`, **not** `client_id`: one merchant may hold several OAuth clients, and their endpoints are merged rather than overwritten (`two_pairs_naming_one_merchant_are_merged`, `a_merchants_endpoints_are_found_and_another_merchants_are_not`) |
| Webhook URL validation (boot) and the runtime egress guard | 🟡 | **Changed 2026-09-04 (Step 8, lane B): the ⛔ this row used to carry is closed, and what remains is narrower.** Boot-time validation is unchanged and is still not SSRF protection: the `id` is 1–64 characters and the `url` 1–2048 in **both** modes, counted exactly as migration `0022`'s CHECKs count them (`the_length_bounds_are_migration_0022s`), the URL must parse, must carry no embedded credentials and must **name a host in both modes**, and `vpay_config::validate_webhook_url` adds exactly two livemode rules — the scheme must be `https` (compared as a scheme, so `HTTPS://Hooks.Example/x` is accepted) and the **host** must contain none of `wiremock`, `stub`, `mock`, `localhost` (so `https://hooks.example/mockups` is accepted and `https://mock.example/x` is not). None of that inspects an address, and it cannot: an address is not a property of the configuration. **The address is now checked at delivery, on every attempt, by `vpay_worker::ssrf`.** `handle_deliver` parses the URL, refuses any scheme but `http`/`https`, resolves the host **once** (`tokio::net::lookup_host`), classifies **every** address the lookup returned — loopback, unspecified, RFC 1918, IPv6 unique-local `fc00::/7`, link-local `169.254/16` and `fe80::/10`, CGNAT `100.64/10`, multicast, broadcast, `0.0.0.0/8`, `240/4`, the IANA special-purpose IPv4 blocks (including `192.88.99.0/24`, the 6to4 relay anycast RFC 7526 deprecated), every IPv6 address outside global unicast, and the special-purpose prefixes inside it — 6to4 `2002::/16`, Teredo `2001::/32`, IETF protocol assignments `2001:1::/32`, benchmarking `2001:2::/48`, ORCHIDv2 `2001:20::/28` and documentation `2001:db8::/32`, and **the IPv4-mapped and IPv4-compatible IPv6 spellings of all of them** — and then builds the delivery client with `reqwest::ClientBuilder::resolve_to_addrs` pinned to exactly those addresses, so the name is never resolved a second time and a DNS rebind between check and connect has nothing to rebind. Redirects stay refused and the proxy environment stays ignored (`vpay_provider::http`), which is what stops a `302` leaving the pin. A refused target is a **permanent** failure — `state = 'exhausted'` on the first attempt, no next attempt, `response_excerpt` beginning `ssrf_blocked: ` and naming the address *class* but never the address, and exactly one `ERROR … alert = true` — while a host that merely fails to **resolve** stays an ordinary failed attempt on `delivery_delay`, because a resolver blip must not cost a merchant an event. `webhooks.allow_private_targets` (default `false`; `vpay_config::WebhookPolicy`) is the one value that changes that verdict, `config/application-sandbox.yml` and the generated `demo` overlay set it `true` because their receiver is a compose service, and **livemode plus `true` is a refusal to boot** (`ConfigError::PrivateWebhookTargetsInLivemode`, `a_livemode_deployment_may_not_allow_private_webhook_targets`). Proven by 9 unit cases over every range in both families including the mapped forms (`vpay_worker::ssrf::tests`, listed on this tree; the refused table grew six rows on 2026-09-04 for the four prefixes Step 8's review found missing — finding 6 — and the deliverable table gained `192.88.98.255`/`192.88.100.1`, so the `/24` is pinned from both sides) and by two container-backed cases against a real receiver: `a_delivery_to_a_private_address_is_refused_permanently_and_delivered_when_allowed` (the **same** address, both verdicts, and the receiver's own request journal holding exactly one POST — the allowed one) and `a_host_that_resolves_to_a_private_address_is_refused_and_an_unresolvable_one_retries` (a *name*, refused after resolution, and `.invalid` walking the ladder instead). Bypassing the classifier makes both fail with the delivery `succeeded` — that revert was run by lane B, and restored. **🟡 and not ✅ for four named residuals:** the guard is on webhook delivery only and not on the rail adapters (their hosts are operator-configured, not merchant-supplied); a receiver behind NAT64 (`64:ff9b::/96`) is refused even when the embedded IPv4 is public, which is fail-closed and unproven against any real receiver; pinning costs the shared connection pool — each delivery now builds its own client (4.0 µs, measured by lane B) and re-handshakes, which nothing has measured under load; and, **added 2026-09-04 (Step 8's correctness review, finding 5, deliberately not fixed): a refused delivery is exhausted on its first attempt and there is no replay path**, so a transiently poisoned DNS answer destroys the event permanently — "fail closed" and "destroy the event" are the same thing while replay is a hand-written transaction in the runbook. The remedy is a merchant-visible state-machine decision and is left to whoever owns [flows/webhooks.md](flows/webhooks.md), which carries lane H's recommendation. **And, standing over all four: no deployment has ever refused a real merchant's endpoint.** The evidence is the container-backed suite and the revert proof, not production |
| Node SDK signature parity (`@vaam-apps/vpay-sdk`, `VPAY_REQUIRE_NODE=1`) | ✅ | **New 2026-09-03 (Step 5).** `the_delivered_signature_verifies_with_the_shipping_node_sdk` takes the body and `Vpay-Signature` **off a real WireMock receiver's request journal** and feeds them to the built `sdks/nodejs` verifier in a `node -e` subprocess, asserting the returned event's `id` and `type`; the decisive negative runs through the same subprocess and requires the wrong secret to be refused. It is not a fixture, on purpose: feeding `@vaam-apps/vpay-sdk` its own vectors would prove the Node SDK agrees with itself, and the two verifiers have genuinely different parse paths (Node checks `t` with `/^\d+$/` over the literal text, Rust parses a checked `i64`), so the Rust test cannot prove Node's. **A missing `node` is a failure, never a skip** — `VPAY_REQUIRE_NODE=1` (set in CI's `rust` job, and in the workspace run this note cites) makes the message say so. That gate is what makes ✅ mean something here: a skip is exactly how this suite would go green while proving nothing. **What it does not prove:** nothing else in the Node SDK — the handshake, the resource calls — has ever reached a vpay |
| `just demo` — the delivered webhook, per outcome | 🟡 | **New 2026-09-03 (Step 5); the webhook check was then step 7 of seven.** Since Step 8 lane A the walkthrough is four steps and the webhook check runs inside step 4, once per outcome (`examples/merchant-demo/src/main.rs`, `[1/4]`…`[4/4]`). It polls the `wiremock-webhook` container's own request journal (`GET /__admin/requests`) for up to 30 s for a POST that carries a `Vpay-Event-Id` **and** whose body names this run's intent — both filters load-bearing, because the receiver answers `200` to anything and its journal outlives the run, so a previous run's delivery would otherwise satisfy this one. It then requires `Stripe-Signature` to equal `Vpay-Signature` byte for byte and verifies the recorded bytes with `vpay_sdk::webhooks::verify`, the call a merchant's handler makes; the body is used exactly as recorded and never re-serialised. **This step used to report an absent webhook and pass; it now fails on an absence, and fails louder on a delivery that does not verify** — at that point vpay is signing something a merchant cannot check, which is worse than sending nothing. The destination is configuration, not code: `just gen-demo-keys` writes the `webhooks:` block into `.e2e/application-demo.yml` and `compose.e2e.yml` runs the receiver, exactly as it runs the two rail stubs (ADR-0006). **The staleness check guarding that block was broken by Step 8 itself and was fixed on 2026-09-04 (review finding 3):** lane B added a *top-level* `webhooks:` key for `allow_private_targets`, `gen-demo-keys`'s `grep -q '^\s*webhooks:'` matched it, and an overlay that had lost the merchant's own indented endpoint list was therefore reported as current — after which this step fails against a receiver nothing points at, which is exactly what the check exists to pre-empt. It is now anchored on the indented `webhooks:` **and** on `url: http://wiremock-webhook`, and the reviewer's mutation was reproduced saying "already exist, keeping them" before the change and "missing the merchant's `webhooks` endpoint list — regenerating the pair" after it. **🟡: it was observed passing by the wiring pass and is cited on that pass's authority — this documentation pass did not re-run `just demo`** — and the receiver is a WireMock host, not a merchant. The demo has never run in CI. **Updated 2026-09-04 (Step 8, lane A): it is no longer one webhook but six**, one per outcome, and the verified event's `type` is now asserted against what that outcome must produce (`payment_intent.succeeded` / `payment_intent.payment_failed`) — so a run in which every payment was delivered as a success could not pass, which the single-outcome version could not have caught. Lane A observed six for six on 2026-09-04 and the journal paste is in `docs/runbooks/demo.md` §4. Still 🟡 for the unchanged reasons — the receiver is a WireMock host, not a merchant, the demo has never run in CI, and **it has not been re-run on the merged Step 8 gate branch**; that run is pending the VM gate |
| Webhook client budgets (5 s connect / 10 s request) — `vpay_worker::{WEBHOOK_CONNECT_TIMEOUT, WEBHOOK_REQUEST_TIMEOUT}` | ✅ | **Changed 2026-09-03 by the Step 5 remediation; this row said "a known wart, recorded rather than fixed" for the length of one commit and the record is kept honest by saying so.** The delivery client is `vpay_provider::http::client_with_timeouts(WEBHOOK_CONNECT_TIMEOUT, WEBHOOK_REQUEST_TIMEOUT)` — vendored roots, redirects refused, proxy environment ignored — built **once** at worker boot and cloned, because `reqwest::Client::new()` panics in the `scratch` runtime image and building one per job would throw away every pooled connection between attempts. The two durations were briefly written twice, in `vpay-worker-bin`'s `main.rs` and again in the integration suite's `delivery_client()` helper, with nothing pinning them together — so a change to the binary's budget would have left every webhook test exercising a client that no longer ships, and no test would have failed. They are now single `pub const`s in `vpay_worker::webhooks`, re-exported from the crate root, and **both** call sites read them, which is the same shape the rails use. The values are load-bearing for merchants and documented as such ([api/README.md](api/README.md), [flows/webhooks.md](flows/webhooks.md), and the receiver checklist in [runbooks/webhook-delivery-failures.md](runbooks/webhook-delivery-failures.md)): 10 s end to end is the whole budget a receiver has to answer in, which is why "acknowledge first, work later" is the advice |
| Idempotency | 🟡 | **New 2026-09-03 (Step 2); was ⛔.** `Idempotency-Key` is **required** on every `/v1` `POST` (stricter than Stripe, where it is optional; both SDKs already always send one), 1–255 printable-ASCII bytes, scoped to the merchant, stored 24 hours in `idempotency_keys` (migration `0015`). A request is identified by a SHA-256 over method, path and raw body, framed so the three cannot be shifted across each other; the stored digest is compared with `subtle::ConstantTimeEq` so the endpoint is not a hash oracle. The claim is one `INSERT … ON CONFLICT`, never check-then-insert. **The six behaviours and the test behind each, all of them run against a real Postgres on 2026-09-03:** a *replay* returns the stored body byte for byte and writes no second row (`a_replayed_idempotency_key_returns_the_same_object_and_no_second_row`, integration; `a_completed_idempotency_key_replays_its_stored_response`, `vpay-db`); a *mismatch* — same key, different body — is `400 idempotency_key_in_use` (`a_reused_key_with_a_different_body_is_the_400_envelope`; `reusing_an_idempotency_key_with_a_different_request_is_a_mismatch`); a key whose first request is *in flight* is `400 idempotency_key_in_flight`, its own code (`a_key_whose_first_request_is_still_running_is_answered_with_its_own_code`; `concurrent_claims_of_one_idempotency_key_yield_exactly_one_fresh`); a `5xx` *releases* the key so the retry re-executes (`a_5xx_releases_its_idempotency_key_so_the_retry_re_executes`; `release_hands_back_an_in_flight_key_and_never_a_completed_one`); an expired in-flight key is *reclaimable* while a live one is not (`an_expired_in_flight_key_is_reclaimable_and_a_live_one_is_not`), and a claim that was superseded by a reclaim can neither overwrite the new response nor delete the new claim — the ABA case, closed by a `claim_id` the database mints on every claim and that `store` and `release` both match on (`a_reclaimed_key_is_not_writable_by_the_claim_it_replaced`); and *sweep* deletes only rows past their window (`sweep_expired_removes_only_the_rows_past_their_window`). A missing header is the documented `400` naming `idempotency_key` (`a_post_without_an_idempotency_key_is_the_documented_400`), and a replay answers what the original answered even after the deployment changed underneath it (`a_replay_survives_the_rail_being_disabled`) — which is why `create` claims *before* validating and releases on a validation failure. **🟡, not ✅, for three named reasons.** (1) **The status code is `400` where Stripe answers `409`** for a key still in flight: [ADR-0011](adr/0011-error-modelling.md) derives the status from `Category`, `Category::Idempotency` is `400`/`idempotency_error`, and splitting one Stripe `type` across two statuses would be an ADR-level change. **That is a maintainer decision and this pass did not take it**; the `code` distinguishes the two cases either way, and `ApiError::IdempotencyKeyInFlight`'s doc comment records the trade. (2) ~~**Nothing sweeps on a schedule.**~~ **Closed 2026-09-03 (Step 4).** `vpay_db::Idempotency::sweep_expired` is the first statement of the worker's hourly `sweep:expired` job and no longer runs at `vpay-server` boot; the sweep's own correctness is unchanged (`sweep_expired_removes_only_the_rows_past_their_window`) and what is new is that it happens more than once per process lifetime. The dependency this introduces is stated in the Client-assertion replay row: no worker, no sweep. (3) It covers `POST /v1/payment_intents` and its two sub-resources only — there is no other `POST` on `/v1` to cover. **Extended 2026-09-03 (Step 5b): a replay now re-emits the response's `stripe-should-retry` too.** Migration `0025` adds `response_retry`, `store` persists the value the fresh response's own `HeaderMap` carried and `replay` writes it back — status, body **and** advisory, none of them re-derived. See the `stripe-should-retry` row for why the column holds text rather than a boolean and why `NULL` is not `'false'` |
| Job queue (`jobs`, migration `0021`) — `vpay_db::jobs` | ✅ | **New 2026-09-03 (Step 4).** One table, claimed with `UPDATE jobs SET locked_at = now(), locked_by = $1, attempts = attempts + 1 WHERE id = (SELECT id … WHERE run_at <= now() AND locked_at IS NULL ORDER BY run_at FOR UPDATE SKIP LOCKED LIMIT 1)`. `SKIP LOCKED` is what makes N workers take N different jobs instead of blocking on one row and then matching nothing; `eight_concurrent_claims_over_one_job_yield_exactly_one_claim` and `eight_concurrent_claims_over_eight_jobs_take_eight_distinct_jobs` prove both halves against a real Postgres, and `two_workers_claiming_together_never_take_the_same_job` proves it again through the real loop. **Every write that ends a lease is guarded on `locked_by`** (`finish`, `reschedule`, `set_payload`, `dead_letter`) — the same ABA close `idempotency_keys.claim_id` is — so a worker whose lease was reaped mid-run discards its answer instead of stamping it over the holder (`set_payload_writes_only_for_the_lease_holder`). `attempts` is incremented **by the claim**, so a job that kills its worker still counts up. **Lease expiry is a separate reaper, not a condition on the claim**, which is what keeps `jobs_claimable_idx` (`(run_at) WHERE locked_at IS NULL`) an exact index match rather than a scan of every leased row (`reap_expired_leases_frees_only_the_stale_lease`). A dead letter is a **park**, `run_at = 'infinity'` with the lease cleared and the reason in `last_error` — never a `DELETE`, so the row stays as the record that work stopped; `oldest_runnable_run_at` excludes parked rows so one dead letter cannot peg the queue-age gauge at infinity (`oldest_runnable_run_at_ignores_leased_future_and_parked_jobs`). `dedupe_key` is unique across the table and names the *work* (`poll:<charge_id>`, `resubmit:<charge_id>`, `sweep:expired`, `scan:live`), so every enqueue is `ON CONFLICT DO NOTHING` and **never** an upsert — `DO UPDATE SET run_at` would let the backstop scan drag a job scheduled an hour out back to now, which is how a poll ladder becomes a hot loop (`enqueue_in_tx_dedupes_on_dedupe_key`). `kind_is_known` closes the vocabulary at the database; `0021` listed four kinds and deliberately withheld the two webhook ones, and Step 5 re-opened it twice: `0022` added `fan_out_events` and `deliver_webhook` alongside their handlers, and `0023` added `scan_deliveries` alongside the backstop scan. **Seven kinds.** The CHECK and `vpay_worker::jobs::JobKind` move together, pinned by `the_kinds_are_exactly_the_check_constraints` and `migration_0022_reopens_the_job_kinds_and_closes_the_delivery_states` |
| Worker job loop (`vpay_worker::run_loop`, `vpay-worker-bin`) | 🟡 | **New 2026-09-03 (Step 4); the row that used to say "there is no job loop".** `run_loop(pool, adapters, rails, policy, concurrency, grace, worker_id, shutdown)` runs `--worker-concurrency` (default 4) claim→run→settle tasks, sleeps 1 s on an empty claim, and is the **same function** the integration suite drives — no `#[cfg(test)]` variant, no injected clock, no second implementation (AGENTS.md rule 1). At boot, **before** seeding, it reaps leases older than `RecoveryPolicy::lease`: a worker that was `SIGKILL`ed leaves rows nothing can claim, and if the dead worker held `sweep:expired` the only other reaper is itself among them (`a_lease_stranded_by_a_crash_is_freed_at_boot_before_any_sweep_runs`). It then reaps again every `lease / 2`, floored at the 1 s idle poll so a short lease cannot become a hot loop (`a_lease_that_expires_while_the_worker_runs_is_reaped_on_its_own_timer`). It seeds `sweep:expired` and `scan:live` in one transaction, because a partial seed is a deployment whose backstop is silently absent. **Observability is one log line a minute**, at INFO, message `job loop gauge`: `worker_id` (hostname/pid/random — the same value in `locked_by`, so an operator can tell which pod holds a stuck job), cumulative `claimed`/`finished`/`rescheduled`/`dead_lettered`/`lost`, and `queue_behind_seconds` = `now() - min(run_at)` over runnable unleased rows (`null` for an empty queue, parked rows excluded). `lost` counts jobs whose lease was reaped while this worker was running them; **any non-zero value is a handler outrunning the lease**, and it is counted separately precisely so it cannot hide inside its neighbours. **🟡, not ✅, for three reasons.** (1) **No metrics endpoint and no dashboard** — the gauge line and the log pipeline are the whole of it, and no alerting rule anywhere consumes `alert = true`. (2) **The loop has never run outside a test or a developer's `just demo`**; there is no CI e2e job for it and no deployment. (3) The contradiction path it logs is untested — see the Settlement row. Unit coverage is 36 tests in `vpay-worker`; the behavioural coverage is 20 `worker_recovery` + 3 `worker_e2e` integration tests, all container-backed and all passing on the tree this note describes |
| Real `SIGKILL` crash test (`backends/tests/integration/tests/worker_kill9.rs`) | ✅ (two of three kill points) | **New 2026-09-04 (Step 8, lane D).** Real `Child::kill()` (`SIGKILL`) to the shipping `vpay-worker-bin` mid-status-query and to the shipping `vpay-server` mid-`requesttopay`, against real Postgres + WireMock, no test double. `a_worker_killed_mid_poll_settles_the_charge_exactly_once_after_its_lease_is_reaped`: the lease and an unanswered `provider_requests` row are the only trace after the kill (the exit status is asserted *signalled with 9*, so a process that chose to `exit(1)` could not stand in for one that was killed); a second worker's boot-time reap recovers it and the charge settles exactly once — one `charges` row, one event, `amount_received` once, and exactly one submit plus two status queries in the rail's own journal. `a_server_killed_mid_submit_leaves_a_charge_the_worker_settles_without_a_second_submit`: the server dies with the POST issued and unanswered; the worker recovers by polling, never resubmitting (one submit in the journal). Guard-failure proof: disabling `vpay_db::jobs::reap_expired_leases` leaves the second worker booted, healthy and permanently unable to claim the stranded job — the test fails deterministically inside its bound. **Two clocks are simulated and nothing else is:** `age_the_dead_workers_lease` moves `jobs.locked_at` ten minutes back (guarded on the dead worker's own `worker_id`) because `RecoveryPolicy`'s five-minute lease has no CLI override, and — **added on the gate branch, after lane G** — `age_the_crashed_charge` moves `charges.created_at` ten minutes back (guarded on `state = 'submitting'`) because lane G's minimum charge age makes a freshly-killed server's charge indistinguishable from a live confirm. That second clock is load-bearing and was measured to be: without it the server-kill case failed on the gate (TRY 3 FAIL, 63 s) with the worker correctly waiting. **Kill point 1** (before any `provider_requests` row exists) is not staged against a real process — there is no network call to interrupt at that moment — and remains proven only by `worker_recovery.rs` writing the state directly. **Neither Orange Money nor a real rail is exercised here**: both cases use `mtn_momo`, and the rail is a WireMock container, so the claim is "the recovery table is executed correctly under a real signal", not "the rails behave as documented" |
| Settlement (`vpay_core::settlement`, `vpay_db::settlement`) | 🟡 | **New 2026-09-03 (Step 4).** The decision is pure (`settle`, `contradiction` — `vpay-core`, no rail, no database) and the write is one transaction (`vpay-db`). **`apply_succeeded`**: charge → `succeeded` with `provider_txn_id` (written with `COALESCE`, so an answer carrying no identifier cannot erase one), intent → `succeeded` with `amount_received = amount`, one `payment_intent.succeeded` event — all or nothing. **`apply_failed`**: charge → `failed` with `failure_code`/`failure_raw`, intent → `requires_payment_method` carrying `last_payment_error`, one `payment_intent.payment_failed` event. Both open with a **compare-and-swap over the live charge states**, so a re-run after a crash matches no row and answers `Ok(None)` — at-least-once job execution does not become at-least-twice event writing (`a_second_apply_succeeded_returns_none_and_writes_no_second_event`). The intent guard is `SETTLEABLE_STATUSES` = `processing`, `requires_action` **and `requires_payment_method`**: a confirm that crashed before it could move the intent leaves a live charge against an intent still reading `requires_payment_method`, and excluding it (as the first implementation did) made the recovered charge dead-letter instead of settling — review finding **F1**, now pinned by `a_settlement_after_a_crashed_confirm_still_moves_the_intent` (`vpay-db`) and `a_settlement_lands_on_the_intent_a_crashed_confirm_left_behind` (integration). `succeeded` and `canceled` stay out of the set, so either one appearing under a live charge is a broken invariant that pages rather than settles. **🟡, and the reason is F4:** a rail answer that *contradicts* an already-settled charge (`failed` against `succeeded`, or the reverse) is classified by `vpay_core::settlement::contradiction` — table-tested over the whole `StatusKind × ChargeState` product, including the negative cases that would make the alert fire on every late poll — and wired to an `error!(alert = true, …)` line in `vpay_worker::handlers`. **The classifier is tested; its two call sites are not.** One is currently unreachable behind the terminal guard and is written out deliberately; the other needs a real multi-worker race that no test stages. So "vpay would tell you if a rail reversed a settled payment" is **not** a claim this repository can make yet |
| Events written by the worker (`events`, migration `0018`) | 🟡 | **New 2026-09-03 (Step 4): the table stopped being schema-only.** Three types, all from [flows/webhooks.md](flows/webhooks.md)'s Stripe list — `payment_intent.succeeded` and `payment_intent.payment_failed` written **inside** the settlement transaction, and (2026-09-04, migration `0029`) `checkout.session.expired` written inside the housekeeping sweep's per-session transaction — with `fanout_state = 'pending'`, one per terminal transition and never two (decision 4 of the Step 4 plan: terminal transitions only). `data` is the `payment_intent` **wire object**, rendered through the same `vpay_api::model::PaymentIntentObject` that `GET /v1/payment_intents/{id}` returns, so the eventual webhook body and the API response cannot disagree about a field; `worker_e2e.rs` asserts the row's type, its `fanout_state` and the contents of its `data`. **Changed 2026-09-03 (Step 5): they are consumed.** The `fan_out_events` drain reads them (`vpay_db::Events::pending_page`, `seq` order) and flips `fanout_state` to `done`; `vpay_db::Events::{list_page, get_by_id}` back `GET /v1/events` (Webhooks row). `worker_e2e.rs`'s `wait_for_fanout` proves the loop that settles a charge is the loop that drains its outbox, which is the seeding a dropped `fanout:events` singleton would silently break. **Still 🟡:** `payment_intent.created`, `.processing` and `.canceled`, and both refund types, are written by nothing — five of the eight documented types |
| `GET /v1/refunds/{id}` — `vpay_api::v1::refunds`, `vpay_db::Refunds` | 🟡 | **New 2026-09-05 (issue #45).** The one route mounted for the Refund resource, and the first read on this surface mounted without its create. The maintainer's decision the issue asked for was that `GET /v1/refunds/{id}` is part of the `/v1` contract **and is served**: a refund is asynchronous and non-terminal (`pending`), the two documented refund event types are written by nothing (see the events row above), and webhook delivery is at-least-once and unordered — so a merchant holding a `re_…` had no authoritative read of any kind, which is the property [flows/provider-port.md](flows/provider-port.md) requires of every other money movement. Merchant-scoped **by a join onto the owning intent**, because `refunds` carries no `merchant_id` and migration `0017` was deliberately not altered to give it one ([reference/vpay-db.md](reference/vpay-db.md) § `refunds`). Renders `vpay_api::model::RefundObject` — **ten** keys [flows/merchant-auth.md](flows/merchant-auth.md) documents, decoded by the shipping `vpay_sdk::Refund` in `the_merchant_sdk_deserialises_the_refund_this_renders`. It was nine when this row was written; issue #46 added `fee` as the tenth on 2026-09-06 and the same renderer serves both surfaces, so the key set moved on the route and on both refund event types at once (`the_refund_object_is_the_documented_ten_keys`). Five container-backed cases in `backends/tests/integration/tests/refunds.rs`, all `PASS` on this machine on 2026-09-05 — **and not re-run since**: issue #46's rebase on 2026-09-06 moved two of them (the key-set assertion is now ten keys with `fee` present and `null`, and the events case loops `charge.refunded` as well as `charge.refund.updated`) on a machine whose Docker daemon was down, so those two assertions are owed to CI and are not claimed as measured here: `a_stored_refund_reads_back_through_the_sdk` (driven by the shipping Rust SDK's new `refunds().retrieve()`), `merchant_b_cannot_read_merchant_as_refund` (the byte-identical `404`, **and** that it is `resource_missing` rather than the `unknown_route` an unmounted route would answer), `the_api_response_and_an_events_payload_for_one_refund_are_byte_identical`, `a_refund_id_without_the_re_prefix_is_never_looked_up` (**added by review the same day** — the `re_` short-circuit was carried by no test at all, and deleting it left the other four cases and all seven `vpay-api` refund unit tests green; the new case seeds a row whose id is not `re_…`, owned by the merchant asking for it, so the short-circuit is the only thing that can refuse it and its removal answers `200`), and `creating_a_refund_is_still_the_honest_404`. Four mutations measured the same day, each reverted: dropping `p.merchant_id = $1` from the join makes the tenancy case answer `200`; deleting the `/refunds/{id}` entry from `V1_ROUTES` fails three of the four cases and the unit test `the_refund_resource_is_mounted_for_a_read_and_for_nothing_else` **while all 136 `vpay-sdk` tests still pass** — the SDK tests prove the client, the route test proves the server; rendering the response through a second hand-built map fails the byte-identity case on `created`; and deleting the `re_` short-circuit fails `a_refund_id_without_the_re_prefix_is_never_looked_up` with a `200`, and nothing else. **🟡, and it will stay 🟡 until a refund can exist:** nothing in this repository writes a `refunds` row. `POST /v1/refunds` is unrouted, `vpay_db::Refunds` exposes one read and **no write**, and `mtn_momo::refund` is still `NotImplemented` while Orange Money answers `Unsupported`. Every row the four cases read is `INSERT`ed by the suite itself. So what is proven is the read, the tenancy and the rendering — and **nothing at all** about how a refund comes to exist. **Migration `0017`'s header comment and its `COMMENT ON TABLE refunds` now say something false, and were deliberately left alone** (recorded by review, 2026-09-05): both read "NOT WRITTEN OR READ BY ANY CODE IN THIS REPOSITORY — no refunds repository, no `/v1/refunds` route", and the first two thirds of that stopped being true with this change. `sqlx::migrate!` checksums each migration's whole file content, comments included, so editing an applied one turns the next boot into a version mismatch — the same reason `0006` and `0013` still cite a deleted test file, and the same place the correction goes. The `COMMENT` is live in every database this has been applied to and an operator reading it will be told the table is unread; **whether to spend a migration number on a `COMMENT ON TABLE` fix is left to the maintainer**, because this change deliberately added no migration and two other branches were numbering against `0018+` the same day. What is still true in that comment: **nothing writes the table**, and `POST /v1/refunds` is still routed nowhere |
| Reconciler | 🟡 | **New 2026-09-03 (Step 4); was ⛔.** The rail-driven half of the state machine is `vpay_core::settlement::settle(StatusKind, ChargeState)` — a `const fn`, total and wildcard-free in both dimensions, so a new state or answer is a compile error rather than a silent default. Its 24-pair table is transcribed as a test *and* a second test proves the transcription covers every pair exactly once, so a deleted row cannot quietly shrink what the first one checks. Deliberately **not** a `Transition` variant: `vpay_core::state::next_status` still answers `None` for every rail-driven edge, which is what keeps `processing → succeeded` unreachable from an HTTP handler. The escalation is real: past `RecoveryPolicy::unresolved_after` (24 h from `charges.created_at`) the charge moves to `unresolved` and the job fails with `JobError::Exhausted` → retry in 1 h with `alert: true`, **never** a dead letter (`a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked`). **Escalating does not depend on the rail answering, and does not stop the polling** — the two halves of review finding F3. A status query that keeps failing used to keep a charge off the horizon entirely, riding the ladder quietly forever (`a_rail_that_never_answers_is_still_escalated_at_the_horizon`); past the horizon the charge is now `unresolved` whatever the rail says or fails to say, **and the worker keeps asking once an hour**, so a terminal answer at hour 30 settles the payment through the ordinary path — which is what [flows/reconciler.md](flows/reconciler.md) means by "a late success is the normal transition". **🟡, for three named reasons.** (1) ~~**No callback route.** `POST /provider/{code}/callback` does not exist, so nothing enqueues a poll from a callback and Orange's `notif_token` is compared against nothing; `parse_callback` is exercised by tests and by nothing else.~~ **Corrected 2026-09-04 (Step 8, lane C): the route exists** — `provider_callback::routes()` is mounted in `vpay-api/src/lib.rs:959`. It enqueues the charge's `poll:<charge id>` job if it is missing and pulls it forward to `now()`, and it writes no charge or intent state; `parse_callback` is now exercised by the route and by `backends/tests/integration/tests/provider_callback.rs`. **The half of this that stays true: Orange's `notif_token` is still compared against nothing**, and no rail has ever called the route — every body it has parsed was transcribed from `docs/flows/adapter-*.md`. (2) **`prompt_ttl_seconds` / `prompt_expired_at` are not implemented** — no column, no config key, no `payment_intent.processing` event with `expired: true`, so a merchant's "check your phone" UI has nothing to turn off. Deferred deliberately (decision 6 of `docs/plans/2026-09-03-step4-worker.md`) and named in [flows/reconciler.md](flows/reconciler.md)'s Status. (3) **The `contradiction` classifier is wired and its call sites are untested** — see the Settlement row |
| Stripe-SDK compatibility on `/v1` ([docs/flows/stripe-sdk-compat.md](flows/stripe-sdk-compat.md)) | 🟡 | **New 2026-09-03 (Step 5b).** A merchant can drive vpay with the **official `stripe` package**: `new Stripe("", { authenticator, host, port, protocol })`, where the authenticator is `createStripeAuthenticator` from `@vaam-apps/vpay-sdk/stripe` and performs the `private_key_jwt` handshake. Evidence is `sdks/stripe-compat` — the real `stripe@22.6.1` driven **out of process over TCP** against a real `vpay-server` + `postgres:16-alpine` + both WireMock rails + `vpay-worker` + the WireMock webhook receiver: **25 cases, 0 skipped**, measured passing on the authoring machine on 2026-09-03 after this branch was rebased onto Steps 4 and 5 (`just demo_port=18080 stripe-compat`, 25 passed in 9.9 s — the settlement case ~2.0 s and the webhook case ~6.1 s of it, both bounded waits on a real worker rather than sleeps), and run by CI's `e2e (compose)` job. The suite **cannot skip**: its `globalSetup` fails the run when `/healthz` does not answer or the merchant handshake does not complete (both failure modes exercised deliberately before this row was written). **A request field is refused only when ignoring it would change where or when money moves**, and that line is now drawn in code and pinned end to end: `confirm: true` on create, `capture_method` with any value but `automatic`, `application_fee_amount`, `transfer_data` and `on_behalf_of` are each a `400` naming the field in `error.param`, on **both** POST bodies; everything else Stripe sends that vpay does not implement (`setup_future_usage`, `confirmation_method`, `receipt_email`, `statement_descriptor`, `customer`, `expand`) is accepted and ignored, because none of it changes the payment that results — `metadata` is accepted on both bodies too but is not one of them: `metadata` is stored; the rest are dropped, and the compat suite reads it back off the created intent. `the_fields_that_move_money_elsewhere_are_refused_through_the_real_decoder` drives both columns through the real form decoder rather than constructing the struct; `sdks/stripe-compat` proves the same four through stripe-node's own encoder, including that `capture_method: "automatic"` is accepted and that a refused confirm leaves the intent at `requires_payment_method`. **On the client side the authenticator is bound to `baseUrl`'s origin**: `@vaam-apps/vpay-sdk/stripe` refuses — `VpayConfigError`, before a token is minted — to write `Authorization` on a request addressed anywhere else, because stripe-node with `host`/`port`/`protocol` omitted addresses `api.stripe.com:443` and an unconditional authenticator would post a merchant's live vpay bearer token to Stripe. **Two of this row's original gaps closed on the rebase onto Steps 4 and 5, and both were assertions that had to be inverted rather than deleted.** (1) `lifecycle.compat.test.ts` used to assert a confirmed intent *stayed* `processing` — correct before the worker existed — and now polls `paymentIntents.retrieve` to **`succeeded`** within a bounded window, failing rather than hanging if it does not arrive; `examples/merchant-stripe-node` was inverted the same way. (2) `webhooks.compat.test.ts` is new and closes the "byte-identical by construction, unobserved in practice" caveat: it makes a payment, waits for the settlement, reads the delivery out of the WireMock receiver's **own request journal** (`GET /__admin/requests` — what a receiver got, not what vpay believes it sent) and puts the recorded bytes and `Stripe-Signature` through `stripe.webhooks.constructEvent`, then requires `StripeSignatureVerificationError` for a body with one byte flipped and for the right body with the wrong secret. Verified decisive by rerunning it with a deliberately wrong secret in the *positive* assertion, which failed the case. 🟡 not ✅ for what remains: the rail and the receiver are both WireMock hosts (MTN has never been called, no merchant endpoint has ever been POSTed to, no money has moved), the `stripe-should-retry: true` direction is unobserved (next row), `stripe.events.list()` against the now-routed `/v1/events` is untested, and nothing pins vpay against a future `stripe` release. This row also retracts a claim this file made at line ~371 and [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md) made in bold: "no Stripe SDK can authenticate against vpay". It was reasoned from Stripe's SDKs sending a static bearer, and missed `config.authenticator`. ADR-0010 carries a dated amendment; the decision itself (no API keys) is unchanged. |
| `request-id` response header (stripe-node's spelling) | ✅ | **New 2026-09-03 (Step 5b).** Every response carries the request id under **both** `x-request-id` and `request-id`, with **one value** — `mirror_request_id_header` reads the id `SetRequestIdLayer` settled on and copies it out, rather than a second `SetRequestIdLayer` that would mint a second UUID. stripe-node reads `headers['request-id']` and nothing else when it populates `err.requestId` and `obj.lastResponse.requestId`, so without this the "contact support with the request id" sentence `Category::Internal` prints was unkeepable for Stripe SDK users. Response-only: a caller sending `request-id` does not get to choose the id (`a_caller_supplied_stripe_request_id_header_is_not_honoured`). Proven by `a_response_carries_the_request_id_under_both_names` in `vpay-api` and, end to end through the real SDK, by `sdks/stripe-compat`'s headers cases. |
| `stripe-should-retry` response header | 🟡 | **New 2026-09-03 (Step 5b).** Emitted on every response `ApiError::into_response` renders, **derived from `Classify::retry`** and never chosen per handler (ADR-0011): `Retry::AfterBackoff` → `true`, `Retry::Never`/`Retry::NewAttempt` → `false`. It exists because stripe-node consults it above its own status rules and vpay needs both directions — it retries every `409` unconditionally (a lifecycle refusal that waiting cannot fix) and no `4xx` at all (so it would never retry an in-flight idempotency key, the one refusal here that clears itself). `the_retry_advisory_follows_the_classification_not_the_status` pins eight category/status/advice rows written from `docs/flows/errors.md`'s policy table, and `the_404_fallback_carries_the_retry_advisory` proves it survives the real router. 🟡 for two gaps, both real: **(1)** the `false` direction is observed through the SDK (`sdks/stripe-compat` reads it off a 409's `err.headers` and pins that the call returns in well under stripe-node's 1.0 s minimum two-retry backoff); the **`true` direction is not observed at all** — staging `IdempotencyKeyInFlight` from outside needs two concurrent requests where one holds the key, and the only slow operation is a confirm whose rail-side delay is keyed by a server-minted reference. A deterministic stage would need a test double in a shipping path ([ADR-0006](adr/0006-no-mocks-in-main-processes.md)). **(2)** `405` and `413` carry no such header — next row. **~~(3)~~ Closed 2026-09-03 on the Steps 4+5 rebase: a replayed response now carries the advisory the original carried.** It used to carry none — `idempotency_keys` stored a status and a body and nothing else, so `replay` rebuilt a response without headers and stripe-node fell back to its own rule, retrying a replayed `409` it would have left alone. Migration `0025` adds `idempotency_keys.response_retry TEXT NULL CHECK (response_retry IN ('true','false'))`; `PostRequest::finish` reads the value **off the rendered response's own `HeaderMap`** and `vpay_db::Idempotency::store` persists it; `replay` writes those bytes back. Re-deriving from the stored status was the fix deliberately *not* taken (ADR-0011: one classification is the source of status **and** retry, and a second derivation running the other way is the drift it exists to prevent) — and it is why the column holds the header's **text** rather than a `BOOLEAN`: a boolean would put a second `bool → "true"/"false"` rendering in the replay path. `NULL` is a distinct answer from `'false'` and means "the stored response carried none", which is what a stored `2xx` looks like; `NOT NULL DEFAULT 'false'` would have invented an advisory for every successful create. The two tests that pinned the gap were **inverted, not deleted**: `a_replayed_response_carries_the_advisory_it_was_stored_with` asserts the replayed value *equals* what the same error renders fresh (so a hard-coded `false` in `replay` fails it), `a_replayed_error_carries_the_same_retry_advisory_the_original_did` does the same against a real Postgres through the real "one charge per intent" refusal, and `the_retry_advisory_round_trips_and_0025_refuses_anything_else` proves the CHECK fires on a third value. |
| `405`/`413`: the two `/v1` answers that are not `ApiError` | 🟡 | **Documented 2026-09-03 (Step 5b); not fixed.** A route asked with the wrong method answers axum's bare `405` with an empty body, and a body over `V1_BODY_LIMIT_BYTES` (64 KiB) answers tower-http's `413` with `text/plain`. Both are produced above `ApiError::into_response`, so neither carries the Stripe error envelope **or** `stripe-should-retry`. The cost is larger than the missing advisory and is **measured, not inferred** (`sdks/stripe-compat/src/errors.compat.test.ts`): stripe-node meets a non-JSON body by discarding the whole response and throwing `StripeAPIError: "Invalid JSON received from the Stripe API"` with `statusCode` **and** `headers` both `undefined` — so a merchant cannot tell a 405 from a 413 from a proxy's HTML 502, and the only thing that survives is `err.requestId`. Both request-id headers *are* present on the wire; stripe-node just drops everything else. The fix is a `method_not_allowed` renderer for the whole surface plus an envelope for the body limit. Neither is built. `docs/api/README.md` has flagged the bare `405` since Step 1. |
| `Category::Rail` `502` is auto-retried by stripe-node | 🟡 | **Reasoned, not observed — 2026-09-03 (Step 5b).** `Category::Rail` renders `502` with `Retry::AfterBackoff`, so vpay sends `stripe-should-retry: true` and stripe-node (which retries every `5xx` anyway) re-POSTs. For a **confirm the rail did not answer**, that replay meets "one charge per intent, forever" and comes back as a `409` — the merchant's error is then a conflict rather than the rail failure that actually happened. **No test covers this**, and the reason is structural: the compose stack's WireMock rails cannot be steered into a transport failure from outside, because the `provider_reference_id` the adapter sends is minted server-side with `Uuid::new_v4()` and the delay/failure mappings are keyed by it. Stated here because it is the predictable consequence of a header this step added, not because anything measured it. Resolving it properly is a question about what a retried confirm should do, which is a maintainer decision, not an implementation detail. |
| Browser checkout surface (`/v1/browser`, `@vaam-apps/vpay-stripe-js`) — `vpay_api::browser` | ✅ | **New 2026-09-03 (Step 5c).** Two routes — `GET /v1/browser/payment_intents/{id}` and `POST /v1/browser/payment_intents/{id}/confirm` — unauthenticated by any bearer token, deliberately outside `V1_ROUTES`/the 401 boundary (own table `BROWSER_ROUTES`, own `.fallback`), authenticated instead by a publishable key + the intent's own `client_secret`. `confirm` reaches the same `confirm_once` `/v1` calls; there is no `create`, `list`, or `cancel` on this surface. Full design and every decision (D1–D5): [docs/flows/browser-checkout.md](flows/browser-checkout.md). Evidence is `backends/tests/integration/tests/browser_checkout.rs` (real Postgres, real WireMock MTN rail, the shipping router and adapters, the shipping merchant SDK to create the intent, raw `reqwest` for the browser half a merchant SDK cannot express): a confirm with a valid credential reaches the rail and the intent moves to `processing` (`a_browser_confirm_reaches_the_rail_and_moves_the_intent_to_processing`); every one of four distinct credential failures — wrong secret, unknown key, another merchant's key, unknown intent — answers the byte-identical 404 (`every_credential_failure_is_the_identical_404`); no route answers 401 and `BROWSER_ROUTES`'s contents are pinned (`every_browser_route_is_reachable_without_a_merchant_token`) — **five entries since Step 9, not two**, and the property that pin defends is no longer the count but that *exactly one of them answers a non-`GET` method*, the confirm that has been there since Step 5c; no `create`/`list`/`cancel` exists (`the_browser_surface_has_no_create_no_list_and_no_cancel`); a confirm needs no `Idempotency-Key` and a double-tap is a `200` then a `409`, never a second charge (`a_browser_confirm_needs_no_idempotency_key_and_a_second_one_is_the_409`). **Corrected 2026-09-03 rebasing this branch onto Step 6:** the nest's own copy of `track_http_metrics` (added while resolving that rebase, since Step 6 landed the metrics middleware after this surface's own commits were written) is proven by `a_browser_get_is_counted_under_its_own_route_pattern`, a real Postgres scrape asserting `vpay_http_requests_total{route="/v1/browser/payment_intents/{id}",method="GET",status="200"} 1`. Constant-time secret comparison is pinned two ways — directly (`a_constant_time_compare_examines_every_byte_even_when_the_first_differs`) and at the `secrets_match` wiring level, which a clippy dead-code failure backstops (`secrets_match_rejects_every_shape_a_boolean_test_can_express`) — and both `PaymentIntentRow` and `PaymentIntentWithSecret` carry hand-written, redacting `Debug` impls proven never to contain the secret (`a_stored_rows_debug_output_never_carries_the_client_secret`, `a_payment_intent_with_secrets_debug_output_never_contains_the_client_secret`). **What is not built:** rate limiting is not in this process by design (D5) — an ingress requirement this repository does not enforce or check; ~~the Orange redirect *return trip* has no route (`/provider/{code}/callback` does not exist — see the Reconciler row above), so this surface ships push-only (D4)~~ — **retired 2026-09-04 (Step 9).** Lane 2 made the rail's return URL a per-charge value, so Orange now sends the payer to the merchant's own `return_url` on a direct confirm; the parenthesis was already stale — `POST /provider/{code}/callback` has existed since Step 8 lane C and was never the return trip. Lane 3 then built the *page*, `frontends/apps/checkout`, and lane 6 drove a payer through Orange's stub hosted page and back to a merchant's site in a real browser (`shop-hosted.cy.ts`). What a merchant integrating `@vaam-apps/vpay-stripe-js` **directly** against a redirect rail still has to do is land the payer on their own `return_url` and poll from there; vpay's page is what removes that work, and it is reached through a Checkout Session rather than through this package. |
| Publishable keys (`merchant_clients[].publishable_keys`) — `vpay_config::oauth`/`config` | ✅ | **New 2026-09-03 (Step 5c).** Explicit YAML list per merchant (D1), never derived from `merchant_id`; empty is the fail-closed default. `Config::validate_all` refuses a duplicate across all merchants, a key not matching `^pk_(test\|live)_[A-Za-z0-9]{16,64}$`, and a `pk_live_` key under `deployment.livemode: false` (or the reverse) — `ConfigError::{DuplicatePublishableKey, MalformedPublishableKey, PublishableKeyLivemodeMismatch}`, each proven against a real fixture file (`backends/tests/fixtures/publishable-key-{duplicate,malformed,livemode-mismatch}.yml`). `config/application.yml`'s `acme-cameroon` carries `pk_test_acmecameroonsandbox01`; `just gen-demo-keys` writes the fixed `pk_test_demomerchantsandbox01` for `demo-merchant` into the generated demo overlay. |
| `client_secret` rendering (`PaymentIntentWithSecret`) — `vpay_api::model` | ✅ | **New 2026-09-03 (Step 5c).** A wrapper (`#[serde(flatten)]` over the untouched 12-key `PaymentIntentObject` plus `client_secret`) used only by `/v1`'s `create`/`retrieve` and the two `/v1/browser` routes (D2) — `list` and every webhook body (`events.data`) keep rendering the plain object, unchanged. `PaymentIntentObject`'s own tripwire test, `every_documented_key_is_present_including_the_null_ones`, was not touched by this step and still asserts exactly 12 keys. Proven live end to end (not just at the type level) by `the_client_secret_is_on_create_and_retrieve_and_never_on_the_list` and `no_event_body_carries_a_client_secret` in `browser_checkout.rs`. **Both merchant SDKs now declare this field** (`sdks/nodejs/src/types.ts`'s `client_secret?: string`, `sdks/rust/src/model.rs`'s `client_secret: Option<String>` with `#[serde(default)]` and a hand-written redacting `Debug`), fixed the same day (`c40a137`) — see the Merchant SDKs section. |
| CORS on `/v1/browser` only — `tower_http::cors::CorsLayer` | ✅ | **New 2026-09-03 (Step 5c).** `allow_origin(Any)`, `GET`/`POST`/`OPTIONS`, `allow_headers([CONTENT_TYPE])`, `max_age(600s)`, `allow_credentials` off. Mounted on the `/v1/browser` nest and **no other** — the merchant `/v1` nest carries no `CorsLayer` at all, so a stolen bearer token cannot be used cross-origin even by mistake. Proven by `the_browser_nest_answers_a_preflight_and_the_merchant_nest_does_not` (integration) and `cors_is_mounted_on_the_browser_nest_and_on_no_other` (router unit test). |

### Unimplemented items tracked by `verify-status`

Every token below appears verbatim in shipping source. Removing an item here
without removing it from the code fails CI, and — since 2026-09-03 — so does
leaving an item here that no shipping code carries any more; both halves of
that are read off the gate's own output by
`verify_status_reports_both_directions_from_the_gate_itself`. The scanner
counts only occurrences in code, so a token quoted in a comment of any kind,
in a `#[doc = "…"]` attribute or in a string literal neither declares
anything nor counts as one — you never have to strip an honest sentence from
a doc comment to keep this gate green
(`a_token_quoted_in_a_comment_is_not_a_shipping_claim`,
`a_token_outside_code_is_never_a_shipping_claim`,
`a_token_in_a_doc_attribute_is_not_a_shipping_claim` and
`the_lexer_tells_the_four_states_apart` in `xtask`).

**There is exactly one, down from eight on 2026-09-03 (Step 3), and the two
that left did so for opposite reasons — which is the distinction this list
exists to keep visible.** Six went because the code was *written*:
`{mtn_momo,orange_money}::{submit, query_status, parse_callback}` are real
HTTP calls now. `orange_money::refund` went because it was **never unbuilt
work in the first place** — Orange's Web Payment product documents no refund
API, so the adapter stops overriding the port and inherits the trait's
default `ProviderError::Unsupported`: a permanent capability answer the core
can branch on (`supports_refunds: false`), asserted by the conformance case
`a_rail_without_the_refund_capability_answers_unsupported`. A rail that will
never support an operation must not be described with the same token as work
someone still has to do.

- `mtn_momo::refund` — MTN refunds are a different product (Disbursements)
  with its own subscription key and its own token scope; nothing in
  `config/application.yml` or `ProviderHost` carries a disbursement key and
  no deployment has been issued one, so nothing honest can be built yet
  (Step 3 design, decision 3). `supports_refunds` stays **`true`** for
  `mtn_momo` on purpose: the *rail* refunds, and answering `Unsupported`
  would be a lie about MTN rather than an admission about us. `refund` is
  therefore the one operation on the one rail that still returns a token —
  reachable only through `POST /v1/refunds`, which is not routed, so no
  caller can currently provoke it
  (`refund_is_not_implemented_and_does_not_pretend`,
  `unimplemented_operations_never_fabricate_success` in the conformance
  suite).

**Declared and unpopulated, beside that token: the refund `fee`.** Added
2026-09-05 for [issue #46](https://github.com/vaam-apps/vpay/issues/46), which
an integrator filed because vpay's `refund` object never said what the
movement cost and their own type had no way to spell "unknown", so they were
shipping a hardcoded `0`. It is **not** a `NotImplemented` token and does not
belong in the list above — nothing about it is unbuilt. What exists, and is
asserted:

- the column `refunds.fee` (migration `0031`), nullable, no `DEFAULT`, with a
  `fee_non_negative` CHECK — `an_unreported_refund_fee_stays_null_and_never_becomes_zero`
  and `a_negative_refund_fee_is_rejected_by_the_database` in `postgres_smoke`;
- the wire field `vpay_api::model::RefundObject::fee` and the ten-key
  tripwire on that object — `the_refund_object_is_the_documented_ten_keys`,
  `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero`,
  `a_refund_delivered_as_either_refund_event_carries_fee_present_and_null`
  (which renders through **both** refund event types, since `data.object` is
  this object), and `a_reported_fee_never_moves_the_payers_amount`, which
  holds the one invariant the field exists to protect: `amount` is the
  payer's money and is never net of the fee. That last case was added on
  review — until it existed, `amount: row.amount - row.fee.unwrap_or(0)`
  passed all 244 of `vpay-api`'s tests
  ([plans/issue-46-notes/review.md](plans/issue-46-notes/review.md), F1). That
  244 is a dated measurement of the pre-rebase tree and is deliberately not
  restated: what matters is that the same mutation was **re-run on 2026-09-06
  after the rebase onto issue #45's merge and still fails that case**, on
  `left: 1750, right: 2000`
  ([plans/issue-46-notes/impl.md](plans/issue-46-notes/impl.md) § 11);
- the port's own `vpay_provider::Refunded::fee` (`Option<Money>`), the only
  thing that could ever fill it;
- both merchant SDKs' `Refund.fee`, with the parity row and its five tests.

None of those four bullets *begins* with a backticked path, and none may:
`verify-status` reads exactly that shape — `- ` then a backtick — as a
declared `NotImplemented` token, and the docs→code half of the gate would
then fail because no shipping code carries one. That is the gate working
rather than a trap, and it is why each bullet above opens with a noun.

**What has to exist before it is ever anything but `null`.** For MTN: a
Disbursements subscription key and token scope in `config/application.yml` /
`ProviderHost` (the same missing credential as the token above), a written
`mtn_momo::refund`, **and** a real Disbursements response that actually
carries a fee — none of the modelled MTN responses in
`vpay-adapter-mtn-momo/src/wire.rs` has a fee field today, and whether that
product reports one has never been verified against MTN's sandbox, which this
repository has never called. For Orange: nothing, ever — the Web Payment
product documents no refund API, the adapter answers `Unsupported`, and there
is no refund to charge a fee for. **An adapter must not invent one**: `None`
is "the rail did not report a fee" and `Some(0)` is "the rail said it was
free", and collapsing them is the exact defect the issue reports one layer up.

**Also missing, and larger:** nothing **writes** a `refunds` row. Reading one
is no longer missing — issue #45 landed `vpay_db::Refunds::get_for_merchant`
and `GET /v1/refunds/{id}` on 2026-09-06, while this branch was open, so
`RefundObject` does cross a wire and `fee` is `null` on every object it can
produce. What is still missing is the whole of the write side: no
`POST /v1/refunds` (it is declared in the wire contract and mounted nowhere),
no `create` in the repository, no adapter that can execute a refund, and no
writer for `charge.refunded` / `charge.refund.updated` — both types are in the
`type_is_a_documented_event` vocabulary and neither has ever been emitted. So
what the tests above prove about the *event* surface is that the contract
holds, not that a refund event works.

### Adapters

Both rails' wire calls are implemented and **proven against a real
`wiremock/wiremock` container**, never against MTN or Orange. The column
split below is the whole point of this section: ✅ means a conformance case
would fail if the behaviour broke; 🟡 means the code is real and its only
witness is a stub; ⛔ means not built.

| Rail | Capabilities | Wire calls (vs. WireMock) | Real sandbox | Callbacks | Refunds |
|---|---|---|---|---|---|
| `mtn_momo` (push) | ✅ declared and tested | ✅ `submit` / `query_status` / `parse_callback` / `account_holder_name` | ⛔ never called | 🟡 parsed, routed, never received | ⛔ `NotImplemented("mtn_momo::refund")` |
| `orange_money` (redirect) | ✅ declared and tested | ✅ `submit` / `query_status` / `parse_callback` | ⛔ never called | 🟡 parsed, routed, never received; `notif_token` unverified | ✅ `Unsupported` — permanent, capability-driven |

**Account-holder lookup, added 2026-09-05 (issue #47).** A sixth column would
have made the table unreadable, so it is here instead:
`supports_account_holder_lookup` is **`true` for `mtn_momo`** and the adapter
implements it against `GET
/collection/v1_0/accountholder/msisdn/{msisdn}/basicuserinfo`, under the
Collections subscription key and token scope `submit` already uses — which is
why this was buildable where `refund` is not. It is **`false` for
`orange_money`**, which therefore inherits the port's
`ProviderError::Unsupported` and carries **no token**: Orange's equivalent
route is unconfirmed from this repository (item 8 on
[flows/adapter-orange-money.md](flows/adapter-orange-money.md)'s "to confirm"
list), and a `NotImplemented` token would claim work someone owes rather than
an absence. Five conformance cases run over both rails
(`an_account_holder_lookup_returns_a_name_and_nothing_else`,
`a_number_the_rail_has_no_record_of_is_not_an_error`,
`a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder`,
`an_oversized_account_holder_body_is_refused_at_the_cap`,
`an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing`),
so the ⛔ in the "Real sandbox" column applies to this call exactly as it does
to the other three. **Two specifics about MTN's endpoint are unverified and
are assumptions, not transcriptions:** the case of the `accountHolderIdType`
path segment (MTN's portal declares the enum upper-case; every published
example, and vpay, spell it lower-case), and the `404` → `Ok(None)` mapping
(MTN documents `200`, `401` and `500` for this operation and no `404` at all).
Both are named in [flows/adapter-mtn-momo.md](flows/adapter-mtn-momo.md).

**What the ✅ in the wire-call column rests on, named.** The shared
conformance suite ran **26 tests, 26 passed, 0 skipped** on 2026-09-03 (measured for
this note, as part of the 115-test container run in the header): 4
capability cases plus 11 port cases parameterised over both rails, each
against a container started by `vpay_testkit::containers::start_wiremock`
and reached over HTTP exactly as a rail is (ADR-0006 — a stub rail is a
WireMock *host*, never a linked implementation). The 11:
`submit_returns_a_reference_and_a_flow_shaped_result` (push returns no
`redirect_url`; redirect returns a URL **and** `pay_token` in one value),
`duplicate_submit_reports_submitted_not_an_error`,
`not_found_is_never_on_its_own_a_failure`,
`a_declined_charge_maps_to_the_documented_failure_code`,
`an_unavailable_rail_is_a_transport_error_never_a_decline`,
`bad_credentials_are_not_reported_as_a_payer_problem`,
`a_callback_body_round_trips_to_identifiers_only`,
`a_rail_without_the_refund_capability_answers_unsupported`,
`pending_then_successful_walks_the_scenario`,
`redirects_are_refused_and_never_followed` and
`an_oversized_rail_body_is_refused_at_the_cap`. **Updated 2026-09-04 (Step 8,
lane C): 26 became 28 and 11 port cases became 12** — the twelfth is
`the_submit_tells_the_rail_where_to_call_back`, which asserts MTN's
`X-Callback-Url` header and Orange's `notif_url` body field are present on
every accepted submit, with the shared WireMock mappings refusing a submit
that omits them (so removing either send makes both that case and
`submit_returns_a_reference_and_a_flow_shaped_result` fail with the rail's own
404 — measured by lane C, restored). `cargo nextest list -p
vpay-tests-conformance` lists **28** on the merged gate branch. Under them sit 48 unit tests
in `vpay-adapter-mtn-momo` and 53 in `vpay-adapter-orange-money` (0 ignored,
measured the same day) covering the request bodies, the failure tables, the
token caches and every redaction. **The three `#[ignore]`d conformance cases
this section used to name are gone**, and `just verify-ignored` now pins
`expected_ignored := "0"`.

**What the ⛔ and 🟡 columns mean, plainly.**

- **Real sandbox — ⛔.** Neither adapter has ever exchanged a byte with MTN
  or Orange. Every assertion above would still pass if
  `docs/flows/adapter-mtn-momo.md` and `docs/flows/adapter-orange-money.md`
  were wrong about the rails, because the mappings were written from those
  documents. Both docs' "to confirm with the rail" lists stand in full.
- **The 401 → re-mint → retry path is unproven on both rails.** The logic
  exists and is bounded at one retry, but no mapping returns 401 from
  `requesttopay` / `webpayment` *after* a good token. What is proven is the
  401 on the token endpoint itself
  (`bad_credentials_are_not_reported_as_a_payer_problem`).
- **Callbacks — 🟡.** `parse_callback` is implemented and tested on both
  rails and returns identifiers only, never a status. ~~**There is no callback
  route**: nothing in a running vpay ever calls it~~ — **corrected 2026-09-04
  (Step 8, lane C): the route exists.** `POST /provider/{code}/callback`
  consumes `parse_callback`'s output in production, to name a charge and pull
  its poll job forward, and nothing else; it writes no charge or intent state.
  **What has not changed is why this is still 🟡:** no rail has ever
  called it — every body it has parsed was transcribed from
  `docs/flows/adapter-*.md` by this repository's own tests — nothing compares
  Orange's `notif_token` against a stored one (the route discards
  `CallbackRef::ref_extra` rather than trusting it), and MTN's callbacks are
  unsigned and unauthenticated in any case, so a callback is a hint on both
  rails and always will be. Orange's parser fails closed without a
  `notif_token` (`a_callback_without_a_notif_token_is_refused`), and that check
  is now load-bearing on a live path rather than only in tests.
- **Orange's duplicate-submit semantics is an assumption.** The stub returns
  the same `pay_token` for a repeated `order_id`, and the port requires a
  duplicate to be `Submitted` rather than an error, but that is a property
  of the mapping, not an observation of Orange.
- **MTN's `externalId` carries the provider *reference*, not the charge id**
  — `ChargeRef` gives an adapter no charge id — and `payerMessage` is not
  sent.
- **Proxies are deliberately refused.** `vpay_provider::http` ignores
  `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`; a deployment behind a corporate
  egress proxy is not served by this client. The merchant SDK's twin
  (`sdks/rust/src/client.rs`) keeps proxy support on purpose, because a
  merchant's process runs on a merchant's network.
- **`vpay-worker-bin` builds an HTTP client and an adapter map, and calls no
  rail with either.** They are used only for the boot-time YAML↔adapter join
  (`boot_seeds`, which exits `78` for a configured rail with no linked
  adapter); there is no job loop to call `submit` or `query_status`.

Capabilities being real matters more than it sounds: `orange_money` declares
`supports_refunds: false`, and that flag — not a rail-specific branch — is what
makes the core refuse a refund on that rail.

---

## Frontend

| Area | Status | Notes |
|---|---|---|
| pnpm workspace, TS strict, `pnpm -r lint` | ✅ | `pnpm -r typecheck` clean. **`pnpm -r lint` became real on 2026-09-05 and was broken until then**: five of fifteen packages declared a `lint` script, four of them for an ESLint that was installed nowhere in the workspace, so the sweep exited 1 on the first one (`@vpay/tokens`, `eslint: not found`) before any rule ran — and `just lint-web` never invoked it, so the `web` gate claimed a lint it did not perform. Now **all 15 packages** run `eslint . --max-warnings 0` over one shared flat config exported from `@vpay/config/eslint` (the `./eslint` export that package had declared since it was created, at a file that did not exist): ESLint `9.39.5`, `typescript-eslint` `8.69.0` **recommended-type-checked** against each package's own tsconfig, `eslint-plugin-react-hooks` `7.1.1` on the four React packages, `@next/eslint-plugin-next` `16.3.4` on the three Next apps, `no-console` as an error in shipping source (tests, stories, Cypress specs and command-line examples exempt), and `no-restricted-imports` refusing `testing/**` from shipping code in `frontends/apps/checkout` and `examples/shop` — a second lock beside the hand-written vitest guards, which are unchanged. **214 files linted**, measured by `eslint . --format json`. `just lint-web` is now `build-sdk-node` → `pnpm -r typecheck` → `pnpm -r lint`, and CI's `web` job runs that recipe rather than a copy of its commands. **Node baseline `22.11.0` → `22.23.2` (the current 22 LTS) on 2026-09-05, and that was forced, not chosen for tidiness.** The first CI run of this branch failed all three of `web`, `rust` and `e2e` at `pnpm install --frozen-lockfile` with `ERR_PNPM_UNSUPPORTED_ENGINE: … "eslint-visitor-keys@5.0.1". Expected ^20.19.0 || ^22.13.0 || >=24. Got: v22.11.0` — a transitive dependency of ESLint **9**.39.5, under `.npmrc`'s `engine-strict=true`, with all three jobs installing Node from `node-version-file: .nvmrc`. So the refusal recorded here before — *ESLint 9 rather than 10, because 10 needs Node `^22.13.0` and `.nvmrc` is `22.11.0`* — **was wrong on its own terms and is retired**: ESLint 9.39's own `engines` are permissive, but its dependency tree already required `^22.13.0`, so pinning 9 moved the baseline anyway. The baseline was raised deliberately rather than pinned around; `engine-strict=true` is kept, the root `package.json` `engines.node` floor is raised `>=22.11.0` → `>=22.13.0` so a stale Node fails naming this repository rather than a transitive package, and **ESLint 10 (`^20.19.0 || ^22.13.0 || >=24`) is now equally admissible** — still not taken here, but as an unmade upgrade, not a refusal. `frontends/Dockerfile` and `examples/shop/Dockerfile` pin no Node minor (`node:22-alpine`, which resolves to `v22.23.2` today) and needed no change. **Proven in both directions on the exact CI Node**: under `v22.23.2` `pnpm install --frozen-lockfile` exits **0**; under `v22.11.0` it exits **1** with `ERR_PNPM_UNSUPPORTED_ENGINE`. **The review miss this records:** both the implementing pass and its review ran a Node newer than CI's, so every local gate was green over a break that only the runner could see — an environment-parity gap, not a code one. Proven in both directions on the authoring machine: a `console.log` in `frontends/apps/checkout/src/lib/money.ts` and in `sdks/nodejs/src/client.ts`, and an `@/testing/memory-store` import in `examples/shop/src/server/context.ts`, each make `pnpm -r lint` exit 1 naming the file and rule; removed, it exits 0 across 15 of 15. **51 findings were raised on the tree; 30 were fixed in source and 21 suppressed at line scope with a reason each** (12 `require-await` where an interface demands `async`, 7 React-Compiler advisories that need a component redesign and are marked as unfixed rather than as false positives, 1 deliberate `console.info` in the demo shop's webhook route, 1 Cypress typing gap) — no rule removed, no blanket disable file. A separate 233 findings on the first run were **an unbuilt `dist/` resolving to `any`**, not defects: `lint` now builds its workspace dependencies exactly as `typecheck` already did. **Every rule family was given a deliberate violation and each caught it** — `no-dupe-keys`, `no-floating-promises` (the one that cannot fire without a real type checker, so it is the evidence `projectService` resolves each package's tsconfig), `rules-of-hooks`, `@next/next/no-img-element`, `no-console` and the `testing/**` ban; see the notes. **One of those six proofs was weaker than it read, and the review pass that followed corrected it**: `@eslint/js` recommended was scoped to `**/*.js`, so the `no-dupe-keys` proof fired in `examples/webhook-receiver/index.mjs` and nowhere else — 42 base rules (`no-fallthrough`, `no-debugger`, `no-unsafe-optional-chaining`, `no-constant-binary-expression`, `no-async-promise-executor`, `use-isnan` …, none of them reported by `tsc` either) were silent over the 207 TypeScript files that are almost all of the source. `eslint --print-config` on a `.ts` file: **48 active rules before, 90 after**. No source change was needed to turn them on. `--max-warnings 0` is load-bearing, not decorative: the Next plugin sets several rules to `warn`, and without the flag such a finding exits 0 (measured both ways). **The gate now has a gate**: `frontends/packages/config/src/eslint.test.js` (63 assertions, run by `just test-web`) resolves each rule set the way ESLint does and fails if a rule family goes missing, and asserts every package `git ls-files` finds still declares `eslint . --max-warnings 0` and still reaches the shared factory. It was written against measured mutations, each of which left `pnpm -r lint` at exit 0 with 15 of 15 `Done`: a deleted `lint` script (pnpm skips a missing script silently — the mechanism that hid the original defect), an `eslint.config.js` rewritten to `export default []`, a dropped `--max-warnings 0`, a deleted `no-console` block, `no-restricted-imports` set to "off", a whole-file disable directive, and a line disable with no stated reason. Every one of them now fails the suite. **The class-string rules landed 2026-09-07, in the review pass, not the implementing one** (exp26 Lane A; [lane-a-review.md](plans/exp26-notes/lane-a-review.md) finding 1): `eslint-plugin-better-tailwindcss@4.7.0` had been added to `@vpay/config`'s dependencies and to the lockfile and then imported by nothing, so plan §5 Lane A step 7, plan §3 rule 1 (the maintainer's "no class attribute longer than one line") and plan §5's own mutation all read as delivered while `eslint --print-config` on a component reported **0** rules with "tailwind" in the name and a deliberately six-line class attribute left `pnpm --filter @vpay/ui lint` at exit 0. Six rules are now wired behind a new `tailwind` option, **off by default and on only for `@vpay/ui`**: the plugin compiles the Tailwind entry point through the linted package's own `tailwindcss`, so enabling it wherever React renders aborts ESLint outright in `@vpay/checkout` and `@vpay/dashboard`, which are on Tailwind 3 until lanes B and D migrate them — each app turns the flag on in the commit that moves it. Six mutations, one per rule, each staged and reverted. Plan §7 names `no-unregistered-classes`; under the pinned 4.7.0 that rule is `no-unknown-classes`, and it is the one that catches a daisyUI-5-removed class inside `@vpay/ui` independently of `just verify-ui`'s grep. **One hole is named, not fixed:** `frontends/packages/ui/.storybook/{main,preview}.ts` are typechecked by nothing — that package's tsconfig says `include: ["src", ".storybook"]`, but TypeScript's include-glob expansion skips dot-directories, so `tsc --listFiles` never lists either file and `pnpm -r typecheck` has not covered them since the package was created. ESLint lints them, minus the type-aware rules (`outsideTsconfig`). Fixing it means changing what `pnpm -r typecheck` covers — a different gate than this pass touched — so it is left as a maintainer's call rather than folded into a lint pass. **A full CI run of this change now exists and is green** (run `33935680386` on `9cf3df0`, 2026-09-05 — the same tree as this commit apart from the documentation lines that record the result): all six jobs — `web`, `rust`, `e2e (compose)`, `supply chain`, `deploy (helm chart)`, `self-checks` — pass. The `web` job installs `node: v22.23.2` from `.nvmrc`; `rust` reports `1166 tests run: 1166 passed, 0 skipped` in 765 s and `verify-ignored: 0 ignored (expected 0), 42 test binaries (expected 42), 1166 total`; `e2e` reports `All specs passed!` on both spec files (7 and 4 tests). **The first run, on the pre-baseline commit, failed `web`, `rust` and `e2e` at `pnpm install --frozen-lockfile`** — see the Node-baseline paragraph above. Earlier, before any CI run: `just test-rust` did not complete on the authoring machine — it aborted on a testcontainers Docker connect failure, with no `DOCKER_HOST` set — and the review pass ran it with the daemon reachable: **1159 passed, 0 skipped, 42 binaries, no retry consumed**, confirming that as an environment result and not a code one (no Rust file was changed by either pass). **Re-measured 2026-09-05 after the rebase onto `master`: 1166 passed, 0 skipped, 42 binaries, 602 s, no retry consumed** — the seven additional tests come from `master` (the `verify-status` lexer and the release-claims passes), not from this branch. `fmt-check`, `clippy`, all four `verify-*`, `test-doc`, `verify-ignored`, `deny`, `lint-web`, `test-web` (**723 passed, 0 skipped** — 660 plus the 63 the new guard adds) and `audit-web` are green; see `docs/plans/step9-notes/web-lint.md` and `docs/plans/step9-notes/web-lint-review.md`. |
| `@vpay/tokens` status tokens | ✅ | 8 tests incl. "success tone belongs to `succeeded` alone" and, new 2026-09-07, D4: `checkoutOutcomeTone.canceled` is `warning`, `failed` stays `error` |
| `@vpay/ui` — Base UI 1.8.0 + Tailwind 4 + daisyUI 5 (exp26 Lane A) | 🟡 | **New 2026-09-07** (`docs/plans/2026-09-07-ui-revamp.md`, Lane A of 4; B/C/D — checkout/shop/dashboard — not yet landed). `@base-ui-components/react@1.0.0-rc.0` (deprecated — the package was **renamed**, not superseded) → `@base-ui/react@1.8.0`; `tailwindcss@3.4.17` → `4.3.3`; `daisyui@4.12.23` → `5.7.28`; `tailwind-merge@2.6.0` → `3.6.0`. Fourteen components plus `cn()`, plus five layout/typography primitives (`PageShell`, `Heading`, `Text`, `Stack`, `List`) the plan's per-screen table names as `@vpay/ui` exports but its component-count table omits — built here so Lane B is not blocked. `cn()` now knows daisyUI: `extendTailwindMerge` with 16 conflict groups, so `cn('btn btn-primary','btn-ghost')` actually resolves to one class, not two — a decisive test per group (`cn.test.ts`, 18 cases; delete one group entry and its case fails). `PayerSheet` (`vaul` + `framer-motion`) is deleted; `Drawer` on `@base-ui/react/drawer` replaces it, this time **with** the interaction test it never had. Decision D2 taken: `Checkbox` renders Base UI's native `<button>` control (`render={<button/>} nativeButton`), not `<span role="checkbox">` — verified by compiling `styles.css` and reading the generated rule: daisyUI 5 styles **both** `.checkbox:checked` and `.checkbox[aria-checked="true"]`, so the visible control paints correctly either way. `just verify-ui` (new, see below) currently fails on the unmigrated `frontends/apps/checkout` (`form-control`/`label-text`, daisyUI 4 classes daisyUI 5 removed) — expected, and Lane B's job, not a Lane A defect. `pnpm --filter @vpay/ui test`: **46 tests, 16 files**, all green, including a structural axe-core pass (see Storybook row) sanity-checked against a deliberately unlabelled `<button>` before being trusted on the real tree. `@vpay/ui`'s own `class_tokens_distinct` rose 35 → 130 (measured with `exp26-plan-count.sh`; the plan predicted 80–110 — higher here because the count also includes the five layout primitives and every `.stories.tsx` variant, neither of which the plan's estimate separated out). The `OUTSIDE @vpay/ui` figures the 80% target reads are **unchanged** (92 distinct tokens, 18 styling files) — Lane A does not touch `checkout`/`dashboard`/`shop`. **Reviewed 2026-09-07** ([lane-a-review.md](plans/exp26-notes/lane-a-review.md)): **not safe as drafted** — six findings fixed, one open question surfaced. Two shipped components carried the exact defect the revamp exists to prevent, a class that still parses and quietly stops styling anything: `Select`'s popup width was written in Tailwind **3**'s bare-variable syntax, which Tailwind 4 compiles to the invalid declaration `width: --anchor-width` rather than rejecting (proved on the compiled CSS, before and after), so the locale-switch popup sized to its content instead of its trigger; and `Drawer`'s backdrop was a raw `bg-black/40` where plan §3 permits no colour utility at all inside this package. `cn()` merged daisyUI's **style** modifiers (`outline`/`soft`/`dash`) into the same conflict group as its **colours**, so `cn('btn-primary','btn-outline')` returned `btn-outline` and the colour vanished — read against daisyUI 5.7.28's own CSS, `.btn-outline` derives its border from `--btn-color`, which `.btn-primary` sets, so the two compose by design. Split into colour and style groups; **no existing assertion changed**, four added. `ghost`/`link` deliberately stay with the colours and the reason is recorded as an open question, not decided. Test coverage 46 → **60 cases in 17 files**: decision D2's own evidence (a wrapping `<label>` and a `htmlFor` label both toggle the native-button control — measured, D2 is sound) which the swap had dropped; `Dialog`'s focus trap and Escape; `Select`'s keyboard navigation; `Input`, which had no test file at all. **Three things are deliberately NOT asserted and the tests say so**: the Space key (jsdom does not translate it to a click on a `<button>`, measured — what is asserted instead is the native `<button type="button">` and the click, the two halves a browser's behaviour is made of), `Select`'s Enter commit (Base UI does not commit on a synthetic keydown, measured on four targets), and contrast. The axe harness's negative control, previously a sentence in this file, is a test that runs first — proved by making `axeViolations` return `[]` unconditionally and watching only that case fail. **The intermittent failure commit 177645e reports could not be reproduced**: the parent commit's `src` restored and the suite run **twelve** consecutive times, 12/12 green, 0 unhandled errors — the `unmount()` discipline is right and kept, its characterised rate is not evidence, and one Base UI render (`drawer.test.tsx`'s Escape case) had been missed by that commit and is fixed. Five consecutive runs on the reviewed head: 60/60 each, 0 unhandled |
| `frontends/apps/checkout` on `@vpay/ui` (exp26 Lane B) | 🟡 | **New 2026-09-07** (`docs/plans/2026-09-07-ui-revamp.md` §4.1, Lane B of 4, on Lane A's reviewed head `08d9b8e`). `@base-ui-components/react` deleted from this app; `screens.tsx`, `checkout-view.tsx`, `return-view.tsx` and `locale-switch.tsx` now compose `@vpay/ui`'s `Alert`, `Badge`, `Button`, `Card`/`CardBody`, `Checkbox`, `Field`/`FieldLabel`/`FieldDescription`/`FieldError`, `Heading`, `Input`, `List`, `PageShell`, `Select`, `Spinner`, `Stack`, `Text` — no `cva`, no daisyUI class literal, anywhere in this app. `tailwind.config.ts` deleted; `postcss.config.js` → `@tailwindcss/postcss`; `globals.css` → `@import '@vpay/ui/styles.css'` plus the one `prefers-reduced-motion` block (unchanged, still the sole `!important`). `next.config.ts` gained `transpilePackages: ['@vpay/tokens', '@vpay/ui']` — this is the first `next build` any app in the workspace has run against a real `@vpay/ui` import, and it failed on the package's `.js`-suffixed relative imports until the Lane A review fixed them upstream (`e2a0a09`); no workaround remains in this app's config. **`OutcomePanel` now passes `checkoutOutcomeTone[kind]` straight into `Alert`'s `tone` prop** — the `TONE_CLASS` lookup map and the `` `alert mt-4 ${tone}` `` template literal the plan named as its headline "no ternary assembling classes" case are both gone; decisive mutation run and confirmed: hard-coding `tone="neutral"` for every outcome fails `checkout-view.test.tsx`'s "does NOT render a failed payment in the neutral tone a cancelled one beats" (`AssertionError: a failure must carry a tone: expected 'mt-4 alert' to contain 'alert-error'`), reverted. **`src/config/theme.ts` collapses from 176 to 97 lines** (§8.5): daisyUI 5's `--color-primary` takes any CSS colour, so the sRGB→OKLCh conversion (`toOklch`, `foreground`, `cut`, `format`, `primaryOverride`) is gone; what remains is `linearRgb` (still the `#rrggbb` validator), `isDark`, and a `themeStyleSheet` that emits `--color-primary:<hex>;--color-primary-content:color-mix(in oklch, <hex> 20%, white|black)` — the browser computes the foreground now, not this module. §8.5's own two open questions are answered: (1) `color-mix` holds contrast — `theme.test.ts` reimplements the OKLab mixing **independently** (not importing from `theme.ts`) and asserts WCAG AA (≥4.5:1) for six colours including `#ffffff`/`#000000`/`#1d4ed8`; measured ratios 4.93:1 to 18.10:1, all six clear it, so the color-mix approach was kept rather than falling back to the OKLCh-literal alternative §8.5 named; (2) the "emits an unvalidated colour" property still holds — decisive mutation run and confirmed: skipping the `linearRgb` null-check lets `''`, `` `#f3c62 3` ``, `javascript:alert(1)` and `` `</style><script>` `` all through, failing 10 of `theme.test.ts`'s cases, reverted. `layout.test.tsx` updated to assert `--color-primary:#f3c623;` (daisyUI 5's literal-colour form) rather than the old `--p:84.2251% 0.165456 91.330667` OKLCh string; its hydration-regression assertions (no explicit `<head>`, `href`/`precedence` hoisting) are unchanged. Decision D2's native-`<button>` checkbox reached this app: `checkout-view.test.tsx`'s "toggles from the sentence and reaches the keyboard" case is rewritten to assert `tagName === 'BUTTON'` and `tabindex="0"` rather than simulating a Space keydown/keyup pair — jsdom does not translate Space to a click on a real `<button>` (the same limitation Lane A's own review measured and documented for `@vpay/ui`'s `checkbox.test.tsx`); ~~a real browser's native activation is what `just test-e2e` now proves instead~~ — **WRONG, corrected 2026-09-07 (review finding 6): no Cypress spec touches the memory opt-in at all, so Space/Enter activation of this control is covered by nothing. Named as a gap rather than as coverage. The label-click half IS measured — `@vpay/ui`'s `CheckboxLabel` case clicks the sentence and expects the handler**. `LocaleSwitch`'s test (`checkout-view.test.tsx`, "switches locale without navigating") is rewritten from `fireEvent.change` on a native `<select>` to click-open-then-pointerdown-then-click on `@base-ui/react/select`'s button-trigger-plus-listbox, matching `@vpay/ui`'s own `select.test.tsx` pattern — a native `<select>` no longer exists on this page. `vitest.setup.ts` gained the same `PointerEvent`/`hasPointerCapture`/`ResizeObserver` jsdom polyfills `@vpay/ui`'s own suite carries (guarded behind `typeof window !== 'undefined'`, since this app's default test environment is `node`, not `jsdom` — the class-declaration-time `extends MouseEvent` in the unguarded copy threw `ReferenceError` in every `node`-environment file until this was caught). `eslint.config.js` gained `tailwind: true` (Lane A review finding 4): `pnpm --filter @vpay/checkout lint` is clean under the six `eslint-plugin-better-tailwindcss` rules, `just verify-ui` is clean (`pnpm --filter @vpay/checkout build`'s compiled CSS was checked directly: `.btn-primary`, `.checkbox`, `.fieldset`, `.select{`, `--color-primary` are all present in `.next/static/css/*.css`, 8 occurrences of the last). **Vitest: 448 → 459 cases, 23 files, 0 skipped** — the suite *grew*, not shrank; `theme.test.ts` alone went from 20 cases (one `for` loop of daisyUI-comparison assertions) to 31 (one `it` per colour and per refused value, not a loop inside a single `it`, so a single bad hex is named in the failure rather than folded into one assertion). **Counts** (`exp26-plan-count.sh`), `@vpay/checkout` only: `styling_files` 5 → **2** (target ≤1, **missed by one** — `app/layout.tsx`'s `bg-base-100 min-h-screen` is unchanged app chrome per plan §4.1's own table, and `screens.tsx` keeps `h-8 w-auto` on the brand logo `<img>` to override Tailwind's own `img{height:auto}` preflight reset, `text-3xl tabular-nums` on the payment amount because `Text`'s largest size variant is `text-lg` and the amount is the single most important number on a payment page, `break-all` so a long session reference cannot overflow its card, `sr-only` for two visually-hidden labels, and `cursor-pointer` on the memory opt-in's `<label>` — none of these five is a daisyUI component class or a colour, and none has an `@vpay/ui` primitive that covers it); `class_tokens_distinct` 71 → **9** (target ≤14, **met** — the nine are exactly the tokens named above, `bg-base-100`/`min-h-screen`/`h-8`/`w-auto`/`text-3xl`/`tabular-nums`/`break-all`/`sr-only`/`cursor-pointer`); `classname_sites` 59 → **8**; `class_tokens_total` 161 → **10**; `css_lines` 13 → **11**. **A daisyUI 5 + bumblebee visual characteristic worth recording, not a defect this lane introduced**: `Checkbox`'s unchecked control renders as a full circle rather than a rounded square, screenshotted and looked at (`docs/plans/exp26-notes/lane-b/entry-screens.png`, `branded.png`) — `--radius-selector: 1rem` on a `1.5rem × 1.5rem` `.checkbox` box exceeds half the box's own dimension, and a browser clamps `border-radius` at 50% in that case; the class this app renders is exactly `class="checkbox"`, daisyUI's own compiled CSS, unmodified by anything in this commit. **Gates**: `pnpm --filter @vpay/checkout typecheck` clean; `pnpm --filter @vpay/checkout lint` clean; `pnpm --filter @vpay/checkout test` 459/459; `pnpm --filter @vpay/checkout build` (a real `next build`) succeeds, and the compiled CSS was read directly rather than trusted; `just verify-ui` exit 0; `just verify` all eleven gates exit 0 (Rust untouched, confirmed by `git status` on `backends/` and `schemas/` before running); `just test-web` green (`@vpay/checkout` 459/459, every other package unaffected). ~~**`just lint-web` cannot go green repo-wide yet, and this is a pre-existing, out-of-scope finding, not a Lane B defect**~~ — **WRONG, corrected 2026-09-07 by the Lane B review (`docs/plans/exp26-notes/lane-b-review.md` finding 3).** `pnpm -r typecheck` failed on `frontends/apps/dashboard/tailwind.config.ts` and that WAS this lane's regression. The comparison reported as "reproduced identically on Lane A's own unmodified head" was run in a worktree at a different filesystem depth, and Node's resolution algorithm walks up the directory tree, so where a worktree sits changes where it stops. Re-run in ONE worktree with only the commit changing, fresh `pnpm install --frozen-lockfile` each time: `pnpm --filter @vpay/dashboard typecheck` exits **0 at `08d9b8e`** and **2 at `c105884`**. Mechanism: daisyUI 4's `src/index.d.ts` imports `tailwindcss/plugin` and declares no `tailwindcss` peer, so under `node-linker=isolated` TypeScript walks up into pnpm's hidden hoist directory and takes whichever version pnpm hoisted — 3.4.19 while two packages were on Tailwind 3, 4.3.3 once this lane moved `@vpay/checkout` to Tailwind 4. Fixed by a `pnpm.packageExtensions` entry in the root `package.json` declaring the peer daisyUI 4 always had (seven lines of lockfile; no `frontends/apps/dashboard` file touched, that app stays Lane D's). `just ci` is green on the reviewed head. **Cypress, this lane's actual gate** — ~~`just test-e2e` itself could not complete~~ **WRONG, corrected 2026-09-07 (review finding 8): `just test-e2e` completes on the reviewed head, exit 0, `checkout.cy.ts` 1/1 + `dashboard.cy.ts` 3/3 + `shop-hosted.cy.ts` 3/3 + `shop-embedded.cy.ts` 4/4 = 11/11, 0 failing, 0 pending, 0 skipped — plan §7 row 3's pass condition exactly, dashboard image built and healthy.** What was reported at `c105884`: — `docker compose`'s `dashboard` image also fails to build for the identical `.js`-import reason checkout's did before the Lane A review fix, because `frontends/apps/dashboard/app/page.tsx` already imports `@vpay/ui`'s `StatusBadge` from the original two-component scaffold and dashboard's own `next.config.ts` has no `@vpay/ui` in `transpilePackages` — out of scope (Lane D's `next.config.ts`), not attempted. Brought up the other seven `compose.e2e.yml`/`compose.demo.yml` services by name (`demo_project=exp26b`, ports 23001/23080/28080/28082/28083, all confirmed free before use) and ran the three specs that do not need `dashboard`: **`checkout.cy.ts` 1/1, `shop-hosted.cy.ts` 3/3, `shop-embedded.cy.ts` 4/4 (the `VPAY_E2E_FRAMED=1` spec) — 8/8, 0 failing, 0 skipped**, including the `data-testid="continue"` migration in `shop-hosted.cy.ts`/`shop-embedded.cy.ts`'s Orange-redirect steps (both specs previously read `button.btn-primary`, a daisyUI class as a test selector). `dashboard.cy.ts` (3 tests) did not run. Stack torn down (`docker compose down -v`) after. Four screenshots regenerated against the compiled stylesheet and looked at: `outcomes-hosted.png` (succeeded/failed/canceled — green/red/**amber**, D4's `canceled → warning` visibly correct), `outcomes-return.png` (succeeded/failed on `ReturnView` — no `outcome_canceled` fixture exists for the return page, a payer only reaches `/return` after actually visiting a rail), `entry-screens.png` (rail selector, MSISDN form, redirect prompt, waiting, in French), `branded.png` (§6.6's decisive check: `primary_color: "#1d4ed8"` on `collect_msisdn` — not `select_rail`, whose buttons are all `variant="outline"` and cannot show a retinted primary — renders `#1d4ed8` with a **white** foreground). **What was NOT done**: `dashboard.cy.ts` (blocked, out of scope, above); `axe-core`/`cypress-axe` real-browser contrast (plan §7 row 6) — not built by this lane, and Lane A's own report already named it unbuilt by anyone; the two Cypress lines this lane owns in `shop-hosted.cy.ts`/`shop-embedded.cy.ts` are the only lines touched in those files, per plan §5's "Owns" list. |
| `frontends/apps/checkout` on `@vpay/ui` — Lane B **sabotage review** (exp26) | ✅ | **New 2026-09-07** (`docs/plans/exp26-notes/lane-b-review.md`), over `c105884`. **Verdict: not safe as drafted — nine findings, all fixed.** Three of the lane's own reports were wrong and are struck through in the row above rather than rewritten. **(1) The payment-failure alert did not clear WCAG AA.** daisyUI 5 bumblebee pairs `--color-error-content` with `--color-error` at **3.53:1** against AA's 4.5:1, and `.alert-error` is what `OutcomePanel` renders for `failed` — the one screen that tells a payer their money did not move. A REGRESSION of this revamp, not inherited: the same alert under daisyUI 4 was black on `#ff5861`, **6.82:1**. Measured three ways — the compiled theme block, the pixels of the lane's own committed `outcomes-hosted.png` (bg rgb(255,98,102), darkest glyph rgb(128,21,24)), and the pixels of the daisyUI 4 screenshot it replaced. `.alert-info` is 4.27:1 by the same measurement. Fixed with two theme tokens in `@vpay/ui/src/styles.css` at daisyUI's own hue and chroma, darkened only as far as AA needs, declared UNLAYERED so they beat daisyUI's `@layer base` block (the mechanism `theme.ts` already uses at runtime for `--color-primary`). **`frontends/packages/ui/src/theme-contrast.test.ts` is the gate plan §7 row 6 asked for and nobody had built**: it compiles `styles.css` with the real `@tailwindcss/postcss` and the real daisyUI, resolves each `--color-*` THROUGH THE LAYER CASCADE, and computes the WCAG ratio for every tone a component can render. Two decisive mutations — delete the override (3 cases fail at 3.54/4.27), and move it INSIDE `@layer base` (the same 3 fail, which is what proves the gate models the cascade rather than source order). Real-browser confirmation, Chrome, computed styles rasterised to sRGB from the app's own `next build` stylesheet: `.alert-error` **4.62:1**, `.alert-success` 5.12:1, `.alert-warning` 5.24:1, `.btn-primary` 5.51:1. `secondary` is 4.09:1 and is deliberately NOT asserted because no component's `cva` map exposes it — a case fails if one starts to. **(2) The language switch had lost its visible label**: `<label className="text-sm opacity-70">` became `aria-label` on `Select`, so the accessible name survived and the visible word did not — `getByLabelText` is satisfied by an `aria-label`, which is why nothing failed. Plan §4.1's row deletes that label's CLASSES, not the label. Restored via `Select`'s new `aria-labelledby` plus a visible `<Text>`; the new case asserts the name is RENDERED TEXT and not `sr-only`. **(3) The `<header>` banner landmark had been deleted from both views** when `<header className="flex items-center justify-between gap-4">` became `<Stack>` (a `<div>`). `Stack` gains `as`; decisive mutation fails with `Unable to find an accessible element with the role "banner"`. **(4) `just ci`'s dashboard typecheck failure was this lane's, not pre-existing** — see the correction in the row above; fixed by a `pnpm.packageExtensions` entry declaring daisyUI 4's undeclared `tailwindcss` peer. **(5) Seven raw utilities in `screens.tsx` against plan §3** ("raw utilities are allowed only inside `frontends/packages/ui/src/`"; `cursor-pointer` is not in §3's permitted set at any location) moved into `@vpay/ui` as `Text`'s `size="3xl"`/`numeric`/`wrap="anywhere"` and the new `VisuallyHidden`, `Logo` and `CheckboxLabel`. This also removed a dead prop: the amount was `<Text size="lg" className="text-3xl">`, where `cn()` drops `text-lg`. `screens.tsx` now contains the string `className` zero times. **(6) Plan §7 row 5's axe run over "every checkout screen" had never been built** — 46 new cases, every state in `CHECKOUT_SCREENS` and `RETURN_SCREENS`, both locales, zero violations; `@vpay/ui` gains a `./testing` export so the rule list is not restated. Decisive mutation: delete `MsisdnForm`'s `<FieldLabel>` — 4 cases fail with `[ 'label: 1 node(s)' ]`. **Counts** (`exp26-plan-count.sh`, final head), `@vpay/checkout`: `styling_files` 5 → **1** (**target ≤1 MET**; the one file is `app/layout.tsx`, exactly what plan §4.1 names as staying), `class_tokens_distinct` 71 → **2** (`bg-base-100`, `min-h-screen`; target ≤14), `classname_sites` 59 → **1**, `class_tokens_total` 161 → **2**. Repo-wide OUTSIDE `@vpay/ui`: `class_tokens_distinct` 92 → **33**, `styling_files` 18 → **14**, `classname_sites` 107 → **49**. Shop and dashboard are Lanes C and D and are untouched, so the 80% repo-wide targets are not expected to be met yet. **Eleven mutations run and reverted**, `git diff --stat` empty after each: the lane's five (15/1/10/13/1 cases failed) plus this review's six, of which one — removing `@vpay/ui` from `transpilePackages` — **fails nothing**, so that line is declared because it is true of the package and not because a gate proves it, and its comment now says so. **Vitest: 459 → 507 cases in 24 files, 0 skipped** for `@vpay/checkout`; `@vpay/ui` 46 → **74 in 18 files**, 0 skipped. **`just test-e2e` completes: 11/11 across all four specs, 0 failing, 0 pending, 0 skipped** (`checkout.cy.ts` 1, `dashboard.cy.ts` 3, `shop-hosted.cy.ts` 3, `shop-embedded.cy.ts` 4 under `VPAY_E2E_FRAMED=1`), `demo_project=exp26b-review` on free ports, stack torn down with `down -v` — plan §7 row 3's pass condition exactly. All four screenshots regenerated on the reviewed head and looked at; the contrast numbers above were read off `outcomes-hosted.png`'s own pixels. **Surfaced, not decided**: `Checkbox` renders as a full circle (Chrome computes `border-radius: 16px` on a 24px control, because bumblebee sets `--radius-selector: 1rem`), which on a payment page invites a payer to read a single opt-in as "choose one of". It is a theme value — the app renders `class="checkbox"` and nothing else, and daisyUI 5.7.28 does style `.checkbox[aria-checked=true]::before`, so decision D2's native `<button>` does get its tick. Overriding `--radius-selector` is one line in the block finding 1 added; it is a visual design decision and belongs to the maintainer. **Gates on the final head, exit codes read from a file**: `just ci` **exit 0** — Rust unchanged from master as it must be (`backends/` untouched): **1466 tests run, 1466 passed, 0 skipped**, `verify-ignored` 0 ignored / 45 binaries, **98 doctests** across 14 crates, `verify` all eleven gates, `deny` clean; web `@vpay/checkout` **507 in 24 files**, `@vpay/ui` **74 in 18**, shop 96, config 63, tokens 8, api-client 4, stripe-js 146, nodejs 190, 0 skipped anywhere. `just test-e2e` **exit 0, 11/11**. `just lint-web`, `just verify-ui`, `just build-storybook`, `just audit-web` ("no known vulnerabilities", both the production graph and the whole workspace including dev dependencies, after this review's lockfile change): all exit 0. **What this review did NOT do**: `cypress-axe` as a general harness (plan §7 row 6) — its specific question is answered and gated, the reusable harness is not; **Space/Enter activation of the memory opt-in is covered by no test anywhere**, and `checkout.cy.ts` is not this lane's file to add one to; plan §6.5's `<head>` mutation was run as a vitest mutation rather than a Cypress one. |
| `@vpay/ui` production build (`next build`) | ✅ | Was broken: relative imports used a `.js` suffix (`'./cn.js'`); `moduleResolution: "bundler"` let `tsc`/Vitest resolve that back to the `.ts` source, so both passed while Next's webpack resolver took the suffix literally and failed with `Module not found`. Suffixes were dropped from `frontends/packages/ui/src/index.ts` and every component file; `pnpm -r build` now compiles all packages including the dashboard's `next build`. **It broke a second time, the same way, and this row was edited past it — corrected 2026-09-07** ([lane-a-review.md](plans/exp26-notes/lane-a-review.md) finding 9). The exp26 component set reintroduced the suffix on all 48 files in `frontends/packages/ui/src`, and the documentation commit that followed edited this very row while the sentence above had stopped being true: `pnpm --filter @vpay/dashboard build` failed with `Module not found: Can't resolve './components/button.js'`, so the dashboard image `just test-e2e` builds could not be built at all. **Nothing in the lane's gate list could see it** — neither `just lint-web` nor `just test-web` runs a `next build`, and `moduleResolution: "bundler"` keeps `tsc` and all 60 vitest cases green over it. Reported by the Lane C pass, reproduced on the unmodified head before fixing. Suffixes dropped again, and **gated this time**: `just verify-ui` check 5 greps for a `.js`-suffixed relative import under `frontends/packages/ui/src` — the gate re-runs the original failure's cause, not a proxy, and restoring one suffix on `index.ts` alone makes it exit non-zero. After: `pnpm --filter @vpay/dashboard build` exit 0 ("Compiled successfully", 4/4 static pages) and `pnpm -r build` exit 0 across the workspace. **The standing lesson, recorded rather than assumed away:** a workspace package that ships TypeScript source is resolved by three different resolvers with three different rules, and only the consumer's bundler is strict. Run `pnpm -r build` before reporting a change to `@vpay/ui` |
| Storybook (`@vpay/ui`) | 🟡 | **Storybook 8 → 10, 2026-09-07** (`addon-essentials`, gone past 8.6.14, replaced by `addon-a11y` + `addon-docs`). `pnpm --filter @vpay/ui build-storybook` green, including the checkout app's still-unmigrated stories the shared install also builds (plan §6.8). **Fifteen of `@vpay/ui`'s components now have stories** (was: only `StatusBadge`), one theme only — `bumblebee` — since that is the only one that ships. Structural axe-core checks (`label`, `button-name`, `aria-*`, `region`, `list`) run in `vitest`, not in Storybook's own a11y addon, against a kitchen-sink render of every component: 0 violations. **`color-contrast` is still not checked anywhere in CI** — jsdom computes no real style, and `cypress-axe` against a running stack (the plan's row 6, a real-browser check) is not built by any lane yet. **Re-measured by the review, 2026-09-07:** `build-storybook` exit 0 under Storybook 10; all 15 component modules have a story, 0 missing. Storybook's a11y addon runs in the Storybook **UI** and `build-storybook` does not invoke axe — no `@storybook/test-runner` is installed — so the only automated a11y numbers this package has are vitest's structural ones, and they now include a negative control. Contrast is verified by nothing, and that is stated rather than implied |
| `just verify-ui` (new gate) | 🟡 | **New 2026-09-07**, wired into `just verify` (11th gate) and CI's `self-checks` job. Five `git grep` checks (four as landed, a fifth added by the review): no palette colour utility and no hard-coded colour value, no daisyUI 4 class daisyUI 5 removed, no `!important` outside a documented exemption, no `cva` outside `@vpay/ui`, and no `.js`-suffixed relative import inside `@vpay/ui` (the regression guard for the row above). Each has a decisive mutation, run and confirmed (not just written). **Currently red** on the untouched tree — `frontends/apps/checkout/src/components/screens.tsx` still writes `form-control`/`label-text`, daisyUI 4 classes daisyUI 5 removed silently (they still parse and render; they just stop styling anything) — and stays red until Lane B's migration lands. Correction to the plan: its own `!important` exemption list named only one file; run as written it also fails on `frontends/apps/checkout/src/config/theme.ts` (a doc **comment** that uses the word in prose) and `examples/checkout-browser/index.html` (a legitimate, pre-existing, explicitly out-of-scope `[hidden]{display:none !important}`) — both now named exemptions with the reason inline. Known limitation: `git grep` only searches tracked/staged files, so a brand-new uncommitted violation is invisible to a local run until `git add`ed (confirmed by testing) — CI is unaffected, since everything it checks out is tracked. **Hardened by the review, 2026-09-07** ([lane-a-review.md](plans/exp26-notes/lane-a-review.md) finding 4). Four mutations PASSED the colour check as written and now fail it: a class held in a lookup object rather than a `className=` attribute (the check required `className=` on the same line — the same blind spot the plan's own counting script §2 records, and `TONE_CLASS` in `screens.tsx` is exactly that shape today), `bg-black/40` (no `black`/`white` in the palette list, and a numeric suffix required), `text-[#ff0000]` and `bg-[rgb(255,0,0)]` (arbitrary colour values were not matched at all, though plan §3 bans "a colour literal, a hex value" in as many words). `frontends/packages/ui/src` is in scope now too, where two further mutations had passed — plan §3's permitted-utility list inside this package ends "every colour utility without exception is a bug report against this list". A negative control (`bg-base-200 text-error`) is part of the evidence: daisyUI theme tokens must NOT fire, and do not. One path exemption added in the gate's existing style, `cn.ts`, whose doc comment names two palette classes in prose |
| `@vpay/api-client` | 🟡 | `formatAmount` done + 4 tests. **Every network call throws `NotImplementedError`** |
| Dashboard app | 🟡 | Renders a scaffold notice and a status-badge reference. **No data, no auth, no routes** — and that is **still unchanged as of 2026-09-07 (exp26 Lane D)**: this pass restyled the scaffold, it did not build slice 1. Slice 1's brief asked for sign-in, a payments list, a payment detail and sign-out; none was built, for the reason recorded since 2026-09-06 and unchanged — a dashboard session needs a token, and no grant this deployment serves can issue one (see the "`/dash/v1` read surface" and "Dashboard auth" rows under Backend). `dashboard.cy.ts` still asserts the scaffold notice, unchanged — and, as of exp26 Lane D, **run for real** against a live `compose.e2e.yml` + `compose.demo.yml` stack (isolated project `exp26d`, non-default ports, torn down after, twice — once pre-rebase, once on the final rebased head): **3/3 passing both times**, and on the final head `just test-e2e`'s full recipe completed end to end, all four specs, **11/11 Cypress tests passing** (`checkout.cy.ts` 1/1, `dashboard.cy.ts` 3/3, `shop-hosted.cy.ts` 3/3, `shop-embedded.cy.ts` 4/4). **What exp26 Lane D did move (`docs/plans/2026-09-07-ui-revamp.md` §4.2, §5 Lane D):** the app onto the same `@base-ui/react` 1.8.0 + Tailwind 4.3.3 + daisyUI 5.7.28 + `@vpay/ui` stack Lane A built, `data-theme` `corporate` → `bumblebee`, and `app/layout.tsx`/`app/page.tsx` rewritten as pure composition — `PageShell`, `Heading`, `Alert`, `Stack`, `Text`, `StatusBadge` from `@vpay/ui`, zero raw `className` strings in either file. Measured (`exp26-plan-count.sh`): `styling_files` 2 → **0** and `class_tokens_distinct` 17 → **0** (both files), exceeding the plan's own "0" target and the brief's "≤1"/"≤4". The former "design-system smoke test" section keeps rendering all five `PaymentStatus` badges once each (`dashboard.cy.ts`'s pinned assertion) but is honestly labelled as a status reference, not a payments list — no row was fabricated. The `<nav>` in `app/layout.tsx` carries the app's one `<h1>` and, correctly, no links: `docs/flows/dashboard.md`'s own rule is that a menu entry for a page nobody wrote is the same lie as an empty table, and there is exactly one page. Building this app with `next build` surfaced a real gap Lane A's first head could not have caught (no app had imported `@vpay/ui` yet): webpack does not resolve `@vpay/ui`'s `.js`-specifier-resolves-to-`.ts` source the way Vite/esbuild do, so `next build` failed with `Module not found` until `next.config.ts` gained a `resolve.extensionAlias` entry. **Superseded**: Lane A's own Opus review found and fixed the same defect at its source (the `.js` suffix removed from all 48 files under `frontends/packages/ui/src`, gated by `just verify-ui` check 5) and landed it as `08d9b8e`. Lane D **rebased onto that head**, confirmed `pnpm --filter @vpay/dashboard build` still passes with the workaround removed, and removed it — `next.config.ts` now reads exactly as the untouched scaffold did. Also on the rebase: `tailwind: true` added to `frontends/apps/dashboard/eslint.config.js` (six `better-tailwindcss` rules, confirmed active — zero findings, since this app carries zero raw `className` strings), and `just verify-ui`'s now-stricter checks (black/white, arbitrary colour values, lookup-object classes) re-run clean when scoped to this app. **New this pass:** `frontends/apps/dashboard` gained its first test files — `vitest.config.ts` + `vitest.setup.ts` (jsdom, mirroring `frontends/packages/ui`'s own Base UI pointer-capture polyfills), a layout test, and four component recipes under `src/recipes/` (`SignInForm`, `PaymentsTable`, `DetailTimeline`, `EmptyState`) with a test each, **9 tests in 5 files, run five times in a row with 0 flakes**, documented with their real (quoted, not paraphrased) source in `frontends/apps/dashboard/README.md` so exp24's pages compose `@vpay/ui` instead of raw Tailwind. None of the four recipes is wired into `app/page.tsx` — none has a data source, and a dashboard screen showing invented rows is the failure mode this file's own introduction names first. `Tabs`/`Skeleton`/`Toast` stayed unbuilt: none of the four recipes needed them. **The layout test exists because plan §5 Lane D's own decisive-mutations table named a case nothing caught**: "the layout's `data-theme` reverts to `corporate`" must fail, and until this test was added nothing did — `dashboard.cy.ts` asserts `h1` text and status badges, not the theme attribute. Run for real: `data-theme="corporate"` compiles successfully with `next build` and produces **zero** `corporate` rules in the built CSS (`frontends/packages/ui/src/styles.css` configures `bumblebee` only) — the page would render completely unthemed in a real browser, with no error anywhere. `src/layout.test.tsx` pins the rendered `data-theme` attribute directly, confirmed to fail against the mutation before landing. `just verify-ui` still fails on this tree, but only on `frontends/apps/checkout`'s pre-existing `form-control`/`label-text` (Lane B's migration, not touched here) — every one of `verify-ui`'s **six** checks (the four as planned plus the review's split arbitrary-colour check and the `.js`-suffix guard) passes when scoped to `frontends/apps/dashboard` alone. **Reviewed 2026-09-07** ([lane-d-review.md](plans/exp26-notes/lane-d-review.md)). The review's first finding was an accessibility **regression** this pass introduced and no gate saw: the `08d9b8e` scaffold wrapped its page in `<main>`, the rewritten composition did not, and axe-core 4.13.0's `region` rule went from **0 violations to 1** — "All page content should be contained by landmarks" — on the real rendered `<body>`. The lane's notes had argued an app-level axe pass was redundant because `@vpay/ui` already has one; it is not, because the landmark is a decision `app/layout.tsx` makes and no component holds. `<main>` restored and `src/a11y.test.tsx` added: axe's structural rules (plan §7 row 5's exact list) over the real layout+page `<body>` and all four recipes, 0 violations, and confirmed to fail with `region: <section>` when `<main>` is removed. Second finding, same shape: the app's own **nav-honesty rule** — "the navigation is only ever allowed to link to slices that exist", stated in `docs/flows/dashboard.md`, in this app's README and in `app/layout.tsx`'s doc comment — was enforced by **nothing**. Measured: adding `<a href="/payments">Payments</a>` to the `<nav>` left 9/9 vitest, `lint` and `dashboard.cy.ts` all green. `src/layout.test.tsx` now resolves every internal `href` the layout renders against `app/**/page.tsx` on disk (with a negative control, so it cannot pass vacuously) and fails with `expected [ '/payments' ] to deeply equal []` against that mutation. Third and fourth findings, both in the **sign-in recipe** — the file this lane wrote specifically as the contract exp24's login page is meant to be built on. Its error string was rendered twice (once through `FieldError`, once through `Alert`); and six probes that build exp24's actual login form (ADR-0017 on `claude/exp24-staff-auth`: argon2id password, then RFC 6238 TOTP behind a compare-and-swap replay guard) from the recipe verbatim **all six failed**. Three of the four missing controls are now built and tested — `pending` (a double-click called `onSubmit` twice, and the *second* submission of one code is refused by the replay guard on purpose, so it would have read as "invalid code" for a good one), `requestId` (vpay emits `request-id` on every response and the error envelope deliberately carries no field for it), and `codeLength` (six by default; the field was constrained to nothing). The fourth, a **password control for the flow's first leg, was deliberately NOT added**: `docs/flows/dashboard.md` on this branch still records "how does a human staff member prove who they are?" as an open decision, and importing exp24's answer here would settle it. It is named in the recipe's own doc comment and in the app README instead, with the shape the second leg takes. `frontends/apps/dashboard`'s suite is **22 tests in 6 files** (was 9 in 5) |
| `frontends/apps/checkout` (the hosted/embedded payment page) | 🟡 | **New 2026-09-04 (Step 9, lane 3).** A Next 15 App Router app, `output: 'standalone'`, no server actions and no cookies, serving `/c/{cs_id}` (hosted), `/e/{cs_id}?key=pk` (embedded) and `/c/{cs_id}/return?t=…` (the return trip, top-level in both modes). Every response carries `Referrer-Policy: no-referrer`, `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`; `Content-Security-Policy: frame-ancestors 'none'` on the hosted and return pages, and on the embedded page the merchant's registered origins resolved **server-side** by `middleware.ts` from `GET {VPAY_API_URL}/v1/browser/checkout/origins?key=…` — fail-closed on a missing key, a missing `VPAY_API_URL`, a failed lookup and an empty list alike, all four proven in `src/middleware.test.ts` against the shipping middleware function. The page reads the session, shows the amount (integer minor units, no floating point anywhere in the conversion), the merchant name where the deployment configured one and a neutral heading where it did not (lane 3b), and a rail selector **only when the intent offers more than one rail this page can drive**; MTN collects a Cameroon E.164 MSISDN and confirms and polls through `@vaam-apps/vpay-stripe-js`; Orange confirms with `redirect: 'if_required'` and then either navigates top-level or asks its parent to (`{type:'vpay:redirect'}`) when framed. `fr` and `en`, chosen from `Accept-Language` server-side, switchable in the page **without navigating** — a `?lang=` link would drop the URL fragment the session credential lives in. **302 vitest tests in 17 files, 0 skipped**, including a `node:http` stub of all five browser routes, both guard directions of the origin check, a credential trace of all three payer paths — the MTN push, the Orange redirect and the return page — asserting that no `console.*` call, no navigation, no `postMessage` and no rendered state ever carries a secret, and that the return trip's `t=` token appears in exactly one request; and a guard (lane r2) that nothing under `src/testing/` is imported from a shipping file. **🟡, and for two reasons.** (1) **No browser has ever seen this page's CSP enforced:** Cypress strips `Content-Security-Policy` from every document it proxies, so `frame-ancestors` is asserted as the server *sends* it and nothing here has watched a browser refuse a frame because of it — what lane 6 *did* watch a browser do is the page's second lock, its own origin check against `document.referrer`, refusing an unregistered framer. (2) The rail behind every payment it has driven is a WireMock host. What is no longer true is "no browser has rendered this page": lane 6 drove it in Chrome through `examples/shop`, hosted and embedded, on both rails. See `docs/plans/step9-notes/lane-3.md` §5, `lane-3b.md` §6 and `lane-6.md` §3a. **Restyled and made runtime-configurable 2026-09-06** (the maintainer's requirements of 2026-09-05; [docs/plans/exp21-checkout-page-notes/opus.md](plans/exp21-checkout-page-notes/opus.md)). Four changes and one refusal. (a) **daisyUI `bumblebee` alone** (was `corporate`+`business`) with **Base UI** (`@base-ui-components/react`, pinned `1.0.0-rc.0` — that package has **no stable release**, and a pre-1.0 dependency on a payment page is a maintainer's call) supplying the MSISDN `Field` and the memory `Checkbox`; the only hand-written CSS in the app is one `prefers-reduced-motion` block. Base UI's checkbox is a `<span role="checkbox" tabindex="0">`, **not** a native control, so this page's stated invariant "every control is a native `button`/`input`" is now **false and has been corrected in source** rather than left standing — the property it defended is tested explicitly instead (role, tab index, accessible name, `aria-checked`, and the label click and space key both *measured*). (b) **The five-second auto-forward is gone**, on both the payment page and the return page: one button, `Back to {merchant}`, and no timer anywhere. A failed outcome also renders the rail's own `last_payment_error.message` as data under the translated sentence, cleaned by `providerReason` (control characters stripped, whitespace collapsed, 300 characters). (c) **`branding.yaml` and `config.yaml`, read once at container start** from `/etc/vpay/checkout/` (overridable by `VPAY_CHECKOUT_BRANDING_FILE`/`VPAY_CHECKOUT_CONFIG_FILE`) and injected into the first byte of HTML — an operator's `primary_color` becomes a daisyUI `--p`/`--pc` override in `<head>`, so there is no flash of the default. Absent, unreadable, malformed, wrong-typed and rule-breaking values each cost **exactly that key** and print one `WARN` naming the file; the page renders regardless. `GET /config/v1` reports what the container loaded (no paths, no problem list — those are in the log). `checkout.allowed_methods` can only **narrow** what an intent offers and never silently: an excluded rail is listed as unsupported, and an **empty** list is refused rather than honoured. `src/config/theme.ts` converts sRGB to OKLCh in forty lines with no colour library, and its tests assert the output against values produced by **daisyUI's own converter** for six colours. (d) **Page memory**: the canonical MSISDN and the last rail, in IndexedDB, opt-in per device, cleared by unticking or by an explicit control, ninety-day horizon on read, re-validated through `normalizeCameroonMsisdn` before it reaches the form, and removable entirely by `page_memory: false`. The cost is stated on the checkbox in the payer's own language. **(e) The PIN vault the requirement asked for was REFUSED and is not built**: this page has no PIN field, `/v1/browser/.../confirm` takes only `payment_method_data[mtn_momo][msisdn]`, and no `ProviderAdapter` in the workspace accepts a PIN — so building it would have meant adding a PIN input to a payment page that goes nowhere. Left to the maintainer. **Also new 2026-09-06, at the `examples/shop` track's request: the popup peer.** A popup is not a frame (`window.parent === window` inside one), so `createFrameChannel` answered `null` and vpay said nothing to a merchant who opened the hosted page with `window.open`. The channel now takes a peer, `parent` or `opener`, with the opener's origin resolved exactly as a framer's is; `middleware.ts` resolves the origin list for `/c/{id}` too, and **its CSP is still `frame-ancestors 'none'` whatever that lookup returned** (asserted, and measured failing with the guard removed). Three deliberate departures from what was asked: no `vpay:redirect` to an opener (it would send the merchant's own page to the rail), `vpay:complete` at most once per page (a second would fire `onComplete` twice for one payment), and an unresolvable opener is not a refusal (a hosted page is complete on its own). **The popup's return trip is wired by a different rule** (the maintainer's decision, the same day): after a redirect rail the return page's referrer is the *rail's* origin, so `resolveParentOrigin` has nothing to match, and it pins the opener with `soleOrigin` — the merchant's **single** registered `checkout_origins` entry where there is exactly one, and no channel at all with none or with several. With one, the target *is* the merchant's own origin and is the only party the message could have been for; with several, picking one would be choosing a `postMessage` target by guess on a page that has just come back from a third party. `soleOrigin` counts after normalising, so one malformed entry is none. **One new cost, named rather than hidden:** `/c/{id}` and `/c/{id}/return` now make the same server-side origins call `/e/{id}` always made, on every load carrying a `key`, because only the browser knows whether it is in a popup. `fetchCheckoutOrigins` catches every failure and answers an empty list, so an API that is down costs those pages **no channel** and nothing else — but there is no timeout on that fetch, so a *hanging* API would hold their first byte, which was already true of the embedded page and is now true of two more routes. **302 → 448 vitest cases in 23 files, 0 skipped.** Fourteen decisive mutations recorded by the implementer, three more by the review; **one survived the first pass** (`offered: true` hard-coded in the client passed all 426) and produced a code change, moving that policy into `pageMemoryFor`. **Reviewed, and driven by a real browser, 2026-09-07** ([opus-review.md](plans/exp21-checkout-page-notes/opus-review.md)). The review's first `just test-e2e` was **3 failing**: the runtime theme override was emitted inside an explicitly written `<head>` element, and React threw **#418** — *hydration failed because the server rendered HTML did not match the client* — uncaught on the hosted payment page, killing all three of `shop-hosted.cy.ts`'s tests with the page stuck on "Loading this payment…". It reproduces only with a `primary_color` configured, so no unit case could have seen it; `just ci` does not run Cypress; and the branch was delivered saying, accurately, that nothing showed the specs still passed. Fixed with React 19's own hoisting (`href` + `precedence`), guarded by `src/layout.test.tsx` on the ELEMENT TREE — React's hoisting puts a `<head>` in the *markup*, which is what hoisting is. Three more fixes: **the origins lookup is bounded** at two seconds (`ORIGINS_TIMEOUT_MS`), failing closed to the same empty list, which closes the "no timeout on that fetch" gap named in the sentence before this one; **a failed outcome is no longer neutral grey while a cancelled one is red** — `@vpay/tokens` gains `checkoutOutcomeTone` and `statusTone` is unchanged, so the dashboard is untouched, and `canceled` staying red rather than amber is left to the maintainer; and **two shipped example values that broke the demo** — a `logo_url` on a host that cannot resolve (a broken image on every screen of the demo payment page) and a `public_base_url` that fired the page's own misconfiguration warning on every load of a correctly configured stack. Five documentation claims that were not true were corrected, including one in this file's flow doc that contradicted the note it cited. **`just test-e2e` on the fixed head: 11 Cypress tests, 11 passing, 0 skipped** — `checkout.cy.ts` (1), `dashboard.cy.ts` (3), `shop-hosted.cy.ts` (3), `shop-embedded.cy.ts` (4) — through a compose stack that **mounts both YAML files**, so "no container has been run with either YAML file mounted" is no longer true; the four screenshots were regenerated on that head and looked at. **What is still not proven:** **no POD has run the page with either file** and the chart templates no ConfigMap for them, so a Kubernetes deployment still cannot supply them; **no real browser has touched the IndexedDB adapter**, which is driven by a hand-written `IDBFactory` stub modelling the object graph and nothing else; **the bumblebee theme's contrast has been checked by nobody** (`@vpay/ui`'s Storybook runs axe against `corporate`/`business`, which this app does not use, and is not in `just ci`); and the rail behind every payment this page has ever driven is still a WireMock host |
| Checkout Storybook stories (a11y addon) | 🟡 | **New 2026-09-04 (Step 9, lane 3).** 22 stories covering every screen the checkout state machine can be in — loading, rail selector, MSISDN form and its rejection, Orange prompt, confirming, waiting (with and without a failed poll), redirecting, succeeded/failed/canceled, forwarding, expired, embedding refused, no drivable rail, invalid link — in both locales for the two busiest, plus three return-page screens. They are built by `pnpm --filter @vpay/ui build-storybook` (CI's `web` job) from the *same* literal states `checkout-view.test.tsx` asserts against, so a screen cannot have a story without a test or a test without a story. 🟡 because the a11y addon is configured but **nothing runs axe in CI**: `build-storybook` proves the stories compile. Lane 3b's unnamed-merchant screens are the one rendering variant covered by vitest and not by a story, stated rather than papered over |
| `examples/shop` (the demo merchant's storefront) | 🟡 | **New 2026-09-04 (Step 9, lane 7).** A Next 16 App Router site — Next 16.3.4, tRPC 11.18.0, ZenStack 2.22.3 over Prisma 6.19.3, Zod 4.5.4 — with a seeded five-product catalogue priced in **XAF integer minor units**, a cart, a guest checkout, and a server-side integration through `@vaam-apps/vpay-sdk`: `orders.create` totals the cart against the catalogue, creates a PaymentIntent and then a hosted Checkout Session under `Idempotency-Key`s derived from the order id, stores both ids and returns `session.url`. `success_url`/`cancel_url` point back at the shop with `{CHECKOUT_SESSION_ID}`. **An order turns `paid` only from the shop's own webhook handler** — `POST /api/vpay/webhook` verifies vpay's signature with the SDK, dedupes by event id in `WebhookEvent`, and answers `2xx` only after the write; the return page polls the shop's database and takes no decision from the return trip. `/orders/{id}/embedded` pays the same order through `initEmbeddedCheckout`. **57 vitest cases, 0 skipped**, with three guard-failure proofs recorded (a replaced signature check fails four cases; a bare `SETTLING_EVENTS[event.type]` lets an event typed `constructor` settle an order off the prototype; widening `Product`'s ZenStack policy fails three). Of the 17 client chunks `next build` emits, none contains the webhook secret, the private-key path, `node:crypto` or `readFileSync` — grepped, not reasoned about. ~~**Prisma 6, not 7**, and that is the newest pair that works: ZenStack 2.22.3 emits a `datasource` with `url = env(...)`, which Prisma 7 rejects (`P1012`).~~ **Superseded 2026-09-06 by the ZenStack 3 row below** — Prisma left the request path entirely and no longer constrains this package. **🟡 for two named gaps.** `ZenStackShopStore` (`PrismaShopStore` until 2026-09-06) — the class every route actually runs against — **has no automated coverage of its own**: the unit suite drives an in-memory store, the class was verified by hand on 2026-09-04 against a real Postgres from the built image, and lane 6's Cypress specs exercise it in the demo stack without asserting anything about it directly. And `orders.get` is reachable by anyone holding an order id — stated in the shop's README rather than hidden behind a policy that evaluates to `true` |
| `examples/shop` — three integration surfaces, first-class failure outcomes, optional e-mail | 🟡 | **New 2026-09-06 (exp22).** The shop now demonstrates the whole of what a merchant integrates against, not only the happy path. (1) **Three surfaces**, selected by `SHOP_CHECKOUT_MODE` (`hosted` | `popup` | `embedded`) with a visible switch on `/checkout` that says on the page that it is a demo affordance a real merchant would not ship. `hosted` and `popup` send an **identical** `POST /v1/checkout/sessions` and therefore share `Idempotency-Key`, so a payer whose popup is blocked falls back to a redirect and gets the session they already had. (2) **Failure outcomes**: `orders.failure_code`/`failure_message` carry `last_payment_error` from the settling event, written by the webhook handler and by nothing else, in the same statement as the status and only on the write that moves it; the buyer reads copy keyed on vpay's own `FailureCode` (`src/lib/failures.ts`), `orders.retry` places a **new** order (one charge per intent, forever), and `orders.cancel` asks vpay to cancel the intent and then waits — the shop still writes no settled status itself. **That wait never ends, and the demo run is what found it: `payment_intent.canceled` is emitted by nothing.** The cancel reaches vpay and the intent's row really does become `canceled`; the `events` table gains nothing, because the worker writes three types and this is not one of them (see the "Events written by the worker" row above, which has always said so). So the shop's `cancelled` status is unreachable, the button and the copy now say exactly that, and the shop was **not** changed to write the status from its own request — which is the one thing this example exists to argue against. Recorded as a server gap in `docs/plans/exp22-shop-demo-notes/opus.md`. (3) **The e-mail is optional**, with the reason on the page: the identity on a mobile-money payment is the phone number the rail holds. Migration `20260906120000_optional_email_and_failure_columns`. (4) **`SHOP_PAYMENT_METHOD_TYPES` takes a per-currency map** (`xaf:orange_money;eur:mtn_momo`) as well as a list, resolved against the order's currency before the order row is written, so the shop cannot offer a rail `currencies_agree` will refuse at confirm. **96 vitest cases, 0 skipped** (was 57 before exp22 and 93 before the review of the same day), measured 2026-09-06, and **driven end to end on a real demo stack the same day** — the run, page by page and with the orders it left behind, is in `docs/plans/exp22-shop-demo-notes/opus.md`. **And driven by a real browser**: the review ran `just … test-e2e` on its own stack and all four Cypress specs passed — **11 tests, 11 passing, 0 skipped**, including `shop-hosted.cy.ts`'s three and `shop-embedded.cy.ts`'s four, which this branch had repaired without running. **No PNG screenshots**: Cypress writes them only for failing tests and a green run produces none; the claim that its binary was unavailable here was wrong and is corrected in `opus.md`. 🟡 carries forward the store class's complete lack of automated coverage and `orders.get` being reachable by anyone holding an order id, and adds one: **no test anywhere opens a real popup** — see the popup row above |
| `examples/shop` on Tailwind 4 + daisyUI 5 (exp26 Lane C) | 🟡 | **New 2026-09-07** (`docs/plans/2026-09-07-ui-revamp.md`, Lane C of 4; branch on Lane A's head `177645e`). Decision D1: the shop adopts `tailwindcss@4.3.3` + `daisyui@5.7.28` directly, through public packages only — it imports neither `@vpay/ui` nor `@vpay/tokens`, so the integration stays one a merchant can reproduce from public npm packages, not from this workspace's private ones. `src/app/globals.css` — 229 lines of hand-written CSS with its own `:root` custom properties — is now six lines: one `@import 'tailwindcss'` and one `@plugin 'daisyui'` block on `bumblebee`, plus a paragraph explaining why daisyUI does not compromise the file's original "no design system" reasoning (the distinction was always *private workspace package* vs. *public library*, not "no CSS framework"). Every page/component that carried a bespoke class name or a `style={{…}}` — the layout, the catalogue, the cart and checkout pages, all four order pages, `cart-table`, `checkout-form`, `order-summary`, `order-actions`, `test-numbers-panel`, `order-poller`, `embedded-checkout` — now composes daisyUI classes directly (`btn`, `badge-{tone}`, `alert-{tone}`, `table table-zebra`, `fieldset`/`fieldset-legend`, `radio`, `input`, `navbar`, `card`) with plain Tailwind utilities for layout: **zero bespoke CSS lines, zero inline styles** (were 229 and 24). Every `data-testid` the two shop Cypress specs assert on is unchanged, `#vpay-embedded-checkout` included (styling becomes `card min-h-96`, per plan). `OrderStatusBadge`'s and the test-numbers table's tone come from a plain `Record<Status, string>` lookup, not `cva` — `verify-ui`'s fourth check forbids `cva(` outside `@vpay/ui`, and `unpaid`/`paid`/`failed` each still get their own badge tone (`failed`/`cancelled` share `error`, matching the CSS this replaces). Two new vitest suites (`order-summary.test.tsx`, `test-numbers-panel.test.tsx`) render the components and assert the tone-per-status and the Orange-caveat `role="alert"` properties the plan's own decisive mutations name — both confirmed to fail under the named mutation (a collapsed single tone; a removed `role`) before being left passing. **100 vitest cases, 0 skipped** (was 96). `pnpm --filter @vpay-examples/shop lint` (ESLint + `prettier --check`) and `typecheck` both green; the compiled stylesheet (`@tailwindcss/postcss` via a new `postcss.config.js` — the shop had no PostCSS pipeline before) was read after a real build, not assumed: `.btn`, `.badge-{success,warning,error}`, `.table-zebra`, `.fieldset-legend`, `.radio`, `.navbar` and `.alert-{warning,error,info}` are all present in the output. **`styling_files` rose 11 → 16 rather than falling, and that is measured and explained rather than hidden.** `exp26-plan-count.sh` only sees `className=` strings, never `style={{…}}`, so converting all 24 inline styles to daisyUI/Tailwind classes necessarily turns files that previously had **zero** `className` (the four `orders/[id]/*` pages, which used only inline styles) into "styling files" under the script's own definition — a strict improvement counted as a regression by a metric that cannot see inline styles at all. Unlike `checkout`/`dashboard`, the shop has no `@vpay/ui`-style layer to absorb classNames into a handful of files — D1 forbids importing it, by design — so this count cannot be driven toward `@vpay/ui`'s shape without either building a shop-local component library (not what the plan's own per-file table for Lane C shows, and not asked for) or leaving the inline-style debt the plan explicitly wants gone. `css_lines` 229 → 5 and `inline_styles` 24 → 0 both meet Lane C's own acceptance criteria in plan §5 (`css_lines 229 → ≤ 8`, `inline_styles 24 → 0`); no per-package `styling_files` target appears in that section, only the whole-revamp aggregate (§2, `OUTSIDE @vpay/ui`, ≤ 3 across all three apps combined, read at the final gate after Lane B and D land too). That aggregate is 92 → 136 with only Lane A + Lane C landed — expected mid-migration, not a Lane C shortfall; it will not reach its target until checkout and the dashboard also move. 🟡 carries forward the two gaps the row above already names (the store class's lack of automated coverage; `orders.get` reachable by anyone holding an order id) — this lane's scope did not touch either. **Driven end to end in a real browser, 2026-09-07**: `docker compose` up on every service but `dashboard` (left out — neither shop spec visits it), project `exp26c`, five free ports picked to avoid the `vpay-demo` project already running on this shared host. `shop-hosted.cy.ts`: **3 passing, 0 failing** (MTN push to `paid`, Orange redirect to `paid`, a declined MTN charge to `cancel_url` never `paid`). `VPAY_E2E_FRAMED=1 shop-embedded.cy.ts`: **4 passing, 0 failing** (the frame's exact `src` and CSP, MTN inside the frame, Orange breaking out, an unregistered framer refused). The full `just test-e2e` recipe itself was not run to a green result — it builds all four app images in one `docker compose … --build` and the `dashboard` image fails first, on an unrelated, pre-existing gap in `frontends/apps/dashboard`'s consumption of `@vpay/ui` through Next's webpack bundler (Lane D's territory; see `docs/plans/exp26-notes/lane-c.md` for the exact error and the workaround used to prove the two specs this lane owns instead)  **Reviewed 2026-09-07 (`docs/plans/exp26-notes/lane-c-review.md`); the branch was rebased onto Lane A's reviewed head `08d9b8e` and four corrections belong on this row.** (1) **The three `badge-{tone}` rules were in the built stylesheet only by accident.** Both components assembled the class (`` `badge badge-${tone}` ``) and Tailwind 4 generates only classes it can READ in source text, so the rules existed solely because `order-summary.test.tsx` carries the strings as expected values and automatic content detection scans test files. Measured: with that one test file moved aside, `.badge-success`/`.badge-warning`/`.badge-error` are all absent from `next build`'s output. Fixed — each map holds the complete class string — and gated by `src/testing/no-dynamic-class-names.test.ts`, which fails if any `className` in shipping source becomes a template literal with an interpolation. The sentence this row previously carried about the compiled stylesheet was true and proved nothing. (2) **Three daisyUI 5 layouts were wrong in a browser** and right in jsdom: the e-mail label sat beside its input (`.label` is an inline-flex for use inside a `fieldset`; the field is now in one), and `OrderFailureNotice` and the test-numbers caveat each split into two columns because `.alert` is a grid with `grid-auto-flow: column`, which makes `flex-col` on it inert. All three measured before and after in a real browser under `bumblebee`. (3) **`just test-e2e` runs and passes on this head** — the full recipe, **11 tests, 11 passing, 0 failing, 0 skipped** across all four specs (`checkout` 1, `dashboard` 3, `shop-hosted` 3, `shop-embedded` 4 framed), project `exp26c-review`, ports 18501-18505. The dashboard image defect that blocked it is fixed on `08d9b8e`. The plan's third decisive mutation was also run for the first time: `data-testid="cart-table"` removed, image rebuilt, `shop-hosted.cy.ts` **0 passing, 3 failing**. (4) **`just lint-web`, and therefore `just ci`, is RED with this lane alone, and this lane causes it.** Adding `tailwindcss@4.3.3` to this package flips the single copy pnpm hoists into `node_modules/.pnpm/node_modules/` from 3.4.19 to 4.3.3; `daisyui@4`'s own type declaration resolves `tailwindcss/plugin` through that copy, and `frontends/apps/checkout/tailwind.config.ts` then fails `TS2322`. Proven by reverting this lane's two manifest files and reinstalling (gate passes) and restoring them (gate fails) — the earlier `git stash` reproduction that called it pre-existing was invalid, because `git stash` uninstalls nothing. Not fixed here: the failing file is Lane B's and plan §4.4 deletes it, and with it moved aside the typecheck exits 0. **Land Lane B with or before Lane C.** `just verify-ui` stays red on the same Lane B file, unchanged. **102 vitest cases, 0 skipped** (was 100), and the shop's ESLint now runs the class-string rules — `eslint-plugin-better-tailwindcss` was wired into `@vpay/config` by Lane A's review behind a flag this package had not flipped, so plan §7's line-wrapping and unknown-class rules ran nowhere here; both proven to fail on a mutation now |
| The demo's fake test numbers, and the panel that shows them | 🟡 | **New 2026-09-06 (exp22).** One documented MSISDN per outcome, per rail, rendered on `/checkout` and `/orders/{id}/embedded` for the rails that deployment offers, and listed in `examples/shop/README.md`. `src/lib/test-numbers.test.ts` parses the README's own tables and fails if the two disagree in either direction (proven by deleting a README row and watching it fail). The steering is entirely in the **stubs' configuration** — `demo-outcomes.json` under `backends/tests/conformance/wiremock/mtn` and `.../orange` — and there is no branch on any of these values in vpay, in the shop or in either adapter. **MTN** (push, MSISDN in the merchant's own submit): `…101` insufficient_funds, `…102` payer_timeout and **new** `…503` provider_unavailable, all three decided by the **status query** and therefore settled by the worker, which emits. `…400` is `invalid_payer` and is **different**: MTN refuses it on the *submit*, so vpay commits it through `vpay_api::v1::payment_intents::persist_decline`, which writes the charge and `last_payment_error` and **emits no event at all** — `payment_intent.payment_failed` is written only by `vpay_db::settlement::apply_failed`, on the worker's path. A webhook-driven merchant therefore cannot learn of a decline at submit, and the shop's order stays `unpaid`; the table, the panel and the README all now say so (corrected 2026-09-06 in review — they promised `failed`). Pinned by `a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`, which asserts the `events` table stays empty and will fail the day vpay emits one. Whether it should is D6 in `docs/plans/exp22-shop-demo-notes/opus-review.md` — a 200 with a `FAILED`/`SERVICE_UNAVAILABLE` body, not an HTTP 503, because a 5xx is a transport failure the poll ladder retries for hours. **Orange** (redirect, no MSISDN reaches vpay): the stub's hosted page grew an **additive** test-number form — the `#pay`/`#cancel` links are untouched — which arms a scenario and 302s to the URL that submit carried: `…102` payer_timeout (`EXPIRED`), `…400` provider_error (`FAILED`). Proven end to end at the adapter by `a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` (4 cases) and the new `a_test_number_typed_on_the_rails_hosted_page_reaches_the_documented_outcome` (3 cases) in `backends/tests/conformance`, against real WireMock containers; deleting the `EXPIRED` mapping makes the second fail with `Succeeded`, measured. 🟡 for two stated limits. **Orange cannot express `insufficient_funds`, `invalid_payer` or `provider_unavailable` at all** — its documented statuses are five and it names no sub-reason for `FAILED` — and **no adapter in this repository produces `payer_declined`**, so that code of the core's eleven is emitted by nothing. And — **measured on the running demo stack, 2026-09-06, and the reason this row is 🟡 rather than ✅ — Orange's numbers do not work from a browser at all.** The confirm handler enqueues the first status query at `now()` (`poll_delay(0)` is the delay before the *second* attempt, which an earlier draft of this row and of the mappings' own comment got wrong), the worker's idle sleep is a second, and the stub's catch-all `SUCCESS` settles the charge before a payer can reach the form: journal timings T, T+449 ms, T+11.96 s, and an order that came back `paid` for `237600000400`. The mappings are nonetheless correct — the conformance case drives the same page and form with no worker racing it — so what is missing is a `PENDING`-while-on-page answer, which changes the timing of a stub `compose.yml`, CI's e2e Cypress job and both Rust suites share. Left as a maintainer decision with the options written out, not closed and not hidden: it is on the shop's own checkout panel, in the README, in the runbook and in the mapping's metadata |
| `examples/shop` on ZenStack **3** (`@zenstackhq/orm` 3.9.3), Prisma out of the request path | 🟡 | **New 2026-09-06 (exp22).** v3 is a rewrite, not a version bump: it replaced Prisma at runtime with its own ORM over Kysely. `@prisma/client` and `@zenstackhq/runtime` are gone from the package; `pg` and `@zenstackhq/{orm,schema,plugin-policy}` are in; the CLI is `@zenstackhq/cli`. **`@zenstackhq/runtime` 3.x was the wrong package to chase** — it stopped at `3.0.0-beta.13` because v3 renamed it, and `@zenstackhq/orm@3.9.3` is `latest`, i.e. **v3 is not beta**, which the previous row's note assumed it still was. The ZModel is unchanged and so is the query surface (v3's ORM is Prisma-API-compatible, which is why `zenstack-store.ts` reads as it did); `enhance()` became `new ZenStackClient(schema, {dialect}).$use(new PolicyPlugin())`. The schema moved to `zenstack/schema.zmodel`, `zen generate`'s TypeScript output is **generated by `postinstall` and not committed** (where `prisma/schema.prisma` had to be committed for the old entrypoint), and the three migration files **moved unchanged** to `zenstack/migrations/` under `zen migrate deploy` — which still drives Prisma Migrate, from a Prisma schema derived on the fly. **Measured on a throwaway `postgres:16-alpine`, 2026-09-06**: all three migrations applied from empty, `zen migrate dev --create-only` afterwards produced an *empty* migration (so zmodel and migrations agree), the catalogue seeded, and `ZenStackShopStore` was driven by hand through create / set-ids / failed-webhook / already-settled / unknown-intent. `next build` is green and none of the 18 `.js` files it emits under `.next/static` carries the webhook secret, the key path, `node:crypto` or `readFileSync` — grepped, not reasoned about. **🟡 for a regression this upgrade cost, stated rather than smoothed over.** v2's `enhance()` refused a denied write *before opening a connection*, so `policies.test.ts` proved the refusals with no database. v3's plugin needs one — every such call now fails `ECONNREFUSED` instead — **so no automated test in CI proves the policies are enforced any more**; the suite proves the rules are still the rules and the plugin is still installed (both mutation-checked), and the refusals were run by hand. And a denied **bulk** write no longer throws: `product.deleteMany({})` resolves `{count: 0}` and the rows survive, where v2 threw. Same effect, different shape; nothing in this shop relied on the throw. **Independently reproduced in review on 2026-09-06** against a throwaway `postgres:16-alpine`: `product.create` threw "operation is rejected by access policies"; `product.deleteMany({})`, `order.deleteMany({})`, `orderItem.deleteMany({})`, `webhookEvent.updateMany` and `webhookEvent.deleteMany` each resolved `{count: 0}` with every row and every column unchanged; `zen migrate deploy` applied all three migrations from empty and `zen migrate dev --create-only` then produced a 30-byte "This is an empty migration", so the zmodel and the migrations agree; and a read of one payer's order by a caller holding only the id was **allowed**, which is what the zmodel says out loud and what the shop's README states rather than hiding behind a rule that evaluates to `true` |
| `examples/shop`'s vpay configuration — `VPAY_OAUTH_AUDIENCE` | ✅ | **New 2026-09-04 (Step 9, lane 5b).** The shop reads an optional `VPAY_OAUTH_AUDIENCE` and both compose files set it on `vpay-shop`. It is the demo stack's only correct value for the client assertion's `aud`: the shop reaches vpay at `http://vpay-server:8080` while the generated overlay's `deployment.public_base_url` is the published host port, and the OP compares `aud` against that public URL alone. Without it the shop could not authenticate at all from inside the compose network — which is the defect lane 6 found and lane 5b fixed; see the client-assertion audience row in the Merchant SDKs section. Proven end to end since 2026-09-04: lane 6's `shop-hosted.cy.ts` cannot reach a payment page without a token, and it is green from nothing in the `vpay-ci` VM |
| `@vaam-apps/vpay-sdk` (`sdks/nodejs`, the Node merchant SDK) | 🟡 | See the "Merchant SDKs" section below for what its **150** tests prove (was 126 before Step 5b: `stripe-auth.test.ts` holds 21 of the 22 added, and the 22nd is a `client.test.ts` case pinning that a bad `baseUrl` is refused at construction; this row said 146 until a 2026-09-03 run corrected it to 148, and **150 since `c40a137` (same day) added `client_secret` to `PaymentIntent` and two `client.test.ts` cases for it**). 🟡 rather than ✅ because **nothing in this package has ever run against vpay itself.** Its 150 tests all run against its own `node:http` stub; the `/v1/oauth` half of the contract now exists server-side (2026-09-02), but the integration suite that exercises it uses the *Rust* SDK. What *has* driven this package's code against a running `vpay-server` is `sdks/stripe-compat`, a separate package that needs a stack — see the Stripe-SDK compatibility row in "Merchant SDKs". The only bridge to the real verifier remains `just sdk-conformance-node`, a manual recipe outside `just ci`. **The 150 in this row is a 2026-09-03 measurement; the 2026-09-04 figure is 168**, measured by the integrator's `just ci` in the `vpay-ci` VM on the merged Step 9 branch — see the `pnpm -r test` sweep row below and the Step 9 rows with it |
| `@vaam-apps/vpay-stripe-js` (`sdks/stripe-js`, the browser Stripe.js-shaped client) | 🟡 | **New 2026-09-03 (Step 5c).** Zero runtime deps, ESM, TS strict. **87 tests, measured 2026-09-03** (`pnpm --filter @vaam-apps/vpay-stripe-js test`, 6 files: `client.test.ts` 35, `polling.test.ts` 16, `redirect.test.ts` 12, `errors.test.ts` 11, `form.test.ts` 10, `compat.test.ts` 3), all against a real `node:http` stub of `/v1/browser` (`src/testing/browser-stub.ts` — excluded from `dist/`, imported only from `*.test.ts`, so it is an SDK's own unit-test stub in ADR-0006's sense, not a test double reachable from `vpay-server`/`vpay-worker-bin`). Compile-time compatibility with `@stripe/stripe-js`'s own types (a devDependency, never loaded at runtime) is pinned by `compat.test.ts` — see `sdks/stripe-js/README.md`'s "Type compatibility, precisely" for exactly what is and is not assignable in each direction. 🟡 rather than ✅ for the same reason `@vaam-apps/vpay-sdk` is: its suite is a stub of its own writing, one author's claim about a wire contract, until something drives it against the real server. That end-to-end proof is `backends/tests/integration/tests/browser_checkout.rs` (the Rust half of the same contract) plus `examples/checkout-browser` driven by `frontends/tests/e2e/cypress/e2e/checkout.cy.ts` (the actual package, running in a real browser, against the real compose stack) — both landed in the same step; see the rows below and [docs/flows/browser-checkout.md](flows/browser-checkout.md) |
| `@vaam-apps/vpay-stripe-js` popup checkout (`openCheckoutPopup`, `notifyCheckoutOpener`) | 🟡 | **New 2026-09-06 (exp22).** The third integration surface: vpay's **hosted** page in a top-level window the merchant's page opened, with completion reported through `window.opener`. The message is posted by the **merchant's own `success_url` page**, not by vpay's checkout page, and that is forced rather than chosen: inside a popup `window.parent === window`, so `frontends/apps/checkout`'s `createFrameChannel` returns `null` and the page deliberately says nothing (see the request to that track in `docs/plans/exp22-shop-demo-notes/opus.md`). `src/popup.test.ts` — **27 tests, measured 2026-09-06** — proves the origin check, the `event.source` check, that the window is opened before the merchant's server call is awaited, the blocked-window error, the opt-in `closed` poll, the round trip between the two halves, and (added in review) that a **reused** window is navigated at all: the window is opened with a stable `windowName`, so the second call of a session gets one already on vpay's origin, where `location.assign()` throws and only the `href` setter is legal. `stubPopup`'s `Location` now throws from `assign`, so pointing the code back at it fails 17 of the 27 rather than none. 🟡 because **every window in that suite is a stub, and nothing has ever opened a real one**: jsdom implements neither `window.open` nor cross-window `postMessage`, and no Cypress spec covers the popup. The demo run of 2026-09-06 got as far as the **fallback** — the automation's synthetic click carries no user activation, so `window.open` answered `null`, `CheckoutPopupBlockedError` was raised and the shop created the order in `hosted` mode and navigated, which is the path a merchant most needs to work. The popup itself remains undriven. Recorded as a dated ⛔ in [docs/sdks/parity.md](sdks/parity.md) |
| `examples/checkout-browser` | ✅ | **New 2026-09-03 (Step 5c).** Static `index.html` + `checkout.js`, no framework, importing `@vaam-apps/vpay-stripe-js`'s built ESM (vendored by `just build-checkout-browser` into the gitignored `dist/stripe-js/`, since `frontends/Dockerfile` deliberately never copies `examples/` into its build context). Served by a zero-dependency `node:http` static file server (`serve.mjs`) — the same script a human runs by hand and Cypress spawns as a child process, not two implementations. `mint.mjs` mints a PaymentIntent through `@vaam-apps/vpay-sdk` and prints a ready-to-open URL; it is plain JavaScript, not TypeScript, for the same reason `examples/merchant-node` is — no build step for a standalone example — not to work around a typing gap: `@vaam-apps/vpay-sdk`'s `PaymentIntent` now declares `client_secret` (`c40a137`, same day), so a TypeScript caller reads it typed too. **Run and verified by hand against `just demo_port=18084 demo` on 2026-09-03**: `mint.mjs` created an intent, the page rendered `requires_payment_method`, confirmed MSISDN `237600000ce0` through `@vaam-apps/vpay-stripe-js`'s `confirmMobileMoneyPayment`, and the rendered status reached `succeeded` after `vpay-worker` settled the charge — screenshotted, not just logged. Ships push-only (D4); the redirect half is untested here (see the Reconciler row). **Unchanged by Step 9 except for one constant:** the rail is now told where to send the payer, but this example has no page to receive one and was not otherwise modified — `mint.mjs`'s currency moved to `xaf` with the demo overlay (lane 4), because CI's `e2e` job brings the stack up with `-f compose.demo.yml` and a EUR intent would be refused at confirm |
| `pnpm -r test` sweep | ✅ | **247 tests total (150 `@vaam-apps/vpay-sdk` + 87 `@vaam-apps/vpay-stripe-js` + 3 `@vpay/tokens` + 4 `@vpay/api-client` + 3 `@vpay/ui`), all passing, re-measured 2026-09-03 by `pnpm -r test`** — **that 247 is the 2026-09-03 figure; for 2026-09-04 read the per-suite re-measurement at the end of this cell instead (656 by their sum, which no single run reported as a total)** — was 245 (148 `@vaam-apps/vpay-sdk`) earlier the same day, before `c40a137` added `client_secret` to `@vaam-apps/vpay-sdk`'s `PaymentIntent`; was 158 before this step added `@vaam-apps/vpay-stripe-js`. `@vaam-apps/vpay-stripe-compat` is deliberately **not** in this sweep: its script is named `compat`, not `test`, because a suite that needs a compose stack must not be able to report green in a job that has none. `just lint-web` now depends on `build-sdk-node`, and CI's `web` job builds the Node SDK before `pnpm -r typecheck`, because `sdks/stripe-compat` imports `@vaam-apps/vpay-sdk/stripe` whose types resolve to the gitignored `dist/`; in the old order a clean checkout failed with `TS2307`. (The sweep was 10 tests before the Node SDK landed on 2026-09-02.) Previously broken: `@vpay/e2e`'s `test` script ran `cypress run`, so the recursive sweep tried to launch Cypress and failed with no binary installed — `just ci` and the CI `web` job could never pass. Fixed by renaming that package's script to `e2e` (`frontends/tests/e2e/package.json`), which `pnpm -r test` no longer touches. **Re-measured 2026-09-04 by the integrator's `just ci` in the `vpay-ci` VM on the merged Step 9 branch, per suite and 0 skipped anywhere:** `@vaam-apps/vpay-stripe-js` 119, `@vaam-apps/vpay-sdk` 168, `@vpay-examples/shop` 57, `@vpay/checkout` 302, `@vpay/tokens` 3, `@vpay/ui` 3, `@vpay/api-client` 4. Two new packages entered the sweep this step (`@vpay/checkout`, `@vpay-examples/shop`) and neither declares `--passWithNoTests` |
| Cypress e2e | ✅ | **Superseded 2026-09-04 (Step 9, lane 6): four specs, 11 tests — see the `shop-hosted.cy.ts`, `shop-embedded.cy.ts` and `just test-e2e` rows below. The rest of this cell is the 2026-09-03 measurement and is kept as history.** **Two specs, 4 tests, measured 2026-09-03 (Step 5c) both on the authoring machine (`just demo_port=18084 demo`, `pnpm --filter @vpay/e2e e2e` against `VPAY_BASE_URL=http://localhost:18084`) and previously by CI.** `dashboard.cy.ts` — 3 tests, written against the compose stack, asserting only the dashboard's scaffold notice; first executed by CI's `e2e (compose)` job on run `33647189156` (2026-09-02), the Cypress CDN being unreachable from the authoring machine at the time. **`checkout.cy.ts` — new this step, 1 test**: `cy.task('mintCheckoutPaymentIntent')` mints a real intent server-side with `@vaam-apps/vpay-sdk` and the `demo-merchant` keypair (never in the browser), visits `examples/checkout-browser` (served by a `before:run`-spawned `serve.mjs`, stopped in `after:run`), confirms MSISDN `237600000ce0` (the same number `examples/merchant-demo` uses for its first outcome — the constant `DEMO_MSISDN` became `OUTCOMES[0]`'s `Steering::Msisdn("237600000ce0")` in Step 8; the number and the WireMock scenario `mtn-e2e-poll` are unchanged), and asserts the rendered `data-status` reaches `succeeded`. **Run locally against the real stack and it passed**: `✓ confirms an MTN MoMo push through @vaam-apps/vpay-stripe-js and settles to succeeded (2914ms)`, `4 passing (4s)` total for both specs. Added to `.github/workflows/ci.yml`'s `e2e` job (builds `@vaam-apps/vpay-stripe-js` and runs `just build-checkout-browser` before Cypress); **not yet observed green in CI itself** — the number above is this pass's own local run, not a CI run link, and that distinction should be closed the next time CI executes this job |
| `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts` | ✅ | **New 2026-09-04 (Step 9, lane 6).** `examples/shop` driven in a real browser against the real compose stack, to vpay's hosted page and back, three times: an MTN push on the digits-only steering MSISDN `237600000100`, an Orange redirect through the stub's own hosted page (D7) with its "Pay" link, and a payment that does not succeed (`237600000101`) that forwards the payer to `cancel_url`. Every "it was paid" assertion is made on the SHOP's return page, which polls the shop's own `orders.get` and reads nothing but the shop's database — written only by the shop's webhook handler after it verifies vpay's signature. **Proven not to pass without `vpay-worker`:** with that container stopped, all three fail on the settlement wait after 120 s, with the intents left `processing`/`requires_action` and all nine orders that run created left `unpaid` (measured on the authoring host, on a shop image carrying lane 6's temporary token-exchange workaround, which none of the three tests reaches). Green from nothing in the `vpay-ci` VM on 2026-09-04, 3 tests in 1 m 18 s. One deviation the plan did not specify: the 'cancel' journey is proven as a **decline** forwarding to `cancel_url` — vpay's page has no cancel control and the Orange stub's cancel link is the same return URL — so the order ends `failed` via the shop's webhook rather than `unpaid` |
| `frontends/tests/e2e/cypress/e2e/shop-embedded.cy.ts` | 🟡 | **New 2026-09-04 (Step 9, lane 6).** vpay's page framed on the shop's own site: the iframe's `src` asserted byte for byte against D6 (id in the path, publishable key in the query, session secret in the fragment and nowhere else), MTN completed INSIDE the frame with `vpay:complete` reaching the shop, Orange breaking out to the rail and returning to the embedded session's `return_url`, and the same credential framed from an origin nobody registered being refused before the page reads anything — the refusal proven to be origin-driven by registering that origin in the overlay and watching the same page render. It runs as a SECOND `cypress run` with `chromeWebSecurity` and Cypress's frame-busting rewrites off, because neither is settable per test and the two page modes need opposite answers from `window.parent`. 🟡 for what it cannot see: **Cypress strips `Content-Security-Policy` from every document it proxies**, so `frame-ancestors` is asserted as the server sends it (`cy.request`) and **no test in this repository has watched a browser refuse a frame because of vpay's CSP** — what a browser was seen enforcing is the app's own origin check; and the `vpay:redirect` hand-off is intercepted before `@vaam-apps/vpay-stripe-js` performs the top-level navigation (that `assign` is covered by the SDK's vitest), while the embedded Orange breakout navigation is performed by the spec. Green from nothing in the `vpay-ci` VM on 2026-09-04, 4 tests in 13 s |
| `just test-e2e` | ✅ | **Changed 2026-09-04 (Step 9, lane 6): it now brings up a stack the specs can pass against.** It used to start `compose.yml -f compose.e2e.yml` alone, which registers no merchant anybody holds a private key for, so every spec that mints anything answered `invalid_client` — **`checkout.cy.ts` could not have passed under this recipe since Step 5c**, and that is recorded here rather than left as folklore. It now depends on `gen-demo-keys`, `build-sdk-node` and `build-checkout-browser`, adds `-f compose.demo.yml`, waits for `/healthz` and for the dashboard, the shop and the checkout page by name, runs BOTH Cypress passes and tears the stack down with `down -v`. CI's `e2e` job was fixed the same way, and one `.gitignore` line with it. Green from nothing in the `vpay-ci` VM on 2026-09-04: **11 tests across four specs, 0 failing, 0 skipped, exit 0** — `checkout.cy.ts` 13 s (1), `dashboard.cy.ts` 0.4 s (3), `shop-hosted.cy.ts` 1 m 18 s (3) in pass 1; `shop-embedded.cy.ts` 13 s (4) in pass 2 (`VPAY_E2E_FRAMED=1`). **Run green once, from nothing, in the VM** — not three times, and nothing here was measured for flakiness |

---

## Infrastructure

| Area | Status | Notes |
|---|---|---|
| `compose.yml` (Postgres + 2 WireMock rails) | ✅ | Written; **never started as a stack on any authoring machine** (Docker Hub unreachable from one; the rootless daemon on another cannot start containers at all). **Started for the first time by the CI `e2e (compose)` job on run `33647189156` (2026-09-02)**, with both WireMock rails and Postgres up and `vpay-server` answering `/healthz` 200 against them — see "GitHub Actions" below. `config/application.yml` used to point both rails at a host named `wiremock`, which no compose file defines; fixed 2026-09-02 to `wiremock-mtn` / `wiremock-orange` (with Orange's `/orange-money-webpay/dev` path prefix, per its flow doc), proven by `vpay-config`'s own test that loads the real file. **Changed 2026-09-03 (Step 3): the mappings these two services bind-mount are now the ones the conformance suite drives.** `backends/tests/conformance/wiremock/{mtn,orange}/mappings/` gained the full failure vocabulary per rail, a WireMock *scenario* (pending → successful), a redirect mapping (`REF_REDIRECT`) and an oversize-body mapping (`REF_HUGE`); a mapping fixed for the suite is fixed for `just up` and for `just demo` in the same edit, because it is one directory. The suite does **not** use this compose stack — it starts its own `wiremock/wiremock` containers via testcontainers, so CI's `rust` job needs no compose services |
| `compose.e2e.yml` (full stack) | ✅ | **Could not have booted before 2026-09-02**: both binaries exit 78 without `--config`/`VPAY_CONFIG` (mandatory since 2026-08-11), and this file never set it. Now sets `VPAY_CONFIG` and the rail `${VAR}` placeholders (stub values for stub rails) on both services — **six of them as of 2026-09-03 (Step 3)**, up from three: `MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `MTN_API_USER`, `ORANGE_MERCHANT_KEY`, `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET`, all present on **both** `vpay-server` and `vpay-worker` (miss one on either and the process exits `78` at boot — an unresolved placeholder is fatal by design, never an empty string). All six are listed in `.env.example`. **Run for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02)**: `vpay-server` answered `/healthz` 200 one second after the stack came up, the dashboard answered 200 on `/` a second later, and Cypress passed — see below. **Changed after that run, and therefore not covered by it (Step 1, same day):** `vpay-server` now exits `78` without an RS256 signing key, so this file sets `VPAY_OAUTH_SIGNING_KEY_FILE=/secrets/oauth-signing-key.pem` and read-only bind-mounts `.e2e/oauth-signing-key.pem` (git-ignored, `0644` because the scratch image runs as UID 65532, generated per stack by `just gen-e2e-signing-key` and thrown away with it; CI runs that recipe before `docker compose up`). **No CI run has yet booted the stack with that mount** — the ✅ above is evidence for the previous shape of this file, and the next `e2e (compose)` run is what proves the new one. **Step 3 added the three new rail variables to this file and to nothing else that runs it**, so the same caveat now covers them: they are correct by inspection and by `docker compose config`, and no run has booted the stack since |
| `backends/Dockerfile` (musl → scratch) | ✅ | **Changed 2026-09-06: `COPY schemas ./schemas` added to the `planner` and `builder` stages, and without it this image no longer builds at all.** `vpay-db` compiles `schemas/vpay.cstack` through `include_server_schema!`, whose path resolves against `CARGO_MANIFEST_DIR` and so climbs two levels out of `backends/`; the context copied `.xtask`, `backends`, `sdks/rust` and `examples/merchant-demo` but not `schemas/`, which had only ever been documentation. The first build attempt after the CrateStack read landed died with `error: failed to read schema file /build/backends/crates/vpay-db/../../../schemas/vpay.cstack: No such file or directory` — **a fourth reason this Dockerfile could not have built, and the first one no gate in this repository can catch**, since `just ci` compiles on the host where the file is present. Found only because the image was actually built. Both stages carry the line because the file's own header rule 3 requires their copy lists to stay identical; the cargo-chef cook needs it in neither, because chef stubs workspace members and the macro never expands there. After the fix: `--target server` builds, `docker run --rm … --version` prints `vpay-server 0.1.0`, image **16.9 MB** against master `6978901`'s **16.1 MB** rebuilt on the same host and builder the same day — **+0.8 MB (+5.0%)** for CrateStack's twelve crates. Last rewritten 2026-08-09 (musl host target, UID 65532). **Could not have produced a bootable image before 2026-09-02**: it never copied `config/` in, so there was no file for `VPAY_CONFIG` to name. Now bakes `config/` at `/config` and sets `ENV VPAY_CONFIG=/config/application.yml` in both runtime stages (secrets stay `${VAR}`, so the layer holds none). **A second reason it could never have built, found in review the same day:** the builder stage copied every workspace member except `sdks/rust`, and cargo refuses to load a workspace whose `members` list names a missing directory — proven by reconstructing the build context outside Docker and running the Dockerfile's own `cargo build`, which failed at manifest load; `COPY sdks/rust` added. **A third, found by the first CI build that reached the Docker step (run `33646048616`, the fix's own PR):** on the alpine builder the host triple *is* `x86_64-unknown-linux-musl`, and with no `--target` cargo applied `.cargo/config.toml`'s `+crt-static` rustflags to proc-macros too, which cannot be static (`cannot produce proc-macro for async-trait`). The build now passes `--target` set to the builder's own host triple (still never a cross-compile) and copies from `target/<triple>/dist/`; the Dockerfile's header comment explains it. It was the last reason: the next run, `33647189156`, built both images and booted the stack. **Built for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02), and the resulting `scratch` image booted, found its baked config, connected to Postgres, ran the migrations and answered `/healthz` 200.** **Built on an authoring machine for the first time 2026-09-03 (Step 6, block A) by `just release-dry-run`** — both targets, `linux/amd64`, exit 0; the `dist` build took 1 m 55 s and the two `scratch` images are 14.8 MB and 10.7 MB. Built, not booted: no local run has started a container from either. **Changed 2026-09-05: dependency compilation is now a cached layer (cargo-chef `0.1.78`, pinned exactly, `--locked`).** Four stages — `chef` (then `rust:1.95.0-alpine3.22`, **`rust:1.98.0-alpine3.22` since the toolchain bump of 2026-09-05 — every timing in this row was measured on the 1.95.0 base and none has been re-measured on 1.98.0**; `cargo install cargo-chef`, 33 s), `planner` (`cargo chef prepare` → `recipe.json`; manifests and the lockfile, no source, 0.1 s), `builder` cook (`cargo chef cook --profile dist --target <host triple> -p vpay-server -p vpay-worker-bin` — **the same flags as the real build**, with `.cargo/` copied in first so `+crt-static` applies), then the unchanged `cargo build`. `ARG`/`ENV VPAY_GIT_SHA` moved from the first instruction of the builder stage to **after the cook**, because `release.yml` passes a different `github.sha` on every push and an `ARG` above the cook throws the dependency layer away on every release build. `vpay-core/build.rs`'s `cargo::rerun-if-env-changed=VPAY_GIT_SHA` is untouched and still fires. **Measured on the authoring host 2026-09-05, `linux/amd64`, on a dedicated `docker-container` builder** (not the shared default one, which another build was using): cold 254 s → 238 s (**retracted by the 2026-09-05 review pass: that pair was two unpaired samples. Two matched cold pairs — the same isolated builder pruned between the two runs of each pair, runs back to back, the second pair in reverse order — give one-stage 193 s / cargo-chef 256 s and cargo-chef 248 s / one-stage 212 s, i.e. the cold path is 36-63 s SLOWER, which is the `cargo install cargo-chef` plus the cook's own pass. The warm numbers reproduced: source touch 105 s and 114 s, sha-only 101 s and 112 s, `ARG` moved above the cook 215 s, a `docs/` edit 1 s**); **one comment line added to `vpay-server/src/main.rs` 260 s → 125 s**, with the cook step logging `CACHED`; a `--build-arg VPAY_GIT_SHA` change alone **116 s**, cook `CACHED`, and the sha verifiably in the binary (`strings … | grep -c` → 1, versus 0 without the arg). **The `ARG` placement was proven load-bearing by mutation, not asserted:** moving it back above the cook makes the same sha-only build recompile the graph — cook not `CACHED`, 251 s. Runtime images unchanged: `vpay-server` 15.9 MB and two layers before and after, the `config/` layer the same digest in both, `docker export | grep -i 'cargo\|chef\|rust'` empty, and both `scratch` images now **run** (`--version` → `vpay-server 0.1.0` / `vpay-worker-bin 0.1.0`, exit 0) — the first time a container has been started from either on an authoring host. **The saving is ~2x, not the ~10x cargo-chef is usually sold on, and the reason is in the profile:** `[profile.dist]` inherits `release`'s `lto = "fat"` / `codegen-units = 1`, and a fat-LTO link re-consumes every dependency's LLVM IR whatever the cache holds. Single sample per number, one host, amd64 only — see [plans/exp8-notes/opus.md](plans/exp8-notes/opus.md) and, for the sabotage review that re-measured all of it and the six mutations it ran, [plans/exp8-notes/opus-review.md](plans/exp8-notes/opus-review.md). **The review also found the `--profile`/`--target` failure modes differ** — a cook missing `--target` dies in a second on the proc-macro/`+crt-static` conflict, a cook missing `--profile dist` succeeds silently and caches nothing (305 s against 105 s) — and that `[profile.dist]`'s inherited `lto = "fat"`, the thing that bounds the saving, is recorded in no ADR and no comment, so overriding it with `"thin"` is an open maintainer decision |
| `frontends/Dockerfile` | ✅ | Last rewritten 2026-08-09. **Never built anywhere yet**: not on an authoring machine, and CI's `e2e (compose)` job, which will build it, has never reached its Docker step. **Built for the first time by CI's `e2e (compose)` job on run `33647189156` (2026-09-02); the standalone Next server answered 200 on `/`.** Its build context had been checked beforehand the same way as the backend one — `pnpm install --frozen-lockfile --filter @vpay/dashboard...` against a reconstruction of exactly what it copies passes the lockfile consistency check with `examples/` and `sdks/nodejs` absent — see below. **Built on an authoring machine for the first time 2026-09-03 (Step 6, block A)** by `just release-dry-run`, `linux/amd64`, exit 0, 339 MB; built, not booted. **Changed 2026-09-04 (Step 9, lane 4): it now builds TWO images.** A shared `base` stage, then `dashboard-builder` → `runner` (unchanged output, name kept so `release.yml`'s matrix entry did not move) and `checkout-builder` → `checkout`. The checkout builder copies `sdks/` as well as `frontends/`, which is load-bearing: `@vpay/checkout` depends on `@vaam-apps/vpay-stripe-js`, whose `exports` resolve to a gitignored `dist/`, so without the SDK's source in the context `next build` fails with `TS2307`. **Every consumer now names its target** (`release.yml`, and `compose.e2e.yml`'s `dashboard` and `vpay-checkout`) — "the last stage wins" stopped being a safe way to say which image you meant. **Both targets built on the authoring host on 2026-09-04 from a clean `git archive` context**, and the `checkout` one was then RUN: `--read-only --tmpfs /tmp`, `uid=1000(node)`, healthcheck healthy, `GET /healthz` 200 carrying `no-store` / `frame-ancestors 'none'` / `no-referrer` / `nosniff`, and `touch /app/nope` refused with "Read-only file system". That is the first time anything in this repository has observed a Next.js image under the constraints the chart asks of it |
| `deny.toml` | ✅ | `cargo deny check` passes clean: `advisories ok, bans ok, licenses ok, sources ok`. The three advisories that failed before were fixed by **upgrading dependencies, not by suppressing them** — see below. One advisory is explicitly ignored: **RUSTSEC-2023-0071** (Marvin Attack in `rsa`, no patched release, an unconditional dependency of `authkestra-engine` per [ADR-0009](adr/0009-dashboard-oidc-provider.md)), accepted deliberately with the reasoning recorded inline in `deny.toml`. **This entry was preemptive when added and now genuinely fires — and this pass found that the previous pass's own note on *how* it fires was already stale, before this note could even be written once.** The last pass said `authkestra-op`/`authkestra-engine` reached `rsa` only via `vpay-tests-integration`'s dev-dependencies, so "the exposure itself is still narrower than 'in production' ... no shipping binary pulls it in." **That is no longer true, independently re-run and confirmed for this update:** `vpay-db` added `authkestra-op` as a genuine, non-dev dependency this pass (for `SqlClientAssertionStore`, OP-2), and both `vpay-server` and `vpay-worker-bin` depend on `vpay-db`. `cargo tree -i rsa` now shows `rsa v0.9.10 ← authkestra-engine ← authkestra-op ← vpay-db ← vpay-api/vpay-server/vpay-worker-bin`, with no `(dev)` marker anywhere on that specific path (the pre-existing `vpay-tests-integration` dev-only path still exists too, unchanged, in parallel). `cargo deny -L info check advisories` still reports the same `note[advisory-ignored]`/`note[vulnerability]` pair it did before — nothing about the ignore mechanism changed, and `cargo deny check` still exits 0 with 0 errors, so this is **not a CI regression**. What changed is the honesty of this row's own claim about scope: `rsa`'s Marvin-Attack timing side-channel is now reachable from both shipping binaries' production dependency graph, not merely from a test-only crate, even though nothing in either binary calls into `rsa` yet (no shipping code path constructs anything from `authkestra-engine`/`authkestra-op` — see "Merchant auth"/"Dashboard auth" above). The original `deny.toml` comment's own reasoning for accepting the advisory (no patched release exists; RS256 has no alternative in this stack; `/dash/v1` is staff-only, not the merchant payment path) does not depend on which dependency edge is dev-only, so the acceptance itself still stands — only the "no shipping binary pulls it in" line needs correcting, which this row now does. Also bans `aws-lc-rs`/`aws-lc-sys` so a second rustls crypto provider cannot reappear. **New this pass:** `CDLA-Permissive-2.0` was added to the allow list, with its justification recorded inline — it covers `webpki-roots` (Mozilla's CA bundle, data not code), pulled in through `sqlx`'s `tls-rustls-ring` feature now that `vpay-db` is a non-dev dependency using it (root `Cargo.toml`'s own comment: previously latent in the workspace's pins, now actually reachable). `tls-rustls-ring` (vendored roots) was chosen deliberately over `tls-rustls-ring-native-roots`: the runtime image is `FROM scratch` ([ADR-0004](adr/0004-musl-mimalloc.md)) with no OS trust store for `rustls-native-certs` to read, so native roots would fail TLS to Postgres in the shipped image only, while passing locally and in CI where a trust store exists — exactly the kind of gap that would not be caught until a real deployment. `rustls-native-certs` does still appear in the dependency graph (via `bollard → testcontainers → vpay-testkit`), but only as a `[dev-dependencies]` chain — `cargo tree -i rustls-native-certs` shows every path terminating in a dev-dependency of `vpay-testkit`/`vpay-db`/`vpay-tests-integration`, never a shipping binary, independently confirmed for this update **Updated 2026-09-05:** Dependabot alert 19 (`serde_with` < 3.21.0, KeyValueMap panics on empty entries, medium) was closed by a lockfile bump 3.17.0 → 3.21.0. `serde_with` reaches this workspace only through `testcontainers` via `vpay-testkit`, so no shipping binary compiled it; `cargo deny` was already green because the RustSec database had not yet carried the advisory. Verified: `cargo check --workspace --all-targets`, `postgres_smoke` 15/15 on a real container. |
| GitHub Actions | ✅ | **Correcting this row, which said "never executed": by 2026-09-02 the `ci` workflow had run 13 times (2026-08-09 → 2026-09-02, every one on a pull request) and failed all 13** (`gh run list --workflow ci`; per-job conclusions from `gh run view`). Job by job: `self-checks` passed 13/13; `rust` passed 10/13 (failed on `31317876404`, `31319267218`, `33618568372`) — on the latest run, `33626567174`, it ran `cargo nextest run --workspace` on `ubuntu-latest` with a working Docker daemon, container suites included, and reported `320 passed, 3 skipped`, which is the evidence for every container-backed row on this page; `supply chain` passed 11/13; `web` passed only the last 2 (the `pnpm -r test` Cypress-script bug fixed in the SDK pass); **`e2e (compose)` failed 13/13** — the first eleven at `pnpm/action-setup@v4` (the `packageManager` conflict `bf9811d` fixed), the last two at `pnpm exec cypress install`, because the workflow set `CYPRESS_INSTALL_BINARY: 0` for *all* jobs and then asked Cypress to install. The Docker steps after that never ran once. Two more defects: `on.push.branches` said `main` (the default branch is `master`, so nothing ever ran on a merge), and the `rust` job's flow-style `{ components: rustfmt, clippy }` parsed `clippy` as a stray key. **All fixed 2026-09-02**: the e2e job downloads Cypress normally and verifies it, builds both images, polls `/healthz` for a 200 and the dashboard for a 200 before running the spec; the compiler version is read from `rust-toolchain.toml` (now pinned to `1.95.0`, matching `backends/Dockerfile`) in every Rust job of `ci.yml` and `docs.yml`; and a new `just verify-ignored` step fails the `rust` job if the ignored-test count is not exactly 3, the number of test binaries is not exactly 30 (the check that actually catches a binary dropping out — 18 of the 30 hold eight tests or fewer), or the suite shrinks below 320 tests. **`expected_suites` moved 30 → 32 on 2026-09-02 (Step 1)**, for the two new `vpay-tests-integration` binaries `client_store` and `merchant_token_flow`; the raised value has not yet been exercised by a CI run. **Run 14 of the workflow — `33647189156`, on this fix's own pull request (#14) — is the first green `ci` run in this repository's history**: all five jobs passed; the `rust` job reported `329 tests run: 329 passed, 3 skipped` and `verify-ignored: 3 ignored (expected 3), 30 test binaries (expected 30), 332 total`; the `e2e (compose)` job built both images (5 min 21 s), got `/healthz answered 200 after 1s` and `dashboard: / answered 200 after 2s`, and Cypress ran the one spec (`dashboard.cy.ts`, 3 passing). The run before it, `33646048616`, was the first ever to reach the Docker step and failed there — see "Docker / compose" below for the proc-macro finding it produced. **✅ as of run `33650294682` (2026-09-02), the first push-triggered run on `master` in the repository's history, triggered by the merge of #14 and green on all five jobs.** The claim this row makes — the workflow runs on the default branch and on pull requests, builds the images, boots the stack and runs every suite — would fail visibly if it broke, which is the bar for ✅ here . **Nothing changed here in Step 3 (2026-09-03): no workflow file was touched, and no CI run exists for the `claude/step3-rails` branch.** **Changed 2026-09-03 (Step 5b), and one of the changes costs coverage.** The `web` job now runs `pnpm --filter @vaam-apps/vpay-sdk build` *before* `pnpm -r typecheck` (`sdks/stripe-compat` imports `@vaam-apps/vpay-sdk/stripe`, whose types resolve to the gitignored `dist/`; in the old order a clean checkout failed with `TS2307` — reproduced locally by `rm -rf sdks/nodejs/dist && pnpm -r typecheck`), and the `e2e (compose)` job gains a Rust toolchain, `just gen-demo-keys`, `-f compose.demo.yml` and a `pnpm --filter @vaam-apps/vpay-stripe-compat compat` step (with `VPAY_RECEIVER_URL` pointing at the `wiremock-webhook` container the base overlay already publishes on 8083, which the `constructEvent` case reads the delivery out of). Master's own `e2e` job did **not** already do any of this — checked on the rebase, and there are no duplicated steps. **`config/application-sandbox.yml` is now exercised by nothing.** compose.e2e.yml sets `VPAY_PROFILE: sandbox`; compose.demo.yml overrides it to `demo`, and `vpay-config` loads one overlay — the one the profile names — so the e2e stack loads `application-demo.yml` *instead of* the sandbox overlay, even though `sandbox` is the CLI's default profile. That is inert **today** and only today: the sandbox overlay sets exactly two things the demo overlay does not, `deployment.name` (cosmetic) and `dashboard_client.redirect_uris` (the OIDC callback at `http://localhost:3000/dash/v1/callback`), and the one Cypress spec asserts the dashboard's scaffold notice without ever starting a login, so no redirect URI is read under either profile. The day a spec signs in, this job stops covering the profile it appears to cover. Deliberately *not* fixed by editing compose: the alternative is a third overlay used only by CI, which is one more file to keep in sync, and the choice belongs to a maintainer rather than to this pass. **No CI run exists for the `claude/step5b-stripe-sdk` branch either**, so every count in this row's Step 5b sentences was measured on the authoring machine — including the post-rebase ones: `886 tests run: 886 passed, 0 skipped` with `0 ignored (expected 0), 38 test binaries (expected 38)`, and 25 stripe-compat cases against a real stack. `min_tests` moved 840 → 870 on that measurement; `expected_suites` stayed 38, because Step 5b added no Rust test *binary* — its Rust tests land in files that already existed and its new suite is TypeScript, which cargo does not run. The conformance suite starts its WireMock containers with testcontainers precisely so the `rust` job needs no new services; whether that works on GitHub's runners is unproven, and the counts in this pass's header were all measured on the authoring machine **Changed 2026-09-03 (frontend dependency audit): the `web` job gained a `just audit-web` step, placed immediately after `pnpm install --frozen-lockfile` and before every build step, plus a SHA-pinned `taiki-e/install-action@e67fa11c` (v2.87.4) to provide `just` — the same pin `deploy` uses, chosen over the `@v2` tag the `rust` and `e2e` jobs carry because this is the step that decides whether a known-vulnerable dependency can land. CI runs the recipe rather than a copy of its two commands, so the gate and the local check cannot drift. The `e2e` job deliberately does *not* audit: it installs from the same lockfile. `actionlint` over the edited file: clean.** **No CI run of this change exists yet** — the gate was exercised on the authoring machine instead, in both directions: green on this tree, and exit 1 with `3 high` on `master`'s own lockfile (restored with `git stash`, since `pnpm audit` resolves from `pnpm-lock.yaml` and not from `node_modules`). That second run is the evidence the step can actually fail; a gate only proved green is not a gate. **Updated 2026-09-03 (Step 7): `master` now has a recent green run, `33792230584`** (commit `ca94eac`, the merge of PR #24) — **six** jobs, not the five this row describes above, since `deploy (helm chart)` joined in Step 6: `rust`, `web`, `e2e (compose)`, `supply chain`, `self-checks (no-mocks, status)` and `deploy (helm chart)`, all successful. That run is the CI evidence the "Merchant auth", "Payment intents" and "Charge submission" rows above now cite. **It is a run of `master`, not of Step 7:** nothing on `claude/step7-cleanup` — including the `test-doc` step and the `verify-docs` report this step adds to this workflow — has been executed by CI even once. |
| `just audit-web` + CI `web` audit step | ✅ | New 2026-09-03. `cargo deny`'s counterpart for the JavaScript half of the repo, and the answer to 21 Dependabot alerts on `master`. **What is gated:** two `pnpm audit` runs at `--audit-level=high` — first `--prod` (the production graph alone: what `@vaam-apps/vpay-sdk`/`@vaam-apps/vpay-stripe-js` hand a merchant and what `frontends/Dockerfile` ships), then the whole workspace with dev dependencies included. Dev dependencies are gated on purpose: **all sixteen** advisories this pass cleared were in build/test tooling, so a `--prod`-only gate would have been green over every one of them. High and critical fail the recipe; moderate is reported by a bare `pnpm audit` but does not fail it — a ceiling chosen so that the weekly moderate in a transitive dev dependency does not train people to reach for the ignore list. **What is ignored: nothing.** `pnpm.auditConfig.ignoreCves` is absent from `package.json` and no advisory is suppressed; `pnpm audit` on this tree reports `No known vulnerabilities found` at every severity, down from 16 (1 critical, 5 high, 10 moderate) on `master` — and `--prod` alone was 3 high + 2 moderate there, all through Next's exactly-pinned `postcss@8.4.31` and `sharp@0.34.5`. Cleared by version bumps where a release existed (`vitest@2.1.9 → 3.2.7` in all seven packages that use it, clearing the one **critical**, GHSA-5xrq-8626-4rwp/CVE-2026-47429; `next@15.5.23 → 15.5.25`, whose optional `sharp` range widens to `^0.35.4` and so clears GHSA-f88m-g3jw-g9cj without an override; `cypress@13.17.0 → 15.21.1`, which drops `extract-zip` — GHSA-jmr9-qjv8-65gv has **no patched version at all**, so no 13.x or 14.x could have cleared it — and moves to `@cypress/request@4`, taking `qs` to 6.16.0 and dropping its `uuid` dependency entirely), and by two documented `pnpm.overrides` where no release existed — `next>postcss ^8.5.28` (Next pins 8.4.31 exactly on every 15.x and 16.x) and `@storybook/addon-actions>uuid ^11.1.1` (the bump landed in Storybook 9, a major migration of `frontends/packages/ui`) — plus a third, `vite ^6.4.3`, which is a **compatibility** pin and not an advisory fix: vite 7.3.6, what pnpm picks without it, is patched too, but it leaves `@storybook/{builder,react}-vite@8.6.18` (peer `^4\|\|^5\|\|^6`) on an unmet peer while vitest 3 wants `^5\|\|^6\|\|^7`, and pnpm hoists one copy. 6.4.3 is the only version in that intersection that is also patched, so it is a ceiling as much as a floor — if a 6.4.x advisory ever lands with the fix only in 7.x, this pin has to go and the Storybook 9 migration becomes urgent. Removing all three overrides and reinstalling was measured, not assumed: 5 advisories (2 high, 3 moderate) come straight back. Each override carries its advisory IDs and a removal condition in `package.json`'s `"//pnpm"` block. **Verified on the authoring machine, not in CI** (no run of this branch exists): `pnpm install --frozen-lockfile` clean, `just lint-web` clean across all fourteen TypeScript projects, `pnpm -r test` **248 passed** on vitest 3 (`sdks/nodejs` 150, `sdks/stripe-js` 88, `frontends/packages/tokens` 3, `frontends/packages/api-client` 4, `frontends/packages/ui` 3, `frontends/apps/dashboard` 0 `--passWithNoTests`), `pnpm --filter @vpay/ui build-storybook` and `pnpm --filter @vpay/dashboard build` both succeed on the pinned vite 6.4.3 / postcss 8.5.28, and `cypress install` + `cypress verify` succeed on 15.21.1 (Node engine `^20.1.0 \|\| ^22.0.0 \|\| >=24.0.0`; `.nvmrc` was 22.11.0 when this was measured and is **22.23.2** since 2026-09-05, which that range also satisfies). **248, not the 247 the Step 5c note above records** — that difference predates this pass and is not caused by it: `sdks/stripe-js/src/client.test.ts` gained `strips a long run of trailing slashes in linear time` after `b57e0ce`, the commit that note was measured on, and it is on `master` today. **What this row does not claim:** nothing here has run on a GitHub runner, and no Cypress *spec* was executed against a stack on this tree — only `cypress verify`. Whether Cypress 15 runs the two existing specs is unproven until the `e2e` job runs **Updated 2026-09-04: a registry outage is told apart from an advisory.** On 2026-09-04 the audit endpoint timed out or answered 503 for about two hours and failed seven consecutive CI runs of a branch whose lockfile had not changed (`ERR_SOCKET_TIMEOUT` in every log, a finding in none). Each `pnpm audit` now runs with a 180 s fetch timeout and up to four attempts 90 s apart, retried **only** when the output carries the registry's own failure signatures; an advisory fails at once with pnpm's report, and a registry that never answers fails with `REGISTRY UNREACHABLE — the audit did not run`. Neither passes: an unreachable audit is an audit that did not run. The retry loop is shell in the recipe, exercised by hand against the live endpoint on 2026-09-04 and by nothing else — there is no test for it, and CI's `web` job is the evidence that the happy path still gates. |
| `just test-doc` + CI `rust` job doctest step | ✅ | **New 2026-09-03 (Step 7, lane 4).** `cargo test --doc --workspace`, wired into `just test`, into `just ci` (between `test-rust` and `verify-ignored`), and into CI's `rust` job as its own step running the recipe rather than a copy of its command. **What it closed:** `cargo nextest` runs no doctests at all, and `just test-rust` and CI's `rust` job are both `cargo nextest run --workspace` — so from this repository's first commit until this change, **not one doctest had ever been compiled by CI**, and there was exactly one to miss (`vpay_core::money`). Measured on this tree with the pinned 1.95.0 toolchain: **77 passed, 1 ignored, 0 failed**, measured on the final Step 7 tree with all five lanes landed — up from the one doctest that existed before this step and that nothing had ever compiled. Per crate: `vpay-core` 42, `vpay-api` 10, `vpay-provider` 8, `vpay-worker` 5, `vpay-sdk` 3, `vpay-config` 3, `vpay-db` 2, `vpay-ledger` 2, both adapters 1 each. The single ignored one is a ```` ```rust,ignore ```` fence at `sdks/rust/README.md:254`, pulled into the crate's doctests by `#[doc = include_str!("../README.md")]` at `sdks/rust/src/lib.rs:106` — it is not a fence written in `src/lib.rs` itself; `vpay-api`'s went away when its example was made real. Two further examples compile but are never run: the ```` ```no_run ```` fences at `sdks/rust/src/lib.rs:20` and `README.md:82`/`:270`. **77 run, 1 ignored, 2 compile-only, all three in `sdks/rust`, outside every lane.** It is a second compile of the workspace under rustdoc, which is why it is its own step: a doctest failure is reported by the step whose job it is. What it does not prove: nothing about a rail, a container or a deployment — a doctest is a pure-function example, and the honesty rule the lanes work to is that an example needing `no_run` or `ignore` to compile is one that should be ```` ```text ```` instead |
| Shared rail token cache and bounded rail read (`vpay_provider::token`) | ✅ | **New 2026-09-03 (Step 7, lane 2).** Both adapters had the same `CachedToken` — same three fields, same redacting `Debug`, same length-prefixed SHA-256 credential fingerprint — and the same wrapper over `vpay_provider::http::bounded_body`; there is now one of each (`vpay_provider::token::CachedToken`, `http::read_rail_body`). **The one place the two rails genuinely differed is preserved and is the thing under test:** the refresh margin is the *caller's*, not the type's, and each rail passes its own 60 s (`the_refresh_margin_mtn_applies_is_sixty_seconds_of_the_rails_own_lifetime`, `the_expiry_margin_orange_applies_is_sixty_seconds_of_the_rails_own_lifetime`, `the_margin_is_the_callers_and_this_type_supplies_none`). **Stated because it is a real limit of those tests:** the two constants are numerically equal, so swapping them is caught by the shape of the API — the margin cannot be read from the shared type — and not by any assertion. The four lane-2 packages run **149 tests**; the conformance suite is still **26 cases**. **One behaviour change, and nothing asserts the difference:** a rail body that fails mid-stream now carries `RailFailure::Body` where it carried `Http`, and MTN's oversize/read messages have new text |
| Provider-port error surface (`ProviderAdapter`'s `# Errors`) | ✅ | **New 2026-09-03 (Step 7, lane 2).** All four trait methods carry an `# Errors` section, and the trait carries a table of which `ProviderError` variants each operation may raise — the natural place for it now that `Transport`/`Malformed` carry a typed `#[source]` (Phase A). Enforced going forward by `#![warn(clippy::missing_errors_doc)]` on the crate root: a per-crate `[lints]` table cannot override the workspace's, so the inner attribute is the only spelling that works. **10 doctests, 0 ignored**, on `vpay-provider` and the two adapters |
| `#[serde(rename_all = "snake_case")]` on vpay's own wire and config | ✅ | **New 2026-09-03 (Step 7, lane 3).** The attribute is now on **22** types across `vpay-api` (13) and `vpay-config` (9) that model vpay's own wire or config, plus `vpay_provider::Capabilities` — up from the base's 2, the lanes' 20. Every one is a **no-op today** — the fields are already snake_case — and that is the point: it is what keeps them snake_case when someone adds a `payTo`. **Deliberately exempt, and the exemption is written where the type is:** both adapters' `wire.rs` headers and `docs/reference/rails.md:42-60` explain the rule for `wire.rs` and each rail's `TokenResponse` alike (a rail's own casing — adding `rename_all` there would at best be masked by the per-field renames and at worst break two adapters and the conformance cases — 26 when this was written, 28 since Step 8 lane C); MTN's `token.rs` carries the same paragraph directly above `TokenResponse` so the claim is true in the file that matters, not only in the docs describing it. Also exempt: `resource_auth.rs`'s `RawClaims` (JWT claim names), the SDK's RFC 6749 / Stripe envelope types, and `Currency` (`UPPERCASE`). **Not enforced by a scanner** — the plan's `verify-serde` was not built (see the tooling rows); today this is a convention a reviewer checks |
| `vpay_api::boot` — the shared boot steps | ✅ | **New 2026-09-03 (Step 7, lane 3).** The boot sequence both binaries follow is named steps in one place instead of two copies inside two 300-line `fn run`s. **Both binaries are wired to it** — `vpay-server` (lane 3) and `vpay-worker-bin` (lane 5, commit `92c02b7`), five call sites each: `load_config`, `adapters_by_code`, `boot_seeds`, `open_migrated_database`, `reconcile_reference_tables`. That is what makes it a shared module rather than duplication with an extra indirection, and it is why this row is ✅ rather than 🟡. The tripwire is the binary's own suite: `cargo nextest run -p vpay-worker-bin` — **12 tests, 12 passed** — which exercises the CLI and boot refusals a shared module could quietly change. The worker binary's 330-line `fn run` is gone from `verify-docs`' long-function list; what replaced it is an 80-line `fn boot`, which is *exactly* the report's threshold — it will reappear on that list the moment anyone adds a line to it. `BootError` is its error type and delegates all five `Classify` methods to `DbError`, which is what took `verify-errors` from 13 classified types to **14** |
| `confirm_once` and `vpay-server::run` split into named steps | ✅ | **New 2026-09-03 (Step 7, lane 3).** `confirm_once` went from **243 lines to under 80** (six named steps) and `vpay-server`'s `fn run` from **371 to under 80**; `verify-docs` no longer lists either. **Two long functions were deliberately not split, and the reasons are in the code:** `vpay_config::validate_all` (138) — its refusal *order* is the behaviour, and splitting it would let the order drift silently; `persist_submitted` (89) — it is one commit point, and a commit point that spans two functions is one a future edit can straddle |
| `vpay_api::v1::paging` is `pub` | 🟡 | **Changed 2026-09-03 (Step 7, lane 3), and it is a public-surface change made for the docs.** The module was `pub(crate)`; its doctests cannot compile against a crate-private item, so it is now `pub`. A doctest that needed `no_run` or a hidden `# use` to compile would have been the alternative, and this step's rule is that an example nothing runs is a claim nothing checks. 🟡 because widening a surface to satisfy a test is a trade worth a maintainer's eye, not a settled convention |
| Doctests in `vpay-api`, `vpay-config`, `vpay-provider`, the adapters, `vpay-db` and `vpay-worker` | ✅ | **New 2026-09-03 (Step 7, lanes 2 and 3).** Lane 2: **10 passed, 0 ignored**. Lane 3: **13 passed, 0 ignored**, and `resource_auth.rs`'s ```` ```ignore ```` fence — the one the design named — is now ```` ```text ````, so the workspace's last un-run Rust example is gone from `backends/`. The remaining ignored doctest in the workspace is in `sdks/rust`, outside `verify-docs`' scope. Workspace total is in the `just test-doc` row |
| `vpay_core::error::display_with_chain` — one rendering for both durable error columns | ✅ | **New 2026-09-03 (Step 7, lane 5).** `jobs.last_error` and `webhook_deliveries.response_excerpt` are the two columns an operator reads after the fact, and they were rendered two different ways — the job loop walked the source chain, the webhook path stored a composite's `Display` alone and lost the leaf. Both now go through `display_with_chain`. **The bound lives with the column, not with the renderer:** neither call site truncates, because `vpay-db` clamps each write against the CHECK the migration declares (`LAST_ERROR_MAX_CHARS`, `bounded_excerpt`) — a caller that truncated would be a second, weaker copy of a constraint the database already enforces, and the first one to drift. `webhook_deliveries.response_excerpt` (**that column, CHECK ≤ 2000** — the Phase A note called it `last_error`, which is a different column on a different table; the plan is corrected) now reads `no response: {error}: {chain}` for a delivery that got no answer. **Operator-visible data only: no wire change**, no field added to any response, nothing a merchant can see |
| Doctests in `vpay-db` and `vpay-worker` | ✅ | **New 2026-09-03 (Step 7, lane 5).** `cargo test --doc -p vpay-db -p vpay-worker`: **7 passed, 0 ignored** — `poll_delay`, `delivery_delay`, `signature_header`, `recovery_step`, `RecoveryPolicy::default`, `TxOutcome` and `TxOutcome::into_inner`. `vpay-core` is **42** with `display_with_chain`'s own example. Every one is a pure function with no I/O, which is why they can be doctests at all: the parts of these two crates that matter most need Postgres and are covered by the container-backed suites instead, not by examples |
| Doc externalisation, `vpay-db` + `vpay-worker` + `vpay-worker-bin` | ✅ | **New 2026-09-03 (Step 7, lane 5).** Prose **5 498 → 4 819 lines**, prose-to-code **95.8% → 84.3%** across the three, with the reasoning moved to [reference/vpay-db.md](reference/vpay-db.md) and [reference/vpay-worker.md](reference/vpay-worker.md) rather than deleted. These two crates carried 36% of the workspace's doc volume at the step's scoping and had no owner in the original four-lane split; a fifth lane is what closed that (refresh decision (11)) |
| Long functions in `vpay-worker` and `vpay-worker-bin` | ✅ | **New 2026-09-03 (Step 7, lane 5).** `poll_charge` **210 → 93**, `resubmit_charge` **106 → 34**, `handle_deliver` **167 → 40**, `run_loop` **173 → 77**, and the worker binary's `run` **330 → 76** (plus `boot`, exactly 80). **What was refused, and why it is the right refusal:** taking `poll_charge` below 80 would have meant a fifth `#[expect(clippy::too_many_arguments)]`, i.e. trading a number in an advisory report for a real lint suppression in a payment path. It stayed at 93. `vpay_db::config_reconcile::reconcile` (105) is untouched — it belonged to no lane and nobody pretended otherwise. `vpay_adapter_orange_money::mint_token` (102) is untouched for the same kind of reason, not the same one: it is one HTTP exchange with its own decode and margin steps, Orange's token mint is not something splitting it would clarify, and it was not lane 2's scope either way |
| Doctests in `vpay-core` and `vpay-ledger` | ✅ | **New 2026-09-03 (Step 7, lane 1).** **43 compiled doctests** (`cargo test --doc -p vpay-core -p vpay-ledger`), **zero** of them ```` ```ignore ```` or ```` ```no_run ```` — every one is compiled and run, which is the only version of "the docs cannot lie" worth writing down. They cover `ids`, `money` (both provider encodings), both state machines, `settlement`, `failure` and the `Classify` tiers. Before this pass the whole workspace had **one** doctest and nothing ran it. The workspace-wide figure is in the `just test-doc` row above; this row is the two crates lane 1 owned |
| `docs/reference/<crate>.md` — the narrative tier | 🟡 | **New 2026-09-03 (Step 7).** A third documentation tier beside `docs/adr/` (a decision) and `docs/flows/` (a process): why a particular piece of code is shaped the way it is. [reference/README.md](reference/README.md) indexes it and names the crates that have no page. **🟡 because six pages cover eight crates of twelve** — `vpay-api`, `vpay-config`, `vpay-core`, `vpay-db`, `vpay-worker`, and `rails.md` for `vpay-provider` plus both adapters (one page, because the argument for the port and the argument for an adapter are the same argument from two ends). `vpay-ledger`, `vpay-testkit` and both binaries have none, and their reasoning is still in the source's module headers — the binaries' boot sequence is in `vpay-config.md`, where the shared code now lives. The measured effect on volume, by the *old* metric that counted a compiled example as a comment: `vpay-core` 128.8% → 160.2%, `vpay-ledger` 11.6% → 107.8% — both *worse*, because the prose those crates lost was replaced by more doctest lines than it had. That is the exact reason `verify-docs` now reports prose and example lines in separate columns and takes its ratio on prose alone; scored the old way, doing what this step asked for reads as a regression |
| `cargo xtask verify-docs` — a report, never a gate | ✅ | **New 2026-09-03 (Step 7, lane 4), and ✅ for being a report, not for anything it enforces.** Per crate: doc-comment lines against code lines; every production function of 80 lines or more (a brace-depth scan that blanks string literals — a naive one miscounts on a `format!` body containing `{`); every ```` ```ignore ```` doctest fence; every `#[allow]`/`#[expect]` in production code. It **exits 0 whatever it finds** — Step 7's decision (4), because the cheapest way to pass a doc-ratio gate is to delete the `# Errors` sections [ADR-0011](adr/0011-error-modelling.md) depends on. `just verify` runs it after the three gates and says so in its own success line; CI's `self-checks` job runs it too, so the justfile's claim that the job runs exactly `just verify` stays true. Numbers on this tree: **prose 13 048 / compiled-example 1 111 / code 12 938 across the twelve crates — **100.8%** prose-to-code, measured on the final tree. The comparable pair is the *old* convention (every doc line counted as prose, looser denominator), where the design's pre-step baseline was **97.1%** and this tree reads **88.6%** — the full table is in the header above, including the four crates still over 100%. Six production functions of 80 lines or more, down from the eleven Phase A measured; the longest is now `vpay_config::validate_all` at 138, and nothing over 200 remains. **Zero** ```` ```ignore ```` fences in the two trees it scans. Four `#[allow]`/`#[expect]`, exactly the ones Phase A already named — none was added to make a function shorter, which was the live temptation in `poll_charge`**. **Scope, and the limits that follow from it:** `backends/crates` + `backends/apps`, `src/` only, everything from a file's first `#[cfg(test)]` onward excluded — so `sdks/rust` and `.xtask` are outside every number, and its ```` ```ignore ```` list therefore does **not** include the one in `sdks/rust/src/lib.rs` that `cargo test --doc` reports as ignored. 13 unit tests over synthetic sources back it, including one for the defect it shipped with for a single run: three files *discuss* `#[cfg(test)]` in their module headers, a scan over raw text stopped at the sentence, and two 200-line functions and two `#[expect]`s vanished from the report. A report that measures nothing looks exactly like a clean one, which is why it also shouts when it finds no sources at all |
| Local demo (`just demo`, `examples/merchant-demo`, `compose.demo.yml`) | 🟡 | New 2026-09-02. `just demo` generates a throwaway server signing key and a demo merchant keypair (`just gen-demo-keys`: `cargo xtask gen-signing-key` for the merchant, its public JWK written into a git-ignored `demo` profile overlay `.e2e/application-demo.yml` that `compose.demo.yml` bind-mounts beside the baked base config), brings up `compose.yml` + `compose.e2e.yml` + `compose.demo.yml`, waits for `/healthz`, and runs `cargo run -p merchant-demo` — a Rust binary using `vpay-sdk` that prints one line per step: discovery and JWKS, an access token's decoded claims (never the token), the 401 envelope without a bearer, and the authenticated 404 `unknown_route` for `payment_intents().retrieve(..)` with the sentence "payment intents are not built yet — this is where the next step lands". `compose.demo.yml` publishes no host port for Postgres (`ports: !reset []`) because 5432 is the most commonly occupied port on a developer machine and the demo never reaches Postgres from the host. **🟡, not ✅:** the demo is an assertion harness a human reads, not a test CI runs — nothing fails a build if it regresses; and it demonstrates authentication only, because no `/v1` resource exists yet. Its first run found the runtime-image panic recorded in the "Resource-server JWT validation" row, which is the kind of thing it exists to find. **Updated 2026-09-03 (Step 2): four steps became five, and the fifth is the honest one.** The demo now runs discovery + JWKS, a token, the unauthenticated `401`, then **`payment_intents().create(…)` followed by `.retrieve(…)` through the shipping Rust SDK** — a real write to a real database, asserting the retrieve returns the object the create did — and finally **`.confirm(…)`, whose success condition is a `501 not_implemented`**. A printed payment intent that had been *confirmed* would mean something fabricated one. `just demo` is also port-configurable now: `just demo_port=18080 demo` propagates one number to the three places that must agree — the published port, the demo overlay's `deployment.public_base_url` (which becomes the OP's `issuer`, and a mismatch is an `invalid_client` whose message names no port), and `VPAY_BASE_URL` for the demo binary — and `gen-demo-keys` regenerates the overlay when either that URL or the newly-required `merchant_id` field is missing from it, a check added after a measured failure in which `just demo` spent its whole 120 s readiness budget on a crash loop while the recipe reported it had kept a file that no longer loads. **Updated 2026-09-03 (Step 3): still five steps, and step 5 now *succeeds*.** `payment_intents().confirm(…)` reaches the compose stack's WireMock MTN rail over HTTP and the demo asserts the intent came back **`processing`** with `next_action: null` — a push rail's one success state — then re-reads it and asserts the confirm's response and the later retrieve are the same object, so a status the handler rendered but did not commit would fail the run. It also asserts the *opposite* of what it used to: a confirm that does **not** reach `processing` is now the failure. The demo intent is **EUR**, because `config/application.yml` puts `mtn_momo` on EUR (MTN's sandbox rejects XAF) and `/v1` refuses a confirm whose intent currency is not the rail's — a property of the profile, expressed as config, never a code branch. **Updated again 2026-09-03 (Step 4): five steps became six, and the sixth ends in `succeeded`.** Step 6 polls `payment_intents().retrieve(…)` — exactly as a merchant integration would, through the SDK and nothing else — until the intent leaves `processing`, and asserts it arrived at **`succeeded`**. Nothing in the demo fakes an approval: the `vpay-worker` container (`compose.demo.yml`) claims the poll job the confirm committed, asks the WireMock MTN rail over HTTP, and the stub answers `PENDING` on the first query and `SUCCESSFUL` on the second because the confirm's documentation MSISDN enters a scenario keyed on that number. The first rung of the ladder is 10 s, so the step normally takes ~10–15 s. What step 6 deliberately **cannot** show is `amount_received`: the settlement writes it, the `payment_intent` object does not carry it, and printing it would mean reading the database behind the API the demo exists to demonstrate — the demo says so itself rather than omitting it. **Still 🟡, and the reason changed on 2026-09-03.** It is no longer "nobody has run it". **It was observed end to end on 2026-09-03 during Step 5b's rebase verification** (`just demo_port=18080 demo`): all **seven** steps passed — six became seven with Step 5, which added the webhook step this row's body above does not yet describe: discovery and JWKS, a token, the unauthenticated `401`, create + retrieve, a confirm that reached `processing`, a poll that reached `succeeded` after 7 retrieves, and step 7, the receiver's journal showing a delivery whose `Stripe-Signature` was byte-identical to its `Vpay-Signature` and verified with `vpay-sdk`. The row stays 🟡 because that run is a human reading output, not a build gate: nothing fails CI if the demo regresses, and the step that would catch it there is `sdks/stripe-compat` rather than this. **Updated 2026-09-04 (Step 8, lane A): six steps became four, and the fourth is six payments on both rails.** The walkthrough is now a table — MTN push to `succeeded`, to `insufficient_funds` (payer decline) and to `payer_timeout` (the prompt expired); Orange redirect to `succeeded` with the `next_action.redirect_to_url` printed, to `payer_timeout` (the hosted page expired) and to `provider_error` (the rail refused and documents no reason) — and each one prints the intent's public fields, asserts the exact `last_payment_error.code`, and verifies the `Vpay-Signature` of the webhook that settlement produced, read out of the receiver's own request journal. **Every outcome is selected at the rail stub by a field a merchant controls** — the MSISDN on MTN (documentation numbers `237600000f01`/`237600000f02`, carried to the status query by WireMock scenario, since MTN's status query is a `GET` that steers no other way) and the amount on Orange (5001/5002, which travel on its `POST` status body) — never by rewriting stored state. `just demo` is now `demo-up` + `demo-walk`, both of which exist separately, alongside `demo-status` and `demo-down`; readiness is `docker compose up --wait` on healthchecks (both rail stubs gained one) plus an external `/healthz` poll for the two `FROM scratch` services that cannot carry one. `compose.demo.yml`'s `name:` reads `${VPAY_DEMO_PROJECT:-vpay-demo}` and three `just` variables (`demo_project`, `demo_port`, `demo_receiver_port`) let two stacks run at once — **proven by running both**, two networks, two volumes, two databases, and the second stack's walkthrough green while the first was up. `docs/runbooks/demo.md` is the procedure, with the output of a real run pasted rather than narrated. **Still 🟡, and the reason changed twice on 2026-09-04.** Lane A first recorded that `just demo` from nothing had **never been observed green**: six walkthrough attempts gave two greens and four `500`s on a confirm, every one of them the `write_matched_no_row` race between `vpay-api`'s confirm and `vpay-worker`'s immediately-runnable poll job — **a defect in vpay, not in the demo** (`docs/runbooks/demo.md` §9, `docs/plans/step8-notes/lane-a.md` §3). **Lane G fixed that defect the same day** (see the confirm/worker race row). ~~Lane A's rebased branch — carrying the fix — then ran the walkthrough from nothing green, six outcomes for six with zero `write_matched_no_row`, and that is the measurement this row rests on.~~ **Corrected 2026-09-04, and the correction matters because it removes this row's only end-to-end evidence for the fix:** one green run from nothing exists (lane A's rebased branch, 2026-09-04, **without** lane G — that branch was rebased onto `068d8b7`, master plus lanes B and D, and lane G merged later as `53f7a7e`; the race is timing-dependent and did not fire, so this is one green *pre-fix* run and not evidence for the fix), lane A's own earlier count was two greens in six attempts and zero for three from nothing, lane G did not re-run the demo. Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in): `just demo` from nothing six times, **four green** (six outcomes for six each, exit 0); the two failures were the VM's Postgres answering single statements in 14–36 s under host I/O pressure, with the settlement and the webhook both landing in the worker's log after the demo's budgets; `write_matched_no_row` appeared in no run. Three from nothing is met in count, not consecutively. What that one run does establish is the walkthrough itself: exit 0, six outcomes for six, `grep -c write_matched_no_row` = 0 over both logs, on a quiet machine. The row therefore stays 🟡 for three reasons and not one: the demo is a human reading output rather than a build gate, both rails and the receiver are WireMock hosts, and the merged branch's own demo run does not exist yet. **Updated 2026-09-04 (Step 9): four steps became five, the currency became XAF on both rails, the stack became eight services, and the merged branch's demo run now exists.** Step 5 creates one hosted and one embedded Checkout Session, each on its own fresh intent, reads each back, prints the hosted `url` in full and redacts the embedded secret — and stops there: it opens no browser and pays neither. `just demo-up` now also starts `vpay-checkout` (vpay's own payment page) and `vpay-shop` (the demo merchant's storefront), and `demo_checkout_port`/`demo_shop_port` join the four existing variables. **Run from nothing three consecutive times, green, in the `vpay-ci` VM on 2026-09-04 on the merged Step 9 branch** (`551ec80`): six outcomes for six each, XAF on both rails, both sessions created and read back, exit 0, `write_matched_no_row` in no run's logs. Step 8's bar of three from nothing is therefore met **and consecutive**, which it was not before. The row stays 🟡 for the two reasons that survive: nothing in CI fails if the demo regresses, and every rail and receiver in it is a WireMock host |
| Two demos on one machine | 🟡 | **New 2026-09-04 (Step 8, lane A).** Compose-layer isolation is done and proven (`${VPAY_DEMO_PROJECT}`, `demo_project`/`demo_port`/`demo_receiver_port`; two projects, networks, volumes and databases observed side by side, the second stack's walkthrough green while the first was up). **`.e2e/` is not isolated:** one merchant key pair and one profile overlay serve the whole checkout, so a second `demo-up` on a different `demo_port` regenerates the shared pair and the first stack's `demo-walk` then fails `invalid_client`. Sequential use is fine; interleaved `demo-up` is not. Fixing it means keying `.e2e/` on `demo_project`, which touches `.github/workflows/ci.yml` (the e2e job's signing-key path), `just stripe-compat`, `examples/merchant-stripe-node` and `sdks/stripe-compat` — whose failure mode here is a *silent* `invalid_client`. **Not fixed, and named as a gap rather than left to be discovered** — see `docs/plans/step8-notes/lane-a.md` §4 |
| `schemas/*.cstack` | 🟡 | ~~**Syntax verified against real CrateStack 0.10.1** (and 0.7.10 / 0.7.8 before it)~~ **Corrected 2026-09-05: that was a hand-run claim, and it is now a gate.** `just check-schema` runs `cratestack check --schema schemas/vpay.cstack` inside `just verify`, `just ci` and CI's `self-checks` job, against **cratestack-cli 0.12.0** (0.11.1 until 2026-09-07) — pinned once, as `cratestack_version` in `justfile`, which the workflow reads back rather than repeating. The file passes at 0.12.0 unchanged: **no edit to `schemas/vpay.cstack` was needed** for either of the last two pin moves. ~~Content remains a design sketch, excluded from the build graph~~ **— corrected 2026-09-06: `vpay-db` compiles this file now** (`mod schema` → `include_server_schema!("../../../schemas/vpay.cstack", db = Postgres)`), so every declaration in it is checked by rustc as well as by the CLI, and **one** of them — `DisabledClient` — has queries running through it: `is_client_disabled` (a `find_unique`) since 2026-09-06, and **`disable_client` (an `upsert`) and `enable_client` (a `delete_many`) since later the same day** — the whole of that table's repository, and the model now carries four `@@allow` arms rather than one. The other six models are still a design sketch that a compiler now type-checks; nothing reads or writes through them, several do not match the live table, and the file still drives no migration. See "The first CrateStack read" and "The first CrateStack writes" below. **The migrations are now the authoritative schema, and this file has diverged from them on two constraints**: raw SQL in `backends/migrations/0002_create-providers.sql` and `0003_create-payment-intents.sql` expresses two `CHECK` constraints (`partial_refunds_imply_refunds`, `no_over_refund`) that CrateStack's grammar cannot — no `@@check(expr)` exists in 0.7.8, 0.7.10, 0.10.1, 0.11.1 or the pinned 0.12.0
(0.10.1's parser adds `@@sql`/`@@embedded_sql`/`@@server_sql` — for views, not
constraints — plus `@@paged`, `@@subscribe`, `@@audit` and `@@soft_delete`, none
of which is a cross-column constraint, and `cratestack-migrate` still gates
CHECK emission on a single field's validator). The `.cstack` file's own `GAP` comments on those two models now point at the migrations that implement them |
| Kubernetes Helm chart (`deploy/helm/vpay`) | 🟡 | New 2026-09-03 (Step 6, block B). Renders `Deployment/vpay-server` (2 replicas), `Deployment/vpay-worker` (1, `strategy: Recreate`), a `Service` carrying 8080 + 9090, a headless worker `Service` so the worker can be scraped at all, a `ServiceAccount` with `automountServiceAccountToken: false`, a server-only `PodDisruptionBudget` (`minAvailable: 1`), an optional profile-overlay `ConfigMap` mounted with **`subPath`** (mounting it at `/config` would replace the directory `backends/Dockerfile` bakes and the process would exit 78 naming a file it can no longer see), two `Ingress` objects (`/v1` and a tighter-limited `/v1/oauth/token`, because ingress-nginx applies `limit-rps` per Ingress), optional `NetworkPolicy`, `ServiceMonitor` and `PrometheusRule`. `DATABASE_URL` comes from `database.existingSecret`; the chart creates **no Secret and no Postgres** (step-6 decision (9) — CloudNativePG is documented in the chart README, deliberately not templated). Pods run `runAsNonRoot` as 65532 with `readOnlyRootFilesystem`, `drop: [ALL]` and `seccompProfile: RuntimeDefault`. **15 named `fail` guards** in `templates/_validate.tpl` refuse value combinations that are well-typed and cannot work (`grace-period`, `database-secret`, `signing-key-secret`, `rails-secret`, `image-digest-format`, `worker-replicas`, `pdb-minavailable`, `observability-port`, `rate-limit-ordering`, `ingress-host`, `overlay-empty`, `dashboard-not-templated`, `networkpolicy-database`, and — added by the Step 6 review pass — `rails-egress-except` and `extra-env-collision`), each with its own values file under `ci/guards/`. **🟡 and not ✅ for one reason that is not going away in this pass: NOTHING HERE HAS EVER BEEN APPLIED TO A CLUSTER** — not a real one, not kind (a smoke test was excluded by decision (9)). `helm lint`, `helm template` and `kubeconform` are the whole of the evidence; they say nothing about scheduling, admission, probe behaviour, `readOnlyRootFilesystem`, whether any CNI enforces the NetworkPolicy, PDB behaviour during a drain, or whether an ingress controller honours `limit-rps`. **One thing the chart was knowingly ahead of the code on, now closed:** ~~its liveness probes target `/livez` on port 9090, a listener that does not exist in any build~~ — block A landed `--observability-bind` and `/livez` on both binaries the same day and both `tests/cli.rs` suites prove they answer there and 404 on the traffic port (see the observability rows above). What remains true is narrower: an image built *before* that has nothing on 9090 and the kubelet would restart both pods in a loop, so `images.*.digest` must be pinned to something newer — and no image has been published at all. **One that is not closed:** `signingKey.defaultMode` is `0440`, not the `0400` the design document specifies, because an `fsGroup`ed Secret volume is owned `root:<fsGroup>` and `0400` would leave the key unreadable by UID 65532 — reasoned from the documented ownership rule, **not observed in a pod**. Resource requests/limits are placeholders; nothing has profiled either binary's RSS. See [flows/deployment.md](flows/deployment.md) and `deploy/helm/vpay/README.md` |
| CI `deploy (helm chart)` job + `just helm-check` | 🟡 | New 2026-09-03 (Step 6, block B). The job installs helm 3.16.1 and kubeconform 0.6.7 and runs **`just helm-check`**. Its third-party actions are pinned to full commit SHAs with version comments (`azure/setup-helm@1a275c3b` v4.3.1, `taiki-e/install-action@e67fa11c` v2.87.4) the way `release.yml` pins its own — `actions/*` stays on tags to match the rest of `ci.yml` — and the kubeconform tarball is verified against `sha256:95f14e87…`, the digest upstream publishes in the release's `CHECKSUMS` asset, because a version tag names a file that can be replaced in place (Step 6 review pass; both SHAs re-resolved against `gh api repos/<owner>/<repo>/git/ref/tags/<tag>` and the tarball re-downloaded and hashed on the authoring machine, rather than copied from a changelog) — the recipe, not a copy of its steps, so the gate and the local check cannot drift. The recipe: `helm lint` on the defaults and on `ci/values-full.yaml`; `helm template` on both; one `helm template` per file in `ci/guards/`, each of which must exit **non-zero with that guard's name in the message**; a grep for `nginx.ingress.kubernetes.io/limit-rps` on the rendered Ingress plus a check that the token endpoint's limit is the tighter of the two; and `kubeconform -strict -summary` over both renders with `-schema-location default` plus the datreeio CRD catalog for `ServiceMonitor`/`PrometheusRule`. Measured on the authoring machine 2026-09-03: **15 guards, all fired by name; 20 resources across the two renders — 20 valid, 0 invalid, 0 skipped.** (13 when block B landed; the Step 6 review pass added `rails-egress-except` — a rails egress `except` list missing `169.254.0.0/16`, i.e. a pod that can reach the cloud metadata endpoint — and `extra-env-collision` — a `server.extraEnv`/`worker.extraEnv` name that silently shadows one the chart sets, `DATABASE_URL` included. The recipe now also asserts that the fifteen names it expects are exactly the fifteen files under `ci/guards/`, so deleting a guard *and* its values file fails instead of passing quietly.) Negative controls run rather than assumed: disabling the `grace-period` guard, and separately the `rate-limit-ordering` guard, makes `just helm-check` fail with a message naming the guard that stopped firing; deleting the `limit-rps` annotation from the Ingress template makes it fail on the grep. The two guards the review pass added were put through the same control on 2026-09-03 — neutering the `fail` in `templates/_validate.tpl` for `rails-egress-except`, then for `extra-env-collision`, makes the recipe report `guard '<name>' did not fire` and exit non-zero — so neither is asserting the implementation back to itself. **🟡, not ✅, because no CI run of this job exists** — there is no CI run for this branch at all — so the claim that it passes on a GitHub runner is inference from a local run, not evidence. `helm-check` is deliberately **not** part of `just ci`: kubeconform downloads its schemas over HTTPS and `just ci` is expected to work offline. It adds no test binary, so `justfile`'s `expected_suites`/`min_tests` are untouched. The rendered *ordering* assertion (token limit ≤ `/v1` limit) is defence-in-depth only: the `rate-limit-ordering` guard fires first, so that branch is unreachable unless the guard is removed. **Changed 2026-09-04 (Step 9, lane 4): fifteen guards became seventeen**, and the recipe gained a two-direction assertion over the RENDERED YAML — the default render must name no `-checkout` object and `ci/values-full.yaml`'s must produce a Deployment, a Service and an Ingress. That second check exists because a `fail` guard cannot assert an *absence*. Green on 2026-09-04 on the authoring host: 17 guards all fired by name, kubeconform 23/23 valid. Still proves nothing about a cluster |
| Image publishing (`.github/workflows/release.yml` → `ghcr.io/vaam-apps/vpay-{server,worker,dashboard,checkout}`) | 🟡 | New 2026-09-03 (Step 6, block A). Triggers on `push` of a `v*` tag and on `push` to `master`. Eight `build` jobs — four images × two architectures (**corrected 2026-09-05**: it said six × three, which stopped being true when `vpay-checkout` joined the matrix on 2026-09-04) — each pushing an **untagged** manifest by digest (`outputs=type=image,push-by-digest=true`), then one `merge` job per image assembling the manifest list with `docker buildx imagetools create` and applying the tags once: `type=semver {{version}}` and `{{major}}.{{minor}}` on a tag, `type=raw,value=edge` on the default branch, `type=sha,format=long` always. **No `latest`** — the chart pins a digest for anything real. **Native runners, not QEMU** (step-6 decision (8)): `ubuntu-latest` for amd64, `ubuntu-24.04-arm` for arm64, which is what keeps `backends/Dockerfile`'s "the builder's own host triple" invariant from becoming an emulated `ring`/mimalloc build. Both Dockerfiles build from the repository root, the same context `compose.e2e.yml` uses (`backends/Dockerfile` COPYs `sdks/rust` and `examples/merchant-demo` because cargo refuses to load a workspace whose `members` list names a missing directory); **corrected 2026-09-05:** this row used to say neither declares an `ARG` and there is no `--build-arg` in the release path, and both halves are false — `backends/Dockerfile` declares `ARG VPAY_GIT_SHA` and the `build` step passes `build-args: VPAY_GIT_SHA=${{ github.sha }}` on every run, which is precisely why the `ARG`'s position in that file matters (see below). `provenance: mode=max` and `sbom: true` on every build. Every third-party action is pinned to a full commit SHA with a version comment, the way `ci.yml` pins `dtolnay/rust-toolchain`. **Permissions were narrowed in the Step 6 review pass:** the workflow-level grant is `{contents: read, packages: write}` and `id-token: write` moved to the `merge` job, the only one that runs cosign — an OIDC token is exchangeable for cloud roles and the six `build` jobs never sign anything; `attestations: write` was dropped everywhere because nothing calls the attestations API (buildx writes provenance and SBOM into the image index, which needs only `packages: write`). `actionlint` reports the file clean after the change. **🟡 and not ⛔ — a deliberate departure from `docs/plans/2026-09-03-step6-deployment.md` §8, which said ⛔ until a tag has pushed — for exactly two reasons, both narrow:** `actionlint` v1.7.12 reports the file clean (with `-shellcheck=` too, since no `shellcheck` was installed to lint the `run:` bodies), and the images it builds do build — see the `just release-dry-run` row below. ~~**🟡 and emphatically not ✅: NO TAG HAS BEEN PUSHED, THIS WORKFLOW HAS NEVER RUN, AND NO IMAGE EXISTS AT ANY OF THE THREE NAMES.**~~ **Corrected 2026-09-03 (Step 7): it has run, four times, and all four are green.** Runs `33772512791`, `33784613048`, `33789060270` and `33792230539`, all on `master` (2026-09-03, 15:25 → 18:43 UTC), **nine jobs each and every job successful** — the six `build` jobs (three images × amd64/arm64, on their native runner pools) and the three `manifest list + sign` jobs. Verified for this note with `gh run view --json jobs`, not taken on report. So GHCR authentication, `push-by-digest`, the `imagetools create` merge and the arm64 half of each manifest list are no longer unevidenced — each ran and exited 0 on a GitHub runner. **What is still unevidenced, and it is why this row stays 🟡:** nobody has pulled any of these images, so *this repository has never verified that an image exists at those names from the outside* — GHCR package visibility is unchecked, and a successful push does not make a package public; the semver tag path (`type=semver {{version}}` / `{{major}}.{{minor}}`) is untouched, because no `v*` tag has been pushed and these four runs took the `type=raw,value=edge` branch; and no image from any of them has been run anywhere. **Changed 2026-09-04: the GitHub organisation was renamed `vaam-store` -> `vaam-apps`, and GHCR does not follow an organisation rename** — a package stays addressed to the installation that pushed it, not to the org's current name. The first `master` run after the rename, `33894388991`, failed all eight `build` jobs at the push step with `denied: permission_denied: The requested installation does not exist`, because `release.yml` still hard-coded `NAMESPACE: vaam-store`. Fixed the same day: the namespace is now derived from `github.repository_owner` (lowercased, GHCR requires it) by a new `namespace` job, so a future rename no longer breaks this file — but the fix is proven only by the next `master` release run, which this pass could not trigger. **Updated 2026-09-05: that next run happened and the fix holds — `release.yml` has now run 13 times on `master` (12 green, the one failure being `33894388991` above), and the four post-rename runs `33898736618`, `33912330063`, `33918831901` and `33929374661` all resolved `NAMESPACE: vaam-apps` from `github.repository_owner` and pushed all four signed manifest lists; the stale "has never run / no image exists / nothing has been signed" claims that this row had already retired were retired the same day in `.github/workflows/release.yml`'s header, [runbooks/release.md](runbooks/release.md) (header and §6), [runbooks/deploy-and-rollback.md](runbooks/deploy-and-rollback.md) (header and §6), [flows/deployment.md](flows/deployment.md) (§9 and Status), [runbooks/README.md](runbooks/README.md) and `deploy/helm/vpay/README.md`, each citing run `33929374661`; GHCR package visibility remains unmeasured, because `gh api "orgs/vaam-apps/packages?package_type=container"` returns 403 for want of a `read:packages` scope and an anonymous `ghcr.io/token` pull request is refused.** **Changed 2026-09-05: `backends/Dockerfile` gained cargo-chef, and the `cache-from`/`cache-to` scopes in this workflow mean exactly what they meant before** — one image on one architecture, `mode=max`, nothing renamed. What changed is how much of a scope survives a commit. Until this date `ARG`/`ENV VPAY_GIT_SHA` was the *first* instruction of the builder stage and the `build-args` line passes a different `github.sha` on every push, so every layer after it missed on every run and `cache-from` could only ever restore the base image and the `apk add`. The `ARG` now sits below the dependency-compilation layer. **Measured by mutation on the authoring host, not on a runner** (2026-09-05, `linux/amd64`): with the `ARG` above the cook a sha-only rebuild costs 251 s and recompiles the graph; with it below, 116 s and the cook logs `CACHED`. **Nobody has read a GitHub Actions cache-hit rate for this file, before or after** — the `type=gha` store is network-backed with its own eviction policy and none of it was observed; this row's claim is about the Dockerfile's layer order, not about GHA. `actionlint` exit 0 after the comment edit. That edit also corrected "six `mode=max` scopes" to eight: the matrix has been four images × two architectures since `vpay-checkout` landed on 2026-09-04. See [flows/deployment.md](flows/deployment.md) §2, [runbooks/release.md](runbooks/release.md) §7 and [plans/exp8-notes/opus.md](plans/exp8-notes/opus.md) |
| Image signing (cosign, keyless) | 🟡 | Step-6 decision (3). `release.yml`'s `merge` job installs `sigstore/cosign-installer` and runs `cosign sign --yes <image>@<digest>` against the **manifest-list (index) digest** it reads back with `docker buildx imagetools inspect`, using the workflow's GitHub OIDC token (`id-token: write`, granted on the `merge` job alone since the Step 6 review pass). No key exists to store or rotate. ~~**⛔, not 🟡: nothing has ever been signed.**~~ **Corrected 2026-09-03 (Step 7): `cosign sign` has run.** In each of the four green `release` runs above, all three `manifest list + sign` jobs succeeded, and the step named `cosign sign (keyless, GitHub OIDC)` is a successful step inside them — checked step by step with `gh run view --json jobs` on `33792230539`, and the workflow sets no `continue-on-error`, so a failed signature would have failed the job. **🟡 and not ✅, because a green step is not a verified signature:** nobody has run `cosign verify` against any of these digests, no Fulcio certificate has been read, and no Rekor entry has been looked up — so the verification command in [runbooks/release.md](runbooks/release.md) §3, including its `--certificate-identity-regexp`, is **still** derived from the workflow's `on:` block and Fulcio's documented identity format rather than from a certificate anyone has seen. That command is the next thing to run, and until someone does, what this row claims is 'the signing step exited 0', not 'the image is verifiably signed'. Two limits are already known and written down rather than discovered later: only the index digest is signed, so a consumer pinning a per-architecture child manifest's digest is outside the signature; and renaming or moving `release.yml` changes the certificate identity and silently breaks every downstream `cosign verify` |
| `just release-dry-run` | ✅ | New 2026-09-03 (Step 6, block A). Builds all **four** release images (**corrected 2026-09-06**: this row said "three" from the day it was written, and stopped being true on 2026-09-04 when `vpay-checkout` joined the recipe's loop — the same stale count the recipe's own closing echo carried and that was fixed on 2026-09-05, one copy of it left behind here) for the **host architecture only** with `--push=false --provenance=false --sbom=false`, then runs `just helm-check`. Host-only on purpose and the recipe says why: the published images are multi-arch, each architecture is built on a native runner, and reproducing the other one locally means QEMU — the exact path step-6 decision (8) exists to avoid. **Run end to end on the authoring machine, 2026-09-03, exit 0.** All three images built for `linux/amd64` — `vpay-server` `sha256:2ef7ac29…` (14.8 MB), `vpay-worker` `sha256:5549b120…` (10.7 MB), `vpay-dashboard` `sha256:023d4d81…` (339 MB) — followed by `helm-check`: 13 guards, all fired by name (15 since the Step 6 review pass — the recipe was not re-run for the *image* half of this row); `/v1` `limit-rps=20` and `/v1/oauth/token` `limit-rps=5`; kubeconform `20 resources found in 2 files - Valid: 20, Invalid: 0, Errors: 0, Skipped: 0`. **This is the first time either Dockerfile has been built on an authoring machine** — the "Docker / compose" section below says they never had been, and that sentence is now out of date; the Rust image took 1 m 58 s (`Finished dist profile [optimized] target(s) in 1m 55s`), not the multi-minute build its reputation here suggested, and the older note that `just build-dist` fails locally for want of an `x86_64-linux-musl-gcc` does not apply inside the alpine builder, whose `cc` is musl-native exactly as the Dockerfile's header says. ✅ is a claim about **this recipe**, which does what it says and fails if a Dockerfile breaks. It is not a claim about the release: what the recipe does not exercise, by construction, is the registry, `push-by-digest`, the manifest merge, the attestations, cosign, and the arm64 build. **Re-run end to end 2026-09-05 after `backends/Dockerfile` gained cargo-chef, exit 0**, on the shared default buildx builder: four images for `linux/amd64` — `vpay-server` 15.9 MB, `vpay-worker` 12.7 MB, `vpay-dashboard` 344 MB, `vpay-checkout` 344 MB — then `helm-check` with **17 guards all fired by name**, `/v1` `limit-rps=20` / `/v1/oauth/token` `limit-rps=5`, kubeconform `23 resources found in 2 files - Valid: 23, Invalid: 0, Errors: 0, Skipped: 0`. The two Rust images share one `chef`/`planner`/cook chain, and the second of them logged the `cargo chef cook` step as `CACHED` — the 76 s cook was paid once for both. **What was NOT reproduced there, recorded because it disagrees with the isolated measurement:** on the dedicated `docker-container` builder, building `--target worker` straight after `--target server` was 1 s (everything cached); inside this `release-dry-run`, the `COPY backends` layers missed and the worker image's final `cargo build` re-ran for 126 s. The confound is known and not controlled for — another agent was building the same repository on the same rootless daemon and the default builder at the same time — so this is reported as an observation, not explained. **A stale string this run surfaced:** the recipe's closing echo said "three images built" while the loop above it builds four. **Fixed 2026-09-05 in the review pass**, which re-ran the recipe on the committed tree: exit 0 in 5 s, every layer of both Rust images `CACHED` on the shared default builder — including the `COPY backends` layer whose miss the note above records, which is the evidence that that miss was a stale cache entry rather than a defect |
| `vpay-checkout` image (`frontends/Dockerfile` target `checkout`) | ✅ | **New 2026-09-04 (Step 9, lane 4).** `node:22-alpine`, Next's `output: 'standalone'` bundle plus `.next/static`, `USER node` (uid 1000), `HOME`/`XDG_CACHE_HOME` on `/tmp` so a read-only root filesystem is survivable, and a `HEALTHCHECK` using Node 22's global `fetch` rather than adding curl to the image for a probe. Built from a clean context and **run** with `--read-only --tmpfs /tmp` on 2026-09-04: healthy, non-root, `/healthz` 200 with every security header lane 3's middleware sets, and the filesystem provably read-only. Published and signed by `release.yml` — ~~**which has still never run**~~ **corrected 2026-09-05: which has now run and published this image.** `ghcr.io/vaam-apps/vpay-checkout` index digest `sha256:5214e408be6062123b51374d99988ef20e28081fa96e7bcb0eb4ac2b5b12e51e`, pushed to `:edge` and `:sha-33d6c253…` by run `33929374661` and cosign-signed in the `manifest list + sign (vpay-checkout)` job (Rekor tlog index 2717615975). Nobody has pulled it |
| `GET /healthz` on the checkout app | ✅ | **New 2026-09-04 (Step 9, lane 4)**, and the only file that lane added under `frontends/apps/checkout`. A route handler that takes no dependency and reports on none: it answers "Next is serving", never "vpay is reachable". Deliberate — `middleware.ts`'s origins lookup already fails closed, so probing vpay from a liveness endpoint would take the payment page down during a rolling `vpay-server` deploy. It is what the compose healthcheck and all three of the chart's probes use |
| `vpay-checkout` Kubernetes workload (`checkout.enabled`) | 🟡 | **New 2026-09-04 (Step 9, lane 4).** Off by default, and that is a complete deployment rather than a missing one: `checkout.public_base_url` is optional in vpay's config, and without it `POST /v1/checkout/sessions` answers `checkout_not_configured`. When enabled the chart templates a Deployment, a Service and a **third** Ingress (its own, because a payer's browser reaches it directly and it carries no `/v1` traffic — its rate limit is looser for that reason). `runAsNonRoot` with **no** invented UID (the image has an `/etc/passwd`, unlike the scratch ones), `readOnlyRootFilesystem: true` with a memory-backed `emptyDir` on `/tmp` that is load-bearing rather than hygiene, no rails Secret and no signing key — this pod holds no credential of any kind. Two named guards: `checkout-not-templated-by-default` (the page's Ingress on while the page is off) and `checkout-templated-when-enabled` (no `publicApiUrl`; an Ingress with neither `host` nor `path` or with both; TLS with nothing to populate the Secret). 🟡 for the reason every chart row is: **no pod has ever run.** What is new is that the *container* has been observed running the way the chart asks it to. The path-prefix Ingress shape is templated and has been run by nobody |
| Image publishing — `ghcr.io/vaam-apps/vpay-checkout` | 🟡 | **New 2026-09-04 (Step 9, lane 4).** A fourth entry in `release.yml`'s `build` matrix (both architectures, native runners, pushed by digest) and in its `merge` matrix (one multi-arch manifest list, the tag set, `cosign sign` over the index digest) — identical treatment to the other three. `actionlint` exit 0. **Nothing in that workflow had ever run and no image had ever been published or signed, as of when this row was written.** **Changed 2026-09-04: it has now run, once, and failed.** The organisation was renamed `vaam-store` -> `vaam-apps` the same day; run `33894388991` (the first `master` run with `vpay-checkout` in the matrix) failed all eight `build` jobs, this one included, at the push step — see the main image-publishing row above for the cause and the fix. **Updated 2026-09-05: it has since run green four times** (`33898736618`, `33912330063`, `33918831901`, `33929374661`); the latest published `ghcr.io/vaam-apps/vpay-checkout` at index digest `sha256:5214e408be6062123b51374d99988ef20e28081fa96e7bcb0eb4ac2b5b12e51e` and signed it (Rekor tlog index 2717615975). Still 🟡: nobody has pulled the image and GHCR package visibility is unmeasured |
| `vpay-shop` / `vpay-checkout` in the demo stack | 🟡 | **New 2026-09-04 (Step 9, lane 4).** `just demo-up` starts eight services, not six. The shop gets its own `shop` database in the same `postgres` container, created by `deploy/dev/postgres-init/10-shop-database.sql` — which Postgres runs **once, on an empty data directory**, so a `pgdata` volume from before this change has no such database and `vpay-shop` dies in `prisma migrate deploy`; `just demo-down` is `down -v` and is the answer. Verified on 2026-09-04 on a green `just demo`: both migrations applied, five products, the catalogue rendering `12 000 FCFA`, and the checkout container serving the hosted session URL step 5 printed. **Lane 4's own run clicked through nothing** — its `orders` table was empty at the end. Lane 6's Cypress specs then did, in the `vpay-ci` VM, and the shop's orders reached `paid` from the shop's own webhook |
| `demo_orange_port`, and two demos on one machine | ✅ | **Changed 2026-09-04 (Step 9, lane 4).** Step 9's lane 2 made this a *checked* value — 8082 was the only one that worked, because the stub's `payment_url` comes from a committed mapping that spells it, and WireMock cannot learn what the host published it on. `gen-demo-keys` now writes a per-project **copy** of those mappings with the port substituted (`.e2e/<demo_project>/wiremock-orange/`) and `compose.demo.yml` mounts the copy; the committed mapping is untouched and stays the CI/e2e default. Measured 2026-09-04: two stacks up at once, sixteen containers, **ten published ports, no collision**, the two Orange containers serving `localhost:8082` and `localhost:18082` respectively (grepped inside them), and the second stack's walkthrough green while the first was up with its redirect and its session URL on its own ports. **`.e2e/application-demo.yml` and both merchant key pairs are still shared** — Step 8's limitation is unchanged and now has a second key pair in it |
| `examples/merchant-demo` step 5 — checkout sessions | 🟡 | **New 2026-09-04 (Step 9, lane 4).** Creates one hosted and one embedded session on a **fresh intent each** (`checkout_sessions.payment_intent_id` is unique and the route requires an intent with no charge, so none of step 4's can be reused), reads each back through `retrieve` and fails if the stored session differs. Prints the hosted `url` **in full** — a human is meant to open it — and the embedded `client_secret` as `[N chars redacted]`, the same treatment step 2 gives the access token, because this output reaches CI logs and pasted transcripts. Verified green on 2026-09-04 and again in three consecutive VM runs. 🟡 because **it stops there**: no browser is opened by the demo, no rail is called for either session, and both are `open`/`unpaid` when the demo exits — the program says so itself, in its last line |
| The demo stack's currency — XAF on both rails | ✅ | **Changed 2026-09-04 (Step 9, lane 4, per lane 7's addendum).** The generated overlay now carries its own `providers:` block putting **both** rails on XAF, because the demo shop prices its catalogue in XAF and offers a payer both, and `currencies_agree` refuses a confirm whose rail settles in another currency. **`config/application.yml` and `application-sandbox.yml` are unchanged and still put `mtn_momo` on EUR, because MTN's real sandbox rejects XAF** ([docs/flows/money.md](flows/money.md)) — the divergence is written into the generated overlay's own comment, `docs/runbooks/demo.md` §"One currency", and `requesttopay-status.json`'s `metadata.why`, which also records that **no MTN mapping matches on a currency at all** and that `StatusResponse` never deserialises one. `sdks/stripe-compat`, `frontends/tests/e2e/cypress/tasks/checkoutTasks.ts` and `examples/checkout-browser/mint.mjs` moved with it — all three run against this overlay, because CI's `e2e` job brings the stack up with `-f compose.demo.yml`. Proven by the three MTN outcomes of every green `just demo`, each an XAF intent confirmed on `mtn_momo`. `gen-demo-keys` regenerates the overlay when it does not settle `mtn_momo` in XAF — an awk range over that sequence item, not a presence grep, after lane r2 measured the presence grep letting an overlay edited back to EUR survive |
| Compose publications bound to `127.0.0.1` (`compose.demo.yml`) | ✅ | **Changed 2026-09-04 (Step 9, lane r2).** All five services this file publishes — `vpay-server`, `wiremock-orange`, `wiremock-webhook`, `vpay-checkout` and `vpay-shop` — are bound to `127.0.0.1:` rather than `0.0.0.0`. Step 8's runbook warned that anyone on the same LAN could post at the demo's unauthenticated callback route; Step 9 published four more ports, including a payment page, so it was bound rather than warned about again. Measured on the authoring host: a container published `-p 19099:6379` answered on both `localhost` and the LAN address; the same container on `-p 127.0.0.1:19099:6379` was connection-refused on the LAN address. `docker compose … config` over the three files reports `host_ip: 127.0.0.1` for all five. **`compose.e2e.yml` was deliberately not touched** — CI's `e2e` job runs against it and a bind address is a thing to get wrong in a runner — so the same two files without the demo overlay still publish on `0.0.0.0`, and `dashboard` stays there in both |
| `aarch64-unknown-linux-musl` + [ADR-0014](adr/0014-builder-host-musl-triple.md) | 🟡 | New 2026-09-03 (Step 6, block A). `.cargo/config.toml` now carries an explicit `[target.aarch64-unknown-linux-musl]` `rustflags = ["-C", "target-feature=+crt-static"]` beside the x86_64 one (step-6 decision (8)). Before this, an arm64 image would have been statically linked by rustc's default for the musl target rather than by this repository's explicit instruction — the same result today, arrived at for a different reason, which is what ADR-0014 refuses to leave implicit. ADR-0014 supersedes **only** ADR-0004's Decision line naming `x86_64-unknown-linux-musl`; static musl, `FROM scratch` and mimalloc are unchanged, and [ADR-0004](adr/0004-musl-mimalloc.md)'s Status now points at it. ~~**🟡 for one blunt reason: `aarch64-unknown-linux-musl` HAS NEVER BEEN COMPILED.**~~ **Corrected 2026-09-03 (Step 7): it has been, on a GitHub runner.** The four green `release` runs (`33772512791`, `33784613048`, `33789060270`, `33792230539`) each include three `build … (arm64)` jobs on `ubuntu-24.04-arm`, and `backends/Dockerfile` builds the *builder's own host triple* on `rust:alpine` — which on an arm64 host is `aarch64-unknown-linux-musl`. So the target compiles and links, three binaries' worth, four times over. It is still **not** an installed rustup target on the authoring machine, so nothing here was reproduced locally. **🟡 rather than ✅ because compiling is not running:** no arm64 image has been pulled or started anywhere, so the `+crt-static` flag's *effect* — a binary that runs in a `FROM scratch` image with no dynamic loader — is still unobserved on this architecture |
| Observability listener (`--observability-bind`, `/livez`, `/metrics`) | ✅ | New 2026-09-03 (Step 6, block A). A **second** TCP listener on **both** binaries, default `0.0.0.0:9090`, serving exactly two routes from `vpay_api::observability` — `GET /livez` (a static `ok`, no state, no database) and `GET /metrics` (`PrometheusHandle::render()`, `text/plain; version=0.0.4`). Neither is mounted on the traffic router: `/metrics` names every rail, route pattern and error code this deployment has, and `--bind`'s port is the one an Ingress fronts. Both directions are asserted twice over — in-process by `neither_livez_nor_metrics_is_reachable_on_the_traffic_router` (`vpay-api`) and `nothing_else_is_mounted_on_this_listener` (`vpay_api::observability`), and against the **running binary over real sockets** by `the_observability_listener_serves_livez_and_metrics_on_its_own_port_only` in each binary's `tests/cli.rs`, which also asserts the port stops accepting once the process is reaped. The listener is bound **after** the last fallible startup step, so `/livez` answering means booting finished rather than merely started; `/healthz` (still on 8080, still running `SELECT 1`) stays the *readiness* probe and the chart wires the split that way. ✅ and not 🟡: every claim in this row has a test that fails if it stops being true |
| `/metrics` scraped by a Prometheus | 🟡 | **Nothing has ever scraped it.** The endpoint is real and its content is asserted by tests (rows above and below), but no Prometheus, agent or `ServiceMonitor` has ever polled a running vpay — the chart's `ServiceMonitor` is off by default and no cluster has run this chart at all. So every number below is a *series a scrape would find*, never a series anyone has watched over time. Rate, histogram quantiles and alert evaluation are all unexercised, and a metric that is exported but never collected is a metric whose cardinality, staleness and reset behaviour are all unmeasured |
| The twelve metric names (`vpay_core::metrics`) — which are emitted, and by which seam | ✅ | **All twelve are now emitted, one per seam.** `vpay_core::metrics` owns the names and installs no recorder (a library that installed one would take the decision out of the binary's hands); each binary installs exactly one in `install_recorder`. The seams, one per name: **`vpay_build_info{version,git_sha}`** — `record_build_info`, from each `install_recorder`. **`vpay_http_requests_total{route,method,status}`** and **`vpay_http_request_duration_seconds`** — `vpay_api`'s `track_http_metrics` middleware, mounted three times inside `router()` (once on the outer routes, once in each nest) because axum only exposes `MatchedPath` to a layer on the router that matched; the outer mount is applied *before* the nests so a `/v1` request is counted once, and `every_mounted_group_is_counted_exactly_once` fails with `2` if that line moves. `route` is always the route **pattern** (`/v1/payment_intents/{id}`) or `unmatched` — never a caller-supplied path, which would let anyone who can reach the port mint a series per request. **`method` was not bounded and now is** (Step 6 review pass): it was `request.method().to_string()`, and an HTTP method is a free-form token (RFC 9110 §9.1) that `http::Method` parses rather than rejects — so `M12345 /healthz`, unauthenticated, on a loop, minted one series per request through the same hole the `route` label was closed against. It is now the ten methods `http::Method` names (`QUERY` included) verbatim or `other` (`vpay_api::OTHER_METHOD`), pinned by `an_extension_method_is_counted_under_a_bounded_label`, which drives an extension method through the real router and asserts the raw text is absent from the render — reverting the seam to `to_string()` fails it. **`vpay_provider_requests_total{provider,operation,error_kind}`** and **`vpay_provider_request_duration_seconds`** — `vpay_provider::Measured`, a port decorator applied in `vpay_api::v1::boot::adapters_by_code`, the one funnel both binaries and the integration harness resolve rails through. Not inside the adapters: a cross-rail concern written twice is two copies that drift, and it would be rail-specific code where ADR-0002 forbids it. It counts **port calls, not wire requests** — an Orange `submit` that mints a token first is two HTTP requests and one increment — and a call refused before the socket opens is counted with that refusal's `error_kind`. **`vpay_charge_transitions_total{provider,from,to}`** — `vpay-db`'s crate-private `charges::record_transition` (a free function, not a repository method: the *only* correct callers are the six statements that move `charges.state`, and a trait method would put it within reach of everything else), backing the six statements that can move `charges.state` (three in `charges`, three in `settlement`) and nothing else. In the database layer because every transition passes through it and only some pass through the worker: a confirm opens and submits a charge inside `vpay-api`. Labels are read off the row the statement returned, so a compare-and-swap that matched nothing counts nothing. **A transition is now counted only once it is committed** (Step 6 review pass). The three writes in `settlement` own their transaction and always recorded after their own `COMMIT`; the three in `charges` run inside a *caller's* transaction and used to record inline, so a rolled-back insert — the confirm path enqueues the poll job in the same transaction — was counted for a charge that does not exist, while the module header claimed the metric "cannot claim a transition the database refused". They now return their row and the caller calls `charges::record_opened` / `charges::record_left_submitting` after `tx.commit()`. `a_rolled_back_charge_insert_counts_nothing_and_a_committed_one_counts_once` in `vpay-db/tests/repositories.rs` proves both halves against a real Postgres, aborting the transaction with migration 0021's `kind_is_known` CHECK rather than an injected failure; putting the call back inside `insert_for_intent` fails it. **`vpay_jobs_claimed_total`**, **`vpay_jobs_completed_total`**, **`vpay_jobs_oldest_claimable_age_seconds`** — `vpay_worker::run_once` and the loop's 60 s gauge task (Step 6 block A; see the two rows below). **`vpay_error_events_total{category,code,severity}`** and **`vpay_alert_events_total{category,code}`** — `vpay_core::metrics::record_error_event`, called from `ApiError::log`, `vpay_worker::handlers::log_failure` and the job loop's queue-unreachable arm: the same statements that write `alert = true`, so the counter and the log field are one decision rather than two that can diverge. **`vpay_webhook_deliveries_total{outcome}`** — `vpay_worker::webhooks::handle_deliver` and `record_failure`, at the two points a delivery attempt's outcome becomes durable: right after `vpay_db::WebhookDeliveries::record_success`'s compare-and-swap commits and actually changed the row (`succeeded`; a second pass over an already-settled delivery counts nothing), and right after `record_failure`'s call to `record_attempt` commits, labelled `retry` or `exhausted` by the same `delivery_delay` result that decided the row's new `next_attempt_at`/`state`. Both are single `UPDATE` statements against the pool with no explicit transaction, so "after `.await?` returns `Ok`" already is "after commit". Proved by `the_ladder_walks_delivery_delay_and_then_succeeds` (three `retry` increments, one `succeeded`) and `a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled` (one `exhausted`) in `backends/tests/integration/tests/webhooks.rs`, both scraping the real observability router rather than reading `PrometheusHandle::render()` directly. **`vpay_core::metrics::ALL`'s doc promised a binary-side test asserting `/metrics` against the vocabulary and there was none; the Step 6 review pass wrote it** rather than dropping the sentence: `the_observability_listener_serves_livez_and_metrics_on_its_own_port_only` in `backends/apps/vpay-server/tests/cli.rs` now parses every sample line the running server renders, folds off the `_bucket`/`_sum`/`_count` suffixes, and fails naming any family that is not in `ALL` — so a metric recorded under a name nothing declares (no description, no runbook, no alert) fails the build |
| `vpay_alert_events_total` vs. `alert = true` in the logs — a **known, deliberate gap** | 🟡 | The counter is a *subset* of the log lines that flag `alert = true`, not the whole of it, and that is worth stating because `VpayPageableErrorEvents` pages on the counter. Counted: the three sites that log a **classified** error at its own severity (`ApiError::log`, `handlers::log_failure`, the job loop's queue-unreachable arm). Not counted: `run_loop::log_disposition` (it re-reports a failure `log_failure` already counted, at a wider severity net, so counting it would double some incidents), and the seed-singletons, release-leases and settlement-contradiction lines, which set `alert = true` unconditionally and carry no `Classify` value to derive `category`/`code` from. Closing this means giving the worker's ad-hoc alerts a classified error to carry — a change to the worker's error model, not to the counter. `vpay_core::metrics::record_error_event`'s own doc comment carries the same list |
| `vpay_build_info{git_sha}` — wired, and `unknown` on every local build | 🟡 | **Changed 2026-09-03 (Step 6, block C): the "no build.rs, no ARG" gap is closed.** `vpay_core::metrics::git_sha()` is `option_env!("VPAY_GIT_SHA")` with the fallback `unknown`, and `backends/crates/vpay-core/build.rs` exists for one line — `cargo::rerun-if-env-changed=VPAY_GIT_SHA` — which puts the variable into cargo's fingerprint so a changed sha rebuilds instead of silently shipping the previous label. `backends/Dockerfile` declares `ARG VPAY_GIT_SHA=unknown` and exports it as an `ENV` into the builder stage (an `ARG` alone is invisible to rustc); `.github/workflows/release.yml` passes `build-args: VPAY_GIT_SHA=${{ github.sha }}`. **Nothing shells out to `git`, ever** — the Docker build context is a `COPY` of source trees with no `.git` in it, and a sha read from a build machine's own checkout would describe another tree. **Consequence, stated plainly: every build anyone can make locally reads `git_sha="unknown"`.** `just demo`, `compose*.yml`, `just release-dry-run` and every `cargo build` pass nothing and get the honest answer. Proven mechanically rather than by inspection: `VPAY_GIT_SHA=deadbeefcafe cargo build -p vpay-server` recompiles `vpay-core` and the string appears in the binary; an unset rebuild recompiles again and it is gone (`strings target/debug/vpay-server | grep -c deadbeefcafe` → `1`, then `0`, measured 2026-09-03). ~~🟡 and not ✅ because **no image has ever been built with a real sha**: `release.yml` has never run, so the `build-args` line is unexecuted configuration~~. **Corrected 2026-09-05: the `build-args` line has executed.** Release run `33929374661`'s `build vpay-server (amd64)` step invokes `docker buildx build … --build-arg VPAY_GIT_SHA=33d6c253a232958604801518a08a2f34accb689c …` (same for the arm64 job and for both `vpay-worker` jobs), so the published `vpay-server` and `vpay-worker` images carry `vpay_build_info{git_sha="33d6c253…"}` rather than `unknown`. **Still 🟡, and the reason is narrower than it was:** this is read from the build command in the run log, not from a `/metrics` scrape of a published image — nobody has pulled or run one, so no one has observed the series with a real sha on it. The sentence about *local* builds above is unchanged and still true |
| OTLP traces / the `opentelemetry` pin | ⛔ | Step-6 decision (6), recorded rather than deferred silently. The root `Cargo.toml` used to pin `opentelemetry = "0.27"` with **no consumer anywhere** (`backends/`, `sdks/`, `.xtask/` all grepped clean); that pin is gone, replaced by `metrics` + `metrics-exporter-prometheus` with `default-features = false` — the exporter's defaults pull `hyper` (a second HTTP stack) and `aws-lc-rs` (banned outright in `deny.toml`, because two rustls providers in one process is what makes `install_default()` panic). What is lost, said plainly: **there are no traces**, so a slow request can be seen in `vpay_http_request_duration_seconds` and not decomposed, and a later OTLP migration means re-instrumenting the call sites listed above. JSON logs with a request id on every event are the correlation mechanism until then |
| Prometheus `ServiceMonitor` / `PrometheusRule` / alert thresholds | ⛔ | Templated by the chart (both off by default, both validated by kubeconform in CI against the upstream CRD schemas). **Changed 2026-09-03 (Step 6, block C): the rules are no longer inert, but they are still unevaluated.** Every metric the five rules — `VpayProviderErrorRateHigh`, `VpayUnresolvedChargesRising`, `VpayJobQueueBehind`, `VpayPageableErrorEvents`, `VpayJobsDeadLettered` — query is now emitted on a real path, with the label sets the queries select on (`error_kind`, `to`, `outcome`, `kind`); the rows above name the seam for each and the integration suites assert the exact series text. **⛔ stands, and for a reason that has not changed: nothing has ever scraped a vpay process.** No Prometheus, no agent, no `ServiceMonitor` has collected one sample, so no rule here has ever been *evaluated* — rate windows, `for:` durations, histogram quantiles and label joins are all unexercised, and a query that is syntactically fine against a series that exists can still select nothing. **Every threshold is PROPOSED, not derived** (step-6 decision (5)): the two runbooks this repo had contain no numbers to transcribe, and these were invented against a system that has never taken a real payment. Each rule carries `provisional: "true"` as a label so that is visible in Alertmanager and not only here. Installed today these alerts can never fire, which is indistinguishable from a healthy system |
| `vpay-dashboard` Kubernetes workload | ⛔ | Deliberately not templated. `dashboard.enabled: true` is a **named template failure** (`dashboard-not-templated`), not a silent no-op — the chart's equivalent of the rule that unwritten code must announce itself. `frontends/Dockerfile` produces a `node:22-alpine` image that declares no `USER`, and Next's standalone server's behaviour under `readOnlyRootFilesystem` has never been observed; a Deployment with an invented UID and a guessed set of `emptyDir` mounts would look finished and be a guess. The image itself is published by block A's `release.yml`, which is not part of this pass either |
| Database backups, PITR and retention ([ADR-0013](adr/0013-database-backups-and-retention.md)) | ⛔ | New 2026-09-03 (Step 6, block C). **The ADR's own status is *proposed*, and nothing in it is implemented: no backup of any vpay database has ever been taken, no restore has ever been performed, and no drill has ever run.** What it contains is an obligations table read off migrations `0001`–`0025` — for each table, what it holds, whether any code writes it today, and what losing it costs — plus proposed objectives (**RPO ≤ 5 min, RTO ≤ 60 min**, both invented, neither measured), continuous WAL archiving with PITR (a managed provider's own, or CloudNativePG's `barmanObjectStore` — documented, deliberately not templated, step-6 decision (9)), and a proposed **30-day PITR window / 90-day full-backup retention** anchored to nothing external. Three consequences it records because they are easy to miss: a restore needs **two inputs from possibly two custodians** — the backup *and* the signing-key Secret, since `oauth_signing_keys` holds public halves only (migration `0010`); restoring `jobs` re-runs work that has already run, bounded by the `jobs_dedupe_key` unique index to one row per work item but **not** prevented by it, which is why `resubmit_charge` is the kind to think about before starting a worker; and a restore **silently un-revokes** a `disabled_clients` row and **re-opens the assertion-replay window** for unexpired `jti`s, neither of which is visible from outside. ⛔ and not 🟡 because nothing here is implemented, enforced or checked — the database is external by decision (9), so no CI job could check it, and the obligations table has no `xtask` guard against drifting as migrations are added |
| Operational runbooks (`docs/runbooks/`) | 🟡 | **Eight documents; none has ever been followed against a deployment, because no deployment exists.** Four are new on 2026-09-03 (Step 6, block C): [deploy-and-rollback.md](runbooks/deploy-and-rollback.md) (`helm upgrade --atomic`, the `grace-period` guard, the per-binary drain semantics — ~~the server's timeout path still has no test~~, corrected in the Step 6 review pass: `an_in_flight_request_that_outlasts_the_grace_period_is_exit_1_and_says_so` in `backends/apps/vpay-server/tests/cli.rs` holds a real request open on a real socket across a real SIGTERM and asserts exit 1 plus the forced-cutoff WARN; it synchronises on an `Expect: 100-continue` interim response rather than a sleep, so a loaded runner cannot make it flake, and it proves nothing about a slow *rail* — and the three things a rollback does not undo: a migration, a signing-key rotation, and anything a rail already did), [rotate-signing-key.md](runbooks/rotate-signing-key.md) (restart-based rotation, the 24 h `ROTATION_OVERLAP`, and the **exit 78 not 69** crash loop a rollback to a retired `kid` produces — `DbError::SigningKeyRetired`, pinned by `a_rollback_to_a_retired_signing_key_exits_78_and_a_dead_database_still_exits_69` at `backends/apps/vpay-server/src/main.rs:843`, which is a **unit test over `exit_code_for`, not a subprocess test**, and by `ensure_active_signing_key_refuses_to_reactivate_a_retired_kid` against a real Postgres), [rotate-rail-credentials.md](runbooks/rotate-rail-credentials.md) (the rail credentials the deployed image's `config/application.yml` references as `${VAR}` — six on this branch, read off the revision at upgrade time because the list grows as features land, and Step 5 on master already adds `MERCHANT_WEBHOOK_SECRET` — exit 78 on **both** binaries for a missing one, restart both workloads, and `--from-env-file` with a `chmod 600` temp file rather than `--from-literal`, which puts a credential in shell history and in `ps` — plus ADR-0010's **dual-authority** revocation check, YAML `merchant_clients` for identity *and* `disabled_clients` for subtraction, which `docs/roadmap.md` had recorded as having no runbook) and [restore-from-backup.md](runbooks/restore-from-backup.md). Two were updated to name their alert and its query: `VpayProviderErrorRateHigh` and `VpayUnresolvedChargesRising`. Each stated plainly that ~~**no build emits the metric it queries**~~ — a sentence written before block C landed on the same day, now retired: both metrics *are* emitted and served on `--observability-bind`, and what has never happened is a **scrape**, which is what those pages now say. Thresholds are still proposed rather than measured. `provider-error-rate.md` was also rewritten in the Step 6 review pass: `VpayProviderErrorRateHigh`'s numerator was `error_kind="provider_error"`, which excluded `provider_unavailable` — so a total rail outage could not fire the alert named for it — and is now `error_kind!=""`. **That set includes `charge_declined`**, a rail decision rather than a rail failure, so against real mobile-money traffic the rule will fire on an ordinary decline rate; whether to exclude declines (`error_kind!~"|charge_declined"`) is left to the maintainer alongside the threshold, and the runbook's first step is now to split by `error_kind` because the alert no longer says which half of the page applies. **The only new evidence anywhere in this row is `restore-from-backup.md`'s SQL**: every statement in it was executed on 2026-09-03 against a scratch `postgres:16-alpine` with **the 21 migrations that existed when it was written** (~~the branch now carries 25~~ **corrected 2026-09-04: the branch carries 27**; the drill has not been re-run against them), on a fixture containing one deliberately torn ledger transaction — the per-transaction balance check found it (`ltx_torn`, 3000 debit vs 2900 credit) and returned `(0 rows)` once the missing leg was added, so the check reads the data rather than asserting itself; the duplicate-charge probe was refused by `one_charge_per_intent` by name; and re-enqueueing an existing `dedupe_key` reported `INSERT 0 0`. That proves the SQL is valid against this schema and nothing about a backup, a restore, or the RTO. **No `kubectl` or `helm` command in any of these pages has been run against a cluster.** 🟡 rather than ✅ for that reason, and it cannot become ✅ until [roadmap.md](roadmap.md)'s "walk each runbook against a real system" item is done |

### Docker / compose — made bootable, proven by CI run `33647189156`

Both Dockerfiles and `compose.e2e.yml` date from 2026-08-09 (musl host
target, non-root UID 65532, `.dockerignore`). Two days later `--config` /
`VPAY_CONFIG` became mandatory in both binaries, and nothing in the image
or the compose file supplied it — so from 2026-08-11 to 2026-09-02 the
"never built" stack was also a stack that could not have booted if it had
been. A previous version of this section said the files were "rewritten
this pass"; `git log` says otherwise, and this section now says what the
files actually are.

What changed on 2026-09-02, and what each change is proven by:

- `backends/Dockerfile` bakes `config/` into both runtime stages and sets
  `VPAY_CONFIG`. Proven by CI's `e2e (compose)` job on run `33647189156`.
- `compose.e2e.yml` sets `VPAY_CONFIG` and the three rail `${VAR}`
  placeholders on both services. `docker compose config` renders it, and
  the same CI run proves the processes boot behind it.
- `config/application.yml` names the real WireMock service hosts. Proven by
  `vpay-config`'s `a_valid_config_loads_and_produces_the_expected_typed_values`,
  which loads the real file.
- No `HEALTHCHECK` was added to the `scratch` image — there is still no
  executable in it that could run one, and the `--healthcheck` self-check
  mode `compose.e2e.yml` describes was not built. CI observes readiness
  from outside by polling `/healthz` instead.

**Corrected 2026-09-03 (Step 6, block A): all three images have now been
built on an authoring machine.** This section used to say "none of it was
built on an authoring machine" — Docker Hub unreachable from one, a
rootless daemon that could not start a container on the other, and
`just build-dist` failing there at `ring`'s C build for want of an
`x86_64-linux-musl-gcc` cross compiler. The daemon works now, and
`just release-dry-run` built `vpay-server`, `vpay-worker` and
`vpay-dashboard` for `linux/amd64`, exit 0 (see the `just release-dry-run`
row above for the digests and the timing). The cross-linker problem never
applied inside the builder anyway — the alpine image's `cc` is musl-native,
exactly as the Dockerfile's header says; it applies to `just build-dist`
run on the host, and that is still unbuilt here. **What this does and does
not change:** the images build locally, which nothing had shown before. It
says nothing about whether they boot — no local run has started a
container from them, and CI's `e2e (compose)` job remains the only evidence
for that. The rows above cite CI run
`33647189156` (the pull request) and `33650294682` (the first run on
`master`) as their evidence; ✅ means exactly "the images build, the stack
boots, `/healthz` answers 200 and the one Cypress spec passes," and nothing
about what the stack can do once up — it still serves only `/healthz`.

### `cargo deny` — fixed properly, not suppressed

`cargo deny check` previously failed and is now clean, without adding a single
`ignore` entry to `deny.toml` (`ignore = []`, confirmed by reading the file).
Two real dependency upgrades did the work:

- `time` 0.3.45 → 0.3.47, a production dependency (reachable from
  `vpay-core` via `sqlx`'s `time` feature, not only from `dev-dependencies`),
  addressing a `time`-crate advisory reported as RUSTSEC-2026-0009.
- `testcontainers` 0.23 → 0.27 and `testcontainers-modules` 0.11 → 0.15 (a
  test-only dependency), which moved onto `bollard` 0.20. That drops
  `rustls-pemfile` entirely (RUSTSEC-2025-0134) and replaces the unmaintained
  `tokio-tar` with the maintained `astral-tokio-tar` fork (RUSTSEC-2025-0111).
  Both advisory IDs are cited in `Cargo.toml`'s own comment next to the
  `testcontainers` pin.

**2026-09-02:** `cargo deny check` had regressed to `advisories FAILED` on
`master` as the advisory database moved, not because of any vpay change:
RUSTSEC-2026-0258 (`h2` 0.4.15, unbounded empty DATA frames, reachable
through `reqwest`/`hyper` in every shipping binary) and a yanked `chacha20`
0.10.1 (dev-only, via `testcontainers → ferroid → rand`). Both fixed the same
way as before — `cargo update -p h2 -p chacha20` to 0.4.19 / 0.10.2 — with
`ignore` still holding only the `rsa` entry above. The `authkestra` upgrade to
0.7.1 added one new crate to the graph, `authkestra-crypto-util` (MIT OR
Apache-2.0); `cargo tree -i aws-lc-rs`/`-i aws-lc-sys`/`-i openssl-sys`/
`-i native-tls` all still report no match, so the single-`ring`-provider and
rustls-only invariants survived the bump.

**Also 2026-09-02, from the SDK work:** the workspace's `reqwest` pin was
`0.12`, and nothing had ever consumed it — every `reqwest` in the lock was
`0.13.4` via `authkestra-*`. `vpay-sdk` became the pin's first consumer,
which would have compiled two HTTP+TLS stacks (`cargo deny` warns on
duplicate majors but does not fail). The pin moved to `0.13` with
`rustls-no-provider` (0.13 has no `ring`-flavoured feature — its `rustls`
feature means aws-lc-rs, which `deny.toml` bans); `cargo tree -d` now shows
one `reqwest`, and the four "no match" checks above were re-run after the
move. The pre-existing duplicate `webpki-roots` (0.26 via `sqlx-core`, 1.0
via `hyper-rustls`) is unchanged and predates all of this.

`rust-version` moved `1.85` → `1.88` in the same pass, computed as the max
`rust_version` declared anywhere in the resolved dependency graph (`cargo
metadata`, including dev-dependencies). **Read the comment block at the top of
`rust-toolchain.toml` before trusting that number**: it states plainly that
1.88 has **not** been verified by actually compiling with a 1.88 toolchain,
and that a large minority of the graph declares no `rust_version` at all, so
the true floor could in principle be higher. **Re-derived 2026-09-05** during
the toolchain bump below and it did **not** move — still 1.88. Two figures in
this paragraph did: it used to say "only stable 1.95.0 was available here"
(the pin is 1.98.0 now) and "63 of 317 packages", which was the graph as it
stood on 2026-09-02; today's `cargo metadata` reports **135 of 477** packages
with no `rust_version`. The crates that set the 1.88 ceiling are now
`darling` 0.23.0, `jsonwebtoken` 11.0.0, `serde_with` 3.21.0, `time` 0.3.47
(with `time-core` 0.1.8 and `time-macros` 0.2.27) and `testcontainers` 0.27.3
/ `testcontainers-modules` 0.15.0 — the old comment named only `time` and the
two `testcontainers` crates.

### Toolchain pin — `1.95.0` → `1.98.0` (2026-09-05)

**Why it moved.** CrateStack 0.11.1 is what this repository had adopted when
this pin moved (0.12.0 since 2026-09-07, which declares the same floor), and
`cratestack check` is the seventh gate in `just verify`. Every crate in that
release declares `rust-version = "1.98.0"`. Under the old pin, obtaining the
tool a gate depends on required stepping outside the checkout or taking a
prebuilt binary — for which **Linux musl has none**. The maintainer's
decision was to move the pin rather than pin CrateStack back to 0.8.15, the
last release that supports 1.95.0.

**Proven in both directions, on the authoring host, 2026-09-05** — this is
the evidence that the bump is both necessary and sufficient, not an
assumption:

```
$ cargo +1.95.0 install cratestack-cli --version 0.11.1 --locked   # exit 101
error: cannot install package `cratestack-cli 0.11.1`, it requires rustc 1.98.0 or newer,
while the currently active rustc version is 1.95.0
`cratestack-cli 0.8.15` supports rustc 1.95.0

$ cargo install cratestack-cli --version 0.11.1 --locked   # inside the worktree, exit 0
Installed package `cratestack-cli v0.11.1` (executable `cratestack`)
```

**What moved, and what deliberately did not.** `rust-toolchain.toml`'s
`channel`; `backends/Dockerfile`'s single `FROM rust:1.98.0-alpine3.22`
(`planner` and `builder` are both `FROM chef`, so one literal covers all
three stages and they cannot drift). **No workflow file names a compiler
version** — every Rust job in `ci.yml` and `docs.yml` already `sed`s
`channel` out of `rust-toolchain.toml`, so nothing there had to change and
the extraction was re-run by hand to confirm it yields `1.98.0`. The **Alpine
base did not move**: `rust:1.98.0-alpine3.22` exists on Docker Hub
(`docker manifest inspect`: six manifest entries, of which **three are
architectures** — `linux/amd64`, `linux/arm64/v8`, `linux/ppc64le` — and three
are `unknown/unknown` attestation manifests; the 1.95.0 tag carries exactly
the same six, and the arm64 one is what `release.yml` needs. This page and
the Dockerfile read "six architecture entries" until the review pass of the
same day counted them), so this changes the compiler and nothing else about the build
environment. `rust:1.98.0-alpine3.23` also exists and was **not** taken — an
Alpine major bump changes musl and gcc under a static build and deserves its
own evidence rather than riding along on a compiler bump. `Cargo.toml`'s
`rust-version` did not move either (see the paragraph above).

**One new clippy lint fired**, workspace-wide, over `--all-targets`:
`clippy::byte_char_slices` at `backends/crates/vpay-core/src/ids.rs:396`, on
the test that asserts the four Crockford excludes are absent from the
alphabet. Fixed by taking the lint's own suggestion — `[b'i', b'l', b'o',
b'u']` → `*b"ilou"`, the same `[u8; 4]`, loop body untouched — **not** by an
`#[allow]`, and `clippy.toml` was not touched. That was the only new
diagnostic in the whole workspace; `cargo fmt --all -- --check` was clean
under 1.98.0's rustfmt with no reformatting.

**Two stale claims were left behind by the bump itself; both are fixed** (its
sabotage review, the same day). `CLAUDE.md`'s "Things that will waste your
time" section said *"`rust-toolchain.toml` pins `1.95.0`"*; the bump had no
authorisation to edit that file and recorded it here instead, and the review —
which did — corrected the number. The bump also claimed CLAUDE.md was **"the
one place in the tree that still names the old pin as current"**, and it was
not: `justfile`'s `check-schema` rationale still said `just install-rust`
leaves the CrateStack CLI out because *"installing it needs a newer compiler
than `rust-toolchain.toml` pins"* — false since the bump, and contradicted by
the same recipe's failure message fifty lines below it, which that very commit
had rewritten. Both are corrected. Nothing in the tree now names 1.95.0 as
current; the remaining `1.95.0` strings are dated `docs/plans/*-notes/` records
and explicitly historical sentences, where they are correct.

**"Bump both together" became a gate, because the mismatch was measured to be
invisible.** `just verify-toolchain` (`cargo xtask verify-toolchain`) is the
**tenth** check in `just verify` and a step in CI's `self-checks` job since
2026-09-05: it fails when the version in `backends/Dockerfile`'s
`FROM rust:<version>-alpine…` and `rust-toolchain.toml`'s `channel` disagree.
It exists because the review pass ran that exact mutation on this branch —
`channel = "1.98.0"` with the `FROM` line left at `rust:1.95.0-alpine3.22` —
and `just verify` and `just fmt-check` both exited **0**, with no other `just
ci` recipe reading either file. The first symptom would have been a release
image built by a compiler no local run and no CI job had ever used. With the
gate, that same mutation fails `just verify` naming the file, the line and
both versions. See the "self-verification" section above for what it does and
does not cover.

**What was NOT verified by this bump:** nothing has compiled this workspace
on `aarch64`, no CI run of this change exists, and the 1.88 MSRV remains
metadata-derived and uncompiled, exactly as before. See
[plans/exp11-notes/opus.md](plans/exp11-notes/opus.md) for every command and
its output, and
[plans/exp11-notes/opus-review.md](plans/exp11-notes/opus-review.md) for the
sabotage review that re-ran all of it, its mutation table, and the two test
counts that settle whether the suite shrank (it did not: on the base this work
was written against, `046892a`, that base and this branch both listed **1220
tests in 42 binaries**, each measured under its own pin). Those two are a
matched pair on `046892a` and are left as measured. **Rebased onto `02ae5cc`
on 2026-09-05**, which brought [ADR-0016](adr/0016-engineering-standards.md)'s
`verify-serde` and `verify-repositories` and their 40 tests, the branch lists
**1270 tests in 42 binaries, 0 ignored** — 1260 on `02ae5cc` plus this
branch's ten, all in `xtask` (184 → 194). `verify-toolchain` is the **tenth**
gate after that rebase, not the eighth it was written as.

### Migration manifest — applied migrations are immutable (2026-09-07, issue #76)

**Landed.** `backends/migrations/MANIFEST.sha256` records the SHA-256 of every
migration file's bytes, and `verify-migrations` is the **eleventh** gate in
`just verify` and a step in CI's `self-checks` job. It fails when a migration
file's hash has moved, when a `.sql` file beside the manifest has no line, and
when a line names a file that is gone. `just migrations-manifest` **refuses**
to rewrite an existing line or to drop a line whose file has vanished — it only
appends — so the gate cannot be silenced by regenerating.

**Why:** `sqlx::migrate!` stores a SHA-384 of each file's *whole bytes*,
comments included, in `_sqlx_migrations.checksum`. PR #39 (the `@vpay` ->
`@vaam-apps` npm rename) reflowed one comment inside
`0028_create-checkout-sessions.sql` after it had shipped; every job in CI stayed
green, and every database brought up between #37 and #39 stopped booting
(exit 78). Nothing in the workspace could have caught it: every test starts from
an empty database and applies the current files, so the mismatch is invisible
until a *pre-existing* database meets a new binary.

**What is proved, and by what.** Twelve unit tests in `.xtask`
(`migration_manifest_tests`) pin each way the gate fails — an edited file, an
unlisted file, a deleted line, a line for a file that does not exist, a
duplicated line, a non-hex hash, and that manifest *order* does not matter.
`the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`
(`backends/tests/integration/tests/postgres_smoke.rs`) is the one that matters
operationally: it migrates a fresh `postgres:16-alpine`, rewinds migration 28's
checksum to the original file's SHA-384, confirms `sqlx::migrate!` then refuses
with the message the runbook quotes, parses the `UPDATE` **out of
`docs/runbooks/migrations.md` itself**, runs it, and confirms the migrator runs
clean afterwards.

**What this gate does NOT stop, stated because the opposite would be the
comfortable thing to write:** a contributor who edits a migration *and*
hand-edits its line in `MANIFEST.sha256` passes. Nothing can stop that —
a manifest whose own hash is checked has to pin that hash somewhere, and
whoever can edit two files can edit three. The manifest makes the edit
**visible as a reviewable one-line diff**; it does not make it impossible. It
also says nothing about a database that is already broken; that is the
runbook's §4.

**Corrected during review (2026-09-07).** The first draft of
`docs/runbooks/migrations.md` gave the repair as the SHA-384 of 0028's
**original** bytes — which is precisely what a broken database already holds.
The `UPDATE` would have reported `UPDATE 1`, changed nothing, and left the
binary exiting 78, with the page telling an on-call operator it had worked.
The correct value is the **current** file's
(`6eeb31ee…07b5ec`); the original (`f4d1a8e1…8ae252`) is now stated beside it
so an operator can tell which state their database is in, and the integration
test above is what keeps both honest. The same draft's gate hashed *every* file
in `backends/migrations/`, not just `*.sql`, which made
`backends/migrations/README.md` unaddable — the gate demanded a manifest line
for it and `just migrations-manifest` would never write one — and accepted
duplicate manifest lines silently.

### sqlx 0.8 -> 0.9 (2026-09-05)

**Landed.** `[workspace.dependencies] sqlx` is `0.9` and `Cargo.lock` resolves
**exactly one sqlx major**:

```
$ cargo metadata | (every package whose name starts with "sqlx")
sqlx 0.9.0  sqlx-core 0.9.0  sqlx-macros 0.9.0  sqlx-macros-core 0.9.0
sqlx-mysql 0.9.0  sqlx-postgres 0.9.0  sqlx-sqlite 0.9.0
```

(`sqlx-mysql`/`sqlx-sqlite` appear because `cargo metadata` enumerates
optional dependencies whether or not a feature activates them; nothing enables
them — `default-features = false` plus eight named features — and `cargo tree`
does not show them.) `cargo tree -d` lists **no sqlx version duplicate**; the
two `sqlx-core v0.9.0 (*)` lines in its output are the same version twice, the
same shape it prints for `base64 v0.22.1` and `log v0.4.33` on this workspace.

**And it is a gate now, not a sentence** (added 2026-09-05 by the review of
this branch). `deny.toml`'s `[bans]` denies `sqlx < 0.9` and `sqlx-core
< 0.9` outright. Until that entry existed, nothing in `just ci` held the
one-major property: `multiple-versions` is `warn`, so restoring
`authkestra-op`'s `sqlx-postgres` feature — one word in one manifest —
resolved 0.8.6 and 0.9.0 side by side with `cargo deny check` still printing
"bans ok" and every other recipe green. Measured, by doing exactly that:
`cargo tree -d` then listed `sqlx v0.8.6` and `sqlx v0.9.0`, `sqlx-postgres`
at both majors, and the whole gate stayed green. With the ban in place the
same mutation is `error[banned]: crate 'sqlx = 0.8.6' is explicitly banned`
and `cargo deny check bans` exits 2. Both bounds move together when the
workspace goes to 0.10, in the same commit as the pin.

**A cost the bump does carry, recorded rather than left for someone to find
in a `cargo deny` log:** sqlx 0.9 brings the RustCrypto 0.11 generation
(`sha2` 0.11.0, `digest` 0.11.3, `hmac` 0.13.0, `hkdf` 0.13.0,
`crypto-common` 0.2.2, `block-buffer` 0.12.1) alongside the 0.10 generation
the rest of the graph still uses, so `cargo deny check bans` warns about six
duplicate crates it did not warn about before — fourteen in total now, eight
before. Two independent SHA-2/HMAC implementations compile into
`vpay-server`. `multiple-versions` stays `warn`, deliberately: the duplication
is upstream's to resolve as `jsonwebtoken`/`rustls`/`sqlx` converge, and
turning it into an error would gate this repo on other people's release
schedules. Net package count went **down**, 477 -> 469.

**Why at all.** `cratestack-sqlx 0.11.1` pins `sqlx-core =0.9.0` and
`sqlx-postgres =0.9.0`. Under the old pin, a crate depending on both
CrateStack and `vpay-db` resolved two majors — two `sqlx::Transaction` types
that cannot share a transaction. See the "decisive negative" below.

**The feature list is copied across unchanged**, checked against both
releases' own manifests rather than assumed: all eight exist in 0.9.0 with the
same meaning, `tls-rustls-ring` still expands to `tls-rustls-ring-webpki`
(vendored Mozilla roots — non-negotiable for a `FROM scratch` image, ADR-0004),
and the four features 0.9 deleted (sqlx#3821: the runtime+TLS combinations)
were none of vpay's. `webpki-roots` moves 0.26.11 -> 1.0.9 and is still
`CDLA-Permissive-2.0`, which `deny.toml` already allows with its own written
reason, so that file needed no edit.

**One API change reached this workspace, and it is the only one.** sqlx#3723
made `query`/`query_as`/`query_scalar` take `impl SqlSafeStr`, implemented for
`&'static str` and for `AssertSqlSafe` and nothing else, so a `format!`-built
`String` no longer compiles as a statement. `cargo check --workspace
--all-targets` produced **36 errors, all of that one kind, all in `vpay-db`**,
and zero warnings. Everything the bump might have been expected to break did
not: `Executor`/`Acquire` bounds, `Transaction<'static, Postgres>` in
`PendingTransaction`, `PgRow`/`FromRow`, `sqlx::migrate!`, `sqlx::Error`
variants, `PgPoolOptions::acquire_timeout`/`connect_lazy`, `classify_write`.
There are no `query!` macros in this workspace and no `.sqlx/` directory, so
`cargo sqlx prepare` is **not applicable** rather than skipped.

**Two more sites that a lib-only check would not have found**, both in
`vpay-tests-integration` and both new to this pass (the earlier measurement in
`docs/plans/exp12-notes/opus.md` could not build that crate at all):
`postgres_smoke.rs`'s `SELECT COUNT(*) FROM {table}` and its
`insert_signing_key({expires_at_clause})`, each now wrapped with its own audit
comment — a table name and a SQL expression, neither of which can be a bind
parameter, both interpolating literals written in that file. And
`payment_intents.rs`'s `count(pool, sql, bind)` helper, whose `sql: &str`
became `sql: &'static str`: every caller passes a literal, so tightening the
parameter keeps the *compiler* doing the checking instead of moving it into a
comment.

**The audit is a test, not a comment.** `AssertSqlSafe`'s contract is that the
caller audited the string; a contract discharged by prose is discharged by
whoever last read the prose. All 36 statements interpolate a `const … : &str`
declared in `vpay-db` — the five per-module `COLUMNS`, `OPEN`,
`LIVE_CHARGE_STATES`, `SETTLEABLE_STATUSES`, `CLAIM_RETURNING`,
`PREVIOUS_STATE` — or `direction`, which is
`if backwards { "ASC" } else { "DESC" }`. **No caller-supplied value reaches a
statement string anywhere in the crate**; every id, cursor, limit and status is
already a bind parameter. `vpay_db::sql_audit` (test-only, 5 tests) reads the
crate's own sources and enforces exactly that, and it was **proven to fire by
three mutations**, each reverted: interpolating `{payment_intent_id}` into
`charges::get_for_intent`; redefining `direction` as something other than the
two-literal `if`; and wrapping a fresh `format!` in `AssertSqlSafe` instead of
the audited `sql` variable. The third is the one that matters — without it the
audit could be bypassed by not using the variable the audit looks at. Full
reasoning: [docs/reference/vpay-db.md § dynamic SQL strings and
sqlx 0.9](reference/vpay-db.md#dynamic-sql-strings-and-sqlx-09).
`QueryBuilder` was considered and rejected there.

**The MSRV floor moved, 1.88 -> 1.94.** `rust-version` in the root
`Cargo.toml` is derived from `cargo metadata`'s `rust_version` across the whole
resolved graph, and sqlx 0.9.0 declares `1.94.0` where 0.8.6 declared none;
the seven `sqlx-*` packages are now the sole maximum. Re-derived by
measurement over all 469 packages, 112 of which declare nothing. The toolchain
pin (`1.98.0`) is unaffected and is what this workspace is actually compiled
with; the floor has never been verified by compiling under 1.94.

**Supply chain.** `cargo deny check`: **advisories ok, bans ok, licenses ok,
sources ok**. `cargo tree -i` finds no `aws-lc-rs`, `aws-lc-sys`,
`openssl-sys` or `native-tls`. `deny.toml` was not edited. The bans warnings
grew by the RustCrypto 0.10/0.11 split sqlx 0.9 pulls in (`digest`,
`sha2`, `hmac`, `hkdf`, `block-buffer`, `crypto-common`, `cpufeatures`) —
warnings, because `multiple-versions = "warn"`, not errors. The two
`license-not-encountered` warnings (`CC0-1.0`, `MPL-2.0`) are unchanged:
measured at two before the bump and two after, by running
`cargo deny check licenses` against the pre-bump lockfile.

**The `sqlx-core` TLS claim was re-read at 0.9.0**, because that is exactly the
kind of statement about a dependency's internals a major bump invalidates. It
still holds: `handshake` selects `rustls::crypto::ring::default_provider()`
under `_tls-rustls-ring-webpki` and hands it to `builder_with_provider`, never
calling `CryptoProvider::get_default()` — so `vpay-db` still does not need to
install a process-wide provider.

**The decisive negative, re-measured on this branch.** A scratch crate
*outside* the workspace depending on `cratestack-sqlx = "0.11.1"` **and** on
`vpay-db` by path:

- with the workspace pin at **0.8**: `sqlx 0.8.6`, `sqlx-core 0.8.6` **and**
  `sqlx-core 0.9.0`, `sqlx-postgres 0.8.6` **and** `sqlx-postgres 0.9.0` — two
  majors, i.e. two incompatible `Transaction` types;
- with the pin at **0.9**: one major, and `cargo check` finishes
  (`Checking cratestack-sqlx v0.11.1 … Checking vpay-db … Finished`).

The scratch crate was deleted afterwards and, ~~on that branch, **no
CrateStack crate was added to this workspace**~~ — **superseded 2026-09-06:
twelve of them were.** See "The first CrateStack read" below. This bump is
what made that possible, and it is also why `sqlx` is now pinned `=0.9.0`
rather than `"0.9"`: `run_in_tx` accepts vpay's `Transaction` only while both
halves resolve the same `sqlx-core`, and a caret pin would let 0.9.1 break
that with a trait error nobody would read as a version problem.

~~**Reserved for the maintainer.** Whether to adopt `cratestack-sqlx` at all
now that it resolves~~ — **answered 2026-09-06: adopted, for one read.**
Asking authkestra upstream to move `authkestra-store-sqlx` to sqlx 0.9 (see
the section below) is still open.

**The gate, on the whole branch.** `just ci` **exit 0**, end to end, on this
machine on 2026-09-05:

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1277 tests across 41 binaries
     Summary [1029.968s] 1277 tests run: 1277 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1277 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
```

1277 is 1272 (the section below) plus `vpay_db::sql_audit`'s five.

**Re-measured 2026-09-05 after the sabotage review of this branch, in full:**

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1278 tests across 41 binaries
     Summary [741.013s] 1278 tests run: 1278 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1278 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
JUST_CI_EXIT=0
```

`just test-doc`: **90 passed, 0 failed, 1 ignored**, unchanged. Web: vitest
across 8 packages, all passing. Image: `docker buildx build -f
backends/Dockerfile --target server .` on a review-owned builder (removed
after), `--version` prints `vpay-server 0.1.0`, **16 MB** — the same figure
the implementer measured. **1278, not 1277:**
The extra case is `vpay_db::sql_audit`'s
`a_positional_capture_is_reported_and_is_neither_a_constant_nor_allowed`, and
the reason it exists is a finding rather than an addition: as first written,
the injection audit below could be walked straight past by spelling the
interpolation positionally. `format!("SELECT {COLUMNS} FROM charges WHERE
payment_intent_id = '{}'", payment_intent_id)` — a live SQL injection through
`AssertSqlSafe`, in the one crate that wraps it 36 times — passed all five
tests, because the scanner dropped a capture with no name and only the
`{payment_intent_id}` spelling was ever mutated. `interpolations` now reports
an unnamed capture as a violation on sight, and the mutation that was silent
now fails `every_interpolation_into_a_statement_is_a_crate_constant`. See
`docs/reference/vpay-db.md` § dynamic SQL strings and sqlx 0.9.
`just test-doc`: **90 passed, 0 failed, 1 ignored** (the ignored one is
`sdks/rust`'s README block, pre-existing). Web: vitest across 8 packages, all
passing. **An earlier attempt at this same gate failed** and is recorded
rather than dropped: `vpay-db::postgres
an_abandoned_transaction_survives_a_rollback_it_cannot_send` hit
`failed to create a container: Timeout error` after 120 s, with 35 containers
up and a load average of 28 on this shared host — a bare `docker run
postgres:16-alpine` took 19.5 s to create at that moment. Nothing else ran
(nextest cancels on the first failure), so that run is evidence about the
host, not about the code; `cargo nextest run -p vpay-db` afterwards is
**91 tests run: 91 passed, 0 skipped**, that case included.

**The image builds and runs.**
`docker buildx build -f backends/Dockerfile --target server .` on a private
builder (`vpay-exp12b-opus`, removed afterwards; the shared default builder
was never touched or pruned): **exit 0**, `docker run --rm … --version` prints
`vpay-server 0.1.0`, image **16 MB** (`FROM scratch`, musl static, ADR-0004).

**Rebased onto `8d907f9` on 2026-09-05, and the gate re-run there.** The three
transcripts above were measured on `d086084`; [PR
#44](https://github.com/vaam-apps/vpay/pull/44) landed on `master` in between
and added the `schemas/vpay.cstack` drift case to
`backends/tests/integration/tests/postgres_smoke.rs` ("The measured drift"
under CrateStack, below). One conflict, in `justfile` and in exactly one
place: both branches appended to the comment above `expected_ignored`, #44
recording 1271/42 and this branch 42 → 41. **Both blocks were kept** — each
names the base it was measured on — and the constant is 41, because #44's case
joined a test binary that already existed while this branch deleted one.
`docs/status.md` and `postgres_smoke.rs` merged without conflict and were
checked line by line rather than trusted: #44's `migrated_postgres_with_url`
and its whole drift test are intact, and so are this branch's three
`AssertSqlSafe` wrappings, its `authkestra.oauth_dpop_jti` row and its
migration-0013 column assertions.

On the rebased tree, `just ci` end to end:

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1279 tests across 41 binaries
     Summary [ 676.578s] 1279 tests run: 1279 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1279 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
JUST_CI_EXIT=0
```

Run **twice** on this tree, before and after these paragraphs were written —
the second run is the one on the commit this branch pushes, and reported the
same three numbers. The `postgres_smoke` case #44 added is in there by name
(`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`, PASS),
which is worth checking rather than inferring from the total: it shells out to
the `cratestack` CLI and *fails* rather than skipping when that binary is
absent, so a green run is evidence the tool ran.

1279 is `8d907f9`'s 1271 minus `authkestra_op_smoke.rs`'s 3, plus
`merchant_token_flow` case (i), `op::refusing_stores`' 4 and
`vpay_db::sql_audit`'s 6. `just test-doc`: **90 passed, 0 failed, 1 ignored**,
unchanged. Web: vitest across 8 packages, all passing. Both mutations were
re-run on the rebased tree and both still fail closed — re-enabling
`sqlx-postgres` on `authkestra-op` reds `cargo deny check bans`, and a
positional `{}` interpolation of a caller's value in `vpay-db` reds
`vpay_db::sql_audit`. Image: `docker buildx build -f backends/Dockerfile
--target server .` on a private builder (`vpay-exp12b-land`, removed
afterwards), `--version` prints `vpay-server 0.1.0`, image **16 MB**.

---

### The three OP stores that pinned sqlx 0.8 (2026-09-05)

**What changed.** `vpay_api::op::MerchantOp::new` filled three of
`CompositeOpStore`'s slots — `AuthorizationCodeStore`, `RefreshTokenStore`,
`DeviceCodeStore` — with
`authkestra_op::sqlx_store::SqlxOpStore<sqlx::Postgres>` over
`Repositories::op_store_pool()`. They now hold
`vpay_api::op::refusing_stores`' `RefusingAuthorizationCodeStore`,
`RefusingRefreshTokenStore` and `RefusingDeviceCodeStore`: twelve async
methods, no SQL, every one returning `Err(OpError::GrantTypeNotPermitted)`
after logging an `UnservedGrantError` that names the grant and the method.
`authkestra-op`'s `sqlx-postgres` feature came off both manifests that
enabled it (`vpay-api`, `vpay-tests-integration`), and `sqlx` moved back to
`[dev-dependencies]` in `vpay-api` — nothing under that crate's `src/`
outside a `#[cfg(test)]` module names an `sqlx` type any more.

**Why.** `SqlxOpStore` is behind a feature that pins `sqlx ^0.8`, and it was
the **only** reverse dependency of that major left in the workspace
(`cargo tree -i sqlx@0.8.6` named `authkestra-op` and nothing else). Three
slots that no request can reach were holding every crate here back from sqlx
0.9 — which is what `cratestack-sqlx 0.11.1` requires, and therefore what
stands between this repository and using CrateStack in a vpay transaction at
all. Bumping authkestra does not help: `authkestra-op` 0.8.1 (published
2026-09-05) **deletes** `src/sqlx_store.rs` and moves it to
`authkestra-store-sqlx`, which is itself still `sqlx ^0.8`. The measurement
behind all of this is `docs/plans/exp12-notes/opus.md` on branch
`claude/exp12-sqlx09-opus`; this pass is its sequel.

**Why refusing rather than implementing.** The alternative was for vpay to
own the OP's code/refresh/device storage on its own pool: roughly four
hundred lines of ported SQL, three of whose methods (`consume_code`,
`consume_token`, `consume_device_code`) must be atomic compare-and-swap or
they *are* an authorization-code replay vulnerability — written for grants
this deployment refuses at the door. Storage nothing reads is not safer than
no storage. Fail-closed is also the pattern `authkestra-op` applies to its
own optional seams (`NoClientAssertionStore`, `NoDpopReplayStore`), and
0.7.1 ships **no** such type for these three traits (checked against the
extracted crate source, not a changelog: `NoClientAssertionStore` and
`NoDpopReplayStore` are the only two, and the only other implementations of
the three traits are `SqlxOpStore`, `redis_store`, a `KvStore` blanket impl
and the crate's own test doubles).

**This is not a stub reachable from a shipping binary** (AGENTS.md rule 1).
The distinction is `Ok(None)` versus `Err`. An "always empty" store answers
`Ok(None)`, every `authkestra_op` handler renders that as `invalid_grant` —
"your code was wrong" — and the day another grant is mounted it becomes a
silent lie. These answer `Err` from all twelve methods, so the first request
on a newly mounted grant fails loudly with a message naming the grant.
`verify-no-mocks` is unchanged and green.

**Proof that they are unreachable, measured rather than argued.**
`backends/tests/integration/tests/merchant_token_flow.rs` case (i),
`the_three_grants_vpay_does_not_serve_are_refused_before_any_store`, POSTs
`authorization_code`, `refresh_token` and
`urn:ietf:params:oauth:grant-type:device_code` to a real token endpoint on a
real socket with a real freshly minted `private_key_jwt` assertion and every
field the grant's own handler would need, and asserts all three come back
**`unauthorized_client` / HTTP 400**, and specifically not `server_error`.
Every authkestra grant handler renders a store error as `server_error`, so
"the stores are unreachable" and "a merchant is never told `server_error`
here" are the same assertion. **Corrected 2026-09-05 by the review of this
branch:** this paragraph used to read "never 500 — a store error would be a
500". That is wrong, and wrong in the direction that matters, because it made
the *status* look like the proof. `op::token::token_error_status` maps
everything except `invalid_client` to **400**, `server_error` included, and
`only_invalid_client_answers_401` asserts exactly that — so this endpoint has
no 500 to emit and a store error would have arrived as a 400 as well. The
`error` code is the whole distinction; the test always asserted it, and the
prose around it was describing a different system. **The refusal shape was
measured on the unmodified tree first** (with `SqlxOpStore` still wired) and
is byte-for-byte the same afterwards; it is `unauthorized_client`, not the
`unsupported_grant_type` one might expect, because each handler's *first*
statement is `client.allows_grant_type(..)` and a merchant registration can
only ever declare `client_credentials`
(`vpay_config::ConfigError::DisallowedMerchantGrant`). The one grant `/v1`
does serve provably cannot reach a store either:
`handle_client_credentials` is not passed the `op_store` at all
(`authkestra-op-0.7.1/src/handlers/token.rs`).

**An amendment to ADR-0010 and ADR-0009, recorded here because an ADR is
immutable.** ADR-0010 describes the merchant OP's store as the composite of
a YAML client store and authkestra's SQL stores; ADR-0009 §"acceptance"
names `authkestra_op_smoke.rs` as the evidence that migrations `0006`/`0013`
match `SqlxOpStore`'s hardcoded DDL. Neither *decision* is reversed — clients
still come from YAML, `client_credentials` is still the only grant, and the
`authkestra.*` tables still exist exactly as transcribed. What changed is
that vpay no longer constructs the store those two documents assume, so the
three grant slots are fail-closed and that acceptance test is gone. Both
ADRs stand as written; this paragraph is the amendment.

**What was given up, plainly.** `authkestra_op_smoke.rs` (3 tests) was
deleted: it drove `SqlxOpStore`'s own hand-built SQL against
`0006`/`0013` — `find_client` decoding `token_endpoint_auth_method`/`jwks`,
`store_token`/`get_token` round-tripping `jkt`,
`check_and_record_dpop_jti` refusing a replay — and it could not compile
without the feature. It proved a property of a type this system no longer
uses. `postgres_smoke.rs` still asserts the four `authkestra.*` tables exist
and that `oauth_codes.client_id`'s foreign key fires — **and, added by the
review of this branch on 2026-09-05, `authkestra.oauth_dpop_jti` and the
three columns 0013 adds (`oauth_refresh_tokens.jkt`,
`oauth_clients.token_endpoint_auth_method`, `oauth_clients.jwks`).** Those
five names were the part of the deleted file that nothing else covered: after
the deletion no test in the repository mentioned any of them, so a later
migration could have dropped or renamed one with the whole gate staying
green. What is genuinely gone with the store, and is *not* replaced, is the
store-versus-schema agreement — that `SqlxOpStore`'s hand-built
`INSERT … ON CONFLICT`, `SELECT … jkt` and `find_client` JSONB decoding match
this DDL. That claim has no owner now and cannot get one while nothing
constructs the store. The four `authkestra.*` tables are now **unread and
unwritten by any vpay code path**; dropping them needs a new migration and is
left to the maintainer.
The header comments in `0006` and `0013` still cite the deleted file and were
**deliberately left wrong** — `sqlx::migrate!` checksums each migration's
whole file content, so editing an applied one turns the next boot into a
version mismatch.

**Reserved for the maintainer.** Asking authkestra upstream to move
`authkestra-store-sqlx` to sqlx 0.9 is an external issue or PR for the
maintainer to file, not for an agent. It would let a future pass put a real
SQL-backed store back in these slots if `/v1` ever mounts one of the three
grants.

**Counts on this change.** `just verify-ignored`: **1272 total, 41 test
binaries, 0 ignored** — 1270/42 on `d086084`, minus `authkestra_op_smoke`'s
3, plus `merchant_token_flow` case (i) and `op::refusing_stores`' 4 units.
`expected_suites` moved 42 → 41 in this commit, with the reasoning in the
justfile; `min_tests` stays 1080. `just test-doc`: **90 passed, 1 ignored**
(86 + the four examples on `refusing_stores`; the ignored one is
`sdks/rust`'s README block, pre-existing).

---

### CrateStack

**This section used to be a transcript. It is a gate now (2026-09-05).**
`just check-schema` runs `cratestack check --schema schemas/vpay.cstack` and
is the **seventh gate in `just verify`**, so it is in `just ci` and in CI's
`self-checks` job — which runs the recipe, not a copy of the command, for the
reason `just audit-web` and `just helm-check` are called the same way.
~~It is the only thing in this repository that reads this file: the schema is
excluded from the build graph, so no compiler has ever looked at it~~ —
**corrected 2026-09-06, see "The first CrateStack read" below: `vpay-db`
compiles this file now.** Before the gate, the evidence it parsed was
whatever transcript was last pasted here by hand.

**Pinned to `cratestack-cli 0.12.0`** (published 2026-09-06; `0.11.1`,
published 2026-09-03, until 2026-09-07). The
pin lives in exactly one place — `cratestack_version` in `justfile` — and
`.github/workflows/ci.yml` reads it back with `just --evaluate
cratestack_version` rather than repeating it, the same way every Rust job
there reads the compiler channel out of `rust-toolchain.toml`. CI installs it
with the upstream action
`cratestack/cratestack/.github/actions/install-cratestack-cli`, pinned to the
commit `v0.12.0` was tagged at (`0823bab`, `6b3053f` for v0.11.1 until
2026-09-07) rather than the `@main` its documentation shows; that action
downloads the prebuilt `x86_64-unknown-linux-gnu` binary and verifies it
against the published `.sha256` sidecar before putting it on `PATH`. The tag
is lightweight (`git/ref/tags/v0.12.0` is `"type": "commit"`), the
`action.yml` blob is byte-identical at the two commits, and the action takes
**no checksum input** — the sidecar is fetched from the release at run time,
so what pins the binary is the release rather than this workflow. Walking
those steps by hand on 2026-09-07 gave a matching digest and a binary that
reports `cratestack 0.12.0`.

~~**Installing it locally needs a compiler this repository does not pin.**~~
**Retired 2026-09-05: the repository pins that compiler now.** This paragraph
used to record that `cratestack-cli 0.11.1` declares
`rust-version = "1.98.0"` while `rust-toolchain.toml` pinned `1.95.0`, so
`cargo install` run *inside* the worktree refused with

```
error: cannot install package `cratestack-cli 0.11.1`, it requires rustc 1.98.0 or newer,
while the currently active rustc version is 1.95.0
`cratestack-cli 0.8.15` supports rustc 1.95.0
```

and that the workaround was to install from a directory outside the checkout.
That refusal is exactly what the toolchain bump above was for: `cargo install
cratestack-cli --locked --version 0.11.1` now succeeds from inside the
worktree, measured on 2026-09-05, and `just check-schema`'s failure message
was rewritten to say so instead of teaching the `cd ~` workaround. The
prebuilt binary is still there for five target triples (Linux musl still has
none), and CI still takes it rather than compiling the CLI on every
`self-checks` run — that is now a speed choice, not a constraint.
`just install-rust` deliberately does not install it.

~~`cratestack.dev/docs` 404s publicly and no authoritative reference was
found.~~ **Corrected 2026-09-05: the docs exist, and this claim only ever
described one wrong URL.** `https://cratestack.dev/docs` still 404s (checked
2026-09-05), but the documentation lives under
`https://cratestack.dev/getting-started/quickstart`,
`https://cratestack.dev/guides/*`, `https://cratestack.dev/architecture/*`,
`https://cratestack.dev/reference/*` and `https://cratestack.dev/tooling/*` —
including `https://cratestack.dev/tooling/cli-install`, which is where the
install action, the five prebuilt target triples and the sentence that Linux
**musl has no prebuilt binary yet** come from, and
`https://cratestack.dev/tooling/schema-diff` for `cratestack diff`. The
history the struck sentence belongs to still stands: `schemas/vpay.cstack`
began as an invented, Prisma-like guess and was rewritten on 2026-09-02
against the real grammar, cross-checked field by field against
`cratestack-parser`'s own test suite rather than against a docs page — which
is why it was correct anyway, and why moving the pin to 0.11.1 needed no
edit.

The gate's output on this tree:

```
$ just check-schema
check-schema: cratestack 0.12.0, schema schemas/vpay.cstack (15 model/enum declarations, datasource present)
schema OK: schemas/vpay.cstack
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.12.0
```

Independently re-run against `0.11.1`, `0.10.1`, `0.7.10` and `0.7.8` before
it — same `schema OK` line from all of them, so each release was a clean
re-verification rather than a claim inherited from an older run. **Moving the
pin from 0.10.1 to 0.11.1, and from 0.11.1 to 0.12.0 on 2026-09-07, required
no edit to `schemas/vpay.cstack`.**

**The gate is proven at 0.12.0 too, not only run at it** (2026-09-07). The
release's breaking change is `SchemaError` file identity, so the obvious
worry is a gate that no longer recognises a failure. Measured, on a copy of
the schema with one field's type deleted: `just check-schema` exits **1** and
prints `Error: expected field type` — the same line and the same exit code
0.11.1 gave for the same mutation. The recipe reads an exit code and never
parsed the diagnostic, which is why the change does not reach it.

**0.12.0 still has no `@@check(expr)`, so the two GAP comments below stand.**
Checked against the pinned crates' own sources rather than a changelog, at
0.11.1 on 2026-09-05 and again at 0.12.0 on 2026-09-07:
`grep -rn '@@check' cratestack-parser-0.12.0/src cratestack-migrate-0.12.0/src`
returns nothing, and `cratestack-migrate-0.12.0/src/convert/checks.rs` still
promotes CHECKs from `@db_enforce` on a **single field**
(`field_has_db_enforce(field: &Field)`). Those two carry the conclusion.
**A third argument was offered and is withdrawn (2026-09-05, review):**
~~`KNOWN_ATTRIBUTE_NAMES` in
`cratestack-parser-0.12.0/src/validate/misspelled_attributes.rs` — which that
module documents as the union of every attribute name the language knows —
lists no `check`.~~ That list also contains no `index`, `sql`, `paged`,
`audit` or `soft_delete`, and `schemas/vpay.cstack` uses `@@index` on two
models and passes — so absence from it does not show an attribute does not
exist. It is a typo-suggestion table whose own module doc says a missing name
is a hazard it tolerates; treating it as an inventory reads a promise into it
that it does not make. What was **not** done: enumerating
what 0.11 *added* over 0.10. The block attributes the schema's own header
lists as unused are still unused, and no attempt was made to find new grammar
this file could benefit from.

**Six mutations prove the gate fires** (2026-09-05,
`docs/plans/exp9-notes/opus.md` and `docs/plans/exp9-notes/opus-review.md`
have the transcripts): adding `tags String[]` to `PaymentIntent` fails `just
verify` with the list-arity rejection the file's own header quotes, and
`verify-docs` never runs; pointing the recipe at a schema path that does not
exist fails with `failed to read schema file` rather than passing on nothing
to check; running with the binary off `PATH` fails and prints the install
command; `|| true` on the `cratestack check` line makes the first two
mutations pass, which is what says the check is load-bearing rather than
decorative; **truncating the schema to an empty file** fails; and **deleting
its `datasource` block** fails. Each was reverted.

**The last two are the reason the recipe does not trust `schema OK` on its
own** (added 2026-09-05 by review). `cratestack check` prints `schema OK` and
exits **0** for an empty `.cstack` file, and prints `schema OK` and exits
**0** for this schema with its `datasource` block deleted *and* `tags
String[]` added — the CLI's own list-arity error names "drop the `datasource`
block" as one way to make it go away, so the mutation this gate is proven
with is exactly the one that block's absence disarms. Both are correct
behaviour for the CLI (a client-only schema is a real thing) and there is no
flag that asks for more, so `check-schema` asserts the shape of what it
checked before reporting green: a `datasource` block must be present, and the
file must still declare at least `cratestack_min_declarations` (15 since
2026-09-06: nine models, six enums — the sub-count still read "seven models"
after the floor moved 13 -> 15, which is the 13 it used to explain; 12 before
that) top-level
`model`/`enum`s. A floor rather than an exact
count, so adding a model does not fail the gate — `verify-ignored`'s
`min_tests` in miniature.

What this does and does not prove:

- **Syntax is verified, and now re-verified on every `just ci`.** Every
  scalar, attribute, relation and enum in the file parses and type-checks
  against the real CrateStack 0.12.0 grammar (0.11.1 until 2026-09-07; the
  file needed no edit to move).
- **It does not prove a working migration or a running server.** ~~The file is
  still **excluded from the build graph** — no crate depends on it, no macro
  consumes it~~ **— corrected 2026-09-06: `vpay-db` depends on it and a macro
  consumes it (below). The rest of this bullet is unchanged and is the part
  that matters:** it **drives no migration**. Nothing generates DDL from it,
  `cratestack migrate diff` has still never been run against a real vpay
  Postgres, and `backends/migrations/*.sql` remains the authoritative schema.
  `just check-schema` does not change that: it parses and type-checks the
  file and stops there, and this row stays 🟡 for exactly that reason.
  ~~Nothing diffs it against `backends/migrations/*.sql`.~~ **Corrected
  2026-09-05: something does now, and it found 86 changes — see "The measured
  drift" below.** Comparing the two is not the same as either of them driving
  the other, so the sentences above are unaffected.
- **`docs/flows/*.md` Status sections did not change, and that is checked
  rather than assumed.** The only two flow documents that mention this file —
  `docs/flows/ledger.md` and `docs/flows/configuration.md` — cite it for what
  its *grammar cannot express*, which the gate does not touch; neither
  Status section makes a claim about whether the file parses.
- **Content is a design sketch, not full coverage.** It models only entities
  with a real, tested Rust type to mirror: `Currency`, `Provider`,
  `PaymentIntent`, `Charge`, and the new `LedgerTransaction`/`LedgerEntry`
  pair. It deliberately omits `provider_requests`, webhooks/outbox, the job
  queue, idempotency keys and `Merchant` — none of those has a backing Rust
  struct yet, and the file's own `GAP` comments say so rather than inventing a
  plausible shape.
- **Two constraints this grammar cannot express now exist in raw SQL, and the
  migrations are the authoritative schema.** The file's `GAP` comments on
  `Provider` and `PaymentIntent` explain that CrateStack's `@db_enforce` only
  promotes a single-field `@range`/`@length`/`@iso4217` validator to a
  column-level CHECK — there is no `@@check(expr)` or any other cross-column
  boolean constraint, so `supports_partial_refunds ⇒ supports_refunds` and
  the over-refund guard could never be expressed in this file. Raw SQL has no
  such limitation: `backends/migrations/0002_create-providers.sql` and
  `0003_create-payment-intents.sql` implement both as real `CHECK`
  constraints, each proven to fire by a test in
  `backends/tests/integration/tests/postgres_smoke.rs` against a real
  Postgres. `Capabilities::is_coherent` in
  `backends/crates/vpay-provider/src/lib.rs` (tested by
  `vpay-provider::tests::partial_refunds_imply_refunds`) still enforces the
  first of those in Rust too — belt and braces, not a replacement for the DB
  constraint. **This file has diverged from what it mirrors**: it is still
  syntax-verified against real CrateStack 0.12.0 (by a gate, since
  2026-09-05) and still excluded from the
  build graph (below), but on these two constraints specifically it is now a
  design sketch that the migrations have moved past, not the other way
  around — see `docs/flows/configuration.md` and `docs/flows/ledger.md` for
  the full corrections.
- **A structural gap surfaced by the rewrite:** `LedgerEntry.account` mirrors
  `vpay_ledger::AccountKind`, which has exactly three variants
  (`MerchantPayable`, `PayerClearing`, `PlatformFeeRevenue`) with no
  per-merchant dimension. `docs/flows/ledger.md`'s invariant 2 — "per
  merchant: `balance(merchant_payable) = Σ captures − Σ fees − Σ refunds`" —
  cannot be computed from the modelled data, because nothing says *which*
  merchant a `merchant_payable` posting belongs to. That is a real gap in the
  Rust type this schema mirrors, not something to paper over in the schema.

#### The measured drift (2026-09-05; **101 / 16 since `model Event` and `model WebhookDelivery`, 2026-09-06**)

**`schemas/vpay.cstack` differs from the database `backends/migrations/*.sql`
builds by 101 pending drift changes across 16 tables/views** — 86 across 17
when first measured on 2026-09-05, 85 across 16 once `model DisabledClient`
landed, 84 across 16 once migration 0032 widened `currencies.exponent`.
Measured, not estimated: `cratestack migrate baseline --strict` at the pinned
CLI 0.12.0 (0.11.1 through 2026-09-06; re-derived at 0.12.0 on 2026-09-07 and
unchanged), against a `postgres:16-alpine` testcontainer with all 32
migrations applied by `sqlx::migrate!`. **It went UP for the first time on
2026-09-06 — 84 -> 101 — and that is the good direction here**, because the
17 new lines are `events` and `webhook_deliveries` being compared column by
column instead of being dismissed as one `table … is not declared` line each.
See "The outbox through CrateStack" below for the line-by-line accounting.
It is asserted by
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` in
`backends/tests/integration/tests/postgres_smoke.rs` — the exact count, the
exact sixteen relations, and the exact ten tables the report names as
present in the database and absent from the schema. The full transcripts are
in [docs/plans/exp13-notes/opus.md](plans/exp13-notes/opus.md) (the original
measurement) and [docs/plans/exp14-notes/opus.md](plans/exp14-notes/opus.md)
(the move).

**Why it moved, and the near-miss worth recording.** `model DisabledClient`
landed on 2026-09-06 for the existing `disabled_clients` table, and that
table left the report entirely — one `table … is not declared in the schema`
line went away with nothing replacing it. That outcome depended on a single
attribute. With `disabled_at DateTime @default(dbgenerated())`, `migrate
baseline` reads the live default as `ColumnDefault::Function("now()")` and
the schema's as `ColumnDefault::DbGenerated`, which never compare equal — so
the missing-table line is *swapped* for a `column disabled_at default value
differs` line and **the total stays at exactly 86**. A whole table entering
the schema would have been invisible to the count. `@default(now())`
converts to the same `Function("now()")` and compares clean. Both spellings
were run; the exact-set assertion beside the count is what would have caught
the first, and is the reason that assertion exists.

**85 -> 84 on 2026-09-06, and two of migration 0032's three changes moved
nothing.** This is the entry worth reading before planning the next table,
because the intuition it corrects is the obvious one.

| Migration 0032 change | Drift effect |
|---|---|
| `currencies.exponent` `INT` -> `BIGINT` | **-1 change, and -1 in the "could not confidently map" block (18 -> 17)** |
| The two hand-named `currencies` CHECKs renamed to `<table>_<column>_<validator>_check` | **0** |
| `providers.flow` native enum -> `TEXT` + `providers_flow_enum_check` | **0** |

The widening is the whole of the -1, and it is the *good* kind: the column was
excluded from the comparison entirely (`Int` emits `int8` and the introspector
deliberately refuses to map `int4` back onto it), so the report is now
comparing **more** and finding no drift on what it gained.

The **rename moved nothing** because introspection reports every
validator-derived CHECK as `CheckKind::Raw(<deparsed text>)` and reconstructs
only `CheckKind::Enum` — never `Iso4217`, `Range` or `Length`. `diff/checks.rs`
matches by name and then compares kinds, so a matching name turns two
unrelated lines into a same-named drop-and-add pair: a clearer report, the
same number. The rename is still right (the database now carries the name a
generated `migrate diff` would emit DDL against, and doing it later means
doing it on a table with rows), but **do not expect a CHECK rename to move
this constant.**

The **enum conversion moved nothing, and the report is structurally blind to
it** — the mirror image of the multi-column-CHECK finding below.
`introspect/postgres/enums.rs` already synthesised `providers_flow_enum_check`
from `pg_enum` for the native column, and `resolve_column` projects a native
enum and a `TEXT` column onto the same `Scalar("String")`, so `providers`
reports the identical four lines before and after. The conversion is real and
load-bearing all the same: CrateStack's generated row decoders read an enum
column with `try_get::<String>()`, so a native enum column fails to decode on
**every** read through that layer, and no CrateStack query could have touched
`providers` before it. What proves it worked is
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` in
`vpay-db`'s own tests, and reverting the `ALTER COLUMN flow TYPE TEXT` is what
makes that test red — measured, in
[docs/plans/exp17-notes/opus.md](plans/exp17-notes/opus.md). One of the four
`providers` lines, `column flow type differs (live: Scalar("String"), schema:
Enum("ProviderFlow"))`, is **permanent at 0.12.0**: the enum's *name* has no
catalog representation to recover it from, which `enums.rs`'s own doc comment
calls documented lossiness. Every enum-typed column in the schema carries one.

Until this test, "content remains a design sketch" was a sentence written from
reading both files, and nothing ran that could have contradicted it. **The
number must move when the schema grows**, and it must never be asserted as 0:
this file still drives no migration, so a 0 would mean the report stopped
finding things rather than that the gap closed. `--strict` writes nothing —
the out-dir is outside the checkout and is asserted empty, no
`cratestack_migrations` row is recorded, and the schema file is byte-identical
afterwards.

**The two cross-column CHECKs do not appear in the drift report at all.** This
is the finding, and it strengthens the `@@check(expr)` ask above rather than
merely illustrating it. Every CHECK `migrate baseline` reports is a
single-column one; read out of `pg_constraint`, the live database has **ten**
multi-column CHECKs — including `providers.partial_refunds_imply_refunds` and
`payment_intents.no_over_refund` — and **none of the ten reaches the report**,
in either direction. Both of those two sit on tables the schema *does* model,
and single-column CHECKs on those same tables are reported one line away, so
the absence is not the tables being skipped. The consequence worth writing
down: `--strict`'s documented purpose is proving in CI that a database already
matches the schema, and **on this database a green `--strict` run would say
nothing whatever about the over-refund guard or the refund-capability rule** —
the two constraints raw SQL added *because* the grammar could not express them
are exactly the two the drift tool cannot vouch for. Deleting `CONSTRAINT
no_over_refund` from migration 0003 leaves the count where it was, measured;
the test
catches it by reading `pg_constraint` directly, and
`over_refund_is_rejected_by_the_database` catches it behaviourally.

**It is CrateStack's own documented gap, not an inference from ten samples**
(added 2026-09-05 by review, from the pinned crate's sources rather than from
the measurement alone). `cratestack-migrate-0.12.0/src/introspect/postgres/
mod.rs` lists it under "Known gaps": *"Multi-column and zero-column CHECK
constraints are skipped. `crate::ir::AddCheck` ties to exactly one column —
there's no IR shape for `CHECK (a < b)` — so `constraints::introspect_checks`
only considers `contype = 'c'` rows with `array_length(conkey, 1) = 1`;
anything else is silently absent from the result rather than mis-attributed to
one of its columns."* The query in `constraints.rs` carries that filter
verbatim. Three things follow that the black-box measurement could not have
told us:

- **The ask is bigger than `@@check(expr)`.** Grammar alone would not fix
  this: the IR has no shape a cross-column CHECK could occupy, so an
  `@@check(expr)` that parsed would still be invisible to `migrate baseline`
  and `migrate diff`. The ask to CrateStack is `@@check(expr)` **and** an IR
  op that is not tied to a single column **and** introspection that reads
  `array_length(conkey, 1) > 1`.
- **The skip is deliberate and defensible**, which is worth saying plainly:
  mis-attributing `CHECK (a < b)` to column `a` would be worse. The defect is
  that a `--strict` run reports success without saying what it could not
  look at — the report's trailing "N column(s) … review manually" block has
  no CHECK equivalent.
- **Zero-column CHECKs are skipped too**, and the test's `pg_constraint`
  query (`cardinality(conkey) > 1`) does not enumerate them. Measured on this
  database: there are none on a `public` table — the only two,
  `cardinal_number_domain_check` and `yes_or_no_check`, are
  `information_schema` domain constraints — so nothing is missed today, and a
  future `CHECK (current_setting(…) = 'x')`-shaped constraint would be
  invisible to both the tool and that assertion.

Two further blind spots, both pinned by the same test so they cannot move
unnoticed: **18 columns are excluded from the comparison entirely** (`jsonb`,
`int2`/`int4`, `bytea` — the report says so itself and asks for a manual
review), and the `authkestra.*` tables are never introspected, because baseline
reads the connection's own schema. `oauth_signing_keys` and
`oauth_client_assertion_jtis` are **not** in that second category — they are
`public` tables, so the schema header's "and the authkestra tables" does not
account for them, and the measured set of undeclared tables is larger than the
header claims. Measuring rather than copying that list is what surfaced it.
`disabled_clients` was a third until 2026-09-06, when it became the first
table this file models *and* `vpay-db` reads through CrateStack.

What was **not** done, as of the 2026-09-05 measurement: `cratestack migrate
diff` was still never run, no snapshot was ever written, nothing was
reconciled, and none of the 86 changes was closed. **One of them has been
closed since** — `disabled_clients` — and `migrate diff` is still never run,
no snapshot is still ever written, and the remaining 85 stand.

#### The first CrateStack read (2026-09-06)

Three things landed together, and each has a mutation that turns a gate red.

**1. The dependency.** `cratestack = { package = "cratestack-pg", version =
"=0.12.0", default-features = false, features = ["postgres"] }` in
`[workspace.dependencies]`, taken by `vpay-db` and by no other crate. The
rename is forced: the schema macros emit absolute `::cratestack::*` paths and
cannot be told otherwise. The exact pin is forced too, by three other places
that already name the same version — `justfile`'s `cratestack_version`, the
drift measurement, and `rust-toolchain.toml`'s 1.98.0 (which exists *because*
every crate of the pinned release declares `rust-version = "1.98.0"`; 0.11.1
when this landed, 0.12.0 since 2026-09-07, and the floor is the same at
both). The library and the CLI
must answer about one grammar.

`Cargo.lock` goes **469 → 497 packages (+28)**; `syn` moves 3.0.3 → 3.0.5.
Twelve of the twenty-eight are `cratestack-*`, all MIT. The rest:
`ar_archive_writer` (Apache-2.0 WITH LLVM-exception), `ariadne`, `chumsky`
(MIT), `const-oid`, `erased-serde`, `hashbrown`, `object`, `psm`, `stacker`,
`typeid`, `unicode-segmentation`, `unicode-width`, `wasm-streams` (MIT OR
Apache-2.0), `foldhash` (Zlib) — and **`minicbor` + `minicbor-serde`
(BlueOak-1.0.0)**, which is the whole reason for the licence exception below.
`cargo tree -i aws-lc-rs` is still empty and `cargo tree -d` still shows
exactly one `sqlx` (0.9.0). **Corrected 2026-09-06 by review:** +28 counts
`Cargo.lock` *entries*, and only **25 are new crate names** — `const-oid`,
`foldhash` and `hashbrown` are extra *versions* of crates already in the
graph, which is also why the new version duplicates are three and not two:
`const-oid` (0.9.6/0.10.2), `foldhash` (0.1.5/0.2.0) **and `hashbrown`**
(three versions to four). All under `multiple-versions = "warn"`, and
`cargo deny check bans` is green.

**The MSRV floor moved 1.94 → 1.98** as a direct consequence: the twelve
`cratestack-*` packages are now the sole maximum over the graph's declared
`rust_version` fields, where the seven `sqlx-*` were. It is still a metadata
floor and still not verified by compiling at it; that it now equals the
toolchain pin is a coincidence of one release, not a policy change.

**2. The licence exception.** `deny.toml` gains a **scoped** exception —
`minicbor` and `minicbor-serde` only, by name, never an entry on the `allow`
list. Blue Oak 1.0.0 is OSI-approved and permissive with an express patent
grant, so it clears the bar that list's comment sets; it is scoped because a
*third* Blue Oak crate arriving is a new fact that should fail the gate.

It is not avoidable. `cratestack-pg` declares `cratestack-axum` and
`cratestack-client-rust` non-optional with no feature gating either, and both
take `minicbor` unconditionally; `default-features = false, features =
["postgres"]` — the smallest set that still yields a data layer — does not
remove them. Dropping to `cratestack-sqlx` + `cratestack-macros` directly does
not help either: `include_server_schema!` emits `pub mod axum { use
::cratestack::HttpTransport; … }` unconditionally.

- **Decisive test:** delete the exception and `cargo deny check licenses`
  **FAILS**, naming `minicbor` and `minicbor-serde` at
  `license = "BlueOak-1.0.0"`. With it, `cargo deny check` reports
  `advisories ok, bans ok, licenses ok, sources ok`. **No other new licence
  or ban appeared** — that was checked before the exception was written, not
  assumed.

**3. One read, and only one.** `DisabledClients::is_client_disabled` — the
OAuth kill-switch lookup — is now
`self.cs.disabled_client().find_unique(id).run(&system_context())`. Nothing
about the trait's public surface changed. `schemas/vpay.cstack` gains `model
DisabledClient` with `@@allow("read", auth().isSystem())` and nothing else;
`cratestack_min_declarations` goes 12 → 13.

**What did NOT move, stated plainly:** ~~the two `disabled_clients` *writes*
are still raw sqlx~~ **— superseded later the same day; see "The first
CrateStack writes" below** — no other table moved, no transaction, no enum
column, and nothing about the transport — the generated `pub mod axum`
compiles and is never referenced. `backends/migrations/*.sql` is still the
authoritative schema and still drives every migration.

- **Decisive test (the policy trap):** delete `@@allow("read",
  auth().isSystem())` from `model DisabledClient` and
  `a_disabled_client_reads_the_same_through_both_paths` in
  `backends/crates/vpay-db/tests/repositories.rs` **FAILS** — the CrateStack
  read returns `None` for a row a direct `SELECT` finds. This is the failure
  mode the whole adoption most has to be unable to make silently: model
  policies are compiled into the `WHERE` clause and a model with no `@@allow`
  is deny-by-default, so a policy mistake does not raise an error, it turns
  the kill-switch off. `just check-schema` stays green through it, and so
  does every other gate. **Run on 2026-09-06, not merely designed:** the
  mutation was applied and the test failed with `CrateStack says false, sqlx
  says true`; the schema was restored afterwards and the tree is clean.
- **Decisive test (the gate):** make `mod schema;` public — or add `pub use
  schema::cratestack_schema;` — and `cargo xtask verify-repositories`
  **FAILS**. **Measured against the gate as it stood before this change:
  both spellings printed `ok`.** The module `include_server_schema!` creates
  does not exist in any source file, so neither of the gate's two original
  signals, nor rustdoc, nor any lint could see it. `DB_HANDLE_TYPES` also
  learned `Cratestack` and `SqlxRuntime`, so a store holding the generated
  runtime instead of a `PgPool` is recognised as an implementation;
  whole-identifier matching keeps `CratestackError` and `CratestackContext`
  out, asserted rather than assumed.

  **Extended 2026-09-06 by review, after two more spellings were measured
  past it.** `pub type CratestackHandle = crate::schema::cratestack_schema::Cratestack;`
  and a `pub fn` returning the same type both compiled and both printed
  `ok`: the check read `pub mod` and `pub use` and nothing else, and
  `concrete_repository_types`' alias fixpoint cannot help, because it only
  promotes an alias whose target is already in the set and `Cratestack` is
  declared by no source file. The gate now also fails any unrestricted-`pub`
  item *signature* naming the module, anywhere in the crate;
  `PgRepositories`' real `pub(crate) cs` field and `boxed`'s body still pass,
  which is asserted rather than hoped for. `cargo nextest run -p xtask`
  197 → 198.

**Errors.** `PersistenceError` (`vpay-db/src/persistence.rs`) is the leaf,
`DbError::Persistence` `#[from]`s it and delegates, and `classify_cratestack`
is the single place a `CratestackError` is read — the mirror of
`classify_write`, branching on the same SQLSTATEs. CrateStack's own
`status_code()` is **not used**: its `DatabaseTyped` is a 500 with `"internal
error"`, which would answer a duplicate charge as an outage. Two honest
limits, both in the code's own doc comments and in
[docs/reference/vpay-db.md](reference/vpay-db.md): a CrateStack *read* never
carries a SQLSTATE at all (`FindUnique::run` stringifies its `sqlx::Error`),
so today every failure of this one query lands on `Storage` — the same answer
the `SELECT` it replaced gave; and a policy denial cannot be produced by the
read path either, because a refused read is a `WHERE` clause rather than an
error. The SQLSTATE and `Denied` arms are unit-tested, not exercised.

**Stays `NotImplemented`: nothing.** No function gained a stub, no test was
weakened, and every method touched already worked and still works.

~~**Three container-backed cases on this branch have NEVER been executed, and
are owed to CI.**~~ **Superseded 2026-09-06: the host's Docker daemon came
back and all three were run.** `just ci` ran **end to end, exit 0**, on this
branch rebased onto master at `6978901` (#50, `refunds.fee`): `just fmt-check`,
`just clippy`, all **ten** `just verify` gates, `just test-rust` **1369 tests
run, 1369 passed, 0 skipped** across 43 binaries in 722.977 s, `just test-doc`
**96 passed, 1 ignored**, `just verify-ignored` **0 ignored (expected 0), 43
test binaries (expected 43), 1369 total**, `just lint-web`, `just test-web`
(1369 is master's 1359 plus this branch's ten tests; this branch still adds no
test binary, so `expected_suites` stays 43 and `min_tests` stays 1080), and
`just deny` `advisories ok, bans ok, licenses ok, sources ok`. The three cases
by name, all **PASS**:

1. `vpay-db::repositories a_disabled_client_reads_the_same_through_both_paths`
   — the parity test. **Executed for the first time on 2026-09-06: PASS**,
   6.371 s standalone and 1.627 s inside the full run. "The CrateStack read
   returns what the sqlx read returns" is now a measurement rather than a
   reading of the generated query builder.
2. The decisive mutation on it — deleting `@@allow("read", auth().isSystem())`
   from `model DisabledClient` — **was run, and the parity test FAILED** with
   `CrateStack says false, sqlx says true`, i.e. the kill-switch silently OFF,
   while `just check-schema` stayed green. The schema was restored. The
   transcript is in [plans/exp14-notes/opus.md](plans/exp14-notes/opus.md) §8.
3. `just test-rust` itself — run, not merely listed, including the
   pre-existing `disabled_client_lookup_reflects_disable_and_enable` that
   exercises the changed method (PASS, 1.274 s), `client_store`'s
   `find_client_reflects_the_disabled_clients_kill_switch` (PASS), and all ten
   `merchant_token_flow` cases.

The drift test was re-run on the final rebased tree: **PASS**, and its
constants do not move — still **85 changes over 16 relations**, 18 unmappable
columns. Migration `0031` adds `refunds.fee` to a table `schemas/vpay.cstack`
does not declare at all, and an undeclared table is one report line whatever
its column count.

~~The server image was not built, so **the size cost of CrateStack's graph on
the static musl link is unmeasured**.~~ **Built 2026-09-06, and it found a
defect this branch had introduced:** `backends/Dockerfile` never copied
`schemas/` into the build context, so `include_server_schema!` failed at macro
expansion with `failed to read schema file .../schemas/vpay.cstack: No such
file or directory` and **the release image could not be built at all**. No
gate in this repository would have caught it — `just ci` builds on the host,
where the file is present. Fixed by adding `COPY schemas ./schemas` to both
the `planner` and `builder` stages. The size cost, measured paired on one host
and builder on 2026-09-06 (master rebuilt from a `git archive` of `6978901`
rather than compared against an older quoted figure): master **16.1 MB**, this
branch **16.9 MB** — **+0.8 MB (+5.0%)** for CrateStack's twelve crates plus
`minicbor`, `chumsky` and `ariadne`. Both images run and print `vpay-server
0.1.0`.
`docs/plans/exp14-notes/opus.md` § 7 has the full list.

#### The outbox through CrateStack, and the transaction seam (2026-09-06)

**Two of the two-step outbox's three writes now run through CrateStack's
`run_in_tx` on a transaction `UnitOfWork::transaction` opened and `TxOutcome`
closes.** This is the first time a CrateStack write has joined a transaction
`vpay-db` did not open for CrateStack's own benefit — `reconcile`'s
transaction contained nothing else, and the fan-out's contains hand-written
`jobs` inserts beside the CrateStack ones.

| `TxRepositories` method | Now | Policy slots |
|---|---|---|
| `create_in_tx` | `WebhookDelivery.upsert(..).do_nothing().on_conflict(&["event_id","endpoint_id"]).run_in_tx(tx, ctx)` | `create` **and** `update` |
| `mark_fanned_out_in_tx` | `Event.update_many().where_(id).where_(fanout_state).set(..).run_in_tx(tx, ctx)` | `update` |
| `insert_in_tx` | **unchanged, raw `sqlx`** — and pinned as blocked | — |

`models Event` and `WebhookDelivery` are new in `schemas/vpay.cstack`, so
`events` and `webhook_deliveries` are the second and third tables ever to
leave the report's "present in the database, absent from the schema" list.
**`jobs` did not move and is not next**: `Jobs::claim` needs `FOR UPDATE SKIP
LOCKED` and `FindMany::for_update()` emits a bare `FOR UPDATE`. There is **no
migration**; the migration count stays at 32.

**What did not move, and why it is a measurement rather than an excuse.**
`insert_in_tx` writes `events.data`, which is `JSONB NOT NULL` with no
`DEFAULT`. CrateStack 0.12.0 *does* have a `Json` scalar — so "it cannot model
JSONB" would be false — but declaring `data Json` costs two things that were
measured rather than argued: it adds a `[blocking]` drift line (because
`map_scalar` does not read `jsonb` back, so the live column stays invisible
while the declared one does not), and it routes the payload through
`cratestack::Value`, whose `from_plain_json` falls back to `f64` for any
number that is not an `i64`. `events.data` is the object that is **signed and
delivered** to a merchant and it carries merchant-authored `metadata`. So the
insert stays one hand-written statement in the same transaction, and two
tests pin the blocker. **What they pin, corrected by the review:** both render
or run the statement the *current* model generates, so they go red when
somebody declares `data Json` — not when CrateStack is fixed. `map_scalar`
lives in `cratestack-migrate`, which is not in `vpay-db`'s compiled graph, so
no Rust test here can see it change; the tripwire for that half is
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`'s
`EXPECTED_UNMAPPABLE_COLUMNS = 17`, which `events.data` would leave. The two
pins are:
`the_events_insert_cannot_move_until_a_json_column_can_be_modelled` (no
database, pins the rendered five-column INSERT) and
`a_generated_events_insert_is_refused_by_the_not_null_on_data` (runs it and
asserts `23502`).

**A second blocker, closed rather than reserved.**
`webhook_deliveries.id` carries `DEFAULT gen_random_uuid()`, and a
`@default(...)` on a primary key makes `generate_upsert_input_struct` emit
nothing — `.upsert(..)` is a compile error, exactly as the five `@default`
booleans block `model Provider`'s upsert. Unlike `Provider`, this one needed
no DDL to unblock: the model simply does not declare the default and
`vpay-db` mints the `Uuid` itself. The database default is untouched and now
vestigial. It costs one drift line, and `.upsert(..).do_nothing()` is not
optional — a bare `INSERT`'s `23505` aborts the enclosing transaction, so it
could not be turned back into the `Ok(None)` the at-least-once drain needs.

**The drift arithmetic, 84 -> 101 over the same 16 relations, 17 unmappable
columns unmoved.** Nineteen `[safe] CHECK/index … exists in the live database
but is not declared` lines arrive, plus two `default value differs`
(`events.seq` is `GENERATED ALWAYS AS IDENTITY`, which has no `pg_attrdef`
default to introspect; `webhook_deliveries.id` above), and two
`table … is not declared` lines leave. Declaring the CHECKs with
`@db_enforce` would make it **worse** — the generated name is
`<table>_<column>_<validator>_check` and the live names are hand-written, so
each would become a drop-and-add *pair*. The four `jsonb`/`int4` columns cost
zero lines in either direction, which is why `EXPECTED_UNMAPPABLE_COLUMNS`
does not move; four of those seventeen now sit inside tables the schema
models fully, which is unmeasured drift hiding inside a compared table and is
recorded on the constant.

**A gate hole this change had to close before it could claim anything.**
`events.type_is_a_documented_event` is cited by migration `0018`'s own
comment, by `docs/api/README.md`, by `docs/flows/webhooks.md` and by three
Rust doc comments — and **nothing asserted it fired.**
`an_undocumented_event_type_is_refused_by_the_database` is new in
`postgres_smoke.rs`. It matters more now: `model Event` cannot express a
multi-value single-column CHECK at 0.12.0 (a `.cstack` enum would match only
under the generated name `events_type_enum_check`, and `diff/checks.rs`
matches by name first), so the constraint stays undeclared.

**Corrected 2026-09-06 by the review.** This paragraph first claimed that
deleting the constraint "takes the drift count from 101 to 100 and fails no
drift assertion at all", and that only the new test notices. The count half
reproduces — losing a `[safe] ... is not declared` line lowers the number —
but the conclusion was wrong: `EXPECTED_DRIFT_CHANGES` is an exact
`assert_eq!`, so the drift test fails on 100 just as loudly as on 102, and
says so ("If it shrank without an edit to the schema, find out what the report
stopped seeing before moving anything"). Two signals, and they say different
things: the count says *a constraint is gone*, the new test says *the
vocabulary is still closed* — the property `docs/flows/webhooks.md` and
`docs/api/README.md` actually rest on, and the one that survives someone
re-pinning the constant. Measured both ways;
[plans/exp18-notes/opus-review.md](plans/exp18-notes/opus-review.md) F1 has
the transcript.

**One contract narrowed, found by the review and pinned rather than papered
over.** `create_in_tx` answers `Ok(None)` for a repeat creation of one
`(event, endpoint)` pair — the quiet answer the at-least-once drain needs.
Through CrateStack that holds only when the earlier row was **committed**: a
second call inside the *same, still-open* transaction is refused with
`PersistenceError::Denied`, where the raw `INSERT … ON CONFLICT DO NOTHING`
it replaced answered `None`. `.do_nothing()` probes for the existing row
through the caller's transaction and then re-checks the update policy on a
**pool** connection, which cannot see it. Unreachable in vpay — a duplicate
endpoint `id` is refused at boot by `vpay_config` and deduped again by
`EndpointRegistry` — but the fan-out's correctness now rests on those two
guards in a way it did not before, so both ends say so and
`a_repeat_creation_inside_one_transaction_is_refused_rather_than_reported_missing`
asserts both halves. Reported upstream, not worked around.

**`PersistenceError::Invalid` is new.** The generated `input.validate()` runs
before any SQL on every write path, and `CratestackError::Validation` was
falling into the `Backend` wildcard — `Category::Storage`, which is
**retryable**. A 65-character endpoint id is exactly as long on every retry.
It is `Category::Internal` / `Retry::Never` now, with the same `code` a CHECK
violation publishes so a merchant cannot tell which of the two layers
refused.

**Gate, re-run on the reviewed head on 2026-09-06 against the pinned 1.98.0
toolchain.** `just ci` end to end, exit 0, with containers and with
`node_modules` installed from the pinned lockfile under the `.nvmrc` Node
(22.23.2). All **ten** `just verify` gates green — `check-schema` reports
**15** model/enum declarations and `cratestack_min_declarations` is raised
13 -> 15 in the same commit, per that floor's own rule. `just verify-ignored`:
**0 ignored (expected 0), 43 test binaries (expected 43), 1390 total** —
master's 1382 plus this change's eight, every one in a file that already
existed, so `expected_suites` stays 43 and the 1080 floor is untouched.
Recipe by recipe, each run on its own so a failure could be attributed:
`fmt-check` 0, `clippy` 0 (13 s), `verify` 0 (8 s), `test-rust` 0
(**1536 s — 1390 tests run, 1390 passed, 0 skipped**), `test-doc` 0 (**96
passed, 1 ignored** — the ignored one is `sdks/rust`'s README block and is
pre-existing), `verify-ignored` 0, `lint-web` 0 (20 s), `test-web` 0 (8 s),
`deny` 0 (`advisories ok, bans ok, licenses ok, sources ok`).

`test-rust` needed a second run, and the reason is recorded rather than
smoothed over: the first re-run failed
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` at
exactly 120.007 s with `failed to create a container: Timeout error` — the
same container-start contention described below, in a test this change does
not touch, on a host running two other container suites. It passes 3/3 in
isolation (117 s / 1.2 s / 1.3 s) and passed in both clean full runs.

The eight: two `vpay-db` unit tests in `webhook_deliveries` (the `preview_sql`
pin on both outbox statements, and the policy-slot assertion), one in `events`
(the JSON blocker pin), four container cases in
`vpay-db/tests/repositories.rs` (the abandon test, its commit control, the
`23502` proof, and the review's
`a_repeat_creation_inside_one_transaction_is_refused_rather_than_reported_missing`),
and one in `postgres_smoke.rs` (the event vocabulary).

**Two flakes were observed and are not this change's**, recorded because
"green on the third run" is worth saying out loud. Under a host load average
of 40-90 (an unrelated container stack cycling on the same machine), one run
failed `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx`
at exactly 120.006 s and another failed
`provider_callback::a_callback_settles_the_charge_…::case_2_orange_money` at
259.9 s. Both are container-start contention, which `.config/nextest.toml`'s
own header documents at length as the reason that file exists; both pass in
isolation (1.5 s and 65 s for the whole suite) and both passed in the clean
run above. Neither touches a line this change wrote.

**Six mutations by the implementer and five more by the review, every one
applied, run and reverted**
([plans/exp18-notes/opus-review.md](plans/exp18-notes/opus-review.md) has the
review's). The implementer's six (transcripts in
[plans/exp18-notes/opus.md](plans/exp18-notes/opus.md)): both `run_in_tx ->
run` swaps make the abandon test red in about a second and **neither hangs**,
unlike the currency upsert's equivalent; the three `@@allow` deletions are
caught with no container in milliseconds and their runtime effects differ
exactly as documented (delivery `create` LOUD everywhere, delivery `update`
LOUD only on a re-run — the crash-recovery path, event `update` SILENT and
total); and dropping the events CHECK behaves as described above.

The review's five, run independently: bypassing `mark_fanned_out_in_tx`
entirely makes `fan_out_creates_one_delivery_and_one_job_per_endpoint_and_is_idempotent`
red on `fanout_state` (`"pending"` vs `"done"`); deleting `model
WebhookDelivery`'s `@@allow("update")` is caught in 4 ms with no container,
**and** — with that no-container assertion also deleted —
`a_second_delivery_for_one_event_and_endpoint_is_not_created` still catches
it, so the two are defence in depth rather than one gate twice; dropping the
events CHECK fails **both** the vocabulary test and the drift assertion
(which is the correction above); and mapping `create_in_tx`'s policy denial
back to `Ok(None)` — the plausible "fix" for the narrowed contract — makes
the new pinning test red in 1.3 s.

#### The first CrateStack writes (2026-09-06)

`DisabledClients::disable_client` is `.upsert(CreateDisabledClientInput).run(&system_context())`
and `DisabledClients::enable_client` is
`.delete_many().where_(client_id.eq(..)).run(&system_context())`. That is the
whole of one table's repository — read and both writes — through the generated
data layer, and still the only table in the workspace that is. `model
DisabledClient` gains `@@allow` arms for `create`, `update` and `delete`
alongside the `read` it already had; `cratestack_min_declarations` does not
move (13), the drift constants do not move (**85 changes over 16 relations,
18 unmappable columns**, re-run and passing), and there is **no migration**.

**The trait surface is unchanged and the `# Errors` contract is not.** Both
writes now return `DbError::Persistence` where they returned `DbError::Query`,
the same correction the read took hours earlier. A caller branching on the
classification sees nothing — `classify_cratestack` and `classify_write` are
asserted against each other in `persistence.rs` — and a caller matching the
*variant* would have silently stopped matching, which is why both trait doc
comments say so in `# Errors`.

**What did NOT move:** every other table, every transaction (`UnitOfWork` is
untouched and no CrateStack `run_in_tx` is called anywhere), every enum
column, and the transport. `backends/migrations/*.sql` is still the
authoritative schema.

**Two deviations from the obvious shape, both measured rather than
preferred:**

1. **`enable_client` is `delete_many`, not `delete(pk)`.** `cratestack-sqlx`
   0.12.0's `query/write/delete_exec.rs` reports a `DELETE … RETURNING` that
   matched no row as `CratestackError::Forbidden("delete policy denied this
   operation")`, and with no `@version` column on this model it has no way to
   tell that from a real policy refusal. `.delete()` would therefore turn
   every re-enable of an already-enabled client into a `Category::Internal`
   error, breaking this trait's documented "a no-op, not an error" contract
   and two existing tests. The cost of the alternative is recorded in the
   table below: `delete_many`'s policy lives in the `WHERE`, so a missing
   `delete` policy is silent.
2. **`disable_client` costs a second round trip.** `.upsert()` is
   transactional by construction — it probes the conflict target with `SELECT
   … FOR UPDATE` and may run an `ON CONFLICT DO NOTHING` first — to tell a
   create from an update for an event/audit fan-out this model has neither of.
   Accepted: disabling a client is an operator action, not a hot path.
   `is_client_disabled` is on the token path and stayed a single
   `find_unique`.

**The rendered upsert is the statement it replaced, byte for byte** —
`INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2) ON CONFLICT
(client_id) DO UPDATE SET reason = EXCLUDED.reason` plus a `RETURNING` nobody
reads. `disabled_at` is in neither the insert list nor the `SET` list, because
`cratestack-macros` excludes every `@default(...)` column from both, which is
what keeps "a second disable leaves the original `disabled_at` untouched"
true. Two new unit tests in `disabled_clients.rs` assert that rendering
without a database; their value is as a guard on a **pinned external crate's**
output, since a schema edit that removed the default is caught earlier by the
compiler (`error[E0063]: missing field \`disabled_at\``, measured).

**Decisive tests — six mutations, all run against a real Postgres, none
merely designed.** The most useful result is that the three write actions do
not behave alike:

| Mutation | Runtime effect | What went red |
|---|---|---|
| drop `@@allow("create", …)` | `disable_client` **errors**: `Forbidden` → `PersistenceError::Denied` → `Category::Internal` | `a_client_disabled_through_cratestack_is_visible_to_both_paths`, at the **first** disable |
| drop `@@allow("update", …)` | `disable_client` **errors on the conflict branch only** — the first disable of a client succeeds | the same test, at the **second** disable, plus `disabled_client_lookup_reflects_disable_and_enable`. A test that disabled once and stopped would have passed |
| drop `@@allow("delete", …)` | `enable_client` removes nothing and returns **`Ok`** — silent | the same test's enable assertion, which reads the row back through a plain `SELECT`; plus the lookup test |
| drop `@@allow("read", …)` | kill switch silently OFF (unchanged from the read change) | `a_disabled_client_reads_the_same_through_both_paths`, `CrateStack says false, sqlx says true` — **re-verified after its seed changed** |
| replace the `upsert` call with `Ok(())` | disable does nothing | `find_client_reflects_the_disabled_clients_kill_switch` (integration): "a disabled client must stop resolving immediately" |
| replace `delete_many` with an `update_many` that sets `reason` | enable updates instead of deleting | the enable assertion, on the row still being there |

So: **`create` and `update` fail loudly, `read` and `delete` fail silently.**
`upsert_exec.rs` evaluates the create policy in Rust before it builds any SQL
and `upsert_resolve.rs` does the same for the conflict branch, so an empty
allow list is a real error there; the read and `delete_many` paths compile the
policy into a `WHERE` clause where an empty allow list renders `FALSE`. The
two silent holes fail in opposite directions, and only one of them is safe: a
missing `read` policy leaves every revoked client **admitted**, a missing
`delete` policy leaves a client **revoked**.
Neither is visible to `just check-schema`, `cargo build`, `just clippy` or any
of the ten `just verify` gates — re-measured for the three new arms.

**`a_disabled_client_reads_the_same_through_both_paths` changed, and had to.**
It seeded its row by calling `disable_client`; that call is now a CrateStack
`upsert`, so leaving it would have made the test a generated write read back
by a generated read, agreeing with itself — the exact thing that test's own
doc comment disclaimed. It seeds with an inline `INSERT` and removes with an
inline `DELETE` now, deliberately not shared with the new write test.

**The gate that was blind, and now is not (review, 2026-09-06).** Every one
of the four mutations above was re-run by the sabotage review, and all four
reproduced — *and* all four left `cargo nextest run -p vpay-db --lib` green at
26 passed. The database-free half of the gate could not see a single missing
`@@allow` arm, including the two whose runtime effect is silent. `vpay-db`'s
unit suite now carries
`disabled_clients::tests::every_action_this_crate_calls_has_an_allow_arm`,
which reads the four `&'static [ReadPolicy]` slots off the compiled
`ModelDescriptor` and fails in 4 ms with no Docker; deleting each arm in turn
was re-run against it and it is red four times out of four. It does not
replace the container tests — a non-empty slot is not the same claim as "the
policy admits this caller" — it removes the wait to find out that a slot is
empty. Details and the transcripts:
[docs/plans/exp16-notes/opus-review.md](plans/exp16-notes/opus-review.md).

**Stays `NotImplemented`: nothing.** No function gained a stub, no test was
weakened, and no `#[ignore]` was added.

**Not done, named:** the transaction seam is still unexercised — no CrateStack
`run_in_tx` is called anywhere, because neither of these two writes has
anything to be atomic with. `providers`, `currencies` and every other table
are unmoved. The `Unique`, `ForeignKey` and `Check` arms of
`PersistenceError` are still unit-tested rather than exercised:
`disabled_clients` has no foreign key and no CHECK, and its only unique
constraint is the primary key the upsert exists to absorb. The release image
was not rebuilt (this change adds no dependency and no file the Dockerfile
would have to copy).
[docs/plans/exp16-notes/opus.md](plans/exp16-notes/opus.md) has the
transcripts and § 6 the full "not measured" list.

**Gate, run on this branch on 2026-09-06 against the pinned 1.98.0
toolchain.** `cargo build --workspace --all-targets`, `just fmt-check`,
`just clippy`, all **ten** `just verify` gates, `cargo nextest run -p vpay-db
--lib` **26 passed** (24 + the two render tests), `cargo nextest run -p xtask`
**215 passed**, `just test-doc` **96 passed, 1 ignored**, `just deny`
`advisories ok, bans ok, licenses ok, sources ok`, `just docs-check`, and
`just verify-ignored` **0 ignored (expected 0), 43 test binaries (expected
43), 1372 total**. `just test-rust`: **1372 tests run, 1372 passed, 0
skipped** across 43 binaries in 689.775 s — 1369 plus this change's three (two
`vpay-db` unit tests and one integration test; no new test binary, so
`expected_suites` stays 43 and `min_tests` stays 1080). The container-backed
drift test was re-run separately and passes with its constants unmoved.

**Re-measured by the review on the same day: 1373 / 43 binaries / 0 skipped,
`-p vpay-db --lib` 27**, after the review added the policy-slot test described
below. `just test-doc` **96 passed, 1 ignored** and all ten `just verify`
gates unchanged; `just ci` green end to end.

The first of the two `just test-rust` runs failed one test —
`webhooks the_delivered_signature_verifies_with_the_shipping_node_sdk`, with
`sh: 1: tsc: not found` — because no `node_modules` had been installed in that
worktree. It is unrelated to this change, which touches no TypeScript;
`pnpm install --frozen-lockfile` under the pinned Node 22.23.2 fixed it and
the rerun above is the green one. Recorded rather than dropped, because
"1372 passed" on a second attempt is a different claim from "1372 passed".

#### `currencies` and `providers`: migration 0032 and the first native-enum conversion (2026-09-06)

**What moved.** `ConfigReconcile::reconcile`'s **currency** pass runs through
CrateStack: `find_unique(code).for_update().run_in_tx(tx, ctx)` then
`upsert(CreateCurrencyInput).run_in_tx(tx, ctx)`, both inside the transaction
`reconcile` opens, after the same `pg_advisory_xact_lock`, in the same sorted
order. This is the first use of `run_in_tx` anywhere in vpay — the paragraph
above that says "the transaction seam is still unexercised" was true until
this change and is not now. It is also the first exercise of
`PersistenceError`'s `Denied` arm against a real database rather than a
synthetic error (mutation 3 below). `model Currency` gains
`@@allow("read"/"create"/"update", auth().isSystem())` and `model Provider`
gains `@@allow("read", …)`.

**Migration 0032** (`0032_currencies-providers-cratestack-shape.sql`, count
31 → 32) does three things: `currencies.exponent` `INT` → `BIGINT`; the two
hand-named `currencies` CHECKs renamed to the generator's
`<table>_<column>_<validator>_check` spelling with the generator's own
predicates; and `providers.flow` converted from the native `provider_flow`
enum to `TEXT` + `providers_flow_enum_check`, with `DROP TYPE provider_flow`.
`providers.partial_refunds_imply_refunds` is untouched, deliberately: it is
multi-column, invisible to `migrate baseline` in both directions, and guarded
only by `partial_refunds_without_refunds_is_rejected_by_the_database`.

**0032 is not backward compatible with the previous release's binary, and
that is an operational cost this section owes an operator.** It is the first
migration here to alter a column *type* that shipping code binds. Measured on
2026-09-06 against a database that already had rows (the review pass; the
repository's own migration tests only ever apply to an empty one): after 0032,
the pre-0032 binary's boot-step-4 insert fails with `type "provider_flow" does
not exist` (SQLSTATE `42704`), and its `i32` read of `currencies.exponent`
fails to decode against `int8` — sqlx refuses the narrowing. Migrations here
are forward-only and both binaries migrate-then-reconcile at boot, so in a
rolling deploy, or after a rollback to the previous image, any old-version
process that restarts once 0032 has landed **crash-loops at boot step 4**.
Ship 0032 with the release that carries the matching code and do not roll that
release back past it. Turning this into an expand/contract pair across two
releases would remove the constraint; that was **not** done and is a
maintainer's decision, not the reviewer's — see
[docs/plans/exp17-notes/opus-review.md](plans/exp17-notes/opus-review.md),
finding 3. The rows themselves are safe: the same measurement confirmed every
`currencies.exponent` and `providers.flow` value survives, and a rail already
`enabled = false` stays disabled.

**`providers.flow` is the first of vpay's seven native enums to be
converted.** The other six — `intent_status`, `charge_state`, `failure_code`,
`account_kind`, `direction`, and `payment_intents.last_payment_error_code` —
are on tables no CrateStack query touches, and each needs the same treatment
before one can. CrateStack's generated row decoders read an enum column with
`try_get::<String>()`, so a native enum column fails to decode on **every**
read through that layer.

**Not done in this change, named, and the reason was measured rather than
pending — SUPERSEDED the same day by migration 0033; see "`providers` through
CrateStack" below.** `reconcile`'s **provider** pass was still a hand-written
`INSERT ... ON CONFLICT` when this section was written, and could not move
while the defaults were there. `cratestack-macros` drops every
`@default(...)` field from both `Create{Model}Input` and
`upsert_update_columns`, and `model Provider` carried one on all five
capability booleans because the live table did. The generated statement was

```text
INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
```

— `supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist` and `enabled` are in neither list. Shipping that would
insert every rail with the column defaults regardless of what the deployment
configured, and would never carry a capability change to an existing row: a
rail an operator had just disabled would come back enabled. That is a
plausible-looking success storing the wrong value, so it was not shipped.
`the_provider_upsert_cannot_carry_the_capability_columns` pinned the rendered
statement so that a fix would turn it red; it is
`the_provider_upsert_carries_all_eight_columns` now, asserting the inverse.

**A maintainer's decision, surfaced rather than taken — and taken the same
day.** Unblocking it on vpay's side meant removing the five `@default(...)`s
from `model Provider` *and* `ALTER TABLE providers ALTER COLUMN ... DROP
DEFAULT` on all five — letting a code generator's input-shaping rule decide
vpay's DDL, and removing a defaulting behaviour any future writer of that
table would expect. The alternative was upstream growing a way to include a
defaulted column in an upsert input. The maintainer's decision (**D7**,
2026-09-06) was the first, on the ground that `reconcile` is the only writer
of those columns and always writes all five, so the default could only ever
invent a capability for a writer that had forgotten one. Migration 0033 is
that decision; see "`providers` through CrateStack" below.

**Also not done in *this* change:** no other table moved; `reconcile` still
owns its own transaction and that did not change; the provider *disable* pass
(`UPDATE providers SET enabled = false WHERE code <> ALL($1)`) is still raw
sqlx **and still is** after 0033, because it addresses rows by their absence
from a list and no generated builder expresses that;
`providers.code_length` and `providers.display_name_length` are still
hand-named (the rename would have bought zero drift and `display_name` has no
`@db_enforce` to converge on); and no production path *reads* `providers`
through CrateStack — `model Provider`'s `read` policy still exists for one
test, and 0033 changed the write, not the read.

**Public API changed.** `CurrencySeed::exponent` and
`DbError::CurrencyExponentConflict`'s `stored`/`seeded` are `i64` rather than
`i32`, because the column is `BIGINT`. `vpay_api::v1::boot::boot_seeds` no
longer returns `ConfigError::Validation` at all: the "exponent does not fit
the column" arm became unreachable *by type* (every `u32` fits an `i64`), so
the fallible conversion was replaced with `i64::from` rather than left as an
error branch nothing could take.

**Gate, run on this branch on 2026-09-06 against the pinned 1.98.0
toolchain.** `just ci` end to end, exit 0, with containers and with
`node_modules` installed from the pinned lockfile. All **ten** `just verify`
gates green — including `check-schema` at cratestack 0.11.1, still 13
model/enum declarations (this change adds policies, not declarations).
`just verify-ignored`: **0 ignored (expected 0), 43 test binaries (expected
43), 1382 total** — master's 1373 plus this change's eight and the review
pass's one, every one of them in a file that already existed, so
`expected_suites` stays 43 and the 1080 floor is untouched. (The whole gate
was re-run recipe by recipe in the review pass at the delivered commit and
was green there too, at 1381: `test-rust` 16 m 30 s, **1381 passed, 0
skipped**.) `just test-doc` **96 passed, 1 ignored** (the ignored one
is `sdks/rust`'s README block and is pre-existing). `just deny`:
`advisories ok, bans ok, licenses ok, sources ok`. `just fmt-check` and
`just clippy` clean.

The eight: four `vpay-db` unit tests in `config_reconcile` (two `preview_sql`
pins, the policy-slot assertion, and the container-backed provider parity
read), two container cases in `vpay-db/tests/repositories.rs`, and two in
`postgres_smoke.rs`.

The ninth is the review pass's:
`a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`,
also in `vpay-db/tests/repositories.rs`. It closes the one gap the mutations
above did not cover — that the CrateStack currency write is inside *vpay's*
transaction. Swapping that one `run_in_tx(&mut tx, &ctx)` for `run(&ctx)`
did not fail the suite, it **hung** it: `upsert`'s own conflict probe is
`SELECT … FOR UPDATE`, so off the transaction it waits on the row lock
`find_unique(...).for_update()` is holding while that transaction waits on
it, and `reconcile_is_idempotent_and_disables_a_dropped_provider_code`
reported `SLOW [>480.000s]` until the run was killed. In a deployment that
is a boot that never returns. `.for_update()` and `run_in_tx` are therefore
coupled, which the comment on that loop had not said — it argued only about
`gate_update_policy`'s policy probe, a different query that genuinely has no
`FOR UPDATE`.

**A hazard found while running the gate, recorded and not fixed:** `just fmt`
is `cargo fmt --all` *then* `pnpm exec prettier --write .`. Running it
rewrites **222** tracked files with prettier's defaults (measured read-only
with `prettier --list-different .` in the review pass) and then **fails** on
`backends/crates/vpay-config/tests/fixtures/malformed.yml`, which is
deliberately malformed YAML — so the recipe leaves the tree reformatted *and*
reports failure. `just ci` is unaffected: it runs `fmt-check`
(`cargo fmt --all -- --check`), never `fmt`. This is a trap for the "Before
you open a PR" instruction in `AGENTS.md`, not a broken gate.

This entry said "this repository ships no prettier configuration" until the
review pass corrected it. It ships `.prettierignore` (Step 6), whose own
header comment describes exactly this failure mode for
`deploy/helm/**/templates/` and whose established remedy is an ignore entry.
That changes what is being left to the maintainer: not "introduce prettier
configuration", but (a) whether `.prettierignore` should grow an entry for a
fixture that is *deliberately* unparseable, which is the pattern already in
that file, and (b) separately, what to do about the 222 files prettier's
defaults would rewrite — which an ignore entry does not address and a
`.prettierrc` or a narrower glob would.
[docs/plans/exp17-notes/opus.md](plans/exp17-notes/opus.md) § 6 has the
transcript and
[opus-review.md](plans/exp17-notes/opus-review.md) finding 2 the correction.

**Mutations run on 2026-09-06**, transcripts in
[docs/plans/exp17-notes/opus.md](plans/exp17-notes/opus.md):

| Mutation | Result |
|---|---|
| Revert `ALTER COLUMN flow TYPE TEXT` in 0032 (and restore the `::provider_flow` bind cast, so the *read* is the only thing that can fail) | `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` FAILS: `the CrateStack provider read failed: database: error occurred while decoding column "flow": mismatched types; Rust type` `alloc::string::String` `(as SQL type TEXT) is not compatible with SQL type provider_flow`. Pins the native-enum finding to a message rather than a paragraph |
| Delete `.for_update()` from the currency read | `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` still **PASSES** — the advisory lock, not the row lock, is what serialises boot against boot. **And so did every other test in the repository** (103/103 in `vpay-db`), which is why this change adds `reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer`: it is red under this mutation, with a concurrent writer's committed 3 clobbered back to 0 |
| Delete `@@allow("create", …)` from `model Currency` | LOUD, on every boot: `Currency: a model policy denied a system upsert: forbidden: create policy denied this upsert` → `PersistenceError::Denied` → `Category::Internal`. `every_action_this_module_calls_has_an_allow_arm` catches it in 5 ms with no container |
| Delete `@@allow("read", …)` from `model Currency` | SILENT at runtime and the dangerous direction. Three container tests go red — the two exponent-conflict cases and the row-lock case — because the read answers `None` for a row that exists and the upsert overwrites the stored exponent instead of refusing to. The no-container policy-slot test also catches it |
| Delete `CONSTRAINT partial_refunds_imply_refunds` from 0002 | `partial_refunds_without_refunds_is_rejected_by_the_database` FAILS, **and the drift count does not move**: the report still says `drift detected in 16 table(s)/view(s) (84 change(s) total)`. The drift test fails only on its own `pg_constraint` read. Re-confirms the multi-column blind spot on this branch's numbers |

#### `providers` through CrateStack: migration 0033 and the D7 decision (2026-09-06)

`ConfigReconcile::reconcile`'s **provider** pass is
`upsert(CreateProviderInput).run_in_tx(tx, ctx)`, inside the same transaction
and after the same `pg_advisory_xact_lock` as the currency pass. The section
above records why it could not be, and this one records the decision that
changed that and exactly what the decision costs.

**The decision (D7, taken by the maintainer's delegate).** `cratestack-macros`
drops every `@default(...)` field from both `Create{Model}Input` and
`upsert_update_columns`, and `model Provider` carried one on all five
capability booleans because migration 0002's table did. The two ways out were
(1) drop the five `@default(...)` *and* the five column `DEFAULT`s, or (2)
wait for upstream to grow a way to include a defaulted column in an upsert
input. **Option 1**, on the ground that `reconcile` is the *only* writer of
those five columns and always writes all five from configuration — so a column
default can never help that writer and can only invent a capability for some
other writer that forgot one. A rail silently recorded as "does not refund",
or silently recorded as enabled, is worse than an `INSERT` that refuses.

**Migration 0033** (`0033_providers-drop-capability-defaults.sql`, count
32 → 33) is five `ALTER TABLE providers ALTER COLUMN … DROP DEFAULT` and one
`COMMENT ON TABLE`. It rewrites no rows (`DROP DEFAULT` touches `pg_attrdef`
only) and, unlike 0032, **is** backward compatible with the previous
release's binary: the pre-0033 `reconcile` names all eight columns in its
hand-written insert, and nothing else in the tree writes this table.

**What it costs, stated as a refusal rather than a difference.** All five
columns are `NOT NULL` and now have no default, so
`INSERT INTO providers (code, display_name, flow) VALUES (…)` typed at a psql
prompt fails with `23502` where it used to succeed and invent four `false`s
and a `true`.
`a_hand_written_provider_insert_must_now_name_every_capability_column`
(`postgres_smoke.rs`) asserts both halves — the refusal, and that an `INSERT`
naming all eight still succeeds — and reads `pg_attrdef` so that a migration
which dropped four defaults and missed one fails here rather than in
production. Three fixtures in that same file had to grow the columns in the
same commit; that is the whole blast radius in this repository.

**The drift did not move, and that is the measurement, not the expectation.**
84 changes / 16 relations / 17 unmappable before and after, and the
`providers` block is byte-identical. While the five `@default(...)` and the
five `DEFAULT`s were both there they agreed; with both gone they still agree.
What creates drift is doing *one* half: with the five `@default(...)` removed
from the schema and 0033 absent, the report is **89 changes**, exactly five
`column … default value differs` lines on `providers` (measured here on
2026-09-06, reproducing exp17's review pass from the opposite side). That is
why the migration and the schema edit are one commit.

**A public API addition, and one classification deliberately changed.**
`CreateProviderInput::flow` is the schema's `ProviderFlow` enum rather than a
`String`, so `reconcile` parses `ProviderSeed::flow` before it builds a
statement. `ProviderSeed::flow` stays a `String` (Step 2's D4 is untouched),
and an unparseable label is the new `DbError::ProviderFlowUnknown` —
`Category::Configuration`, **exit 78**, where the `providers_flow_enum_check`
CHECK it replaces produced `DbError::Query` → `Category::Storage` → **exit
69**, i.e. "wait for Postgres" for a typo in a deployment. The CHECK is still
in the database and still fires for a writer that is not `reconcile`
(`an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`
is unchanged). The parse is a `match`, not `unwrap_or_default()`, because
`cratestack-macros` derives `Default` on every generated enum with the *first*
variant as the default — so `unwrap_or_default()` here would store a typo'd
rail as a push rail and return `Ok`.

**`model Provider` gained `@@allow("create", …)` and `@@allow("update", …)`**,
in the commit that moved the write and not before it, and still has no
`delete` arm: `reconcile` disables a dropped rail rather than removing one.

**No `find_unique(...).for_update()` ahead of the provider upsert, and that is
deliberate.** The currency pass needs its read because the upsert renders
`SET exponent = EXCLUDED.exponent` and a stored exponent must never be
overwritten — the read *is* the guard. A provider has no such value: all eight
columns are owned by configuration and overwriting them is the point, so the
read would return a row nothing compares. The row lock it would take is taken
anyway, in the same transaction, by `upsert`'s own conflict probe
(`upsert_exec.rs::run_upsert_in_tx` → `select_for_update_by_conflict_target`
on `tx`). A measured consequence worth recording: because no `providers` row
lock is held when the upsert runs, swapping `run_in_tx` for `run` **fails in
1.2 s instead of hanging**, which is the opposite of what the same mutation
does to the currency pass.

**Tests: +3 net in `vpay-db`, +1 in `postgres_smoke.rs`, one replaced** (the
review pass below added more; see its own subsection for the counts)**.**
`the_provider_upsert_cannot_carry_the_capability_columns` became
`the_provider_upsert_carries_all_eight_columns` (same file, inverted claim).
New: `an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement`
(no container),
`a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile` and
`a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
(`vpay-db/tests/repositories.rs`), and
`a_hand_written_provider_insert_must_now_name_every_capability_column`
(`postgres_smoke.rs`).
`a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
had to change its expected variant from `DbError::Query` to
`DbError::Persistence(PersistenceError::Check { … })`, because the statement
that raises the `23514` is a CrateStack upsert now — the exact "a caller
matching the variant would have silently stopped matching" the module doc
warns about, caught by a test rather than in production.

**Gate, run on this branch on 2026-09-06 against the pinned 1.98.0 toolchain
and Node 22.23.2 from `.nvmrc`.** `just ci` end to end, **exit 0**, containers
included and `node_modules` installed with `pnpm install --frozen-lockfile`.
All **ten** `just verify` gates green, `check-schema` at cratestack 0.11.1
still 13 model/enum declarations (this change adds policies and removes
defaults, not declarations). `just test-rust`: **1386 tests run, 1386 passed,
0 skipped** (973 s). `just verify-ignored`: **0 ignored (expected 0), 43 test
binaries (expected 43), 1386 total** — 1382 plus this change's four, all in
files that already existed, so `expected_suites` and the 1080 floor are
untouched. `just test-doc`: 96 passed, 1 ignored (the `sdks/rust` README
block, pre-existing). `just deny`: `advisories ok, bans ok, licenses ok,
sources ok`. `lint-web` and `test-web` green.

Two earlier `just ci` runs on the same tree failed for reasons that were not
this change and are recorded rather than hidden: one on a
`failed to create a container: Timeout error` starting the MTN stub while
other suites were saturating the machine, and one on
`the_delivered_signature_verifies_with_the_shipping_node_sdk` with
`sh: 1: tsc: not found` — a worktree whose `node_modules` had not been
installed yet. Neither reproduced once the machine was quieter and the
lockfile install had run.

**`fn reconcile` is now 237 lines** (203 before), still the longest production
function on `verify-docs`' advisory list and still almost all comment. exp17's
review flagged the same thing at 203 and left it for a maintainer; this change
condensed the provider loop's comments once (the long-form argument lives in
migration 0033's header and in
[reference/vpay-db.md](reference/vpay-db.md#cratestack)) and did not split the
function, because the split would separate the advisory lock from the
statements it protects.

**Mutations run on 2026-09-06**, transcripts in
[docs/plans/exp20-provider-defaults-notes/opus.md](plans/exp20-provider-defaults-notes/opus.md):

| Mutation | Result |
|---|---|
| Provider upsert `.run_in_tx(&mut tx, &ctx)` → `.run(&ctx)` | `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction` **FAILS in 1.226 s**, `left: 1, right: 0`, with the message naming `run_in_tx`. **Nothing hung** — the whole reconcile set finished, longest case 5.1 s. That is the difference from the currency pass, where the same mutation deadlocks against `find_unique(...).for_update()` |
| Delete `@@allow("update", …)` from `model Provider` | LOUD, and **only on the second boot**: `every_action_this_module_calls_has_an_allow_arm` fails in 3 ms with no container, and two container cases fail with `Provider: a model policy denied a system upsert: forbidden: update policy denied this upsert` — `reconcile_is_idempotent_…` at "a second, identical reconcile must succeed" and `a_rail_the_configuration_disables_…` at "reconciling a disabled rail must succeed". A fresh database's first boot succeeds, which is why the no-container slot test exists |
| Delete migration 0033, keep the schema edit | **Test-time**, three ways: `the_cstack_schema_drifts_…` fails at **89 vs 84** with five `column … default value differs` lines on `providers`; `a_hand_written_provider_insert_must_now_name_every_capability_column` fails because the three-column INSERT succeeds again; `schema_migrates_cleanly_on_an_empty_database` fails at 32 vs 33 |
| Restore one `@default(false)` in `model Provider`, keep 0033 | **Compile-time**: `error[E0560]: struct `inputs::CreateProviderInput` has no field named `supports_refunds``. The schema half cannot regress silently — the crate stops building before any test runs |

##### Review pass, 2026-09-06: one mutation the delivered change did not survive

The four mutations above were re-run and reproduce. A fifth, not run when the
change was written, **survived it**, and closing it is why this section has a
review subsection at all.

| Mutation | As delivered | After the review |
|---|---|---|
| `reconcile`'s flow parse `.map_err(…)` → `.unwrap_or_default()` | **SURVIVES.** `cargo nextest run -p vpay-db`: **112/112 passed**. `redirekt` is stored as a **push rail** — `cratestack-macros` marks the first variant of every generated enum `#[default]` and `ProviderFlow`'s first variant is `push` — and boot step 4 returns `Ok` | **FAILS in 1.27 s** on `a_flow_label_the_schema_cannot_name_is_refused_by_reconcile_before_any_row_is_written`, at `a flow that is neither ``push`` nor ``redirect`` must be refused` |

Nothing exercised `reconcile` with an unnameable flow.
`an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement`
constructs `DbError::ProviderFlowUnknown` **by hand** and asserts how it
classifies; it never calls `reconcile`, so it is green whatever `reconcile`
does with the label. The `unwrap_or_default()` trap that the code comment,
the variant's doc, this page and
[reference/vpay-db.md](reference/vpay-db.md#cratestack) all describe as
guarded was, until this test, guarded by prose only.

The new container test calls the public trait, asserts the variant, the
`Category::Configuration`/exit-78 classification, and that **neither** a
`providers` row nor the `currencies` row the pass before it had already
upserted survives — so it also pins that the refusal rolls the whole boot-step-4
transaction back.

**Two coverage gaps closed alongside it, both in the decisive direction the
task brief named.**

1. `a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile` asserted
   all eight columns after the disable and the repeat, but never the *reverse*:
   nothing in the tree turned `providers.enabled` from `false` back to `true`
   through `reconcile`. A fourth reconcile now does, asserting all eight
   columns again. Stated plainly, because it matters for how much this is
   worth: **no single-line mutation was found that this step alone kills** —
   every one tried is caught earlier by the first or second assertion. What it
   pins is a documented behaviour (`ProviderHost::enabled`'s doc and
   [flows/configuration.md](flows/configuration.md): turning a rail off in the
   YAML is not the same as deleting its block) that no test exercised.
2. Nothing asked what a `reconcile` that *waited* on the advisory lock does
   with state the holder **committed** while it waited — which is the question
   the deliberately absent `find_unique(...).for_update()` on the provider
   pass raises. `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_
   released` releases the lock by rollback, so the waiter always meets an
   empty table.
   `a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed`
   now holds the lock in one connection, writes a `providers` row that
   disagrees with the waiter's configuration on all eight columns, **commits**,
   and asserts the waiter's own configuration is what survives. The answer is
   that the provider pass needs no row lock of its own: the advisory lock
   serialises, and `upsert`'s conflict probe locks the committed row.

**A dependency this made visible, and it was not written down anywhere: boot
step 4 requires READ COMMITTED.** Adding `SET TRANSACTION ISOLATION LEVEL
REPEATABLE READ` after `reconcile`'s `pool.begin()` — a plausible "make boot
safer" edit — makes the new test FAIL in 4.2 s with `could not serialize
access due to concurrent update` (`40001`), because the snapshot is taken when
the `pg_advisory_xact_lock` statement starts, i.e. *before* the holder
commits, so the conflict probe cannot see the row it must update. Measured
2026-09-06; **every other reconcile case passes under that mutation**,
`reconcile_waits_for_the_boot_lock_…` and
`two_concurrent_reconciles_…_converge` included. Recorded in
`config_reconcile.rs`'s comment on that loop and in
[reference/vpay-db.md](reference/vpay-db.md#cratestack).

**A second classification moved, and this page and the module doc both said it
had not.** `config_reconcile`'s "`# Errors` moved" paragraph claimed *the
classification is unchanged — a `23514` is still `Category::Internal` …
because `persistence::classify_cratestack` and `error::classify_write` are
asserted against each other*. Both halves are wrong. The two functions are
asserted against each other for `23505` and `23503` only
(`a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx`),
and on `23514` they **disagree by design**: `classify_write` deliberately
leaves a CHECK violation in the unclassified `DbError::Query` bucket →
`Category::Storage` → exit **69**, while `classify_cratestack` gives it
`PersistenceError::Check` → `Category::Internal` → exit **1**.

`partial_refunds_imply_refunds` is the only `23514` boot step 4 can raise, and
until the provider pass moved onto CrateStack it reached a supervisor as `69`,
"wait for Postgres", for an adapter whose declared `Capabilities` are
incoherent. It is `1` now. Measured 2026-09-06 and pinned by a new assertion
in `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`.

**Maintainer decision, surfaced not taken:** `Internal`/`1` is defensible —
the database is healthy, and `Capabilities::is_coherent` exists but is checked
only in `vpay-server`'s `#[cfg(test)]` assertion and the conformance suite,
never at boot — but `Category::Configuration`/`78` ("fix the deploy") is what
the flow label got for the same class of mistake one paragraph earlier, and
the two now disagree. Either answer is better than the `69` this replaced.
Deciding between them, and whether boot should check
`Capabilities::is_coherent` before it reconciles at all, is not the review's
call.

**Migration 0033 was only ever applied to an empty table.** Its header makes
three claims. The refusal it creates is asserted
(`a_hand_written_provider_insert_must_now_name_every_capability_column`, on a
database migrated from empty). Backward compatibility with the previous
release's binary is a fact about *code* — the pre-0033 `reconcile` names all
eight columns in its hand-written `INSERT`, and nothing else in the tree writes
`providers`; both re-checked against `06e27f9` in the review, and no test can
execute a binary that is not in this checkout. The third, *"NO DATA CHANGES …
every existing row keeps exactly the capabilities it had"*, is the one an
upgrade depends on and nothing exercised it.
`migration_0033_changes_no_stored_capability_on_a_populated_table`
(`postgres_smoke.rs`) now does: it restores the five pre-0033 defaults,
asserts through `pg_attrdef` that the restore was real (so the test cannot go
vacuous), writes one row that takes all five defaults and one that contradicts
all five, applies **0033's own text** via `include_str!` rather than a copy of
its statements, and asserts both rows are byte-identical afterwards, that no
default survives, and that the three-column `INSERT` is refused on this
database too.

**Review pass test count: +3 test cases, no new test binary** (all three land
in files that already existed), so `expected_suites` and the `min_tests` floor
are untouched.

Full transcript, including the two mutations that were re-measured rather than
accepted and the exp18 merge check, in
[plans/exp20-provider-defaults-notes/opus-review.md](plans/exp20-provider-defaults-notes/opus-review.md).

**Gate on the review head (`137a75e`), 2026-09-06, rustc 1.98.0 and Node
22.23.2 from `.nvmrc` with `pnpm install --frozen-lockfile`:** `just ci` end to
end, **exit 0**. All ten `just verify` gates green (`check-schema` still 13
model/enum declarations at cratestack 0.11.1; `verify-links` 807 links in 147
files). `just test-rust`: **1389 tests run, 1389 passed, 0 skipped** (932 s) —
1386 plus the review's three. `just verify-ignored`: **0 ignored (expected 0),
43 test binaries (expected 43), 1389 total** (floor 1080). `just test-doc`: 96
passed, 1 ignored (the pre-existing `sdks/rust` README block). `just deny`:
`advisories ok, bans ok, licenses ok, sources ok`. `lint-web` and `test-web`
green.

An earlier full run on the delivered head failed on `provider_callback ::
case_1_mtn_momo` with `failed to create a container: Timeout error` at 120 s,
while an abandoned `vpay-demo` compose stack was crash-looping on this host
(`Restarting (69)`, its database gone) and the load average was above 22. Not
this change: every test it did reach passed, including the drift test at 84,
and the same suite is green above. Recorded rather than hidden.

**`fn reconcile` is 249 lines** on this head (203 before exp20, 237 as
delivered): the review added two comment blocks, one for the READ COMMITTED
dependency and one for the `23514` classification. Still the longest production
function on `verify-docs`' advisory list, still almost all comment, and still a
maintainer's call.

#### `customers`: the first table born with a model (2026-09-06, S4a)

Migration `0034` creates `customers`, and `schemas/vpay.cstack`'s
`model Customer` lands in the **same commit**. Every table before it acquired
its model afterwards, and migration `0033` exists because of what that costs:
`cratestack-macros` drops every `@default(...)` field from
`Create{Model}Input`, so `providers`' five capability booleans could not be
written through the generated input until their column `DEFAULT`s came off.
`0034` pays that up front — **no column CrateStack may write carries a
`DEFAULT`** — and the two that do, `created_at` and `updated_at`, are declared
`@default(now())` for `DisabledClient.disabled_at`'s measured reason.

**Two of seven repository methods run through the generated layer**, taking
the count to **eight queries over five tables**: `touch_last_used`
(`update_many`) and `delete` (`delete_many`). One column decides the split —
`metadata JSONB NOT NULL`, undeclared for the two costs `model Event.data`
already measured (`map_scalar` does not read `jsonb` back;
`Value::from_plain_json` demotes any number outside `i64` to `f64`) — and the
consequence is sharper here than there: an undeclared column is absent from
the generated model **struct** too, so a CrateStack *read* could not render
the wire object at all. Create, update and every read are hand-written.

That is not a consolation prize: the two that moved are the two where being
wrong is irreversible, and **both policy failures are silent** —
`update_many` and `delete_many` compile the `@@allow` into the statement's own
`WHERE`, so a deleted arm matches zero rows and returns `Ok`. A missing
`update` freezes every customer's retention clock at creation and the sweep
deletes live customers twelve months later; a missing `delete` answers
`{deleted: true}` while the row stays.
`every_action_this_module_calls_has_an_allow_arm` asserts the compiled
descriptor with no container, and also asserts the **absence** of the three
arms this model deliberately does not grant.

`metadata` carries **no column `DEFAULT`** precisely so that a generated
`INSERT` omitting it is a loud `23502` rather than a silent `{}` over a
merchant's data. Two tests arm that: `vpay-db`'s
`a_generated_customer_insert_cannot_carry_metadata` pins the rendered
statement, and `postgres_smoke`'s
`customers_metadata_has_no_default_so_an_omitted_insert_is_loud` reads
`pg_attrdef` *before* asserting the failure so it cannot go vacuous.

**Drift: 101 → 113 over 16 → 17 relations, unmappable 17 → 18.** Measured
against a freshly migrated database and accounted for line by line on
`EXPECTED_DRIFT_CHANGES`. The +12 is ten on `customers` (six undeclared
hand-named CHECKs, three undeclared indexes, one `seq` default line —
`model Event.seq`'s known trade) and two on `payment_intents` (the
`customer_id` column and its partial index). **`checkout_sessions` gained the
identical column and index and cost zero**, because it is an undeclared
*table* and the report collapses the whole thing to one line — so the marginal
drift of a column depends on whether its table is modelled, which is the
opposite of the intuition that modelling reduces drift.
`at_least_one_identifier`, the eleventh multi-column CHECK, contributes
nothing in either direction, which is why `postgres_smoke` asserts it
directly.

**Drift again: 113 → 130 over 17 → 20 relations, unmappable unchanged at 18**
(2026-09-07, ADR-0017's migration `0035`). The +17 is nine on
`staff_members`, four on `staff_sessions` and four on
`oauth_authorization_codes`, **and every one of the seventeen is a hand-named
CHECK or an undeclared index**. Not one `column … type differs`, not one
`column … default value differs`, not one `column … is declared in the schema
but does not exist`, and no `table … is not declared` line for any of the
three. Every earlier modelled table carries at least one of those — `charges`
and `ledger_entries` carry enum-type lines **no migration can remove**, and
`customers` and `events` carry identity-default lines.

That is what "shaped so the data layer can write every column" buys, and the
constant is the evidence rather than the claim: no `bytea` (so the unmappable
count does not move either), no native enum, no `DEFAULT` on any column a
writer names, and no `seq` cursor. `staff_members_totp_is_paired`, the twelfth
multi-column CHECK, contributes nothing in either direction and is asserted
directly for `at_least_one_identifier`'s reason.

**A naming fact with no gate behind it, found by a container test.** 0.11.1
derives a table name from the model name with
`pluralize(to_snake_case(model))` and has no `@@map`, so `model Staff` reads
and writes `staffs`. The first draft of `0035` created a table called `staff`;
`cargo build`, `just check-schema`, `clippy` and all ten `just verify` gates
stayed green, and the first thing to say anything was a Postgres-backed test
answering `relation "staffs" does not exist`. The model is `StaffMember`, the
table is `staff_members`, and the Rust trait is still `vpay_db::Staff`.

**`to_chrono` is the second chrono/time crossing in this crate and the first
in that direction.** CrateStack's inputs and filters take
`chrono::DateTime<Utc>`; every TIMESTAMPTZ this crate binds by hand is
`time::OffsetDateTime`. Its `unwrap_or` is unreachable — chrono's range is
~±262,000 years and `time`'s is −9999..=9999 — and
`the_chrono_conversion_is_total_over_every_instant_time_can_hold` proves it at
both extremes rather than in prose, so a future `time` with `large-dates`
turns the branch red instead of clamping a retention clock silently.

**`sql_audit` fired twice, unprompted, on the first draft.** Once correctly,
on a computed `NOT EXISTS` fragment (now `const UNREFERENCED`); once as a
false positive on a `format!` inside a `#[cfg(test)]` assertion, which the
textual scanner read as a statement because it looks for the word `sql` within
forty characters before a `format!` and the assertion's own message printed
`{sql}`. Recorded in [reference/vpay-db.md](reference/vpay-db.md) rather than
worked around: the scanner will do it again, and the answer is to avoid
`format!` in a test that mentions `sql`, never to widen the allowlist.
`EXPECTED_ASSERT_SITES` 37 → 43, with the audit re-done.
### CrateStack 0.11.1 → 0.12.0 (2026-09-07)

**Nothing in this repository had to change but the version.** The CLI and the
library moved together, as the pin's own comment requires: `justfile`'s
`cratestack_version`, `Cargo.toml`'s `cratestack = { package =
"cratestack-pg", version = "=0.12.0" }`, twelve `cratestack-*` entries in
`Cargo.lock`, and both `install-cratestack-cli` steps in
`.github/workflows/ci.yml` to the commit `v0.12.0` was tagged at
(`0823bab382425e1fe4d04c42b9657b7e7bb7b286`).

**What the bump could have broken, and what was measured instead of assumed.**

- *The gate.* 0.12.0's one breaking change gives `SchemaError` file identity,
  so a `check-schema` that parsed the CLI's diagnostic would be the obvious
  casualty. It does not parse it — and that was proven rather than reasoned
  about, by deleting a field's type from a copy of `schemas/vpay.cstack` and
  running the recipe: exit **1**, `Error: expected field type`, the same line
  0.11.1 prints. The green run reports `cratestack 0.12.0, schema
  schemas/vpay.cstack (15 model/enum declarations, datasource present)`.
- *The drift constants.* `EXPECTED_DRIFT_CHANGES` and its two companions are
  measurements against a tool, so a tool bump is exactly when they can move
  with nobody touching the schema. Re-derived against a fresh
  `postgres:16-alpine` with the 0.12.0 binary on `PATH`: **101 changes / 16
  relations / 17 unmappable columns**, all three unchanged, and the test
  printed `cratestack CLI under test: 0.12.0 (justfile pins 0.12.0)`.
- *The licence surface.* The bump added **no package at all** — the set of
  package names in `Cargo.lock` is identical before and after, only twelve
  versions and their checksums moved — so `deny.toml`'s two Blue Oak
  exceptions are still the only ones needed. `cargo deny`: `advisories ok,
  bans ok, licenses ok, sources ok`.
- *The toolchain floor.* Every `cratestack-*` 0.12.0 manifest still declares
  `rust-version = "1.98.0"`, `cratestack-cli` included, so
  `rust-toolchain.toml` and `backends/Dockerfile` did not move.
- *The action pin.* `git/ref/tags/v0.12.0` resolves to `0823bab382…` with
  `"type": "commit"` — a lightweight tag, nothing to dereference. The
  `action.yml` blob is byte-identical at the old and new commits, and the
  action has **no checksum input**: it fetches the `.sha256` sidecar from the
  release at run time, so the release is what pins the binary. Walking those
  steps by hand gave a matching digest and a binary reporting `cratestack
  0.12.0`.

**All four measured upstream gaps are still open at 0.12.0** — `@default(...)`
fields absent from `Create{Model}Input`, `upsert` gating its update policy on
a second pooled connection, `from_plain_json`'s `f64` demotion, and no
read-back for `jsonb`/`bytea`/`int2`/`int4`. The evidence is file identity
against the 0.11.1 sources every earlier claim was measured from; the table
is in [reference/vpay-db.md § CrateStack](reference/vpay-db.md#cratestack),
under "Re-checked at 0.12.0". `@@check(expr)` is still absent too.

**Gate on this head, 2026-09-07, rustc 1.98.0 and Node 22.23.2 from `.nvmrc`
with `pnpm install --frozen-lockfile`, `cratestack 0.12.0` on `PATH`:**
`just ci` end to end, **exit 0**. All ten `just verify` gates green
(`check-schema` 15 model/enum declarations at cratestack 0.12.0;
`verify-links` 838 links in 153 files; `verify-sdk-parity` 385 proving tests,
29 dated gaps; `verify-serde` 53 types, 16 exempted; `verify-repositories` 4
concrete impls; `verify-toolchain` 1.98.0). `just test-rust`: **1401 tests
run, 1401 passed, 0 skipped** (886 s), containers included. `just
verify-ignored`: **0 ignored (expected 0), 43 test binaries (expected 43),
1401 total** (floor 1080). `just test-doc`: 96 passed, 1 ignored (the
pre-existing `sdks/rust` README block). `just deny`: `advisories ok, bans ok,
licenses ok, sources ok`. `lint-web` green; `test-web` 797 tests across eight
packages, 0 skipped.

Review transcript, including what the draft claimed without measuring, in
[plans/exp25-cratestack-012-notes/opus-review.md](plans/exp25-cratestack-012-notes/opus-review.md).


---

## Merchant SDKs

Two client libraries for the merchant API, both implementing the wire
contract in [docs/flows/merchant-auth.md](flows/merchant-auth.md): the RFC
7523 `private_key_jwt` handshake, token caching, every planned `/v1`
resource, and `Vpay-Signature` webhook verification. They landed on 2026-09-02
ahead of any server route; later the same day the merchant OP landed, so
the Rust SDK completed the handshake against a real `vpay_api::router`.

**Corrected 2026-09-03 (Step 5b).** The two sentences that used to close this
paragraph — "the Node SDK still has not spoken to a vpay at all" and "no vpay
serves any `/v1` *resource*, so neither SDK has ever completed a resource
call" — are both false now. Step 2 landed `/v1/payment_intents` and Step 3
made `confirm` reach a rail, and `sdks/stripe-compat` drives the Node SDK's
own `createStripeAuthenticator` (`@vaam-apps/vpay-sdk/stripe`) plus the official
`stripe` package against a **live compose stack** across create, retrieve,
list, confirm and cancel. What is still true: **no test inside
`sdks/nodejs` itself has ever talked to a vpay** — every server in that
package's tests is a `node:http` stub — and `sdks/rust`'s server contact is
the in-process router in `backends/tests/integration`. Except where a row
below names `sdks/stripe-compat`, every claim in this section is about what
the tests prove against stubs and against the real Authkestra verifier, and
nothing more.

| SDK | Status | What is proven, and how strongly |
|---|---|---|
| Rust — `sdks/rust`, crate `vpay-sdk` (workspace member, `publish = false`) | 🟡 | ~~**113 tests, 0 ignored**~~ ~~**136 tests, 0 ignored**~~ ~~**141 tests, 0 ignored**~~ **149 tests, 0 ignored — re-measured 2026-09-06** after S4a added eight `customers` cases (`cargo nextest run -p vpay-sdk`); the 141 below was measured before them with `cargo nextest run -p vpay-sdk` on issue #46 rebased onto issue #45's merge (the 136 was taken on #45's own branch and did not carry issue #46's `a_refund_fee_decodes_as_unknown_free_or_a_real_cost`); the 113 was taken on 2026-09-03 and had not moved through Step 9's Checkout Session cases or issue #45's two refund-retrieve cases. (It was 107 until `c40a137` on 2026-09-03, which added `PaymentIntent::client_secret: Option<String>` with `#[serde(default)]` and a hand-written `Debug` that redacts it — `sdks/rust/tests/{debug_redaction,resources}.rs`.) Run by `cargo nextest run --workspace` and therefore by `just ci`. **One of them is flaky on this machine and is not mine:** `a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched` (`tests/token_exchange.rs`) failed once in five consecutive runs on 2026-09-05 under a loaded host and passed the other four; nothing in issue #45 touches that file. **The assertion it mints is accepted by the real OP verifier** — `authkestra_op::client_assertion::verify_client_assertion` at the pinned `=0.7.1`, called directly in `tests/op_conformance.rs` against a `ClientRegistration` holding the matching public JWK, with `expected_audiences = [token_endpoint, issuer]` exactly as `handlers/token.rs` passes them — with and without a `kid`, and refused for a different keypair, a different `aud`, and a `kid` the key did not sign with. That is a **CI-gated** proof. Everything on the wire — token form fields, `Bearer` header, caching, single-flight refresh, the single 401 re-auth replaying the identical `Idempotency-Key` and body, each resource's exact path and form body, the Stripe-shaped error envelope, transport and timeout errors — is asserted byte-for-byte against a `wiremock` stub; the webhook verifier and form encoder are unit-tested. The README's Status section lists every source mutation that was run and which test each one fails. Cross-SDK parity is pinned by `src/form.rs` tests carrying the exact body string the Node encoder emits for the same parameters. TLS is built by the SDK itself (ring + vendored roots) and proven not to require or install a process-default provider (`tests/tls.rs`); **there is no live TLS test** — nothing here serves TLS, so certificate verification against the vendored roots is exercised by no test, and a merchant behind a private-CA proxy is not trusted. **2026-09-02 (Step 1): this SDK now drives a real vpay**, not only a stub — `backends/tests/integration/tests/merchant_token_flow.rs` uses it as the client for the whole handshake, and `vpay-api` takes it as a `[dev-dependency]` so `an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds` checks the shipping SDK against the registration the server builds from YAML. Still 🟡, now for one narrower reason. ~~those integration tests have never run under Docker or in CI (header paragraph)~~ **— corrected 2026-09-05: all seven `merchant_token_flow` tests are `PASS` in CI run `33929374663` (2026-09-04, `ubuntu-latest`, 1159/1159, 0 skipped), so they have run under Docker in CI.** ~~What remains is that every one of the eight `/v1` resource methods still has no route to call~~ **— corrected 2026-09-05 by counting them, and re-counted 2026-09-06 on the rebased tree: this SDK exposes fourteen resource methods, and twelve of them have a route.** The figure predated Checkout Sessions (four methods, 2026-09-04), `refunds.retrieve` (issue #45) and `account_holders.retrieve` (issue #47) — the "thirteen/eleven" it read until the rebase was measured on a tree without issue #47's resource. The two with no route are `refunds().create()` and `balance().retrieve()` |
| Node — `sdks/nodejs`, package `@vaam-apps/vpay-sdk` (`private: true`, zero runtime dependencies, Node ≥ 22.11) | 🟡 | ~~**150 tests, 0 skipped**~~ ~~**180 tests across 9 files, 0 skipped**~~ **190 tests across 9 files, 0 skipped — re-measured 2026-09-06** after S4a added nine `customers` cases in `client.test.ts` and one type-level case in `types.test.ts` (`pnpm -r test`, Node 22.23.2 per `.nvmrc`); the 180 below was measured before them (`pnpm -r test`, Node 22.23.2 per `.nvmrc`) on issue #46 rebased onto issue #45's merge; the 150 below had not moved through Step 9, issue #45 or issue #46. Historically: **150 tests, 0 skipped** (was 126 before Step 5b added `src/stripe-auth.test.ts` — **21** of the 22 new ones, the 22nd a `client.test.ts` case for a refused `baseUrl`; 148 after a 2026-09-03 run corrected this row's earlier 146; **150 since `c40a137`, same day, added `PaymentIntent.client_secret?: string` and two `client.test.ts` cases asserting it decodes typed on `create`/`retrieve` and is `undefined` on a list item** — measured by `pnpm --filter @vaam-apps/vpay-sdk test`), run by `pnpm -r test` and therefore by `just ci`; `pnpm --filter @vaam-apps/vpay-sdk build` is a CI step too — and now runs **before** `pnpm -r typecheck` in the `web` job, because `sdks/stripe-compat` imports `@vaam-apps/vpay-sdk/stripe` whose types resolve to the gitignored `dist/`. The same wire assertions as Rust, against a real `node:http` server started by each test — never a mocked `fetch` — including the fake-timer expiry and short-TTL margin cases, five-way concurrent single-flight, the 401 retry replaying the same `Idempotency-Key` and body on a `POST`, path ids percent-encoded so `../../admin` or `pi_1#frag` cannot leave `/v1`, a stalled response body surfacing as `VpayTransportError` rather than a raw `DOMException`, amounts refused unless a non-negative safe integer, `exactOptionalPropertyTypes`-safe public types (a compile-time test), and every README code block type-checked against `dist/`. The assertion's RS256 signature is verified with `node:crypto` and its claim set pinned to exactly `aud, exp, iat, iss, jti, sub`. **Node cannot link the Rust verifier, so its real-OP proof is weaker than Rust's and must not be read as equivalent:** `just sdk-conformance-node` mints an assertion with the built Node SDK and pipes it into `sdks/rust/examples/verify_assertion.rs`, which runs the real `verify_client_assertion`. It is a manual recipe, **not part of `just ci`**. Last run 2026-09-02 09:19 UTC, on this tree: `verified: the pinned authkestra-op verifier accepted this assertion for client_id=merchant_a` (`jti=e6ff9a35-59a9-4663-bd0a-7316609e817e`, `exp=2026-09-02 09:20:37 UTC`), exit 0; the same recipe exits 1 for a wrong `client_id`, a wrong `aud`, and a single flipped signature byte, so it discriminates. Re-run it and update this line whenever `auth.ts` or the pinned `authkestra-op` changes |
| Stripe-SDK compatibility — `@vaam-apps/vpay-sdk/stripe` + `sdks/stripe-compat` (`@vaam-apps/vpay-stripe-compat`, `private: true`) | 🟡 | **New 2026-09-03 (Step 5b).** `createStripeAuthenticator` is a `config.authenticator` for the official `stripe` package, built on the *same* `TokenManager` `VpayClient` uses — not a second implementation of the handshake — and exported from a `./stripe` entry point so `@vaam-apps/vpay-sdk` core keeps zero dependency on `stripe` (optional peer). Its own unit tests run against a real `node:http` server, as the rest of the package's do. **The authenticator is bound to `baseUrl`'s origin and refuses every other one** — a request addressed elsewhere is a `VpayConfigError` naming both origins, thrown before a token is minted — because stripe-node with `host`/`port`/`protocol` omitted addresses `api.stripe.com:443`, and an authenticator that wrote `Authorization` unconditionally would hand a merchant's live vpay bearer token to Stripe the moment one line of config was forgotten. A malformed `privateKey` and a non-absolute `baseUrl` are likewise `VpayConfigError` at **construction**, which matters more here than in `VpayClient`: a handshake that rejects mid-flight never settles through stripe-node (a defect this package pins). `sdks/stripe-compat` is the out-of-process half: **25 cases, 0 skipped**, the real `stripe@22.6.1` against a real stack, measured passing 2026-09-03 after the rebase onto Steps 4 and 5, and run by CI's `e2e (compose)` job; it fails rather than skips when no stack answers. Its Step-5b additions prove the money-moving refusal set (`capture_method: "manual"` and `transfer_data` on create, `application_fee_amount` on confirm, `capture_method: "automatic"` accepted), that `expand` survives stripe-node's indexed array encoding and is ignored, that a confirmed intent polls through to **`succeeded`** once the worker asks the rail, and that a delivered webhook verifies with the real package's `stripe.webhooks.constructEvent` (and is refused for a tampered body and a wrong secret). Together they are the only thing in this repository that has driven a `/v1` *resource* call from Node against a running `vpay-server`. 🟡 for what the Backend section's "Stripe-SDK compatibility" row lists: the rail and the receiver are both WireMock hosts, the `stripe-should-retry: true` direction unobserved, `stripe.events.list()` untested, no pin against a future `stripe`. Full ledger: [docs/flows/stripe-sdk-compat.md](flows/stripe-sdk-compat.md). |
| Customers in the merchant SDKs | 🟡 | **New 2026-09-06 (S4a).** `sdks/rust` `client.customers()` and `sdks/nodejs` `client.customers` both offer `create`/`retrieve`/`update`/`list`/`del`, landed in one PR per [ADR-0015](adr/0015-sdk-parity.md), with eleven parity rows. Three things had to be checked against each other rather than assumed at parity: the **update**'s three states (leave alone / set / **clear**), which Rust spells `Option<Option<String>>` and TypeScript spells `string | null | undefined` — each column's proving test asserts the resulting **body** (`email=` for a clear, the key absent for a leave-alone) rather than the type, because collapsing two of the three is a one-word edit in either language and makes a payer's email unclearable; `del` rather than `delete` in **both**, even though `delete` is legal in Rust, because the name is what a merchant looks up when they read one SDK's docs and write against the other; and `DELETE`, the first non-`GET`/`POST` verb either SDK sends, which carries an `Idempotency-Key` and **no body** — both halves pinned. 🟡 rather than ✅ for the reason every SDK resource row here is: **neither SDK has ever run this resource against a live vpay** — every server in their customer cases is a stub, and `backends/tests/integration/tests/customers.rs` drives the real one but not through either SDK. Dated ⛔/⛔ in [sdks/parity.md](sdks/parity.md), as the Checkout Session and account-holder rows are. `customer.created`/`customer.updated` are a second dated ⛔/⛔ there and are owned by the **vpay** maintainers rather than the SDK ones: the server emits neither, so a union entry would be a false claim about vpay |
| Checkout Sessions in the merchant SDKs | ✅ | **New 2026-09-04 (Step 9, lane 5).** `sdks/rust` `client.checkout().sessions()` and `sdks/nodejs` `client.checkout.sessions` both offer `create`/`retrieve`/`list`/`expire`, landed in one PR per [ADR-0015](adr/0015-sdk-parity.md). Proven against `wiremock` and a `node:http` stub, byte for byte, with the `create` bodies pinned as identical literals on both sides. **Since 2026-09-04 it is also run against a real server**: `examples/shop` creates every PaymentIntent and every Checkout Session through `@vaam-apps/vpay-sdk` against `vpay-server` in the compose stack, and lane 6's Cypress specs drive that path from a browser. `sdks/rust`'s own contact is still the in-process router and its stubs |
| Embedded Checkout in `@vaam-apps/vpay-stripe-js` | ✅ | **New 2026-09-04 (Step 9, lane 5).** `initEmbeddedCheckout({ fetchClientSecret })` frames `{checkoutBaseUrl}/e/{cs_id}?key={pk}#{client_secret}` and acts only on `message` events whose origin is the checkout app's and whose source is that frame; the frame is sandboxed `allow-scripts allow-same-origin allow-forms` — `allow-top-navigation` is withheld, which is what makes `vpay:redirect` a necessity rather than a convention. `retrieveCheckoutSession` reads the browser session route (where `payment_intent` is the expanded intent, per the integrator's 2026-09-04 ruling — so it returns a live **confirm** credential as well as a session-read one, documented on the type and in the README rather than designed away) and never rejects. 20 jsdom tests against a real `<iframe>`. **Run against vpay's own checkout page since 2026-09-04** by lane 6's `shop-embedded.cy.ts`, with two limits recorded there: the `vpay:redirect` top-level `assign` is intercepted by the spec (covered by this package's vitest instead), and no browser has been observed refusing a frame on vpay's CSP |
| A Checkout Session's secret in diagnostics | ✅ | **New 2026-09-04 (Step 9, lane 5).** Both merchant SDKs redact `client_secret` **and the fragment of a hosted session's `url`, which carries the same value**, from `Debug`/`util.inspect`, while leaving `Serialize`/`JSON.stringify` faithful. The `url` half was a leak the tests found rather than one they were written around: the first Node redaction hid the field and printed the URL verbatim, so `console.log(session)` still leaked it. The equivalent gap for `PaymentIntent` in `sdks/nodejs` is still open ([docs/sdks/parity.md](sdks/parity.md), 2026-09-03) — this step did not repeat it for a new object and did not close it for the old one |
| The client assertion's audience is its own setting (`assertionAudience` / `ClientBuilder::assertion_audience`) | ✅ | **New 2026-09-04 (Step 9, lane 5b), and it fixes a real defect lane 6 found: no merchant server that reaches vpay by an internal URL could authenticate at all.** Both SDKs signed the assertion's `aud` with the token endpoint they were about to POST to, and derived both from `baseUrl`. vpay's OP derives its issuer solely from `deployment.public_base_url` and `authenticate_client` accepts only `{public_base_url}/v1/oauth/token` or `{public_base_url}/v1/oauth`, so a merchant behind a compose service name, a private DNS name or a mesh address was refused with a bare `invalid_client` / `InvalidAudience` while its signature, `client_id`, `kid` and lifetime were all correct. The fix is a **third** setting, not a redefinition of either existing one — `audience` was already taken, in both SDKs, by the OAuth2 `audience` *request parameter* (`vpay:v1`). Both default to the token endpoint, which is right only when the two coincide. Proven by `sdks/rust/tests/op_conformance.rs`'s `the_real_verifier_refuses_a_client_that_reaches_vpay_internally_and_sets_no_audience` and `the_real_verifier_accepts_the_same_client_once_assertion_audience_is_set` — the real pinned `authkestra_op` verifier refusing and then accepting the same `Client`, the same registration and the same key — and, on the Node side, by `examples/shop/src/server/vpay.test.ts` against a real ephemeral server, which is verifier-*shaped* rather than the verifier itself (the standing `sdks/nodejs` gap, 2026-09-03). **ADR-0010-adjacent and not an amendment to it**: the decision in ADR-0010 does not change; what changed is that the SDKs stopped conflating two of its three strings. Recorded in [docs/flows/merchant-auth.md](flows/merchant-auth.md) |

**Decisions this work left to a maintainer — three of the four are now
settled by the server, 2026-09-02 (Step 1):**

- ~~**The token endpoint path.**~~ **Decided: `{public_base_url}/v1/oauth`
  is the issuer and `{issuer}/token` is the token endpoint** — exactly what
  both SDKs already defaulted to, so no SDK default changes.
  `vpay_api::op::issuer_for` is the single derivation in the workspace,
  `MerchantOp::new` and `vpay-server`'s `main` both call it, and
  `the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url` pins
  it. The discovery document is compared against what the SDK derived
  independently, over a booted server, by
  `the_jwks_and_discovery_documents_describe_this_process` — so a merchant
  who never fetches discovery lands on the same URLs as one who does.
- ~~**`audience=vpay:v1`.**~~ **Decided, and no longer "provisional in
  `resource_auth.rs`":** the string is `vpay_config::MERCHANT_AUDIENCE`,
  `Surface::Merchant.audience()` returns that constant, and a deployment
  whose merchant registration cannot target it refuses to boot
  (`ConfigError::MerchantMissingV1Audience`, proven by
  `a_merchant_client_that_cannot_target_the_v1_audience_is_rejected` and by
  `the_example_config_registers_its_merchant_for_the_v1_audience` on the
  real `config/application.yml`). The "keep the two constants equal"
  instruction this bullet used to give is structurally unnecessary now:
  there is one constant.
- ~~**Array encoding — still open, and still untestable.**~~ **Decided by
  the server, 2026-09-03: both spellings are accepted.** `vpay_api::form`
  decodes Stripe's indexed form (`k[0]=v`, what both SDKs send) and the
  unindexed form (`k[]=v`, what the curl examples use) into the same array,
  pinned by `both_array_spellings_produce_the_same_array`, with
  `array_indices_order_the_elements_and_holes_are_compacted` fixing what an
  index means. `create_payment_intent_body_decodes_to_the_params_the_sdk_encoded`
  and `confirm_body_decodes_to_the_params_the_sdk_encoded` carry the exact
  strings the SDK encoder emits. The decoder is hand-rolled rather than
  `serde_urlencoded`, because bracket nesting is not a shape that crate
  models; it refuses a duplicated scalar key rather than taking the last one,
  refuses a key used as both scalar and container, bounds nesting at 8, and
  is the reason `VpayForm`/`VpayQuery` exist at all.
- ~~**Whether ADR-0010's YAML-only merchant registry still stands**~~ now
  that `authkestra-op` 0.7.1 persists `token_endpoint_auth_method`/`jwks`.
  **Decided: kept.** `vpay_api::op::clients::YamlClientStore` resolves
  identity from `merchant_clients` in YAML and consults the database only
  for `disabled_clients`, which can subtract access but never grant it.
  The cost of that decision is unchanged and still real: merchant
  onboarding is a PR-then-deploy (ADR-0003, no hot-reload), and a rolling
  deploy has a window where old and new pods disagree about the client list.

**Updated 2026-09-03 (Step 2): the Rust SDK has now completed resource
calls, not only the handshake.** `payment_intents().create(…)`,
`.retrieve(…)`, `.list(…)`, `.confirm(…)` and `.cancel(…)` are exercised
against a real `vpay_api::router` and a real Postgres in
`backends/tests/integration/tests/payment_intents.rs`, and the wire object the
server renders is decoded by the shipping SDK's own type in
`the_merchant_sdk_deserialises_what_this_renders` — not by a copy of it
written for the test. **`.confirm(…)` succeeds only in the sense that it
receives the documented `501`**, which is what the demo's fifth step asserts
too. **The Node SDK's model is exercised against this renderer by nothing**:
`vpay_api::model`'s tests pin the object against the Rust SDK's fixture and
type only, and Node's parity rests on the SDK-to-SDK form-body tests
described above. ~~Three of the SDKs' eight resource endpoints — refunds,
events, balance — still have no route at all.~~ **Corrected 2026-09-05, and
re-counted 2026-09-06 on the rebased tree: two of fourteen.** `events.list`
has had a route since Step 5 (2026-09-03), `GET /v1/refunds/{id}` since issue
#45 and `GET /v1/account_holders` since issue #47; what is left with no route
is `refunds.create` and `balance.retrieve`. The "two of thirteen" this
sentence read until the rebase was measured on a tree without issue #47's
resource.

**Not done, stated plainly:** neither SDK has been exercised against a
*deployed* vpay. What changed on 2026-09-02 is that the server half of the
contract exists — `/v1/oauth/token`, discovery and JWKS are served — and the
**Rust** SDK completes the whole handshake and reaches `/v1` in
`backends/tests/integration/tests/merchant_token_flow.rs`, against a server
booted in-process by the test, in the implementer's single manual run
against a scratch database (header paragraph): never under Docker, never in
CI, never against a container. **`sdks/nodejs` has still never spoken to a
vpay of any kind** — all ~~126~~ **174 of its tests (re-measured 2026-09-05)** run against its own `node:http`
stub, and no integration test uses it. Neither SDK is published anywhere
(`publish = false` / `private: true`). *The reason given here used to be
"until a `/v1` resource exists for it to call"; ~~five of the eight now
exist~~ **twelve of the fourteen do, re-counted 2026-09-06 on the rebased
tree**, so the reason is different and narrower:* nothing either SDK calls can actually take a
payment, and `refunds.create` and `balance.retrieve` are still a `404`.
~~`confirm` answers `501`~~ **— stale since Step 3; `confirm` reaches the
rail and answers `200`/`409`/`502`** (HTTP surface row above). So
`examples/merchant-node` and `examples/merchant-curl` can authenticate,
create, retrieve, list and cancel against a running vpay, read an event and
read a refund, and get a `404` for creating a refund or reading a balance.

**Gap found 2026-09-03 (Step 5c), fixed the same day (`c40a137`).** Neither
SDK's `PaymentIntent` type had declared `client_secret`, even though `/v1`'s
own `create` and `retrieve` had rendered one since this step's migration
`0026` (`vpay_api::model::PaymentIntentWithSecret`, decision D2 — see
[docs/flows/browser-checkout.md](flows/browser-checkout.md)). The worktree
constraint that had left it unfixed (read-only on `backends/`/`sdks/` while
a security reviewer shared it) lifted the same day: `sdks/nodejs/src/types.ts`
now declares `client_secret?: string` (present on `create`/`retrieve`,
`undefined` on `list` items and `event.data.object`), and
`sdks/rust/src/model.rs` declares `client_secret: Option<String>` with
`#[serde(default)]` so it decodes to `None` where the server omits the key,
plus a hand-written `Debug` that redacts it. Proof: `sdks/nodejs/src/client.test.ts`
and `sdks/rust/src/model.rs`/`tests/{debug_redaction,resources}.rs` (decodes
when present, `None`/`undefined` when absent, never appears in `Debug`
output). The cast this gap had forced is gone too —
`frontends/tests/e2e/cypress/tasks/checkoutTasks.ts` reads
`intent.client_secret` directly now that the type declares it.

**Updated 2026-09-03 (Step 8, Lane F): a parity matrix and a machine check,
not just this narrative.** Every row above was, until this pass, prose —
accurate the day it was written, with nothing stopping it from drifting the
next time either SDK changed. [docs/sdks/parity.md](sdks/parity.md) (record:
[ADR-0015](adr/0015-sdk-parity.md)) now carries one row per capability
(the auth handshake, token cache and refresh margin, every `/v1` resource
operation including `events.retrieve` and `cancel`, idempotency-key
handling, error mapping including `request-id` and `stripe-should-retry`,
webhook verification, `client_secret` exposure, `Debug`/inspect redaction,
timeouts, retries, and the Stripe authenticator) and one column per merchant
SDK, checked on every `just verify` by `cargo xtask verify-sdk-parity`:
every `✅` cell's named test(s) must exist in that SDK's sources, and every
`⛔` must carry a `YYYY-MM-DD` date. As of this pass: **267 proving tests**
named across `sdks/rust` and `sdks/nodejs`, **24 dated gaps**, all owned by
"SDK maintainers." Real gaps this pass found by reading both trees, not
inferred from a file name: `sdks/nodejs` has `createStripeAuthenticator`
and `sdks/rust` does not (three rows, scoped as a follow-up by ADR-0010's
own amendment); `sdks/rust` never validates a token response's
`token_type`, so a non-`Bearer` grant would be accepted and presented as
one, while `sdks/nodejs` refuses it; neither SDK calls `GET
/v1/events/{id}`, though the server serves it; neither surfaces
`request-id` or reads `stripe-should-retry`, though `sdks/stripe-compat`
proves the server sends both; `sdks/nodejs`'s `PaymentIntent` is a plain
interface, so `console.log` prints a live `client_secret`, where
`sdks/rust`'s `Debug` redacts it. The matrix's own "Gap ledger" is the
complete, current list — this paragraph is a snapshot, not the record.
`@vaam-apps/vpay-stripe-js` gets its own table in the same document rather than a
third merchant column, per ADR-0015's decision 4: it authenticates a
payer's browser with a publishable key and a per-intent `client_secret`,
not a merchant with `private_key_jwt`, so no capability row is shared.
**Re-measured 2026-09-04 on the merged Step 8 gate branch: `verify-sdk-parity:
ok — 267 proving test(s) named in docs/sdks/parity.md all exist, 24 dated
gap(s)`** — no drift across the rebase and the five other lanes, and **none of
the 24 gaps was closed by this step.** Closing them is the SDK maintainers'
work, not the gate's.

**The gate reached CI on 2026-09-04, and not before** (Step 8 review
remediation, finding 2). ADR-0015's decision (3) says the matrix "fails the
build", and [the Step 8 plan](plans/2026-09-03-step8-production-gate.md) §3
spelled that out as "in `just verify` (and therefore CI's `self-checks`)". The
parenthesis was false: `.github/workflows/ci.yml`'s `self-checks` job ran
`verify-no-mocks`, `verify-status`, `verify-errors` and the `verify-docs`
report, and **not** `verify-sdk-parity` — so a `✅` naming a test somebody had
since renamed would have merged with CI green, and the only thing standing
between the matrix and decay was a developer remembering to run `just verify`.
The step is now in that job, placed before the `verify-docs` report; the
justfile's dependency list moved `verify-sdk-parity` ahead of `verify-docs` to
match, so its claim that "CI's `self-checks` job runs exactly this list, in
this order" can be checked by reading the two files side by side. The job's
display name is unchanged (`self-checks (no-mocks, status)`) because branch
protection pins it. **No CI run of this change exists yet** — the evidence is
`actionlint` clean on the workflow and a local `just verify`.

## What would have to be true to call this "an MVP"

*Re-answered item by item on 2026-09-04 (Step 8, the production gate). **No
item moved to met.** Four moved *within* themselves — item 2's callback clause,
item 3's crash-test clause, item 4's confirm race and item 5's SSRF residual —
and each is marked below with what changed and what still blocks it. The one
sentence that decides items 2, 3, 4 and 5 together is unchanged: **no real rail
has ever been called.***

*Re-answered again the same day for **Step 9** (hosted and embedded checkout).
**Item 6 — `just test-e2e` green against the compose stack — moved to met**,
the first item to move since item 1; **item 8 was added at the maintainer's
request and is answered without being met**; and item 5's webhook clause
narrowed. The list is **eight items** from here on. The sentence above still
decides items 2, 3, 4 and 5 — and item 8 with them.*

1. ~~Database schema + migrations, with the `one_charge_per_intent` unique
   index.~~ **Done.** `backends/migrations/0004_create-charges.sql:73`
   creates `one_charge_per_intent` as a plain unique index on
   `charges (payment_intent_id)`, proven to reject a second charge by
   `one_charge_per_intent_is_enforced_by_the_database` in
   `backends/tests/integration/tests/postgres_smoke.rs`. This item is about
   the schema existing and its constraints holding, not about the
   application using it — see the "Database schema / migrations (core)" row
   above for that distinction; the remaining items below are unaffected.
2. Both adapters making real HTTP calls, passing the shared conformance suite
   with the `#[ignore]`s removed. **Updated 2026-09-03 (Step 3): the literal
   words of this item are met; the item is not.** Both adapters make real
   HTTP calls, and the shared suite passes with **no `#[ignore]`s left at all**
   — 26 tests, 26 passed, 0 skipped, measured on the authoring machine
   (`just verify-ignored` now pins `expected_ignored := "0"`). What every one
   of those 26 talks to is a `wiremock/wiremock` container. **Neither MTN's
   nor Orange's real sandbox has ever been called**, so this item stays open
   in the sense that matters for taking a payment: what is proven is that the
   adapters implement the protocol `docs/flows/adapter-*.md` describes, not
   that those documents are right. Also unproven: the 401-after-a-good-token
   re-mint path on both rails; Orange's duplicate-submit idempotency (an
   assumption about the rail, not an observation); and ~~the callback path,
   because no callback route exists~~ **the callback path end to end: the route
   exists as of 2026-09-04 (Step 8, lane C) and is proven against WireMock
   (`provider_callback.rs`, 9 cases), but no real rail has ever called it, and
   Orange's `notif_token` is still not compared against the stored one.**
   `mtn_momo::refund` remains `NotImplemented` — see the token list above.
   ***Re-answered 2026-09-04 (Step 8): still open, and the count moved.*** The
   shared suite is now **28 tests, 0 skipped** (12 port cases over both rails
   plus 4 that are not rail-specific), the twelfth being
   `the_submit_tells_the_rail_where_to_call_back`. Every one of the 28 still
   talks to a `wiremock/wiremock` container, so **the literal words of this
   item remain met and the item remains unmet**, for exactly the reason it has
   always been unmet.
3. The worker's job loop, poll ladder and reconciler, with crash tests.
   **Updated 2026-09-03 (Step 4): substantially met, with two named gaps.**
   The loop, the ladder, the recovery table, the settlement transaction and
   the 24-hour `unresolved` escalation all exist and are proven by 20
   `worker_recovery` + 3 `worker_e2e` container-backed tests; a confirmed
   intent reaches `succeeded` without operator intervention. **The two gaps
   are stated rather than rounded off:** (a) the "crash tests" do not kill a
   process — they write the state each of the three documented kill points
   leaves and run the real handlers against it, which proves the recovery
   table and not the process's behaviour under a signal; (b) a rail answer
   contradicting an already-settled charge is classified by a tested pure
   function whose **call sites no test reaches**. And the rail behind every
   one of these observations is still a WireMock container.
   ***Re-answered 2026-09-04 (Step 8, lanes D and G): gap (a) is narrowed to
   one kill point; gap (b) is unchanged; the item is still not met.***
   `backends/tests/integration/tests/worker_kill9.rs` `SIGKILL`s the shipping
   `vpay-worker-bin` mid-status-query and the shipping `vpay-server`
   mid-`requesttopay`, and both charges settle exactly once — so two of the
   three kill points are now *caused* rather than written. **Kill point 1
   remains written, not caused**, and deliberately: there is no network call to
   interrupt at the moment before the reference is minted, so there is nothing
   for a signal to land during. Two clocks are simulated in that file (the dead
   worker's lease, and — since lane G — the crashed charge's age); nothing else
   is. **Orange is not exercised by the kill test at all**, and the rail is
   still a WireMock container. Lane G additionally fixed a defect this step's
   demo found: the worker was applying that same recovery table to a charge
   whose confirm was still running — see the confirm/worker race row.
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency,
   authenticated. **Updated 2026-09-03 (Step 3): confirm now reaches a rail
   and moves the intent — `processing` on a push rail, `requires_action`
   with a `next_action.redirect_to_url` on a redirect rail, `409
   charge_declined` when the rail refuses, `502` when it cannot be reached.
   Seven integration tests in `confirm_rails.rs` drive all four outcomes
   against real Postgres and WireMock containers. The item is still not met,
   and now for exactly two reasons rather than one:** the rail behind every
   one of those observations is a stub, and **nothing polls the charge
   afterwards** — a confirmed intent stops at `processing` forever, so
   "create + confirm" produces a payment that is never resolved.
   ***Updated 2026-09-03 (Step 4): the second of those two reasons is
   closed** — the worker polls the charge and drives the intent to
   `succeeded`, `failed`/`requires_payment_method`, or `unresolved` with an
   alert. **The first is not, and it is the one that decides this item:** the
   rail is a WireMock host, so what has been proven is that vpay resolves a
   payment correctly against a stub that behaves the way these documents say
   a rail behaves.*
   ***Re-answered 2026-09-04 (Step 8, lane G): a defect on this exact path was
   found and fixed, and the item is still not met.*** Step 8's demo produced a
   `500 api_error` on confirm in four of six walkthrough runs — the worker
   claiming the poll job the confirm had just committed, applying the
   crash-recovery table to a charge whose process had not crashed, and moving
   it out from under the confirm's own compare-and-swap. On a push rail the
   merchant was told the confirm failed and then sent a
   `payment_intent.succeeded` webhook; on a redirect rail a live order was
   killed as `provider_unavailable`. The fix is a minimum charge age in
   `recovery_step`, proven by a unit table and three `worker_recovery` cases,
   the last of which runs the shipping loop against the confirm's own write.
   **This makes create + confirm *correct* under contention; it does not make
   this item met**, and the reason is the same one it has been since Step 3:
   the rail is a stub. *The Step 2
   note follows, and its account of where confirm stopped is now history.*
   **(Step 2): create is done; confirm reaches the rail boundary and stops
   there, so this item is not met.**
   `POST /v1/payment_intents` is form-encoded, idempotent, authenticated and
   merchant-scoped, and writes a real row — with `create_then_retrieve_round_trips_through_the_sdk`,
   `a_replayed_idempotency_key_returns_the_same_object_and_no_second_row` and
   `a_post_without_an_idempotency_key_is_the_documented_400` behind it, all
   run against a real Postgres on 2026-09-03. Retrieve, list and cancel are
   there too. **`confirm` is where this item stays open:** it performs the
   crash-safe write ordering and then gets `501 not_implemented` from the
   adapter, so no payment intent has ever reached `processing`,
   `requires_action` or `succeeded`, and no rail has been called. This item
   asks for create **and** confirm; half of it is now real and the other half
   is blocked on item 2. *The paragraph that follows was written when the
   resource half had not started; its account of the credential model still
   holds and is kept for that.* **The *authenticated* half of this item is
   built; the resource half is now built as far as the rail.** As of 2026-09-02 (Step 1) a merchant can
   obtain an access token from `/v1/oauth/token` with `client_credentials` +
   `private_key_jwt` and carry it past the authentication boundary — see the
   "Merchant auth" and "Merchant OP" rows above for the six integration tests
   that cover it, and the header paragraph for the one thing that keeps those
   rows at 🟡 (they have run once, manually, never under Docker or in CI).
   **What an authenticated `/v1/payment_intents` request gets today is a 404**,
   and that is the honest answer rather than a placeholder: no payment-intent
   route, no handler, no idempotency implementation, no form-decoding of a
   create body. Nothing about creating or confirming a payment intent moved
   this pass. The credential-model history this item used to recount still
   holds: `merchant_api_keys` (migration `0008`) is dropped (migration
   `0009`) and the design it backed is reversed by
   [ADR-0010](adr/0010-merchant-auth-private-key-jwt.md) — there is no opaque
   key of any kind on `/v1`. The two decisions that were open on the wire
   contract are now decided *by the server*, not merely proposed by the SDKs:
   the token endpoint is `{public_base_url}/v1/oauth/token` and the audience
   is `vpay:v1` (see the Merchant SDKs section). The one ADR-0010 premise
   that shifted at `authkestra-op = "=0.7.1"` — that `SqlxOpStore::find_client`
   can now persist `token_endpoint_auth_method`/`jwks`, making a
   database-backed merchant registry buildable — was **resolved by keeping
   YAML**: `YamlClientStore` reads `merchant_clients` from the config file and
   the database only ever subtracts access via `disabled_clients`. That is
   ADR-0010's original choice, now implemented rather than merely recorded.
5. Signed webhooks with the two-step outbox. **Updated 2026-09-04 (Step 8,
   lane B): the one ⛔ this item carried is closed and the item is still not
   met.** Every delivery now goes through `vpay_worker::ssrf` — resolve once,
   classify every answered address, pin the connection to them, refuse a
   redirect — so the "no SSRF protection of any kind" row is retired and
   replaced by three named residuals (webhook delivery only; NAT64 refused
   fail-closed; the shared connection pool given up for the pin). What keeps
   the item open is unchanged and is not about SSRF: **no merchant endpoint has
   ever been POSTed to**, delivery is unordered, the ladder's 1 h / 6 h / 24 h
   rungs have never elapsed, and replaying an exhausted delivery is still a
   hand-written transaction.
   ***Re-answered 2026-09-04 (Step 9): one clause is now narrower, and the item
   is still not met.*** `examples/shop`'s `POST /api/vpay/webhook` is the first
   receiver in this repository's history that **acts** on a delivery rather than
   recording it in a WireMock journal: it verifies the signature with
   `@vaam-apps/vpay-sdk`, dedupes by event id, and marks an order `paid` — and lane 6's
   Cypress specs assert an order reaching `paid` only through it, with the
   payer's return page reading the shop's own database and taking no decision
   from the return trip. **It is still not a merchant endpoint**: `vpay-shop` is
   a container on the same compose network, permitted by
   `webhooks.allow_private_targets`, and it is code this repository wrote and
   tests. The other three clauses are untouched.
6. ~~`just test-e2e` green against the compose stack.~~ **Done 2026-09-04
   (Step 9, lane 6) — the first item on this list to move since item 1, and it
   moved because the recipe was fixed, not because a spec was weakened.**
   `just test-e2e` from nothing in the `vpay-ci` VM, unpatched: **exit 0, 11
   tests across four specs, 0 failing, 0 skipped**, in two Cypress passes
   (`checkout.cy.ts`, `dashboard.cy.ts`, `shop-hosted.cy.ts`, then
   `shop-embedded.cy.ts` with `VPAY_E2E_FRAMED=1`). **What that took is worth
   recording rather than celebrating:** the recipe used to bring up
   `compose.yml -f compose.e2e.yml` alone, which registers no merchant anybody
   holds a private key for, so every spec that mints anything answered
   `invalid_client` — **`checkout.cy.ts` could not have passed under
   `just test-e2e` since Step 5c**, and CI's `e2e` job was the only place that
   spec had ever run. It now depends on `gen-demo-keys` and adds
   `-f compose.demo.yml`. This item asks for a green gate and the gate is
   green; it does **not** say anything about money, and the rails behind all
   eleven tests are WireMock hosts. Nothing about flakiness was measured — one
   green run, `retries: 2` unchanged. What Step 8 added beside it is
   `just demo`, a different artefact: a human-read walkthrough, not a build
   gate. Its own state is in the "Local demo" row, and since Step 9 the merged
   branch's own three consecutive green runs exist.
7. `/dash/v1` login working end to end against a real database — issuing an
   access token, verifying it on a subsequent call, and rotating a signing
   key at least once. **Two of those three verbs now have an implementation,
   and it belongs to the *merchant* surface, not this one.** Tokens are
   issued and verified on `/v1` (item 4); a signing key is generated,
   loaded, announced in `oauth_signing_keys` and published at
   `/v1/oauth/jwks.json`. **Nothing here has ever performed a login**, and
   the gap is not a matter of wiring the same parts to a second router:
   there is no `/login` or `/authorize` route, no `SessionStore` (
   `authkestra-engine` is pinned without its `sql-postgres` feature, so no
   SQL-backed session store is compiled in at all), and
   `authkestra-op`'s authorization-code handler mints `aud = <client_id>`
   with no requested-audience path, which `Surface::Dashboard`'s
   `vpay:dash/v1` would reject on every call — a design question whoever
   builds this must answer first. See the "Dashboard auth" row. **And
   "rotating a signing key at least once" is still unmet in the sense this
   item means it:** `ensure_active_signing_key` will rotate the database
   record when a process boots with a different key, but `TokenManager`
   holds one key for the life of the process, so rotation is restart-based,
   nothing re-reads the key file, and no rotation has been observed on any
   deployment. Not part of "does this take payments"; listed here because it
   is the half of Phase 2 this pass deliberately did not build. **Unchanged by
   Step 8, deliberately:** the Step 8 plan names the dashboard as out of scope
   and says why — there is no `/dash/v1` API for it to read, so booting the
   screen would invite a reader to look at something that cannot show the
   payments just made (`docs/runbooks/demo.md` §6). Nothing about a login moved.

8. **A hosted page for driving payments on the web: one in-iframe version,
   one fully hosted page — "we need that before prod."** *(Added to this list
   2026-09-04 at the maintainer's request, verbatim, and answered by Step 9.)*
   **Both pages exist, both have been driven by a real browser, and this item
   is still not met for the purpose it names — going to production.** What is
   built: a `checkout.session` object with a hosted and an embedded mode
   (`cs_…`, migration `0028`); `frontends/apps/checkout`, a Next app vpay
   serves, in French and English, with `frame-ancestors` derived per merchant
   from a new `checkout_origins` list; the return trip for a redirect rail,
   which closes browser-checkout's D4 at both ends; `initEmbeddedCheckout` in
   `@vaam-apps/vpay-stripe-js` and `checkout.sessions` in both merchant SDKs; a fourth
   image, a Helm workload behind `checkout.enabled`, and a demo shop
   (`examples/shop`) that a payer can actually buy from. Lane 6 drove all of it
   in Chrome: MTN inside the frame, Orange breaking out to the rail's page and
   back, an order reaching `paid` only from the shop's own verified webhook,
   and an unregistered framer refused. **What stands between that and prod, in
   the order it would bite:** (a) every rail in every one of those runs is a
   WireMock container — the sentence that decides items 2, 3, 4 and 5 decides
   this one too; (b) **no browser has been observed enforcing vpay's
   `frame-ancestors`** — Cypress strips the header, so it is asserted as sent,
   and the refusal a browser was seen performing is the page's own origin
   check, which is a second lock and not the header; (c) this step added a
   **second unauthenticated surface** (the checkout app, plus three new
   `/v1/browser` reads) with the same ingress rate-limiting requirement
   browser-checkout's D5 already stated and nothing in this repository
   enforces or checks; (d) **no pod has ever run** the checkout Deployment, and
   its path-prefix Ingress shape has been run by nobody; (e) the demo shop's
   `ZenStackShopStore` — the code path every order actually goes through — has
   **no automated coverage of its own**; and (f) `checkout_not_configured`
   answers `500` where a `503` would be truthful, which is an ADR-level change
   left to the maintainer. The maintainer's sentence asked for the pages before
   prod; the pages are here, and the list of what "before prod" still means is
   above.

**Nothing in Step 8 moved an item on this list to met, and that is the honest
summary of the step.** What it moved is the *reasons*: four of the seven items
had a clause that was false or too broad by the end of 2026-09-04, and each is
struck and corrected above rather than quietly rewritten. The single fact that
keeps items 2, 3, 4 and 5 open is one fact, not four: every rail and every
webhook receiver in this repository's entire history is a `wiremock/wiremock`
container.

**Step 9 moved one item to met — item 6, the e2e gate — and added item 8,
which it answers without meeting.** That is the honest summary of this step:
the thing the maintainer asked for is built and a browser has walked it end to
end, and the one fact above is as true of Step 9 as of every step before it.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
