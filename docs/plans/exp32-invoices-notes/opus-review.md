# exp32 — Invoices (S4b): sabotage review

2026-09-07. Branch `claude/exp32-invoices`, reviewed at `e1206e3` (three
commits on `174e16b`). Companion to
[opus.md](opus.md), the implementation notes.

This file records what was **measured** — every mutation applied to the source
and re-run, every claim checked against a database rather than against a
comment — and what was left undone or handed to the maintainer.

---

## Verdict

**Not safe as delivered.** The design is right and most of it is enforced where
it says it is, but three mutations on the money path passed the delivered suite
and two written claims were false. All five are fixed below; two further gaps
are recorded rather than closed because closing them is not this resource's
work.

The single most important measurement: **deleting `NO_LIVE_INTENT` from
`attach_intent` left all twelve delivered wire cases green while two concurrent
`POST /v1/invoices/{id}/pay` requests produced two payment intents and two
hosted checkout URLs for one bill.** The implementer knew the API's own read
masks the statement — that is exactly why
`attaching_a_second_intent_to_an_invoice_is_refused_by_the_statement` exists at
the repository seam — but the suite never put two `pay` requests in flight
_together_, which is the shape a real double-charge arrives in.

---

## Findings

| #   | Severity         | Finding                                                                                                                                                                                                                                                                                                              | State                                                                                             |
| --- | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| F1  | gate-hole        | Two concurrent `pay` requests were never raced. With `NO_LIVE_INTENT` deleted from `attach_intent`, twelve of twelve wire cases pass while one invoice acquires two live intents and two hosted URLs                                                                                                                 | fixed — `two_concurrent_pays_attach_exactly_one_intent`                                           |
| F2  | gate-hole        | Neither list cursor's tenancy predicate was pinned. Unscoping either sub-select in `list_page` leaves every case green while another merchant's `in_…` pages the caller's own rows                                                                                                                                   | fixed — `a_foreign_cursor_is_an_empty_page_and_never_an_oracle`                                   |
| F3  | gate-hole        | Nothing pinned that a settlement flips **its own** invoice. Keying `mark_paid_for_intent_in_tx` on anything but the intent leaves all seven delivered invoice/settlement cases green while one intent's settlement pays an invoice it was never bound to                                                             | fixed — `a_settlement_pays_only_the_invoice_its_own_intent_is_bound_to`                           |
| F4  | correctness      | `pay` mints its intent through `PaymentIntents::insert`, not `parse_amount`, so `MAX_AMOUNT` (`2^53 - 1`) did not apply. Ninety-one lines at both `invoice_items` ceilings finalized `200` at 9,100,000,000,000,000 — an `amount` `POST /v1/payment_intents` refuses, on an object every JSON client silently rounds | fixed — refused at `finalize`, `an_invoice_over_the_representable_ceiling_is_refused_at_finalize` |
| F5  | misleading-claim | `docs/api/README.md` said the invoice object has **seventeen** keys. It has eighteen, and no test of any name held the number — the customer object's own review found this exact shape the day before and the invoice inherited the habit without the tripwire                                                      | fixed — count corrected, `the_invoice_object_is_the_documented_eighteen_keys`                     |
| F6  | misleading-claim | `invoices.rs`' module header and one case doc said a reused idempotency key with a different body is a `422`. The case has always asserted the `400` `idempotency_key_in_use` envelope, which is this API's documented answer                                                                                        | fixed — both comments corrected                                                                   |
| F7  | misleading-claim | Migration `0036`'s first line cites `docs/plans/2026-09-06-data-layer.md`, which is neither tracked nor on disk. `docs/flows/invoices.md` corrects `0034`'s identical citation and says it "is not repeated here" — while the same commit repeated it                                                                | fixed — the correction now names both; the `.sql` is immutable                                    |
| F8  | misleading-claim | The brief asked that the payer see the invoice number. The intent carries `Invoice {number}` in `description` and the browser read expands it, but `frontends/apps/checkout` renders the amount and the merchant name only — and this was not in the NOT-built list                                                  | recorded as a dated gap, not closed                                                               |
| F9  | nit              | `docs/flows/invoices.md` says nothing about what a refund does to a paid invoice                                                                                                                                                                                                                                     | recorded as a dated gap **and a maintainer decision**; unreachable today                          |
| F10 | nit              | `invoices.rs`' `form_body` encoded a space as `+`, which `vpay_api::form` treats as a literal plus by design, so every description the suite sent was stored as `One+month+of+hosting`. No assertion read one back                                                                                                   | fixed — `%20`, and the round-trip is now asserted                                                 |

### Not findings, checked and cleared

- **`pay` after `void`, and every other terminal refusal.** Already covered by
  `the_two_terminal_transitions_and_the_transitions_they_refuse`, which loops
  `void`/`mark_uncollectible`/`pay`/`finalize` over a voided invoice with the
  `pay` parameters attached so a missing-parameter `400` cannot masquerade as
  the `409`.
- **Finalize twice.** Covered, with the "no second event" assertion beside it.
- **A rolled-back finalize burning a number.** Covered, and the answer is that
  it burns none — `invoice_number_sequences` is a table row, not a
  `SEQUENCE`. The migration, the repository doc and the flow doc all say the
  same thing, and the test is what makes it true.
- **A `succeeded` intent whose invoice was voided in between.** Covered by
  `a_settlement_whose_invoice_moved_emits_no_invoice_event`: the charge and
  the intent still settle (the money is not lost and
  `payment_intent.succeeded` is still delivered), the invoice is left exactly
  as it was, and **no** `invoice.paid` is written. Fail-closed, and reaching it
  at all needs the `NO_LIVE_INTENT` guard to have been bypassed, because a
  live intent cannot be voided around.
- **Overflow of `quantity × unit_amount`.** `QUANTITY_MAX × UNIT_AMOUNT_MAX =
10^14`, four orders inside `i64`, and a unit test asserts the product of the
  two constants is `checked_mul`-safe so they cannot be raised independently.
  The DB computes the product in `BIGINT` and would raise `22003`; nothing
  the API accepts reaches it. The _invoice total_ was the unbounded one — F4.
- **Zero and negative `quantity`, decimal `unit_amount`, XAF's zero exponent.**
  `parse_quantity`/`parse_unit_amount` are the only path, both refuse with a
  `400` naming the parameter, and both are unit-tested at the boundary.
  `1500.50` is refused by the parse rather than rounded, which is the right
  answer for a zero-decimal currency.
- **Tenancy on every route.** `another_merchants_invoice_is_byte_identical_to_
one_that_never_existed` compares **bytes** across nine routes including both
  `invoice_items` paths, and pins that `invoice=` on a line create is the same
  `400` for a foreign invoice as for a missing one. The cursor was the only
  hole (F2).
- **Events in the transition's own transaction.** `write_with_event` returns
  `TxOutcome::Abandon` on a refused compare-and-swap, so no event and no
  burnt number; the delivered suite asserts the count after every refusal.
  `invoice.paid` is inside TX1 and `an_aborted_settlement_leaves_the_invoice_
open_and_emits_nothing` is the proof.
- **`@@allow` arms.** Both actions this module calls through CrateStack have
  one, and `every_action_this_module_calls_has_an_allow_arm` fails in
  milliseconds without a container. `model Invoice`'s `update` arm fails
  _silently_ if deleted, which is why the assertion exists at all.

---

## Mutations run

Each was applied to the source, the named tests were run, and the source was
restored and verified clean with `git diff --quiet`.

| #   | Mutation                                                                    | Result                                                                                                                  | Caught by      |
| --- | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | -------------- |
| M1  | drop `AND {NO_LIVE_INTENT}` from `attach_intent`                            | **12/12 delivered wire cases PASS**; the new case FAILS with `[200, 200]` — two intents, two hosted URLs, one invoice   | F1's case only |
| M2  | drop `AND merchant_id = $1` from `list_page`'s `starting_after` sub-select  | FAIL — merchant B's own older invoice comes back for a cursor naming merchant A's                                       | F2's case only |
| M3  | the same on `ending_before`                                                 | FAIL — merchant B's own newer invoice comes back                                                                        | F2's case only |
| M4  | key `mark_paid_for_intent_in_tx` on `status = 'open'` instead of the intent | **7/8 delivered invoice+settlement cases PASS**; the new case FAILS — an invoice is paid by an intent never bound to it | F3's case only |
| M5  | render `merchant_id` on `InvoiceObject`                                     | FAIL, naming the column and why an `events` row cannot be retracted                                                     | F5's case only |
| M6  | delete the `amount_due` ceiling from `transition_once`                      | FAIL — finalize answers `200` at 9,100,000,000,000,000                                                                  | F4's case only |

**M2 was measured twice, and the first measurement is the one worth keeping.**
The first version of F2's case gave merchant B a single invoice _newer_ than
merchant A's, and M2 **passed** it: `seq < seq(theirs)` excludes B's only row
whether or not the sub-select is scoped. The fixtures now straddle the foreign
invoice — one of B's older, one newer — so each cursor has a row it would leak.
A tenancy test whose fixture ordering makes the mutation invisible is a test
that reports success for a check that never ran.

---

## Measurements

### `just ci`, recipe by recipe

Run twice: once on the delivered `e1206e3`, once on this review's head. Node
`22.23.2` (`.nvmrc`) after `pnpm install --frozen-lockfile`; `cratestack`
0.12.0 in a private `--root`, never in the shared `~/.cargo/bin`.

| Recipe           | Result                                                                                                                                                                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fmt-check`      | ok                                                                                                                                                                                                                                                      |
| `clippy`         | ok                                                                                                                                                                                                                                                      |
| `verify`         | the twelve gates ok — including `check-schema` under cratestack **0.12.0** (22 declarations, datasource present), `verify-sdk-parity` (407 proving tests, 41 dated gaps, 19 methods over 23 rows) and `verify-migrations` (36 files match the manifest) |
| `test-rust`      | **1589 passed, 0 skipped** on `e1206e3`; **1593 passed, 0 skipped** with this review's four new cases                                                                                                                                                   |
| `test-doc`       | ok                                                                                                                                                                                                                                                      |
| `verify-ignored` | ok                                                                                                                                                                                                                                                      |
| `lint-web`       | ok                                                                                                                                                                                                                                                      |
| `test-web`       | ok — 1,116 tests across nine packages                                                                                                                                                                                                                   |
| `deny`           | ok                                                                                                                                                                                                                                                      |

**One honest correction about my own run.** The first `just ci` failed in
`test-web` (`@vpay/ui`'s `select.test.tsx`), and it was my environment, not the
change: I had not run `pnpm install --frozen-lockfile` before starting, so the
worktree's `node_modules` did not match the lockfile. After the frozen install
the same recipe passes. Recorded because "a red gate I caused" and "a red gate
the branch caused" are different facts and only one of them is about this
work.

### Drift, re-derived rather than trusted

`cratestack migrate baseline --strict` at **0.12.0** against a fresh
`postgres:16-alpine` with every migration applied:

```
drift detected in 23 table(s)/view(s) (156 change(s) total)
```

Both numbers match the constants `postgres_smoke.rs` pins. The per-table split
matches the accounting in `EXPECTED_DRIFT_CHANGES`' own comment, read off the
report: `invoices` **16** (eight hand-named CHECKs, six undeclared indexes,
`seq`'s default, `status`' type), `invoice_items` **9**,
`invoice_number_sequences` **1**.

**The zero-cost enum CHECK claim is true, and this is how it was checked.**
`invoices_status_enum_check` appears in the report in **neither** direction —
not as "exists in the live database but is not declared", not as "is declared
in the schema but does not exist in the live database". The contrast that
makes it a measurement rather than an absence: `ledger_entries` in the same
report carries `[blocking] CHECK ledger_entries_amount_range_check is declared
in the schema but does not exist in the live database`, which is exactly the
line a name mismatch produces. Migration `0036` creates the constraint under
`naming.rs::check_name(table, column, "enum")`'s own spelling, so the declared
and the live constraint are one object to `diff/checks.rs`, which matches by
name first.

The permanent `[lossy] column status type differs (live: Scalar("String"),
schema: Enum("InvoiceStatus"))` line is there, as the notes said it would be.

**The five multi-column CHECKs contribute nothing in either direction**, also
confirmed by reading the report: none of `number_is_assigned_at_finalize`,
`paid_means_nothing_remaining`, `only_a_live_invoice_has_an_intent`,
`amounts_add_up` or `amount_is_the_product` appears anywhere in it. That is
why `the_invoice_invariants_are_enforced_by_the_database_itself` writes the row
each one refuses instead.

---

## Maintainer decisions

The implementer surfaced four (D1–D4 in [opus.md](opus.md)); all four stand and
none is reversed here. Two more:

### D5 — what a refund does to a paid invoice

Not reachable today: nothing in this repository writes a `refunds` row
(`POST /v1/refunds` is unrouted, `mtn_momo::refund` is `NotImplemented`,
Orange Money answers `Unsupported`). It is surfaced because the answer is not
obvious and the schema has already taken a position:
`paid_means_nothing_remaining` makes a `paid` invoice with anything remaining
**unstorable**, so a refund cannot decrement `amount_paid` on a paid invoice
without either a status change or a constraint change. Stripe's answer is a
credit note, which vpay does not have.

Written down in `docs/flows/invoices.md` under "What is not built" with the
constraint that forces the choice. Not decided here.

### D6 — whether the payer should see the invoice number

The number is already on the wire the checkout page reads (`pay` writes
`Invoice {number}` into the intent's `description`, and
`GET /v1/browser/checkout/sessions/{id}` expands the intent). The page renders
the amount and the merchant name. Showing the number is a change to
`frontends/apps/checkout`'s screens and to `@vpay/tokens`' copy — a different
surface, with Cypress coverage of its own — so it was recorded as a dated gap
rather than opened here.

The reason it is a maintainer's call and not a bug report: a payer following a
link from an e-mail cannot currently tell which bill they are paying, which is
a real weakness of paying an invoice through a generic checkout page, and the
alternative the brief ruled out ("do not build an invoice page") is the other
way to solve it.

---

## What this review did NOT do

1. **No Cypress case follows a `hosted_invoice_url`.** Unchanged from the
   delivered state, and it is the natural home for D6's fix; running the
   browser suite needs `compose.e2e.yml`, whose dashboard port is hard-coded
   (#78), and adding a spec that nothing in this pass would have exercised is
   not worth the collision risk against the maintainer's own stack.
2. **Neither SDK gained an invoice method.** See the verdict below.
3. **`lines.data` is still empty in `invoice.*` webhook bodies.** The reason
   given — a second query on a connection holding the sequence row's lock — is
   sound for `finalize` and weaker for `create` and `void`, which hold no such
   lock. It is documented in three places and consistent across all four
   types, and making one type differ from the other three would be worse than
   the gap. Not changed.
4. **`due_date` is still read by nothing.** Documented as advisory in the
   column comment, the row doc, the flow doc and the API reference. Left alone.
5. **The `+`-is-a-literal-plus rule was not changed.** `vpay_api::form`
   percent-decodes and only percent-decodes, deliberately and with its own
   test, so a client that sends `description=a+b` stores `a+b`. That is
   pre-existing, documented, and outside this branch; only the test helper was
   corrected.
6. **No load or timing measurement.** "Two concurrent requests" is two, on one
   machine, through `tokio::join!`. It is enough to make the mutation fail and
   it is not a statement about contention at scale.

---

## The SDK gap: acceptable, and what the follow-up is

**Verdict: acceptable, and correctly recorded.** ADR-0015's rule is that every
merchant SDK ships every method **or carries a dated gap row with an owner**,
checked in both directions by `cargo xtask verify-sdk-parity`. Three rows were
added, `verify-sdk-parity` is green **with** them (407 proving tests, 41 dated
gaps), and the gate is two-directional since 2026-09-06 — deleting a whole row
fails it — so these rows cannot quietly disappear.

Two things make this the right call rather than a convenient one:

- The rows **separate** "the resource is missing" from "the four `invoice.*`
  types are missing from the event union" from "it has never run against a
  live stack", so building the methods against stubs cannot silently close all
  three.
- The second row states the sharper half plainly: unlike the
  `customer.created` row above it, **the server does emit these four**, so a
  merchant's typed handler drops an event that is really being delivered.
  That is the more urgent half of the gap and it is the one a reader would
  otherwise miss.

And the alternative would have been worse. Writing the integration suite
against a client that did not exist is the failure `CLAUDE.md` names by hand;
the suite drives raw HTTP for exactly that reason, and says so in its own
harness doc.

**The follow-up, in the order it should be done:** (1) the four `invoice.*`
entries in both event unions — smallest change, largest correctness win, and
the only part of the gap where vpay is actively delivering something a
merchant's code drops on the floor; (2) `invoices.*` and `invoice_items.*` in
both SDKs with body-asserting tests, closing rows one and three together;
(3) the Stripe-compat cases the real `stripe` package can drive; (4) a Cypress
case from a `hosted_invoice_url` to a paid charge, which is also where D6 lands.
