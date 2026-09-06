# exp15 (arm C) — sabotage review of `claude/exp15-parity-C`

Date: 2026-09-06. Base `1bd2183`, reviewed head `c086f56` (four commits).
Every command below was run in this worktree; every number is pasted.

## 0. The gate, as delivered

| Command | Result |
|---|---|
| `just verify` | `verify: ok — the ten gates above passed` |
| `cargo xtask verify-sdk-parity` | `ok — 350 proving test(s) …, 28 dated gap(s), 13 SDK method(s) enumerated across 16 row(s)`, exit 0 |
| `cargo test -p xtask` | **208 passed, 0 failed, 0 ignored** (master's 194 + the 14 new cases) |
| `just docs-check` | `verify-status: ok — 1 unimplemented item(s)`; `verify-links: ok — 743 link(s) in 134 file(s)` |
| `just fmt-check` | exit 0 |
| `just clippy` | exit 0 |
| `just verify-ignored` | `0 ignored (expected 0), 42 test binaries (expected 42), 1333 total (minimum 1080)` |
| `just lint-web` / `just test-web` | **not run — no `sdks/` file is changed by the diff**, which this review confirmed with `git diff --stat 1bd2183..HEAD` |

## 1. The claims, checked rather than taken

Independently enumerated, not read off the notes:

- **13 methods per SDK, and the two sets are identical.** `awk` over
  `sdks/rust/src/resources.rs`'s `impl …Resource` blocks gives 5
  `payment_intents` + 4 `checkout.sessions` + `refunds.create` +
  `events.list` + `account_holders.retrieve` + `balance.retrieve` = 13; the
  same 13 over `sdks/nodejs/src/resources/*.ts`. No `pub async fn` lives
  outside `resources.rs` in the Rust SDK.
- **16 capability rows, 14 distinct capabilities.** `docs/sdks/parity.md`
  lines 154-159, 162-165, 169-174. `account_holders.retrieve` and
  `checkout.sessions.create` each hold two rows; `events.retrieve` (line 163)
  is the ⛔/⛔ 2026-09-03 planned gap. Every one of the 13 shipped methods has
  a row. The notes' "nothing to fix on this tree" is true.
- **The two stale counts.** `justfile:488` lists eleven recipes and the
  recipe's own echo says "the ten gates above passed; the verify-docs report
  is advisory" — so `CLAUDE.md`'s new "ten self-checks … plus the
  `verify-docs` report" is right. `release-dry-run`'s loop iterates four
  specs (`vpay-server`, `vpay-worker`, `vpay-dashboard`, `vpay-checkout`) —
  so `docs/status.md`'s new "**four**" is right.
- **`accountHolders` ↔ `account_holders`.** No mapping is exercised: both
  SDKs name the type `AccountHoldersResource`, and `snake_case` takes it to
  `account_holders` in both. The camelCase spelling only ever appears as the
  Node *client accessor* (`client.accountHolders`), which the enumerator
  never reads. Matched, not stale — but for a duller reason than the brief
  supposed.
- **Node object literals / arrow properties.** The implementer calls these
  invisible. Confirmed invisible (mutations N4, N5 below) **and** confirmed
  absent from `sdks/nodejs` today: all six resource modules declare
  `export class …Resource`, no getters, no arrow-function members.

## 2. Mutations — as delivered

Each applied to the committed tree, gate run, reverted, `git status
--porcelain` clean after each. The gate binary was run directly so the
mutation could not be confused with a rebuild.

| # | Mutation | As delivered |
|---|---|---|
| M1 | delete the `refunds.create` row | **FAIL** exit 1, `sdks/rust/src/resources.rs:705: \`refunds.create\` is shipped and has no row` |
| M3 | add a `payments.teleport` row, two ✅ cells | **FAIL** exit 1, `docs/sdks/parity.md:160: row \`payments.teleport\` names a method no SDK declares` |
| M4 | rename a named proving test | **FAIL** exit 1 (the pre-existing direction, preserved) |
| M5 | the same row rewritten ⛔/⛔ with a date | **PASS** exit 0, 30 dated gaps |
| M5b | the same row half-dated (one ⛔, one ✅) | **FAIL** exit 1 — correct: one ✅ is a claim |
| R2 | `pub async fn frobnicate(` on the Rust `RefundsResource` | **FAIL** exit 1, `resources.rs:706` |
| R6 | rename `create`→`creat` on the **Rust** SDK only | **FAIL** exit 1, one problem (`refunds.creat` unrecorded); the row still stands because Node declares `create` |
| N1 | `async frobnicate(` on the Node `RefundsResource` | **FAIL** exit 1, `sdks/nodejs/src/resources/refunds.ts:15` |
| N2 | rename `create`→`creat` on the **Node** SDK only | **FAIL** exit 1, mirror of R6 |
| VAC | break the enumerator wholesale (`RESOURCE_TYPE_SUFFIX` → a suffix nothing carries) | **FAIL** exit 1, **14 of the 16 rows** each named — a wholly-vacuous enumeration is caught structurally by the doc→code direction, not only by the unit test |
| N3 | a TS method with a type parameter, `async listAll<T>()` | **PASS** exit 0, still `13 SDK method(s)` — **silent miss, finding 2** |
| N4 | a TS arrow-property member, `readonly teleport = async () => {}` | **PASS** exit 0 — invisible, and disclosed |
| N5 | a resource declared as an object literal | **PASS** exit 0 — invisible, and disclosed |
| LEX-2 | `let _sentinel = b'}';` in `PaymentIntentsResource::create`, then `pub async fn teleport(` later in the **same** impl | **PASS** exit 0, still `13 SDK method(s)` — **silent miss, finding 1** |
| LEX-2 control | the same `teleport` with no char literal | **FAIL** exit 1, `resources.rs:583: \`payment_intents.teleport\` is shipped and has no row` |

VAC is the good news and it matters: an enumerator that finds *nothing*
cannot pass, because every capability row then loses its backing. The
dangerous shape is the *partial* one — one `impl` going quiet while the
others still enumerate — because the rows keep their backing from the other
SDK and nothing is said. That is findings 1 and 2.

## 3. Findings

### F1 — gate-hole (silent) + misleading-claim: `code_only` mis-lexes Rust character and byte literals

`code_only` (`.xtask/src/main.rs`) treats `'` as never opening a literal, so
`b'}'` and `'{'` leave their brace **in the code stream**. One of them
unbalances `code_block_span`, the enclosing `impl …Resource` body is
truncated, and every method after it stops being enumerated — with no
message and no change to the printed method count, because the *other* SDK
still backs the rows.

Proved by LEX-2 above: identical mutation, exit 1 without the char literal
and exit 0 with it.

This is not hypothetical. `sdks/rust/src/webhooks.rs:321` already ships

    altered.push(if last == b'}' { b')' } else { b'}' });

which is net **two** unbalanced closing braces. It is harmless only because
`webhooks.rs` happens to declare no `…Resource` impl. The delivered notes
(§7) and `code_only`'s own doc comment both assert *"Neither SDK contains
one"* — that is **false**, and it is the sentence that made the limitation
look acceptable.

Aggravating, and the reason this is a rule-break as well: this repository
already carries a correct, tested Rust literal lexer for exactly this
problem — `end_of_literal` / `end_of_quoted` / `end_of_raw` /
`end_of_char_literal` (`.xtask/src/main.rs:1064-1155`), which handle `"`,
`r#"…"#`, `b"…"`, `c"…"`, `b'…'` and the lifetime-vs-char-literal ambiguity,
and which `verify-status`, `verify-serde` and `verify-docs` all use. The
`justfile`'s own comment above `verify-status` advertises it ("a token …
inside a string, raw-string or character literal is prose"). Writing a
second, weaker Rust lexer beside it is ADR-0016 standard 4 (DRY) and the
weaker one is wrong where the older one is right.

**Fixed** — see §4.

### F2 — gate-hole (silent): a TypeScript method with a type parameter is not enumerated

`ts_method_name` requires the member's name to be followed immediately by
`(`, so `async listAll<T>(params: T)` is read as a non-method and dropped.
Mutation N3: exit 0, count unchanged at 13. Not disclosed anywhere — the
notes' §7 list covers object literals, arrow properties and non-`…Resource`
type names, and stops there.

**Fixed** — see §4.

### F3 — misleading-claim: "Neither SDK contains one"

Folded into F1. The claim appears twice (notes §7, and `code_only`'s doc
comment) and both copies are retracted in §4's commits.

### F4 — nit: the vacuity guard also asserts something ADR-0015 does not require

`the_repositorys_own_sdks_enumerate_exactly_the_capabilities_the_matrix_records`
asserts that **each** column yields exactly the same 13 names. ADR-0015
decision 2 explicitly allows a capability to land in one SDK with a dated
gap row for the other; the day that happens, this test fails with a message
about capability names while the actual cause is a legitimate divergence.
The instinct — assert the list, not the count — is right and is kept. The
failure message now says which of the two things broke and what to do.

**Fixed** — see §4.

### F5 — recorded, not fixed: "at least one SDK" is per-row, not per-column

Deleting `sdks/rust/src/resources.rs` outright leaves the gate green: Node
still declares all 13, so every row keeps its backing, and the ✅ cells name
tests that live under `sdks/rust/tests/` and still exist. The implementer
discloses this in §7 ("per-column cell/method agreement is not checked") and
it is what the brief specified for direction (b). It is the obvious next
check and it is **not** in scope here; left as the delivered notes leave it.

### F6 — recorded, not fixed: ADR-0015 now describes three of five rules

`docs/adr/0015-sdk-parity.md`'s "How the check reads the matrix" section
lists the three cell rules only. The implementer deliberately did not edit
it (ADRs here are superseded, not edited) and reserved the question for the
maintainer. This review agrees that is the maintainer's call and does not
take it either.

## 4. Fixes

One commit per finding; each names the mutation it re-runs.

| Commit | Finding | The mutation it re-runs, before → after |
|---|---|---|
| `fix(xtask): verify-sdk-parity read Rust with its own weaker lexer, and lost methods` | F1, F3 and the nested-comment defect the same reuse fixes | LEX-2 (`b'}'` + an unrecorded `pub async fn` in the same impl) **exit 0 → exit 1**, naming `sdks/rust/src/resources.rs:584`; the nested-comment probe (a parked method inside `/* … /* … */ … */`) **exit 1, a false positive → exit 0** |
| `fix(xtask): a TypeScript method with a type parameter was read as a field` | F2 | N3 (`async listAll<T>()` on the Node `RefundsResource`) **exit 0 → exit 1**, naming `sdks/nodejs/src/resources/refunds.ts:15` |
| `test(xtask): the parity vacuity guard asserts two things; say which one broke` | F4 | no behaviour change; the guard's two unlike causes now carry two different sentences |

F1 and the nested-comment defect share one commit rather than two, and this
is a deliberate departure from one-commit-per-finding: they are the same
defect — `code_only` was a second, weaker Rust lexer — and the remedy is one
change, deferring to `end_of_literal` and giving the scan a language. Split
the other way round, neither half compiles: the nesting rule needs the
language distinction (TypeScript's block comments do not nest) that the
literal fix introduces.

### The new tests were mutation-driven, not written to the implementation

Each was measured failing against the behaviour it replaced before being
kept:

- `a_character_literal_holding_a_brace_does_not_truncate_the_impl` — with
  `end_of_char_literal` stubbed back to `None` (the exact old bug), it fails
  `["widgets.create"]`: the method *after* the literals is gone, which is the
  silent truncation itself.
- `a_method_with_a_type_parameter_is_still_a_method_and_a_field_is_not` —
  with the type-parameter skip deleted, it fails `left: None, right:
  Some("listAll")`.
- `a_nested_block_comment_hides_the_method_it_parked` — the probe it
  generalises exited 1 on this repository's own `resources.rs` before the fix.

## 5. Gate, after

| Command | Result |
|---|---|
| `just verify` | `verify: ok — the ten gates above passed` |
| `cargo xtask verify-sdk-parity` | `ok — 350 proving test(s) …, 28 dated gap(s), 13 SDK method(s) enumerated across 16 row(s)`, exit 0 — unchanged, because neither fix changes what this tree ships |
| `cargo test -p xtask` | **211 passed, 0 failed, 0 ignored** (208 as delivered, 194 on master) |
| `just docs-check` | ok |
| `just fmt-check` | exit 0 |
| `just clippy` | exit 0 |
| `just verify-ignored` | `0 ignored (expected 0), 42 test binaries (expected 42), 1333 total (minimum 1080)` |
| `just lint-web` / `just test-web` | **not run**: no file under `sdks/` is touched by this branch, before or after the review |

## 6. What this review did NOT do

- **Did not push and did not open a PR.**
- **Did not take the ADR-0015 decision** (F6). The section describing three
  of five rules is left exactly as delivered, for the maintainer.
- **Did not build the per-column check** (F5). "A row names a method at least
  one SDK declares" is what the brief asked for; "this SDK's cell says ⛔ and
  this SDK declares the method" is a different rule needing a decision about
  what ⛔ means when the method exists but is untested.
- **Did not touch `sdks/`**, so the web gates were not run and neither SDK's
  behaviour is claimed to have changed.
- **Did not close the disclosed enumerator holes** that mutations N4 and N5
  confirm: a Node resource declared as an object literal, or whose methods
  are arrow-function properties, is still invisible. Neither shape exists in
  `sdks/nodejs` today — all six resource modules are `export class
  …Resource` with ordinary members, no getters and no arrow members, checked
  file by file — and closing them means guessing which of a module's objects
  is a resource, which is the judgement `RESOURCE_TYPE_SUFFIX` exists to
  avoid. Recorded, not fixed.
- **Did not verify master's 194** by building the base commit. It is inferred
  from 208 on the delivered tree minus the 14 test functions the diff adds,
  which agrees; it is not an independently measured number.
