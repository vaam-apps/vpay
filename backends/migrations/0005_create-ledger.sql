-- LedgerTransaction / LedgerEntry: mirrors backends/crates/vpay-ledger/src/lib.rs
-- (AccountKind, Direction, Transaction, Entry) field-for-field, and
-- schemas/vpay.cstack's `LedgerTransaction`/`LedgerEntry` models.
--
-- STATUS: no SQLx query in this repo writes to either table yet. This is the
-- intended persistence shape mirroring vpay_ledger's tested types, not
-- evidence that ledger persistence exists — see docs/status.md and
-- docs/flows/ledger.md.

-- backends/crates/vpay-ledger/src/lib.rs — AccountKind. Exactly three
-- variants, with no per-merchant dimension — see the GAP note below.
CREATE TYPE account_kind AS ENUM (
    'merchant_payable',
    'payer_clearing',
    'platform_fee_revenue'
);

-- backends/crates/vpay-ledger/src/lib.rs — Direction.
CREATE TYPE direction AS ENUM ('debit', 'credit');

CREATE TABLE ledger_transactions (
    -- vpay_ledger has no persisted id type yet (schemas/vpay.cstack modelled
    -- `Cuid @default(dbgenerated())`, but Postgres has no native CUID
    -- generator, and inventing one to fake a default would be exactly the
    -- plausible-but-fabricated failure mode this schema avoids elsewhere).
    -- The id is supplied by the caller; nothing in this repo calls this yet.
    id TEXT PRIMARY KEY,
    charge_id TEXT NOT NULL REFERENCES charges (id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()

    -- GAP — invariant 1 in docs/flows/ledger.md, "per transaction:
    -- SUM(debit) = SUM(credit), per currency", is an *aggregate* invariant
    -- over every ledger_entries row sharing a transaction_id. A row-level
    -- CHECK constraint cannot express it under any schema grammar — it
    -- evaluates one row at a time. This is deliberately NOT faked with a
    -- constraint trigger here: application enforcement already exists and is
    -- tested (vpay_ledger::Transaction::validate(), covered by
    -- a_capture_with_a_fee_balances / an_unbalanced_transaction_is_rejected),
    -- and a hand-written deferred constraint trigger would be new,
    -- unexercised logic duplicating that same check in SQL with no test
    -- proving it fires — the exact failure mode CLAUDE.md warns against. If
    -- persistence lands and this becomes a real gap in practice, add the
    -- trigger then, with a test that proves it rejects an unbalanced insert.
);

CREATE TABLE ledger_entries (
    id TEXT PRIMARY KEY,
    transaction_id TEXT NOT NULL REFERENCES ledger_transactions (id),
    account account_kind NOT NULL,
    direction direction NOT NULL,
    amount BIGINT NOT NULL,
    currency_code TEXT NOT NULL REFERENCES currencies (code),

    CONSTRAINT amount_non_negative CHECK (amount >= 0)

    -- GAP — docs/flows/ledger.md invariant 2, "per merchant:
    -- balance(merchant_payable) = Σ captures − Σ fees − Σ refunds", cannot be
    -- computed from this table as modelled: AccountKind has no per-merchant
    -- dimension, so nothing here says *which* merchant a merchant_payable
    -- posting belongs to. That is a gap in the Rust type this table mirrors,
    -- not something to paper over with a merchant_id column the Rust side
    -- doesn't have.
);

CREATE INDEX ledger_entries_transaction_id_idx ON ledger_entries (transaction_id);

COMMENT ON TABLE ledger_transactions IS
    'Mirrors vpay_ledger::Transaction. Not yet written to by any code path — see docs/status.md.';
COMMENT ON TABLE ledger_entries IS
    'Mirrors vpay_ledger::Entry. Not yet written to by any code path — see docs/status.md.';
