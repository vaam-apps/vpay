-- Reshapes oauth_signing_keys so no private key material is ever persisted.
--
-- Decision (post-0006..0008): the RS256 private key comes from a Kubernetes
-- Secret via env at process boot and is never written to the database.
-- Verified upstream fact this relies on: `authkestra_engine::TokenManager::
-- new_asymmetric(pem, issuer, kid)` (~/.cargo/registry/.../authkestra-engine-
-- 0.3.4/src/token/mod.rs) parses the PEM exactly once, at construction —
-- `EncodingKey::from_rsa_pem` and the derived `rsa::RsaPrivateKey` are both
-- consumed synchronously inside that function body to build the struct's
-- `encoding_key`/`decoding_key`/`public_jwk` fields; the PEM bytes themselves
-- are not retained anywhere on `TokenManager`. So the database never needs
-- the PEM again once a key has been loaded — it only needs enough to publish
-- `/jwks.json` across a rotation window: the public half, its `kid`, and the
-- validity window.
--
-- CHOICE: ALTER, not drop-and-recreate. Both are safe here — nothing has
-- ever been deployed (docs/status.md) and the integration tests run against
-- ephemeral testcontainers, so there is no real data to migrate either way.
-- ALTER is chosen because it is the smaller, more honest diff: the table's
-- identity, its unrelated constraints (`one_active_signing_key`,
-- `active_key_has_no_expiry`, `expiry_after_creation`) and its indexes carry
-- forward untouched, and this migration's body says exactly what changed —
-- one column swapped for another, one column renamed — rather than
-- restating the whole table as if everything about it were new.
--
-- `id` is renamed to `kid`: the column always meant "the JWT header `kid`
-- this key signs under" (it is literally the third argument to
-- `TokenManager::new_asymmetric`), and now that this table's job is
-- explicitly "back /jwks.json", giving the column the name every other piece
-- of this flow already uses (`Jwk.kid`, `new_asymmetric`'s `kid` parameter)
-- is more honest than the generic `id` it shipped with.
--
-- `ADD COLUMN public_jwk JSONB NOT NULL` with no DEFAULT is only valid
-- against an empty table — true in every environment this migration will
-- ever run against (fresh testcontainers in CI, and no real deployment
-- exists yet per docs/status.md). If that stops being true, this migration
-- must be revisited before being applied to a database with real rows.

ALTER TABLE oauth_signing_keys DROP CONSTRAINT private_key_pem_looks_like_pem;
ALTER TABLE oauth_signing_keys DROP COLUMN private_key_pem;
ALTER TABLE oauth_signing_keys ADD COLUMN public_jwk JSONB NOT NULL;

ALTER TABLE oauth_signing_keys RENAME COLUMN id TO kid;
ALTER TABLE oauth_signing_keys RENAME CONSTRAINT id_length TO kid_length;

COMMENT ON TABLE oauth_signing_keys IS
    'vpay-owned RS256 signing-key rotation store for the /dash/v1 OP. Not part of authkestra-op''s schema — authkestra ships no key/rotation type at all. Stores no secret material: the private key comes from a Kubernetes Secret via env at boot and is parsed once by authkestra_engine::TokenManager::new_asymmetric, never persisted here. Only the public JWK is stored, so /jwks.json can publish every currently-valid key during a rotation window.';
COMMENT ON COLUMN oauth_signing_keys.public_jwk IS
    'The public half of an RS256 signing key, as the JWK authkestra_engine::TokenManager derives it (kty/alg/n/e/kid). No private key material is stored anywhere in this table or this repository.';
COMMENT ON COLUMN oauth_signing_keys.kid IS
    'The JWT header `kid` this key signs/verifies under — the same value passed as TokenManager::new_asymmetric''s third argument. Renamed from `id` to match that usage.';
