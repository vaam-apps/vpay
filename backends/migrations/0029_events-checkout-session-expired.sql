-- checkout.session.expired: the eighth documented event type.
--
-- WHY THE VOCABULARY IS REOPENED, AGAIN
--
-- Same mechanism as 0024's `fanout_state_is_known` and 0022/0023's
-- `jobs.kind_is_known`: the database is what refuses a type no code writes,
-- so the CHECK moves in lockstep with the code that writes it rather than
-- being written permissively ahead of it. 0018 closed this over seven types
-- and said why — docs/flows/webhooks.md's rule is "only real Stripe event
-- types", because a custom type is silently dropped by any merchant using
-- `stripe-node`'s typed event union or an exhaustive `switch`.
--
-- `checkout.session.expired` keeps that rule: it is Stripe's own type name
-- for Stripe's own Checkout Session expiry, so a Stripe-shaped handler
-- already has a branch for it. The *object* under `data.object` is vpay's
-- `checkout.session` (Step 9's D10 lifecycle: three statuses, no
-- `line_items`, no `amount_total`), which is a narrower object than
-- Stripe's — but a merchant's handler reads the id and re-reads the session,
-- and an invented type would have had no branch at all.
--
-- WHAT WRITES IT
--
-- `vpay_db::checkout_sessions::CheckoutSessions::expire_due`, inside the same
-- transaction as the `open` -> `expired` compare-and-swap it names, called
-- once per due session by `vpay_worker::handlers::sweep_expired`. That is the
-- same shape `vpay_db::settlement` has written `payment_intent.succeeded` and
-- `payment_intent.payment_failed` in since Step 4: a crash between the status
-- flip and the event row is not expressible, because there is no window
-- between them.
--
-- WHAT DOES *NOT* WRITE IT, AND WHY THAT MATTERS
--
--   * the settlement transaction, which already moves a session to
--     `complete`/`paid` or `expired`/`failed` and already emits
--     `payment_intent.succeeded` / `payment_intent.payment_failed` for the
--     same transition. A second event for one thing happening is a merchant
--     dedupe problem vpay would have created;
--   * `POST /v1/checkout/sessions/{id}/expire`, the merchant's own abandon.
--     A merchant who just asked for an expiry does not need to be told about
--     it, and this is a stated gap rather than an oversight — see
--     docs/flows/hosted-checkout.md, "What is not built".
--
-- `object_id` for this type is the `cs_…`, which is the fourth prefix the
-- polymorphic `object_id` column now carries (0018's comment named `pi_…`,
-- `ch_…` and `re_…`, which is three); its comment is re-issued below rather
-- than edited, because a migration is history (ADR-0003).
ALTER TABLE events DROP CONSTRAINT type_is_a_documented_event;
ALTER TABLE events ADD CONSTRAINT type_is_a_documented_event CHECK (type IN (
    'payment_intent.created',
    'payment_intent.processing',
    'payment_intent.succeeded',
    'payment_intent.payment_failed',
    'payment_intent.canceled',
    'charge.refunded',
    'charge.refund.updated',
    'checkout.session.expired'
));

COMMENT ON COLUMN events.type IS
    'Constrained to the eight event types in docs/flows/webhooks.md. Only real Stripe event types, so a merchant''s existing Stripe-shaped handler recognises every one of them. checkout.session.expired (0029) is the only one whose data.object is not a payment_intent or a refund.';
COMMENT ON COLUMN events.object_id IS
    'The id of the object this event is about: pi_ for payment_intent.*, ch_/re_ for the refund types, and cs_ for checkout.session.expired (0029). Untyped and un-foreign-keyed on purpose — it points into four different tables depending on type, and a polymorphic reference cannot be a foreign key.';
