# `vpay-db` — `customers` and `invoices`

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## `customers`

The first vpay table **born** with a `schemas/vpay.cstack` model rather than
acquiring one afterwards, and the only one whose whole content is another
person's personal data. [`../flows/customers.md`](../../flows/customers.md) is the
product document; this section is why the code is shaped the way it is.

### Two of eight methods go through CrateStack, and one column decides which

| Method                                                                                                                                                                                |                                                  |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `touch_last_used`                                                                                                                                                                     | **CrateStack** — `update_many(..).set(..)`       |
| the hard-delete branch of `erase_in_tx`                                                                                                                                               | **CrateStack** — `delete_many(..).run_in_tx(..)` |
| `insert_in_tx`, `lock_for_update`, `update_in_tx`, `get_for_merchant`, `list_page`, `idle_since`, `erase_idle`, and the anonymise branch and six trailing statements of `erase_in_tx` | hand-written `sqlx`                              |

_(The table read `create`, `update` and "two of seven" until 2026-09-10. The
two pooled writers are gone: `POST /v1/customers` and
`POST /v1/customers/{id}` each emit an event that must commit with the row, so
the statements moved behind `TxRepositories` and a third joined them —
`lock_for_update`, the `SELECT … FOR UPDATE` the update's `metadata` merge is
computed under. See "The customer writes are transactional" below.)_

The column is `metadata JSONB NOT NULL`, undeclared on `model Customer` for
the two costs that model's GAP note measures: `map_scalar` does not read
`jsonb` back, so declaring `metadata Json` would make the live column stay
invisible while the _declared_ one became a `[blocking]` drift line; and
`Value::from_plain_json` demotes any JSON number outside `i64` to `f64`,
silently, on a column that is merchant-authored and echoed back verbatim.

**That second cost is also what decided the shape of the GPS half of the
address** (2026-09-11, "address in our system means both formal as well as
GPS"). A coordinate had to be something this stack can carry exactly, and
`from_plain_json` routes every JSON number through `Number::as_i64()` — so a
decimal degree is a value CrateStack rounds. `address_latitude_microdeg` and
`address_longitude_microdeg` are therefore `BIGINT` counts of millionths of a
degree, declared on `model Customer` as `Int?` with a `@range` mirroring
`0041`'s CHECKs, and the unit is in the column name so no layer can read the
number as degrees. It is the money layer's rule — integer minor units with the
scale named — applied to the other quantity vpay stores at a fixed scale. The
[customers flow doc](../../flows/customers.md) carries the product half.

What that model **cannot** say is the pair rule:
`address_coordinates_are_both_or_neither` is multi-column, this grammar has no
cross-field validator, and so the only places the rule exists are migration
`0041` and `vpay_api::v1::customers`' `400`. Two columns and three constraints
cost exactly **two** drift lines — the two single-column range CHECKs, which
are reported once each as `[safe] … is not declared` exactly as the six
`address_*_length` bounds are; the columns themselves and the multi-column pair
rule cost nothing. `EXPECTED_DRIFT_CHANGES` 177 → 179, measured.

The consequence is not obvious and is worth stating: because the model does
not declare the column, the generated model _struct_ has no field for it
either, so a CrateStack **read** could not render the wire object at all.
Every operation that touches `metadata` in either direction — create, update,
and every read — is therefore a hand-written statement, which is five of the
seven.

That is not a consolation prize. The two that did move are the two where being
wrong is irreversible: `touch_last_used` is the write the twelve-month
retention sweep reads, and `delete_many` is the hard delete of personal data.
It stayed on CrateStack through migration `0041` deliberately, even though its
caller became a `pub(crate)` function inside a larger transaction, because
`a_customer_delete_is_a_delete_and_not_a_soft_delete` pins the _rendered_
statement — and adding `@@soft_delete` to `model Customer` is a one-line
schema edit that compiles, passes `check-schema`, and turns every erasure into
a flag on a row that keeps the payer. Both
policy failures are **silent** — `update_many` and `delete_many` compile the
`@@allow` into the statement's own `WHERE`, so a deleted arm matches zero rows
and returns `Ok` — which is why `every_action_this_module_calls_has_an_allow_arm`
asserts the compiled descriptor with no container, and why it also asserts the
_absence_ of the three arms this model deliberately does not grant.

### `list_page` and the sweep stay raw SQL, and it is the query shape

`list_page`'s cursor is `seq < (SELECT seq FROM customers WHERE id = $2 AND
merchant_id = $1)` — a correlated subquery, which `cratestack::Filter` has no
constructor for. The two-statement alternative (resolve the cursor id to a
`seq`, then filter on the literal) is a _different_ query with a race in it:
between the two reads the cursor row can be deleted, and this table's rows
**are** deleted, by both `DELETE /v1/customers/{id}` and the retention sweep.
The one-statement form degrades to an empty page there.

`erase_in_tx` reads a `NOT EXISTS` triple over `payment_intents`,
`checkout_sessions` and `invoices` — correlated subqueries over _different_
tables, which `Filter` compares nothing of, and for which there is no
`@relation` to side-load because none of those tables is modelled.

It is one `const UNREFERENCED`, and it is a constant because `sql_audit` made
it one: it was first written as a function taking the outer query's alias, and
the gate failed on the computed `{guard}`. Both call sites spelled the table
`customers`, so there was no alias to parameterise.

**It stopped being a guard and became a branch on 2026-09-10** (migration
`0041`, issue #68). It used to be in `idle_since`'s `WHERE` as well, so a
customer any of the three tables referenced was skipped by the sweep and
refused by `DELETE /v1/customers/{id}` — which exempted from the twelve-month
retention promise exactly the payers vpay had taken money from. It now selects
between hard-deleting the row and anonymising it, and what a missing table
costs got worse: the branch would choose the hard delete, the `NO ACTION`
foreign key would raise `23503`, and the whole erasure — event and four
redaction statements included — would roll back, leaving the payer un-erased
with the merchant told nothing.
`the_sweep_guard_names_every_table_that_can_reference_a_customer` pins the
count at three, and
`an_invoiced_customer_is_anonymised_rather_than_deleted` is the behavioural
half.

### The erasure is seven statements in one transaction, and six of them are not about `customers`

`erase_in_tx` is the whole of issue #68, and the shape is what the issue got
wrong. The issue asks for identifiers to be redacted "on retained intents and
sessions"; an intent has never carried one. The copies that survived a
customer deletion were `events.data` — every `customer.*` body ever written,
in a table nothing prunes — `charges.payer_ref`/`payer_ref_masked`, reachable
from a customer only _through_ an intent, and
`idempotency_keys.response_body`, the exact JSON a `POST /v1/customers`
answered. The sabotage review of 2026-09-11 added two more found the same way:
`charges.failure_raw` and `refunds.failure_raw`, which are not identifier
columns and hold identifiers anyway — the rail's message kept verbatim, and a
rail refusing a collection names the subscriber. All five are rewritten in the
transaction that erases the row, because "vpay erased this payer" may not be
true of one table and false of four.

The seventh statement is not about a copy of the payer at all: rewriting
`events.data` changes the bytes a webhook delivery already mid-ladder would
re-render, and `webhook_deliveries.payload_sha256` is the digest the first
signed attempt recorded. So the deliveries of those events that are still
`pending` or `failed` have that column cleared in the same transaction, and
the next attempt signs and sends the redacted body. Without it the erasure
dead-letters exactly the delivery that announces it — measured, not argued,
in `an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering`.

The `events` and `idempotency_keys` statements share one `const
REDACT_CUSTOMER_KEY` — a `CASE` over `jsonb_each`'s key, naming the four
identifier keys and passing everything else through, so `metadata` (the
merchant's data) and `id` survive.

**The `address` arm replaces the whole key rather than walking into it**, and
since 2026-09-11 that is what makes the erasure reach a payer's GPS point
inside `data.object.address`. The replacement is `redacted_address_json()`,
which has to be key-for-key what `vpay_api::model::AddressObject` renders —
a stored body this rewrites is read back by a merchant on a replay or a
redelivery, and a key here the object does not have is a shape they meet
there and nowhere else. Its six formal components carry the marker and its
two coordinate keys carry JSON `null`, for the reason the column does: a
coordinate is an integer, the marker is not a value it can take, and a string
in a field both SDKs decode as an integer fails the decode rather than reading
as redacted. `the_redacted_address_body_is_the_shape_the_wire_renders` pins
the key set and the per-type value in milliseconds; the container-backed
scanner cannot tell an `address` reduced to `{}` from a redacted one, because
an absent key holds no identifier either. They cannot share the whole statement:
one reads `events.data` and the other `idempotency_keys.response_body`, and
`sql_audit` refuses a computed fragment, so the `FROM` differs and only the
rule is shared. That is the half that could drift, and the half a test can
read: `the_event_redaction_names_the_identifiers_and_spares_the_merchants_data`
asserts both directions of it in milliseconds.

`provider_requests` needs no statement, and that is a property of migration
`0016` rather than an omission: it stores a status code, an attempt number and
an operator-facing `error_kind` and no bodies at all. `webhook_deliveries`
holds no copy either — `0022` keeps a `payload_sha256` and not the payload —
and gets its statement for the opposite reason to a leak, above.

### The customer writes are transactional, and there is no pooled variant

`POST /v1/customers` emits `customer.created` and `POST /v1/customers/{id}`
emits `customer.updated` (2026-09-10, issue #66). Both events have to commit
with the row or not at all, so both statements live behind `TxRepositories`
and the pooled `Customers::create`/`Customers::update` were **deleted**. That
deletion is the load-bearing part: leaving them beside the transactional ones
would have kept "write the row and tell nobody" one call away, and no gate in
this repository objects to a method nobody happens to call.

`vpay-api` opens the transaction rather than `vpay-db`, for the reason
`insert_invoice_in_tx` gives: the event's `data` is the **wire object**, whose
shape this crate does not know, and it has to be rendered from the row the
statement returned — `seq` and both timestamps are the database's, so a
projection of the request would be a second implementation of the insert.

`lock_for_update` is the third method and the one worth reading the argument
for. `metadata` is merged key-wise in `vpay-api` (Stripe's contract), so the
written value is a function of the stored one and the update is a
read-modify-write. Reading on the pool left a window in which two concurrent
updates each adding one key lost one of them — documented, and tolerable while
nothing depended on the result being definite. An event is exactly such a
dependency: a merchant acting on a body describing the losing merge acts on a
state the database does not hold. So the read takes the row lock and the whole
sequence is one transaction. `a_locked_customer_read_waits_for_the_writer_and_then_sees_its_value`
forces the interleaving at this seam rather than racing for it, and
`a_customer_write_and_its_event_roll_back_together` is the abandon case.

The merge itself stays in Rust rather than becoming a `jsonb ||` in the
statement, deliberately: that would move Stripe's semantics into a migration
and out of the layer that documents them.

### `touch_last_used` is monotonic by _filter_, not by `GREATEST`

The method's contract is "a stamp never moves a customer's clock backwards",
and the obvious statement for that is `SET last_used_at = GREATEST(last_used_at,
$2)`. `UpdateCustomerInput` renders a plain assignment and cannot express it.

So the guard is the **predicate** instead: `where_(last_used_at.lt(now))`
means a stale stamp matches zero rows and is a no-op, which is the same
observable behaviour. `Ok(false)` therefore covers one more normal case —
"already stamped at or after this instant" — and the method's doc says so.

Why it matters at all: `now` is the _calling process's_ instant, two vpay
processes do not share a clock, and the horizon is twelve months. A rewind is
not a rounding error; it is the difference between a customer surviving a
sweep and not. It is also why migration `0034` carries **no**
`CHECK (last_used_at >= created_at)`: both instants come from process clocks,
so a server a second behind would turn a `POST /v1/payment_intents` into a
`500` for a reason no merchant could act on.

### The chrono/time boundary, crossed a second time

`to_chrono` is the second place this crate crosses it and the first in that
direction — `client_assertion::chrono_to_offset_date_time` converts the other
way for `authkestra-op`. CrateStack's generated inputs and filters take
`chrono::DateTime<Utc>`; every TIMESTAMPTZ this crate binds by hand is
`time::OffsetDateTime`.

The conversion has an `unwrap_or` that cannot be reached: chrono's range is
roughly ±262,000 years and `time::OffsetDateTime`'s (default features) is
years −9999..=9999, four orders of magnitude narrower.
`the_chrono_conversion_is_total_over_every_instant_time_can_hold` proves that
at both extremes rather than asserting it in prose, so a future `time` with
`large-dates` enabled turns the branch red instead of silently clamping.

ADR-0007 denies `expect`, so the unreachable branch still answers something,
and _which_ answer is not arbitrary: `MAX_UTC` means "freshly used" in both
slots this function feeds, so the branch fails in the direction that **keeps**
a merchant's personal-data record. `UNIX_EPOCH` would do the opposite — stamp
a live customer as maximally idle and hand it to the next sweep.

## `invoices`

The merchant's bill to a payer and the lines it is made of
(`backends/migrations/0036_create-invoices.sql`, S4b). Twelve repository
methods over two tables, plus three that are `TxRepositories` methods and one
that is `pub(crate)` to `settlement`.
[`../flows/invoices.md`](../../flows/invoices.md) is the product document; this
section is the persistence argument.

### Two of twelve go through CrateStack, and three separate things decide it

`customers`' split had **one** cause (`metadata JSONB`). This family has
three, and each method's doc names the one that applies to it, because
collapsing them into "CrateStack cannot do it" would be false for two thirds
of the surface:

1. **`invoices.metadata` is `JSONB NOT NULL` and undeclared.** `model
Customer`'s two measured costs, unchanged — `map_scalar` does not read
   `jsonb` back, so declaring it would be a `[blocking]` drift line; and
   `Value::from_plain_json` demotes any JSON number outside `i64` to `f64`, on
   a column that is merchant-authored and echoed back inside every `invoice.*`
   webhook body. Every **read** of `invoices` has to carry that column, so
   every read stays raw.
2. **Three transitions write an `events` row in the same transaction**, and
   `Events`' insert is itself blocked on `events.data` (see "`events.data`:
   the one write that did not move").
3. **Every `invoice_items` write guards on a different table's column** —
   `EXISTS (SELECT 1 FROM invoices WHERE id = invoice_items.invoice_id AND
status = 'draft')`, which is what freezes an issued document.
   `cratestack::Filter` compares columns of the model's own table, and there
   is no relation that side-loads a parent's status into a child's `WHERE`.

What is left is exactly the two in the table above.
`mark_uncollectible` is the **only** transition with no event — Stripe's
`invoice.marked_uncollectible` is deliberately outside migration `0036`'s
vocabulary because nothing would write it — so it is the only one that is a
single statement, and a compare-and-swap `update_many` expresses the whole
operation. `items_for_invoice` goes through the generated layer because
`invoice_items` was **shaped** so it could: no `jsonb`, no `bytea`, no native
enum, no `int4`, which is migration `0035`'s discipline applied to a table that
also had to be readable through this layer.

### The transactions are opened by `vpay-api`, not here, and that is a departure

`settlement` and `Customers::erase_idle` both take a caller-rendered
`event_data` parameter and open their own transaction. The three invoice
transitions that emit an event do the opposite: they are
[`TxRepositories`] methods, so `vpay_api::v1::invoices` opens the transaction,
gets the written row back, renders the wire object **from that row**, and
appends the event with `insert_in_tx`.

The reason is specific rather than stylistic. Those two describe an object
whose post-write shape the caller can _project_ exactly — an intent that is
about to be `succeeded`, a customer that is about to be deleted. **A finalized
invoice cannot be projected**: its `number` comes out of a sequence the
statement itself advances and its `amount_due` is summed by the statement from
the lines, so a projection would be a second implementation of the assignment,
and the first thing it would get wrong is the number.

`invoice.paid` is the exception that proves it, and it is projected — see
below.

### The number sequence is a table row, and that is the whole design

`invoice_number_sequences` has no `.cstack` model, deliberately: its only write
is `next_number = invoice_number_sequences.next_number + 1`, a `SET` whose
right-hand side names the column being set, and `Update{Model}Input` carries
values rather than expressions. That is the same shape
`Customers::touch_last_used` wanted `GREATEST` for and could not have. A model
would declare a table nothing could write through, which this schema's header
says it does not do.

`INSERT … ON CONFLICT (merchant_id) DO UPDATE SET next_number = … + 1
RETURNING prefix, next_number - 1` does three things in one statement: it
creates the merchant's sequence on their first finalize (the row is born
pointing at 2 and that finalize takes 1), it advances it on every later one,
and it **takes the row lock that serialises two concurrent finalizes**. Under
`READ COMMITTED` the blocked transaction re-reads the committed value when the
lock is released, so two racing finalizes get two consecutive numbers.

It is not a Postgres `SEQUENCE`, and that is the point rather than an
oversight: `nextval` is non-transactional, so a rolled-back finalize would
burn a number. Migration `0036` argues why a hole matters more here than it
does for Stripe.

### `invoice.paid` in TX1, and why _this_ one is projected

`invoices::mark_paid_for_intent_in_tx` is `pub(crate)` and reached only from
`settlement::flip_invoice` — `checkout_sessions::settle_for_intent`'s
visibility argument verbatim: the point is that the write is not reachable
without the settlement it belongs to.

The worker projects the paid invoice before the transaction opens
(`vpay_worker::handlers::invoice_snapshot`), which is `intent_snapshot`'s
device. The projection is exact because an `open` invoice with a live intent
**cannot change**: `amount_due` was frozen at finalize, its lines are
immutable, and both `void` and `mark_uncollectible` refuse while the intent is
uncanceled (`NO_LIVE_INTENT`). If it changes anyway, the compare-and-swap
inside the transaction matches nothing and **no event is written at all** —
the fail-closed direction, pinned by
`a_settlement_whose_invoice_moved_emits_no_invoice_event`.

### The refund settlement, and why it is a second transaction rather than a hook

`Settlement::apply_refund_succeeded` (issue #91 D5, migration `0042`) opens one
transaction and runs two statements in it:

1. `refunds::settle_in_tx` — `UPDATE refunds SET status = 'succeeded' WHERE id
= $1 AND status = 'pending'`. `Ok(None)` is "already settled", the answer an
   at-least-once retry has to get.
2. `invoices::add_refund_for_intent_in_tx` — `UPDATE invoices SET
amount_refunded = amount_refunded + $2 WHERE payment_intent_id = $1 AND
status = 'paid'`. `Ok(None)` is "this intent pays no invoice", which is most
   of them.

Both are `pub(crate)`, so a consumer of this crate cannot settle a refund
without the document update in the same commit — `mark_paid_for_intent_in_tx`'s
visibility argument, applied to the other direction of the money.

**The increment is an expression, not a value the caller computed.** A
read-then-write would let two refunds settling concurrently both read `0` and
both write their own amount, losing one. The expression makes the second writer
block on the row lock MVCC already takes and re-evaluate against the first's
committed value — which is also what makes migration `0042`'s
`refunded_at_most_paid` a real ceiling rather than an advisory one.

**It fails closed, and the abandon test is the over-refund.** A refund past
`amount_paid` trips the CHECK on statement 2, after statement 1 has flipped the
refund inside the same transaction; everything rolls back and the refund is
still `pending`. Move the invoice write out — commit the flip, update on the
pool — and
`two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` goes red
with the refund `succeeded` and the invoice unmoved, which is the permanent
inconsistency the transaction exists to prevent.

**No event.** `invoice.paid` is not re-emitted (the invoice did not
transition), and `charge.refunded` / `charge.refund.updated` stay types nothing
writes: emitting one needs the wire object `vpay-api` shapes, which is the
caller's to supply, and there is no caller.

**There is no caller at all, and that is the honest part.** No rail can refund,
`POST /v1/refunds` is unrouted and `Refunds` still exposes no `create`, so
nothing in `vpay-server` reaches this method and every deployment's
`invoices.amount_refunded` is `0`. It exists because D5 is a decision about
what the database does when a refund lands, and the alternative was to leave
that decision as a sentence in a document. `docs/status.md` carries the gap.

`payment_intents.amount_refunded` and `amount_refund_pending` (migration
`0003`) are deliberately **not** maintained here. Migration `0017`'s own GAP
note pairs them with the `INSERT` that creates a `refunds` row, and that insert
does not exist; incrementing one half of a paired total whose other half
nothing writes would leave `no_over_refund` counting money twice the day the
insert lands.

### `NO_LIVE_INTENT` is one rule with three call sites, and one of them cannot carry it

Voiding an invoice somebody is paying, writing it off, and minting a _second_
intent for it are three ways to end up with money moved against a document
that says nothing is owed. All three refuse on the same condition — the
attached intent is `canceled` — spelled once as a `const` so they cannot drift.

Two of them carry it in SQL. `mark_uncollectible` cannot: it goes through
CrateStack, and the condition is a correlated sub-select over
`payment_intents`. So `vpay-api` applies it from the row it has already read,
and the window that leaves is closed by **the settlement's own
compare-and-swap** rather than by the statement: a settlement that lands first
flips the invoice to `paid`, and `update_many`'s `status = 'open'` filter then
matches nothing. Neither ordering can produce an invoice that is both paid and
written off.

That is the one place in this module where a guard is not in the statement it
guards, and it is stated here for that reason.

### Migration 0042: what `amount_refunded` costs

**+1 change, 173 over 24, unmappable still 19.** Two of that migration's three
additions move nothing and the third is the whole of the +1:

- the **column** costs zero. `BIGINT`, no DEFAULT, declared in `model Invoice`
  as a plain `amount_refunded Int`, so both sides compare equal. The ADD's
  backfill DEFAULT is dropped on the very next line for exactly this reason —
  left in place it would have been a permanent
  `column amount_refunded default value differs` line, migration `0033`'s
  problem.
- `refunded_at_most_paid` costs zero because it is **multi-column**, and
  `migrate baseline` skips every multi-column CHECK in both directions. It is
  the load-bearing half of the migration and the report cannot see it at all,
  which is why it is in `postgres_smoke.rs`'s live-database CHECK inventory
  instead.
- `amount_refunded_non_negative` is the +1: single-column and hand-named, so
  one `[safe] … exists in the live database but is not declared in the schema`
  line, exactly like its six siblings on this table.

### Migration 0036: what the two tables cost

**156 changes over 23 relations, unmappable 19** (from 130 / 20 / 18). The +26
splits sixteen / nine / one across `invoices`, `invoice_items` and
`invoice_number_sequences`, and every line is a hand-named CHECK, an
undeclared index, a `seq` identity default, or the one permanent `status` type
line.

**`invoices_status_enum_check` costs nothing, and it is the first enum column
in this repository that does.** Migration 0032 had to _rename_
`providers.flow`'s hand-named CHECK after the fact, because `diff/checks.rs`
matches by name first; 0036 creates the constraint under
`naming.rs::check_name(table, column, "enum")`'s own spelling from the start,
so the declared constraint and the live one are one object and neither side
reports the other missing. The `[lossy] column status type differs (live:
Scalar("String"), schema: Enum("InvoiceStatus"))` line is still permanent, for
`introspect/postgres/enums.rs`' documented lossiness — a TEXT column has no
catalog representation that recovers an enum's name.

The five multi-column CHECKs contribute **nothing in either direction**, like
the fifteen before them, which is why
`the_invoice_invariants_are_enforced_by_the_database_itself` writes the row
each one refuses against a real Postgres.
