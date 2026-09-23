# ADR-0025: A checkout session's customer is written onto a customer-less intent

- **Status:** Accepted
- **Date:** 2026-09-23
- **Deciders:** the vpay maintainer, who chose on 2026-09-23 to "do"
  [ADR-0024](0024-customer-filters-and-manual-payments.md)'s question 3
  rather than leave it open. The rules below (D1–D6) were set out in the brief
  for this change, and nothing in them was re-decided while building it.
- **Answers:** ADR-0024 § "Left to the maintainer", question 3, and the
  "consequence" paragraph under its D12. **ADR-0024 is not edited.** ADRs here
  are immutable, so its question 3 still reads "Still open". This ADR is the
  answer, and every page that repeated the consequence now points here.
- **Implementation:** the same change as this ADR (branch
  `feat/session-customer-onto-intent`). The `docs/status/` rows and
  [the verification page](../status/verification/2026-09-23-session-customer-onto-intent.md)
  say what is built and proven. This ADR records the decision and not what
  works.
- **Number checked at branch time, not assumed:** `ls docs/adr` on
  `origin/master` `b747e5d5` returns `0001`…`0024`, 24 files for 24 numbers.
  `gh pr list --state open` on 2026-09-23 returns #252 (the justfile) and
  #250 (the release). Neither touches `docs/adr/`.

## Context

ADR-0024's D12 has checkout sessions filter on their **own**
`checkout_sessions.customer_id`, intents on `payment_intents.customer_id`, and
refunds on their intent's. Before this change the two columns could differ,
and only in one way. That was established by reading
`vpay_api::v1::checkout_sessions::prepare_create` and
`vpay_db::CheckoutSessions::create` on `b747e5d5`:

- A session with **no** `customer` copies the intent's, so the two agree.
- A session naming a customer **different** from the intent's is refused with
  a `400` naming `customer`.
- A session naming a customer **on an intent with none** stored the customer
  on the session row only. `create` was one `INSERT` into
  `checkout_sessions` and never wrote `payment_intents`.

The third case had three effects. Two were already known, and the third was
found while writing this ADR.

1. **The filters disagreed.** The payment that session collected was listed
   by `GET /v1/checkout/sessions?customer=X`, and not by
   `GET /v1/payment_intents?customer=X` or `GET /v1/refunds?customer=X`.
2. **One intent could have sessions naming two payers.** The reviewer's
   belief is **confirmed**. `checkout_sessions_one_open_per_intent` allows
   only one _open_ session per intent. Once the first expires, by the
   merchant or by the 24-hour sweep, a second can be created. Its `customer`
   was compared with the **intent's**, which was still `NULL`, so naming
   `Y` after a session had named `X` was a `201`. The sequential version
   needs no race at all.
3. **Customer erasure did not reach that payment.** Erasure redacts
   `charges.payer_ref` (the payer's MSISDN) for charges reached **through
   `payment_intents.customer_id`**
   (`vpay_db::customers`, the `charges` statement of the erasure
   transaction). A charge collected through such a session hangs off an
   intent with no customer, so erasing `X` left that charge's `payer_ref`
   in place.

## Decision

**D1. In the same transaction as the session insert,** when the session
names a customer, the insert is preceded by a compare-and-swap:

```sql
UPDATE payment_intents SET customer_id = $3, updated_at = now()
WHERE id = $1 AND merchant_id = $2 AND customer_id IS NULL
```

It lives in `vpay_db::checkout_sessions::claim_intent_customer`, called from
`CheckoutSessions::create`. `create` now opens a transaction, runs this, then
the `INSERT … RETURNING`, then commits. `updated_at = now()` is not in the
brief's SQL. It is there because every writer of `payment_intents` keeps that
column current, and there is no trigger to do it.

**D2. Zero rows affected is decided by re-reading the intent, in the same
transaction.**

- The intent is already for **this** customer: the session proceeds. That
  covers a session that inherited or repeated its intent's customer, and
  every session `POST /v1/invoices/{id}/pay` creates.
- It is for **another** customer: the new
  `vpay_db::DbError::IntentCustomerConflict`, which `vpay_api` turns into the
  existing refusal **byte for byte**. It is one function,
  `customer_contradiction`, shared by the pre-check and by `create_error`.
- There is no such intent for this merchant: the insert's foreign key refuses
  it, as before.

**D3. If the intent already has a different customer, the existing refusal
stands, unchanged.** `prepare_create` still refuses it before any write,
with the same sentence.

**D4. Two concurrent creates naming different customers for one customer-less
intent: exactly one wins.** Under `READ COMMITTED` the second `UPDATE` waits
on the first's row lock, re-evaluates `customer_id IS NULL` against the
committed row, and matches nothing. The re-read then finds the winner's
customer and refuses, and the transaction rolls back with no session written.
**Which refusal the loser gets depends on when it read the intent**, and in
both cases it is what the same request would get arriving second:

- if both passed `prepare_create` before either committed, the loser gets
  the `400` naming `customer`, the same bytes as a session naming a customer
  different from its intent's;
- if the loser's `prepare_create` ran after the winner committed, the
  one-open-session pre-check answers first with its `409`. That is what a
  session naming another customer gets today when the intent has an open
  session, because that check has always run before the customer check. The
  order was not changed.

**D5. Same customer as the intent already has: unchanged.** The `UPDATE`
matches nothing, and the re-read agrees. `last_used_at` is still stamped by
`resolve_for_attachment` before the transaction, as the create path already
did. That includes a request that then loses the race, just as a request the
pre-check refuses was already stamped.

**D6. No new event type, and nothing to amend in an event already emitted.**
The vocabulary only takes Stripe types that have a writer, and this write
does not need one. The three `payment_intent.*` types vpay writes
(`succeeded`, `payment_failed`, `canceled`) are all written at a charge's
outcome or at cancellation. A session can only be created for a
`requires_payment_method` intent with no charge, so no `payment_intent.*`
event exists for the intent when this write happens. Session creation itself
writes no event, so no event body in this transaction has a stale
`customer`. Later `payment_intent.*` bodies are rendered from the row, and
they carry the customer. The one earlier copy is the intent create's stored
idempotent response, which keeps `customer: null`. A replay answers what the
original answered, as it already does for every field that changes after
create.

### No backfill

**Intents created before this change keep `customer_id` NULL.** A session
that named `X` on a customer-less intent before 2026-09-23 still leaves that
intent's payment out of the intent and refund filters, and out of `X`'s
erasure (effect 3 above). Those historical rows stay inconsistent across the
three filters.

The reason: **a backfill would have to choose between sessions that named
different customers.** Effect 2 means an intent can carry sessions naming
`X` and `Y`, and nothing recorded which payer the merchant meant. The newest
session, the oldest one, and the one that was paid through are all guesses.
A wrong guess attaches a payment to a payer who did not make it. The guess
is also not harmless under erasure: erasing the wrongly chosen customer
would redact another person's `payer_ref`, and erasing the right one would
still miss it. Backfilling only the unambiguous intents would leave the
ambiguous ones behind, so "historical rows can disagree" would become
"some historical rows can disagree", which is harder for a merchant to
reason about and no more true. The dividing line is kept as a date instead.
A merchant who needs a historical session's payment by customer reads
`GET /v1/checkout/sessions?customer=X` and follows each session's
`payment_intent`, which is what D12 already offered.

## Alternatives considered

- **Leave it and document it** (ADR-0024's status quo). Rejected. It keeps
  effect 2, one intent offered to two payers, for every session from now on,
  and the filters would disagree for ever rather than only for rows written
  before a known date.
- **Filter intents and refunds through their sessions**, so
  `GET /v1/payment_intents?customer=X` also matches an intent whose session
  named `X`. Rejected. An intent whose sessions named `X` and `Y` would be in
  both customers' lists. The rendered intent would still say
  `customer: null` while being listed under `X`. It adds a second join to
  two list queries, and it neither stops effect 2 nor fixes effect 3.
- **Backfill.** Rejected for the reason in § No backfill.
- **Refuse `customer` on a session whose intent has none**, so a merchant has
  to put it on the intent. Not chosen. It would be a breaking change to a
  parameter merchants can send today, and the maintainer's choice was to make
  the parameter write through rather than to take it away.

## Consequences

- `CheckoutSessions::create` is now **a transaction of up to three
  statements** (the `UPDATE`, a re-read if it matched nothing, and the
  `INSERT`) where it was one. Every session create with a customer pays for
  them, including an invoice's, whose `UPDATE` always matches nothing.
- `vpay_db::DbError` gains `IntentCustomerConflict`
  (`Category::InvalidRequest`, code `intent_customer_conflict`). It reaches a
  merchant only through `create_error`, which renders the existing `400`. If
  another caller surfaced it unmapped, it would be a `400` with that code.
- **Idempotency.** The loser of the forced race gets its `400` from
  `create`, after the key is claimed, so the response is **stored** under its
  `Idempotency-Key`, as the one-open-session `409` from the same place always
  was. The pre-check's identical `400` **releases** the key, so a retry under
  it runs again and gets whatever the intent's state answers then. That is
  the same `400`, or the one-open-session `409` while the winner's session
  is still open. Neither retry can succeed, because an intent's customer is
  never rewritten once it is set.
- Erasure now reaches a charge collected through a session created from
  today onward, because its intent names the customer. For historical rows it
  still does not, and **this ADR does not decide whether erasure should also
  go through `checkout_sessions.customer_id`**. That is a privacy question
  for the maintainer, raised here because nothing else recorded it.
- Two existing integration cases relied on the old shape and now stage it in
  SQL, because the API can no longer produce it. They are
  `the_customer_filter_reads_the_sessions_own_customer_and_is_not_an_oracle`
  (`checkout_sessions.rs`), where it is the row that shows why sessions
  still filter on their own column, and
  `a_sessions_customer_is_inherited_supplied_or_a_refused_contradiction`
  (`customers.rs`), where it is the customer only a session references.
- Nothing on the wire changes shape. Both SDKs' `customer` doc comments
  change. No migration is added, because the column and its foreign key
  already exist.
