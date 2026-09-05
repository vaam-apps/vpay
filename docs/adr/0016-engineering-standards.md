# ADR-0016: Six engineering standards, three of them machine-checked

- **Status:** Accepted
- **Date:** 2026-09-05
- **Deciders:** vpay maintainers

## Context

By 2026-09-05 this repository had a documented decision for almost every
*structural* choice — the provider port ([ADR-0002](0002-provider-port.md)),
configuration over environment branching ([ADR-0003](0003-yaml-configuration.md)),
error modelling ([ADR-0011](0011-error-modelling.md)), the lint policy
([ADR-0007](0007-lint-policy.md)), SDK parity ([ADR-0015](0015-sdk-parity.md)) —
and no written decision at all for the *habits* those structures are built
with. The habits existed; they were simply spread across places nobody reads
as a rule:

- `AGENTS.md` said "`thiserror` for library crates, `anyhow` only in binaries",
  which ADR-0011 later turned into a three-tier model with a gate.
- The serde convention — "`rename_all` is for *our* wire, never a rail's" —
  was written in `docs/reference/rails.md`, in the module doc of
  `vpay-adapter-mtn-momo/src/wire.rs`, in the module doc of
  `vpay-adapter-orange-money/src/wire.rs`, and in a comment above
  `vpay_core::Currency`. Four correct statements of the same rule, in four
  files, checked by nobody. **Measured before this ADR: 28 of the 64
  serialisable types under `backends/crates/*/src` did not carry the
  attribute** — and only 15 of those 28 had a reason.
- The repository split was real (`PgRepositories` is `pub(crate)` and
  `vpay_db::connect` is the only way to obtain one) and had one hole:
  `SqlClientAssertionStore` was `pub`, and `vpay-api` named it. Nothing said
  that was a defect, so nothing said it was fixed either.
- "Doctests so documentation cannot lie" arrived in Step 7 as a task, not as a
  rule, and `just test-doc` exists because before it not one doctest in the
  workspace had ever been compiled by CI.

The failure mode this ADR is about is not "the code is wrong". It is that a
convention which lives only in prose is a convention that decays silently, and
this repository's whole discipline is that a claim nobody checks decays. Three
of the six standards below are mechanical enough to check; three are not, and
saying which is which is most of the value of writing them down.

## Decision

Six standards. Each states the rule, what enforces it, and what a human still
has to judge.

### 1. Errors: typed at the leaves, composed per layer, classified once

`thiserror` enums in library crates, layered domain → service/port →
transport, `anyhow` only at a binary edge, and each error type implements
`vpay_core::error::Classify` exactly once at its own layer.

This is not a new decision — it is [ADR-0011](0011-error-modelling.md) and its
2026-09-03 amendment, restated here so the six standards read as one list.
ADR-0011 remains the authority for the detail; nothing here modifies it.

- **Mechanically enforced by** `cargo xtask verify-errors`: a `pub`
  `…Error`/`…Rejection` type in `backends/crates` with no `impl Classify`
  fails; `anyhow` under a library crate's `[dependencies]` fails; a `#[from]`
  variant that a composite's `Classify` method answers for with a wildcard
  instead of naming fails.
- **Left to review:** whether a leaf's `category()` is the *right* category,
  and whether an override carries the comment ADR-0011 asks for.

### 2. Rails stay behind the trait, and every adapter has a clean error surface

The port is `vpay_provider::ProviderAdapter`, an `async_trait` object; a rail
is reached only through it. Every adapter maps its rail's failures into the
closed `FailureCode` vocabulary and into `ProviderError`, never into a
stringly-typed catch-all.

- **Mechanically enforced by** `cargo xtask verify-errors` (the adapters'
  error enums are covered by standard 1) and by the shared conformance suite
  in `backends/tests/conformance`, which is one suite parameterised over every
  adapter. `cargo xtask verify-no-mocks` keeps a test double out of the
  process that would otherwise be the easy way to fake an adapter.
- **Left to review:** that a new rail's failures were *mapped* rather than
  flattened, and that no `if provider == "…"` appeared outside
  `backends/crates/vpay-adapter-*` (ADR-0002's rule; no gate reads for it
  today, and that is a known gap).

### 3. `#[serde(rename_all = "snake_case")]` on everything vpay serialises

Every type deriving `Serialize` or `Deserialize` under
`backends/crates/*/src` carries `#[serde(rename_all = "snake_case")]`, **or**
renames every field/variant itself with `#[serde(rename = "…")]`, **or** is
listed in the exemption table below with a reason.

Rust visibility is not part of the rule. `verify-errors` scans `pub` types
only, because a `pub(crate)` error reaches no boundary — but a `pub(crate)`
type with a `Serialize` derive reaches a *rail*, and both adapters' entire
wire modules are `pub(crate)`. A wire does not care what Rust thinks of a
type's visibility.

A tuple struct and a unit struct are compliant by construction: neither
serialises a name, so the attribute would rename nothing and requiring it
would be a rule about characters.

**The rule the exemptions are all instances of:** `rename_all` is a statement
about *our* wire. Where the names belong to somebody else — a rail's JSON, an
`untagged` union whose variant names never appear at all — the attribute is
either inert or actively dangerous, because the day the other party sends a
name that is not already snake_case the attribute renames it away from their
spelling. This is `docs/reference/rails.md` §"serde: `rename_all` is for *our*
wire, never a rail's", now with a gate behind it.

The line this ADR draws, because it is the one that will be argued about:
a vocabulary frozen in a standards registry is **not** an exemption.
`vpay_api::resource_auth::RawClaims` decodes RFC 7519 registered claims
(`sub`) and RFC 6749's `scope`; those names are snake_case and the IANA
registry cannot retroactively rename them, so the attribute states the truth
rather than making a promise. A rail's product roadmap is not a registry.

#### Exemption table

The gate reads this table. It is two-directional: a row naming a type that
now complies, or a type that no longer exists, **fails the build** — a stale
exemption describes a decision the code has already reversed, and the next
person reads it as current.

| Type | File | Reason |
|---|---|---|
| `TokenResponse` | `backends/crates/vpay-adapter-mtn-momo/src/token.rs` | Models MTN's OAuth token response. Already snake_case by coincidence, which is what makes the attribute a promise rather than a no-op. |
| `ExpiresIn` | `backends/crates/vpay-adapter-mtn-momo/src/token.rs` | `#[serde(untagged)]` — variant names never reach the wire, so there is nothing for `rename_all` to rename. |
| `RequestToPay` | `backends/crates/vpay-adapter-mtn-momo/src/wire.rs` | Models MTN's camelCase Collections wire (`externalId`); the per-field `rename`s are what make it exact. |
| `StatusResponse` | `backends/crates/vpay-adapter-mtn-momo/src/wire.rs` | Models MTN's camelCase Collections wire (`financialTransactionId`). |
| `Reason` | `backends/crates/vpay-adapter-mtn-momo/src/wire.rs` | `#[serde(untagged)]` — MTN sends `reason` as a bare string or as an object, and neither shape carries a variant name. |
| `Scalar` | `backends/crates/vpay-adapter-mtn-momo/src/wire.rs` | `#[serde(untagged)]` — a value MTN sends as a string or a number; no variant name on the wire. |
| `ApiError` | `backends/crates/vpay-adapter-mtn-momo/src/wire.rs` | Models MTN's error envelope. That module's own doc comment forbids `rename_all` for every type in it, for the reason above. |
| `WebPaymentRequest` | `backends/crates/vpay-adapter-orange-money/src/wire.rs` | Models Orange's Web Payment wire. Snake_case today, which makes the attribute more dangerous rather than less. |
| `WebPaymentResponse` | `backends/crates/vpay-adapter-orange-money/src/wire.rs` | Models Orange's Web Payment wire. |
| `TransactionStatusRequest` | `backends/crates/vpay-adapter-orange-money/src/wire.rs` | Models Orange's Web Payment wire. |
| `TransactionStatusResponse` | `backends/crates/vpay-adapter-orange-money/src/wire.rs` | Models Orange's Web Payment wire. |
| `TokenResponse` | `backends/crates/vpay-adapter-orange-money/src/wire.rs` | Models OAuth 2's token response as Orange serves it. |
| `CallbackBody` | `backends/crates/vpay-adapter-orange-money/src/wire.rs` | Models the body Orange POSTs to `notif_url`. |
| `ExpandableIntent` | `backends/crates/vpay-api/src/model.rs` | `#[serde(untagged)]` — the wire shape is a string or an object with no discriminator, exactly as Stripe's expansion is. |
| `Currency` | `backends/crates/vpay-core/src/money.rs` | `rename_all = "UPPERCASE"`: ISO-4217 codes, not vpay field names. `"XAF"` is the spelling the database, both adapters and `Currency::code` already agree on. |

Fifteen rows, and the reason each one gives is a claim about somebody else's
wire that a reviewer can check against that rail's documentation. **The gate
cannot judge a reason** — "models MTN's camelCase Collections wire" and "too
many to fix" are both non-empty strings — so it only refuses a blank one. The
table exists to put the sentence where a reviewer will see it.

- **Mechanically enforced by** `cargo xtask verify-serde`, in `just verify`
  and CI's `self-checks` job.
- **Left to review:** whether a reason is honest.

### 4. SOLID and DRY

Stated as an aspiration and enforced by review, not by a gate, and this ADR
says so rather than pretending otherwise. What is actually checkable about it
here is already covered by other rules: single responsibility shows up as the
port (standard 2) and the repository split (standard 5); dependency inversion
is what makes both of those traits rather than types; "don't repeat yourself"
is what the one-conformance-suite rule and `vpay_core`'s single
`Money::to_provider_string` are.

- **Mechanically enforced by** nothing, deliberately. A cyclomatic-complexity
  or duplicate-token gate measures the shape of code rather than whether a
  responsibility is in the right place, and the cheapest way to pass one is a
  worse design that scores better. `cargo xtask verify-docs` *reports* every
  production function of 80 lines or more, which is the closest honest proxy
  and is not a gate for the same reason.
- **Left to review:** all of it. A reviewer asking "what would have to change
  for this to be wrong, and does that live in one place?" is the enforcement.

### 5. Repositories are traits; their implementations are private to `vpay-db`

`vpay-db` declares one trait per table concern (`Charges`, `Jobs`,
`PaymentIntents`, …), one umbrella `Repositories`, and `TxRepositories` for
work inside a transaction. Every implementation of those traits is
`pub(crate)`; `vpay_db::connect` returns `Arc<dyn Repositories>` and is the
only way to obtain one. A handler or a service names the trait — never
`PgRepositories`, never a `Sql…Store`.

Same rule for a store over a *foreign* trait. `SqlClientAssertionStore`
implements `authkestra_op`'s `ClientAssertionStore`; it was `pub`, and
`vpay-api` constructed it by name. It is now `pub(crate)`, reached through
`vpay_db::client_assertion_store`, which returns `impl ClientAssertionStore` —
so the caller has the behaviour and no way to name the type.

The exemption that already existed stays exactly as it was:
`Repositories::op_store_pool` is the one place a raw `sqlx` pool leaves the
crate, because `authkestra_op::sqlx_store::SqlxOpStore` is a foreign
implementation over a pool whose queries vpay does not own (Step 7's decision
9, `docs/status.md`). That is a decision about a *pool*, not a licence to name
an implementation type.

- **Mechanically enforced by** `cargo xtask verify-repositories`. It derives
  the set of concrete implementations from `vpay-db`'s own source — a
  declaration holding a `PgPool`/`Transaction` field, a type on the right
  of `impl <a vpay-db trait> for …`, or a name `vpay-db` publishes for one of
  those (`pub use … as`, `pub type`, to a fixpoint) — rather than from a list
  here, so a store nobody has written yet is covered the day it is added. The
  third signal is there because the gate matches names textually and the first
  draft of it did not have one: `pub use repository::PgRepositories as Repos;`
  in `vpay-db` plus `use vpay_db::Repos;` in `vpay-api` is the same type
  reaching the same handler under a word the gate had never heard of, and both
  spellings passed. There is no exemption mechanism, because there is no
  exception today and an escape hatch nobody needs is the one that gets used.
- **Left to review:** whether a new method belongs on an existing trait or on
  a new one, and whether a query behind `op_store_pool` is a repository method
  that was not written.

### 6. Doctests, and documentation externalised to `.md`

An example in a doc comment is compiled and run — `just test-doc` is in
`just ci` and in CI's `rust` job. ` ```ignore ` and ` ```no_run ` are not the
way to make an example compile: an example nothing runs is a claim nothing
checks. The reasoning behind a piece of code belongs in
`docs/reference/<crate>.md`, reached from a one-paragraph module doc;
`# Errors`, `# Panics` and `# Examples` stay in the source. Where a module doc
is long enough to be a document, it is one: `#[doc = include_str!("…md")]`.

- **Mechanically enforced by** `just test-doc` (a doctest that stops
  compiling fails CI) and, partially, by `cargo xtask verify-docs`, which
  *reports* every ` ```ignore ` fence, the doc-comment volume per crate, the
  non-doc comment volume per crate, and the number of `#[doc = include_str!]`
  modules. **The report is not a gate and must not become one:** the cheapest
  way to pass a comment-volume gate is to delete the `# Errors` sections
  ADR-0011 depends on. That is Step 7's decision (4) and this ADR keeps it.
- **Left to review:** whether a module doc is a paragraph and a link or an
  80-line essay, and whether an in-file comment is explaining *why* or
  restating the line below it.

### Migration rule

**Existing code is migrated as it is touched; a new crate complies from its
first commit.** No sweep. Standards 3 and 5 were brought to zero violations in
the change that introduced this ADR, because a gate cannot land otherwise —
those are the two that had a countable backlog. Standards 1, 2 and 6 already
had their gates. Standard 4 has no backlog to count, which is another way of
saying it has no gate.

## Alternatives considered

- **A ratio gate on comments-to-code.** Rejected, again — Step 7 decision (4)
  settled it and this ADR does not reopen it. The number is now *reported* per
  crate so the rule in standard 6 has a baseline, and a baseline is what makes
  a later argument about it evidence-based rather than aesthetic.
- **An allowlist in `.xtask` instead of a table in the ADR.** Rejected: a
  constant in a Rust file is not where a reviewer looks for the reason an
  exception exists, and a reason nobody reads is the same as no reason. The
  same instinct puts `verify-status`' token list in `docs/status.md` and the
  parity matrix in `docs/sdks/parity.md`.
- **Making `verify-serde` scan `pub` types only**, matching `verify-errors`.
  Rejected on the measurement: only 8 of the 28 violations were `pub`. The
  other 20 were `pub(crate)`, `pub(super)` or private, and 13 of those 20 are
  the two adapters' `wire.rs`/`token.rs` — the types where getting a field
  name wrong costs a real payment, and the modules whose own doc comments had
  been arguing this rule to nobody for two days. Visibility is the wrong axis
  for a rule about a wire.
- **Exempting `resource_auth::RawClaims` as "a foreign wire".** Rejected; see
  standard 3 for the distinction. Left recorded because it is the exemption
  most likely to be proposed again.
- **A gate for standard 4.** Rejected: see standard 4.

## Consequences

- Two more gates in `just verify` (nine, plus the advisory `verify-docs`
  report), and two more steps in CI's `self-checks` job.
- Adding a serialisable type to `backends/crates` costs one attribute, or one
  table row and a sentence. Adding a repository costs nothing new — the rule
  is what the crate already did.
- A stale exemption is now a build failure rather than a paragraph nobody
  re-reads. That is the direction that rots, and it is the direction
  `verify-status` had to learn to check the hard way.
- `vpay_db::SqlClientAssertionStore` is no longer public API. The two test
  suites that constructed it by name go through `vpay_db::client_assertion_store`.
- This ADR is immutable like every other. A standard that turns out to be
  wrong is superseded by a new ADR, not edited here — including the exemption
  *table*, which is the one part of this document a routine change touches.
  Adding or removing a row is a change to an accepted decision's data, not to
  the decision; it is expected, and the gate is what keeps it honest.
