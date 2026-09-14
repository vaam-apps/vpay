# Lane D — the four slices, and the four things that would leak if nobody checked

_Read from `schemas/vpay.cstack` and the 0.12.0 macro sources on 2026-09-13,
before any procedure was declared. Each item below is a property of the data
model that decides the shape of a `procedure search*`, and three of the four
are only visible if you read the model rather than the table name._

## 1. `@sensitive` does not protect anything over the wire

`cratestack-macros`' `is_sensitive_field` is read in exactly one place: to
**redact a value from the audit log** (`shared/attrs.rs:64`). It does not
exclude a field from a response, does not mask it, and does not stop a
procedure's summary type from naming it.

So `Customer.phone`, `.email`, `.name` and the six address columns — all
`@sensitive` — are returned **in full** by any summary type that lists them.
Anyone reading the schema and concluding "these are protected" is wrong, and
that is the most likely wrong conclusion available here.

The dashboard's customers list therefore has to decide, explicitly, which
identifiers an operator sees, and say so. It is not decided by the schema.

## 2. Two of the four tables have no `merchant_id` at all

| Model             | Tenancy column | How a tenancy predicate reaches it                |
| ----------------- | -------------- | ------------------------------------------------- |
| `Event`           | `merchant_id`  | directly                                          |
| `Customer`        | `merchant_id`  | directly                                          |
| `Refund`          | **none**       | `payment_intent_id` → `PaymentIntent.merchant_id` |
| `WebhookDelivery` | **none**       | `event_id` → `Event.merchant_id`                  |

`searchPaymentIntents` gets to write `WHERE merchant_id = $1` because its
table has the column. **Refunds and webhook deliveries cannot**, and a body
that forgets this does not fail — it returns every tenant's rows. The join is
the tenancy predicate, and the mutation that proves it fired is the same one
`search_payment_intents.rs` documents: replace the predicate with a tautology
and assert another merchant's row appears in this merchant's page.

## 3. An anonymised customer must not come back

`Customer.anonymized_at` exists because erasure is implemented (issue #111,
closed in #143 — the 24-hour window). A customers list that ignores that
column re-exposes, through a new surface, records the erasure path already
answered for. Decide what an erased row looks like in a list — absent, or
present as a tombstone with no identifiers — and test it. Absent is not
automatically right: an operator counting customers may need the row to exist.

## 4. Fields that must not enter a list

- `Refund.failure_raw` — `@sensitive`, up to 2 000 characters of a rail's own
  prose. `searchPaymentIntents` already refuses the equivalent
  (`last_payment_error_code` is in the summary, the raw text is not) and its
  comment says why: "a list is not where it is read".
- `WebhookDelivery.response_excerpt` — same shape, up to 2 000 characters of
  a merchant endpoint's response body.

Both belong to a detail read if they belong anywhere.

## The rules that apply to every slice

Carried from `docs/reference/vpay-db/cratestack.md` and from
`searchPaymentIntents`' own body, which is the template:

- **The `search` prefix is mandatory.** Every model generates
  `handle_{list,create}_<plural>` and `handle_{get,update,delete}_<singular>`
  whether routed or not, so `listRefunds` collides with the generated `list`
  CRUD handler and fails to compile as `error[E0428]`.
- **A generated model read answers zero rows** on these tables. `Refund` and
  `Customer` carry no `@@allow("read", …)` arm, and a model with no arm is
  deny-by-default: the read renders `FALSE` into its `WHERE`. The procedure
  body is ours; that is what a procedure is for.
- **Name the filter fields.** `FindMany<Model>` publishes a filter over every
  filterable scalar, which on `PaymentIntent` was rejected because it would
  have exposed `startsWith`/`lt`/`gt` on `client_secret_suffix` — a prefix
  oracle on a live payer credential. Hand-roll the filter type.
- **The tenancy predicate comes from `ctx.tenant_id()`, never from the
  arguments**, and its absence is a refusal rather than an unscoped read.
- **Each slice is container-tested against real Postgres before it is
  claimed.** A unit test cannot observe a predicate; only the rows it
  excluded can.
