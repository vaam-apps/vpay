-- Currencies: mirrors vpay_core::Currency (backends/crates/vpay-core/src/money.rs)
-- and schemas/vpay.cstack's `Currency` model.
--
-- The exponent is a property of the currency itself, universally — see
-- docs/flows/money.md. XAF is zero-decimal (exponent 0); EUR is two-decimal
-- (exponent 2).

CREATE TABLE currencies (
    -- ISO-4217 alphabetic code, e.g. 'XAF', 'EUR'. The `code_is_iso4217_shape`
    -- CHECK is a shape check (three uppercase letters), not a membership check
    -- against the real ISO-4217 list — the same limit CrateStack's own
    -- `@iso4217` validator has (schemas/vpay.cstack: `@iso4217 @db_enforce`).
    code TEXT PRIMARY KEY,
    -- Minor units per major unit, as a power of ten. 0..4 covers every
    -- ISO-4217 currency in circulating use (most currencies use 2; a few use
    -- 0 or 3).
    exponent INT NOT NULL,

    CONSTRAINT code_is_iso4217_shape CHECK (code ~ '^[A-Z]{3}$'),
    CONSTRAINT exponent_in_range CHECK (exponent BETWEEN 0 AND 4)
);

COMMENT ON TABLE currencies IS
    'Mirrors vpay_core::Currency. Reference data — not admin-editable; see docs/flows/money.md.';
