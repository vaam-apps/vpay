-- credentials: how a subject proved who they are, as its own object
-- (ADR-0019), and the five columns it takes off `staff_members`.
--
-- WHY THIS IS ONE MIGRATION AND NOT TWO
--
-- `sqlx::migrate!` runs each file in a transaction, so the create, the
-- backfill and the drop below are one atomic step. THERE IS NO INSTANT AT
-- WHICH THE MATERIAL HAS LEFT `staff_members` AND NOT ARRIVED IN
-- `credentials` — which is the whole requirement: every live row has a
-- password hash and possibly a TOTP secret, and nobody may be locked out for
-- any window at all.
--
-- THE MATERIAL IS COPIED BYTE FOR BYTE AND NO SECRET IS RE-DERIVED. That is
-- what makes "nobody is locked out" true rather than hoped: the argon2id PHC
-- string verifies under the same unchanged `staff_auth.password_pepper`, and
-- the sealed RFC 6238 secret opens under the same unchanged
-- `staff_auth.totp_encryption_key`. A migration that re-hashed anything would
-- be a migration that logged everyone out, and there is nothing here that
-- could.
--
-- WHY THE DROP IS IN THE SAME FILE
--
-- Leaving the five columns behind as a "safety net" would leave two sources
-- of truth for a password hash, and the one nobody reads is the one that goes
-- stale and then gets read by accident. A migration is never edited
-- afterwards (`just verify-migrations` refuses changed bytes), so the
-- rollback story is a NEW migration either way; keeping dead credential
-- columns would not improve it and would meanwhile leave a live argon2id
-- digest in a column no code maintains.
--
-- THE SHAPE IS MIGRATION 0035'S, TO THE LETTER
--
--   * NO `DEFAULT` on any column a writer names. `cratestack-macros` drops
--     every `@default(...)` field from `Create{Model}Input`, so a defaulted
--     column is one `CreateCredentialInput` could not set.
--   * NO `seq` IDENTITY COLUMN. Nothing paginates credentials.
--   * NO NATIVE POSTGRES ENUM. CrateStack decodes every enum column with
--     `try_get::<String>()` and a native enum fails to decode on every read
--     (upstream #228, the defect migration 0032 had to convert
--     `providers.flow` out of). `kind` is TEXT with a hand-named CHECK.
--   * NO `BYTEA` AND NO `JSONB`. Neither is in `cratestack-migrate`'s
--     `map_scalar`, so a column of either is excluded from the drift
--     comparison OUTRIGHT, in both directions. Per-kind material in a `jsonb`
--     blob is the obvious design for a table like this and it silently leaves
--     the measurement. A WebAuthn public key will be base64url TEXT, exactly
--     as the sealed TOTP secret already is.

-- HOW A SUBJECT PROVED WHO THEY ARE.
CREATE TABLE credentials (
    -- `cred_…`, minted by `vpay_core::ids::credential_id` before the insert,
    -- exactly as `stf_…` and `cus_…` are. The rows this migration backfills
    -- derive theirs deterministically from the staff id (see the INSERTs
    -- below) rather than randomly, and the derivation is chosen so that
    -- `ids::is_well_formed` accepts a migrated row exactly as it accepts a
    -- minted one.
    id TEXT PRIMARY KEY,

    -- WHICH SUBJECT. A real FK with ON DELETE CASCADE: a credential that
    -- outlives its subject is an authenticator with nobody to authenticate,
    -- and this is the shape `staff_sessions.staff_id` and
    -- `oauth_authorization_codes.staff_id` already use on this same table.
    --
    -- NULLABLE, although `credentials_has_exactly_one_subject` below makes it
    -- effectively NOT NULL today. ADR-0019 decision 2 takes the fork — a
    -- nullable FK per subject type, NOT `subject_kind` + `subject_id` — and
    -- this column is written for the second subject type before it exists:
    -- adding customers is one nullable `customer_id`, one FK, and one
    -- replaced CHECK.
    --
    -- WHAT A CASCADE DOES NOT COVER, stated here because it will be read here
    -- first. `Customers::erase_in_tx` (issue #111, closed in #143) hard-deletes
    -- a customer nothing references and ANONYMISES IN PLACE one that an
    -- intent, a session or an invoice references — those FKs are NO ACTION on
    -- purpose, so a payment is never detached from its payer. Any customer
    -- who has ever paid is in the second branch and NO CASCADE FIRES FOR
    -- THEM. When `customer_id` arrives, `erase_in_tx` must carry an explicit
    -- `DELETE FROM credentials WHERE customer_id = $1` in the same
    -- transaction.
    staff_member_id TEXT REFERENCES staff_members (id) ON DELETE CASCADE,

    -- One of eight (`credentials_kind_is_known` below). DECLARING A KIND IS
    -- NOT IMPLEMENTING IT: exactly two — 'password' and 'totp' — are
    -- reachable by any code path in this deployment. The other six are shapes
    -- this table can hold; nothing mints them and `vpay_api::staff_auth` has a
    -- verifier for two. `docs/status.md` carries the gap.
    kind TEXT NOT NULL,

    -- THE SECRET, OPAQUE. An argon2id PHC string for 'password'; the
    -- AES-256-GCM sealed RFC 6238 secret, nonce-prefixed and base64url, for
    -- 'totp'. `vpay_db` never hashes, never verifies and never decrypts it.
    --
    -- THE PEPPER IS NOT IN THIS COLUMN AND MUST NEVER BE, and neither is the
    -- TOTP encryption key. Both are deployment secrets
    -- (`staff_auth.password_pepper`, `staff_auth.totp_encryption_key`,
    -- refused at boot in livemode when absent). LOSING EITHER INVALIDATES
    -- EVERY ROW THIS COLUMN HOLDS FOR THE CORRESPONDING KIND — migration
    -- 0035 said so of the two columns this one replaces and it is no less
    -- true here.
    --
    -- NULLABLE, and that nullability is the FEDERATION requirement's whole
    -- footprint on this table. A federated identity IS NOT A SECRET: an
    -- 'oidc' row holds (iss, sub), which is public, and its verification
    -- material is the issuer's JWKS, which is fetched and not stored. A NOT
    -- NULL here would force a non-secret into a secret-shaped hole, which is
    -- how an issuer ends up in a `password_hash` column. Which kinds may and
    -- may not be NULL is `credentials_federated_carries_identity_and_no_material`
    -- below, so the nullability is per-kind rather than free.
    material TEXT,

    -- THE FEDERATED IDENTITY. NULL for every kind but 'oidc', and NOT NULL
    -- for that one.
    --
    -- WHICH ISSUERS A DEPLOYMENT ACCEPTS IS CONFIGURATION, NOT DATA, AND THIS
    -- COLUMN IS NOT AN ALLOW-LIST. If trust were decided by a table a
    -- credential row could point at freely, a row naming an attacker's issuer
    -- would be a VALID CREDENTIAL: they sign their own token, their own JWKS
    -- validates it, and vpay believes it. The accepted issuers live in the
    -- vpay configuration document beside the rest of `staff_auth`; a row
    -- whose issuer is not in that configuration verifies nothing.
    --
    -- 512 matches `staff_members.email`. An Issuer Identifier is an https URL
    -- with no query or fragment, the specification sets no length, and the
    -- longest real shapes
    -- (`https://login.microsoftonline.com/<tenant-guid>/v2.0`) are under a
    -- hundred.
    issuer TEXT,

    -- The IdP's `sub`. MATCHED ON; the email in a token never is, and is
    -- treated as non-authoritative display data. An email is mutable, may
    -- arrive unverified, and inside a tenant is reassignable — a departing
    -- employee's address handed to their replacement is normal IT hygiene and
    -- would be an account takeover if it were the join key.
    --
    -- 255 is OpenID Connect Core 1.0 §2's OWN CEILING — a `sub` "MUST NOT
    -- exceed 255 ASCII characters in length" — and not an estimate from a
    -- sample. Real values are far below it: Google 21 digits, Entra ID ~43
    -- base64url characters, Okta `00u` + 17, Keycloak a 36-character UUID,
    -- Auth0 a provider-prefixed `google-oauth2|…` in the thirties.
    subject TEXT,

    -- `staff_members.last_totp_step`, renamed and generalised: the RFC 6238
    -- time step of the LAST ACCEPTED code for 'totp', the RFC 4226 counter
    -- for 'hotp', 0 for every other kind. The whole replay guard: a code is
    -- accepted only if its step is strictly greater than this, so the ±1-step
    -- window that makes TOTP usable across a clock skew cannot be used to
    -- present the same six digits twice.
    --
    -- `NOT NULL`, seeded to 0, and the argument is INHERITED VERBATIM rather
    -- than restated from taste. The guard is a compare-and-swap —
    -- `update_many().where_(id).where_(counter().lt(next))` — and in SQL
    -- `NULL < 12345` is NULL, not true, so a nullable column would make the
    -- FIRST code of every subject match zero rows and be refused forever.
    --
    -- BIGINT and not INT: `int4` is one of the six types `map_scalar` does
    -- not map, and `currencies.exponent` had to be widened by migration 0032
    -- for exactly this reason.
    counter BIGINT NOT NULL,

    -- `staff_members.password_change_required`, moved, because it is a
    -- property of the CREDENTIAL — "this material was printed by an operator
    -- and must be replaced by material the person chose" — and not of the
    -- person. Every authenticated route refuses a session whose staff member
    -- holds a password credential with this set.
    must_change BOOLEAN NOT NULL,

    -- When a transient credential stops being presentable. NO WRITER TODAY.
    -- It is a column rather than a future migration because
    -- `credentials_transient_kinds_expire` below makes it load-bearing the
    -- moment a transient kind is implemented: a magic link with no expiry is
    -- a permanent bearer credential sitting in an inbox, and the CHECK
    -- refuses one before any reviewer has to notice.
    expires_at TIMESTAMPTZ,

    -- Written by `vpay_db::credentials`, never by a trigger or a DEFAULT.
    -- For a 'totp' row `created_at` IS the enrolment date, which is why
    -- `staff_members.totp_enrolled_at` has no counterpart here: under the
    -- split, ENROLMENT IS THE EXISTENCE OF THE ROW.
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,

    CONSTRAINT credentials_id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT credentials_staff_member_id_length CHECK (char_length(staff_member_id) BETWEEN 1 AND 64),
    CONSTRAINT credentials_kind_length CHECK (char_length(kind) BETWEEN 1 AND 32),
    CONSTRAINT credentials_issuer_length CHECK (char_length(issuer) BETWEEN 1 AND 512),
    CONSTRAINT credentials_subject_length CHECK (char_length(subject) BETWEEN 1 AND 255),

    -- THE VOCABULARY, CLOSED. `vpay_db::credentials::CredentialKind` mirrors
    -- it exactly; a value spelled here and not there is a row this crate
    -- declines to decode rather than one it guesses about, and one spelled
    -- there and not here is refused at the insert.
    --
    -- EIGHT NAMES, TWO IMPLEMENTATIONS. See the `kind` column's comment: a
    -- reader who finds 'webauthn' here must not conclude vpay supports
    -- WebAuthn.
    CONSTRAINT credentials_kind_is_known CHECK (kind IN (
        'password', 'totp', 'hotp', 'magic_link',
        'email_otp', 'phone_otp', 'webauthn', 'oidc'
    )),

    -- EXACTLY ONE SUBJECT. Single-column today, so `cratestack-migrate` CAN
    -- see this one; written as `num_nonnulls(...)` rather than `IS NOT NULL`
    -- so that adding customers is
    -- `num_nonnulls(staff_member_id, customer_id) = 1` and nothing else about
    -- the column list changes. At that point it becomes multi-column and
    -- therefore invisible to the tooling in both directions, exactly as
    -- `customers.at_least_one_identifier` is.
    CONSTRAINT credentials_has_exactly_one_subject CHECK (
        num_nonnulls(staff_member_id) = 1
    ),

    -- A FEDERATED CREDENTIAL CARRIES AN IDENTITY AND NO SECRET; EVERY OTHER
    -- KIND CARRIES A SECRET AND NO IDENTITY. Multi-column, therefore
    -- invisible to `cratestack migrate baseline` in both directions, so
    -- `postgres_smoke.rs` asserts it directly against a real Postgres.
    --
    -- This is the constraint that keeps `material`'s nullability from being a
    -- hole. Without it, "material is optional" means a `password` row with no
    -- hash — a credential that verifies against nothing — is a legal row.
    CONSTRAINT credentials_federated_carries_identity_and_no_material CHECK (
        CASE WHEN kind = 'oidc'
             THEN material IS NULL AND issuer IS NOT NULL AND subject IS NOT NULL
             ELSE material IS NOT NULL AND issuer IS NULL AND subject IS NULL
        END
    ),

    -- A TRANSIENT CREDENTIAL EXPIRES. Nothing writes these kinds today; the
    -- constraint exists so that the slice which does cannot write one that
    -- never expires.
    CONSTRAINT credentials_transient_kinds_expire CHECK (
        kind NOT IN ('magic_link', 'email_otp', 'phone_otp')
        OR expires_at IS NOT NULL
    ),

    -- `must_change` is a password's flag. On a table with eight kinds a bare
    -- BOOLEAN would be a field whose meaning depends on a column nothing
    -- relates it to; this is what keeps it meaning one thing.
    CONSTRAINT credentials_only_a_password_may_require_change CHECK (
        NOT must_change OR kind = 'password'
    ),

    -- A step is never negative: it is `unix_seconds / 30`, and the seed is 0.
    CONSTRAINT credentials_counter_is_not_negative CHECK (counter >= 0)
);

-- ONE PASSWORD PER SUBJECT, ONE TOTP, ONE HOTP — AND MANY OF EVERYTHING ELSE.
--
-- A PARTIAL unique index, which `cratestack` cannot see at all (undeclared
-- indexes are one of the two drift kinds it structurally cannot close), so
-- NO COMPILE-TIME GATE WILL EVER NOTICE IF THIS IS DROPPED. What proves it is
-- a container-backed test that writes the second row and asserts
-- `PersistenceError::Unique`, and the mutation that makes that test worth
-- having is dropping this index and watching it go red.
--
-- 'webauthn', 'magic_link', 'email_otp' and 'phone_otp' are deliberately
-- absent from the predicate: MANY of each is correct. A person may enrol
-- several security keys and may have several links in flight.
--
-- This also replaces `Staff::enrol_totp`'s `totp_enrolled_at IS NULL` guard
-- with something a future edit cannot drop by accident: a second enrolment is
-- refused by the database rather than by a `WHERE` clause.
CREATE UNIQUE INDEX credentials_one_singleton_kind_per_staff_member
    ON credentials (staff_member_id, kind)
    WHERE kind IN ('password', 'totp', 'hotp');

-- AT MOST ONE LINK PER ISSUER PER SUBJECT. Account linking is many-to-one and
-- mixed — one staff member may hold a Google link, an Entra link and several
-- WebAuthn keys at once — but two rows for one person at one issuer is a
-- duplicate, not a second account.
CREATE UNIQUE INDEX credentials_one_federated_link_per_issuer
    ON credentials (staff_member_id, issuer)
    WHERE kind = 'oidc';

-- (ISSUER, SUBJECT) IS GLOBALLY UNIQUE, and this is the federated equivalent
-- of two people sharing a password hash: whoever signs in as (iss, sub) gets
-- whichever row the query returns first.
--
-- GLOBAL — not scoped to a subject and not scoped to a merchant. An IdP
-- identity denotes ONE HUMAN. Scoping this per merchant would make exactly
-- that bug legal as long as the two rows belonged to different tenants, which
-- is the cross-tenant account-takeover shape ADR-0018 spends a whole document
-- keeping out.
CREATE UNIQUE INDEX credentials_federated_identity_key
    ON credentials (issuer, subject)
    WHERE kind = 'oidc';

-- "What credentials does this person hold?" — the login path's read, and what
-- the FK's own cascade asks on every staff delete. An unindexed FK turns a
-- `DELETE FROM staff_members` into a sequential scan of this table.
CREATE INDEX credentials_staff_member_idx ON credentials (staff_member_id);

COMMENT ON TABLE credentials IS
    'How a subject proved who they are (ADR-0019). One row per credential, generic over kind now and over subject later: staff_member_id is a nullable FK with a CHECK that exactly one subject column is set. Eight kinds are declared; exactly two -- password and totp -- are implemented. Stores opaque material and interprets none of it.';
COMMENT ON COLUMN credentials.material IS
    'The opaque secret: an argon2id PHC string for password, the AES-256-GCM sealed RFC 6238 secret for totp. NULL for oidc, because a federated identity is not a secret -- (iss, sub) is public and the verification material is the issuer''s JWKS. The pepper and the TOTP key are NOT in this column: they are deployment secrets, and losing either invalidates every row of the corresponding kind.';
COMMENT ON COLUMN credentials.counter IS
    'The RFC 6238 time step of the last ACCEPTED code (totp) or the RFC 4226 counter (hotp). A code is accepted only if strictly greater, which is what stops the +/-1-step skew window being used to replay the same six digits. NOT NULL seeded to 0 because NULL < step is NULL in SQL and a nullable column would refuse every subject''s first code forever.';
COMMENT ON COLUMN credentials.subject IS
    'The IdP''s sub, bounded at OpenID Connect Core 1.0 section 2''s own ceiling of 255 ASCII characters. Matched on; an email claim never is, because an email is mutable, may be unverified, and is reassignable inside a tenant.';

-- THE BACKFILL. Every live row's material moves, and it moves unchanged.

-- One 'password' credential per staff member. Every row has a password hash
-- — `staff_members.password_hash` was NOT NULL — so this is one row per
-- member with no WHERE.
--
-- THE ID IS DERIVED, NOT RANDOM. `md5` is core Postgres and needs no
-- extension, and its hex alphabet `0123456789abcdef` is a SUBSET of the
-- base32 alphabet `vpay_core::ids` mints from
-- (`0123456789abcdefghjkmnpqrstvwxyz`), so 24 hex characters are 24 legal id
-- characters and `ids::is_well_formed(CREDENTIAL_PREFIX, ...)` accepts a
-- backfilled row exactly as it accepts a minted one. `md5` is doing id
-- derivation here and NO SECURITY WORK WHATSOEVER; a random id per row would
-- make this migration non-deterministic for no gain, since a credential id is
-- not a secret and is never presented by a caller.
INSERT INTO credentials (
    id, staff_member_id, kind, material, issuer, subject,
    counter, must_change, expires_at, created_at, updated_at
)
SELECT
    'cred_' || substr(md5(s.id || ':password'), 1, 24),
    s.id,
    'password',
    s.password_hash,
    NULL,
    NULL,
    0,
    s.password_change_required,
    NULL,
    s.created_at,
    s.updated_at
FROM staff_members s;

-- One 'totp' credential per ENROLLED staff member. `totp_secret IS NOT NULL`
-- is exactly "enrolled" — `staff_members_totp_is_paired` made that an
-- invariant — so `totp_enrolled_at` is NOT NULL on every row this selects and
-- can be the credential's `created_at` without a COALESCE that would hide a
-- broken pair.
--
-- `counter` takes `last_totp_step` UNCHANGED. Seeding it to 0 instead would
-- re-admit every code the ±1-step window still covers, which is precisely the
-- replay this column exists to refuse — a migration that quietly reopened a
-- 90-second replay window on every enrolled account.
INSERT INTO credentials (
    id, staff_member_id, kind, material, issuer, subject,
    counter, must_change, expires_at, created_at, updated_at
)
SELECT
    'cred_' || substr(md5(s.id || ':totp'), 1, 24),
    s.id,
    'totp',
    s.totp_secret,
    NULL,
    NULL,
    s.last_totp_step,
    FALSE,
    NULL,
    s.totp_enrolled_at,
    s.updated_at
FROM staff_members s
WHERE s.totp_secret IS NOT NULL;

-- THE DROP. Five columns, and the two CHECKs that only spoke about them go
-- with them: `staff_members_totp_is_paired` (both its columns are leaving)
-- and `staff_members_totp_step_is_not_negative` (its one column is), which
-- Postgres drops automatically with the columns rather than needing a
-- separate statement.
--
-- `staff_members` keeps `last_sign_in_at`: it is when this PERSON last
-- completed a full sign-in, across every credential they hold and across the
-- two factors one sign-in presents. ADR-0019 decision 6.
ALTER TABLE staff_members
    DROP COLUMN password_hash,
    DROP COLUMN password_change_required,
    DROP COLUMN totp_secret,
    DROP COLUMN totp_enrolled_at,
    DROP COLUMN last_totp_step;

COMMENT ON TABLE staff_members IS
    'A human who may sign in to /dash/v1 (ADR-0017). Created only by `vpay-server staff add`; no HTTP endpoint creates one. Belongs to exactly one merchant. Credentials are NOT here: they are rows in `credentials` (ADR-0019), one per credential, generic over kind.';
