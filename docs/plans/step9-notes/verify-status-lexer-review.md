# exp3 review — `verify-status` counts code, not prose (`opus`)

Reviewer's record for `claude/exp3-verify-status-opus` (two commits,
`4271fb6` + `2b49654`) against `origin/master` (`c531407`). Everything below
was measured in this worktree on 2026-09-05 with `CARGO_BUILD_JOBS=4`; no
Docker was needed.

## 1. The implementer's claims, checked

| claim (`docs/plans/step9-notes/verify-status-lexer.md`) | verdict | how it was measured |
|---|---|---|
| The brief's premise is half wrong: the old `searchable` already stripped **leading** `//`, `///`, `//!` and `/* */`; only a **trailing** `//` and a **raw string** got through | **true, and stronger than stated** | probe below, run on the `origin/master` binary |
| Guard proof A — a real token in code, undeclared, still fails | true | reproduced |
| Guard proof B — the same token in prose only passes | true | reproduced |
| Guard proof D — `mtn_momo::refund` is still the one found item | true | `verify-status: ok — 1 unimplemented item(s)` |
| `verify-docs` and `verify-errors` output byte-identical old vs new | **true, and so are the other three** | all five gates diffed old-vs-new binary on this tree: identical |
| 89 xtask tests, 0 skipped | true | `89 tests run: 89 passed, 0 skipped` |
| `cargo fmt --all --check`, `cargo clippy -p xtask --all-targets -- -D warnings` clean | true | reproduced |
| Two-directional semantics "untouched… `the_status_check_fails_in_both_directions` still passes" | **true but not evidence** — see F3 | mutation M8 |
| Lifetimes are deliberately not literals | true and correct | mutation M3, plus ten adversarial probes |

### The premise correction, re-measured

A prose-only probe was inserted into
`backends/crates/vpay-adapter-mtn-momo/src/lib.rs` (after the `impl` block,
so outside `#[cfg(test)]`) carrying the token in seven non-code places, then
`.xtask/src/main.rs` was reverted to `origin/master`, rebuilt, and run:

```
$ cargo run -q -p xtask -- verify-status          # OLD scanner
xtask: these unimplemented items are missing from docs/status.md under
  `### Unimplemented items tracked by `verify-status``:
  - probe::doc_attr
  - probe::raw
  - probe::trailing
EXIT=1
```

`probe::in_doc` (a `///` line), `probe::leading` (a leading `//`) and
`probe::block` (a `/* */`) are **absent** — the implementer's correction is
right, and the `///` half of the brief's premise was already fixed on master.
`probe::doc_attr` is a third name for the same raw-string shape
(`#[doc = r#"…"#]`), which the notes call out in prose but do not measure.

The same tree on the branch's scanner:

```
$ cargo run -q -p xtask -- verify-status          # NEW scanner
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
EXIT=0
```

and with one real code occurrence added alongside the seven prose ones:

```
xtask: these unimplemented items are missing from docs/status.md under
  `### Unimplemented items tracked by `verify-status``:
  - probe::real_code
EXIT=1
```

Exactly one, the code one. The tree was restored (`git status --short`
empty) before every gate below.

The docs→code direction was proved live too: a `- ` + backtick-quoted
`phantom::gap` bullet added under the heading in `docs/status.md` makes
`verify-status` exit 1 with "docs/status.md declares these unimplemented
items and no shipping code carries them". Restored.

## 2. Mutation table

Each mutation was applied to `.xtask/src/main.rs`, `cargo nextest run -p
xtask` was run, and the file restored (`git status --short` empty after
each).

| # | mutation | caught by |
|---|---|---|
| M1 | line comments kept verbatim unless they *begin* a line (trailing `//` counts again) | `a_binary_that_opens_its_pool_lazily_is_a_violation` |
| M2 | `end_of_literal` returns `None` for raw strings | `the_token_is_read_from_the_calls_own_literal` |
| M3 | `'static` read as a char literal, swallowing to the next `'` | `a_lifetime_is_not_a_character_literal` |
| M4 | block comments no longer nest (`depth += 1` dropped) | **NOT CAUGHT** — 89/89 green → F1 |
| M5 | a line comment ends at `"` or `'` | 89/89 green — the mutation was inert on the tests' inputs |
| M5b | a line comment ends at `"` | **NOT CAUGHT** — 89/89 green → F2 |
| M6 | `end_of_literal` returns `None` for plain strings (`//` in a URL opens a comment) | `a_block_comment_neither_declares_nor_classifies` |
| M7 | `BlocksOnly` strips `//` lines too (`verify-docs` sees no docs) | 4 × `doc_report_tests::*` |
| M8 | the docs→code half of `verify_status` deleted | **NOT CAUGHT** — 89/89 green → F3 |
| M9 | the code→docs half of `verify_status` deleted | **NOT CAUGHT** — 89/89 green → F3 |

Four of ten mutations survive, and M5 is a fifth that only fails to survive
by accident. All of them are behaviours the branch — or the task brief —
claims in prose.

**After the fixes in §5, all ten are caught:**

```
M1   43/90 tests run: 42 passed, 1 failed
M2   90 tests run: 89 passed, 1 failed
M3   60/90 tests run: 59 passed, 1 failed
M4   90 tests run: 89 passed, 1 failed
M5   90 tests run: 89 passed, 1 failed
M5b  90 tests run: 89 passed, 1 failed
M6   47/90 tests run: 46 passed, 1 failed
M7   24/90 tests run: 20 passed, 4 failed
M8   90 tests run: 89 passed, 1 failed
M9   90 tests run: 89 passed, 1 failed
```

`git status --short` was empty after every restore.

## 3. Adversarial probes of the lexer itself (read-only, all passed)

A throwaway test exercised ten shapes the tests do not cover: a nested block
comment; a `//` comment containing a balanced `"…"`; a block comment
containing a lone `"`; a **non-raw** `#[doc = "…"]`; `r##"a "# b …"##`; a
string containing `/*`; an apostrophe inside a string followed by a trailing
comment; a lifetime followed by `'\''`; `b"x\"y"`; and a block comment whose
`*/` sits inside what looks like an unterminated string (rustc's rule: the
comment ends there). **All ten behave correctly.** The lexer is sound; what
is missing is the tests that would keep it sound.

## 4. Findings

| # | severity | where | evidence | status |
|---|---|---|---|---|
| F1 | correctness (untested claimed behaviour) | `.xtask/src/main.rs` `strip_comment_kinds`, doc at `:894`; `docs/status.md:13` | M4 survives all 89 tests. Both the function's doc comment ("nested or not") and `docs/status.md` ("`/* */` nested or not") claim nesting; nothing proves it. The task brief names "block comments (nested or not)". | **fixed** |
| F2 | correctness (untested claimed behaviour, brief not met) | `.xtask/src/main.rs` `the_lexer_tells_the_four_states_apart` case 1 | The brief lists five edge cases the lexer "must be tested on"; the first is *a comment containing `"`*. Case 1 substitutes an apostrophe (`// it isn't a string`) and says so in its own message. M5b — a lexer that ends a line comment at `"` — passes all 89 tests. | **fixed** |
| F3 | correctness / rule-break | `.xtask/src/main.rs` `verify_status` `unbuilt` branch; `the_status_check_fails_in_both_directions` | Deleting **either** half of `verify_status` leaves 89/89 green (M8, M9). The existing test re-implements the comparison in its own body (`declared.iter().any(|t| !found.contains(t))`) while its doc comment claims the opposite ("a test that reimplemented the comparison would pass whatever the check does"). Pre-existing on master — but the brief required the two-directional semantics be kept, and the notes cite this test as the evidence that they were. | **fixed** |
| F4 | nit | `docs/status.md:21` | The edited sentence leaves a 94-column line in a file wrapped to ~76. | **fixed** |
| F5 | disclosed, left | `.xtask/src/main.rs` `end_of_cfg_test_item` | Its private literal scanner is still not raw-string aware. Pre-existing, disclosed in the implementer's §7, and *less* reachable than before (comments are now stripped first). Out of scope; left visible. | left |

Nothing else fired. Checked and clean: no new `#[allow]`/`#[expect]`
(`verify-docs` reports the same four as master), no `unwrap`/`expect` outside
`#[cfg(test)]` in the new code, no hard-coded success, no test that asserts
nothing, `STUB_ADAPTER_NAMES` still matched against the raw file (documented,
deliberate), the `connect_lazy` guard's loosening is exactly the one its own
doc comment always claimed and is pinned by M1's test, and every gate's
output on the real tree is byte-identical to master's.

## 5. Fixes

One commit per finding, each with the mutation it now catches.

* **F1** — a sixth case in `the_lexer_tells_the_four_states_apart`: a token in
  the *tail* of an outer block comment, after an inner one has closed. Under
  M4 the tail becomes code and the scan returns two tokens.
* **F2** — case 1 rewritten to carry an **odd** number of `"` in the comment,
  which is the shape that matters: a lexer that ended the comment at the
  quote would read from there to the next `"` in the file as a literal and
  swallow the call underneath. The apostrophe is kept in the same comment.
* **F3** — `verify_status_reports_both_directions_from_the_gate_itself`:
  builds a two-file tree in a `TempDir` and calls `verify_status` directly,
  asserting both halves of the printed message, then that a corrected
  `docs/status.md` passes. The misleading doc comment on
  `the_status_check_fails_in_both_directions` is corrected in the same commit.
* **F4** — the sentence reflowed.

## 6. Final gates — the task brief's list, run here after the fixes

| command | result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy -p xtask --all-targets -- -D warnings` | clean |
| `cargo nextest run -p xtask` | **90 tests run: 90 passed, 0 skipped** (was 89) |
| `just verify` | ok — `verify-no-mocks` ok; `verify-status` **1** item; `verify-errors` **15** types / **14** `#[from]` variants; `verify-sdk-parity` **342** tests / **26** gaps; `verify-docs` report |
| `just verify-ignored` | **0 ignored (expected 0), 42 test binaries (expected 42), 1154 total (minimum 1080)** |
| `cargo xtask verify-no-mocks` | ok — no test double reachable from a shipping binary |

All five gate outputs on the real tree remain **byte-identical to
`origin/master`'s**, `verify-docs` included — re-diffed after the fixes.

Four fix commits, one per finding:

```
096f774 test(xtask): prove block comments nest
b4c52ea test(xtask): a comment containing a `"` is the edge case, not an apostrophe
e00c18d test(xtask): drive verify-status's two directions through the gate itself
2368489 docs(status): reflow the sentence the last change left at 94 columns
```

## 6b. Verdict

**Would it have been safe to merge as delivered? Yes — narrowly, and not
for the reason the notes give.**

The lexer itself is right. Ten hand-picked adversarial shapes beyond its own
tests all behave correctly, every gate's output on the real tree is unchanged
from master, the two guard-failure proofs reproduce, and the premise
correction in §1 of the implementer's notes is not only true but understated.
Nothing here is a hard-coded success, a weakened test or a false ✅ — the
failure mode `CLAUDE.md` names is absent.

What was not safe was the *evidence*. Three of the behaviours the branch
newly claims in `docs/status.md` and in the justfile — that block comments
nest, that a comment carrying a `"` is still a comment, and that the check
runs in both directions — had no test that would fail if they broke. The
first two are named verbatim in the task brief's list of what the lexer "must
be tested on"; the third the notes cite a test for that cannot see it. A gate
whose own guarantees are unguarded is the thing this repo is most careful
about, so this is a real finding rather than a stylistic one — but it is a
finding about test coverage, not about behaviour, and no wrong answer would
have shipped.

## 7. What this review did not check

* `just ci` in full — `test-web`, `lint-web`, `deny` and `test-rust` were not
  run. Nothing outside `.xtask/`, `justfile` and `docs/` changed, and the
  task brief's gate list does not include them.
* The three *other* lexers in the file (`strip_code_noise`,
  `end_of_cfg_test_item`, and `match_cfg_test`'s balanced-paren reader) were
  read but not mutation-tested; only `end_of_cfg_test_item` is asserted here
  to have a gap, and that is the implementer's own disclosure.
* No property/fuzz testing of the lexer against `rustc`'s own tokenizer. The
  ten adversarial probes in §3 are hand-picked, not exhaustive.
* Nothing was run against a database or a container.
