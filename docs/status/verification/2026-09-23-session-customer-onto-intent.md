# 2026-09-23 — A session's customer is written onto a customer-less intent (ADR-0025)

Branch `feat/session-customer-onto-intent`, cut from `origin/master` at
`b747e5d5`. It answers ADR-0024's question 3, as the maintainer directed on
2026-09-23. [ADR-0025](../../adr/0025-session-customer-onto-intent.md) is the
decision. This page records what was run, on this branch, before it was
committed.

## Environment

macOS (darwin 25.5), `rustc 1.98.0`, `cargo-nextest 0.9.146`, pnpm 11.18.0,
Docker 29.1.3. Every cargo invocation ran with `CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=line-tables-only
CARGO_PROFILE_TEST_DEBUG=line-tables-only`. Those settings change debuginfo
and incremental caches only, not behaviour, and **CI does not set them**.
Other agents were running tests on the same host at the same time.

## What the old code allowed

Read on `b747e5d5`: `vpay_api::v1::checkout_sessions::prepare_create`
resolves a sent `customer` and refuses it only when the intent **has** a
different one. `vpay_db::CheckoutSessions::create` was a single `INSERT` into
`checkout_sessions`. So a session naming `X` on a customer-less intent left
the intent with no customer. Once that session expired, a second session
naming `Y` was a `201`, because the partial unique index admits one _open_
session per intent and `Y` was compared with the intent's `NULL`. The
reviewer's belief is **confirmed**, and it needs no concurrency: two
sequential sessions do it.

## What changed

- `CheckoutSessions::create` is a transaction. When `customer_id` is set it
  first runs `claim_intent_customer`:
  `UPDATE payment_intents SET customer_id = $3, updated_at = now() WHERE id = $1 AND merchant_id = $2 AND customer_id IS NULL`.
  If that matches nothing, it re-reads the intent: the same customer
  proceeds, and a different one is `DbError::IntentCustomerConflict`.
- `vpay_api` maps that error to `customer_contradiction`, the function the
  pre-check's `400` now also calls, so the two refusals are one sentence.
- No migration, no route, no event type, no gate, and no `NotImplemented`
  token moved. No wire shape changed.

## The cases, and what each proves

All run against a real Postgres (testcontainers) and the shipping router.
Where a case needs a payment, it also uses the shipping worker loop and a
WireMock MTN rail.

| Case                                                                                | Suite                  | Proves                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ----------------------------------------------------------------------------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a_session_naming_a_customer_writes_it_onto_a_customer_less_intent`                 | `checkout_sessions.rs` | The intent renders `customer: X` after the create. Both rows carry the **same `xmin`**, so they were written by one transaction. `GET /v1/payment_intents?customer=X` and `GET /v1/checkout/sessions?customer=X` find them. A second session naming `Y` after an expiry is the `400`, with the exact sentence. A session naming nobody inherits `X`. After a real payment and a seeded refund, `GET /v1/refunds?customer=X` finds the refund and `?customer=Y` does not.                            |
| `two_sessions_naming_two_customers_for_one_intent_make_one_session_and_one_refusal` | `checkout_sessions.rs` | A test transaction holds the intent `FOR UPDATE` until `pg_stat_activity` shows **both** creates waiting on the `UPDATE`, then commits. Result: `[201, 400]`, the intent names the winner's customer, and there is one session. The loser's body equals, as JSON, the one a sequential create gets after the winner's session is expired. Five unforced rounds follow: exactly one `201` each time (the other a `400` or the pre-check's `409`), and the intent always names the winner's customer. |
| `a_session_naming_another_merchants_customer_writes_nothing_onto_the_intent`        | `checkout_sessions.rs` | Merchant B's real `cus_…` and one that never existed give identical `400`s naming `customer`. The intent keeps no customer, and there is no session.                                                                                                                                                                                                                                                                                                                                                |
| `paying_an_invoice_writes_nothing_onto_its_intent_through_the_session`              | `invoices.rs`          | After `POST /v1/invoices/{id}/pay`, the intent and session name the invoice's customer. The intent's `xmin` **differs** from the session's, so the session transaction did not write the intent. Both list filters find the payment.                                                                                                                                                                                                                                                                |

Two existing cases relied on the old shape and now stage it in SQL, because
the API can no longer produce it:

- `the_customer_filter_reads_the_sessions_own_customer_and_is_not_an_oracle`
  (`checkout_sessions.rs`) sets `s2`'s intent back to `customer_id = NULL`.
  `s2` is still the row that proves sessions filter on their own column,
  which is now a statement about pre-ADR-0025 rows.
- `a_sessions_customer_is_inherited_supplied_or_a_refused_contradiction`
  (`customers.rs`) first asserts that the session wrote `Y` onto the intent.
  It then resets it, because case 5 needs a customer that only a session
  references.

## Mutations

Each was applied to `backends/crates/vpay-db/src/checkout_sessions.rs`, run,
and reverted. The `git diff` afterwards showed only the intended change.

1. **Drop `AND customer_id IS NULL`** from the compare-and-swap. The forced
   race went red: `left: [201, 409]`, `right: [201, 400]`. The loser's
   `UPDATE` overwrote the winner's customer, reached the insert, and was
   refused by `checkout_sessions_one_open_per_intent`. The invoice case also
   went red, because the session's transaction now rewrote the intent (both
   `xmin`s were `"847"`).
2. **Skip `claim_intent_customer` entirely.** Three cases went red:
   `a_session_naming_a_customer_writes_it_onto_a_customer_less_intent`
   (`left: None`), the forced race, and `customers.rs`'
   `a_sessions_customer_is_inherited_supplied_or_a_refused_contradiction`.

## Suites, passed and ignored

| Run                                                                                                                                          | Passed | Failed | Ignored / skipped |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------ | ----------------- |
| `vpay-tests-integration` `checkout_sessions` (in full, twice — before and after the clippy fix)                                              | 35     | 0      | 0                 |
| `vpay-tests-integration` `payment_intents`                                                                                                   | 26     | 0      | 0                 |
| `vpay-tests-integration` `refunds`                                                                                                           | 20     | 0      | 0                 |
| `vpay-tests-integration` `invoices`                                                                                                          | 30     | 0      | 0                 |
| `vpay-tests-integration` `customers`                                                                                                         | 24     | 0      | 0                 |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-sdk`: 247 + 403 + 186 (`sdks/rust`), including `vpay-db`'s container-backed `repositories` | 836    | 0      | 0                 |
| `just test-doc` (`cargo test --doc --workspace`)                                                                                             | 124    | 0      | 1                 |
| `just test-sdk-node` (`sdks/nodejs`, vitest)                                                                                                 | 229    | 0      | 0                 |

The five integration suites ran together as one nextest invocation: 135
tests, 135 passed, 0 skipped. The one ignored doctest was already on
`master`; this change adds no doctest and no ` ```ignore ` fence.

## Gates

`just verify` exited 0 and printed `verify: ok — the fifteen gates above
passed`. Each gate's own line, on the final run:

```text
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 20 error type(s), all classified; 17 `#[from]` variant(s) delegate every `Classify` method they match on; anyhow confined to binaries
verify-sdk-parity: ok — 767 proving test(s) named in docs/sdks/parity.md all exist, 45 dated gap(s), 35 SDK method(s) enumerated across 40 row(s)
verify-links: ok — 2037 repository link(s) in 427 tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.12.0
verify-serde: ok — 104 serialisable type(s) spell the workspace's wire convention, 17 exempted with a reason in docs/adr/0016-engineering-standards.md
verify-toolchain: ok — rust-toolchain.toml pins 1.98.0 and all 1 `FROM rust:` instruction(s) in backends/Dockerfile name it (rust:1.98.0-alpine3.22)
verify-migrations: ok — 49 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256
verify-versions: ok — 24 version references all say 0.5.0
verify-privacy-inventory: ok — 307 database column(s) classified in schemas/privacy-inventory.yaml across 26 elements (17 personal-data, 17 necessary) [...]
verify-doc-counts: ok — 13 documented count(s) in 11 of 275 markdown file(s) agree with what this tree measures
```

`verify-npm-scope` and `verify-repositories` also passed; their lines are omitted here for width. `verify-ui` prints nothing when it passes, and the recipe went on to the next gate. The first `just verify` run failed
`verify-doc-counts` exactly as designed. `docs/status/README.md` said the
verification directory held 64 pages, and this page made it 65. The count and
the hand-maintained list beside it were corrected, and the gate then passed.
**A migration count, schema, privacy inventory or toolchain line did not
move**, because none of them was touched.

`just fmt` (`cargo fmt --all` and `prettier --write .`) and `just clippy`
(`cargo clippy --workspace --all-targets -- -D warnings`) were clean. The
first clippy run failed on three `indexing_slicing` errors in the new tenancy
case, which were fixed by destructuring into an array, as `customers.rs`
already does.

## What was not run

- **`just ci` as one recipe was not run.** Its parts listed above were.
  `just test-rust` over the whole workspace was not: only the five affected
  integration suites and the three crates' own suites were run.
- `just test-storybook`, `just test-e2e` and the Tauri recipes were not run.
  No checkout screen, browser flow or Tauri file changed.
- No live-rail or live-SDK suite was run.
- The **vpay-skills companion PR was not opened.** It is left to the
  coordinator, as briefed.
