# 2026-09-23 — The `customer` list filter (RFC-0004 § 5, first bullet)

Branch `feat/rfc-0004-customer-filters`, cut from `origin/master` at
`d98fdaf`. **`just ci` was not run on this branch**, and the full integration
suites were not either — see "What was not run" before trusting anything
below as a whole-suite result.

## What changed

`customer=cus_…` on `GET /v1/payment_intents`, `GET /v1/checkout/sessions` and
`GET /v1/refunds`, in the server and in both merchant SDKs. `GET /v1/customers`
is unchanged and stays unfiltered, for the reason its `ListParams` doc gives.
`/v1/subscriptions` does not exist, so its filter was not built.

| List                    | Compared column                                                      | Why that column                                                                                                                                                                                                               |
| ----------------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/v1/payment_intents`   | `payment_intents.customer_id`                                        | The intent's own column (migration `0034`).                                                                                                                                                                                   |
| `/v1/checkout/sessions` | `checkout_sessions.customer_id`, the **session's own**               | It exists (0034), and it differs from the intent's for a session created with `customer=` on an intent that has none. It is also the `customer` the list renders. Going through the intent would drop exactly those sessions. |
| `/v1/refunds`           | `payment_intents.customer_id` of the joined intent (`p.customer_id`) | `refunds` has no customer column (0017; nothing since adds one), and the list already joins the intent for the tenant predicate.                                                                                              |

The rules shared by all three:

- **Shape only.** `vpay_api::v1::customers::filter_param` trims the value,
  treats blank as absent, and checks it with
  `vpay_core::ids::is_well_formed(CUSTOMER_PREFIX, …)`. It never looks the id
  up.
- **Malformed value.** `400` naming `customer`, with the message
  `` `customer` must be a Customer id — `cus_` followed by 24 characters. ``
  That is the sentence `GET /v1/invoices` and `resolve_for_attachment` already
  used. It is now one private function (`malformed_customer`), shared by
  `resolve_for_attachment` and `filter_param`. `invoices.rs` still spells its
  own copy; it was deliberately left untouched on this branch. Each
  integration case compares the two responses byte for byte.
- **Another merchant's real `cus_…`, or an unknown one.** `200` and an empty
  page, byte-identical. The predicate is `$n IS NULL OR <col> = $n`, in the
  same `WHERE` as `merchant_id`.
- **Cursors.** Unchanged, and deliberately not narrowed by the filter. A
  cursor resolves to its position in the merchant's whole list. That is what
  `vpay_db::invoices::list_page` does for its own `customer` filter; it was
  found by reading that code, not chosen. So a cursor naming one of the
  merchant's own rows _outside_ the filter pages from that row.
- **Erased customer (0041).** The id survives anonymisation and every
  reference to it stays, so the filter still finds them. Asserted for intents
  and refunds. Sessions share the mechanism but have no separate assertion.

No migration, no new route, no gate, no `NotImplemented` token moved.

## Local build environment

Every local cargo/just run after the disk incident below used
`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=line-tables-only
CARGO_PROFILE_TEST_DEBUG=line-tables-only`, because the host was at ~15 GiB
free with a second agent building in parallel. They change debuginfo size and
incremental caching only. **CI does not set them.**

Partway through, the host disk filled (`ENOSPC`) during a
`cargo nextest run -p vpay-api -p vpay-db`. The worktree's `target/` (11 GiB)
was deleted by the coordinator, and every run from "After the disk incident"
onward is a cold build. Docker did not recover from the full disk: a single
`docker ps` hung for over seven minutes and was killed, and `docker system df`
from other sessions hung for over twelve. **So no testcontainers-backed test
ran after the incident.**

## What ran, and what it printed

### Before the disk incident (same source tree, full debuginfo)

- `just verify`, **before** any change: exit 0,
  `verify: ok — the fifteen gates above passed`.
- `cargo nextest run -p vpay-tests-integration -E 'test(customer_filter)'`:
  **3 tests run: 3 passed, 329 skipped** (skipped = filtered out by the
  expression, not `#[ignore]`d). The three are
  `payment_intents::the_customer_filter_pages_inside_its_set_and_is_not_an_oracle`,
  `checkout_sessions::the_customer_filter_reads_the_sessions_own_customer_and_is_not_an_oracle`
  and
  `refunds::the_customer_filter_goes_through_the_intent_and_is_not_an_oracle`,
  against a testcontainers Postgres.
- **Mutations, all three red at once, then reverted** (the revert was checked
  with a grep for the three predicates):
  - intents: `customer_id = $8` → `TRUE`. Red: the list returned all seven
    intents, four expected.
  - sessions: `customer_id = $5` → a sub-select of the intents whose
    `customer_id = $5`, i.e. through the intent. Red: `s2` went missing,
    which is the case that decides the column.
  - refunds: `p.customer_id = $5` → `TRUE`. Red: five returned, three
    expected.
- `cargo nextest run -p vpay-sdk`: **181 tests run: 181 passed, 0 skipped.**
- `sdks/nodejs`: `pnpm run typecheck` clean; `pnpm test` **225 passed
  (9 files), 0 skipped**; `pnpm run lint` clean.

### After the disk incident (cold build, the flags above)

- `just fmt`: clean.
- `just clippy` (`cargo clippy --workspace --all-targets -- -D warnings`):
  exit 0, no warnings.
- `cargo nextest run -p vpay-api`: **398 tests run: 398 passed, 0 skipped**,
  including
  `v1::customers::tests::the_customer_filter_is_checked_for_shape_and_answers_the_one_sentence`.
- `just test-doc`: exit 0, **121 passed, 0 failed, 1 ignored**, summed over
  every doctest binary.
- `cargo xtask verify-sdk-parity`: **756 proving test(s), 45 dated gap(s),
  35 SDK method(s) across 39 row(s)**.
- `just verify`, after all changes (new page staged, since `verify-links`
  counts tracked paths only): exit 0, `verify: ok — the fifteen gates above
passed`. The first attempt failed `verify-links` on six links to this page,
  because it was still untracked. `verify-doc-counts`: 13 documented counts
  agree, `docs/status/README.md`'s verification-file count included (62 → 63).

## Which index serves each query — NOT measured (first commit)

_Superseded for `payment_intents` by the coordinating session's measurement
in "Review round" below. Kept as written, because it is what the first
commit claimed._

The brief asked for `EXPLAIN` on the testcontainers Postgres. **It was not
run**: Docker was wedged. What follows comes from reading the migrations, and
it is a prediction, not evidence.

- **Intents.** `payment_intents_customer_idx` (0034, on `customer_id`,
  partial on `customer_id IS NOT NULL`) and
  `payment_intents_merchant_seq_idx (merchant_id, seq DESC)` (0014) are both
  candidates.
- **Sessions.** `checkout_sessions_customer_idx` (0034, the same partial
  shape) and `checkout_sessions_merchant_seq_idx` (0028) are both candidates.
- **Refunds.** The existing join shape is unchanged: `refunds` →
  `payment_intents` on `refunds_payment_intent_id_idx` (0017). The new
  predicate is one more comparison on the joined intent row.
- **The partial-index caveat.** The partial indexes are usable only when the
  planner can see `customer_id IS NOT NULL`. A _custom_ plan, with `$n` bound
  to a value, can see it. A _generic_ plan for `$n IS NULL OR customer_id =
$n` cannot, and would fall back to the merchant/seq index plus a filter.
  That is the same trade the `/dash/v1` status and `created_*` filters already
  make on this statement. Whether it matters at a merchant's real volume is
  unmeasured.

**No migration was added because none was shown to be necessary — and none
was shown to be unnecessary either.** Whoever runs the EXPLAIN should record
it here.

## What was not run (first commit)

_See "Review round" below for what the second commit ran._

- **The full integration suites** for `payment_intents.rs`,
  `checkout_sessions.rs` and `refunds.rs`. Only the three new cases ran, and
  that was before the disk incident. Every other case in those files, several
  of which call the same list routes, was not re-run on this tree.
- **`vpay-db`'s own tests** (`cargo nextest run -p vpay-db`). Some are
  container-backed. That is the run the disk filled during.
- **`EXPLAIN`**, as above.
- **`just ci`** as a whole, including `test-rust --workspace`, `test-web`,
  `audit-web` and `deny`.
- **No live-suite case** in either SDK. The intent integration case drives
  the filter through `vpay-sdk` against the real server. The session and
  refund cases use raw HTTP.
- **The vpay-skills companion change** is left to the coordinator.

## Review round — second commit, 2026-09-23

An independent review of `8d328d4` found no blocker and eight things to fix.
They are one new commit on the same branch; `8d328d4` was not amended. Docker
was healthy again for this round (the Docker Desktop VM had died behind a
"running" status; the coordinator restarted it). The build flags above were
used for every run.

### What changed

1. **The session/intent consequence, documented and not changed.** A checkout
   session created with `customer=X` on an intent with no customer stores
   `X` on the session only. So the payment it collects is listed by
   `GET /v1/checkout/sessions?customer=X` and **not** by
   `GET /v1/payment_intents?customer=X` or `GET /v1/refunds?customer=X`. The
   column each list compares is **accepted** in ADR-0024 (D12 for sessions;
   the maintainer confirmed D9–D19 "as proposed" on 2026-09-23, PR #248).
   Whether a session's customer should be written onto a customer-less intent
   is its **open question 3**
   (`docs/adr/0024-customer-filters-and-manual-payments.md`, not on `master`
   yet). This is now stated in `docs/api/README.md`'s three rows, the Status
   sections of `merchant-auth.md`, `hosted-checkout.md` and `customers.md`,
   `docs/reference/vpay-db/payment-intents-and-checkout-sessions.md`, and
   both SDKs' doc comments for `customer`.
2. **`/dash/v1` refuses `customer`.** Its `ListParams` ignored unknown keys,
   so `GET /dash/v1/payment_intents?customer=…` answered every customer's
   intents. It is now a `400` naming `customer` (blank is still absent), via
   `refuse_customer`, which follows `parse_status`'s shape. Proven by the unit
   case `the_customer_parameter_is_refused_and_not_ignored` and by a new
   assertion in
   `dashboard_read_surface::the_status_filter_narrows_the_page_and_refuses_an_unknown_status`.
   Both SDKs' `customer` doc comments also state the minimum server: **vpay
   0.6.0**, the first release after 0.5.0 (release-please's
   `bump-minor-pre-major` makes both this `feat` and its breaking footer a
   minor bump). A self-hosted server at 0.5.0 or earlier ignores the
   parameter and answers the unfiltered list.
3. **Status rows** are ✅ only for what the runs below proved.
4. **Performance**: see the next subsection.
5. **One copy of the refusal.** `v1/invoices.rs` now calls
   `customers::filter_param`. Because every list now shares one function, the
   byte comparison against `GET /v1/invoices` can no longer catch a change to
   the sentence itself. So each integration case now also asserts the literal
   message, which is the text `invoices.rs` spelled before the move. The unit
   case asserts it too.
6. **Test gaps closed.**
   - Each list's foreign-customer answer is now also compared byte for byte
     with the answer for **the caller's own** real customer that has no rows.
     That is `W`: no intents, no sessions, or intents with no refunds.
   - Blank `customer=` is tested on sessions and refunds.
   - The erased customer is tested on sessions.
7. **Stale comments** fixed:
   - `PaymentIntents::list_page_filtered`'s doc said it served the dash list
     only.
   - `v1::payment_intents::retrieve`'s comment pointed at a `list_page` call
     that is now `list_page_filtered`.
   - The vpay-db reference said "the filter", singular.
8. **Breaking Rust SDK change**, recorded in
   [../merchant-sdks.md](../merchant-sdks.md), `sdks/rust/README.md` §
   Status, and the commit's `BREAKING CHANGE:` footer.

### Performance — measured separately by the coordinating session, payment_intents only

Measured on Postgres 16 (`postgres:16-alpine`) with 200,000
`payment_intents`: 180,000 for one merchant, 120,000 of those spread over
2,000 customers. The predicate shape is exactly `list_page_filtered`'s. This
session did not run these measurements; they are recorded as reported.

- **Custom plan:** Bitmap Index Scan on `payment_intents_customer_idx`,
  0.47 ms.
- **Forced generic plan** (`plan_cache_mode=force_generic_plan`): Index Scan
  Backward on `payment_intents_seq_key`, with the customer as a Filter —
  31,949 rows removed, 8.95 ms. An unused or foreign customer id scans all
  200,000 rows, 37.8 ms.
- **Default `auto`:** custom plans chosen and the index used — 0.064 ms for a
  customer with 66 intents, 0.013 ms for an unused id.
- **After 50 unfiltered executions** of the same prepared statement on one
  connection, `pg_prepared_statements` shows `generic_plans=0`,
  `custom_plans=51`, and the filtered call still uses the customer index
  (0.20 ms).

**Conclusion: no migration is needed at this scale and distribution.** The
degradation exists only under generic plans — forced, or if the planner's
estimates change at larger or differently distributed data. **Not
measured:** `checkout_sessions` and `refunds`. The same predicate shape
predates this change, in `invoices::list_page`.

### What ran, and what it printed

All on the second commit's tree, with the flags above, against a healthy
Docker. "Skipped" is nextest's count, and it includes `#[ignore]`d tests.

| Run                                                                                                                                                                                      | Passed                                    | Failed       | Skipped / ignored                  |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- | ------------ | ---------------------------------- |
| `cargo nextest run -p vpay-tests-integration` over `payment_intents` (26), `checkout_sessions` (32), `refunds` (20), `invoices` (16), `dashboard_read_surface` (21) — **the full files** | 115                                       | 0            | 0 (17 other binaries not selected) |
| `cargo nextest run -p vpay-db -p vpay-api`                                                                                                                                               | 641, then the one failure re-run alone: 1 | 1, see below | 0                                  |
| `cargo nextest run -p vpay-sdk`                                                                                                                                                          | 181                                       | 0            | 0                                  |
| `sdks/nodejs` `pnpm test` (9 files); `typecheck` and `lint` clean                                                                                                                        | 225                                       | 0            | 0                                  |
| `just test-doc`                                                                                                                                                                          | 121                                       | 0            | 1 ignored                          |

- **The one `vpay-db` failure was infrastructure, not an assertion.**
  `repositories::a_cancel_racing_a_settlement_leaves_one_terminal_state_and_one_event`
  failed on testcontainers startup — `container '…' does not expose port
5432/tcp` — while the manual-payments agent was using the same Docker. It
  touches no filter code. Re-run alone, it passed: 1 passed, 242 skipped by
  the name filter. So `vpay-db` is **243 of 243**, but not in a single run,
  and this page does not claim one.
- `vpay-api`'s 399 include both new unit cases:
  `dash::payment_intents::tests::the_customer_parameter_is_refused_and_not_ignored`
  and
  `v1::customers::tests::the_customer_filter_is_checked_for_shape_and_answers_the_one_sentence`.
- `just fmt` clean. `just clippy` clean, after fmt.
- `just verify`: see below.

`just verify` on the staged second commit: exit 0, `verify: ok — the
fifteen gates above passed`. `verify-sdk-parity` is unchanged at 756
proving tests: no test was renamed, and the new SDK doc text is not a
capability row.

**Still not run on this branch:** `just ci` as a whole, meaning
`test-rust --workspace` across the other 17 integration binaries,
`test-web`, `audit-web`, `deny` and `verify-ignored`. Also no live-suite SDK
case, and no `EXPLAIN` for `checkout_sessions` or `refunds`.
