-- customers: the address object (issue #67) and the anonymisation columns
-- that make "delete this customer" a complete erasure of the payer
-- (issues #68 and #96 item 2).
--
-- WHY THESE TWO ARE ONE MIGRATION
--
-- They are the same promise from two ends. #67 adds six more columns of
-- another person's personal data to the one table in vpay whose entire
-- content is personal data; #68 is the erasure that has to cover them. A
-- migration that added the address and left the erasure for later would
-- widen the retention gap on the way to closing it, and the redaction
-- statement in `vpay_db::customers` names the address columns, so the two
-- cannot ship apart without one of them being wrong for a while.
--
-- WHAT #68 ASKED FOR, AND WHY IT IS NOT WHAT THIS MIGRATION DOES
--
-- Issue #68 says "deletion redacts identifiers on retained intents and
-- sessions". Read against the live schema that premise is false, and the
-- reading is what changed the design: `payment_intents.customer_id` and
-- `checkout_sessions.customer_id` (0034) and `invoices.customer_id` (0036)
-- are `NO ACTION` foreign keys, so a customer any of the three references
-- CANNOT be deleted at all — the API answered `409` and the retention sweep
-- skipped it. There was never a retained intent carrying a payer identifier
-- to redact: the intent carries an amount, a status and a `cus_…`, and the
-- payer's name, email and phone lived on the customer row that could not go.
--
-- So the payer identifiers that actually survived a "deletion" were, in
-- full:
--
--   * `customers.{name,email,phone}` — because the row could not be deleted;
--   * `charges.payer_ref` / `charges.payer_ref_masked` (0004) — the payer's
--     MSISDN as the rail was given it, reachable only through an intent;
--   * `events.data` — every `customer.created` / `customer.updated` /
--     `customer.deleted` body stores the WHOLE rendered object, and nothing
--     prunes `events`. This was the real leak and no code named it.
--   * `idempotency_keys.response_body` (0015) — the exact JSON a
--     `POST /v1/customers` answered, kept for 24 hours to replay.
--
-- And the `409`'s own advice — "clear `name`, `email` and `phone` instead" —
-- is refused by `at_least_one_identifier`. It could not be followed.
--
-- THE DECISION (maintainer's delegate, 2026-09-10)
--
-- A customer with payment history is never row-deleted. `DELETE
-- /v1/customers/{id}` ANONYMISES it instead: every identifier column becomes
-- the marker `[redacted]`, `anonymized_at` is stamped, `metadata` stays (it
-- is the merchant's own data, not the payer's), the foreign-key rows are
-- untouched so the payment record and the ledger survive intact, and the
-- object renders `deleted: true` Stripe-style so a merchant's stored `cus_…`
-- keeps resolving instead of turning into a `404`. A customer with NO
-- history is hard-deleted exactly as before, and its `GET` is a `404`
-- afterwards. That asymmetry is Stripe's too.
--
-- `DELETE` therefore always succeeds or answers the uniform `404`. The `409`
-- is gone.
ALTER TABLE customers
    -- Stripe's address, six nullable text columns and no defaults. Prefixed
    -- rather than a JSONB `address` column for the reason `metadata`'s GAP
    -- note gives from the other side: a JSONB column is invisible to
    -- `cratestack migrate baseline` in both directions and cannot be
    -- declared on `model Customer` without `Value::from_plain_json`'s number
    -- demotion, so six TEXT columns are the shape that can be compared,
    -- CHECKed per component, and redacted by name.
    ADD COLUMN address_line1 TEXT,
    ADD COLUMN address_line2 TEXT,
    ADD COLUMN address_city TEXT,
    ADD COLUMN address_state TEXT,
    ADD COLUMN address_postal_code TEXT,
    -- ISO 3166-1 alpha-2, upper case, validated on the way in by
    -- `vpay_api::v1::customers` and by the CHECK below. Two letters and not a
    -- name, because it is the value a merchant's own address book and every
    -- shipping API already speak.
    ADD COLUMN address_country TEXT,
    -- When this customer was anonymised, or NULL for a live one.
    --
    -- NOT a soft-delete flag, and the difference is the whole reason the
    -- column is allowed to exist at all next to
    -- `a_customer_delete_is_a_delete_and_not_a_soft_delete`. A soft delete
    -- keeps the record of the person and hides it behind a predicate; this
    -- records that the record of the person is GONE — every identifier
    -- column on the row is the literal `[redacted]` by the time this is
    -- non-NULL, which `anonymized_customers_carry_the_marker` below makes a
    -- database invariant rather than a promise two call sites remember.
    ADD COLUMN anonymized_at TIMESTAMPTZ;

-- THE MARKER IS THE INVARIANT, AND IT IS CHECKED RATHER THAN TRUSTED
--
-- Anonymisation is one `UPDATE` in `vpay_db::customers`, and the failure mode
-- that matters is not that it fails — it is that it misses a column. A
-- future column added to the identifier set, or a hand-edited `SET` list, or
-- a merge that drops one assignment, all leave a row that says "this payer
-- was erased" while still holding one of their identifiers, and nothing in
-- Rust would object.
--
-- So the database states it: a row with `anonymized_at` set carries the
-- marker in EVERY identifier column, with no exceptions and no NULLs. All
-- nine are written unconditionally — including the components the payer
-- never had — because "which fields did this payer fill in?" is itself
-- information about them, and a scheme where a NULL stays NULL leaks the
-- shape of the record it claims to have erased.
--
-- The literal is spelled here and in `vpay_db::customers::REDACTED`, and the
-- two must agree: `an_anonymised_customer_carries_the_marker_in_every_
-- identifier_column` in postgres_smoke.rs is what proves they do, by writing
-- the Rust constant into a real row.
--
-- WHY `IS NOT DISTINCT FROM` AND NOT `=`
--
-- A CHECK is violated only when its expression evaluates to FALSE; NULL
-- passes. `name = '[redacted]'` with a NULL `name` is NULL, so the `=`
-- spelling of this constraint accepted exactly the row it is written to
-- refuse: `anonymized_at` set, `address_line2` (or `name`, or any of the
-- nine) left NULL. That is not a theoretical hole. It is the FIRST state a
-- missed assignment produces, because the columns an erasure most easily
-- misses are the ones a payer never filled in, and the whole argument for
-- writing all nine unconditionally is that "which fields did this payer
-- fill in?" is itself information about them. `IS NOT DISTINCT FROM` is
-- FALSE rather than NULL when one side is NULL, so the constraint now says
-- what its comment always claimed.
--
-- Multi-column, therefore invisible to `cratestack migrate baseline` in both
-- directions (`introspect/postgres/constraints.rs` filters
-- `array_length(c.conkey, 1) = 1`), exactly as `at_least_one_identifier` is.
-- That is why the proof is a test against a real Postgres and not a drift
-- line.
ALTER TABLE customers ADD CONSTRAINT anonymized_customers_carry_the_marker CHECK (
    anonymized_at IS NULL OR (
        name IS NOT DISTINCT FROM '[redacted]'
        AND email IS NOT DISTINCT FROM '[redacted]'
        AND phone IS NOT DISTINCT FROM '[redacted]'
        AND address_line1 IS NOT DISTINCT FROM '[redacted]'
        AND address_line2 IS NOT DISTINCT FROM '[redacted]'
        AND address_city IS NOT DISTINCT FROM '[redacted]'
        AND address_state IS NOT DISTINCT FROM '[redacted]'
        AND address_postal_code IS NOT DISTINCT FROM '[redacted]'
        AND address_country IS NOT DISTINCT FROM '[redacted]'
    )
);

-- `at_least_one_identifier` IS DELIBERATELY NOT RELAXED.
--
-- The design note this migration was written from proposed relaxing it to
-- `anonymized_at IS NOT NULL OR (...)`, on the assumption that anonymisation
-- NULLs the identifiers. It does not — it writes the marker, which is not
-- NULL — so the constraint already passes for an anonymised row and the
-- relaxation would have weakened a live constraint for no behaviour at all.
-- It stays exactly as 0034 wrote it, and it now also backstops the
-- anonymisation in the one direction that matters: a future erasure that
-- NULLed the three identifiers instead of marking them would be refused
-- outright rather than quietly producing a `cus_…` that names nobody.

-- The two SHAPE CHECKs the marker is not shaped like.
--
-- `[redacted]` is neither a canonical MSISDN nor an ISO 3166-1 alpha-2 code,
-- so both have to admit an anonymised row. Each is widened by exactly one
-- disjunct — `anonymized_at IS NOT NULL` — and NOT by loosening the pattern,
-- because `anonymized_customers_carry_the_marker` above already pins the
-- value those rows may hold. The pair says the whole rule: a live customer's
-- phone is a phone number, and an anonymised one's is the marker. Neither
-- can hold anything else.
--
-- Both become multi-column and therefore leave the drift report; the
-- postgres_smoke cases that assert them fire against a real Postgres, which
-- is where a cross-column CHECK has always had to be proved here.
ALTER TABLE customers DROP CONSTRAINT phone_is_a_canonical_msisdn;
ALTER TABLE customers ADD CONSTRAINT phone_is_a_canonical_msisdn CHECK (
    phone IS NULL OR anonymized_at IS NOT NULL OR phone ~ '^[0-9]{8,15}$'
);

-- The six component bounds, at the same place `name_length` and
-- `email_length` are and for 0034's reason: the API refuses an over-long
-- component with a `400` naming the parameter, and this is the backstop that
-- stops a writer which does not. The floors are 1 — `address[city]=` is what
-- a client templating an absent field emits, and the API turns blank into
-- absent before it gets here.
--
-- 256 for the free-text five, matching `name_length` rather than inventing a
-- second number: a street line and a city are the same order of thing as a
-- person's name, and a merchant hitting one bound and not the other for no
-- reason they can see is a worse API than one number.
ALTER TABLE customers ADD CONSTRAINT address_line1_length CHECK (
    address_line1 IS NULL OR char_length(address_line1) BETWEEN 1 AND 256
);
ALTER TABLE customers ADD CONSTRAINT address_line2_length CHECK (
    address_line2 IS NULL OR char_length(address_line2) BETWEEN 1 AND 256
);
ALTER TABLE customers ADD CONSTRAINT address_city_length CHECK (
    address_city IS NULL OR char_length(address_city) BETWEEN 1 AND 256
);
ALTER TABLE customers ADD CONSTRAINT address_state_length CHECK (
    address_state IS NULL OR char_length(address_state) BETWEEN 1 AND 256
);
ALTER TABLE customers ADD CONSTRAINT address_postal_code_length CHECK (
    address_postal_code IS NULL OR char_length(address_postal_code) BETWEEN 1 AND 64
);
-- Two upper-case letters, which is the whole of ISO 3166-1 alpha-2's shape.
-- NOT a list of the 249 assigned codes: the list changes (South Sudan in
-- 2011, the Netherlands Antilles out in 2010), and a CHECK that had to be
-- migrated whenever the world did would refuse a merchant's perfectly real
-- address until somebody shipped a migration. The API validates the same
-- shape and names the parameter; nothing in vpay resolves a country code to
-- anything, so a syntactically valid unassigned code costs nothing.
ALTER TABLE customers ADD CONSTRAINT address_country_is_iso_3166_1_alpha_2 CHECK (
    address_country IS NULL OR anonymized_at IS NOT NULL OR address_country ~ '^[A-Z]{2}$'
);

-- The sweep's backlog query changed shape, and this index is why it is
-- mentioned here at all: `customers_idle_idx ON (last_used_at)` (0034) still
-- serves it. What changed is the WHERE — `idle_since` no longer carries the
-- `NOT EXISTS` triple, because a customer WITH payment history is now
-- eligible (it is anonymised rather than deleted), and it now excludes rows
-- that are already anonymised so the sweep cannot offer one twice. A partial
-- index on `anonymized_at IS NULL` would serve that better the day most rows
-- are anonymised, and is deliberately not created today: no deployment has
-- ever run this sweep (docs/flows/customers.md), so its selectivity is a
-- guess, and an index chosen from a guess is one nobody can justify dropping.

COMMENT ON COLUMN customers.address_line1 IS
    'Street address, line 1 (issue #67). One of six nullable components spelling Stripe''s address object; vpay stores them as six TEXT columns rather than one JSONB so each can be compared by cratestack migrate baseline, CHECKed on its own, and redacted by name when the customer is anonymised.';
COMMENT ON COLUMN customers.address_country IS
    'ISO 3166-1 alpha-2, upper case (issue #67). The shape is checked, the code is not resolved against any list: the list changes and a CHECK that had to be migrated whenever the world did would refuse a real address. NULL, two upper-case letters, or the redaction marker on an anonymised row.';
COMMENT ON COLUMN customers.anonymized_at IS
    'When DELETE /v1/customers/{id} or sweep_idle_customers erased this payer''s identifiers, or NULL for a live customer. NOT a soft-delete flag: by the time this is non-NULL every identifier column on the row is the literal [redacted], which anonymized_customers_carry_the_marker enforces. A customer with no payment history is still hard-deleted and never reaches this state; one an intent, session or invoice references cannot be deleted (NO ACTION) and is anonymised instead, so the payment record survives with no payer on it.';
COMMENT ON CONSTRAINT anonymized_customers_carry_the_marker ON customers IS
    'A row that says the payer was erased holds no identifier of theirs: all nine identifier columns are the literal [redacted], and none of them is NULL. Written unconditionally, including components the payer never filled in, because which fields a record had is itself information about the person. Spelled with IS NOT DISTINCT FROM rather than =, because a CHECK passes when its expression is NULL and the = spelling admitted every partial erasure that left a column NULL. Multi-column, therefore invisible to cratestack migrate baseline in both directions; postgres_smoke.rs asserts it directly, against the Rust constant vpay_db::customers::REDACTED, holding back each column first as a value and then as a NULL.';
COMMENT ON TABLE customers IS
    'A merchant-owned record of a payer they expect to see again (S4a). Personal data. DELETE /v1/customers/{id} and the twelve-month sweep_idle_customers job hard-delete a customer with no payment history and ANONYMISE one that has any (0041): every identifier column becomes [redacted], anonymized_at is stamped, metadata (the merchant''s own data) stays, and the NO ACTION foreign keys keep the payment record intact with no payer on it. Never soft-deleted — an anonymised row is not the record of the person. No cross-merchant identity: nothing here is unique across merchant_id, deliberately.';
