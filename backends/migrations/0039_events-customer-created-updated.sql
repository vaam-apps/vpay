-- customer.created and customer.updated: the fourteenth and fifteenth
-- documented event types.
--
-- WHY THE VOCABULARY IS REOPENED, AND WHY ONLY NOW
--
-- 0023's rule, unchanged and restated by 0029, 0034 and 0036: the database is
-- what refuses a type no code writes, so the CHECK moves in lockstep with the
-- code that writes it rather than being written permissively ahead of it.
-- 0034 is the migration that *declined* to add these two, and its comment
-- said exactly why:
--
--   > `POST /v1/customers` and `POST /v1/customers/{id}` are single
--   > statements on the pool; emitting an event means putting the write and
--   > the event in one transaction, which is a change to the shape of two
--   > repository methods rather than a line in a CHECK.
--
-- That change is in this commit (issue #66), so the labels come with it. Both
-- are Stripe's own spellings, which is docs/flows/webhooks.md's standing
-- rule: a custom type is silently dropped by any merchant using
-- `stripe-node`'s typed event union or an exhaustive `switch`.
--
-- WHAT WRITES THEM
--
--   * `customer.created` — `vpay_api::v1::customers::create_with_event`, in
--     the same transaction as the INSERT. The event's `data` is rendered from
--     the row the INSERT returned, not from the request: `seq` and the two
--     timestamps are this database's, so a projection of the request would be
--     a second implementation of the insert.
--   * `customer.updated` — `vpay_api::v1::customers::update_once`, in the same
--     transaction as the UPDATE, and after a `SELECT … FOR UPDATE` on the same
--     row. The lock is load-bearing rather than defensive: `metadata` is
--     merged key-wise (Stripe's contract), so the written value is a function
--     of the stored one, and a pooled read left a window in which two
--     concurrent updates each adding one key lost one of them. A merchant
--     acting on an event that described the losing merge would be acting on a
--     state the database does not hold.
--
-- WHAT DOES *NOT* WRITE THEM, AND WHY THAT MATTERS
--
--   * a **bodiless** `POST /v1/customers/{id}`. Stripe answers such a request
--     with the object unchanged and writes nothing; so does vpay, and an event
--     about a change that did not happen is a webhook a merchant has to work
--     out how to ignore.
--   * `vpay_db::Customers::touch_last_used`, the retention stamp. It moves
--     `last_used_at`, which is on **no** wire object at all (migration 0034's
--     column comment) — so an event for it would carry a body byte-identical
--     to the previous one, once per payment, for ever.
--   * the twelve-month retention sweep, which writes `customer.deleted`
--     (0034) and nothing else.
--
-- `object_id` for both types is the `cus_…`, which 0034 already introduced as
-- a prefix this polymorphic column carries; no comment on that column is
-- re-issued here.
--
-- NUMBERING. This file is 0039 and there is no 0038 in this tree: 0038 is
-- taken by a branch in flight (staff-auth follow-ups). `sqlx::migrate!`
-- orders by the numeric prefix and does not require them to be contiguous, so
-- the gap is a merge convenience and not a state. `schema_migrates_cleanly_on_an_empty_database`
-- counts *files*, which is 38 on this tree.
ALTER TABLE events DROP CONSTRAINT type_is_a_documented_event;
ALTER TABLE events ADD CONSTRAINT type_is_a_documented_event CHECK (type IN (
    'payment_intent.created',
    'payment_intent.processing',
    'payment_intent.succeeded',
    'payment_intent.payment_failed',
    'payment_intent.canceled',
    'charge.refunded',
    'charge.refund.updated',
    'checkout.session.expired',
    'customer.created',
    'customer.updated',
    'customer.deleted',
    'invoice.created',
    'invoice.finalized',
    'invoice.paid',
    'invoice.voided'
));

COMMENT ON COLUMN events.type IS
    'Constrained to the fifteen event types in docs/flows/webhooks.md. Only real Stripe event types, so a merchant''s existing Stripe-shaped handler recognises every one of them. Eleven of the fifteen have a writer; the four that do not are payment_intent.created, payment_intent.processing and the two charge.refund types, and each is listed as unwritten in that document rather than removed.';
