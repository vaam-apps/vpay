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
- **A Rust char literal holding a lone brace would unbalance the lexer.**
  Neither SDK contains one; the limitation is the same one
  `balanced_delimited` has carried since it was written, and it is written
  down rather than left to be found.
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
