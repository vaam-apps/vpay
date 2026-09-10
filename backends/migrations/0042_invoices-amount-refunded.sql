-- What has been given back out of what was paid: the running total of
-- succeeded refunds against the intent that paid this invoice, in the
-- invoice's own `currency_code`. Issue #91 item 3, decision D5
-- (docs/plans/exp32-invoices-notes/opus-review.md).
--
-- THE DECISION THIS COLUMN IMPLEMENTS. A refund against a paid invoice leaves
-- the invoice `paid` and does not create a credit note — vpay has no such
-- object and D5 declined to invent one. The refunded total is therefore a
-- column on the document itself, shaped exactly as `payment_intents`'
-- `amount_refunded` has been since migration 0003: a GROSS counter that sits
-- BESIDE the money columns rather than inside their arithmetic.
--
-- WHY IT IS NOT SUBTRACTED FROM `amount_paid`, and what that buys. The
-- alternative — decrement `amount_paid`, let `amounts_add_up` push the
-- difference into `amount_remaining`, and amend `paid_means_nothing_remaining`
-- to tolerate it — was considered and rejected here. It makes a fully
-- refunded invoice read `status = paid, amount_paid = 0, amount_remaining =
-- <the whole bill>`, i.e. a settled document that claims the payer owes the
-- lot again. `amount_remaining` is the number `POST /v1/invoices/{id}/pay`
-- mints an intent for and the number a merchant chases a payer with, so
-- moving it for a refund is the misreading this repository can least afford.
-- Keeping the three existing amounts frozen means NEITHER
-- `paid_means_nothing_remaining` NOR `amounts_add_up` has to be amended: a
-- refunded paid invoice was already storable under both, and this migration
-- deliberately leaves both exactly as migration 0036 wrote them. See
-- docs/plans/exp47-invoice-followups-notes/opus.md, which records that this
-- is a deviation from the brief that asked for the CHECK to be amended, and
-- why the amendment would have been either vacuous or harmful.
--
-- NO DEFAULT, deliberately, and this is `invoices.metadata`'s tripwire rather
-- than `payment_intents.amount_refunded`'s convenience. Every writer of this
-- table names every amount it writes (`vpay_db::invoices`' insert, re-sum and
-- finalize all write `amount_paid = 0` for exactly this reason), so a DEFAULT
-- would be a value no statement had to think about — and `schemas/vpay.cstack`'s
-- `model Invoice` declares this column, so a DEFAULT would also be one more
-- permanent `column ... default value differs` line in the drift report
-- (migration 0034's rule, and 0033's problem).
--
-- The backfill is the ADD's own DEFAULT, dropped immediately afterwards:
-- every row that exists when this migration runs has had no refund, because
-- nothing in this repository could have made one.
ALTER TABLE invoices ADD COLUMN amount_refunded BIGINT NOT NULL DEFAULT 0;
ALTER TABLE invoices ALTER COLUMN amount_refunded DROP DEFAULT;

-- `payment_intents`' own pair of guards (migration 0003), applied to the
-- document rather than to the intent, and named the same way so the two
-- read as one rule in two places.
--
-- `refunded_at_most_paid` is the over-refund guard, and it is what makes
-- "fail closed" true of the settlement statement rather than merely intended:
-- `vpay_db::settlement`'s refund transaction increments this column with
-- `amount_refunded + $n` inside a transaction, so a second refund that would
-- take the total past what was actually collected is refused by the database
-- and the whole transaction rolls back — including the refund row's own
-- pending -> succeeded flip. Two concurrent increments serialize on the row
-- lock MVCC already takes for any UPDATE and the second re-evaluates against
-- the first's committed value, which is 0003's argument verbatim.
--
-- It is `<= amount_paid` and not `<= amount_due`: a merchant cannot give back
-- money that was never collected, and on a `void` or `uncollectible` invoice
-- `amount_paid` is 0, which makes any refund against it refused rather than
-- merely odd.
ALTER TABLE invoices
    ADD CONSTRAINT amount_refunded_non_negative CHECK (amount_refunded >= 0);
ALTER TABLE invoices
    ADD CONSTRAINT refunded_at_most_paid CHECK (amount_refunded <= amount_paid);

COMMENT ON COLUMN invoices.amount_refunded IS
    'Running total of succeeded refunds against the intent that paid this invoice, in minor units of this row''s currency_code (issue #91, D5). GROSS: it is not subtracted from amount_paid and does not enter amounts_add_up, so a refunded invoice stays paid with nothing remaining. Written by exactly one statement, vpay_db::invoices::add_refund_for_intent_in_tx, inside vpay_db::settlement''s refund transaction. NO RAIL CAN REFUND YET (ProviderAdapter::refund is NotImplemented on MTN and Unsupported on Orange), so every stored value is 0 in any deployment - see docs/status.md.';

-- GAP, stated rather than half-closed: `payment_intents.amount_refunded` and
-- `amount_refund_pending` (migration 0003) are NOT maintained by the
-- settlement that writes this column. Migration 0017's own GAP note pairs
-- them with the INSERT that creates a `refunds` row, and that insert does not
-- exist anywhere in this repository — `POST /v1/refunds` is unrouted and
-- `vpay_db::Refunds` exposes no create. Incrementing one half of a paired
-- total whose other half nothing writes would leave `no_over_refund` counting
-- money twice the day the insert lands. When it lands, the decrement of
-- `amount_refund_pending` belongs in the same statement as this column's
-- increment.
