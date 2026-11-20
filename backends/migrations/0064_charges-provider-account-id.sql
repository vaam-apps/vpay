-- charges.provider_account_id: written with every charge, resolved from the
-- charge (RFC-0009 § 3, issue #290 Phase A2).
--
-- Phase A deployments hold one account per rail at rail level (`default`).
-- Recording `provider_account_id` now means Phase B never has to guess which
-- account an old charge used. Backfilled to `default` for existing rows, then
-- the column DEFAULT is dropped so every new INSERT must name it explicitly.

ALTER TABLE charges
    ADD COLUMN provider_account_id TEXT NOT NULL DEFAULT 'default';

ALTER TABLE charges
    ALTER COLUMN provider_account_id DROP DEFAULT;

COMMENT ON COLUMN charges.provider_account_id IS
    'Provider account identifier used for this charge (RFC-0009). Phase A always writes ''default''; explicit on every INSERT (no column DEFAULT).';
