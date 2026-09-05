# exp10 (opus): the six engineering standards, and gates for the two mechanical ones

2026-09-05, on `claude/exp10-standards-opus` (base `master` `2ce13d0`).
Everything below was run in this worktree with `CARGO_BUILD_JOBS=4` and the
pinned toolchain (`rust-toolchain.toml`, 1.95.0). The `cratestack` CLI was on
`PATH` for `check-schema`; Docker was used only for `just test-rust`.

## What changed

| file | change |
|---|---|
| `docs/adr/0016-engineering-standards.md` | new. The six standards, each with rationale, what enforces it, what is left to review; the serde exemption table the gate reads; the migration rule |
| `AGENTS.md` | new "Standards" section — the six rules an agent applies on every change, pointing at the ADR; the gate count corrected seven → nine |
| `.xtask/src/main.rs` | `verify-serde` and `verify-repositories`; a shared declaration scanner (`declarations`, `declaration_shape`, `attribute_block_before`, `blank_cfg_test_items`); `verify-docs` gains two report lines; module docs, `verify-all`, `help` |
| `justfile` | two recipes, both in `verify` after `check-schema` and before `verify-docs`; the header's invariant list and the `verify` preamble renumber seven → nine |
| `.github/workflows/ci.yml` | `self-checks` gains `just verify-serde` and `just verify-repositories`, through the justfile rather than a copy of the command |
| 6 files under `backends/crates/*/src` | 13 `#[serde(rename_all = "snake_case")]` attributes added |
| `backends/crates/vpay-db/src/client_assertion.rs`, `lib.rs` | `SqlClientAssertionStore` is `pub(crate)`; `vpay_db::client_assertion_store(pool) -> impl ClientAssertionStore` replaces it as the public surface |
| `backends/crates/vpay-api/src/op/mod.rs`, `op/token.rs` | the call site and two doc links |
| `backends/crates/vpay-db/tests/repositories.rs`, `backends/tests/integration/tests/{client_store,merchant_token_flow}.rs` | the same rename in the two suites that constructed the store |
| `docs/status.md` | the two gates, the two report lines, the measured counts |
| `docs/reference/vpay-db.md`, `docs/roadmap.md` | the two live documents that named `vpay_db::SqlClientAssertionStore` as a path a caller can write |

`docs/flows/*.md` Status sections: **none changed, and this was checked rather
than assumed.** `grep -rn -i 'rename_all\|serde\|PgRepositories\|SqlClientAssertionStore' docs/flows/`
matches nothing; no flow document makes a claim these two gates could falsify.
`docs/reference/rails.md` §"serde: `rename_all` is for *our* wire, never a
rail's" already stated standard 3 correctly and was left verbatim — the ADR
cites it rather than restating it.

## Standard 3 (serde) — before and after

Measured with the gate itself on the unmodified base tree, with an exemption
table containing zero rows:

| | count |
|---|---|
| serialisable types under `backends/crates/*/src` | 64 |
| carrying `rename_all = "snake_case"` or renaming every member | 36 |
| **violations before** | **28** |
| fixed by adding the attribute | 13 |
| exempted with a reason | 15 |
| **violations after** | **0** |

`cargo xtask verify-serde` now prints
`49 serialisable type(s) spell the workspace's wire convention, 15 exempted`.

**28 is not a count of defects.** The convention was already written down —
in `docs/reference/rails.md`, in `vpay-adapter-mtn-momo/src/wire.rs`'s module
doc, in `vpay-adapter-orange-money/src/wire.rs`'s module doc, and in a comment
above `vpay_core::Currency`. Four correct statements of the same rule, in four
files, checked by nobody. Every one of the 15 exemptions is a case one of
those four paragraphs had already argued for; the gate did not discover them,
it made them enforceable and made *deleting* one a build failure.

### The 13 fixed, and why none of them moved a wire

| type | file |
|---|---|
| `SessionCredential`, `ReturnCredential`, `OriginsQuery`, `Origins` | `vpay-api/src/browser/checkout_sessions.rs` |
| `PayerCredential`, `BrowserConfirmParams` | `vpay-api/src/browser/mod.rs` |
| `CheckoutMerchantObject`, `CheckoutSessionForPayer` | `vpay-api/src/model.rs` |
| `RawClaims` | `vpay-api/src/resource_auth.rs` |
| `WebhookPolicy` | `vpay-config/src/config.rs` |
| `PollChargePayload`, `ResubmitPayload`, `DeliverWebhookPayload` | `vpay-worker/src/jobs.rs` |

All thirteen are **structs with named fields**, and every field of every one of
them is already snake_case in Rust — which is what rustc's own
`non_snake_case` lint guarantees for a field name nobody wrote an `#[allow]`
for. `rename_all = "snake_case"` maps a snake_case identifier to itself, so
the attribute is the identity function on each of these types and **no
serialised name changed**. That is why the diff touches no fixture, no
`sdks/*` type, no `.sql` and no Cypress spec.

This is the claim most worth attacking in this branch, so here is how it was
checked rather than asserted: the attribute was added only to
`DeclShape::NamedFields` declarations (the gate reports the shape, and the
message for an enum says "does not rename every **variant**"); no enum was
fixed, because `rename_all` on an enum *does* change the wire — `PollCharge`
would become `poll_charge`. The four enums in the 28 are all exempted instead,
three of them `#[serde(untagged)]` and one already carrying
`rename_all = "UPPERCASE"`.

### The 15 exempted, grouped by the reason

- **Models a rail's wire (10):** `RequestToPay`, `StatusResponse`, `ApiError`
  and `TokenResponse` in `vpay-adapter-mtn-momo`; `WebPaymentRequest`,
  `WebPaymentResponse`, `TransactionStatusRequest`,
  `TransactionStatusResponse`, `TokenResponse` and `CallbackBody` in
  `vpay-adapter-orange-money`. MTN's bodies are camelCase and the per-field
  `rename`s are what make them exact; Orange's happen to be snake_case, which
  makes the attribute *more* dangerous there and not less, because it would
  read as a promise that those names are ours to normalise.
- **`#[serde(untagged)]` (4):** `ExpiresIn` and `Reason` and `Scalar` in
  `vpay-adapter-mtn-momo`, `ExpandableIntent` in `vpay-api`. A variant name of
  an untagged enum never reaches a wire, so there is nothing to rename.
- **A frozen external vocabulary (1):** `vpay_core::Currency` carries
  `rename_all = "UPPERCASE"` — ISO-4217 codes, which the database, both
  adapters and `Currency::code` already agree on.

The full table with one sentence each is in the ADR, which is where the gate
reads it from.

### Two judgement calls, stated because they could have gone the other way

1. **Scope: every serialisable type, not only `pub` ones.** The brief says
   "`pub struct`/`pub enum`", which in Rust also literally covers
   `pub(crate) struct`. I read it as the broader set — every declaration with
   a serde derive, whatever its visibility — and the gate is written that way.
   The measurement is why: **only 8 of the 28 violations are `pub`.** The
   other 20 are `pub(crate)`, `pub(super)` or private, and 13 of those 20 are
   the two adapters' `wire.rs`/`token.rs`, which are `pub(crate)` in their
   entirety. Those are the types where getting a field name wrong costs a real
   payment. A gate scoped to `pub` alone would have found 8 violations, needed
   2 exemption rows (`ExpandableIntent` and `Currency`) and seen neither
   adapter. The cost of the broader scope is the other 13 rows of the table.
2. **`RawClaims` was fixed, not exempted.** It decodes a JWT, which is
   somebody else's wire, so by the adapters' own argument it could have been a
   sixteenth row. It is not, and the ADR records the distinction: `sub` and
   `scope` are RFC 7519 / RFC 6749 registered names in an IANA registry that
   cannot retroactively rename them, whereas a rail's product roadmap can
   change `pay_token` to `payToken` next quarter. If a reviewer disagrees,
   the change is one table row and deleting one attribute — and the
   two-directional check means neither can be done by halves.

## Standard 5 (repositories) — before and after

| | count |
|---|---|
| concrete implementations found in `vpay-db` | 3 |
| source files scanned outside `vpay-db` | 65 |
| **violations before** | **2** |
| **violations after** | **0** |

The 3 are `PgRepositories`, `PendingTransaction` and
`SqlClientAssertionStore`. The 2 violations were both
`backends/crates/vpay-api/src/op/mod.rs`, lines 26 and 215: a `use
vpay_db::{Repositories, SqlClientAssertionStore}` and a
`SqlClientAssertionStore::new(pool)`.

**Fixed rather than exempted.** `SqlClientAssertionStore` is now `pub(crate)`
and `vpay_db::client_assertion_store(pool) -> impl ClientAssertionStore` is
the whole public surface, so `vpay-api` gets the behaviour with no way to
spell the type — the same shape `PgRepositories::boxed` already used for
`Arc<dyn Repositories>`. `impl Trait` rather than `Arc<dyn …>` because
`authkestra_op`'s `CompositeOpStore::with_client_assertion_store` takes
`J2: ClientAssertionStore` by value and there is no blanket impl for `Arc<T>`
in that crate; a newtype wrapper would have been a second public type to name.

The gate has **no exemption mechanism**, and that is the deliberate half of
the design: there is no exception today, and an escape hatch nobody needs is
the one that gets used. `Repositories::op_store_pool` (Step 7's decision 9)
stays exactly as it was — it is a decision about a raw *pool*, not a licence
to name an implementation type.

### The set is derived, not listed

Two signals unioned, both read out of `vpay-db`'s own source:

- a declaration whose body holds a `PgPool` or a `Transaction` — it owns a
  connection, so it is an implementation and not a row struct;
- a type on the right of `impl <a trait `vpay-db` declares `pub`> for …`.

Neither alone is enough, and this is measured rather than argued:
`SqlClientAssertionStore` implements a *foreign* trait
(`authkestra_op::client_assertion::ClientAssertionStore`), so the second
signal cannot see it; a hypothetical implementation that reaches its pool
through another type would be invisible to the first. The unit tests pin which
signal catches which.

**One false positive the first draft had, and how it was found.** The first
version of the impl scanner counted a *blanket* impl's target:
`impl<S: TransactionSource + ?Sized> UnitOfWork for S` in
`vpay-db/src/repository.rs` put the single letter `S` into the set of
"concrete implementations", and `S` appears in a generic bound throughout
`vpay-api` — the gate failed 37 times on the workspace it is supposed to pass
on. It was caught by `repository_tests::the_repositorys_own_tree_passes`
before any of it was believed. The fix parses the impl's own generic parameter
list and refuses a target that is one of them; the comment at that line says
so, with the number.

## Standard 6 — the two report lines, and the baseline they establish

`cargo xtask verify-docs` now prints a second table. Measured 2026-09-05 over
`backends/crates` and `backends/apps`, `src/` only:

```
  crate                       comment     code     ratio  include_str
  vpay-adapter-mtn-momo            62      625      9.9%            0
  vpay-adapter-orange-money        54      572      9.4%            0
  vpay-api                        740     4968     14.8%            0
  vpay-config                      95      965      9.8%            0
  vpay-core                        49      842      5.8%            0
  vpay-db                         228     2830      8.0%            0
  vpay-ledger                      11       89     12.3%            0
  vpay-provider                    70      507     13.8%            0
  vpay-testkit                      0       89      0.0%            0
  vpay-worker                     337     3024     11.1%            0
  vpay-server                      76      358     21.2%            0
  vpay-worker-bin                  67      288     23.2%            0
  TOTAL                          1789    15157     11.8%            0
```

`comment` counts `//` lines that are neither `///` nor `//!` — the in-file
kind standard 6 asks for fewer of, and deliberately *not* `doc`, which is the
documentation the same standard asks for more of. **0 `#[doc = include_str!]`
modules is the honest number**: the externalised-module-doc habit does not
exist in this tree yet, and printing zero is more useful than not printing it.

**Nothing was moved.** No comment was deleted, no module doc was externalised,
no ratio was improved. The brief asked only for the measurement, and a
baseline taken on a tree that was tidied first is not a baseline. Both numbers
are advisory and neither can fail a build; the ADR records why, and it is the
same reason Step 7 gave for the doc-ratio number.

## Mutations

Every one below was applied to the real tree, run through **`just verify`**
(not the gate in isolation), and reverted with `git checkout --`. Each has a
unit test that drives the same case over a synthetic tree, so the mutation is
reproducible without editing production code.

| # | mutation | `just verify` | what it printed | unit test |
|---|---|---|---|---|
| M1 | delete `#[serde(rename_all = "snake_case")]` from `DeliverWebhookPayload` (`vpay-worker/src/jobs.rs`), a type with no exemption | **exit 1** | ``backends/crates/vpay-worker/src/jobs.rs:241: `DeliverWebhookPayload` derives serde but carries no `#[serde(rename_all = "snake_case")]`, does not rename every field, and is not in docs/adr/0016-engineering-standards.md's exemption table`` | `serde_tests::deleting_the_attribute_is_visible_to_the_gate_itself` |
| M2 | add `use vpay_db::PgRepositories;` to `vpay-api/src/op/mod.rs` | **exit 1** | ``backends/crates/vpay-api/src/op/mod.rs:27: `PgRepositories` is a concrete repository implementation in `vpay-db`; name the trait instead`` | `repository_tests::a_consumer_naming_a_concrete_type_fails_the_gate_itself` |
| M3 | delete the exemption row for `Currency`, which still needs it | **exit 1** | ``backends/crates/vpay-core/src/money.rs:32: `Currency` derives serde but carries no … and is not in … the exemption table`` | `serde_tests::deleting_a_needed_exemption_fails` |
| M4 | add an exemption row for `PollChargePayload`, which complies | **exit 1** | ``docs/adr/0016-engineering-standards.md:137: `PollChargePayload` (backends/crates/vpay-worker/src/jobs.rs:141) is exempted but complies — delete the row`` | `serde_tests::an_exemption_for_a_complying_type_fails` |

M3 and M4 together are the two-directional property: the table cannot be
satisfied by adding rows and cannot be satisfied by removing them. A fifth
case — a row naming a type that does not exist at all, which is what a rename
leaves behind — is covered by
`serde_tests::an_exemption_naming_nothing_fails` and by
`serde_tests::an_exemption_is_keyed_by_file_as_well_as_name` (two crates each
have a `CallbackBody` and a `TokenResponse`; an exemption for one must not
cover the other).

Two mutations that must **not** fire, because the cheapest way to clear a
badly-written gate is to delete an honest sentence:

- a doc comment quoting `` `#[serde(rename_all = "snake_case")]` `` — both
  adapters' module docs do exactly this at length — satisfies nothing
  (`serde_tests::a_comment_quoting_the_attribute_satisfies_nothing`);
- a doc link to `PgRepositories` from `vpay-api`, and a `#[cfg(test)]`
  construction of one, are not reaches
  (`repository_tests::a_doc_link_and_a_test_construction_are_not_reaches`).

## Gate results

```
$ just verify
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 15 error type(s), all classified; 14 `#[from]` variant(s) delegate every `Classify` method they match on; anyhow confined to binaries
verify-sdk-parity: ok — 342 proving test(s) named in docs/sdks/parity.md all exist, 26 dated gap(s)
verify-links: ok — 698 repository link(s) in 122 tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)
verify-npm-scope: ok — 2 publishable package(s) under sdks/ (@vaam-apps/vpay-sdk (sdks/nodejs/package.json), @vaam-apps/vpay-stripe-js (sdks/stripe-js/package.json)), 1 private one(s) declaring no publishConfig, and no retired package name outside docs/plans, docs/adr and docs/status.md
check-schema: cratestack 0.11.1, schema schemas/vpay.cstack (12 model/enum declarations, datasource present)
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.11.1
verify-serde: ok — 49 serialisable type(s) spell the workspace's wire convention, 15 exempted with a reason in docs/adr/0016-engineering-standards.md
verify-repositories: ok — 3 concrete implementation(s) in backends/crates/vpay-db (PendingTransaction, PgRepositories, SqlClientAssertionStore), named by none of the 65 source file(s) outside it
verify: ok — the nine gates above passed; the verify-docs report is advisory
```

*(The `verify-docs` report prints between `verify-repositories` and the final
line and is elided here; it is reproduced under "Standard 6" above. This is
the command's real output, re-captured 2026-09-05 after the review's commits.
An earlier revision of this file abbreviated four of these lines and rendered
`check-schema` as a string the recipe does not print. **`verify-links`' two
numbers count this branch's own documents**, so they move whenever a file or a
link is added here — that first revision read `697 ... 121`, and adding the
ADR's links and `opus-review.md` is the whole difference. A transcript pasted
into a tracked document is a claim about the tree that contains it.)*

- `just docs-check` (`verify-status`, `verify-links`): ok.
- `just fmt-check`: ok.
- `just clippy` (`--workspace --all-targets -- -D warnings`): ok. One lint was
  hit and fixed rather than allowed (`clippy::manual_contains` on the blanket
  impl parameter check).
- `cargo test -p xtask`: **144 before, 184 after, 0 ignored** (181 as first
  delivered; the review added 3 for the alias evasion — see
  `docs/plans/exp10-notes/opus-review.md`, finding 1).
- `actionlint .github/workflows/ci.yml`: ok.
- `just test-rust`: **1257 run, 1257 passed, 0 skipped** as first delivered,
  and **1260 total** after the review's three xtask guards
  (`just verify-ignored`: 0 ignored, 42 test binaries, 1260 total).
  Run because non-test Rust outside `.xtask` changed — see below.

## What I did not do

- **No `docs/reference/<crate>.md` was written or moved, and no comment was
  deleted.** Standard 6's "externalise the documentation" half is *measured*
  by this branch and not *applied* by it. The `include_str` column is 0 for
  every crate, which is the accurate state.
- **Standard 4 has no gate**, and the ADR says so in its own section rather
  than in a footnote. Nothing here measures SOLID or DRY.
- **Standard 2's "no `if provider == …` outside the adapters" has no gate
  either.** ADR-0002 states the rule, `verify-errors` covers the adapters'
  error surface, and nothing reads for a provider-code branch. It is named as
  a known gap in ADR-0016 standard 2 rather than left implied.
- **`verify-serde` does not judge a reason.** A row saying "too many to fix"
  passes. Only a blank reason fails.
- **`verify-serde` does not read `#[serde(skip)]`, `#[serde(flatten)]` or
  `#[serde(rename(serialize = …, deserialize = …))]`.** Each of the three is a
  member that serialises no name of its own — or names it in a spelling the
  gate does not parse — and each is still counted as a member that has to be
  renamed under the "rename every field" alternative. So a struct mixing one
  of them with explicit renames needs the blanket attribute or a row. Every
  such miss is in the direction that *fails* a compliant type rather than
  passing a non-compliant one, which is the safe direction. Six types in
  `backends/crates/*/src` carry a `#[serde(flatten)]` field
  (`PaymentIntentWithSecret`, `CheckoutSessionForPayer`,
  `CheckoutSessionWithSecret`, `BrowserConfirmParams`, and
  `v1::payment_intents`' `CreateParams` and `ConfirmParams`); all six take the
  blanket attribute, so none is affected. No type uses the two-sided
  `rename`, and none mixes `skip` with explicit renames.
  *(`flatten` and the two-sided `rename` were added to this list on review,
  2026-09-05; the original named only `skip`. A first draft of this bullet
  said "one type" — it is six, counted with
  `grep -rn 'serde(flatten)' backends/crates --include='*.rs' | grep /src/`.)*
- **`verify-serde` sees `derive`d implementations only.** A hand-written
  `impl Serialize for X` is invisible to it. That is the rule as ADR-0016
  states it ("every type **deriving** `Serialize`/`Deserialize`"), and the
  four such impls in the workspace are `model.rs`'s `object_tag!` unit structs,
  which serialise one fixed string and have no field names to rename. A
  hand-written impl over a struct with named fields would not be caught.
  *(Recorded on review, 2026-09-05.)*
- **Neither gate scans `sdks/rust`, `examples/`, `backends/tests` or a crate's
  own `tests/`.** The serde rule is about `backends/crates/*/src` because that
  is where vpay's wire is defined; a test fixture names nothing a merchant can
  reach. The SDKs model the wire the API emits and are covered by
  `verify-sdk-parity` and the conformance suite instead.
  `verify-repositories`' consumer set is `backends/crates` plus
  `backends/apps`; nothing under `examples/` or `sdks/` depends on `vpay-db`
  today (checked, not assumed — `grep -rn 'vpay-db' --include=Cargo.toml`
  lists only `vpay-api`, `vpay-worker`, both binaries and
  `backends/tests/integration`), so the scope has no live hole, but a new
  crate outside those two directories that took the dependency would not be
  scanned. *(`examples/` added to this list on review, 2026-09-05.)*
- **`docs/plans/*` and `docs/roadmap.md`'s "Original text, for the record"
  block still spell `vpay_db::SqlClientAssertionStore`, and were left
  alone.** They are dated records of what was built on the day they were
  written, and this repository's convention is to correct a false *claim*, not
  to rewrite history. The two live statements that named the path as something
  a caller can write — `docs/reference/vpay-db.md` and `docs/roadmap.md`'s
  Postgres-over-Redis rationale — were updated.
- **The exemption table is keyed on `(file, type)` textually.** Moving
  `wire.rs` to another path is a table edit, and the gate will say so — but it
  says "found no such serialisable type there", not "the file moved".
