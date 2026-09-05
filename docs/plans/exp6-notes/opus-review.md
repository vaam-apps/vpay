# exp6 (opus): sabotage review of `verify-links` / `verify-citations`

2026-09-05. Reviewed `git diff 2b37f47..2a97e53` on
`claude/exp6-docs-links-opus`; the implementation's own account is
[opus.md](opus.md). Everything below was run in this worktree on the pinned
toolchain (`rust-toolchain.toml`, 1.95.0), `CARGO_BUILD_JOBS=4`.

Two notes on how this document is written, because the gate it reviews reads
it:

- The invented ids used as mutations are spelled in words ("eleven nines", "a
  pull request numbered 99999") rather than as digits. `verify-citations`
  would resolve them and fail, and the alternative — a fourth and fifth entry
  in `CITATIONS_THAT_ARE_NOT_CLAIMS` — would loosen the constant that its own
  test pins to one id in three mutation records. Spelling them out costs
  nothing and keeps the exemption list arguable. **This is a real cost of the
  design, not a complaint about it:** a document cannot report a citation
  mutation in the notation the mutation used.
- Adding this file moved the counts. Every number below is measured with it
  present.

## 1. Verifying the implementer's claims

Every claim in `opus.md` was re-run rather than read.

| claim | verdict | how |
|---|---|---|
| 114 tracked `*.md` scanned | **true** | `git ls-files '*.md' \| wc -l` → 114 at `2a97e53` |
| 672 repository links checked | **true, and independently corroborated** | `cargo xtask verify-links` → 672. An independent oracle written against **markdown-it-py 3.0.0** (a real CommonMark implementation), applying the same skip/strip/resolve rules, reports **672** — exact agreement |
| 5 broken links found on `master` | **true, and independently corroborated** | the same oracle on a detached worktree at `2b37f47` reports **5 broken**, and they are the five the notes name, file for file |
| 5 fixed | **true** — but see finding **F1**; they were quotations | |
| 39 unique ids resolve (24 runs, 14 PRs, 1 issue) | **true** | `cargo xtask verify-citations` re-run live → `39 unique id(s) … all resolve against vaam-apps/vpay`, 24 `run`, 14 `PR`, 1 `issue` |
| 90 → 126 xtask tests, 0 skipped | **true** | `cargo nextest run -p xtask` → `126 tests run: 126 passed, 0 skipped` at `2a97e53` |
| `verify-links` added to CI's `self-checks` job | **true** | `.github/workflows/ci.yml` gains a `verify-links` step between `verify-sdk-parity` and `verify-docs`, matching the justfile comment's order exactly. `actionlint` (`/home/selast/go/bin/actionlint`) exits 0 |
| the four-backtick fence regression | **true, and the number is exact** | deleting the info-string clause from `fence_marker` makes the gate print `ok — 591` instead of `672` and fails `a_backtick_run_whose_info_string_holds_backticks_opens_no_fence` |
| the run pattern is any standalone 11-digit token | **true, but its justification is not** — finding **F3** | |
| an uncued `#n` is deliberately unchecked | **true**, and `an_uncued_hash_number_is_not_a_citation` pins the tree's actual set | |
| the pasted citation run | **stale** — finding **F6** | the per-id counts in `opus.md` are from before that file grew (`33929374661` 35 vs 40 measured, `PR 17` 2 vs 5). The 39-unique total is right |
| `just test-rust` 1202/1202 | see §4 | |

The environment finding `opus.md` reports (a first `just test-rust` in a fresh
worktree fails the Node-SDK signature parity test with `tsc: not found`,
because `test-rust` does not depend on `install-node`) is real and is
**unrelated to this change**. `node_modules/` and `sdks/nodejs/dist/` are
present in this worktree, so the runs below do not hit it.

## 2. Mutations

Each applied to the branch head, the named check run, then restored;
`git status --porcelain` empty after every one. "the gate" means `cargo xtask
verify-links` or `verify-citations` run over the real repository, rebuilt
first — `cargo nextest run -p xtask` does **not** rebuild `target/debug/xtask`,
which is a trap worth writing down.

| # | mutation | caught by | result |
|---|---|---|---|
| 1 | existence check always succeeds (`if false && !files.contains(…)`) | 4 unit tests | `a_file_that_is_present_but_untracked_does_not_satisfy_a_link`, `a_fragment_does_not_excuse_a_missing_file`, `a_reference_definition_is_checked`, `a_backtick_run_whose_info_string_holds_backticks_opens_no_fence`. **Not** by the gate, and cannot be: the tree has no broken link to miss |
| 2 | stop masking fenced blocks | 2 unit tests **and** the gate | `a_link_inside_a_fenced_block_is_not_a_link`, `an_inner_fence_with_an_info_string_does_not_close_the_block`; the gate exits 1 with 8 false failures |
| 3 | stop parsing reference definitions | 1 unit test | `a_reference_definition_is_checked`. The gate does not notice — there is not one reference definition in the tree, which `opus.md` discloses |
| 4 | break `#fragment` / `:line` stripping | 1 unit test **and** the gate | `a_fragment_and_a_line_suffix_resolve_to_the_file_itself`; the gate exits 1 with 15 false failures |
| 5 | delete `fence_marker`'s info-string-backtick clause | 1 unit test | `a_backtick_run_whose_info_string_holds_backticks_opens_no_fence`; the gate silently drops to 591 links. This is `opus.md`'s own M4, reproduced exactly |
| 6 | remove `verify-links` from `just verify` | **nothing** | expected, and pre-existing: no gate checks the justfile against `.github/workflows/ci.yml`, which the justfile's own comment admits ("the only thing keeping this comment honest is someone reading the workflow beside it"). CI would still run the step, so the branch-protection gate holds |
| 7 | `verify-citations` prints "skipped" and exits 0 when `gh` is missing | **nothing, as delivered** — finding **F2**; a guard now exists | |
| 8 | one broken relative link appended to `docs/roadmap.md` | the gate | exit 1, `docs/roadmap.md:1388: ../nope/does-not-exist.md -> nope/does-not-exist.md` |
| 9 | an invented run id (eleven nines) and a pull request numbered 99999 in `docs/roadmap.md` | the gate | exit 1, both reported `HTTP 404` with `docs/roadmap.md:1388` |
| 10 | untracked `scratch.md` holding a broken link | correctly **not scanned** | gate still `ok — 672 … 114` |
| 11 | link to a file present on disk but never staged | the gate | exit 1, `docs/roadmap.md:1388: untracked-target.md -> docs/untracked-target.md`. This is the right answer and the reason `git ls-files` is used |

Additionally, and not as mutations:

- **`gh` genuinely absent** (`PATH` holding only `git`): exit **1**,
  `verify-citations needs the GitHub CLI and cannot run without it … This
  command never skips`. Correct.
- **`gh` present but unauthenticated** (`GH_TOKEN` garbage): exit **1**,
  `gh: Bad credentials (HTTP 401) … Run 'gh auth status'`. Correct.
- **The 403/429 abort is reachable and reads correctly.** Simulated with a
  `gh` shim on `PATH` that answers every `api -i` with `HTTP/2.0 403`: the run
  stops at the first id with `GitHub refused … rate limited or out of scope
  for this token. Nothing was concluded about the remaining citations`. On a
  normal run it is not reachable — the command makes 40 authenticated
  requests against a 5 000/hour limit — which is the right side of the
  trade-off to be on.

## 3. Findings

| # | severity | where | fixed |
|---|---|---|---|
| F1 | correctness / misleading-claim | `docs/plans/step8-notes/lane-c.md`, `lane-h.md`, `docs/status.md` | yes |
| F2 | rule-break (an unguarded "never skips") | `.xtask/src/main.rs` `verify_citations` | yes |
| F3 | robustness / misleading-claim | `.xtask/src/main.rs` `run_id_citations` | yes |
| F4 | robustness (latent) | `.xtask/src/main.rs` `mask_non_links` | documented, not fixed — reasoning below |
| F5 | nit (a false failure) | `.xtask/src/main.rs` `ancestor_directories` | yes |
| F6 | nit (stale measurement) | `docs/plans/exp6-notes/opus.md` | yes |

### F1 — the five "broken links" were quotations, and the fix altered them

All five sit inside blockquotes that quote Markdown belonging in another
directory, and every destination was **already correct where that text
lives**:

| notes site, as written | the applied text |
|---|---|
| `lane-c.md:175` `(adapter-orange-money.md)` | `docs/flows/reconciler.md:173` |
| `lane-c.md:181` `(reconciler.md)` | `docs/flows/crash-safety.md:320` |
| `lane-c.md:189` `(reconciler.md)` | `docs/flows/crash-safety.md:320` |
| `lane-h.md:289` `(flows/provider-port.md)` | `docs/status.md:1449` |
| `lane-h.md:332` `(../reference/vpay-api.md)` | `docs/flows/reconciler.md:95` |

`lane-c.md:108` says "Each is quoted verbatim as it stands today, with the
replacement". After the fix it is not, and a reader who pastes the replacement
as instructed writes a link `verify-links` will then reject. `docs/status.md`
also described the fifth as "a `../` one level short", which is not what it
is — it is the same quotation mismatch as the other four.

**Fixed by disclosure, not by reverting.** The rewrites stay: a blockquote is
a link to any Markdown reader whatever the author meant, and reverting
re-breaks the gate. What changed is that each of the three quotation sites now
carries a dated note naming the applied text and where to copy it from, and
`docs/status.md`'s bullet states the cost instead of filing five clean wins.

The general lesson is worth keeping: **the first thing a new documentation
gate finds is usually a document about documents.** A quoted patch is not a
claim this repository makes about itself, and the gate cannot tell.

### F2 — "it never skips" had no guard

`verify_citations`'s doc comment leads with the property, `justfile` repeats
it, and `AGENTS.md` now does too. The behaviour was correct as delivered
(measured above). Nothing failed if it were deleted: all thirteen citation
tests were offline pattern tests, and mutation 7 — replacing the `?` on
`github_repository` with `println!("skipped"); return Ok(())` — left
126/126 green.

Fixed: `verify_citations_via(root, gh)` takes the CLI's name so a test can
pass one that is not on `PATH`, and `classify_gh_status` is the pure half of
`gh_status`. Two tests,
`a_missing_gh_fails_the_gate_rather_than_skipping` and
`a_refused_request_stops_the_run_rather_than_reporting_a_missing_id`. With the
fix in place, mutation 7 fails the first of them.

### F3 — "every eleven-digit number in the tree is a run id" is false

That sentence is the whole justification for widening the pattern away from
the brief's `run <11 digits>` cue, and it is true of tracked Markdown, not of
the tree:

| token | where | what it is |
|---|---|---|
| `01753401600` | `backends/crates/vpay-worker/src/signing.rs:243`, `sdks/rust/src/webhooks.rs:44,347,353,354` | a zero-padded webhook timestamp |
| `01700000100` | `sdks/nodejs/src/webhooks.test.ts:180,198` | the same, in the Node SDK |
| a French MSISDN (`+33` and a nine-digit mobile number, eleven digits with no separators) | `frontends/apps/checkout/src/lib/msisdn.test.ts:32` | a phone number |

**This document is the proof, and it cost a gate failure to get.** The first
draft of the table above printed that MSISDN as digits.
`cargo xtask verify-citations` then failed with

```text
xtask: 1 cited id(s) do not exist in vaam-apps/vpay. …
  - run 336…678 does not exist (HTTP 404), cited at
    docs/plans/exp6-notes/opus-review.md:151
```

(The digits are elided in that transcript for the same reason they are elided
in the table: pasting the real output back in makes this file fail the gate
again, which is the second time it did so while this paragraph was written.)

— a true sentence about a test fixture, reported as a false claim about this
repository's history. It is spelled in words above for that reason. The two
zero-padded timestamps *are* printed as digits, two rows up, and the same run
ignores them: that is the fix in this branch working on the document that
describes it.

Only `*.md` is scanned, so the gate is green on the rest of the tree; but
`docs/flows/webhooks.md:5`
already writes `t=1753401600` in prose, and the padded form is precisely what
`sdks/rust/src/webhooks.rs` documents as a signing hazard. One paste and the
gate tells an author that a true statement is a false citation — the failure
direction that gets a gate switched off.

Fixed by refusing a leading zero. A run id is a decimal integer and is never
zero-padded, so this can only remove a false positive and can never miss a
real run. **The MSISDN case is not fixed** and the doc comment says so: an
eleven-digit phone number in a document would still be looked up. That is the
residue of the widening, and it is now written down where the widening is
argued for rather than only in its favour.

### F4 — the four-backtick bug has siblings, in the other direction

`fence_marker`'s info-string clause is the mask-too-much failure. Three
regions are unmasked, which is the mask-too-little failure — a `[a](b)` in one
is checked and reported broken when it is not a link:

- a fence inside a blockquote (`> ```): `fence_marker` trims spaces only;
- a four-space indented code block: `fence_marker` correctly refuses to
  *open* on that indent, but nothing masks the body;
- HTML `<pre>`/`<code>`: only `mask_html_comments` would see it, and it looks
  for `<!--`.

**Measured: none of the three is in the tree** —
`git grep -nE '^ {0,3}> *(```|~~~)' -- '*.md'`,
`git grep -nE '^ {4,}.*\]\(' -- '*.md'` and `git grep -n '<pre' -- '*.md'` are
all empty.

**Not fixed, deliberately.** Masking a blockquoted fence needs blockquote
state threaded through the whole masker, and building that for a case the
repository does not contain is how a lexer acquires the bug it was meant to
prevent. The three commands are recorded in `mask_non_links`'s doc comment so
the next person re-measures instead of trusting a sentence.

### F5 — a link to the repository root was reported broken

`resolve_against` folds `[the repository](../)` written in `docs/` to the
empty string; `ancestor_directories` derived no such entry, so neither set
contained it. Latent — no such link exists today — but a false failure, so
fixed and guarded by `a_link_to_the_repository_root_resolves`.

### F6 — the pasted citation run is stale

`opus.md` pastes a run whose per-id counts predate its own growth. Corrected
in place with a dated note rather than silently replaced.

## 4. Gates, measured after the fixes

Run in this worktree, pinned toolchain, `CARGO_BUILD_JOBS=4`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `VPAY_REQUIRE_NODE=1`.

| command | result |
|---|---|
| `just fmt-check` | ok |
| `just clippy` (`cargo clippy --workspace --all-targets -- -D warnings`) | ok — the workspace includes `.xtask`, so the four new tests are linted too |
| `just verify` | ok — five gates: `verify-no-mocks`; `verify-status` (1 unimplemented item); `verify-errors` (15 types, 14 `#[from]` variants); `verify-sdk-parity` (342 proving tests, 26 dated gaps); **`verify-links` — 676 links in 115 files** |
| `just docs-check` | ok — `verify-status` and `verify-links`, no echo |
| `just docs-check-citations` | ok — **39 unique ids** (24 runs, 14 PRs, 1 issue) over 115 files, all resolving against `vaam-apps/vpay` |
| `cargo nextest run -p xtask` | **130 run, 130 passed, 0 skipped** (126 as delivered) |
| `just test-rust` | **1206 run, 1206 passed, 0 skipped** (1202 as delivered, +4 here) |
| `just test-doc` | see below |
| `just verify-ignored` | see below |
| `just lint-web`, `just test-web`, `just deny` | see below |
| `actionlint` (`/home/selast/go/bin/actionlint`) | exit 0 |
| `just ci` end to end | see below |

Counts moved because this review's own notes are a tracked Markdown file: 114
files / 672 links as delivered, 115 / 676 with this document in the index. The
independent markdown-it-py oracle agrees with the gate at every step of that
(672 → 673 → 676).


## 5. Verdict

**Would this have been safe to merge without the review? Yes, narrowly — and
it would have merged three claims that are not true.**

The mechanism is sound and unusually well tested for a first pass. Ten of the
eleven mutations were caught, every count in the notes reproduces, an
independent CommonMark implementation agrees with the parser to the link, and
the one gap the implementer found by counting (the four-backtick fence) is the
subtle one — most link checkers in this repository's position would have
shipped 591 links and called it 672. Nothing here is a hard-coded success, a
vacuous test, or a gate that passes by finding nothing.

What would have merged with it:

1. **Five verbatim quotations silently altered**, in a file that says in as
   many words that they are verbatim, with `docs/status.md` recording it as
   four of one kind and one typo. This is the finding that matters, because
   it is the failure mode CLAUDE.md names: the document was changed to fit the
   gate, and the change was written up as a clean win. It is small and
   recoverable — the worst outcome is a broken link that `verify-links` then
   catches — but it is the one place the review changed what the repository
   says about itself.
2. **"It never skips" as prose with nothing behind it.** The behaviour was
   right; a one-line edit would have made it wrong with 126/126 green. For the
   single property that CLAUDE.md and AGENTS.md both put first, that is a
   rule-break rather than a nit.
3. **A false justification for the widened run-id pattern.** "Every
   eleven-digit number in the tree is a run id" is the sentence the whole
   widening rests on, and the tree contains three counterexamples. The gate
   was green only because it reads Markdown; this review's own notes tripped
   it on the first draft, twice.

None of the three is a gate bypass. `verify-links` and `verify-citations`
would both have done their job on the day they landed. The verdict is
therefore "safe, but three claims short of honest" — and in a repository whose
stated failure mode is looking more finished than it is, that is exactly the
distance worth closing before merge.


## 6. What this review did not check

- **Anchors and external URLs**, which the gate does not check either. Nothing
  here proves a `#fragment` names a heading that exists.
- **That a cited id supports the sentence around it.** The gate proves 39 ids
  exist. Whether run `33929374661` did what the paragraph says it did is still
  only checkable by a human, and this review did not check it.
- **Cross-repository citations** (`authkestra#287`), out of scope by design.
- **The `verify-citations` API path shapes against a repository where an id is
  an issue rather than a pull request.** `CitationKind::Pull` resolving
  `/pulls/{n}` and 404ing for an issue is asserted by a unit test on the path
  string, not against GitHub.
- **`just ci`'s web half in isolation**; it was run only as part of the full
  `just ci` below.
- **Whether a scheduled CI job should run `verify-citations`.** `opus.md`
  leaves it as a maintainer decision and this review agrees it is one.
