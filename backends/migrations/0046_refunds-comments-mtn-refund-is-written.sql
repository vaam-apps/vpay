-- 0046: correct two shipped COMMENTs that say no adapter can refund.
--
-- On 2026-09-15 `mtn_momo::refund` became MTN's Disbursements `transfer`
-- call (RFC-0003 § 5) and the `ProviderError::NotImplemented("mtn_momo::refund")`
-- token was retired. Two COMMENTs written before that day assert the opposite,
-- and because they are `COMMENT ON` statements they are not merely stale text
-- in a file — they are **in every database that applied 0017 and 0042**, and
-- an operator reading `\d+ refunds` or `\d+ invoices` sees them.
--
-- `0020_provider-requests-status-code-comment.sql` is the precedent this follows:
-- a migration whose whole purpose is a comment. It is also the only mechanism
-- available. An applied migration is immutable down to the byte (sqlx stores a
-- SHA-384 of the whole file and refuses to boot when it moves — issue #76,
-- `docs/runbooks/migrations.md`), and `backends/migrations/README.md` says in
-- as many words: "To fix a mistake in an applied migration, write a new
-- migration that corrects it. Never touch the old file."
--
-- This migration changes no data, no column and no constraint.
--
-- What is corrected, and what is NOT: the parenthetical about which adapter
-- can refund. Both COMMENTs' substance stands — nothing in this repository
-- writes a `refunds` row, `POST /v1/refunds` is unrouted, and every stored
-- `invoices.amount_refunded` is 0 in any deployment. The reason changed, not
-- the fact, and the replacement text says which.
--
-- `0031_refunds-fee.sql` carries the same stale claim in its FILE HEADER
-- ("`mtn_momo::refund` is the one remaining `NotImplemented` token"), and so
-- does `0017`'s. Those are not in any database and cannot be reached by SQL;
-- they are recorded in `backends/migrations/README.md` § Errata instead.

COMMENT ON TABLE refunds IS
    'Intended persistence shape for refunds. NOT WRITTEN OR READ BY ANY CODE IN THIS REPOSITORY — no INSERT anywhere, no POST /v1/refunds route. Schema only, like the ledger tables in 0005. (0017 added "and the adapter refund path is still NotImplemented"; corrected by 0046 on 2026-09-15, when mtn_momo::refund became MTN''s Disbursements transfer call — against a credential no deployment holds and a product this repository has never called. Orange Money still answers Unsupported. See docs/status.md.)';

COMMENT ON COLUMN invoices.amount_refunded IS
    'Running total of succeeded refunds against the intent that paid this invoice, in minor units of this row''s currency_code (issue #91, D5). GROSS: it is not subtracted from amount_paid and does not enter amounts_add_up, so a refunded invoice stays paid with nothing remaining. Written by exactly one statement, vpay_db::invoices::add_refund_for_intent_in_tx, inside vpay_db::settlement''s refund transaction. EVERY STORED VALUE IS 0 IN ANY DEPLOYMENT: nothing inserts a refunds row and POST /v1/refunds is unrouted. (0042 gave the reason as "NO RAIL CAN REFUND YET (ProviderAdapter::refund is NotImplemented on MTN and Unsupported on Orange)"; corrected by 0046 on 2026-09-15 — mtn_momo::refund makes MTN''s Disbursements transfer call, under a credential no deployment holds and against a product this repository has never called, while Orange still answers Unsupported. Note also that MTN''s transfer answers 202 ACCEPTED: an Ok from the port is not a settlement, so the writer that eventually maintains this column must not treat one as succeeded - RFC-0003 open question 8.) See docs/status.md.';
