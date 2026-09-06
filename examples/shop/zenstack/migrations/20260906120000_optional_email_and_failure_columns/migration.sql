-- Two changes, both so the demo can show what happens when a payment does
-- *not* work (exp22, 2026-09-06).
--
-- 1. `orders.email` becomes nullable. The shop asks for an e-mail so it can
--    send a receipt, and a receipt is not a condition of paying: the identity
--    on a mobile-money payment is the payer's phone number, which the rail
--    holds and the shop never sees (the customers decision of 2026-09-05 —
--    phone-only customers are allowed). Widening a NOT NULL is safe on a
--    populated table and needs no backfill.
--
-- 2. `orders.failure_code` and `orders.failure_message` carry
--    `last_payment_error` from the event that failed the order. They are
--    written by the webhook handler and by nothing else, in the same
--    statement as the status they explain. Stored rather than derived
--    because a delivered event is the only place the code ever appears:
--    this shop never calls vpay to read an intent, by design.
--
-- No catalogue change. An earlier draft of this migration seeded a EUR item
-- so the shop could reach the push rail, on the belief that the demo stack
-- settles `mtn_momo` in EUR. It does not: `just gen-demo-keys` writes a
-- `providers:` block that puts `mtn_momo` on `currency: XAF` and then
-- *checks* that it did, precisely so the shop's MTN button is payable. A EUR
-- product would therefore have been an item nothing in the demo can buy.

ALTER TABLE "orders" ALTER COLUMN "email" DROP NOT NULL;

ALTER TABLE "orders" ADD COLUMN "failure_code" TEXT;
ALTER TABLE "orders" ADD COLUMN "failure_message" TEXT;
