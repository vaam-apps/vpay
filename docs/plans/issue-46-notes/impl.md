# Issue #46 — an optional `fee` on the `refund` object

Implementation notes. The decision the issue asked for was taken by the
maintainer (the field exists, nullable, absent-not-zero when unknown); this
records what was built, what was *established* rather than assumed, and what
was deliberately not done.

## 1. The route truth, established rather than repeated

Issue #46 says `/v1/refunds` "is not routed at all". The brief this work came
from set that against a claim it attributed to a *pull request* numbered 45.

**That attribution does not resolve, and is recorded rather than quietly
dropped.** `cargo xtask verify-citations`, run on 2026-09-05 with a token,
answers HTTP 404 for a pull request 45 in `vaam-apps/vpay`; there is none.
Issue #45 does exist and is open, and says something else entirely —
*"SDK/API: a refund cannot be polled — `RefundsResource` has only `create()`,
and `GET /v1/refunds/{id}` is not in the wire contract"*. So the brief's
source for "the SDKs have `refunds.create`" is not a document that exists. The
statement itself is true; it was established by reading the SDKs, below, and
not by trusting either citation.

Read from the source of each claim, on this branch, at base `65a5952`:

| Claim | Source read | Verdict |
|---|---|---|
| `/v1/refunds` is routed | `vpay_api::v1::V1_ROUTES` (`backends/crates/vpay-api/src/v1/mod.rs`) — nine entries: `/payment_intents` ×4, `/events` ×2, `/checkout/sessions` ×3 | **False.** No refunds entry. An authenticated call reaches the `/v1` nest's 404 fallback |
| Both SDKs have `refunds.create` | `sdks/rust/src/resources.rs` (`RefundsResource::create` → `POST /refunds`), `sdks/nodejs/src/resources/refunds.ts` | **True** |

Both statements are therefore true at once, and `docs/sdks/parity.md` already
said so at the bottom of the file: "`/v1/refunds` and `/v1/balance` are not
mounted at all, so the SDK methods for them reach the nest's 404 — a server
gap, tracked in `docs/status.md`, not a parity gap."

Two further facts that shaped the work, and that neither the issue nor the
brief could assume:

* **There was no `refund` object anywhere in the server.** Not unrouted —
  absent. `vpay-api::model` had `PaymentIntentObject`, `CheckoutSessionObject`,
  `EventObject`, `ListObject` and no refund. So "add `fee` to the refund
  object" meant writing the object.
* **Nothing writes a refund event either.** `charge.refunded` and
  `charge.refund.updated` are in the `type_is_a_documented_event` vocabulary
  (migrations `0018`/`0029`) and no code path emits either. The events
  renderer passes `events.data` through verbatim, so the refund payload is
  whatever wrote the snapshot — and nothing does.

## 2. The migration

**`0031_refunds-fee.sql`.** One nullable `BIGINT` on `refunds`, **no
`DEFAULT`**, plus `CONSTRAINT fee_non_negative CHECK (fee IS NULL OR fee >= 0)`
and a `COMMENT ON COLUMN`.

* No `DEFAULT 0`: the whole field is the difference between "unknown" and
  "free", and a default erases it at the layer furthest from anyone who would
  notice.
* Non-negative rather than positive, unlike `amount_positive` on the same
  table: a zero-amount refund is a caller mistake, a zero *fee* is a real
  answer a rail can give.
* No second currency column — `docs/flows/money.md` allows one currency per
  object, so the fee is minor units of `refunds.currency_code` or nothing.

`postgres_smoke.rs`'s migration-count assertion moved 30 → 31 in the same
commit. Two new cases there prove the column rather than the SQL parsing:
`an_unreported_refund_fee_stays_null_and_never_becomes_zero` (four rows —
`NULL`, `0`, `250`, and one whose `INSERT` does not mention the column at all
— read back distinct; the fourth exists because of mutation 5 below) and
`a_negative_refund_fee_is_rejected_by_the_database` (asserts the constraint
*name*, so a different CHECK firing would not pass for it).

**The CrateStack drift constant did not move, and that is not an oversight.**
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` pins 86.
Reading the report on this branch: `refunds` appears as a single change,
"table `refunds` exists in the live database but is not declared in the
schema", so a column added to an entirely undeclared table changes no count.
Measured 2026-09-05 by running the test and reading its stdout, not inferred.

## 3. The port

`ProviderAdapter::refund` returned `Result<Submitted, ProviderError>`.
`Submitted` carries `redirect_url`, which a refund can never have — no payer's
browser is involved in giving money back — so every adapter answered `None` to
a question nobody could ask.

Split into a new `vpay_provider::Refunded { ref_extra, fee: Option<Money> }`.
That is what lets `fee` exist on the refund path *without* also appearing on
the charge path, where a rail's charge fee is a different number with a
different owner. `Money`, not `i64`, so the currency travels with the amount.

The MTN adapter's `refund` keeps its `ProviderError::NotImplemented("mtn_momo::refund")`
untouched; Orange still inherits the trait default `Unsupported`. **No adapter
fabricates a fee**, and the conformance suite is unchanged because it asserts
error variants, not payloads.

## 4. The wire object

`vpay_api::model::RefundObject`, ten keys, `#[serde(rename_all = "snake_case")]`,
built by `TryFrom<&vpay_db::RefundRow>`.

**Why a row struct at all**, given there is no refunds repository: the mutation
that matters is `fee: row.fee` → `fee: Some(row.fee.unwrap_or(0))`, and a
renderer fed by literals in a test can never fail it. So `vpay-db` gained
`refunds.rs` holding `RefundRow` and **nothing else** — no trait, no `COLUMNS`,
no SQL, not even `#[derive(sqlx::FromRow)]`, because deriving it would read as
evidence that something `SELECT`s the table. It carries the nine columns the
wire object needs and not the table's other five, following this crate's own
rule (written on `EventRow::fanout_state`) that a row mirrors its callers
rather than its table.

`vpay_core::RefundStatus` was added alongside `IntentStatus` and `ChargeState`,
with the same `as_wire_str`/`from_wire` pair, so a `refund_status` label
outside the four is `ApiError::Internal` and never a status invented to make a
conversion total.

`metadata_of` gained a `column` parameter: its message hardcoded
`payment_intents.metadata`, which would have sent an operator to the wrong
CHECK for a refund.

## 5. The SDKs (ADR-0015 decision 2, same PR)

| | Rust | TypeScript |
|---|---|---|
| Shape | `pub fee: Option<i64>` + `#[serde(default)]` | `fee?: number \| null` |
| absent | `None` | `undefined` |
| `null` | `None` | `null` |
| `0` | `Some(0)` | `0` |

**The asymmetry is deliberate and documented in `docs/sdks/parity.md`.** Rust
collapses absent and `null`; TypeScript keeps them apart.

> **Corrected on review, 2026-09-05 (F3 in [review.md](review.md)).** This
> paragraph originally credited the TypeScript half to "this package's
> `exactOptionalPropertyTypes`". Measured with the workspace's own `tsc`, the
> read type of `fee?: number | null` is `number | null | undefined` with that
> flag and without it — the `?` is what keeps absent and `null` apart. The
> flag's contribution is only that `{ fee: undefined }` stops compiling. Both preserve the distinction
the field exists for — unknown versus a measured zero. `Option<Option<i64>>`
in Rust would push a state with no producer (vpay emits every documented key
on every object) into every match a merchant writes; dropping `?` in
TypeScript would make an older vpay's response fail to type-check for no gain.

Parity row: *The `refund` object's `fee`, and the absent / `null` / `0`
distinction it exists for*, naming two tests per column.

## 6. Mutations run

Each was applied on this branch, the command run, then reverted. **Observed**
is what the run actually printed, not what was expected — two of the six did
not fail where the brief predicted they would, and both are recorded as such.

| # | Mutation | Command | Observed |
|---|---|---|---|
| 1 | Delete `pub fee` from `RefundObject` and its assignment | `cargo nextest run -p vpay-api -E 'test(fee) + test(refund)'` | **4 failed**, 2 passed: `the_refund_object_is_the_documented_ten_keys` (9 keys ≠ 10), `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero`, `a_refund_delivered_as_charge_refunded_carries_fee_present_and_null`, `the_merchant_sdk_deserialises_the_refund_this_renders` |
| 2 | `fee: row.fee` → `fee: Some(row.fee.unwrap_or(0))` | same | **4 failed**. The decisive line: `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero` — `left: Number(0)`, `right: Null` |
| 3 | Delete `pub fee` from `vpay_sdk::Refund` | `cargo xtask verify-sdk-parity`, then `cargo nextest run -p vpay-sdk` | **The parity gate PASSED** (see below). The test binary fails to compile: `error[E0609]: no field 'fee' on type 'Refund'` ×2, so `cargo nextest run -p vpay-sdk` exits 101 |
| 4 | Delete `fee?:` from `sdks/nodejs`'s `Refund` | `pnpm --filter @vaam-apps/vpay-sdk lint`, `… typecheck`, `… test` | `lint` **passed**; `test` (vitest) **passed, 173/173**; `typecheck` **failed** with three `TS2339: Property 'fee' does not exist on type 'Refund'`. `just ci` catches it through `lint-web`, which runs `pnpm -r typecheck` — *not* through `test-web` |
| 5 | `ADD COLUMN fee BIGINT DEFAULT 0` in `0031` | `cargo nextest run -p vpay-tests-integration -E 'test(refund_fee)'` | **Passed 2/2 as first written** — the test bound `NULL` explicitly on every insert, so the default was never reached. A fourth insert that does not mention the column at all was added because of this run; with it, the mutation fails: `left: Some(0)`, `right: None` |
| 6 | Delete `CONSTRAINT fee_non_negative` from `0031` | same | **1 failed**: `a_negative_refund_fee_is_rejected_by_the_database` panics on its `expect_err` — the `-1` insert succeeds |

**Two corrections to the brief's predicted proof, stated plainly:**

* *"Remove the SDK field → the parity gate … FAIL."* It does not.
  `verify-sdk-parity` checks that every test **named** in `docs/sdks/parity.md`
  **exists** as a live `#[test]`/`it(…)`; it does not compile or run anything.
  Removing the field leaves the test functions in place, so the gate is green
  and the *compiler* is what fails. That is still a `just ci` failure, but it
  is a different gate from the one the brief named, and worth knowing before
  someone relies on the parity matrix to catch a field deletion.
* *"…and the SDK decode test FAIL."* True in Rust (a compile error). In
  TypeScript the runtime test passes: types are erased, `result.fee` is
  whatever the JSON carried, and `expect(result.fee).toBe(0)` still holds.
  The Node column's protection for this field is `tsc`, reached by
  `just lint-web`, not vitest. Mutation 4 is the measurement that says so.

## 7. The gate, measured

`just ci` end to end on this branch, **exit 0**, twice — once at `78c1d2d` and
again at `0ccd785` after the citation correction. From the second run's log:

* `fmt-check`, `clippy --workspace --all-targets -- -D warnings`: clean.
* `verify`: *"ok — the ten gates above passed"*.
* `test-rust`: **1289 tests run, 1289 passed, 0 skipped** in 793 s, against
  real Postgres and WireMock containers (testcontainers).
* `verify-ignored`: *"0 ignored (expected 0), 41 test binaries (expected 41),
  1289 total"*. Base `65a5952` was 1279 across the same 41 binaries, so this
  branch adds **10** Rust tests: 2 in `postgres_smoke`, 1 in `vpay-core`, 6 in
  `vpay-api::model`, 1 in `vpay-sdk`.
* `test-doc`: every crate ok; `vpay-core` 46 passed, which includes the two
  new `RefundStatus` examples.
* `lint-web` (`pnpm -r typecheck` then `pnpm -r lint`): clean. `test-web`:
  `sdks/nodejs` **173 passed**, including the new case; every other package
  unchanged.
* `deny`: *"advisories ok, bans ok, licenses ok, sources ok"*.

Separately, and **not** part of `just ci` because it needs the network:
`cargo xtask verify-citations` with a token — *"ok — 45 unique id(s) cited by
131 markdown file(s) all resolve"*. It failed on the first run; see section 1.

## 8. Out of scope, and said so

* **`fee_borne_by` and `fee_settlement_ref`.** Marketplace concepts. Fault
  attribution is the platform's judgement, not the rail's; vpay reports what
  the movement cost. The issue proposed leaving them out and this agrees.
* **Any rail call.** `mtn_momo::refund` stays `NotImplemented`.
* **Populating the fee.** Nothing can. See `docs/status.md` for what must
  exist first.
* **A refunds repository, a `/v1/refunds` route, a refund event writer.** None
  of these was built, and `docs/status.md` says so in both directions.
* **A ledger posting for the fee.** `docs/flows/ledger.md` now states the
  decision — reported only, not posted — and names invariant 2 as the line
  that has to change before anything posts one.

## 9. What a reviewer should be sceptical about

* This PR adds **more server surface than the issue proposed** (the issue's
  steps 1–4 were docs and SDKs only). `RefundObject`, `RefundRow` and
  `RefundStatus` are new public types that nothing outside a test constructs.
  The argument for them is section 4's mutation; the argument against is that
  they are the read half of a refund path that does not exist. Trimming them
  to docs-and-SDKs-only is a coherent alternative and would cost the two
  server-side mutation proofs.
* `RefundRow` in `vpay-db` is a row struct with no queries — an unusual shape
  for that crate, deliberately marked as such in its module header.
* Nothing here has ever spoken to MTN or Orange. Whether either rail reports a
  refund fee at all remains unverified, exactly as the issue said.
