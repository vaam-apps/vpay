# 2026-09-23 — customer erasure through checkout sessions (ADR-0027)

Branch `fix/erasure-through-checkout-sessions`, based on `origin/master` at
`9184e42`. macOS 26.5, Docker Desktop, real Postgres containers
(testcontainers). Other agents were building on the same host at the same
time, and that matters for one flake recorded below.

## What changed

The maintainer decided on 2026-09-23 that customer erasure must also reach
payments through `checkout_sessions.customer_id`
([ADR-0027](../../adr/0027-erasure-reaches-through-checkout-sessions.md)).
The gap was found while building vaam-apps/vpay#253.

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
customer's own id or its invoices. No stored intent, session or refund body
renders a payer identifier (`vpay_api::model`), so none of them needs a
statement on either path. The idle sweep (`erase_idle`) and `DELETE` both
call `erase_in_tx`, so one change covers both.

All three now use `IN (PAYERS_INTENTS)`:

```sql
SELECT id FROM payment_intents WHERE customer_id = $1
UNION ALL
SELECT p.id FROM checkout_sessions s
JOIN payment_intents p ON p.id = s.payment_intent_id
WHERE s.customer_id = $1 AND p.customer_id IS NULL
```

**The guard** is `p.customer_id IS NULL` on the session branch. Together with
the first branch, a session leads the erasure only to an intent whose own
customer is NULL or the customer being erased.

The `payment_intents` statement also gained
`last_payment_error_code IS NOT NULL`. It then locks only intents that have
decline text. A charge-less intent, the only kind a new session can be
created on, is never locked. vpay#253's create writes the intent and then
waits on the customer, which is the opposite of the erasure's order.

Also in this change: `erase_in_tx`'s doc said "six more statements". The
real number is **thirteen more, fifteen in all**, counted by reading the
code. The list also lacked #211's two statements.
`EXPECTED_ASSERT_SITES` goes 71 → 74, because the three statements now
interpolate the constant.

## Tests (real Postgres)

New, in `backends/tests/integration/tests/customers.rs`:

- `an_erasure_reaches_a_payment_whose_only_link_to_the_payer_is_a_checkout_session`.
  The historical shape is staged in SQL: a customer-less intent, a session
  naming `X`, a charge carrying `payer_ref`, a refund, and decline text. The
  `customer.created` body, a delivery whose excerpt echoes the MSISDN, and
  the stored `POST /v1/customers` response come from the API. One round
  erases through `DELETE` and one through the shipping worker's sweep. Each
  round checks the columns directly and then runs the whole-database scan
  before and after.
- `an_erasure_through_a_session_never_reaches_an_intent_that_names_another_customer`.
  An intent naming `Y` gets a session naming `X`, seeded in SQL. Erasing `X`
  leaves all six of `Y`'s copies byte-identical. A control payment, a
  customer-less intent whose session names `X`, _is_ redacted by the same
  erasure, so the case is not vacuous. `Y`'s own erasure then reaches `Y`'s
  payment.
- `an_erasure_takes_no_lock_on_a_session_reached_intent_it_has_nothing_to_erase_on`.
  The test holds `FOR UPDATE` on a charge-less, customer-less intent whose
  session names `X`. `DELETE /v1/customers/X` must finish within 10 s while
  the lock is held.

Extended: `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`
now also seeds a session-only payment carrying a sixth literal,
`SESSION_MSISDN`. The scan must find it in `charges.payer_ref`,
`charges.failure_raw`, `refunds.failure_raw` and
`payment_intents.last_payment_error_message` before the erasure, and nowhere
after it.

## Mutations (each reverted; the file was restored from a copy)

| Mutation                                                                                        | Result                                                                                                                                                                                                                 |
| ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A. `PAYERS_INTENTS` = `SELECT id FROM payment_intents WHERE customer_id = $1` (no session path) | **3 red:** the scan (`237600000772` left in the four columns above), the new case's `delete` round, and the guard case's control. The `sweep` round was also run alone and is red, with `237600000782` in `payer_ref`. |
| B. drop `AND p.customer_id IS NULL` (no guard)                                                  | **the guard case red**: `Y`'s `payer_ref` became `[redacted]`. The other two stay green, as expected.                                                                                                                  |
| C. drop `last_payment_error_code IS NOT NULL`                                                   | **the lock case red**, on its 10 s timeout, because the erasure waited on the held intent.                                                                                                                             |

## Suites

| Suite                                       | Passed | Failed | Ignored |
| ------------------------------------------- | -----: | -----: | ------: |
| integration `customers.rs`                  |     27 |      0 |       0 |
| integration `checkout_sessions.rs`          |     32 |      0 |       0 |
| integration `payment_intents.rs`            |     26 |      0 |       0 |
| integration `invoices.rs`                   |     29 |      0 |       0 |
| integration `webhooks.rs`                   |     20 |  1 (a) |       0 |
| `vpay-db` (lib and `tests/repositories.rs`) |    246 |      0 |       0 |

(a) `a_submit_decline_emits_one_payment_failed_and_it_reaches_the_receiver`
failed once in the five-binary run, with "the fan-out job is claimable". It
does not touch erasure. Re-run alone three times, it passed 3/3. It looks
like the class that
[2026-09-18-macos-loopback-and-two-load-flakes.md](2026-09-18-macos-loopback-and-two-load-flakes.md)
§ 2 describes: a job claim that depends on the container's clock, on a host
shared with other builds. That was **not investigated**, this test is not
the one that page names, and nothing here fixes it.

`webhooks.rs` is in the table because
`an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering`
lives there, and it passed.

## Gates, as run

- `just fmt`: done. `just fmt-check` passes.
- `just clippy` (`--workspace --all-targets -D warnings`): clean.
- `just test-doc`: **124 passed, 0 failed, 1 ignored.**
- `just verify`: "the fifteen gates above passed". `verify-links` reported
  2043 links in 428 tracked files. `verify-doc-counts` reported 13 counts
  agree, including the verification-directory file count, which this page
  moves to 66. `verify-privacy-inventory` reported 307 columns across 26
  elements, unchanged.
- `vpay_db::sql_audit`: green with `EXPECTED_ASSERT_SITES = 74`.

## Not done

- **No backfill, and nothing written onto an intent.** ADR-0025's reasons
  stand.
- The ambiguous historical intent, whose sessions named two payers, is
  redacted by either payer's erasure. That is stated in ADR-0027, not solved.
- The deadlock that the lock filter prevents was **not** reproduced against
  vpay#253's real create, which is not on `master`. The lock case uses a
  test-held row lock in its place.
- `just ci`, and the integration binaries not listed above, were not run.
  The vpay-skills PR was not opened (the coordinator does it).
