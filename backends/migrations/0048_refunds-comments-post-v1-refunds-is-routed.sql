-- 0048: correct three shipped COMMENTs that describe a refund surface the
-- repository no longer has.
--
-- On 2026-09-16 the four `/v1` refund routes landed (RFC-0003 section 2):
-- POST /v1/refunds, POST /v1/refunds/{id}, GET /v1/refunds and
-- POST /v1/refunds/{id}/cancel, beside the GET /v1/refunds/{id} issue #45
-- shipped on 2026-09-05. A merchant request can create a refunds row, and
-- vpay_api::v1::refunds became the first writer of `charge.refunded` and
-- `charge.refund.updated` this repository has ever had.
--
-- Three COMMENTs written before that day assert the opposite, and because
-- they are `COMMENT ON` statements they are not merely stale text in a file
-- — they are in every database that applied 0039 and 0047, and an operator
-- reading `\d+ refunds` or `\d+ events` during an incident is told that no
-- merchant request can create a refund. That is the wrong sentence to read
-- while looking at a refunds table that now has rows in it.
--
-- 0020_provider-requests-status-code-comment.sql is the precedent and
-- 0047_refunds-comments-mtn-refund-is-written.sql is the most recent
-- application of it: a migration whose whole purpose is a comment. It is
-- also the only mechanism available. An applied migration is immutable down
-- to the byte (sqlx stores a SHA-384 of the whole file and refuses to boot
-- when it moves - issue #76, docs/runbooks/migrations.md), and
-- backends/migrations/README.md says in as many words: "To fix a mistake in
-- an applied migration, write a new migration that corrects it. Never touch
-- the old file."
--
-- This migration changes no data, no column and no constraint.
--
-- WHAT IS CORRECTED, AND WHAT IS NOT. Only the claims about what is ROUTED
-- and about which event types have a writer. Everything else in all three
-- COMMENTs stands, and the replacement text keeps it:
--
--   * no rail has ever returned money. MTN's Disbursements product has never
--     been called from this repository and no REAL MTN Disbursements
--     credential exists in this project: the only subscription key anywhere
--     is the stub the e2e/demo stack points at a wiremock container, which
--     is why this file does not say "no deployment holds it". The stack that
--     runs the SDKs' live refund suites holds one. orange_money::refund is a
--     declared NotImplemented token. Neither answers Unsupported.
--   * NOTHING SETTLES A PENDING REFUND. The provider port has no refund
--     status read and there is no refund poll ladder (RFC-0003 open question
--     8, open), so a refund created through /v1 stays `pending` until an
--     operator moves it. This is why invoices.amount_refunded is still 0 in
--     every deployment: the route being mounted did not give that column a
--     reachable writer, it only changed the reason it has none.
--
-- The `--` headers of 0017, 0031, 0039, 0042 and 0047 carry the same stale
-- claims and are in no database, so no statement can reach them. They are
-- recorded in backends/migrations/README.md section Errata instead.

COMMENT ON TABLE refunds IS
    'Persistence shape for refunds, REACHABLE FROM A DEPLOYMENT since 2026-09-16: POST /v1/refunds (RFC-0003 section 2) writes this table through vpay_db::Refunds::create_in_tx, reserving the amount on payment_intents and emitting charge.refunded in the same transaction, and the four /v1 refund routes read, update, list and cancel these rows. (0017 said "no refunds repository, no /v1/refunds route, and the adapter refund path is still NotImplemented"; 0047 corrected that on 2026-09-15 but kept "NOT REACHABLE FROM ANY DEPLOYMENT ... POST /v1/refunds is routed nowhere, so no merchant request can create a row and every deployment''s table is empty but for rows an operator or a test put there", which stopped being true on 2026-09-16 and is corrected here by 0048.) WHAT A ROW IN THIS TABLE STILL DOES NOT MEAN: that money came back. No rail has ever returned money - mtn_momo::refund is MTN''s Disbursements transfer call, against a product this repository has never called and under no REAL MTN Disbursements credential - the only subscription key anywhere in this project is the stub the e2e/demo stack points at a wiremock container, and orange_money::refund is a declared NotImplemented token because no Orange transfer API is documented here - and NOTHING SETTLES A PENDING REFUND, because the provider port has no refund status read (RFC-0003 open question 8). Read this row''s status. See docs/status.md.';

COMMENT ON COLUMN invoices.amount_refunded IS
    'Running total of succeeded refunds against the intent that paid this invoice, in minor units of this row''s currency_code (issue #91, D5). GROSS: it is not subtracted from amount_paid and does not enter amounts_add_up, so a refunded invoice stays paid with nothing remaining. Written by exactly one statement, vpay_db::invoices::add_refund_for_intent_in_tx, inside vpay_db::settlement''s refund transaction. EVERY STORED VALUE IS STILL 0 IN ANY DEPLOYMENT, AND THE REASON CHANGED ON 2026-09-16: 0042 gave it as "NO RAIL CAN REFUND YET", 0047 gave it as "POST /v1/refunds is routed nowhere, so nothing a merchant can call creates the refund this column counts", and that route IS routed now (RFC-0003 section 2). A merchant can create a refund; this column still never moves, because the statement above runs only in the settlement that moves a refund to succeeded and NOTHING SETTLES A PENDING REFUND - the provider port has no refund status read and there is no refund poll ladder (RFC-0003 open question 8). Also still true from 0047: mtn_momo::refund makes MTN''s Disbursements transfer call against a product this repository has never called and under no REAL MTN Disbursements credential - the only subscription key anywhere in this project is the stub the e2e/demo stack points at a wiremock container, orange_money::refund is a declared NotImplemented token, Unsupported is neither rail''s answer, vpay_db::Refunds::create inserts a refunds row, payment_intents.amount_refunded / amount_refund_pending ARE maintained by vpay_db::settlement::apply_refund_succeeded and apply_refund_failed in the transaction that writes this column, and MTN''s transfer answers 202 ACCEPTED so an Ok from the port is not a settlement. See docs/status.md.';

COMMENT ON COLUMN events.type IS
    'Constrained to the fifteen event types in docs/flows/webhooks.md. Only real Stripe event types, so a merchant''s existing Stripe-shaped handler recognises every one of them. THIRTEEN of the fifteen have a writer since 2026-09-16; the two that do not are payment_intent.created and payment_intent.processing, and each is listed as unwritten in that document rather than removed. (0039 said "Eleven of the fifteen have a writer; the four that do not are payment_intent.created, payment_intent.processing and the two charge.refund types"; corrected by 0048. charge.refunded and charge.refund.updated acquired their first writer ever when the four /v1 refund routes landed - RFC-0003 section 2 - and it is vpay_api::v1::refunds, which writes each event in the same transaction as the write it reports, as every other writer in this repository does. A charge.refunded reports that a charge now HAS a refund against it, not that money moved: the refund it carries is pending and nothing settles a pending refund.) See docs/status.md.';
