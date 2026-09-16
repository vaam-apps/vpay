-- Give `ledger_entries` the per-merchant dimension `vpay_ledger::AccountKind`
-- just grew, so that docs/flows/ledger.md invariant 2 — "per merchant:
-- balance(merchant_payable) = Σ captures − Σ fees − Σ refunds" — becomes a
-- query this table can answer. RFC-0003 § 4.
--
-- THIS IS THE SECOND HALF OF A CHANGE, NOT A SCHEMA PATCH. Migration 0005's
-- own GAP note refused a `merchant_id` column on exactly these grounds:
--
--     That is a real gap in the Rust type this table mirrors, not something
--     to paper over with a merchant_id column the Rust side doesn't have.
--
-- The Rust side has one now. `AccountKind::MerchantPayable` carries
-- `merchant_id: String` and the other two variants carry nothing, because
-- `payer_clearing` and `platform_fee_revenue` are vpay's own accounts, pooled
-- across every tenant. `ledger_entries_merchant_id_iff_merchant_payable`
-- below is that sum type, in SQL: the column is NOT NULL exactly when
-- `account = 'merchant_payable'`, so neither of the two shapes the Rust type
-- makes unrepresentable is storable either.
--
--
-- DEPLOY ORDERING — CAN THE PREVIOUS RELEASE'S BINARY SERVE AGAINST THIS
-- SCHEMA?
--
-- **While it keeps running, yes, completely, and this migration is unusual in
-- how easy that is to establish.** No binary in any release vpay has cut
-- reads or writes `ledger_entries` or `ledger_transactions` at all. Before
-- RFC-0003 § 4 the two table names appeared in hand-written Rust in exactly
-- three places, and every one of them is inside a `#[cfg(test)]` module, so
-- none is even linked into a shipping binary, let alone issued by one:
--
--   * `vpay-db/src/schema.rs`'s `MODEL_TABLES`, the nineteen-name list
--     `no_generated_model_route_is_mounted_only_the_one_procedure_is` probes;
--   * an `assert!` message in `config_reconcile.rs`'s tests, naming
--     `ledger_entries` as one of the tables whose foreign key onto
--     `currencies` is why nothing deletes a currency;
--   * `backends/tests/integration/tests/postgres_smoke.rs`.
--
-- **A grep of `*.rs` is not the whole argument, and the half it misses is the
-- half worth stating.** `schemas/vpay.cstack` declares `model
-- LedgerTransaction` and `model LedgerEntry`, and `vpay-db`'s private `mod
-- schema` compiles that file — so the previous release's binary *does* carry
-- generated CRUD and generated row decoders for both tables, in a module that
-- exists in no source file and that no grep of this repository can find (see
-- CLAUDE.md, and `docs/reference/vpay-db/cratestack.md`). That generated code
-- is unreachable rather than absent: `schema.rs::dashboard_procedure_router`
-- mounts `cratestack_schema::axum::procedure_router(...)` and never
-- `router()`, so the generated model routes are not mounted, and the only
-- procedure that is mounted is `searchPaymentIntents`. The first of the three
-- tests above is what pins that, by probing all seventy-six
-- `/{plural}`/`/{plural}/{id}` × four-method paths for a 404.
--
-- There is therefore no in-flight failure mode of 0037's kind to drain for:
-- an old process serving a confirm, a settlement, a cancel or any read
-- whatsoever touches no statement this migration changes, because this
-- migration changes no statement any old process issues. Nothing here alters
-- an existing column, an existing type or an existing constraint; it adds one
-- nullable column, two constraints on it and one partial index.
--
-- Unlike 0037, which measured its claim by building the previous release's
-- binary and running it against a migrated database, this half is argued
-- rather than measured — deliberately, because there is no statement to run:
-- the measurement 0037 took exists precisely to find out which statements
-- break, and the claim here is that the set of candidate statements is empty.
-- What that leaves unmeasured is the generated surface above, which is why
-- its unreachability is named as a test rather than asserted.
--
-- **Across a RESTART, no — and that is 0032's and 0037's property, shared by
-- every migration in this repository rather than created by this one.**
-- `run_migrations()` never sets `ignore_missing`, so
-- `Migrator::validate_applied_migrations` refuses to boot a binary that does
-- not carry a version the database records as applied
-- (`sqlx-core-0.9.0/src/migrate/migrator.rs:363-380`):
--     migration 45 was previously applied but is missing in the resolved
--     migrations   (`sqlx::MigrateError::VersionMissing(45)`)
-- So a rollback past this release does not serve; a rolling deploy across it
-- does. Unlike 0037 there is nothing to drain first.
--
-- Migrations here are forward-only: there is no `down.sql`, and the way to
-- undo this is a later migration, never an edit to this file
-- (backends/migrations/README.md, issue #76).
--
--
-- WHY A COLUMN AT ALL, WHEN THE MERCHANT WAS ALREADY DERIVABLE
--
-- It was. `ledger_entries.transaction_id -> ledger_transactions.charge_id ->
-- charges.payment_intent_id -> payment_intents.merchant_id` is a three-hop
-- join that answers "whose posting is this?" against the schema exactly as it
-- stood before this migration. So migration 0005's GAP note — repeated in
-- docs/flows/ledger.md and in the opening line of this file — is precise about
-- `vpay_ledger::AccountKind`, which carried no merchant at all and genuinely
-- could not compute invariant 2, and overstated about the *table*, which
-- could. Read "becomes a query this table can answer" above as "answers it
-- without leaving the ledger", which is the claim that is actually true.
--
-- The reason to store it anyway is not that the value is unavailable but that
-- the derivation is not stable. A ledger records what was true when the money
-- moved; a balance computed through `charges` and `payment_intents` is a
-- balance that silently changes whenever an operational row does, and a
-- historical figure that moves under a later UPDATE is not a ledger. This
-- column pins the posting's merchant at write time. The partial index below
-- is the second reason and much the smaller one.
--
-- THE COST OF THAT CHOICE, STATED HERE RATHER THAN DISCOVERED LATER: nothing
-- constrains `ledger_entries.merchant_id` to agree with the merchant of the
-- charge its transaction names. No SQL constraint can — the fact is three
-- tables away, and a CHECK sees one row — and `vpay_db::ledger::post_in_tx`
-- does not check it either: it binds whatever
-- `AccountKind::MerchantPayable` was built with. Whoever assembles the
-- posting is the only guarantor that a merchant's balance is made of that
-- merchant's charges, and that is a property the call sites owe a test, not
-- something this file can enforce.
--
--
-- WHY NULLABLE, AND WHY THAT IS NOT A HOLE
--
-- A `NOT NULL` column would be wrong rather than merely inconvenient: two of
-- the three accounts have no merchant, and a sentinel (`''`, `'-'`, the
-- platform's own id) would make "which merchant" answerable with a value that
-- names no merchant. The pair CHECK is what stops the nullability from being
-- a hole — it is the constraint that makes `merchant_id IS NULL` mean
-- "a pooled account" and nothing else.
--
-- The table is empty in every deployment (see the deploy-ordering note
-- above), so there is no backfill here and none is possible to get wrong. A
-- future deployment with rows would need the column added nullable, backfilled
-- from `ledger_transactions -> charges -> payment_intents.merchant_id`, and
-- only then constrained; that ordering is recorded here because the absence of
-- a backfill in this file is a fact about today's data, not a general licence.
--
--
-- WHAT THIS MIGRATION DOES NOT FIX, AND IS THE OBVIOUS PLACE TO HAVE FIXED
--
-- `ledger_transactions.id` and `ledger_entries.id` carry no
-- `CHECK (char_length(id) BETWEEN 1 AND 64)`. `payment_intents`, `charges`,
-- `refunds`, `events`, `customers`, `checkout_sessions`, `credentials`,
-- `invoices`, `invoice_items`, `merchant_api_keys`, `oauth_signing_keys`,
-- `rate_limit_windows`, `staff_members` and `staff_sessions` all bound their
-- caller-supplied id; migration 0005 gave these two none, and THIS migration
-- adds `ledger_entries_merchant_id_length` to a *new* column on exactly the
-- grounds that would demand one — "a copy with a wider domain than its source
-- is a copy that can hold something the source could not". A primary key with
-- no source at all has a wider domain than any of them. They also have no
-- minter:
-- `vpay_core::ids` closes the `pi_`/`ch_`/`re_`/`evt_`/`cs_`/`cus_`/`in_`/
-- `ii_`/`stf_`/`cred_` vocabulary and mints a prefix even for ids no surface
-- ever renders, and `ledger_transactions.id` is supplied by the caller with
-- no prefix, no shape check and nothing to check it against.
--
-- Left alone here rather than fixed, for one reason and it is not a good one:
-- two more single-column CHECKs would move `EXPECTED_DRIFT_CHANGES` by +2 and
-- that constant may only be moved by a measurement, which needs the pinned
-- `cratestack` 0.12.0 this branch could not run. Recorded so the gap is
-- visible rather than silent; it is a maintainer's call whether the id
-- vocabulary grows an `lt_`/`le_` prefix or the ledger stays caller-named,
-- and whichever way that goes, the bound belongs in the same migration.
--
--
-- DRIFT — measured, not predicted, on 2026-09-15 against a freshly migrated
-- Postgres 16 with `cratestack migrate baseline --strict`: see
-- `EXPECTED_DRIFT_CHANGES` in
-- `backends/tests/integration/tests/postgres_smoke.rs`, whose note carries the
-- before/after numbers and the per-line account of them. `model LedgerEntry`
-- in `schemas/vpay.cstack` declares the column in this same commit, which is
-- what keeps the column itself off the report.

ALTER TABLE ledger_entries ADD COLUMN merchant_id TEXT;

-- The sum type, in SQL. Multi-column, so `cratestack migrate baseline` cannot
-- see it in either direction (`introspect/postgres/constraints.rs` filters
-- `array_length(c.conkey, 1) = 1`) — which is why
-- `a_merchant_payable_entry_must_name_its_merchant` and
-- `a_pooled_account_entry_must_not_name_a_merchant` in `postgres_smoke.rs`
-- write the two rows it refuses instead of trusting the drift report to
-- notice it.
ALTER TABLE ledger_entries
    ADD CONSTRAINT ledger_entries_merchant_id_iff_merchant_payable
    CHECK ((account = 'merchant_payable') = (merchant_id IS NOT NULL));

-- `payment_intents.merchant_id`'s own bound (migration 0003), because this
-- column holds a copy of that value and a copy with a wider domain than its
-- source is a copy that can hold something the source could not.
ALTER TABLE ledger_entries
    ADD CONSTRAINT ledger_entries_merchant_id_length
    CHECK (merchant_id IS NULL OR char_length(merchant_id) BETWEEN 1 AND 128);

-- The index invariant 2 is read through: `balance(merchant_payable)` for one
-- merchant in one currency. PARTIAL on the account, because every row this
-- predicate excludes has `merchant_id IS NULL` and would be dead weight in
-- the index; `(merchant_id, currency_code)` in that order because the
-- merchant is always given and the currency narrows within it.
CREATE INDEX ledger_entries_merchant_payable_idx
    ON ledger_entries (merchant_id, currency_code)
    WHERE account = 'merchant_payable';

COMMENT ON COLUMN ledger_entries.merchant_id IS
    'Whose money a merchant_payable posting is; NULL on the two pooled accounts. Mirrors vpay_ledger::AccountKind::MerchantPayable.merchant_id — see docs/flows/ledger.md invariant 2.';

-- Migration 0005 left both tables commented 'Not yet written to by any code
-- path'. That is no longer the whole truth and the correction belongs in a new
-- migration, never in an edit to 0005 (README.md's one rule). What changed is
-- narrow, and the new text says exactly how narrow: `vpay-db` now carries the
-- INSERT (`TxRepositories::post_ledger_transaction_in_tx`), and no shipping
-- code path calls it — `Settlement::apply_succeeded` and
-- `apply_refund_succeeded` do not post, so every deployment's ledger is still
-- empty. Overstating this is the one thing docs/status.md and CLAUDE.md both
-- warn against, so the comment states the caller, not the capability.
COMMENT ON TABLE ledger_transactions IS
    'Mirrors vpay_ledger::Transaction. Written by vpay_db::ledger::post_in_tx, which NO shipping code path calls yet — see docs/status.md and docs/flows/ledger.md.';
COMMENT ON TABLE ledger_entries IS
    'Mirrors vpay_ledger::Entry, including the merchant dimension AccountKind::MerchantPayable carries. Written by vpay_db::ledger::post_in_tx, which NO shipping code path calls yet — see docs/status.md.';
