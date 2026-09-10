# exp46 — the Customer address, and an erasure that is actually complete

Issues [#67](https://github.com/vaam-apps/vpay/issues/67),
[#68](https://github.com/vaam-apps/vpay/issues/68) and
[#96](https://github.com/vaam-apps/vpay/issues/96) item 2. Branch
`claude/exp46-customer-address`, base `44e0c80`, migration `0041`.

## What the reading found, and why it changed the design

**Issue #68's premise is false.** It asks that "deletion (and the 12-month
sweep) also redacts phone/email on retained intents and sessions". Read
against the live schema:

- an intent carries an amount, a status and a `cus_…`. It has never carried a
  payer identifier, so there was nothing on it to redact;
- a customer any intent, session or invoice referenced **could not be
  deleted** at all — `payment_intents.customer_id` (0034),
  `checkout_sessions.customer_id` (0034) and `invoices.customer_id` (0036) are
  `NO ACTION` — so the identifiers survived on the customer row itself;
- the `409` that said so advised clearing `name`, `email` and `phone`, which
  `at_least_one_identifier` refuses. The advice could not be followed.

The copies that actually survived a "deletion" were somewhere else, and no
code named any of them:

| Where                                   | What                                                                                             |
| --------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `customers.{name,email,phone}`          | the row could not be deleted                                                                     |
| `events.data`                           | **every** `customer.*` body ever written; nothing prunes `events`                                |
| `charges.payer_ref`, `payer_ref_masked` | the payer's MSISDN as the rail was given it — reachable from a customer only _through_ an intent |
| `idempotency_keys.response_body`        | the exact JSON a `POST /v1/customers` answered, kept 24 hours                                    |

`provider_requests` (0016) and `webhook_deliveries` (0022) carry no bodies and
need no statement — checked, not assumed.

## The design taken

A customer with payment history is **never row-deleted**; `DELETE
/v1/customers/{id}` anonymises it. The row and the payment record stay, and
every one of the nine identifier columns becomes the literal `[redacted]`.
`metadata` stays (the merchant's data). The object renders `deleted: true`, so
a merchant's stored `cus_…` keeps resolving; a retrieve is `200`, not `404`.
A customer with **no** history is still hard-deleted, and a retrieve is a
`404`. The asymmetry is Stripe's.

`DELETE` therefore always succeeds or answers the uniform `404`. The `409` is
gone. A second `DELETE` is idempotent on the object's own state and emits no
second event. An erased customer cannot be updated or attached to a new
payment — `409` both, because `[redacted]` is not NULL and nothing below the
API would have objected.

The sweep does the same and stops skipping referenced customers, which is what
had exempted every paying payer from the twelve-month promise.

## Two decisions taken against the written brief, and why

**1. `at_least_one_identifier` is NOT relaxed.** The brief specified
`anonymized_at IS NOT NULL OR (…)`, on the assumption that anonymisation NULLs
the identifiers. It writes the marker, which is not NULL, so the constraint
already passes and the relaxation would have weakened a live constraint to buy
no behaviour. Left strict, it now backstops the erasure in the one direction
that matters: an erasure that NULLed the three identifiers rather than marking
them is refused outright.

What _is_ added is `anonymized_customers_carry_the_marker`, which is stronger
than anything the brief asked for: a row whose `anonymized_at` is set carries
`[redacted]` in **all nine** identifier columns. That makes a partially
completed erasure unrepresentable rather than merely discouraged.

**2. The invalid-country refusal is a `400`, not the `422` the brief names.**
This API has no `422`. `Category::InvalidRequest.http_status()` is `400` and
`vpay-core`'s own doctest asserts it; every parameter refusal on this resource
is a `400`.

## Measurements

|                                    | before | after                           |
| ---------------------------------- | ------ | ------------------------------- |
| migrations applied                 | 39     | 40 (no `0040` — exp44 holds it) |
| `EXPECTED_DRIFT_CHANGES`           | 172    | 176                             |
| `EXPECTED_DRIFTED_RELATIONS`       | 24     | 24                              |
| `EXPECTED_UNMAPPABLE_COLUMNS`      | 19     | 19                              |
| `EXPECTED_ASSERT_SITES`            | 56     | 60                              |
| `customers.rs` integration cases   | 18     | 21                              |
| workspace tests (`just test-rust`) | —      | 1681, 0 ignored, 46 binaries    |

The drift `+4` is a `+5` and a `-1`, and both halves are worth reading. The
`+5` is five hand-named single-column address CHECKs, the class every modelled
table in this schema pays a line for. The `-1` is
`phone_is_a_canonical_msisdn` **leaving the report**: `0041` widened it by one
disjunct so the marker is storable, which made it multi-column, and
`introspect/postgres/constraints.rs` filters those out. A constraint that got
_stronger_ as a pair with the marker CHECK reads here as drift going down.

**Seven new columns cost zero column-level lines** — no `type differs`, no
`default value differs`, no undeclared column. `model Customer` declares all
seven and none carries a DB `DEFAULT`, which is the condition migration `0034`
established for this table.

## Mutations run, and what caught each

| #   | Mutation                                                         | Caught by                                                                                                                                         |
| --- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1  | drop `address_city` from `anonymized_customers_carry_the_marker` | `an_anonymised_customer_carries_the_marker_in_every_identifier_column`, `the_redaction_marker_is_the_one_the_migration_enforces`                  |
| M2  | skip the `charges.payer_ref` redaction                           | `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`                                                                                |
| M3  | skip the `events.data` redaction                                 | same                                                                                                                                              |
| M4  | skip the `idempotency_keys.response_body` redaction              | same                                                                                                                                              |
| M5  | abandon the erasure's transaction after writing the event        | `a_customer_with_payment_history_is_anonymised_rather_than_deleted`, `a_delete_removes_the_row_and_answers_the_stripe_deleted_shape`, the scanner |
| M6  | drop `checked_country`, route `country` through `checked_text`   | `a_country_that_is_not_alpha_2_is_a_400_and_not_the_databases_503`, `an_address_round_trips_…`                                                    |
| M7  | drop the `invoices` clause from `UNREFERENCED`                   | `an_invoiced_customer_is_anonymised_rather_than_deleted`, `the_sweep_guard_names_every_table_that_can_reference_a_customer`                       |
| M8  | drop `anonymized_at IS NULL` from `idle_since`                   | `the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one`                                                                 |
| M9  | merge `address_city` component-wise instead of replacing         | `an_address_round_trips_is_replaced_whole_and_is_cleared_by_an_empty_value`                                                                       |
| M10 | render `deleted: false` on every live customer                   | `the_customer_object_is_the_documented_nine_keys`, `a_phone_only_customer_renders_the_absent_identifiers_as_null`                                 |
| M11 | rename the Rust SDK's erased-customer test                       | `verify-sdk-parity`, naming the row and the column                                                                                                |
| M12 | drop `address` from `CreateCustomerParams::to_form` (Rust)       | `create_customer_sends_the_documented_body_and_decodes_the_object`                                                                                |
| M13 | drop `address` from the Node create body                         | `customers.create: exact path, method, Idempotency-Key, and body`                                                                                 |

M6 is worth one note: `the_country_is_two_letters_upper_cased_and_nothing_else`
**passed** under that mutation, because it calls `checked_country` directly and
the function still exists — the mutation only stops routing `country` through
it. The integration case is what caught it. A unit test that exercises a
helper cannot see a caller that stopped calling it.

M11 is worth another: `verify-sdk-parity` checks **method** names and the
**test** names a ✅ cell claims. It does not check _fields_. Removing an SDK
field is caught by that SDK's own body-asserting test (M12, M13), not by the
gate.

## Where the erasure's five statements live

`vpay_db::customers::erase_in_tx`, in the caller's transaction:

1. `SELECT NOT (UNREFERENCED)` — chooses the branch. It runs under the
   caller's row lock, and that lock does **not** stop a concurrent
   `POST /v1/payment_intents` inserting a reference (an insert takes only a
   share lock on the customer). So the branch can be wrong by one race, in one
   direction, and the database catches it: the hard delete raises `23503` and
   the whole transaction rolls back, leaving the customer live and a retry
   that takes the other branch. The opposite race cannot happen — history is
   never removed.
2. the branch itself: `delete_many` through CrateStack, or the anonymising
   `UPDATE`.
3. the `customer.deleted` event, with the **redacted** body.
4. the `events` / `idempotency_keys` rewrites, sharing one
   `const REDACT_CUSTOMER_KEY`. Step 4 runs after step 3 deliberately, so the
   invariant is "no `events` row holds this payer's identifiers" and not "none
   except the newest".
5. the `charges` rewrite.

## What was deliberately not done

- **`GET /v1/customers` still lists erased customers.** Filtering them out
  would make the list deny a `cus_…` that `GET /v1/customers/{id}` answers,
  and make `has_more` describe a different set from the rows. Stated in
  `docs/flows/customers.md` as a choice, with the shape a filter would take.
- **Nothing tells the merchant to erase _their_ copy.** They received the
  identifiers in `customer.created` and every `customer.updated` before the
  erasure. vpay redacts its own stored bodies and cannot reach theirs. There
  is no `customer.redacted` fan-out and no design for one.
- **No `email` filter on the list** — unchanged, and unchanged for its
  original reason.
