-- What the movement cost: the fee the rail charged *us* to return the money,
-- in the refund's own `currency_code`. Issue #46, filed by an integrator whose
-- own domain type had no `Option` and so hardcoded `provider_fee_minor: 0` —
-- the exact shape AGENTS.md rule 2 refuses.
--
-- STATUS, stated before anything else: **nothing in this repository WRITES
-- this column, and every stored value is therefore NULL.** It is READ:
-- `vpay_db::refunds`' COLUMNS projection selects it and `GET /v1/refunds/{id}`
-- renders it as the `refund` object's tenth key (issue #45 landed that
-- repository and that route on 2026-09-06, while this migration was on an open
-- branch; this header said "no refunds repository, no /v1/refunds route" until
-- then, which was true when it was written).
--
-- What is still missing is the *writer*: there is no `POST /v1/refunds`, no
-- create in the repository, and no adapter that can produce a fee — Orange's
-- Web Payment product documents no refund API at all, and MTN refunds are the
-- Disbursements product this deployment has never been issued a credential
-- for (`mtn_momo::refund` is the one remaining `NotImplemented` token; see
-- docs/status.md). The column exists so the declared wire contract
-- (docs/flows/merchant-auth.md, the `refund` object's tenth field) has exactly
-- one place to come from when a rail finally reports one, rather than landing
-- in a later ALTER on a table someone has by then started writing.
--
-- NULLABLE, and that is the whole design. `NULL` means "the rail did not tell
-- us"; `0` means "the movement was free". Collapsing those two is what the
-- issue was filed about, so the column has no DEFAULT: a row written without
-- a fee is honestly unknown, never free.
--
-- No second currency column, deliberately: docs/flows/money.md allows exactly
-- one currency per object and no conversion anywhere in vpay, so the fee is
-- minor units of `refunds.currency_code` or it is nothing.
ALTER TABLE refunds ADD COLUMN fee BIGINT;

-- Non-negative rather than strictly positive, unlike `amount_positive` on the
-- same table: a zero-amount refund is a caller mistake, but a zero *fee* is a
-- real answer a rail can give ("we did not charge you for this"), and it is
-- the answer the nullability above exists to keep distinguishable from
-- silence. A negative fee would be a rebate, which vpay has no concept of and
-- must not silently render to a merchant as a cost.
ALTER TABLE refunds
    ADD CONSTRAINT fee_non_negative CHECK (fee IS NULL OR fee >= 0);

COMMENT ON COLUMN refunds.fee IS
    'What the rail charged us to execute this refund, in minor units of this row''s currency_code. NULL means the rail reported no fee; 0 means the movement was free. NOT WRITTEN BY ANY CODE IN THIS REPOSITORY (issue #46), so every stored value is NULL; it is read by vpay_db::refunds and rendered by GET /v1/refunds/{id} — see the migration header and docs/status.md.';
