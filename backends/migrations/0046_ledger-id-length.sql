-- Bound the two caller-supplied ledger ids, which are the only caller-named
-- id columns in this schema that were unbounded. RFC-0003 § 4, and the gap
-- migration 0045's own "WHAT THIS MIGRATION DOES NOT FIX" section named and
-- deliberately left open.
--
-- 0045 wrote, about itself:
--
--     `ledger_transactions.id` and `ledger_entries.id` carry no
--     `CHECK (char_length(id) BETWEEN 1 AND 64)`. [fourteen tables] all bound
--     their caller-supplied id; migration 0005 gave these two none, and THIS
--     migration adds `ledger_entries_merchant_id_length` to a *new* column on
--     exactly the grounds that would demand one — "a copy with a wider domain
--     than its source is a copy that can hold something the source could not".
--     A primary key with no source at all has a wider domain than any of them.
--
-- and gave one reason for leaving it: two more single-column CHECKs move
-- `EXPECTED_DRIFT_CHANGES` by +2 and that constant may only move by a
-- measurement. It is measured here — see that constant's own note for the
-- 192 -> 194 line and for which cratestack release took it.
--
-- The other half of 0045's gap closed in the same commit: `vpay_core::ids`
-- now mints `lt_…` (`LEDGER_TRANSACTION_PREFIX`, `ledger_transaction_id()`),
-- so the bound below is a bound on a vocabulary that exists rather than on an
-- arbitrary string. `vpay_db::settlement`'s two call sites — the first code in
-- this repository's history to post to the ledger — mint through it.
--
--
-- WHY 64 FOR A TABLE WHOSE ENTRY IDS ARE DERIVED, NOT MINTED
--
-- `vpay_db::ledger::post_in_tx` names each leg `{transaction_id}_{index}`, so
-- `ledger_entries.id` is *longer* than the transaction id it is derived from.
-- 64 is still the right number and not a trap:
--
--   * a minted `lt_…` is 27 characters, so a leg is 29 or 30 — less than half
--     the bound, and a transaction would need ~10^34 legs to reach it;
--   * a caller that hand-built a 64-character transaction id would produce a
--     66-character entry id and be refused *by this CHECK*, which is the
--     correct outcome: that caller is outside the minted vocabulary and its
--     ids are exactly the ones that can collide with each other (see
--     `LEDGER_TRANSACTION_PREFIX`'s doc and
--     `two_minted_ledger_ids_cannot_derive_the_same_entry_id`).
--
-- A wider bound on `ledger_entries` than on `ledger_transactions` was
-- considered and rejected: it would be a column whose domain is wider than
-- every other id column in this schema, which is the shape 0045 argued
-- against in the sentence this migration is closing.
--
--
-- DEPLOY ORDERING — CAN THE PREVIOUS RELEASE'S BINARY SERVE AGAINST THIS
-- SCHEMA?
--
-- Yes, and for a stronger reason than 0045 could give. 0045 argued that no
-- shipping statement touches these tables; that is still true of every
-- release cut before this one, because `Settlement::apply_succeeded` and
-- `apply_refund_succeeded` began posting in this same commit and not before.
-- A CHECK constraint also only ever narrows what an INSERT may carry, and the
-- tables are empty in every deployment, so `ALTER TABLE ... ADD CONSTRAINT`
-- validates zero rows and takes the lock for the time of a catalogue write.
--
-- Across a RESTART, no: `run_migrations()` never sets `ignore_missing`, so a
-- binary that does not carry version 46 refuses to boot against a database
-- that records it as applied — every migration in this repository has that
-- property (see 0032, 0037, 0045). Migrations here are forward-only; the way
-- to undo this is a later migration, never an edit to this file
-- (backends/migrations/README.md, issue #76).
--
--
-- DRIFT — measured, not predicted. `EXPECTED_DRIFT_CHANGES` in
-- `backends/tests/integration/tests/postgres_smoke.rs` goes 192 -> 194: two
-- hand-named, single-column CHECKs, which is the class that has cost every
-- table in this schema one line each. `EXPECTED_DRIFTED_RELATIONS` does not
-- move — `ledger_transactions` and `ledger_entries` are both already on it.

-- Named `id_length` on each table, which is the name `payment_intents`,
-- `charges`, `refunds`, `events`, `customers`, `checkout_sessions`,
-- `invoices`, `invoice_items` and `staff_sessions` all use for this exact
-- constraint. Constraint names are scoped to their table in Postgres, so the
-- two below do not collide with each other or with the nine above; using the
-- schema's existing name is what makes a `constraint()` assertion in a test
-- read the same way for every table that has one.
ALTER TABLE ledger_transactions
    ADD CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64);

ALTER TABLE ledger_entries
    ADD CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64);

COMMENT ON COLUMN ledger_transactions.id IS
    'Caller-minted lt_… (vpay_core::ids::ledger_transaction_id). Bounded by id_length since migration 0046 — see docs/flows/ledger.md.';
COMMENT ON COLUMN ledger_entries.id IS
    'Derived by vpay_db::ledger::post_in_tx as {transaction_id}_{index}, so it is longer than the id it comes from; bounded by id_length since migration 0046.';

-- Three table comments that have become false, corrected in a new migration
-- rather than by editing the files that wrote them — README.md's one rule, and
-- 0045's own precedent for exactly this.
--
-- `refunds` was commented 'NOT WRITTEN OR READ BY ANY CODE IN THIS REPOSITORY'
-- by 0017 and that was true until this commit. `vpay_db::Refunds::create` is
-- the first INSERT; `get_for_merchant`/`list_for_intent` have been reading the
-- table since issue #45. What has NOT changed, and is what the new text says
-- instead of the old claim, is that no rail can execute a refund and no route
-- reaches the writer — overstating this is the one thing docs/status.md and
-- CLAUDE.md both warn against, so the comment states the caller, not the
-- capability.
COMMENT ON TABLE refunds IS
    'Refunds. Written by vpay_db::Refunds::create/cancel and vpay_db::settlement (RFC-0003 § 3) and read by vpay_db::Refunds. NO RAIL CAN EXECUTE ONE: ProviderAdapter::refund is NotImplemented on both adapters and POST /v1/refunds is unrouted — see docs/status.md.';

-- And the two ledger tables, which 0045 commented 'Written by
-- vpay_db::ledger::post_in_tx, which NO shipping code path calls yet'. Two
-- call sites call it now, both in vpay_db::settlement, each inside the
-- transaction that settles the thing it records.
COMMENT ON TABLE ledger_transactions IS
    'Mirrors vpay_ledger::Transaction. Written by vpay_db::ledger::post_in_tx, called by Settlement::apply_succeeded (the capture) and apply_refund_succeeded (the refund) — see docs/flows/ledger.md.';
COMMENT ON TABLE ledger_entries IS
    'Mirrors vpay_ledger::Entry, including the merchant dimension AccountKind::MerchantPayable carries. Written by vpay_db::ledger::post_in_tx; the merchant is derived from the intent the charge belongs to at the call site, because no constraint can check it — see docs/flows/ledger.md § Status.';
