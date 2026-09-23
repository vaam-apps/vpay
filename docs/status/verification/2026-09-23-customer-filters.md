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

## Which index serves each query — NOT measured

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

## What was not run

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
