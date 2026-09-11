# Customers — events (2026-09-10, issue #66)

_Split out of [docs/flows/customers.md](../customers.md) on 2026-09-11 by exp57, which broke a 888-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## Events (2026-09-10, [issue #66](https://github.com/vaam-apps/vpay/issues/66))

Three of this resource's four writes emit, and each writes its `events` row in
the transaction of the write it describes.

| Write                                             | Event              | Where                                                                                                                                                                                                                     |
| ------------------------------------------------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /v1/customers`                              | `customer.created` | `vpay_api::v1::customers::create_with_event`                                                                                                                                                                              |
| `POST /v1/customers/{id}`, when something changes | `customer.updated` | `vpay_api::v1::customers::update_once`                                                                                                                                                                                    |
| `POST /v1/customers/{id}` with no body            | — nothing          | Stripe's no-op; nothing is written, so there is nothing to report                                                                                                                                                         |
| `DELETE /v1/customers/{id}`                       | `customer.deleted` | `vpay_api::v1::customers::delete_once` → `vpay_db::customers::erase_in_tx` (**2026-09-10**, [#96](https://github.com/vaam-apps/vpay/issues/96) item 2 — until then this route emitted **nothing** and only the sweep did) |
| The retention sweep                               | `customer.deleted` | `vpay_db::customers::erase_idle` (2026-09-06)                                                                                                                                                                             |
| A second `DELETE` on an erased customer           | — nothing          | the erasure already happened; a second event would say it happened twice                                                                                                                                                  |

`customer.created` and `customer.updated` were **not** in
`type_is_a_documented_event` until migration `0039`, and this document's
"What is not built" section said so for four days. The reason it gave was
migration `0023`'s rule — the vocabulary moves in lockstep with the code that
writes it, and both routes were single statements on the pool. Making them
transactional is what let the labels come in.

### The update takes the row's lock, and that is not defensive

`metadata` is merged key-wise (Stripe's contract), so the value written is a
function of the value stored: the handler reads, merges in Rust, and writes.
Until 2026-09-10 that read ran on the pool, and
`vpay_api::v1::customers::update`'s own doc comment said in as many words that
two concurrent updates each adding one key could lose one of them, and that it
was not closed. That was tolerable while nothing depended on the result being
definite.

`customer.updated` is exactly such a dependency. A merchant acting on an event
that described the losing merge would be acting on a state the database does
not hold — and unlike a stale read, they would have no reason to doubt it.

So the read is `SELECT … FOR UPDATE` inside the transaction that writes and
emits. A second request blocks on the lock, re-reads the **committed** merge,
and merges onto that; the two events then describe the two states in the order
they happened. `two_concurrent_metadata_merges_keep_both_keys_and_the_event_carries_the_committed_state`
drives two real requests through the shipping router, and
`a_locked_customer_read_waits_for_the_writer_and_then_sees_its_value` forces
the interleaving deterministically at the repository seam. Removing
`FOR UPDATE` was measured on 2026-09-10: both fail.

The three scalar fields were never at risk and still are not — each is written
from the request alone, and the statement assigns only the columns the request
mentioned.

### What does not emit, and why each one would be noise

- **A bodiless `POST /v1/customers/{id}`.** Stripe answers the object
  unchanged and writes nothing; so does vpay. An event about a change that did
  not happen is a webhook a merchant has to work out how to ignore.
- **`touch_last_used`, the retention stamp.** It moves `last_used_at`, which
  is on **no** wire object at all — so an event for it would carry a body
  byte-identical to the previous one, once per payment, for ever.
- **A `404`.** The transaction is abandoned before any write.
