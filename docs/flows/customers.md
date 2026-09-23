# Customers

The merchant-owned record of a payer they expect to see again — `cus_…`, on
`/v1/customers`. It is the object a merchant stores against their own user
record and sends back on every later payment intent, and it is the only object
in vpay whose entire content is **another person's personal data**.

Everything below is about that second sentence. The shape of the object is
three paragraphs; the rules around it are the rest of the document.

---

## What it is

| Field      |                                            |                            |
| ---------- | ------------------------------------------ | -------------------------- |
| `id`       | `cus_…`                                    |                            |
| `object`   | `"customer"`                               |                            |
| `name`     | string or `null`                           |                            |
| `email`    | string or `null`                           |                            |
| `phone`    | string or `null`                           | canonicalised — see below  |
| `address`  | object of eight components, or `null`      | see below                  |
| `metadata` | ≤ 50 keys, ≤ 40-char key, ≤ 500-char value |                            |
| `created`  | unix **seconds**                           |                            |
| `livemode` | boolean                                    |                            |
| `deleted`  | `true`, **or the key is absent**           | only on an erased customer |

Nine keys, and a tenth only when the payer has been erased. **At least one of
`name`, `email` and `phone` is always present**: a customer with none of them
names nobody, can never be matched to a payer, and is the shape an integration
creates by accident from a form with every field blank. An address does not
count — it does not name anybody, so a create carrying only an address is
refused exactly as one carrying nothing is.

`vpay_api::model`'s `the_customer_object_is_the_documented_nine_keys` is what
holds the count. It said _eight_ until 2026-09-10; `address` is the ninth.

## The rest of this flow

This page was 888 lines until 2026-09-11. What the object is, what is not on
the wire, the three decisions the maintainer took, and the Status section stay
here; the rest is below, moved **verbatim**:

- [customers/address-and-gps.md](customers/address-and-gps.md) — the address
  object, microdegrees and why there is no float anywhere, and "both or
  neither"
- [customers/privacy-and-erasure.md](customers/privacy-and-erasure.md) — no
  cross-merchant identity, the two shapes of `DELETE`, why an anonymised row is
  not a soft delete, **and the window the erasure does not close** — which was
  two until 2026-09-12, when issue #111 closed the first of them
- [customers/retention-sweep.md](customers/retention-sweep.md) — the
  twelve-month sweep, and why `customer.deleted` is the only way a merchant
  learns
- [customers/api-and-code.md](customers/api-and-code.md) — the routes, the
  three-state patch, tenancy, and where the code lives
- [customers/events.md](customers/events.md) — which transitions emit, and what
  deliberately does not

**The erasure page is the one with the caveats.** Its "One window the erasure
does not close, and one that used to be two" is the paragraph that bounds every
privacy claim this document makes. _(Both sentences above named "Two windows the
erasure does not close" until 2026-09-12 and went on naming it after issue #111
renamed the heading: `verify-links` checks a destination path and never a
heading, so a reference to a section that no longer exists is not a gate
failure.)_

### `last_used_at` is not on the wire

`customers.last_used_at` exists and is not a field. It is the retention
sweep's clock, vpay moves it whenever an intent or a session names the
customer, and a merchant who could read it would be building on a value whose
motion is vpay's business and whose meaning widens the day invoices exist.
`vpay_api::model`'s
`the_customer_object_is_the_documented_nine_keys` is the tripwire that keeps
it off — and `anonymized_at`, added by migration `0041`, is in the same list
of internals it refuses: adding it would put it in every `customer.*` webhook body, signed and
stored in `events` forever, before anybody wrote it down.

That tripwire **did not exist until 2026-09-07**, and this paragraph named it
anyway. Rendering `last_used_at` on the object was measured against the whole
repository on 2026-09-06 and nothing objected — `vpay-api`, `vpay-sdk`, the
container-backed cases in `backends/tests/integration/tests/customers.rs` and
the Node SDK were all green with the retention clock on the wire. The same
review found the count in this section was wrong: the table above lists eight
rows and the prose said seven.

---

## The three decisions the maintainer took (2026-09-05)

These are recorded decisions, not inferences, and each one is visible in a
specific place:

**1. Phone is the customer identity, and is stored by default.**
There is no opt-in, no configuration flag and no `store_phone` parameter. A
merchant who sends `phone` gets it stored and echoed back. On a mobile money
rail the phone number _is_ the payer, so an object that treated it as optional
extra data would be modelling a different market.

**2. Phone-only customers are allowed.**
`at_least_one_identifier` (migration `0034`) requires _one_ of the three, not
a name and not an email. A `cus_…` whose only content is a phone number is a
complete customer. `backends/tests/integration/tests/customers.rs`'s
`a_customer_with_only_a_phone_number_is_created_and_reads_back_canonical` is
the proof at the wire, and `postgres_smoke.rs`'s
`a_customer_with_no_name_email_or_phone_is_refused_by_the_database` proves
both halves at the database — the refusal _and_ that a phone-only insert is
accepted, which is the direction a "require all three" constraint would break
silently.

**3. Retention is twelve months.**
`vpay_worker::handlers::CUSTOMER_IDLE_AFTER`, and the `sweep_idle_customers`
job below.

### The phone number is stored canonical

`237600000200` — twelve digits, no `+` — whatever the merchant typed.
`+237 6 00 00 02 00`, `237600000200` and `600000200` all store and read back
as the same string, through
`vpay_api::v1::account_holders::canonical_msisdn`, which is the _same_
function that canonicalises an account-holder lookup.

That is a wire contract rather than an implementation detail, and it is the
reason the function is shared rather than copied: it is the value a rail is
given, so a merchant comparing a customer's `phone` against a charge's payer
reference is comparing the same string, and widening the market rule (a second
country code, a second mobile prefix) moves both surfaces at once. Neither SDK
validates it locally, deliberately and identically — a client-side copy of a
market rule refuses offline a number a later server accepts.

### …but the phone number is **not** a key, and does not deduplicate

"Phone is the customer identity" (decision 1) is a statement about what a
`cus_…` _means_, not about uniqueness, and the difference is worth spelling
out because the natural reading is the wrong one. Two `POST /v1/customers`
with the same phone number, under the same merchant, create **two customers**
with two ids. vpay does not look for an existing row and does not answer with
one.

That is Stripe's behaviour and it is deliberate here for the reason the next
section gives: the index that would make a phone number unique is the index
that would make cross-merchant identity possible, and there is no way to have
the first without either the second or a per-merchant partial index nobody has
asked for. The merchant owns the mapping from their user to a `cus_…` and is
the only party that can — vpay has no view of which of two rows is the "real"
payer.

What canonicalisation buys, then, is narrower than deduplication and is still
worth having: `+237 6 00 00 02 00` and `600000200` sent as _one_ customer's
phone across a create and a later update do not leave two spellings in the
column, and a merchant comparing a customer's `phone` against a charge's payer
reference is comparing the same string. It does not stop a merchant creating
the same payer twice. The `Idempotency-Key` on `POST /v1/customers` is what
stops a _retry_ doing so
(`a_replayed_create_answers_the_stored_customer_and_a_reused_key_is_refused`);
two deliberate creates are two customers.

---

## Status

**Updated 2026-09-23 ([ADR-0027](../adr/0027-erasure-reaches-through-checkout-sessions.md)):
an erasure now reaches a payment whose only link to the payer is a checkout
session.** Before vaam-apps/vpay#253, a session created with `customer=X` on
a customer-less intent stored `X` on the session only. Erasing `X` therefore
left that payment's `charges.payer_ref`, the rail's `failure_raw` on the
charge and its refund, and the intent's decline text. vpay#253 stops new rows
taking that shape and backfills nothing. The maintainer decided on 2026-09-23
that erasure must reach those payments through `checkout_sessions.customer_id`
as well. It does so, through `DELETE` and through the sweep, in the same
transaction. **It never reaches an intent that names a different customer.**
One consequence is stated rather than smoothed over: an old intent whose
sessions named two payers is redacted by either payer's erasure. See
[customers/privacy-and-erasure.md](customers/privacy-and-erasure.md) §
"Added 2026-09-23, later the same day" and
[the verification page](../status/verification/2026-09-23-erasure-through-checkout-sessions.md).
`customers.rs` is **twenty-seven** cases now, three more than the
twenty-four counted below, and the whole-database scan seeds the new shape.
Nothing here changes the list filters in the paragraph that follows.

**Updated 2026-09-23 (RFC-0004 § 5): a customer's payments, sessions and
refunds can be listed.** `customer=cus_…` filters
`GET /v1/payment_intents`, `GET /v1/checkout/sessions` and
`GET /v1/refunds` — the Stripe spelling of a customer-scoped read — each in
the same `WHERE` as `merchant_id`, which is the shape the `email` bullet
below names as the safe one. It is a filter by the merchant's own id and
not a lookup by a payer identifier, so it is not that bullet's gap closed:
**`GET /v1/customers` is still unfiltered**, and for the same reason. An
erased customer keeps its `cus_…`, and the filter still finds everything
that names it. [merchant-auth.md](merchant-auth.md) § Status has the
details.

**Resolved for sessions created from 2026-09-23 on, by
[ADR-0025](../adr/0025-session-customer-onto-intent.md).** A checkout
session created with `customer=X` on an intent that has no customer now
writes `X` onto the intent in the same transaction as the session insert. It
is a compare-and-swap on `customer_id IS NULL`
(`vpay_db::checkout_sessions::claim_intent_customer`). So its payment is
listed by all three filters, and a later session on that intent naming
another customer gets the `400` naming `customer`. Of two concurrent sessions
naming two customers for one such intent, exactly one is created. The other
gets the `400` a session naming a customer different from its intent's has
always got, byte for byte, or the one-open-session `409` if it read the
intent after the winner committed. Proven against a real Postgres by
`a_session_naming_a_customer_writes_it_onto_a_customer_less_intent`,
`two_sessions_naming_two_customers_for_one_intent_make_one_session_and_one_refusal`
and `a_session_naming_another_merchants_customer_writes_nothing_onto_the_intent`
(`checkout_sessions.rs`), and, for `POST /v1/invoices/{id}/pay`, by
`paying_an_invoice_writes_nothing_onto_its_intent_through_the_session`
(`invoices.rs`). Evidence:
[../status/verification/2026-09-23-session-customer-onto-intent.md](../status/verification/2026-09-23-session-customer-onto-intent.md).

**Still true of historical rows.** Intents whose session named a customer
before this change were **not** backfilled, so they keep no customer. Their
payment is listed by `GET /v1/checkout/sessions?customer=X` and not by
`GET /v1/payment_intents?customer=X` or `GET /v1/refunds?customer=X`, and
that customer's erasure does not reach the charge's `payer_ref`. ADR-0025
§ No backfill says why: a backfill would have to choose between sessions
that named different customers.

_(Until ADR-0025, the same day, this paragraph read: "**One consequence,
recorded rather than decided.** A checkout session created with `customer=X`
on an intent that has no customer stores `X` on the session only. The
payment it collects is therefore listed by
`GET /v1/checkout/sessions?customer=X` and **not** by
`GET /v1/payment_intents?customer=X` or `GET /v1/refunds?customer=X`, which
read the intent's column. The column each list compares is accepted in
ADR-0024 (D12 for sessions); whether a session's customer should be written
onto a customer-less intent is its **open question 3**
(`docs/adr/0024-customer-filters-and-manual-payments.md`, not yet on
`master`). Nothing here changes until that is answered.")_

**Built and proven against a real Postgres and the shipping router, worker and
SDKs (2026-09-06, S4a; extended 2026-09-10 twice — issue #66's events, then
issues #67/#68/#96's address and erasure — and again on 2026-09-11 with the
GPS half of the address).**
`backends/tests/integration/tests/customers.rs` is **twenty-four** cases (it
was twenty-three until 2026-09-12, when
[issue #111](https://github.com/vaam-apps/vpay/issues/111)'s race added one);
`postgres_smoke.rs` adds six at the schema; `vpay-db`'s own module adds
eleven with no container plus two container-backed ones for the transaction
seam.

**The idempotency window closed, 2026-09-12 (issue #111).** A
`POST /v1/customers/{id}` that committed, lost the race to a `DELETE` and only
then stored its response wrote the pre-erasure payer back into
`idempotency_keys.response_body`, where replaying its `Idempotency-Key`
re-read it for up to 24 hours. This section described it as an open window from
2026-09-11. It was reproduced before it was closed —
`an_update_that_loses_the_race_to_an_erasure_stores_no_payer_identifier` pins
the interleaving with row locks the test itself takes, and both erasure
branches run — and `vpay_db::Idempotency::store` now takes a
`ResponseSubject`: `Verbatim` for every other route, unchanged and one
statement, and `Customer { id }` for the three customer routes, which reads the
payer's row `FOR SHARE` and redacts the body it stored if that payer is gone.
The full argument, and the paragraph this replaced quoted verbatim, are in
[customers/privacy-and-erasure.md](customers/privacy-and-erasure.md) § "One
window the erasure does not close".

**The GPS half (2026-09-11).** `address` carries `latitude_microdeg` and
`longitude_microdeg`, integers, because "address in our system means both
formal as well as GPS". Round-tripped as a point with no street at all,
replaced and cleared with the rest of the address, refused as a decimal
degree, out of range or half a pair with a `400` naming `address`, and NULLed
by the erasure — with the marker CHECK refusing a row that kept either half.
Seven mutations were run against it, each against a real Postgres or a real
wiremock, and each is named in
[../plans/exp46-customer-address-notes/exp49-gps.md](../plans/exp46-customer-address-notes/exp49-gps.md).

**The retention promise is complete as of 2026-09-10; of the two windows
stated on 2026-09-11, one is closed (2026-09-12) and the other is a
contract.** Before migration `0041` a customer with payment
history could not be deleted, so the payer's `name`, `email` and `phone`
survived every "deletion" — and so did every copy in `events.data`,
`charges.payer_ref` and `idempotency_keys.response_body`, which no code named.
The review of 2026-09-11 found two more the same way the first four were
missed — `charges.failure_raw` and `refunds.failure_raw`, the rail's own words
about the payer — and they are redacted in the same transaction as the rest.
`an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table` is the
evidence: it scans every `text`, `varchar` and `jsonb` column
`information_schema` reports in `public`, finds five fixture literals in seven
named places before the `DELETE` and none anywhere after. Its third limit is
stated on `scan_for` itself and covered beside it: the two `BIGINT` coordinate
columns are outside any literal scan in principle, so they are asserted
directly, by name, as NULL.

**What is not built, and is a gap rather than a decision against it:**

- **Nothing erases a payer from a merchant's own copy**, and nothing can. The
  merchant received `name`, `email`, `phone` and `address` — the payer's
  position included — in
  `customer.created` and in every `customer.updated`, over signed bodies to
  endpoints they configured, before the erasure. vpay redacts _its_ stored
  copy of those bodies and cannot reach theirs. What vpay does do is **tell
  them** — `customer.deleted`, in the erasure's transaction, carrying the
  `cus_…` and the instant — and the review of 2026-09-11 declined to add a
  second `customer.redacted` type saying the same thing. What is missing is a
  contract obliging the merchant to act on it, which is a data processing
  agreement and a maintainer's decision. **It is still not taken.** The four
  options and what each costs are written out in
  [customers/privacy-and-erasure.md](customers/privacy-and-erasure.md) §
  "The decision, written out — NOT TAKEN" (2026-09-12, issue #111); nothing
  in this repository assumes any of them.
- **No `email` filter on the list.** Stripe's takes one. A filter on a payer
  identifier turns the list into a lookup, and a lookup by email over a table
  holding one merchant's payers is one scoping mistake away from being a
  lookup over everybody's. The shape that would make it safe — the filter in
  the same `WHERE` as `merchant_id`, as `checkout_sessions`' `payment_intent`
  filter is — is available whenever it is wanted.
- **Neither SDK has run against a live vpay for this resource.** Every server
  in their customer cases is a stub; the integration suite drives the real one
  but not through either SDK. Dated ⛔/⛔ in
  [../sdks/parity.md](../sdks/parity.md), exactly as the Checkout Session and
  account-holder rows are.
- **No deployment has ever run the retention sweep.** It is proven against a
  real Postgres through the real worker loop with a horizon this suite
  controls; no vpay has been up for twelve months.
- **An erased customer is still listed by `GET /v1/customers`**, which is a
  choice rather than a gap but is worth reading as one: a merchant paging
  their customers sees `[redacted]` rows. The alternative — filtering them
  out — makes the list deny a `cus_…` that `GET /v1/customers/{id}` answers,
  and makes `has_more` describe a different set from the rows. If a merchant
  wants them out, the shape is a `deleted` filter in the same `WHERE` as
  `merchant_id`, and nobody has asked.

**Updated 2026-09-12 ([issue #113](https://github.com/vaam-apps/vpay/issues/113)):
the write path now redacts what the read path already did, and the two open
questions about the coordinate are written up and left open.** Issue #113 asks
two things — whether the checkout page asks a payer for a location at all, and
whether a merchant reads back a coarser coordinate by default — and says
"Decision needed" for both. **Neither is taken**, nothing on that branch moves
either one, and the write-up that prepares them is
[../plans/issue-113-notes/decision.md](../plans/issue-113-notes/decision.md):
the options, what each costs, what "coarser" means in metres at Cameroon's
latitudes, and a recommendation each, labelled as recommendations. The current
state it establishes is in
[customers/address-and-gps.md](customers/address-and-gps.md) § "Two questions
this shape does not answer". What **was** built is the one defect that is gated
on neither: `vpay_db::{CustomerAddress, NewCustomer, CustomerPatch}` still
**derived** `Debug`, so a `{:?}` on the values `insert_in_tx` and
`update_in_tx` hold — the frame an `anyhow` chain or a `tracing` event carries
when a CHECK fires — wrote the payer's name, email, phone, street and GPS point
out in full. That is the same hole #70 closed one layer up, left open on the
way **in**; `CustomerRow`'s guard did not cover it, and in fact depended on it,
since the row counted the address components itself rather than delegating.
All three have hand-written impls now, the count lives in one place per crate,
and `no_customer_type_ever_prints_a_payers_identifiers_street_or_gps_point` in
`vpay-db` asserts all four types at once — negatively, on seven fixture
literals the coordinate's digits included, and positively, so that an impl
printing nothing fails too. Restoring any of the four derives is `E0119`, a
`cargo check` failure rather than a test to keep green. Five mutations, and
what was and was not run, are in
[../status/verification/2026-09-12-customer-debug-redaction.md](../status/verification/2026-09-12-customer-debug-redaction.md)
— that page's first claim is that **`just ci` was not run on this branch**.
The review of the same day carried the fix the rest of the way in:
`vpay_api::v1::customers`' request types (`CreateParams`, `UpdateParams`,
`AddressParam`/`AddressParams`, `ValidCreate`) derived `Debug` too, and
`AddressParams` holds the coordinate as the **string the wire sent**, before
`checked_microdeg` parses it. Five more hand-written impls and
`no_customer_request_type_ever_prints_a_payers_identifiers_street_or_gps_point`
close it; four mutations are on the verification page.

**One thin spot named and not closed (2026-09-12).** The twelve-month sweep's
own case,
`the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one`,
builds its fixtures with a `name` and no address, so **no test drives a
coordinate through `erase_idle`**. The property holds transitively — the sweep
runs the same `anonymize` statement as `DELETE`, and
`anonymized_customers_carry_the_marker` would raise a `23514` and roll the
transaction back — so this is a thin fixture rather than an unproven property.
The fix is to give the anonymised fixture a point and assert both columns
`NULL` after the sweep, exactly as
`a_customer_with_payment_history_is_anonymised_rather_than_deleted` does.

**Updated 2026-09-11 ([issue #70](https://github.com/vaam-apps/vpay/issues/70)):
`CustomerObject`'s own `Debug` is hand-written, and a checkout session's
`customer` can now be sent, not only read.** Nothing formatted a
`CustomerObject` before this change, but `#[derive(Debug)]` on a type
carrying a payer's name, email, phone, postal address and GPS point (the
address and coordinates landed the same day, in the GPS paragraph above) was
a redaction with a hole waiting for the first `{:?}`. The hand-written impl
redacts `name`/`email`/`phone` as character counts — the judgement
`vpay_db::CustomerRow`'s own `Debug` already makes, for the same reason — and
delegates `address` to `AddressObject`'s own (also hand-written) `Debug`,
which redacts every component and the GPS pair as a count rather than a
value. A first pass at this issue redacted the three identifiers and left
`address` on a derived `Debug`, printing the payer's street and coordinates
in the clear; `a_customer_object_debug_output_redacts_personal_identifiers`
now builds a fixture with a full address and both coordinates, and a
mutation restoring the leak fails it. Re-adding `#[derive(Debug)]` to either
type is not a test to keep green — it is `E0119`, a `cargo check` failure,
confirmed by mutation. Separately, `CreateCheckoutSessionParams` gained a
`customer` field in both SDKs, wired into the request body: the field
existed on the response type in both SDKs since 2026-09-06, but nothing sent
one, and a first pass at this issue described the capability as "already
implemented" on the strength of the read half alone. The parity row in
[../sdks/parity.md](../sdks/parity.md) is now ✅/✅.

### Correction, 2026-09-07 (S4b)

This section listed **Invoices** as a gap until 2026-09-07: "the definition of
'used' above names two referencing objects and would name three. Invoices do
not exist". [Migration `0036`](invoices.md) is when that stopped being true.
`invoices.customer_id` is a **required** `NO ACTION` foreign key, so a customer
with any invoice cannot be row-deleted, and
`vpay_db::customers::UNREFERENCED` grew its third `NOT EXISTS` in the same
commit — with `the_sweep_guard_names_every_table_that_can_reference_a_customer`
moved from two to three, which is the assertion that made the omission
impossible to ship.

**Second correction, 2026-09-10.** That paragraph said an invoiced customer
"cannot be deleted at all", and until migration `0041` it could not: the sweep
skipped it and `DELETE` answered `409`. It is now **anonymised** instead. The
third `NOT EXISTS` is still load-bearing and what it guards is worse than
before — without it the erasure takes the hard-delete branch, the foreign key
raises `23503`, and the whole transaction rolls back, leaving the payer
un-erased with the merchant told nothing.
`an_invoiced_customer_is_anonymised_rather_than_deleted` (renamed from
`an_invoiced_customer_is_never_offered_to_the_sweep`) is the test.

Creating an invoice also **stamps** the customer's retention clock, exactly as
creating an intent or a session does, so a merchant who bills a payer monthly
never has that payer swept.

Without the third clause nothing would have broken loudly: the foreign key
would still refuse the delete, and the sweep would simply _offer_ a customer it
can never remove — minting an `evt_…` and building a `customer.deleted` object
for a payer whose record is not going anywhere, once an hour, forever.
`an_invoiced_customer_is_never_offered_to_the_sweep` is the test, and deleting
the clause is what makes it fail.
