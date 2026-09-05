# exp10 (opus) — sabotage review of `claude/exp10-standards-opus`

2026-09-05. Reviewed `git diff 2ce13d0..bbd5c43` (one commit) against the task
brief and against ADR-0016's own claims. Everything below was measured in this
worktree; nothing is taken from `docs/plans/exp10-notes/opus.md`, which is the
account under review rather than evidence for it.

Two findings were fixed (`e34e708`, `178ecb2`); the rest are recorded because
they are either judgement calls that belong to the maintainer or limitations
that are now written down rather than closed.

## Verdict

**Safe as delivered, with one gate hole that is now closed.** The change does
what the brief asked, the two gates fail on the mutations they are supposed to
fail on, and — the claim most worth attacking — **no wire moved**. The hole
was in `verify-repositories`, not in the shipped payment code.

## What was verified independently, and how

| claim | how it was checked | verdict |
|---|---|---|
| 64 serialisable types under `backends/crates/*/src` | `grep -rnE 'derive\(.*(Serialize\|Deserialize)' backends/crates --include='*.rs' \| grep /src/` → **67**, minus the 3 inside `#[cfg(test)]` (`error.rs:2085`, `form.rs:763`, `form.rs:774`, each confirmed below a `#[cfg(test)]` at 1160 / 495) = **64** | exact |
| 49 comply + 15 exempted, 0 violations | `cargo xtask verify-serde` | exact |
| the 15 exemption reasons are real | every row opened in the source: `ExpiresIn`, `Reason`, `Scalar`, `ExpandableIntent` all carry `#[serde(untagged)]`; `Currency` carries `rename_all = "UPPERCASE"`; the other 10 are all in `vpay-adapter-*/src/{wire,token}.rs` | **all 15 true** |
| no enum was given the attribute | all 13 additions in the diff are `struct`; `apply_to_variant` is *not* the identity, so this is load-bearing | true |
| 2 repository violations, 0 after | `verify-repositories`; the only crates depending on `vpay-db` are `vpay-api`, `vpay-worker`, both binaries and `backends/tests/integration` | true |
| `cargo test -p xtask` 144 → 181 | measured 181 directly; `#[test]` markers 146 → 183, with the same 2 markers-inside-string-literals at both ends, so the base was 144 | exact |
| `just test-rust` 1257 | 1257 total, 0 failed, 0 ignored | exact |
| verify-docs' new numbers | `vpay-testkit`'s "0 comments" cross-checked by hand: its 6 `//` lines are all below `containers.rs`'s `#[cfg(test)]` at 292 | honest |

## The wire question, settled two ways

The branch's central claim is that adding `#[serde(rename_all = "snake_case")]`
to 13 types moved nothing on any wire — which matters because three of them
(`PollChargePayload`, `ResubmitPayload`, `DeliverWebhookPayload`) are **job
rows already sitting in a live Postgres**, one (`WebhookPolicy`) is a config
YAML key, and two (`CheckoutMerchantObject`, `CheckoutSessionForPayer`) are API
responses a merchant's page reads.

1. **From serde's source.** `serde_derive-1.0.229`'s
   `internals/case.rs::RenameRule::apply_to_field` returns `field.to_owned()`
   for `SnakeCase` — the identity, *unconditionally*, for any input. This is
   stronger than the notes' argument (which reasoned from rustc's
   `non_snake_case` lint): it holds even for a field that is not snake_case.
   `apply_to_variant` is a real transformation, which is why "structs only" is
   the load-bearing part.
2. **Empirically.** Each of the 13 field lists was declared twice — with and
   without the attribute — and serialised. All 13 pairs are byte-identical.
   The same harness declares `enum { PollCharge }` both ways and gets
   `"PollCharge"` vs `"poll_charge"`, so the proof is not vacuous.

`RawClaims` specifically: it deserialises `sub` and `scope` only, both
identity. `client_id` is a field of `ResourceClaims`, which derives no serde at
all and never touches a wire — it is populated *from* `sub` in a `From` impl.
`iss`/`aud`/`exp` are validated by `jsonwebtoken`'s `Validation`, not by
`RawClaims`, and no `deny_unknown_fields` was added, so a token carrying them
still parses. Proven by `merchant_token_flow` passing end to end.

The `SqlClientAssertionStore` → `client_assertion_store` refactor changed
visibility and the constructor, and **not one line of SQL** — the
`impl ClientAssertionStore` block is untouched in the diff. Its four proving
tests all pass against a real Postgres:
`a_client_assertion_jti_is_fresh_once_then_replayed`,
`concurrent_record_jti_calls_for_the_same_jti_yield_exactly_one_fresh_result`,
`expired_client_assertion_jtis_are_swept_and_live_ones_are_kept`, and
`the_same_client_assertion_cannot_be_spent_twice` (which is the one that proves
the store is still *wired into* the token handler).

## Mutations

Every row applied to the real tree, run, and reverted. Exit codes checked
directly, not through a pipe — a gate that prints a violation and exits 0 is
not a gate.

| # | mutation | expected | result |
|---|---|---|---|
| M-A | delete `rename_all` from `DeliverWebhookPayload` | fail | `just verify` **exit 1**, `jobs.rs:241`, stops at `verify-serde` |
| M-B | `#[derive(serde::Serialize)] pub struct { a: u8 }`, non-test module | fail | exit 1, names file:line |
| M-C | the same inside `#[cfg(test)] mod` | pass | exit 0, count unchanged at 49/15 |
| M-D | multi-line `#[derive(\n Debug,\n serde::Serialize,\n)]` | fail | exit 1 |
| M-E | `use vpay_db::PgRepositories;` in `vpay-api` | fail | exit 1, `op/mod.rs:27` |
| M-F | `vpay_db::PgRepositories` by full path, no `use` | fail | exit 1, `op/mod.rs:123` |
| M-G | exemption row for `PollChargePayload`, which complies | fail | exit 1, names the **ADR** line |
| M-H | delete `Currency`'s row, still needed | fail | exit 1, says "variant" not "field" |
| M-I | exemption row with a blank reason | fail | exit 1 |
| M-J | reach `PendingTransaction` from `vpay-worker` | fail | exit 1 |
| M-K | one field renamed, one not | fail | exit 1 |
| M-L | doc comment quoting the attribute above an unattributed struct | fail | exit 1 — prose satisfies nothing |
| **H1** | `pub use repository::PgRepositories as Repos;` in `vpay-db` + `use vpay_db::Repos;` in `vpay-api` | fail | **exit 0 — evasion** → fixed in `e34e708`, now exit 1 |
| **H2** | `pub type RepoAlias = …PgRepositories;` + `use vpay_db::RepoAlias;` | fail | **exit 0 — evasion** → fixed in `e34e708`, now exit 1 |
| H3 | `pub struct SqlLeakyStore { pool: PgPool }` in `vpay-db`, unconsumed | — | exit 0; see "left to the maintainer" |
| H4 | empty-body `struct X {}` with a serde derive | — | exit 0, vacuously and correctly (it serialises no names) |

Not caught, and expected not to be: deleting an xtask unit test, and removing
either gate from `just verify`. Nothing in the repository checks that
`just verify`'s recipe still lists what its own header says it lists — a
pre-existing drift gap, and the same one AGENTS.md has had to correct by hand
three times (three → five → seven → nine). Recorded, not fixed: closing it is a
gate about the justfile, which is a different change.

## Findings

### 1. gate-hole — `verify-repositories` missed a name `vpay-db` publishes (FIXED, `e34e708`)

Both spellings above cleared the gate. It matches names textually, and derived
its set from two signals that each find a *declaration*; neither finds
`vpay-db` re-exporting an implementation under a different word. "Make it
`pub(crate)` and re-export it under a friendlier name" is a plausible thing to
do while believing you are complying — which is exactly the failure mode this
gate exists for, since the compiler has no opinion either.

Fixed by a third signal: a `pub use … as` / `pub type` alias that `vpay-db`
declares for a type already in the set joins the set, to a fixpoint. Private
aliases are not read. The tree's own set is **unchanged at 3**, because
`vpay-db` publishes no such alias — its one `pub type`, `TxFuture`, has a
`Pin<Box<dyn Future>>` on the right. Three guard tests; both original evasions
re-run and now exit 1.

### 2. misleading-claim — the notes' `just verify` transcript was paraphrased (FIXED, `178ecb2`)

`docs/plans/exp10-notes/opus.md` presented a fenced `$ just verify` block as
that command's output. `check-schema: schema OK` is a string this repository
never prints (the recipe prints two lines, both naming the pinned CrateStack
version); `verify-npm-scope` and `verify-errors` were abbreviated;
`verify-links` read 697 against an actual 698. **Every gate does pass** — this
is not a false green — but a hand-edited transcript presented as output is
precisely CLAUDE.md's "failure mode to avoid", and it is the one paragraph in
that file that makes its genuinely careful measurements worth less. Replaced
with the real output.

### 3. nit — three disclosures were narrower than the truth (FIXED, `178ecb2`)

`verify-serde` misreads `#[serde(flatten)]` and the two-sided
`#[serde(rename(serialize = …, deserialize = …))]` exactly as it misreads
`#[serde(skip)]`, which was the only one the notes named. All three fail in the
safe direction. Six types carry a flattened field, all six take the blanket
attribute. Separately: the gate sees `derive`d impls only — a hand-written
`impl Serialize` is invisible, which is true to ADR-0016's wording but was
unwritten; the four in the workspace are `object_tag!` unit structs with no
field names to rename. And `verify-repositories`' consumer set excludes
`examples/`, which the list did not say (no live hole: nothing under
`examples/` or `sdks/` depends on `vpay-db`).

## Left to the maintainer — not decided here

**Standard 5 is enforced on the consumer side only.** A new
`pub struct SqlSomethingStore { pool: PgPool }` in `vpay-db` that nobody has
named yet passes the gate (H3): it is added to the reported set, but nothing
requires its *declaration* to be `pub(crate)`. That is half of the standard's
own sentence ("implementations are private to their crate **and** reached only
through the trait") going unchecked, and it is the precursor state of exactly
the defect this branch fixed — `SqlClientAssertionStore` was caught only
because `vpay-api` had already named it.

It was not fixed here because the obvious check would fail on
`PendingTransaction`, which is `pub`, re-exported from `vpay_db`, and appears
in a public trait signature (`TransactionSource::begin_transaction`) — so
tightening this means first deciding whether `PendingTransaction` is a
repository implementation that should be hidden or a transaction handle that
must stay public. That is a design decision about `vpay-db`'s surface, not a
review call, and picking a defensible default would bury it.

**The `pub`-only scope question.** The maintainer's rule 3 says "every *public*
type"; the gate scans every serialisable declaration regardless of visibility.
The implementer recorded this as a deliberate widening with a measurement (only
8 of the 28 violations were `pub`; both adapters' wire modules are `pub(crate)`
in their entirety) and ADR-0016 argues it under "Alternatives considered". Left
exactly as delivered — the reasoning is sound and the maintainer's own phrase
was "keep and **generalise**" — but it is a widening of the stated rule and is
flagged as such rather than treated as obviously correct.

**`PendingTransaction` in the forbidden set.** It is legitimately caught by the
trait signal (`impl TxRepositories for PendingTransaction`), so the gate now
forbids any consumer from naming a type `vpay_db` publicly exports and whose
own doc comment describes callers outside the crate obtaining one. Nothing
names it today, so the gate passes; a consumer that wanted to bind
`begin_transaction`'s result to a named type would be blocked. Arguably correct
pressure toward `UnitOfWork::transaction`. Flagged, not changed — changing it
would weaken the gate.

## Not checked

- Whether each exemption's *reason* is true of the rail's live documentation.
  The rows were verified to describe the code accurately (the type really is in
  an adapter's wire module, really is `untagged`); whether MTN's Collections
  API is in fact camelCase today was not checked against MTN's own docs.
- `just test-web`, Cypress, and the e2e job. Out of scope: nothing in this
  change touches the frontends, and `verify-serde` does not scan `sdks/`.
- The comment/code ratio numbers per crate were spot-checked on one crate
  (`vpay-testkit`), not recomputed for all twelve.
