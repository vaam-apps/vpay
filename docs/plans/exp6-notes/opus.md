# exp6 (opus): making `just docs-check` a real gate

2026-09-05. Branch `claude/exp6-docs-links-opus`, base `master` `2b37f47`.

## The defect, measured

On `master`:

```
$ just docs-check
cargo xtask verify-status
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
note: link checking is not implemented yet — see docs/status.md
$ echo $?
0
```

113 tracked Markdown files link to each other and to source files. Nothing
proved those links resolved. The same documents cite GitHub Actions run ids
and pull request numbers as evidence — `run 33929374661` appears 35 times —
and nothing proved those ids existed.

This was not an unknown gap. `docs/plans/step9-notes/release-claims-review.md`
had already measured both by mutation and left them open as a maintainer
decision:

| | mutation | result on `master` |
|---|---|---|
| M2 | `33929374661` → `39999999999` throughout `runbooks/release.md` | "not caught by anything — all five green" |
| M3 | break `../adr/0014-builder-host-musl-triple.md` | "not caught by `just docs-check`, which prints `note: link checking is not implemented yet` and exits 0" |

Both rows now carry a dated correction, and both mutations are re-run below.

## What was built

### `cargo xtask verify-links` — a gate

In `just verify` (fifth gate), `just docs-check`, `just ci`, and CI's
`self-checks` job. The justfile comment and `.github/workflows/ci.yml` moved
in the same commit, for the reason the `verify-sdk-parity` comment beside them
already records.

Sources are `git ls-files`, not a directory walk, so an untracked scratch file
can never satisfy a link. It parses inline links, image links and reference
definitions, with fenced code blocks, inline code spans and HTML comments
masked out first; skips `http(s)://`, `mailto:` and bare `#anchor`; and for
everything else strips a `#fragment` and a trailing `:line`/`:line:column`,
decodes percent escapes, resolves against the linking file's own directory,
and requires a **tracked** file or directory. Angle-bracketed targets
(`[x](<a file.md>)`) parse. Failures read `file:line: target -> resolved path`.

**Deliberately not checked, and the function's doc comment says so:**

- **`#anchor` fragments.** Resolving one means agreeing with GitHub's
  heading-slug algorithm — emoji handling, duplicate suffixes, `<a name>` —
  and disagreeing with it silently turns a correct link into a build failure.
  The fragment is stripped, so `money.md#rounding` proves the file exists and
  says nothing about the heading.
- **`http(s)` URLs.** They need the network, they go stale for reasons outside
  this repository, and a gate that fails because someone else's site is down
  gets disabled. `verify-citations` is the deliberate exception: it resolves
  ids that are claims about *our own* history.
- **`mailto:` targets.**
- **Reference *usages*** (`[text][label]` with no definition). Only the
  definitions are resolved; a dangling label renders as literal text, which a
  reader sees.

### `cargo xtask verify-citations` — a gate that needs the network

`just docs-check-citations`. Not in `just ci` and in no CI job: a required
check that fails when the GitHub API rate-limits is a check people learn to
re-run without reading.

- **A run** is any standalone eleven-digit number, plus `actions/runs/<n>` in a
  URL naming this repository. **This is wider than the brief's `run <11
  digits>`, deliberately.** This tree writes runs in lists — ``Runs
  `33772512791`, `33784613048`, `33789060270`, `33792230539` `` — where a
  cue-word rule checks the first and ignores the three places a wrong id would
  actually hide. ~~Every eleven-digit number in the tree is a run id.~~
  **Corrected 2026-09-05 (review, finding F3): every eleven-digit number in
  tracked *Markdown* is a run id; the tree also holds two zero-padded webhook
  timestamps and a phone number.** A leading zero is refused since the review
  — a run id is never zero-padded — and the phone-number case is a live false
  failure, stated rather than fixed.
  `:sha-33929374661`, `1.33929374661` and twelve-digit numbers are not
  standalone and do not count.
- **A pull request** is `#n` whose nearest preceding word is `PR`, `PRs`,
  `pull`, `pulls` or `pull request(s)`, plus any `#n` continuing such a run
  through nothing but separators (`PRs #16–#17`, `PRs #23, #24`, `PR #27 and
  #28`); plus `pull/<n>` in a URL naming this repository. Resolved against
  `/pulls/{n}`, which 404s for a plain issue — so an `#11` written with a
  pull-request cue would fail, because 11 is an issue.
- **An issue** is the same, cued by `issue`/`issues`, plus `issues/<n>`.
  Resolved against `/issues/{n}`, which answers for pull requests too.

**Why a cue is required.** A rule that read every `#n` as a citation would
fail this repository's correct prose. The uncued `#n` in the tree are:
`Order #42` and `Order #1234` (example payloads in `docs/runbooks/`),
``Commit `#7` `` and `` `237c716` (#1, CLI/env config) `` (commit *ordinals* in
`docs/roadmap.md` — "Seven commits on `master`" — not pull requests), and
`AGENTS.md open question #4` in ADR-0009. `PKCS#8` and `authkestra#287` are
excluded by the character before the `#`; a `#n` that begins a line is a
heading; `#9-the-known-flake` is an anchor. All of these are in
`an_uncued_hash_number_is_not_a_citation` and its neighbours.

**The cost of that, stated rather than hidden:** an id cited *only* without a
cue is not checked. Today that set is empty — every bold `**#17**` in
`docs/roadmap.md`'s third addendum is also written `PR #17` in the same file,
and the check is deduped by id, so all of them resolve.

**Cross-repository references are out of scope.** `authkestra#287` and
`github.com/marcjazz/authkestra/issues/185` are claims about somebody else's
tracker; resolving them against `vaam-apps/vpay` would ask GitHub the wrong
question and answer confidently. A URL only counts when its `owner/repo` is
this repository — the live name from `gh`, plus the two historical ones
(`vaam-store/vpay`, `vymalo/vpay`; the git remote and the resolved name still
disagree, and GitHub redirects both).

**It never skips.** Without `gh`, or unauthenticated, it fails and says which.
(Nothing *proved* that until the review: every citation test was an offline
pattern test, and a mutation printing "skipped" and returning `Ok(())` left
126/126 green. Guarded now by
`a_missing_gh_fails_the_gate_rather_than_skipping` — finding F2.)
A 403 or 429 stops the whole run rather than reporting the remaining ids as
missing, so a rate limit can never send somebody to delete true claims.

**Three exemptions**, in `CITATIONS_THAT_ARE_NOT_CLAIMS`, all of them
`39999999999`: in `release-claims.md`, in `release-claims-review.md`, and in
this file. All three are mutation records, and a document cannot say
"substituting `39999999999` leaves every gate green" without printing an id
that does not exist. A constant in
the source rather than a marker in the prose, because a marker
(`<!-- verify-citations: ignore -->`) is invisible to a reader and can be
sprayed over a document by anyone who wants the gate quiet; a pair here is a
code change that shows up in review, and it is scoped per file — the same
eleven digits in `runbooks/release.md` are still checked, which mutation M2
below proves.

## Counts

| | |
|---|---|
| Markdown files scanned | **114** (`git ls-files '*.md'`) — 113 on `master`, plus this notes file. **115 since the review added `opus-review.md`** |
| Repository links checked | **672** (**673** with the review's notes) |
| Broken links found | **5** |
| Broken links fixed | **5** |
| Links skipped as out of scope | 90 bare anchors, 5 `http(s)`/`mailto` (counted separately; not part of the 672) |
| Unique cited ids resolved | **39** — 24 workflow runs, 14 pull requests, 1 issue |
| False citations found | **0** (plus the 2 exempted mutation records) |

### The five broken links, and what each was

All five were in `docs/plans/step8-notes/`. None named a file that was never
written, so nothing became plain text — each was repointed at the file it
meant.

| file:line | as written | resolved to | the real target |
|---|---|---|---|
| `lane-c.md:175` | `adapter-orange-money.md` | `docs/plans/step8-notes/adapter-orange-money.md` | `../../flows/adapter-orange-money.md` |
| `lane-c.md:181` | `reconciler.md` | `docs/plans/step8-notes/reconciler.md` | `../../flows/reconciler.md` |
| `lane-c.md:189` | `reconciler.md` | `docs/plans/step8-notes/reconciler.md` | `../../flows/reconciler.md` |
| `lane-h.md:289` | `flows/provider-port.md` | `docs/plans/step8-notes/flows/provider-port.md` | `../../flows/provider-port.md` |
| `lane-h.md:332` | `../reference/vpay-api.md` | `docs/plans/reference/vpay-api.md` | `../../reference/vpay-api.md` |

~~The first four are one mistake: the notes quote a passage out of a
`docs/flows/` document into a blockquote and keep the quoted document's own
relative path, which is correct there and wrong here.~~ **Corrected 2026-09-05
(review, finding F1): all five are that, `lane-h.md:332` included — its
`../reference/vpay-api.md` is exactly what `docs/flows/reconciler.md:95`
contains, not a `../` one level short.** Display text is unchanged, so the
quotes still read as the originals do — but the **destinations are no longer
verbatim**, and `lane-c.md:108` said they were. Each site now carries a dated
note naming the applied text; see [opus-review.md](opus-review.md) §3, F1.

### The citation run, in full

**Stale, corrected 2026-09-05 (review, finding F6).** The per-id counts below
are from before this file finished growing; re-measured on the reviewed head
they read `run 33929374661 (40)`, `PR 17 (5)`, `PR 24 (5)` and so on, over
**115** files. The line that matters is unchanged — **39 unique ids, 24 runs,
14 pull requests, 1 issue, all resolving** — and is re-run in
[opus-review.md](opus-review.md) §4. The transcript is left as it was measured
rather than edited to look current.

```
$ just docs-check-citations
cargo xtask verify-citations
  ok    run 31317876404 (1 citation(s))
  ok    run 31319267218 (1 citation(s))
  ok    run 33618568372 (1 citation(s))
  ok    run 33626567174 (2 citation(s))
  ok    run 33646048616 (2 citation(s))
  ok    run 33647189156 (11 citation(s))
  ok    run 33650294682 (2 citation(s))
  ok    run 33772512791 (5 citation(s))
  ok    run 33784613048 (5 citation(s))
  ok    run 33789060270 (5 citation(s))
  ok    run 33792230539 (6 citation(s))
  ok    run 33792230584 (8 citation(s))
  ok    run 33802515513 (1 citation(s))
  ok    run 33817293354 (1 citation(s))
  ok    run 33846132186 (1 citation(s))
  ok    run 33849098945 (1 citation(s))
  ok    run 33894388991 (8 citation(s))
  ok    run 33898736618 (5 citation(s))
  ok    run 33912330063 (5 citation(s))
  ok    run 33918831901 (5 citation(s))
  ok    run 33929374661 (35 citation(s))
  ok    run 33929374663 (14 citation(s))
  ok    run 33934371223 (1 citation(s))
  ok    run 33935680386 (2 citation(s))
  ok    PR 14 (1 citation(s))
  ok    PR 15 (1 citation(s))
  ok    PR 16 (1 citation(s))
  ok    PR 17 (2 citation(s))
  ok    PR 18 (2 citation(s))
  ok    PR 19 (1 citation(s))
  ok    PR 20 (4 citation(s))
  ok    PR 21 (3 citation(s))
  ok    PR 22 (4 citation(s))
  ok    PR 23 (1 citation(s))
  ok    PR 24 (4 citation(s))
  ok    PR 27 (1 citation(s))
  ok    PR 28 (1 citation(s))
  ok    PR 31 (2 citation(s))
  ok    issue 11 (18 citation(s))
verify-citations: ok — 39 unique id(s) cited by 114 markdown file(s) all resolve against vaam-apps/vpay
$ echo $?
0
```

Every id this repository cites as evidence exists. That is a real result and a
modest one: it says the ids resolve, not that they say what the sentence
around them claims.

## Mutation proofs

Each applied to the head of this branch, the named gate run, then
`git checkout --` and `git status --porcelain` confirmed clean.

| # | mutation | gate | result |
|---|---|---|---|
| M1 | the review's **M3**: `../adr/0014-builder-host-musl-triple.md` → `…-triples.md` in `docs/runbooks/release.md` | `cargo xtask verify-links` | **caught**, exit **1**: `docs/runbooks/release.md:86: ../adr/0014-builder-host-musl-triples.md -> docs/adr/0014-builder-host-musl-triples.md` and the same at `:222`. On `master` this exits 0. |
| M2 | the review's **M2**: `33929374661` → `39999999999` throughout `docs/runbooks/release.md` | `cargo xtask verify-citations` | **caught**, exit **1**: `MISS run 39999999999 — HTTP 404 … cited at docs/runbooks/release.md:11, :204, :208`. The per-file exemption does not travel with the digits. |
| M3 | `docs/status.md`'s `PR #31` citation repointed at pull request 9999, which does not exist | `cargo xtask verify-citations` | **caught**: `MISS PR 9999 — HTTP 404 … cited at docs/status.md:1394` |
| M4 | delete the backtick-info clause from `fence_marker` | `cargo nextest run -p xtask`, then `verify-links` | **caught** by `a_backtick_run_whose_info_string_holds_backticks_opens_no_fence` (1 failed). The gate itself still printed `ok` — with **591** links instead of 672. That is the finding below. |

### The finding M4 records

`docs/status.md` line 69 begins ```` ```` ```ignore ```` ````: a four-backtick
*code span* whose content is a three-backtick fence, which is how this
repository writes about doctest fences. A fence scanner that ignores
CommonMark's "a backtick fence's info string may not contain a backtick" reads
it as an opening fence that never closes, and masks **2 200 of that file's
2 268 lines**. `verify-links` would have reported `ok` while checking almost
nothing in the single most link-dense document in the tree — the
delete-too-much failure this repository's own lexers warn about. It was found
by counting: 591 links against a naive scan's 672. The clause and the
regression test are both in the first commit; the doc comment on
`fence_marker` records the number.

Two other maskers are conservative for the same reason and say so: an inline
code span must be closed **on the same line** (a multi-line span is legal
CommonMark, but honouring it lets one stray backtick blank a document), and an
unterminated `<!--` is left as ordinary text
(`an_unterminated_html_comment_does_not_swallow_the_document`).

## Tests

36 new, all in `.xtask/src/main.rs`; `xtask` goes 90 → 126 and the workspace
1166 → 1202. (**The review added four more: 130 and 1206** — three guards for
properties that were prose only, and one for a link to the repository root.
See [opus-review.md](opus-review.md) §3.) No new test binary, so `verify-ignored`'s `expected_suites` stays
42 and its floor stays 1080; the justfile's history comment records the bump.

`link_tests` (23). Nine drive `verify_links` end to end over a throwaway
`git init`ed tree, because "tracked, not merely present" is the rule that
makes a green run mean anything and only a real index proves it:

- `one_broken_relative_link_fails_the_gate` — the decisive negative; asserts
  the `file:line: target -> resolved` text
- `the_same_link_passes_once_its_target_is_tracked` — so the failure is about
  the target, not a parser that refuses everything
- `a_file_that_is_present_but_untracked_does_not_satisfy_a_link`
- `a_link_inside_a_fenced_block_is_not_a_link`
- `a_backtick_run_whose_info_string_holds_backticks_opens_no_fence` — the M4
  regression
- `an_inner_fence_with_an_info_string_does_not_close_the_block`
- `a_link_inside_a_code_span_or_an_html_comment_is_not_a_link`
- `an_unterminated_html_comment_does_not_swallow_the_document`
- `a_fragment_and_a_line_suffix_resolve_to_the_file_itself`
- `a_fragment_does_not_excuse_a_missing_file`
- `a_reference_definition_is_checked`
- `an_angle_bracketed_target_with_spaces_parses`
- `an_image_target_is_checked_like_any_other`
- `a_link_to_a_directory_that_holds_a_tracked_file_resolves`
- `a_target_that_climbs_above_the_repository_root_is_reported`
- `http_mailto_and_bare_anchor_targets_are_skipped`
- `a_percent_escaped_target_is_decoded_before_it_is_resolved`
- `a_stray_close_bracket_and_paren_in_prose_is_not_a_link`
- `a_destination_may_carry_balanced_parentheses_and_a_title`
- `link_text_may_wrap_across_lines`
- `a_destination_containing_a_newline_is_not_a_link`
- `a_line_suffix_is_stripped_only_when_it_is_digits`
- `a_root_relative_target_resolves_from_the_repository_root`

`citation_tests` (13), all offline:

- `every_run_id_in_a_list_is_a_citation_not_just_the_first`
- `digits_attached_to_something_else_are_not_a_run_id`
- `a_pull_request_cue_and_an_issue_cue_choose_different_endpoints`
- `a_second_number_continues_the_first_citations_cue`
- `a_cue_does_not_carry_past_intervening_prose`
- `an_uncued_hash_number_is_not_a_citation` — every uncued `#n` this tree
  actually contains; the first test to fail if the cue rule is loosened
- `a_hash_attached_to_a_word_is_a_cross_repository_reference_or_not_a_number_at_all`
- `a_markdown_heading_and_an_anchor_link_are_not_citations`
- `the_two_word_pull_request_cue_is_recognised`
- `a_url_is_a_citation_only_when_it_names_this_repository`
- `a_citation_inside_a_code_fence_is_still_a_claim`
- `a_citation_carries_the_line_it_was_written_on`
- `the_only_exempt_ids_are_the_two_mutation_records`

The network half of `verify-citations` is exercised by running the command,
whose output is pasted above, and by mutations M2 and M3.

## Gates run

On this branch's head, in this worktree, `CARGO_BUILD_JOBS=4`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `VPAY_REQUIRE_NODE=1`.

| command | result |
|---|---|
| `just fmt-check` | ok |
| `just clippy` (`cargo clippy --workspace --all-targets -- -D warnings`) | ok — the workspace **includes `.xtask`**, so both new commands and all 36 new tests are linted |
| `just verify` | ok — five gates: `verify-no-mocks`, `verify-status` (1 unimplemented item), `verify-errors` (15 types, 14 `#[from]` variants), `verify-sdk-parity` (342 proving tests, 26 dated gaps), `verify-links` (672 links, 114 files) |
| `just docs-check` | ok — `verify-status` **and `verify-links`**; no echo |
| `just docs-check-citations` | ok — 39 unique ids, all resolve |
| `cargo nextest run -p xtask` | **126 run, 126 passed, 0 skipped** (was 90) |
| `just test-rust` (`cargo nextest run --workspace`) | **1202 run, 1202 passed, 0 skipped** |
| `just test-doc` | **86 passed, 0 failed, 1 ignored** — the ignored one is `sdks/rust`'s README block and is pre-existing |
| `just verify-ignored` | `0 ignored (expected 0), 42 test binaries (expected 42), 1202 total (minimum 1080)` |
| `just lint-web` | ok |
| `just test-web` | ok — 302 `@vpay/checkout`, 57 `examples/shop`, 3 `@vpay/ui`, and the rest |
| `just deny` | `advisories ok, bans ok, licenses ok, sources ok` |

**One environment finding, unrelated to this change and reported rather than
worked around.** The first `just test-rust` in a fresh worktree fails
`webhooks::the_delivered_signature_verifies_with_the_shipping_node_sdk` with
`` "`pnpm --filter @vpay/sdk build` failed:\nsh: 1: tsc: not found" ``: that
test shells out to the shipping Node SDK, `just test-rust` does not depend on
`install-node`, and a worktree with no `node_modules` has no `tsc`. CI's
`rust` job installs Node and builds the SDK first, so it does not see this.
Running `pnpm install --frozen-lockfile` and re-running gives 1202/1202. The
test is behaving exactly as designed — it fails rather than skipping, which is
the point — but a first-run failure that looks like a signature defect and is
a missing toolchain is worth someone's attention.

## What was NOT done

- **No CI job runs `verify-citations`.** Whether a scheduled or nightly job
  should is a maintainer decision, not a docs fix — the same call the Step 9
  review's finding F3 left open.
- **Anchors are not resolved.** A link to a heading that no longer exists
  still passes if the file exists.
- **External URLs are not fetched.** Nothing here proves
  `https://github.com/datreeio/CRDs-catalog` resolves.
- **Cross-repository citations are not resolved** (`authkestra#287`,
  `marcjazz/authkestra/issues/185`).
- **A cited id is proven to *exist*, not to *support the sentence*.** Run
  `33929374661` exists; that it "pushed four digests" is still only checkable
  by a human.
- **An uncued `#n` is not checked.** Empty set today; not guaranteed to stay
  empty.
- **Reference *usages* with no definition are not reported.**
- **Nothing in the tree exercises three of the parser's branches on real
  input**: there is not one reference definition, one angle-bracketed target
  or one `:line` suffix in any tracked `*.md` today. They are implemented and
  unit-tested, and this is the first thing to re-measure if they ever appear.
