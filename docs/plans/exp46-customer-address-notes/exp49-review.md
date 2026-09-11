# exp49 review — the GPS half, sabotaged

The adversarial review of the commits `80f660c`..`b5dc04a` on
`claude/exp46-customer-address` ([PR #112](https://github.com/vaam-apps/vpay/pull/112)).
[exp49-gps.md](exp49-gps.md) is the implementer's account of the same work;
[opus-review.md](opus-review.md) is the earlier review of the address and the
erasure. PR #112 was green on every CI check and had never been reviewed,
which is the state a branch is most likely to be landed on trust in.

**No correctness or privacy defect was found in the GPS half.** Everything
below is documentation that claimed more, or something different, from what
the code does. That is the honest headline and it is unusual for this
repository: every mutation this review ran was already caught by a named
test.

## Mutations run, each restored afterwards

Each was applied to this branch's own head and each is reported with the
message it produced. `-j 6` throughout, on a host shared with another agent.

| #   | Mutation                                                                                                                                                        | Result                                                                                                                                                                                                                         |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| R1  | `LATITUDE_MAX_MICRODEG` 90 000 000 → 180 000 000                                                                                                                | **caught** — `vpay_api::v1::customers::tests::the_coordinate_bounds_are_the_ones_the_migration_enforces`: "`address_latitude_microdeg_range` does not enforce the bound this module refuses with (180000000)"                  |
| R2  | the API's pair refusal made unreachable (`if false && !address.coordinate_is_paired()`)                                                                         | **caught** — `half_a_coordinate_is_refused_by_the_api_and_not_by_the_check`                                                                                                                                                    |
| R3  | `CustomerAddress::redacted` keeps the payer's point                                                                                                             | **caught twice** — `vpay_db::customers::tests::an_erasure_projects_the_coordinate_to_absent_and_the_rest_to_the_marker` and `vpay_api::model::tests::an_erased_payers_coordinates_are_null_and_not_a_marker`                   |
| R4  | the coordinate removed from the erasure `UPDATE` **and** its two conjuncts removed from `anonymized_customers_carry_the_marker`, so the database cannot mask it | **caught twice, against a real Postgres** — `a_customer_with_payment_history_is_anonymised_rather_than_deleted` and `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`: "left: (Some(4061777), Some(9786777))" |
| R5  | the two coordinate keys dropped from the Node SDK's `addressBody` loop                                                                                          | **caught by the test suite, NOT by the parity gate** — see below                                                                                                                                                               |
| R6  | the two coordinate fields dropped from the Node SDK's response `Address`                                                                                        | **caught** — `tsc --noEmit`, five errors across `client.test.ts` and `types.test.ts`                                                                                                                                           |

## What `verify-sdk-parity` cannot see, measured

R5 is the one worth writing down. With `latitude_microdeg` and
`longitude_microdeg` deleted from `addressBody`'s key loop — so every
`customers.create` and `customers.update` silently drops the payer's point
on the way out — `cargo xtask verify-sdk-parity` **exits 0** and prints
"ok — 463 proving test(s) named in docs/sdks/parity.md all exist".

That is not a defect in the gate and not a false claim anywhere: AGENTS.md
describes the gate as "every claimed capability names a test that exists",
and it does exactly that. It is worth stating plainly because the ✅ in the
parity row for the GPS half reads like a guarantee the gate is giving, and
the guarantee is actually the test's: `pnpm --filter @vaam-apps/vpay-sdk test`
fails the named case `customers.create: exact path, method, Idempotency-Key,
and body` on the exact body string. The capability is guarded. It is guarded
one layer lower than the row's ✅ suggests.

## Range, pair rule and float freedom, re-checked rather than read

- The column is `BIGINT` and the axis maxima are ±90 000 000 and
  ±180 000 000 — four orders of magnitude inside the type.
- Both boundary values are legal on both axes and both signs; one
  microdegree outside each is refused by that axis's **own** named
  constraint, which `a_half_written_coordinate_is_refused_by_the_database`
  asserts by name for all four. 91° and −181° are therefore refused, at the
  API with a `400` naming `address` and at the database as the backstop.
- The prior `=`-on-NULL defect is **not** repeated. `anonymized_customers_
carry_the_marker` is `IS NOT DISTINCT FROM` in all eleven conjuncts, the two
  coordinate ones spelled `IS NOT DISTINCT FROM NULL`; the pair rule is
  `(a IS NULL) = (b IS NULL)`, whose two sides are booleans that are never
  NULL. No comparison against a possibly-NULL column remains in either.
- No `as f64`, no `/ 1_000_000.0`, no `parseFloat`, no `toFixed` anywhere in
  `vpay-api`, `vpay-db` or either SDK. The only `f64` in the whole path is
  `elapsed.as_secs_f64()` in an unrelated metrics histogram, and prose
  explaining why a coordinate is not one.

## Documentation corrected, in `a491655`

A false test citation in `sdks/rust/tests/support/mod.rs`; eight doc
comments still saying "six" about structures this branch widened to eight;
and every one of the six sub-counts in `docs/status.md`'s opening sentence
for the customers row, which had carried the 2026-09-06 numbers through
three changes. The commit message has the measurements.

`docs/flows/customers.md`'s own counts — twenty-three, six, eleven, two —
were re-measured and are all **correct**.

## What this review did not check

- `just test-e2e` (Cypress against `compose.e2e.yml`) and `just helm-check`,
  both out of `just ci` by design.
- `just docs-check-citations`, which needs the network and a GitHub token, so
  the issue, PR and run-id citations the docs commit added are unverified.
- `EXPECTED_DRIFT_CHANGES = 179` was not re-derived by hand; it is taken from
  `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` passing.
- The pre-GPS half of PR #112 (up to `a4aa095`), except where the GPS commits
  touch it.
- Concurrency between an erasure and an update on the same coordinate, beyond
  the guards already asserted for the rest of the row.

## The gate

`just ci` on `a491655`, exit code **0** read from a file: the twelve verify
gates, 1723 nextest tests passed with 0 ignored across 46 binaries, 113
doctests, every web suite, and cargo-deny.
