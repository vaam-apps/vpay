-- 0063: bound break-glass console keys below the ordinary 30 days, and widen
-- the audit journal so a break-glass pair/revoke leaves an attributable row
-- (issue #152, GAP-152-BREAK-GLASS; ISO 27001 A.8.2).
--
-- WHAT CHANGES, AND WHY
--
--   * `console_keys_lives_exactly_thirty_days` pinned EVERY console key to
--     `paired_at + 720 hours`. That is right for a phone pairing through vaam
--     (owner ruling R3, ADR-0036), but it left the operator's break-glass
--     command (`vpay-server staff pair-key`) with no way to hand out a key that
--     outlives the incident it was paired for — the gap's "no shorter expiry".
--     The constraint becomes an UPPER BOUND: a key may live up to 720 hours and
--     no longer, and at least an instant. The ordinary pairing still writes
--     exactly 720 hours (`vpay_db::CONSOLE_KEY_LIFETIME`); a break-glass pairing
--     writes the operator's requested, shorter life
--     (`vpay_api::console_keys::BREAK_GLASS_KEY_LIFETIME` by default).
--
--   * `audit_events`' vocabulary CHECKs admitted only an account-holder lookup
--     by a signer or a machine token, so a break-glass pairing could not be
--     journalled at all — the gap's "no audit row". The action, actor-kind,
--     outcome and target-class vocabularies each gain the break-glass names
--     below. The table stays append-only (0060's triggers are untouched), and
--     no column is added, so the generated model and the privacy inventory are
--     unchanged in shape.
--
-- 0056 and 0060 are APPLIED and are never edited: this migration supersedes the
-- one constraint and re-states the four the vocabulary grew, and the column
-- COMMENTs are overwritten here so an operator reading the live schema sees the
-- new rules rather than 0056's/0060's frozen ones.

-- console_keys: an upper bound on life, not an equality.
ALTER TABLE console_keys DROP CONSTRAINT console_keys_lives_exactly_thirty_days;

ALTER TABLE console_keys ADD CONSTRAINT console_keys_lives_at_most_thirty_days CHECK (
    expires_at > paired_at
    AND expires_at <= paired_at + INTERVAL '720 hours'
);

COMMENT ON TABLE console_keys IS
    'A person''s registered console signing key (VP-6a, ADR-0036): a P-256 public key whose private half lives non-extractable in a browser, paired by vaam through POST /svc/v1/console_keys or by an operator with vpay-server staff pair-key. Lives AT MOST 720 hours from paired_at (console_keys_lives_at_most_thirty_days): the vaam pairing writes exactly 720 hours, a break-glass pairing writes a shorter life (issue #152). Never extended, revoked_at is set once, never deleted.';
COMMENT ON COLUMN console_keys.expires_at IS
    'At most paired_at plus 720 hours and strictly after paired_at (console_keys_lives_at_most_thirty_days). Ordinary vaam pairing writes paired_at + 720 hours; vpay-server staff pair-key writes a shorter, operator-chosen life (issue #152). A request signed under an expired key is the layer''s coarse 401.';

-- audit_events: admit the break-glass pair and revoke that #152 introduced.
ALTER TABLE audit_events DROP CONSTRAINT audit_events_actor_kind_is_known;

ALTER TABLE audit_events ADD CONSTRAINT audit_events_actor_kind_is_known CHECK (
    actor_kind IN ('signer', 'machine_token', 'operator')
);

ALTER TABLE audit_events DROP CONSTRAINT audit_events_action_is_known;

ALTER TABLE audit_events ADD CONSTRAINT audit_events_action_is_known CHECK (
    action IN ('account_holder:lookup', 'console_key:pair', 'console_key:revoke')
);

ALTER TABLE audit_events DROP CONSTRAINT audit_events_outcome_is_known;

ALTER TABLE audit_events ADD CONSTRAINT audit_events_outcome_is_known CHECK (
    outcome IN (
        'initiated',
        'found',
        'not_found',
        'invalid',
        'unsupported',
        'rate_limited',
        'rail_error',
        'paired',
        'revoked'
    )
);

ALTER TABLE audit_events DROP CONSTRAINT audit_events_target_class_is_known;

ALTER TABLE audit_events ADD CONSTRAINT audit_events_target_class_is_known CHECK (
    target_class IN ('account_holder', 'console_key')
);

COMMENT ON COLUMN audit_events.actor_kind IS
    'Actor kind: signer, machine_token (CallerCredential), or operator (a break-glass command line, issue #152).';
COMMENT ON COLUMN audit_events.actor_id IS
    'Actor identifier: lowercase hex thumbprint for a signer, client_id for a machine_token, or the operator identity named by --operator for a break-glass command (at most 128 characters, issue #152).';
COMMENT ON COLUMN audit_events.action IS
    'Privacy/security action: account_holder:lookup, console_key:pair, or console_key:revoke.';
COMMENT ON COLUMN audit_events.outcome IS
    'Outcome: initiated, found, not_found, invalid, unsupported, rate_limited, rail_error for a lookup; paired, revoked for a break-glass console-key operation.';
COMMENT ON COLUMN audit_events.target_class IS
    'Target class: account_holder or console_key.';
COMMENT ON COLUMN audit_events.target_repr IS
    'Approved target representation: a keyed HMAC-SHA256 hex digest for an account-holder lookup, or a console key''s RFC 9679 thumbprint for a break-glass pair/revoke. Never raw PII.';
