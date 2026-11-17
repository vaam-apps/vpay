-- audit_events: append-only generic audit journal for privacy-sensitive and
-- security-sensitive operations (ADR-0020 §3, ISO 27001 A.12.4).
--
-- Provisional pending RFC-0002 D7: initiated + terminal audit records,
-- vocabulary ('account_holder:lookup', ...). See PR #56/152.
--
-- rate_limit_windows: extends rate_limit_windows_scope_is_known check constraint
-- with 'lookup:merchant_key' (provisional pending RFC-0002 D6).

ALTER TABLE rate_limit_windows DROP CONSTRAINT rate_limit_windows_scope_is_known;

ALTER TABLE rate_limit_windows ADD CONSTRAINT rate_limit_windows_scope_is_known CHECK (
    scope IN (
        'sign_in:email',
        'sign_in:address',
        'change_password:session',
        'lookup:merchant_key'
    )
);

CREATE TABLE audit_events (
    -- aud_..., minted by vpay_core::ids::audit_event_id.
    id TEXT PRIMARY KEY,

    -- When the event occurred.
    occurred_at TIMESTAMPTZ NOT NULL,

    -- The tenant (merchant_id) owning this event.
    tenant TEXT NOT NULL,

    -- Actor kind: signer or machine_token (CallerCredential).
    actor_kind TEXT NOT NULL,

    -- Actor identifier: lowercase hex thumbprint for signer, client_id for machine_token.
    actor_id TEXT NOT NULL,

    -- Privacy/security action: account_holder:lookup.
    action TEXT NOT NULL,

    -- Outcome: initiated, found, not_found, invalid, unsupported, rate_limited, or rail_error.
    outcome TEXT NOT NULL,

    -- Target class: account_holder.
    target_class TEXT NOT NULL,

    -- Approved target representation: keyed HMAC-SHA256 hex digest, never raw PII (provisional pending RFC-0002 D3).
    target_repr TEXT NOT NULL,

    -- op_..., groups initiated and terminal rows of one operation.
    operation_id TEXT NOT NULL,

    CONSTRAINT audit_events_id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT audit_events_tenant_length CHECK (char_length(tenant) BETWEEN 1 AND 64),
    CONSTRAINT audit_events_actor_kind_is_known CHECK (
        actor_kind IN ('signer', 'machine_token')
    ),
    CONSTRAINT audit_events_actor_id_length CHECK (char_length(actor_id) BETWEEN 1 AND 128),
    CONSTRAINT audit_events_action_is_known CHECK (
        action IN ('account_holder:lookup')
    ),
    CONSTRAINT audit_events_outcome_is_known CHECK (
        outcome IN (
            'initiated',
            'found',
            'not_found',
            'invalid',
            'unsupported',
            'rate_limited',
            'rail_error'
        )
    ),
    CONSTRAINT audit_events_target_class_is_known CHECK (
        target_class IN ('account_holder')
    ),
    CONSTRAINT audit_events_target_repr_length CHECK (char_length(target_repr) BETWEEN 1 AND 128),
    CONSTRAINT audit_events_operation_id_length CHECK (char_length(operation_id) BETWEEN 1 AND 64)
);

-- Append-only, enforced by the database rather than by convention.
--
-- `vpay_db::audit_events` exposes no update or delete, but that is a Rust
-- convention: any statement on the same role could still rewrite or remove
-- a row. These two triggers refuse UPDATE, DELETE and TRUNCATE on the table
-- for every role, so a mistaken hand-written statement, a generated delegate
-- or an operator's ad-hoc SQL fails loudly instead of silently editing the
-- evidence. Proven against a real Postgres by `vpay-db`'s
-- `the_audit_journal_refuses_update_delete_and_truncate`.
--
-- What this is NOT: tamper-proofing against the table's owner. The owner (the
-- role migrations run as) can `ALTER TABLE ... DISABLE TRIGGER` or drop the
-- trigger, and a superuser can do anything. A retention handler (RFC-0002 D9,
-- not built) will need its own deliberate exception to this rule.
CREATE FUNCTION audit_events_refuse_mutation() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_events is append-only: % is refused', TG_OP
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER audit_events_append_only
    BEFORE UPDATE OR DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_refuse_mutation();

CREATE TRIGGER audit_events_no_truncate
    BEFORE TRUNCATE ON audit_events
    FOR EACH STATEMENT EXECUTE FUNCTION audit_events_refuse_mutation();

-- Queries filtering by tenant and ordered by time (ADR-0020 §3, bounded tenant reads).
CREATE INDEX audit_events_tenant_occurred_at_idx ON audit_events (tenant, occurred_at);

-- Correlating initiated and terminal events of one operation (ADR-0020 §3).
CREATE INDEX audit_events_operation_id_idx ON audit_events (operation_id);

COMMENT ON TABLE audit_events IS
    'Generic append-only audit and security-event journal (ADR-0020 §3, ISO 27001 A.12.4). Holds immutable records of privacy-sensitive actions with bounded target representations. Provisional pending RFC-0002 D7.';
COMMENT ON COLUMN audit_events.id IS
    'aud_..., minted by vpay_core::ids::audit_event_id.';
COMMENT ON COLUMN audit_events.occurred_at IS
    'When the event occurred.';
COMMENT ON COLUMN audit_events.tenant IS
    'The tenant (merchant_id) owning this event.';
COMMENT ON COLUMN audit_events.actor_kind IS
    'Actor kind: signer or machine_token (CallerCredential).';
COMMENT ON COLUMN audit_events.actor_id IS
    'Actor identifier: lowercase hex thumbprint for signer, client_id for machine_token.';
COMMENT ON COLUMN audit_events.action IS
    'Privacy/security action: account_holder:lookup.';
COMMENT ON COLUMN audit_events.outcome IS
    'Outcome: initiated, found, not_found, invalid, unsupported, rate_limited, or rail_error.';
COMMENT ON COLUMN audit_events.target_class IS
    'Target class: account_holder.';
COMMENT ON COLUMN audit_events.target_repr IS
    'Approved target representation: keyed HMAC-SHA256 hex digest, never raw PII (provisional pending RFC-0002 D3).';
COMMENT ON COLUMN audit_events.operation_id IS
    'op_..., groups initiated and terminal rows of one operation.';
