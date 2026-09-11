# CrateStack — the adoption, and the 0.11.1 → 0.12.0 bump

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

**The per-step measured findings are on nine dated pages**, and the text below jumps
straight from the adoption section to the 0.11.1 → 0.12.0 bump because they sat
between the two on `docs/status.md` as it was:

- [cratestack/drift.md](cratestack/drift.md) — the measured migration/model drift
- [cratestack/2026-09-06-first-read.md](cratestack/2026-09-06-first-read.md)
- [cratestack/2026-09-06-outbox.md](cratestack/2026-09-06-outbox.md) — the transaction seam
- [cratestack/2026-09-06-first-writes.md](cratestack/2026-09-06-first-writes.md)
- [cratestack/2026-09-06-currencies-and-providers.md](cratestack/2026-09-06-currencies-and-providers.md) — migration 0032
- [cratestack/2026-09-06-providers-and-d7.md](cratestack/2026-09-06-providers-and-d7.md) — migration 0033, decision D7
- [cratestack/2026-09-06-customers.md](cratestack/2026-09-06-customers.md)
- [cratestack/2026-09-07-money-tables.md](cratestack/2026-09-07-money-tables.md) — migration 0037
- [cratestack/2026-09-11-search-payment-intents.md](cratestack/2026-09-11-search-payment-intents.md) — the first `procedure`

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
`cargo install` run _inside_ the worktree refused with

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
what 0.11 _added_ over 0.10. The block attributes the schema's own header
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
**0** for this schema with its `datasource` block deleted _and_ `tags
String[]` added — the CLI's own list-arity error names "drop the `datasource`
block" as one way to make it go away, so the mutation this gate is proven
with is exactly the one that block's absence disarms. Both are correct
behaviour for the CLI (a client-only schema is a real thing) and there is no
flag that asks for more, so `check-schema` asserts the shape of what it
checked before reporting green: a `datasource` block must be present, and the
file must still declare at least `cratestack_min_declarations` (15 since
2026-09-06: nine models, six enums — the sub-count still read "seven models"
after the floor moved 13 -> 15, which is the 13 it used to explain; 12 before
that; **and all of that is the arithmetic of the floor on the day it was set,
not a count of the file — `check-schema` reports 25 declarations today,
2026-09-10: seventeen models and eight enums, against a floor still at 15**)
top-level
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
  its _grammar cannot express_, which the gate does not touch; neither
  Status section makes a claim about whether the file parses.
- ~~**Content is a design sketch, not full coverage.** It models only entities
  with a real, tested Rust type to mirror: `Currency`, `Provider`,
  `PaymentIntent`, `Charge`, and the new `LedgerTransaction`/`LedgerEntry`
  pair. It deliberately omits `provider_requests`, webhooks/outbox, the job
  queue, idempotency keys and `Merchant`~~ **— corrected 2026-09-10 (issue
  #87). Both halves are stale: the file declares seventeen models, not six,
  and the outbox is two of them** (`Event` and `WebhookDelivery`, 2026-09-06),
  alongside `DisabledClient`, `Customer`, `CheckoutSession`, `Invoice`,
  `InvoiceItem`, `StaffMember`, `StaffSession`, `OauthAuthorizationCode` and
  `Refund`. What survives the correction is the _rule_ the bullet was stating,
  and it still holds: `provider_requests`, the job queue, idempotency keys and
  `Merchant` are modelled by nothing, because none of them has a backing Rust
  struct yet — and the file's own `GAP` comments say so rather than inventing a
  plausible shape. Twelve of the seventeen carry production statements; see the
  `schemas/*.cstack` row above for the list, the count and how to re-derive
  both.
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
  cannot be computed from the modelled data, because nothing says _which_
  merchant a `merchant_payable` posting belongs to. That is a real gap in the
  Rust type this schema mirrors, not something to paper over in the schema.

### CrateStack 0.11.1 → 0.12.0 (2026-09-07)

**Nothing in this repository had to change but the version.** The CLI and the
library moved together, as the pin's own comment requires: `justfile`'s
`cratestack_version`, `Cargo.toml`'s `cratestack = { package =
"cratestack-pg", version = "=0.12.0" }`, twelve `cratestack-*` entries in
`Cargo.lock`, and both `install-cratestack-cli` steps in
`.github/workflows/ci.yml` to the commit `v0.12.0` was tagged at
(`0823bab382425e1fe4d04c42b9657b7e7bb7b286`).

**What the bump could have broken, and what was measured instead of assumed.**

- _The gate._ 0.12.0's one breaking change gives `SchemaError` file identity,
  so a `check-schema` that parsed the CLI's diagnostic would be the obvious
  casualty. It does not parse it — and that was proven rather than reasoned
  about, by deleting a field's type from a copy of `schemas/vpay.cstack` and
  running the recipe: exit **1**, `Error: expected field type`, the same line
  0.11.1 prints. The green run reports `cratestack 0.12.0, schema
schemas/vpay.cstack (15 model/enum declarations, datasource present)`.
- _The drift constants._ `EXPECTED_DRIFT_CHANGES` and its two companions are
  measurements against a tool, so a tool bump is exactly when they can move
  with nobody touching the schema. Re-derived against a fresh
  `postgres:16-alpine` with the 0.12.0 binary on `PATH`: **101 changes / 16
  relations / 17 unmappable columns**, all three unchanged, and the test
  printed `cratestack CLI under test: 0.12.0 (justfile pins 0.12.0)`.
- _The licence surface._ The bump added **no package at all** — the set of
  package names in `Cargo.lock` is identical before and after, only twelve
  versions and their checksums moved — so `deny.toml`'s two Blue Oak
  exceptions are still the only ones needed. `cargo deny`: `advisories ok,
bans ok, licenses ok, sources ok`.
- _The toolchain floor._ Every `cratestack-*` 0.12.0 manifest still declares
  `rust-version = "1.98.0"`, `cratestack-cli` included, so
  `rust-toolchain.toml` and `backends/Dockerfile` did not move.
- _The action pin._ `git/ref/tags/v0.12.0` resolves to `0823bab382…` with
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
is in [reference/vpay-db.md § CrateStack](../reference/vpay-db.md#cratestack),
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
[plans/exp25-cratestack-012-notes/opus-review.md](../plans/exp25-cratestack-012-notes/opus-review.md).
