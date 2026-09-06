# exp15 (arm C) — making `verify-sdk-parity` two-directional

Date: 2026-09-06. Branch `claude/exp15-parity-C`, base `1bd2183`
(`origin/master`). Host: the authoring machine, `CARGO_BUILD_JOBS=4`, no
Docker. Every command below was run; every number is pasted, not paraphrased.

## 0. The defect, measured before anything was written

`cargo xtask verify-sdk-parity` read `docs/sdks/parity.md` and checked whether
what the document *said* was true. It could not check what the document did
not say. Measured on the base commit, deleting the whole `refunds.create` row
(line 115):

```
$ sed -i '115d' docs/sdks/parity.md
$ cargo run -q -p xtask -- verify-sdk-parity; echo "exit=$?"
verify-sdk-parity: ok — 347 proving test(s) named in docs/sdks/parity.md all exist, 28 dated gap(s)
exit=0
```

350 proving tests became 347 and the gate stayed green. An SDK method with no
row at all was invisible for the same reason. [ADR-0015](../../adr/0015-sdk-parity.md)'s
rule is a claim about the SDKs — "every SDK ships every feature or a dated
gap" — and its decision 3 says the gate "fails the build the moment the
document and the trees disagree", which was true only of the half of the
disagreement the document mentions.

## 1. What was built

Five rules now, where there were three. The three cell rules are untouched. Two
are new, in `.xtask/src/main.rs`:

- **code → doc.** Every `<resource>.<method>` either SDK declares must have a
  row. Failure names the `file:line` the method is declared on.
- **doc → code.** Every `<resource>.<method>` row must name a method at least
  one SDK declares, unless **every** cell of that row is a dated `⛔` — the
  planned-gap escape ADR-0015 needs, and the one the `events.retrieve` row has
  used since 2026-09-03. Failure names the row's line.

The enumerator scans sources, like every other `verify-*` gate and for the
same reason: compiling two SDKs in two languages to ask what they export would
make `just verify` depend on a `cargo build` and a `tsc`, and the gate would
then be unable to run on the tree that most needs it. One lexer (`code_only`)
blanks strings and comments while preserving byte offsets and line breaks;
everything downstream reads code and only code.

### The naming convention, decided and written into `parity.md`'s header

| Source | Read as |
|---|---|
| Rust `impl <Resource>Resource { pub async fn <method>(` | `<resource_snake>.<method>` |
| Node exported `<Resource>Resource` class methods | `<resource_snake>.<method>` |

Resource names map by snake_case in both languages. One alias table entry:
`checkout_sessions` → `checkout.sessions`, because that is what
`client.checkout().sessions()` and `client.checkout.sessions` say and what
every row has said since 2026-09-04. Constructors, `#private`/`private`
members and namespace accessors (Rust `CheckoutResource::sessions`, a
`pub fn`, not a `pub async fn`; Node `CheckoutResource.sessions`, a field) are
not capabilities.

A row is a **capability row** when its first cell *opens* with a code span of
that shape. Opening with it is load-bearing: `parity.md` also carries rows
that mention a dotted code span mid-sentence — the `checkout.session.expired`
event-type rows — and reading one of those as a capability would demand an SDK
method that must not exist. That case is a unit test, not an argument.

## 2. Run on this repository's own tree: nothing to fix

```
verify-sdk-parity: ok — 350 proving test(s) named in docs/sdks/parity.md all exist,
28 dated gap(s), 13 SDK method(s) enumerated across 16 row(s)
```

Both SDKs declare exactly the same 13 capabilities:

```
account_holders.retrieve       balance.retrieve
checkout.sessions.create       checkout.sessions.expire
checkout.sessions.list         checkout.sessions.retrieve
events.list                    payment_intents.cancel
payment_intents.confirm        payment_intents.create
payment_intents.list           payment_intents.retrieve
refunds.create
```

All 13 already had rows. Sixteen capability rows name fourteen distinct
capabilities (`account_holders.retrieve` and `checkout.sessions.create` each
have two rows); the fourteenth, `events.retrieve`, is ⛔/⛔ dated 2026-09-03
and is exactly the planned-gap case. **So the two directions are recorded here
as newly *enforced*, not as newly *discovered defects*.** Nothing in
`parity.md`'s cells was changed and no gap row was added — a finding that did
not exist is not written down as if it had.

`refunds.retrieve`, which the brief named as a mutation target, does not exist
on this base: neither SDK declares it and no row mentions it. The equivalent
mutation was run against `refunds.create` instead.

## 3. Mutations — each applied, gate run, reverted

Verified reverted with `git status --porcelain` after each.

| # | Mutation | Result |
|---|---|---|
| 1 | delete the `refunds.create` row | **FAIL**, `sdks/rust/src/resources.rs:705: `refunds.create` is shipped and has no row` |
| 2 | add `pub async fn frobnicate(` to a Rust resource | **FAIL**, `sdks/rust/src/resources.rs:705: `refunds.frobnicate` …` |
| 2b | add `async frobnicate(` to the Node `RefundsResource` | **FAIL**, `sdks/nodejs/src/resources/refunds.ts:14: `refunds.frobnicate` …` |
| 3 | add a `payments.teleport` row with two ✅ cells | **FAIL**, `docs/sdks/parity.md:116: row `payments.teleport` names a method no SDK declares` |
| 4 | rename a named proving test | **FAIL** (the pre-existing direction, preserved) |
| 5 | the same `payments.teleport` row rewritten ⛔/⛔ with a date | **PASS**, exit 0, 30 dated gaps |
| 6 | rename `create` → `creat` on the **Rust** `RefundsResource` | **FAIL**, one problem: `refunds.creat` is shipped and has no row |
| 7 | rename it on **both** SDKs | **FAIL**, two problems, one per direction |

Every mutation was re-run against the committed tree, with the exit code
printed each time: 1, 1, 1, 1, 1, 0 for mutations 1–5 and 6.

Mutations 6 and 7 are the pair worth reading together, because 6 shows the
rule's shape rather than only that it fires. Renaming a method in **one** SDK
raises only the code→doc failure: the `refunds.create` row still names a
method Node declares, and "a row names a method **at least one** SDK declares"
is deliberately what ADR-0015 asks for — the ✅/⛔ cells are what say which SDK
has it, and duplicating that in the row name would make a per-SDK divergence
fail twice for one reason. Renaming it in **both** raises both failures:

```
xtask: sdk parity violations:
  - docs/sdks/parity.md:159: row `refunds.create` names a method no SDK declares — …
  - sdks/rust/src/resources.rs:705: `refunds.creat` is shipped and has no row in docs/sdks/parity.md — …
```

## 4. Unit tests

`cargo test -p xtask`: **194 → 208 passed, 0 failed, 0 ignored** (before and
after; the base number was measured on `1bd2183` before any edit). Fourteen
new cases: the Rust enumerator and its four exclusions, the Node enumerator
and its four, the `checkout.sessions` alias, the row parser (including the
`checkout.session.expired` mid-sentence case), both new directions, the
half-dated row, the test-only-resource case, and the deleted-row regression.

Two of them exist against a specific failure mode rather than for coverage:

- `the_repositorys_own_sdks_enumerate_exactly_the_capabilities_the_matrix_records`
  asserts the 13 capabilities **by name**. Both new directions are satisfied
  vacuously by an enumerator that finds nothing, and the success line's method
  count is printed for the same reason. A count alone would survive the list
  changing under it, so the list is asserted.
- `deleting_a_whole_row_fails_and_names_the_method_it_stopped_recording` is
  §0's measurement as a regression test.

The lexer was written *because* of a measured failure, not defensively: the
first implementation matched braces without reading comments, and a doc
comment carrying a lone `{` ran the matcher off the end of the fixture and
silently enumerated **nothing** from that resource — the exact shape of
failure this gate exists to prevent, discovered by a fixture that deliberately
contained one.

## 5. The two stale counts

- `CLAUDE.md` said `just verify # three self-checks`. It runs ten, and has
  been wrong since 2026-09-03. Corrected, with a dated note pointing at
  [AGENTS.md](../../../AGENTS.md), which carries the count and the history of
  every gate that moved it.
- `docs/status.md`'s `just release-dry-run` row said "Builds all three release
  images". The recipe's loop builds four, and has since `vpay-checkout` joined
  it on 2026-09-04. Corrected with a dated note. The recipe's own closing echo
  carried the same stale string and was fixed on 2026-09-05; this was the copy
  left behind.

## 6. Gates

All on this tree, this branch:

| Command | Result |
|---|---|
| `just verify` | ten gates, all ok |
| `cargo test -p xtask` | 208 passed, 0 failed, 0 ignored (was 194) |
| `just fmt-check` | ok |
| `just clippy` | ok (one `redundant_closure` on new code, fixed) |
| `just docs-check` | see below |

**Updated 2026-09-06 by the review pass:** every row above was re-run and
still holds, plus `just verify-ignored` (`0 ignored (expected 0), 42 test
binaries, 1333 total`), which this pass had not run. `cargo test -p xtask` is
now **211**, the three added cases being the two lexer regressions and the
TypeScript type-parameter case. See [C-review.md](C-review.md).

## 7. What was NOT done

- **`docs/adr/0015-sdk-parity.md` was not edited.** ADRs are immutable in this
  repository — superseded, never edited — and its "How the check reads the
  matrix" section now describes only three of the five rules. Whether that
  warrants a superseding ADR is a maintainer's decision, not one this pass
  took. Decision 3's own wording ("fails the build the moment the document and
  the trees disagree") is what the gate now finally implements.
- **No `sdks/` file was changed**, so `just lint-web` and `just test-web` were
  not run.
- **No column whose language the enumerator does not read is covered in the
  code→doc direction.** A hypothetical `sdks/kotlin` column would contribute
  no methods and the direction would be silent for it. The doc→code direction
  still covers its rows. Stated in the function's doc comment.
- **A resource type not named `…Resource`, or declared as an object literal
  rather than a class, is invisible to the code→doc direction.** Both SDKs
  name every resource `<X>Resource` and declare each as a Rust `impl` or a TS
  `class`; the alternative was a scanner that guessed which of a module's
  types is a capability, which fails quietly. Recorded in the code.
- ~~**A Rust char literal holding a lone brace would unbalance the lexer.**
  Neither SDK contains one; the limitation is the same one
  `balanced_delimited` has carried since it was written, and it is written
  down rather than left to be found.~~ **Retracted 2026-09-06 by the review
  pass ([C-review.md](C-review.md) F1): the second sentence was false when it
  was written.** `sdks/rust/src/webhooks.rs:321` has shipped
  `altered.push(if last == b'}' { b')' } else { b'}' });` since it was
  written — two unbalanced closing braces — and is harmless only because that
  file declares no `…Resource` impl. The limitation was not merely
  undesirable, it was reachable: with one such literal inside
  `PaymentIntentsResource`, adding an unrecorded `pub async fn` to the same
  impl left the gate green at `13 SDK method(s)`, where the same addition
  without it failed with exit 1. `code_only` now hands Rust literals to
  `end_of_literal` — the lexer `verify-status`, `verify-serde` and
  `verify-docs` already share, which this pass should have reused rather than
  written a weaker copy of (ADR-0016 standard 4). The same commit fixed a
  second silent defect the same reuse would have avoided: Rust block comments
  nest and this lexer did not count them, so a method parked inside
  `/* … /* … */ … */` was enumerated as shipped and the gate demanded a row
  for a method that does not exist.
- **Method-name spelling is not normalised across languages.** A Rust
  `list_all` and a Node `listAll` would be two capabilities needing two rows.
  That is the brief's "the SDKs' own spelling" and is arguably the right
  answer — a real divergence surfaced — but it is a choice, not an accident.
- **Per-column cell/method agreement is not checked.** If Rust declares a
  method and the row's `sdks/rust` cell is ⛔, that passes: the row exists and
  the method exists. Mutation 6 above is the same hole seen from the other
  side — one SDK renaming a method away leaves its row standing, because the
  other still has it. Catching "this SDK has it but its own cell says it does
  not", and its converse, is a further check this pass did not build; it is
  the obvious next one, and it would need a rule for what a ⛔ cell means when
  the method exists but is untested.

## Rebased onto `bb8de92`, 2026-09-06

Rebased onto `origin/master` at `bb8de92`, which merged PR #51: `GET
/v1/refunds/{id}`, `refunds.retrieve` in both merchant SDKs, a
`refunds.retrieve` row in `docs/sdks/parity.md`, re-measured counts in
`docs/api/README.md`, `docs/status.md` and `README.md`, and
`expected_suites` 41 → 43.

**Git reported no conflict in any of the nine commits**, and that is the
interesting part rather than a convenience. The two branches touched
overlapping files — `docs/sdks/parity.md`, `docs/status.md`, `justfile` — but
never overlapping line ranges: #51 edited parity.md's "Measured by reading
both SDKs" paragraph and added a table row, this branch inserted a new
section above that paragraph; #51 edited the `justfile`'s `verify-docs`
commentary and `expected_suites` near line 1010, this branch edited the
header comment at line 5. Both sides were kept in every case, which is what
the merge should do, and no hand-editing was needed.

**A clean rebase was nevertheless not a correct one.** The branch's own
vacuity guard failed immediately:

    sdk_parity_tests::the_repositorys_own_sdks_enumerate_exactly_the_capabilities_the_matrix_records
    assertion `left == right` failed: sdks/rust
      left:  [… "refunds.create", "refunds.retrieve"]
      right: [… "refunds.create"]

This is the guard doing exactly the job it was written for, one commit
earlier than expected. The `expected` list was written against a tree with 13
methods; #51 added a fourteenth to both SDKs. Nothing about the gate changed
and nothing about #51 was wrong — the *fact this tree asserts* changed, and
because the assertion names the capabilities rather than counting them, it
said so, named the SDK and printed both lists. Had it asserted `len() == 13`,
or merely "non-empty", the rebase would have been silently green with the
guard's meaning quietly widened. That is the case the doc comment on that
test now records, because a guard whose value is hypothetical is easy to
relax later.

**What was re-measured, in the same commit as the list:**

| | Before the rebase | After |
|---|---|---|
| `verify-sdk-parity` proving tests | 350 | **354** |
| dated gaps | 28 | **28** |
| SDK methods enumerated | 13 | **14** |
| capability rows | 16 | **17** |
| `cargo test -p xtask` | 211 passed, 0 ignored | **211 passed, 0 ignored** |
| `just verify-ignored` total | 1336 | **1351** |
| test binaries / `expected_suites` | 42 | **43** (set by #51, not by this branch) |

The success line on the rebased tree, in full:

    verify-sdk-parity: ok — 354 proving test(s) named in docs/sdks/parity.md all exist, 28 dated gap(s), 14 SDK method(s) enumerated across 17 row(s)

The gate found **nothing to fix** on the rebased tree: `refunds.retrieve` is
declared in both SDKs and #51 gave it a row, so both new directions were
already satisfied. Fourteen shipped capabilities all have rows; fifteen
distinct capabilities are named by rows, the fifteenth being the
`events.retrieve` dated ⛔ that has been ⛔/⛔ since 2026-09-03.

The three doc comments quoting `13 SDK method(s)` — the byte-literal lexer
note, the TypeScript type-parameter note, and the mutation record beside them
— were **not** rewritten to 14, because each states what the gate printed on
the tree its measurement was taken on and changing the figure would make the
record false. Each now carries the re-measured number alongside it instead.

**Three decisive mutations re-run on the rebased tree**, each applied, run and
reverted, all exit 1:

| Mutation | Result |
|---|---|
| delete the `refunds.retrieve` row | **exit 1**, `sdks/rust/src/resources.rs:726: \`refunds.retrieve\` is shipped and has no row` |
| `pub async fn frobnicate(` added to `PaymentIntentsResource` | **exit 1**, `sdks/rust/src/resources.rs:512` |
| a `b'}'` byte literal *then* the same method, same `impl` (the lexer fix) | **exit 1**, `sdks/rust/src/resources.rs:516` |

The third is the one that matters: before the `end_of_literal` reuse, the
byte literal's brace truncated the enclosing `impl` and every method after it
vanished, so this mutation passed. It does not now.

**The full gate on the rebased tree**, every command run rather than
reconstructed:

| Command | Result |
|---|---|
| `just verify` | `verify: ok — the ten gates above passed`, exit 0 |
| `cargo xtask verify-sdk-parity` | `ok — 354 proving test(s) …, 28 dated gap(s), 14 SDK method(s) enumerated across 17 row(s)`, exit 0 |
| `cargo test -p xtask` | **211 passed, 0 failed, 0 ignored** |
| `just docs-check` | exit 0 |
| `just fmt-check` | exit 0 |
| `just clippy` | exit 0 |
| `just verify-ignored` | `0 ignored (expected 0), 43 test binaries (expected 43), 1351 total (minimum 1080)`, exit 0 |

**`verify-ignored` failed once before it passed, and the cause was not this
branch.** The first run died linking the `browser_checkout` test binary with
`rust-lld: error: undefined hidden symbol: anon.<hash>.llvm.<hash>`,
referenced from `core/src/ub_checks.rs:73`. Nothing in this branch's diff
reaches that crate — it touches `.xtask` and documentation only — and
`cargo clean -p vpay-tests-integration` followed by a rerun passed with the
numbers above. Recorded rather than quietly re-run: it is a stale-incremental
linker failure on this host, it is reproducible only from a dirty `target/`,
and anyone who hits it should clean that crate rather than go looking for a
defect in the gate.

**One expectation not met, and it is a design decision rather than a defect.**
The first mutation fails naming *one* SDK's `file:line`, not both. `shipped`
is a `BTreeMap` keyed by capability and the comment above it says why —
"first declaration wins, so the reported `file:line` is stable rather than
dependent on column order" — so a capability both SDKs declare yields one
violation, deterministically the Rust one. Reporting every declaring SDK
would be more informative and would match the matrix's per-column shape, but
it is the same question as the per-column strictness already reserved for the
maintainer below ("at least one SDK" versus per-SDK), so this pass did not
pre-empt it. It is listed in the PR as reserved, not silently changed.
