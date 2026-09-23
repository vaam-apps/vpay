# 2026-09-23 — Manual (out-of-band) invoice payments (RFC-0004 § 6)

Branch `feat/rfc-0004-manual-payments`, based on `origin/master` at
`d98fdaf`. Migration `0049_manual-payments.sql`, the `paid_out_of_band` path on
`POST /v1/invoices/{id}/pay`, the `manual_payments` table and model, the
erasure of the reference, twenty-one invoice keys, and both merchant SDKs.
[../../flows/invoices.md](../../flows/invoices.md) § "Paid out of band" is the
feature; this page is what was run.

## Environment, and one interruption

macOS (darwin 25.5), `rustc 1.98.0`, `cratestack 0.12.0` (on-pin),
`cargo-nextest 0.9.146`, pnpm 11.18.0, Docker 29.1.3. Every cargo invocation
ran with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=line-tables-only
CARGO_PROFILE_TEST_DEBUG=line-tables-only`, to keep the build small on a host
whose disk had filled; those change debuginfo and incremental caches only, not
behaviour.

**The host's disk filled partway through this work and Docker's daemon died
with it.** For a stretch every `docker ps` hung and testcontainers answered
`failed to create a container: Timeout error`, so the drift constants in
`postgres_smoke.rs` were first written from a line-by-line derivation rather
than a measurement. The daemon was restarted the same day, and **every
container-backed suite below ran after that, before the branch was
committed.** The derivation held line for line (below).

## The review round (second commit, 2026-09-23)

An independent review of `471d9b7` found no blocker and asked for fixes; the
maintainer then accepted ADR-0024 (`docs/adr/0024-customer-filters-and-manual-payments.md`,
PR #248, D9–D19 "as proposed"). What the second commit changed:

- `out_of_band[method]` is optional, defaulting to `other` (D11), so Stripe's
  bare `paid_out_of_band=true` succeeds;
- both SDKs take one optional `out_of_band` object (Rust
  `Option<OutOfBandParams>`, Node `outOfBand`) — **source-breaking in
  `sdks/rust`** for struct-literal callers without `..Default::default()`;
- migration `0049` (unshipped, so edited; manifest line re-derived) gained
  `invoices_payment_record_key`, a `manual_payments.paid_out_of_band` column
  with `records_an_out_of_band_payment`, and the composite foreign key
  `manual_payments_agree_with_their_invoice`;
- `received_at` against `finalized_at` is documented and tested **to the
  second**;
- the erasure, idempotency-race and touch cases the review found missing.

Docker failed a second time during this round (the Desktop VM had died behind
a "running" status) and was restarted before any of the runs below.

### Suites at the second commit

| Suite                                                                     | Passed | Failed | Ignored |
| ------------------------------------------------------------------------- | ------ | ------ | ------- |
| `vpay-tests-integration --test invoices` (real Postgres, shipping router) | 29     | 0      | 0       |
| `vpay-tests-integration --test customers`                                 | 24     | 0      | 0       |
| `vpay-tests-integration --test postgres_smoke`                            | 54     | 0      | 0       |
| `vpay-db`, all targets                                                    | 246    | 0      | 0       |
| `vpay-core` lib                                                           | 74     | 0      | 0       |
| `vpay-api` lib                                                            | 401    | 0      | 0       |
| `vpay-worker` lib                                                         | 75     | 0      | 0       |
| `vpay-sdk` (`sdks/rust`, without `live-stack`)                            | 183    | 0      | 0       |
| `@vaam-apps/vpay-sdk` (`sdks/nodejs`, `pnpm test`)                        | 226    | 0      | 0       |
| `just sdk-live`: `sdks/rust` `live_invoices` + `live_refunds`             | 5      | 0      | 0       |
| `just sdk-live`: `sdks/nodejs` live project                               | 6      | 0      | 0       |
| Doctests, `cargo test --doc --workspace`                                  | 124    | 0      | 1       |

**The live suites ran**, against the compose demo stack `just sdk-live`
brings up (torn down afterwards with `just demo_project=vpay-demo
demo-down`); both new live cases passed and are now in the parity row's ✅
cells.

**Drift, measured:** `drift detected in 26 table(s)/view(s) (201 change(s)
total)`, 19 unmappable. The two new lines, both predicted before Docker came
back: `manual_payments: [safe] CHECK records_an_out_of_band_payment …` and
`invoices: [safe] index invoices_payment_record_key …`. The composite foreign
key costs nothing, as 0.12.0 introspects no foreign key.

**The race case found its own blind spot.** Its first version, with a
`payment_first > 0` assertion added as the review asked, failed: the erasure
won 30 rounds of 30, because the pay request does three reads before its
transaction opens. The erasure's start is now staggered by 3 ms a round, so
the rounds sweep the collision window, and the case passes with at least one
round in which the payment wrote its reference first and every copy of it —
the replayed idempotent response included — came back as the marker.

## Gates

`just verify` before the change (on `d98fdaf`): all fifteen gates ok.

`just verify` after — all fifteen ok:

```text
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 20 error type(s), all classified; …
verify-sdk-parity: ok — 757 proving test(s) named in docs/sdks/parity.md all exist, 45 dated gap(s), 35 SDK method(s) enumerated across 40 row(s)
verify-links: ok — 2001 repository link(s) in 423 tracked markdown file(s) resolve to a tracked path
verify-npm-scope: ok — …
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.12.0   (29 model/enum declarations; 27 before)
verify-serde: ok — 104 serialisable type(s) …
verify-repositories: ok — …
verify-toolchain: ok — …
verify-migrations: ok — 49 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256
verify-versions: ok — 24 version references all say 0.5.0
verify-privacy-inventory: ok — 306 database column(s) classified … across 26 elements (17 personal-data, 17 necessary), checked in both directions against 306 column(s)
verify-doc-counts: ok — 13 documented count(s) in 11 of 273 markdown file(s) agree with what this tree measures
verify: ok — the fifteen gates above passed; the verify-docs report is advisory
```

The advisory `verify-docs` report now lists three new functions of 80 lines or
more, all in `vpay_api::v1::invoices`: `pay_out_of_band_once` (98),
`write_with_event` (94, which grew the fourth write), `pay_hosted_once` (81,
the pre-existing body of `pay_once` moved into a function).

`just migrations-manifest` **does not run on macOS** — it calls GNU
`find -printf` (`find: -printf: unknown primary or operator`). The `0049` line
was appended with `shasum -a 256`, the same digest `sha256sum` writes, and
`verify-migrations` accepts it. _(True when measured. Later on 2026-09-23 the
recipe was made portable and now runs on macOS; run there against this
tree it leaves that `0049` line, and the whole manifest, byte for byte as
they are — see
[2026-09-23-migrations-manifest-macos.md](2026-09-23-migrations-manifest-macos.md).)_

## Suites at the first commit (`471d9b7`)

| Suite                                                                     | Passed  | Failed | Ignored |
| ------------------------------------------------------------------------- | ------- | ------ | ------- |
| `vpay-tests-integration --test invoices` (real Postgres, shipping router) | 26      | 0      | 0       |
| `vpay-tests-integration --test postgres_smoke`                            | 54      | 0      | 0       |
| `vpay-db`, all targets (lib 128, `postgres` 4, `repositories` 114)        | 246     | 0      | 0       |
| `vpay-core` lib                                                           | 74      | 0      | 0       |
| `vpay-api` lib                                                            | 401     | 0      | 0       |
| `vpay-worker` lib                                                         | 75      | 0      | 0       |
| `vpay-sdk` (`sdks/rust`, without `live-stack`)                            | 181     | 0      | 0       |
| `@vaam-apps/vpay-sdk` (`sdks/nodejs`, `pnpm test`)                        | 224     | 0      | 0       |
| Doctests, `cargo test --doc --workspace`                                  | 124     | 0      | 1       |
| SDK live suites (`just sdk-live`)                                         | not run | —      | —       |

Ignored counts were taken with `cargo nextest list --run-ignored only` (0 in
each crate above). The one ignored doctest is `vpay_sdk`'s pre-existing
`no_run` quick-start in `lib.rs`. Doctests per crate: `vpay_core` 58 (one new,
`manual_payment_id`), `vpay_api` 19 (one new, `OutOfBandMethod`), `vpay_sdk`
9 + 1 ignored (one new, `OutOfBandMethod`), `vpay_db` 8, `vpay_provider` 12,
`vpay_ledger` 6, `vpay_config` 5, `vpay_worker` 5, both adapters 1 each.

Also: `cargo clippy -p vpay-core -p vpay-db -p vpay-api -p vpay-worker -p
vpay-sdk --all-targets -- -D warnings`, `cargo clippy -p
vpay-tests-integration --test invoices --test postgres_smoke -- -D warnings`
and `cargo clippy -p vpay-sdk --features live-stack --test live_invoices -- -D
warnings` — all clean. `sdks/nodejs`: `tsc -p tsconfig.json --noEmit` and
`eslint . --max-warnings 0` clean. `just fmt` run.

**Not run at the first commit: the two SDK live suites** (they ran in the review round, above). `just sdk-live` sat at `Image
wiremock/wiremock:3.9.2 Pulling` for over thirty minutes against the
restarted daemon (Docker Hub answered a direct `curl` with its usual `401`, so
the network was up; the daemon's pull was not moving) and was stopped. The
two new live cases — `an_invoice_is_marked_paid_out_of_band_and_carries_its_record`
and `records an invoice as paid out of band and reads its record back` —
compile, lint and typecheck, and have never run against a stack. Also not
run: `just ci` as a single recipe; `just clippy` over the whole
workspace (the five changed crates and the two changed test targets were
clipped instead); the rest of `vpay-tests-integration`'s test binaries
(`customers.rs`, `staff_sign_in.rs` and the others this change does not
touch); `just test-storybook`; `just test-e2e`.

## The drift measurement

Derived first, then read off `cratestack migrate baseline --strict` at 0.12.0:
`drift detected in 26 table(s)/view(s) (199 change(s) total)` and 19 unmappable
columns. The new block, verbatim:

```text
manual_payments:
  [safe] CHECK `amount_positive` exists in the live database but is not declared in the schema
  [safe] CHECK `id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `merchant_id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `reference_length` exists in the live database but is not declared in the schema
  [lossy] column `method` type differs (live: Scalar("String"), schema: Enum("ManualPaymentMethod"))
```

`invoices:` carries no new line: `paid_out_of_band` costs nothing, and the
three new CHECKs on it are multi-column. `manual_payments_method_enum_check`
and `manual_payments_invoice_id_key` cost nothing, being born under
CrateStack's generated names.

## Two mutations

1. **Resolve `merchant_clients[].invoices` before the `paid_out_of_band`
   fork** in `pay_once` (the ordering the brief called crucial): seven of the
   ten new `invoices.rs` cases went red, among them
   `paying_out_of_band_needs_no_configured_urls_and_posts_nothing`. Reverted.
2. **Drop the intent the rewritten `the_invoice_invariants_are_enforced_by_the_database_itself`
   fixture attaches** (`… WHERE id = $1 AND false`): red, with
   `violates check constraint "paid_names_how"` on the refunded-`paid` row
   the case asserts is storable. Reverted.

## The two things asked to be double-checked

1. **`paid_names_how` and the existing fixture.** Before the change the case
   wrote a partial-payment `paid` row (expected refused by
   `paid_means_nothing_remaining`) and a refunded `paid` row (expected
   storable) on an invoice with **no** intent. Under `paid_names_how` the
   second would be refused, and the first would be refused by two constraints
   at once. The fixture now attaches the intent the case already creates —
   the settlement's own shape — so each row violates at most the one
   constraint it is about, and both refusals are asserted **by constraint
   name** (`refused_by(..) == Some("paid_means_nothing_remaining")`,
   `Some("refunded_at_most_paid")`). Mutation 2 above is the evidence the
   storable half still bites. One limit, stated: with the attachment removed,
   Postgres happened to report `paid_means_nothing_remaining` rather than
   `paid_names_how` for the partial-payment row, so the name pin alone would
   not have caught that half — the storable row did.
2. **The customer share lock and deadlocks.** The paying transaction's first
   statement is `SELECT … FROM customers … FOR SHARE`; the erasure's first is
   `FOR UPDATE` on the same row (`lock_for_update`, and the sweep's
   `erase_idle`), and `POST /v1/customers/{id}` takes `FOR UPDATE` first too
   and touches no invoice. Read from the statements: every transaction that
   holds a customer lock **and** an invoice or `manual_payments` lock takes
   the customer first; the settlement, `void`, `attach_intent` and the other
   invoice writes take no customer lock; `POST /v1/invoices`' foreign-key
   check takes `FOR KEY SHARE`, which does not conflict with `FOR SHARE`; the
   idempotency store's `OutOfBandInvoice` path takes the same `FOR SHARE` and
   then updates only its own claimed row, which the erasure's rewrite cannot
   match (its `response_body` is `NULL` until the store writes it) and cannot
   reach while the store holds the share lock. One acquisition order, so no
   cycle. The empirical half,
   `an_erasure_racing_an_out_of_band_payment_neither_deadlocks_nor_leaves_the_reference`,
   passed (a `40P01` would have surfaced as a `500`). _(At the first commit it
   did not record which side won, so it proved "no deadlock and no surviving
   reference" only. Since the review round it records the winner of every round
   and fails unless at least one round let the payment write its reference
   first — see "The race case found its own blind spot" above.)_
