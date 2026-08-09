-- Providers: mirrors vpay_provider::Capabilities plus the provider's identity
-- (backends/crates/vpay-provider/src/lib.rs) and schemas/vpay.cstack's
-- `Provider` model.

-- backends/crates/vpay-core/src/state.rs — ProviderFlow, transcribed
-- variant-for-variant in its #[serde(rename_all = "snake_case")] wire form.
CREATE TYPE provider_flow AS ENUM ('push', 'redirect');

CREATE TABLE providers (
    -- e.g. 'mtn_momo', 'orange_money'.
    code TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    flow provider_flow NOT NULL,
    supports_refunds BOOLEAN NOT NULL DEFAULT FALSE,
    supports_partial_refunds BOOLEAN NOT NULL DEFAULT FALSE,
    delivers_callbacks BOOLEAN NOT NULL DEFAULT FALSE,
    requires_ip_allowlist BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,

    CONSTRAINT code_length CHECK (char_length(code) BETWEEN 1 AND 64),
    CONSTRAINT display_name_length CHECK (char_length(display_name) BETWEEN 1 AND 128),

    -- `supports_partial_refunds ⇒ supports_refunds`.
    --
    -- schemas/vpay.cstack's GAP comment on `Provider` explains why this is
    -- NOT expressed there: CrateStack's `@db_enforce` only promotes a
    -- single-field `@range`/`@length`/`@iso4217` validator to a column-level
    -- CHECK; there is no `@@check(expr)` or other cross-column boolean
    -- constraint in that grammar. Raw SQL has no such limitation, so this is
    -- a genuine improvement over the .cstack design sketch — see this
    -- migration's note in the task report. Until now this invariant was
    -- Rust-only (`Capabilities::is_coherent` in
    -- backends/crates/vpay-provider/src/lib.rs, tested by
    -- `partial_refunds_imply_refunds`); docs/flows/configuration.md's
    -- correction describing it as "Rust-only" is now itself one step behind
    -- this migration and should be updated again.
    CONSTRAINT partial_refunds_imply_refunds
        CHECK (NOT supports_partial_refunds OR supports_refunds)
);

COMMENT ON TABLE providers IS
    'Mirrors vpay_provider::Capabilities. Reference/config data — reconciliation from YAML is not implemented yet (docs/flows/configuration.md).';
