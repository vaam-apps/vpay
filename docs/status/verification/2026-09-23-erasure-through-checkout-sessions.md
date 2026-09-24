# 2026-09-23 — customer erasure through checkout sessions (ADR-0027)

Branch `fix/erasure-through-checkout-sessions`. macOS 26.5, Docker Desktop,
and real Postgres 16 containers (testcontainers). Other agents were building
on the same host at the same time, and that matters for one flake recorded
below.

This page has two passes:

- **§ 1** was run on `origin/master` at `9184e42`, before vaam-apps/vpay#253
  merged.
- **§ 2** was run after this branch merged `origin/master` at `fec2fc23`
  (#253, [ADR-0025](../../adr/0025-session-customer-onto-intent.md), and
  #254). It fixes the lock-order conflict that § 1 could only predict. § 2's
  mutations were measured on 2026-09-23, and its suites and gates finished
  just after midnight, on 2026-09-24.

## What changed

The maintainer decided on 2026-09-23 that customer erasure must also reach
payments through `checkout_sessions.customer_id`
([ADR-0027](../../adr/0027-erasure-reaches-through-checkout-sessions.md)).
The gap was found while building #253.

**Every copy that erasure reached through `payment_intents.customer_id`.**
This list was made by reading `vpay_db::customers` in full. There are three
statements, all in `redact_stored_copies`:

| Statement         | Columns                                                                 |
| ----------------- | ----------------------------------------------------------------------- |
| `charges`         | `payer_ref` → marker, `payer_ref_masked` → NULL, `failure_raw` → marker |
| `refunds`         | `failure_raw` → marker (via the charges)                                |
| `payment_intents` | `last_payment_error_code`, `_message` → NULL                            |

Nothing else in the erasure reaches a payment. `UNREFERENCED`, the branch
between hard delete and anonymise, already named `checkout_sessions`. The
event, delivery, idempotency and invoice statements are keyed on the
customer's own id or on its invoices. No stored intent, session or refund
body renders a payer identifier (`vpay_api::model`), so none of them needs a
statement on either path. The idle sweep (`erase_idle`) and `DELETE` both
call `erase_in_tx`, so one change covers both.

All three statements now use `IN (PAYERS_INTENTS)`:

```sql
SELECT id FROM payment_intents WHERE customer_id = $1
UNION ALL
SELECT p.id FROM checkout_sessions s
JOIN payment_intents p ON p.id = s.payment_intent_id
WHERE s.customer_id = $1 AND p.customer_id IS NULL
```

**The guard** has two parts:

- `p.customer_id IS NULL` on the session branch. Together with the first
  branch, a session leads the erasure only to an intent whose own customer is
  NULL or the customer being erased.
- Since § 2, `(customer_id IS NULL OR customer_id = $1)` on the row the
  `payment_intents` statement writes. The sub-select's check is evaluated
  against the statement's snapshot, and it is not re-checked when the
  statement waits on a row that a session create is giving to someone else.

Also in this change: `erase_in_tx`'s doc said "six more statements". The
real number is **thirteen more, fifteen in all**, counted by reading the
code, and the list also lacked #211's two statements.
`EXPECTED_ASSERT_SITES` goes 71 → 74, because the three statements now
interpolate the constant.

## § 1 — on `9184e42`

New cases, all in `backends/tests/integration/tests/customers.rs`:

- `an_erasure_reaches_a_payment_whose_only_link_to_the_payer_is_a_checkout_session`.
  The historical shape is staged in SQL: a customer-less intent, a session
  naming `X`, a charge carrying `payer_ref`, a refund, and decline text. The
  `customer.created` body, a delivery whose excerpt echoes the MSISDN, and the
  stored `POST /v1/customers` response all come from the API. One round
  erases through `DELETE`, and one through the shipping worker's sweep. Each
  round checks the columns directly and then runs the whole-database scan
  before and after.
- `an_erasure_through_a_session_never_reaches_an_intent_that_names_another_customer`.
  An intent naming `Y` gets a session naming `X`, seeded in SQL. Erasing `X`
  leaves all six of `Y`'s copies byte-identical. A control payment, a
  customer-less intent whose session names `X`, _is_ redacted by the same
  erasure, so the case is not vacuous. `Y`'s own erasure then reaches `Y`'s
  payment.
- Extended: `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`
  now also seeds a session-only payment carrying a sixth literal,
  `SESSION_MSISDN`. The scan must find it in `charges.payer_ref`,
  `charges.failure_raw`, `refunds.failure_raw` and
  `payment_intents.last_payment_error_message` before the erasure, and nowhere
  after it.
- A third case, `an_erasure_takes_no_lock_on_a_session_reached_intent_it_has_nothing_to_erase_on`,
  pinned a `last_payment_error_code IS NOT NULL` filter on the erasure's
  `payment_intents` statement. That filter was the first answer to the lock
  order. § 2 replaced both the filter and the case.

| Mutation                                                                                        | Result                                                                                                                                                                                                                 |
| ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A. `PAYERS_INTENTS` = `SELECT id FROM payment_intents WHERE customer_id = $1` (no session path) | **3 red:** the scan (`237600000772` left in the four columns above), the new case's `delete` round, and the guard case's control. The `sweep` round was also run alone and is red, with `237600000782` in `payer_ref`. |
| B. drop `AND p.customer_id IS NULL` (no guard)                                                  | **the guard case red**: `Y`'s `payer_ref` became `[redacted]`. The other two stay green, as expected.                                                                                                                  |
| C. drop the filter (§ 1 only)                                                                   | the lock case red, on its 10 s timeout. Both the case and the filter are gone as of § 2.                                                                                                                               |

§ 1's suite runs were: `customers.rs` 27, `checkout_sessions.rs` 32,
`payment_intents.rs` 26, `invoices.rs` 29, `webhooks.rs` 20 plus 1 failure (a),
and `vpay-db` 246. Nothing was ignored. `just fmt-check`, `just clippy`,
`just test-doc` (124 passed, 1 ignored) and `just verify` (15 gates) were
green.

(a) `a_submit_decline_emits_one_payment_failed_and_it_reaches_the_receiver`
failed once in the five-binary run, with "the fan-out job is claimable". It
does not touch erasure. Re-run alone three times, it passed 3/3. It looks
like the class that
[2026-09-18-macos-loopback-and-two-load-flakes.md](2026-09-18-macos-loopback-and-two-load-flakes.md)
§ 2 describes: a job claim that depends on the container's clock, on a host
shared with other builds. That was **not investigated**, this test is not the
one that page names, and nothing here fixes it. It passed in § 2's run.

## § 2 — after merging `fec2fc23` (#253): the lock order

**The conflict.** #253's `CheckoutSessions::create` ran the intent
compare-and-swap (`UPDATE payment_intents … WHERE customer_id IS NULL`) and
then the session `INSERT`. The insert's foreign key takes `FOR KEY SHARE` on
the customer. Erasure takes `FOR UPDATE` on the customer first, and then,
since § 1, writes intents it reaches through a session. On an intent reached
both ways, each side held what the other wanted.

**The fix.** `create` now calls `customers::erased_under_share_lock`
(`FOR SHARE` on the customer) inside its transaction, **before** the
compare-and-swap. `FOR KEY SHARE` would also block erasure's `FOR UPDATE`.
It does not block a plain `UPDATE` of `anonymized_at`, though, and the create
reads that column under the lock. `FOR SHARE` is also ADR-0024 D18's choice
for out-of-band pay. Under the lock, a customer erased since the pre-check is
never written onto a customer-less intent. The create is refused instead,
with `DbError::CustomerErased`, which `vpay_api` renders as
`erased_customer()`, the pre-check's `409`. An intent that already names the
customer (every invoice session) proceeds as before. § 1's filter was removed,
because it no longer prevents anything; ADR-0027 D4 and its Alternatives
say why.

New cases (they replace § 1's lock case):

- `a_session_create_and_an_erasure_of_its_customer_serialise_in_either_order`.
  It uses the contested shape: a customer-less intent, an **expired** session
  naming `X`, and decline text seeded without a charge.
  `stage_contested_intent` says why it has no charge: in production, the text
  reaches a create's transaction only through a confirm that fails after the
  create's pre-check. `X` also has a second, charged payment. Both orders are
  forced with locks the test holds:
  - **create first**: the test holds the intent row, and the create and then
    the erasure block. The expected result is create `201` and erasure `200`,
    the intent now naming `X`, and its decline text NULLed.
  - **erasure first**: the test holds the other payment's charge, and the
    erasure and then the create block. The expected result is erasure `200`
    and create `409`, whose `error` object equals the one the same request
    gets afterwards from the pre-check. The intent names nobody, and no
    session is added.

  Both orders also assert `pg_stat_database.deadlocks` did not move, that the
  other payment is erased, and that a scan for `X`'s MSISDN finds nothing.

- `an_erasure_and_a_session_create_naming_another_customer_leave_that_customers_intent_alone`.
  The same contested shape. A create naming `Y` holds the intent while `X`'s
  erasure waits on it. After both commit, the intent names `Y` and its
  decline text is intact.

| Mutation                                                                                    | Result                                                                                                                                                                                                                                                                                      |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D. no customer-first lock (`claim_intent_customer(…, false)`, the order before this change) | **red in both orders.** Create first: "Postgres aborted 1 transaction(s) with 40P01. The session create answered 201, the erasure 503", read from `pg_stat_database.deadlocks`. Erasure first (run alone): the create answered **201** and wrote the erased `X` onto the intent, not `409`. |
| E. drop `(customer_id IS NULL OR customer_id = $1)` from the `payment_intents` statement    | **the Y case red**: the decline text on the intent naming `Y` became NULL. The either-order case and § 1's guard case stay green.                                                                                                                                                           |
| A and B, re-checked on the merged tree                                                      | not re-run after the merge. The statements they mutate are unchanged apart from E's predicate.                                                                                                                                                                                              |

### Suites, § 2 (2026-09-24, merged tree)

| Suite                                       | Passed | Failed | Ignored |
| ------------------------------------------- | -----: | -----: | ------: |
| integration `customers.rs`                  |     28 |      0 |       0 |
| integration `checkout_sessions.rs`          |     35 |      0 |       0 |
| integration `payment_intents.rs`            |     26 |      0 |       0 |
| integration `invoices.rs`                   |     30 |      0 |       0 |
| integration `webhooks.rs`                   |     21 |      0 |       0 |
| `vpay-db` (lib and `tests/repositories.rs`) |    248 |      0 |       0 |

`checkout_sessions.rs` and `invoices.rs` include #253's cases, among them
the forced two-customer race and the invoice session that writes nothing
onto its intent, and they pass with the customer-first lock.
`vpay-db` has 248 tests. It had 246 in § 1: #253's error-classification test,
and this change's `a_session_attaching_an_erased_customer_is_a_conflict_not_a_storage_outage`.

### Gates, § 2

- `just fmt` was run, and `just fmt-check` is clean.
- `just clippy` (`--workspace --all-targets -D warnings`) is clean.
- `just test-doc`: **124 passed, 0 failed, 1 ignored.**
- `just verify`: "the fifteen gates above passed".
  - `verify-errors` reports 20 error types, all classified. The new
    `DbError::CustomerErased` arms are explicit.
  - `verify-links` reports 2071 links in 431 tracked files.
  - `verify-doc-counts` reports 13 counts that agree, among them the
    verification-directory count, re-measured at 68 after the merge.
  - `verify-privacy-inventory` reports 307 columns, unchanged.
- `vpay_db::sql_audit`: green, with `EXPECTED_ASSERT_SITES = 74`. The share
  lock is an existing statement, and E's predicate is inside a site that
  already existed.

## Not done

- **No backfill, and the erasure writes nothing onto an intent.** ADR-0025's
  reasons stand.
- The ambiguous historical intent, whose sessions all named two payers
  before ADR-0025 was deployed, is redacted by either payer's erasure. That
  is stated in ADR-0027, not solved, and § 2 does not change it.
- The confirm-between-pre-check-and-transaction interleaving is **not**
  driven through a real confirm. The decline text is seeded, as
  `stage_contested_intent` explains.
- Paying an erased customer's open invoice through a rail is unchanged. It
  was not in scope.
- `just ci`, and the integration binaries not listed above, were not run.
  The vpay-skills PR was not opened (the coordinator does it).
