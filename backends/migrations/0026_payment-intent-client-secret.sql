-- payment_intents.client_secret_suffix: the payer-side credential for
-- `/v1/browser` (Step 5c, docs/plans/2026-09-03-step5c-stripejs.md §1).
--
-- WHY A SUFFIX AND NOT THE WHOLE SECRET
--
-- The credential a payer's browser presents is `pi_…_secret_<suffix>` —
-- `vpay_core::ids::client_secret`. Only the second half is stored, because
-- the first half is the row's own primary key: storing the joined string
-- would be the id written twice, and two copies of one value are two things
-- that can disagree. `vpay_api::browser::authenticate` rebuilds the expected
-- secret from `(id, suffix)` and compares it against what was presented.
--
-- WHY IT IS NOT A HASH
--
-- Every other credential this system holds is stored hashed or not at all
-- (ADR-0010: vpay stores no merchant secret in any form). This one is
-- different in kind: it is a **capability handed to the payer**, scoped to a
-- single payment intent, and vpay is the party that mints it and the party
-- that renders it back — `create` and `retrieve` return it in full, because
-- the merchant's own page has to be able to re-read it. A hash would make
-- that impossible without a second, plaintext copy somewhere else. The
-- exposure is bounded instead: 160 bits from the OS CSPRNG, one intent, a
-- uniform 404 on every failure, and one-charge-per-intent behind it.
--
-- BACKFILL
--
-- Two `gen_random_uuid()` draws, hyphens removed, concatenated: 64 hex
-- characters, inside the CHECK below, from pgcrypto's own CSPRNG (built into
-- core Postgres since 13). Every existing row is a `/v1`-created intent that
-- no browser has ever been able to address, so what the value *is* does not
-- matter — what matters is that it is unguessable and distinct per row, so
-- backfilled intents are exactly as safe as ones minted after this migration.
-- A constant default would have made every pre-existing intent share one
-- credential.
--
-- WHY THE COLUMN IS ADDED NULLABLE AND THEN TIGHTENED
--
-- `ADD COLUMN … NOT NULL DEFAULT gen_random_uuid()` would evaluate the
-- default **once** for the whole table under Postgres 11+'s fast-default
-- path, giving every existing row the same secret. Three statements — add,
-- backfill row by row, tighten — is what makes each row's value its own.
ALTER TABLE payment_intents ADD COLUMN client_secret_suffix TEXT;

UPDATE payment_intents
   SET client_secret_suffix =
           replace(gen_random_uuid()::text, '-', '')
        || replace(gen_random_uuid()::text, '-', '')
 WHERE client_secret_suffix IS NULL;

ALTER TABLE payment_intents
    ALTER COLUMN client_secret_suffix SET NOT NULL,
    -- 32 is what `vpay_core::ids::client_secret_suffix` mints (32 Crockford
    -- base32 characters = 160 bits); the backfill above writes 64. The
    -- ceiling is the same order as every other id CHECK in this schema and
    -- exists so a writer that put something *else* in this column — a whole
    -- `pi_…_secret_…`, a JSON blob — is refused rather than stored. The
    -- floor is the load-bearing half: a short suffix is a guessable
    -- credential, and this is the only place that cannot be bypassed by a
    -- future writer that forgets.
    ADD CONSTRAINT client_secret_suffix_length
        CHECK (char_length(client_secret_suffix) BETWEEN 32 AND 128);

COMMENT ON COLUMN payment_intents.client_secret_suffix IS
    'The second half of this intent''s payer-facing client_secret; the first half is `id`. Joined by vpay_core::ids::client_secret into `pi_…_secret_…`, rendered by POST/GET /v1/payment_intents and by /v1/browser, and NEVER by GET /v1/payment_intents (the list) or by events.data — see vpay_api::model::PaymentIntentWithSecret. Redacted in vpay_db::PaymentIntentRow''s hand-written Debug.';
