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

## 10. Rebased onto `1bd2183` (2026-09-06)

This branch was rebased onto `origin/master` at **`1bd2183`** — the merge of
PR #49 (issue #47, account-holder name lookup) — after that PR landed first.
Fifteen commits replayed; no commit was dropped and the branch's net diff
against master is byte-identical in shape to what it was against `65a5952`
(25 files, 1760 insertions, 31 deletions), which is the check that says the
rebase added and lost nothing.

**Five files conflicted, and every one was resolved by keeping both sides:**

* `backends/crates/vpay-provider/src/measured.rs` and
  `backends/crates/vpay-adapter-mtn-momo/src/lib.rs` — both branches widened
  the same `use vpay_provider::{…}` list. Union: `AccountHolder` (#49) and
  `Refunded` (this branch) are both imported.
* `backends/crates/vpay-api/src/model.rs` — both branches added an
  `object_tag!` invocation at the same insertion point, and git's markers cut
  *through* the two macro calls rather than between them: the conflict shared
  one `object_tag!(` and one `);` between `AccountHolderTag` and `RefundTag`.
  Resolved by reconstructing **both** complete invocations, not by trusting
  the marker boundaries. Six `object_tag!`s now, in the order
  `PaymentIntentTag`, `ListTag`, `EventTag`, `AccountHolderTag`, `RefundTag`,
  `CheckoutSessionTag`.
* `docs/sdks/parity.md` — two independent new paragraphs at the same anchor.
  Both kept: #49's `account_holders` note first, this branch's `refund.fee`
  shape note second. Both row sets (five `account_holders` rows, one
  `refund.fee` row) and both ⛔-index entries survive.
* `docs/flows/merchant-auth.md` — the **only** hunk where "keep both" would
  have duplicated a paragraph rather than merged one: #49 and this branch
  rewrote the *same* "**Served** marks…" sentence. #49's edit was the date
  `2026-09-03` → `2026-09-05`; this branch's rewrite already carries that
  date and adds the provenance (`re-read off vpay_api::v1::V1_ROUTES`) and
  the `/v1/checkout/sessions` caveat, so this branch's paragraph was taken as
  the superset. Nothing from #49 is lost by it. The Resources table itself
  merged cleanly and holds **both** branches' rows — `/v1/events`,
  `/v1/events/{id}` and `/v1/account_holders`.

**One break carried no conflict marker at all,** which is the reason a build
is not optional after a rebase. #49 added
`backends/crates/vpay-api/src/v1/account_holders.rs`, whose `AnsweringRail`
test double implements `ProviderAdapter::refund` returning `Submitted`. This
branch changes that return type to `Refunded`. The two never touched the same
line — the file did not exist on this branch — so git merged them silently and
`cargo build --workspace --all-targets` failed with `E0053`. Fixed in the
commit that changes the port (`feat(provider): refund answers Refunded…`), by
changing the double's signature; its body was and remains
`Err(ProviderError::Unsupported)`, so no behaviour moved.

**This branch still does not touch the refund object's route.** `V1_ROUTES`
is unchanged by it; `/v1/refunds` is still unmounted, and `RefundObject` is
still exactly ten keys (`the_refund_object_is_the_documented_ten_keys`).

### The gate on the rebased tree, measured 2026-09-06

Measured, not recalled, and **the container-backed part of `test-rust` did
not run** — see the caveat below, which is stated here rather than left out
because a partial gate reported as a whole one is worse than a red one.

* `fmt-check`, `clippy --workspace --all-targets -- -D warnings`: clean.
* `verify`: *"ok — the ten gates above passed; the verify-docs report is
  advisory"*.
* `cargo build --workspace --all-targets`: clean.
* `verify-ignored`: *"0 ignored (expected 0), 42 test binaries (expected 42),
  1330 total (minimum 1080)"*. `origin/master` at `1bd2183` was measured on
  the same machine at **1319 total across the same 42 binaries, 0 ignored**,
  so this branch adds **11** Rust tests: `vpay-api` +7, `vpay-core` +1,
  `vpay-sdk::resources` +1, `vpay-tests-integration::postgres_smoke` +2. No
  new test binary, which is why the `expected_suites` recipe variable does
  not move.
* `test-doc`: **96 passed, 0 failed, 1 ignored** (master after #49 was 94).
* `lint-web` (`pnpm -r typecheck` then `pnpm -r lint`): clean.
* `test-web`: every package passes — `sdks/nodejs` **178 passed** across 9
  files, `sdks/stripe-js` 119, `frontends/apps/checkout` 302,
  `examples/shop` 57, `frontends/packages/config` 63, plus the small ones.
* `deny`: *"advisories ok, bans ok, licenses ok, sources ok"*.
* **`test-rust` did NOT complete.** Two full `just ci` runs reached it and
  both died on the host's rootless Docker, not on this branch: the first at
  1019/1330 with *"container is not ready: container startup timeout"*, the
  second at 998/1330 with *"failed to create a container: Timeout error"*.
  The daemon then wedged outright (a zombie `dockerd` with 24 threads stuck
  in `sync_inodes_sb`/`wb_wait_for_completion` behind ~11 GB of dirty pages)
  and was replaced by a recovery daemon with an empty image cache which, at
  the time of writing, cannot pull: `postgres:16-alpine` and
  `wiremock/wiremock:3.9.2` sit at *"Pulling fs layer"* with **0 bytes**
  of host network ingress, while the host itself reaches
  `registry-1.docker.io` fine. Every test that failed in either run failed
  on container startup; **no assertion failed in either run.**
* The named unit cases that do not need a container were run on the rebased
  tree and pass: all seven of `vpay-api`'s refund/fee cases, including
  `the_refund_object_is_the_documented_ten_keys`,
  `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero`,
  `a_reported_fee_never_moves_the_payers_amount` and
  `a_refund_delivered_as_either_refund_event_carries_fee_present_and_null`.
  **`postgres_smoke`'s two new cases and its migration-count assertion (31)
  are exactly what could not be re-run**, so the claim "31 migrations apply
  and `fee_non_negative` fires" rests on the pre-rebase measurement in
  section 7 plus the fact that this rebase changed no migration and no SQL.

### The two decisive mutations, re-run on the rebased tree

Both applied to `TryFrom<&vpay_db::RefundRow> for RefundObject`, run, and
reverted; `git status` clean afterwards.

1. `amount: row.amount` → `amount: row.amount - row.fee.unwrap_or(0)` —
   **`a_reported_fee_never_moves_the_payers_amount` FAILS**, on
   `a fee of Some(250) changed the amount the payer gets back / left: 1750,
   right: 2000`. The other six cases still pass, which is the point: this
   mutation is invisible to every one of them.
2. `fee: row.fee` → `fee: Some(row.fee.unwrap_or(0))` — **the absent-vs-zero
   case FAILS**, on `a rail that reported no fee must render null; 0 would
   tell a merchant the movement was free, which nobody measured / left: 0,
   right: Null`. Three others fall with it (`…the_documented_ten_keys`,
   `the_merchant_sdk_deserialises…`, `a_refund_delivered_as_either_refund_event…`).

### Still true after the rebase, and worth restating

PR #51 adds a **second** refund renderer on top of the same object. Whichever
of #50 and #51 lands second will have to rebase again, and the collision will
be in the same place this one was — `vpay_api::model`'s refund block and the
`refund` rows of `docs/flows/merchant-auth.md` and `docs/sdks/parity.md`.

## 11. Rebased onto `bb8de92` (2026-09-06) — the second rebase, onto PR #51

Section 10 predicted this one: *"PR #51 adds a **second** refund renderer on
top of the same object. Whichever of #50 and #51 lands second will have to
rebase again, and the collision will be in the same place this one was."* It
did, and it was.

`origin/master` is now **`bb8de92`**, the merge of PR #51 (issue #45,
`GET /v1/refunds/{id}`). Sixteen commits replayed onto it; none dropped.

### The decisions this resolution was made under

Taken by the coordinator under [ADR-0016](../../adr/0016-engineering-standards.md)
and not re-opened here:

1. **One `RefundObject`/`RefundTag`, ten keys, `status` typed.** Master's nine
   keys plus this branch's `fee`, with `status: vpay_core::RefundStatus`
   rather than `String`. Master's doc comment argued for a `String` on the
   grounds that an unparseable label should not turn a merchant's read into a
   `500`; that concern is **kept, inverted, and written on the field**:
   `refunds.status` is a Postgres `ENUM` (`refund_status`, migration `0017`),
   a fifth value cannot be written without a migration, so a label that fails
   to parse is a corrupted row rather than a vocabulary this code has not
   caught up with — and `Internal` is then the honest answer.
   `every_stored_refund_status_renders_and_decodes_in_the_merchant_sdk` (kept
   from #51, retargeted) is what stops the two vocabularies drifting.
2. **One `vpay_db::Refunds` repository** — master's, with its `COLUMNS`
   projection and its merchant-scoped read (the join onto `payment_intents`,
   because `refunds` has no `merchant_id`), extended with `r.fee` and one
   `RefundRow` carrying `fee: Option<i64>`. This branch's trait-less
   `RefundRow` is gone; there is one row type and no duplicate.
3. **Master's `v1/refunds.rs` renders through that single ten-key renderer.**
   The route is untouched by this branch.
4. **#51's integration suite moved 9 → 10 keys** and now asserts `fee` is
   *present and `null`* for a seeded row that has none.
5. **Both refund event types** are covered on both surfaces: the unit case
   `a_refund_delivered_as_either_refund_event_carries_fee_present_and_null`
   and, in `backends/tests/integration/tests/refunds.rs`,
   `the_api_response_and_an_events_payload_for_one_refund_are_byte_identical`,
   which now loops `charge.refunded` **and** `charge.refund.updated`.
6. **Every count both PRs measured was re-measured**, not carried — see the
   gate below.

### Six files conflicted

* **`backends/crates/vpay-api/src/model.rs`** — the substantive one. Five
  marked hunks, and **three duplicate definitions git produced with no marker
  at all** because the two branches wrote them at different offsets:
  two `RefundObject` structs (this branch's above `ListObject`, #51's beside
  its `TryFrom`), two `refund_row` test fixtures with different signatures
  (`fn refund_row(id: &str)` vs `fn refund_row(fee: Option<i64>)`), and two
  `metadata_of` functions with different parameter orders. Resolved to one of
  each: #51's `metadata_of(value, table)` signature, one struct carrying #51's
  "one renderer, two surfaces" argument plus this branch's `fee`
  documentation, and one fixture. #51's five refund cases were kept and
  retargeted onto that fixture rather than deleted — its whole-value nine-key
  case is subsumed by `the_refund_object_is_the_documented_ten_keys`, its
  `reason` case now asserts ten keys and both the `null` and the supplied
  spelling, and its status-vocabulary case now proves the parse as well as the
  SDK decode. Its `RefundTag` assertion lives where it landed, inside
  `the_object_discriminator_cannot_be_anything_else`, so this branch's
  standalone duplicate of it was dropped.
* **`backends/crates/vpay-db/src/refunds.rs`** (add/add — both branches
  created the file) — resolved to #51's module: repository, `COLUMNS`,
  merchant-scoped read, `sqlx::FromRow` row. `r.fee` joins the projection and
  `fee: Option<i64>` the row; the module header names `0031` alongside `0017`.
  Nothing else of this branch's version survives, and nothing of it needed to.
* **`backends/crates/vpay-db/src/lib.rs`** — both branches wrote the `pub mod
  refunds;` comment and the re-export. #51's kept in both hunks
  (`pub use refunds::{RefundRow, Refunds};`).
* **`docs/api/README.md`** — the `POST /v1/refunds` "not served" row. Merged:
  #51's reason (no rail can refund; the repository has one read and no write)
  plus the fact that `GET /v1/refunds/{id}` **is** served and renders ten keys.
* **`docs/flows/merchant-auth.md`** — twice. The "**Served** marks…" paragraph
  and the events rows (#51 had already fixed the same `⛔ 404` staleness this
  branch's commit `f07dcd0` fixed; #51's header paragraph plus this branch's
  richer per-row test citations were taken), and the "No `/v1/refunds`…"
  bullet, where #51's correction is kept and the `fee` sentence appended.
* **`docs/sdks/parity.md`** — twice: the provenance sentence (both dates kept)
  and the row block, where #51's `refunds.retrieve` row and this branch's
  `refund.fee` row are both present.

Two more files needed edits with no conflict, because a claim they made had
become false rather than contested:

* **`backends/migrations/0031_refunds-fee.sql`** — its header and its
  `COMMENT ON COLUMN` said "no refunds repository, no `/v1/refunds` route" and
  "NOT WRITTEN OR READ BY ANY CODE". Both were true when written and are not
  now: the column **is** read and rendered. Rewritten to say what is still
  true — nothing *writes* it, so every stored value is `NULL`.
* **`docs/status.md`** — the "Also missing, and larger" paragraph said nothing
  reads a `refunds` row at all. Corrected to name the write side as the half
  that is still missing.

### One break carried no conflict marker, again

Reconstructing the two whole SDK test functions that git had fragmented in
`sdks/rust/tests/resources.rs` (this branch's `a_refund_fee_decodes_as_…` and
#51's `retrieve_refund_is_a_get_…`, interleaved across two marker regions)
left **one stray `}`** behind. It is invisible to `git status`, to a
marker grep and to every doc gate; `cargo build --workspace --all-targets`
caught it (`unexpected closing delimiter`). The first build after a rebase is
not optional, and neither is reading its exit code rather than a pipeline's.

### The gate on the rebased tree, measured 2026-09-06

* `cargo build --workspace --all-targets`: **clean** (`Finished dev profile`).
* `just fmt-check`: clean (one rustfmt nit in the merged test module, fixed).
* `just clippy` (`--workspace --all-targets -- -D warnings`): clean.
* `just verify`: *"ok — the ten gates above passed; the verify-docs report is
  advisory"* — including `verify-sdk-parity: ok — 359 proving test(s) named in
  docs/sdks/parity.md all exist, 28 dated gap(s)` and `verify-links: ok — 765
  repository link(s) in 137 tracked markdown file(s)`.
* `cargo nextest run -p vpay-api -p vpay-core -p vpay-sdk`: **475 tests run,
  475 passed, 0 skipped.** The named refund/fee cases all pass — filtered to
  `test(refund) or test(fee)`, **18 run, 18 passed**, including the ten-key
  tripwire, the absent-vs-null-vs-zero case, `a_reported_fee_never_moves_the_
  payers_amount` and the both-events case.
* `just test-doc`: **96 passed, 0 failed, 1 ignored** (`vpay_core` 47, which
  includes `RefundStatus`'s two doctests).
* `just verify-ignored`: *"0 ignored (expected 0), 43 test binaries (expected
  43), 1342 total (minimum 1080)"*. No new test binary, so `expected_suites`
  stays 43.
* `just lint-web`: clean (`pnpm -r typecheck`, `pnpm -r lint`; Node 22.23.2
  per `.nvmrc`).
* `just test-web`: every package passes — `sdks/nodejs` **180 across 9 files**,
  `sdks/stripe-js` 119, `frontends/apps/checkout` 302, `examples/shop` 57,
  `frontends/packages/config` 63, plus the small ones.
* `just deny`: advisories, bans, licenses and sources ok.
* `cargo nextest run -p vpay-sdk`: **141 tests run, 141 passed, 0 skipped** —
  the number `docs/status.md`'s Rust-SDK row now carries, re-measured rather
  than carried from #45's 136.

### What did NOT run, and is owed to CI

**`just test-rust`, attempted once and stopped on the host, not on this
branch.** The rootless Docker daemon on this machine is mid-repair: there is
no socket at `/run/user/1000/docker.sock` at all, and `docker info` fails
before any test runs. The single attempt reached
*"672/1342 tests run: 671 passed, 1 failed"*, and the one failure is
`vpay-db::postgres an_abandoned_transaction_survives_a_rollback_it_cannot_send`
with *"failed to create a container: Error in the hyper legacy client (Connect)"*
— nextest then stopped, leaving 670 unrun. **No assertion failed.**

Owed to CI, therefore, and named so nobody has to guess:

* `backends/tests/integration/tests/refunds.rs` — including this rebase's
  edits: the ten-key assertion, `fee` present-and-null, and the both-event-type
  loop in `the_api_response_and_an_events_payload_for_one_refund_are_byte_identical`;
* `backends/tests/integration/tests/postgres_smoke.rs` — the migration count
  (31), `an_unreported_refund_fee_stays_null_and_never_becomes_zero` and
  `a_negative_refund_fee_is_rejected_by_the_database`;
* every other container-backed suite in `backends/tests/integration`,
  `backends/tests/conformance` and `vpay-db`'s `postgres` binary.

### The two decisive mutations, re-run on the rebased tree

Applied to `TryFrom<&vpay_db::RefundRow> for RefundObject`, run, reverted;
`git status` shows `model.rs` unmodified afterwards.

1. `amount: row.amount` → `amount: row.amount - row.fee.unwrap_or(0)` —
   **`a_reported_fee_never_moves_the_payers_amount` FAILS**: *"a fee of
   Some(250) changed the amount the payer gets back — left: 1750, right:
   2000"*. Ten of the eleven filtered cases still pass, which is the point.
2. `fee: row.fee` → `fee: Some(row.fee.unwrap_or(0))` — **`an_unreported_
   refund_fee_renders_null_and_a_reported_zero_renders_zero` FAILS**: *"left:
   0, right: Null"*. Three others fall with it
   (`the_refund_object_is_the_documented_ten_keys`,
   `the_merchant_sdk_deserialises_the_refund_this_renders`,
   `a_refund_delivered_as_either_refund_event_carries_fee_present_and_null`).

### One dated record deliberately left alone

[review.md](review.md) §1 records, as measured on 2026-09-05, that
`V1_ROUTES` had nine entries and no refunds route. That was true of the tree
it was measured on and is false of this one — #51 added the tenth. It is a
dated measurement in a review record, not a live claim, and rewriting it
would be rewriting history; this paragraph is the correction. Re-measured on
this tree, `V1_ROUTES` has **eleven** entries — #47's `/account_holders` and
#51's `/refunds/{id}` are the two the review's nine did not have.
