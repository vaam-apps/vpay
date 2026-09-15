# 2026-09-16 — the four `/v1` refund routes, and the first refund events

RFC-0003 § 2, wave 3. Branch `refunds/w3-routes`, off
`claude/orange-refund-transaction-2b95e5` at `7e2e5381`.

## What this page is evidence for, and what it is not

It is evidence that a merchant can now create, read, list, annotate and cancel
a refund over HTTP; that the destination is validated against the rail's
**capability** and parsed by the rail's **adapter**; that the rail call carries
the refund's own reference; and that `charge.refunded` and
`charge.refund.updated` are emitted, in the transaction of the write each
reports, for the first time in this repository's history.

**It is not evidence that vpay can return money, and a `201` from
`POST /v1/refunds` does not say that it did.** Three things stand between
these routes and a refund a payer receives, and none of them moved here:

1. **MTN.** `mtn_momo::refund` is written and WireMock-proven. MTN's
   Disbursements product **has never been called from this repository**, in
   sandbox or anywhere else, and **no deployment holds its subscription key**
   — so on every deployment there is, this handler reaches
   `ProviderError::Config` naming the credential that is missing.
2. **Orange.** `orange_money::refund` is a declared
   `ProviderError::NotImplemented("orange_money::refund")` token, because an
   Orange refund is an outbound transfer and this repository has no
   specification for one (RFC-0003 § 5). The handler turns that into a
   `failed` refund with its reservation released, which is correct and is not
   a refund.
3. **Nothing settles a `pending` refund.** The port has no refund status read
   and `Refunded` has no status field, so there is no refund poll ladder
   (RFC-0003 open question 8, **still open**). A refund created through this
   route stays `pending` until an operator or the settlement path moves it.

Everything the WireMock stub proves is proof about **a stub transcribed from
MTN's published Transfer operation**, for which MTN's portal serves no OpenAPI
schema. A stub faithful to the documentation but not to the rail would pass
every case below.

## Gates run

`just ci` was **not** run — the brief for this work forbade it (five
concurrent local builds have OOM-killed this host). CI is the gate. What was
run, and its exact output:

| Command                                                                                                 | Result                            |
| ------------------------------------------------------------------------------------------------------- | --------------------------------- |
| `cargo nextest run -p vpay-api`                                                                         | 384 passed, 0 skipped, 0 ignored  |
| `cargo nextest run -p vpay-db`                                                                          | 243 passed, 0 skipped, 0 ignored  |
| `cargo nextest run -p vpay-core -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money` | 260 passed, 0 skipped, 0 ignored  |
| `cargo nextest run -p vpay-tests-integration --test refunds --test payment_intents`                     | 40 passed, 0 skipped, 0 ignored   |
| `cargo nextest run -p vpay-tests-integration --test payment_intents --test postgres_smoke`              | 78 passed, 0 skipped, 0 ignored   |
| `cargo nextest run -p vpay-sdk`                                                                         | 168 passed, 0 skipped, 0 ignored  |
| `cargo test --doc -p vpay-api -p vpay-db -p vpay-provider -p vpay-sdk`                                  | 18 + 8 + 12 + 8 passed, 1 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings`                                                 | clean                             |
| `cargo build --workspace --all-targets`                                                                 | clean                             |
| `cargo +nightly fmt --all --check`                                                                      | clean                             |
| `pnpm exec prettier --check .`                                                                          | clean (whole tree)                |
| `cargo xtask verify-status`                                                                             | ok — 1 token, declared            |
| `cargo xtask verify-sdk-parity`                                                                         | ok — 559 proving tests, 37 gaps   |
| `cargo xtask verify-links`                                                                              | ok — 1 694 links                  |
| `cargo xtask verify-errors` / `verify-repositories` / `verify-no-mocks` / `verify-serde`                | ok                                |

### Re-run on the contract review's head, 2026-09-16

Branch `review/w3f-contract`. `just ci` was **not** run here either, for the
same reason. Every row below is the command as run, and every container-backed
run had `DOCKER_HOST=unix:///run/user/1000/docker.sock` set — a `vpay-db` run
without it passes with zero cases.

| Command                                                              | Result                                        |
| -------------------------------------------------------------------- | --------------------------------------------- |
| `cargo test -p vpay-api --lib`                                       | 385 passed, 0 failed, 0 ignored               |
| `cargo nextest run -p vpay-db`                                       | 243 passed, 0 skipped                         |
| `cargo test -p vpay-tests-conformance --test adapter_conformance`    | 67 passed, 0 failed, 0 ignored (was 66)       |
| `cargo nextest run -p vpay-tests-integration --test refunds`         | 17 passed, 0 skipped (was 16)                 |
| `cargo nextest run -p vpay-tests-integration --test postgres_smoke`  | 53 passed, 0 skipped                          |
| `cargo nextest run -p vpay-tests-integration --test account_holders` | 6 passed, 0 skipped                           |
| `cargo test --doc -p vpay-api -p vpay-db -p vpay-provider`           | 18 + 8 + 12 passed, 0 ignored                 |
| `cargo clippy --workspace --all-targets -- -D warnings`              | clean                                         |
| `cargo +nightly fmt --all --check`                                   | clean                                         |
| `just fmt-check-web`                                                 | clean (whole tree, after formatting 4 files)  |
| `cargo xtask verify-all`                                             | all ten ok; `verify-migrations` now 48 files  |
| `just check-schema`                                                  | ok (cratestack 0.11.1 on PATH, pin is 0.12.0) |

Three mutations were run and each was reverted:

| Mutation                                                                                     | What failed                                                                                                                                    |
| -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `verify_registered_holder` calls `adapter.account_holder_name` directly again (as delivered) | `a_refund_path_lookup_is_counted_and_logged_like_the_routes`, on the counter, and nothing else                                                 |
| `orange_money` declares `RefundDestination::Origin`                                          | `no_shipping_rail_returns_a_refund_to_the_paying_instrument`, with the message naming what is owed                                             |
| `p.merchant_id = $1` becomes `$1 = $1` in `vpay_db::refunds::get_for_merchant`               | `the_write_routes_answer_the_same_404_for_a_foreign_refund_as_for_a_missing_one`: a foreign id answers `500` while a missing one answers `404` |

One thing verified against a real database rather than by reading the SQL: all
48 migrations applied to a `postgres:17-alpine` in order, and
`obj_description('refunds')`, `col_description('events','type')` and
`col_description('invoices','amount_refunded')` each read back with `0048`'s
text.

**Postgres and WireMock tests ran; none skipped.** Every container count above
is a real `postgres:16-alpine` and, for the eleven new refund cases, a real
`wiremock/wiremock`, under
`DOCKER_HOST=unix:///run/user/1000/docker.sock`; nextest reports `0 skipped`
on every run. The `vpay-db` suite in particular **silently passes with zero
Postgres tests** if that variable is unset, which is why the number and the
`0 skipped` are both recorded.

The one `ignored` doctest is `vpay-sdk`'s and predates this change.

## The decisive mutation, run and reverted

The brief named one:

> Delete the capability check in the create handler and confirm a refund on an
> `Origin` rail with a destination is then **wrongly accepted**.

Run on 2026-09-16. `resolve_destination`'s `(RefundDestination::Origin,
Some(_))` arm — the `400` naming `destination` — was replaced by `Ok(None)`,
which is exactly the shape of the bug: the merchant's nominated payee is
silently dropped and the refund proceeds as though the rail had been told
about it.

**Result: one test failed, and it is the one written for it.**

```
FAIL vpay-api v1::refunds::tests::an_origin_rail_refuses_a_destination
  an Origin rail takes no destination: None
Summary: 29 tests run: 28 passed, 1 failed, 355 skipped
```

**Nothing else in the workspace failed** — and that is the finding worth
recording rather than the pass. vpay carries **no `Origin` rail**: both
`mtn_momo` and `orange_money` declare `RefundDestination::Required`, so no
integration suite, no conformance case and no adapter test can reach that
branch at all. The only thing standing between that mutation and a merge is a
unit test with a hand-written fake rail. That is why the fake exists, and it
is why it is a fake rather than one of the two real adapters.

The arm was restored and `an_origin_rail_refuses_a_destination` passes.

## The other mutation this arm owed, from the wave-1 reviews

**A bad destination must not answer `502`.** `parse_destination` refuses a
malformed payee with `ProviderError::Malformed`, whose classification is
written for a rail that answered gibberish: forwarded unchanged it is HTTP
502, `Retry::AfterBackoff`, `api_error`, and the envelope sentence _"The
payment rail is temporarily unavailable. The charge will be retried."_ — for a
typo in the merchant's own request.

Two cases hold it shut, at two layers, because either alone is bypassable:

- `a_malformed_payee_is_the_callers_error_and_not_the_rails` (unit) asserts the
  `400` **and** `Retry::Never` on the `ApiError` the handler builds;
- `a_missing_or_malformed_destination_is_the_callers_400` (integration)
  asserts the status, `error.param == "destination"` and
  `error.type == "invalid_request_error"` through the real route — the last of
  those is what fails if a future author `?`s the `ProviderError`, because
  `Category::Rail` renders `api_error`.

Both also assert that the refusal does not echo the number it refused.

## A third mutation, which found a real bug in this arm's own first draft

Not named by the brief. Found by re-reading the handler against
`crate::v1::customers::create`'s stated rule — **"a replay must answer
whatever the original answered, whatever has changed since"** — and then
measured rather than argued.

The first draft claimed `POST /v1/payment_intents/{id}/confirm`'s ordering and
resolved everything **before** claiming the `Idempotency-Key`, on the grounds
that a refused refund should leave the key unspent. That reasoning is right
for `confirm`, whose pre-claim check reads the request body alone. It is wrong
here: every refusal in the refund create reads **mutable rows**, the intent's
counters among them. So a merchant whose `201` was lost to a timeout, retrying
the same full refund under the same key, was answered `409 invalid_state`
("this payment intent has nothing left to refund") — because their own first
request had taken it — instead of the refund they already had, and with no
`re_…` anywhere in the envelope to find it with.

`a_replayed_key_answers_the_stored_refund_even_when_the_intent_has_moved_on`
was written for it, and the **first version of that test did not catch it**:
it sent an explicit `amount`, which the handler resolves identically both
times, so only the database refused — after the claim — and the case passed
under both orderings. Measured, and recorded here because it is the more
useful half of the finding: a test aimed at an ordering has to be written
against the code path whose _meaning_ depends on the order, which for a refund
is `amount` **omitted** — "all of what is left" is a function of what the
first request did.

With the corrected test and the resolve-before-claim ordering restored as a
mutation, the result was:

```
FAIL a_replayed_key_answers_the_stored_refund_even_when_the_intent_has_moved_on
  a replay must answer the stored response, not re-run the rules against an
  intent the first request itself changed: 409 invalid_state
  "Payment intent pi_… has nothing left to refund."
```

and every other case in the file passed, including the other replay case.
The handler now claims first and **releases the key on every refusal**, which
is `customers::create`'s shape: nothing is written before the transaction
opens, so a corrected retry under the same key is equivalent to the request
never having been made.

## The silent money bug this arm was positioned to create

RFC-0003 open question 7: `ProviderAdapter::refund` receives one `ChargeRef`
carrying one reference, and a refund needs its **own**. A handler passing the
charge's would have every partial refund after the first answered
`409 RESOURCE_ALREADY_EXIST` by MTN — which the adapter reads as _accepted_,
correctly, because that is the crash-retry story — and reported to the
merchant as a refund that happened, with no money moved and no error anywhere.

`two_partial_refunds_of_one_charge_carry_two_references` asserts three things
against a real stub, and all three are needed:

1. the two refunds carry different `provider_reference_id`s;
2. neither is the charge's;
3. the **rail's own request journal** (`__admin/requests/count`, matched on
   `X-Reference-Id`) shows one transfer per reference.

Points 1 and 2 alone would pass an implementation that minted a reference and
then sent a different one.

## What the eleven new integration cases cover

Against a real Postgres and a real `wiremock/wiremock` serving the conformance
suite's own MTN mappings:

- a refund is created `pending`, reserved on its intent, and instructed to the
  rail under its own reference — and the payee's number is in **neither** the
  response body nor the event body;
- two partial refunds carry two references (above);
- `charge.refunded` is emitted, its `livemode` is the intent's, and its
  `data.object` is byte-identical to the API's response;
- an unbuilt rail's refund is `failed` with `failure_code = provider_error`,
  its reservation released, and both event types emitted in order;
- a missing destination and a malformed one are each a `400` naming
  `destination`, and neither writes a row;
- a payee the rail has no record of is refused **before** any transfer is
  instructed — the rail's journal shows zero;
- an over-refund is the database's `409`/`over_refund`, and writes nothing;
- the update merges `metadata` key-wise, removes a key sent empty, refuses
  `amount`, and emits one event for the write and none for the refusal;
- a cancel releases the reservation, cannot be repeated, and posts nothing to
  the ledger;
- the list is merchant-scoped, newest-first, filterable by `payment_intent`,
  and another merchant's intent id is an empty page rather than a `404`;
- a replayed `Idempotency-Key` answers the stored response and instructs no
  second transfer;
- **added by review, 2026-09-16** — `POST /v1/refunds/{id}` and
  `POST /v1/refunds/{id}/cancel` answer another merchant's real refund id and
  a `re_…` that names nothing with the **same body**, and it is the
  `resource_missing` envelope rather than the nest's `unknown_route`
  (`the_write_routes_answer_the_same_404_for_a_foreign_refund_as_for_a_missing_one`).
  `merchant_b_cannot_read_merchant_as_refund` had proved this for the read
  only, and the two write routes are where the property is easiest to lose:
  both read the refund before their write to tell a `404` from a `409`, and
  `cancel_once` re-reads to render the _status_ that refused it, which is a
  sentence that would confirm a foreign id exists and say what state it is
  in. Measured decisive: replacing `p.merchant_id = $1` with `$1 = $1` in
  `vpay_db::refunds`' `get_for_merchant` makes the foreign id answer `500`
  while the missing one stays `404`, and the case fails on the difference.

## Two findings from the contract review, 2026-09-16

**The refund path's account-holder lookup was counted and logged by nothing.**
`verify_registered_holder` called `ProviderAdapter::account_holder_name`
directly, making it the only caller of that port method outside
`GET /v1/account_holders`. Two stated properties broke at once:
`vpay_account_holder_lookups_total` stopped being every lookup vpay makes —
and [account-holder-lookup.md](../../flows/account-holder-lookup.md) § Status
asks an operator to alert on a sustained `not_found` rate near 1.0, which
reads a denominator — while a refund refused for an unregistered payee is
refused before any row, any `provider_requests` attempt and any rail
instruction, so it left **no trace in vpay at all**. Both callers now go
through `v1::account_holders::ask_rail`.
`a_refund_path_lookup_is_counted_and_logged_like_the_routes` fails on the
counter when the call is pointed back at the adapter, and nothing else in the
workspace fails with it.

**Ten claims that `POST /v1/refunds` is unrouted were left standing**, seven
in tracked files and three in `COMMENT ON` statements live in every database.
The file ones included this repository's `README.md`, `vpay-api`'s own crate
header, the boot `warn!` every deployment prints, the `V1_ROUTES` doc comment
directly above the new entries, and
[stripe-sdk-compat.md](../../flows/stripe-sdk-compat.md)'s
"`stripe.refunds.create()` remains a `404`, and correctly so". The database
ones are corrected by migration `0048` on `0020`/`0047`'s precedent — the
`refunds` table's, `invoices.amount_refunded`'s and, since both refund event
types now have a writer, `events.type`'s "eleven of the fifteen". All 48
migrations were applied to a real Postgres 17 and the three comments read
back with the new text.

## What was NOT done, and is recorded rather than left to be discovered

- **Neither merchant SDK can call the create.** Neither `CreateRefundParams`
  has a `destination` field, and both rails declare `Required`, so
  `refunds.create()` from either SDK is answered `400`. It was harmless while
  the route was a `404`; it is a dated gap in
  [../../sdks/parity.md](../../sdks/parity.md) and in both SDKs' own doc
  comments as of this change. Adding the field is a wire-shape change both
  SDKs make together, which is a separate arm.
- **The destination is persisted nowhere.** There is no `destination` column
  on `refunds` and this change adds none: its retention is RFC-0003 open
  question 3 and is **undecided** (the customer object settled on twelve
  months; a refund destination has no policy). Storing it would be answering a
  question reserved for the maintainer. The cost, stated: vpay cannot tell an
  operator which payee a refund was sent to — the rail's records can, keyed by
  the `provider_reference_id` on the row.

  **One qualification the review added**, because "persisted nowhere" is
  stronger than the code can promise: a `ProviderError::Rejected`'s message
  is stored in `refunds.failure_raw`, and a rail is free to echo the payee it
  refused back in it. Nothing renders that column — `RefundObject` is ten
  keys and none is a failure field — so it reaches no response, no event and
  no webhook. It is kept rather than scrubbed because a refusal reason with
  the rail's words removed is one nobody can act on, and pattern-matching a
  third party's text for phone numbers would be a guess. It is the only route
  by which a destination can outlive the request, and it is now stated in
  `vpay_api::v1::refunds`' module header rather than left for whoever answers
  open question 3 to discover.

- **The `Origin` arm of `resolve_destination` is guarded by one unit test on
  a hand-written fake, and that was reviewed and kept.** No fixture `Origin`
  rail was added to the conformance suite — an earlier arm refused that and
  ADR-0006 is why. What the review added is
  `no_shipping_rail_returns_a_refund_to_the_paying_instrument`, which asserts
  the premise that makes one fake adequate: every adapter in the workspace
  declares `RefundDestination::Required`. Its failure message names the three
  things owed before the assertion may be relaxed. Measured decisive:
  flipping `orange_money` to `Origin` fails it.
- **No refund poll ladder.** RFC-0003 open question 8 is untouched and open.
- **`refunds.fee` is still written by nothing.** The handler logs a warning if
  an adapter ever reports one.
- **`just ci` was not run**, per the brief. The table above is what was run.
