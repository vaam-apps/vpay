-- customers: the address object -- formal AND GPS (issue #67) -- and the
-- anonymisation columns
-- that make "delete this customer" a complete erasure of the payer
-- (issues #68 and #96 item 2).
--
-- WHY THESE TWO ARE ONE MIGRATION
--
-- They are the same promise from two ends. #67 adds eight more columns of
-- another person's personal data — six formal components and the two-column
-- coordinate that is the other half of what an address means here (the
-- maintainer, 2026-09-11) — to the one table in vpay whose entire content is
-- personal data; #68 is the erasure that has to cover them. A payer's
-- coordinates are the most sensitive field on the object: a name is how
-- somebody is addressed and a point is where they sleep. A
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
    -- THE OTHER HALF OF AN ADDRESS IN THIS MARKET (the maintainer, 2026-09-11)
    --
    -- "address in our system means both formal as well as GPS". Formal
    -- addressing is unreliable across the markets vpay serves -- a street
    -- with no sign, a quarter with no postcode, a building known by the shop
    -- on its corner -- and a coordinate is how a place is actually found. The
    -- address is therefore ONE object with two halves, and these two columns
    -- are the second half rather than a separate `location` concept: they are
    -- replaced whole with the six above, cleared with them, and erased with
    -- them.
    --
    -- INTEGER MICRODEGREES, NEVER FLOATING POINT, and the unit is in the
    -- column name. Two measurements in this repository decide that, neither
    -- of them an aesthetic preference:
    --
    --   * CrateStack's `Value::from_plain_json` routes every JSON number
    --     through `Number::as_i64()` and demotes anything else to `f64`
    --     (docs/reference/vpay-db.md). A decimal degree stored as a JSON
    --     number is a value this stack cannot carry without rounding it.
    --   * the money layer's own precedent is integer minor units with the
    --     scale named (docs/flows/money.md, and float arithmetic is denied
    --     workspace-wide by ADR-0007). A coordinate is the same kind of
    --     quantity: an exact count of a fixed unit, not a measurement to be
    --     re-rounded by every layer it passes through.
    --
    -- A microdegree is 1e-6 degrees, about 0.11 m of latitude at the equator
    -- -- two orders of magnitude finer than consumer GPS, so the unit costs
    -- no precision anybody can observe. BIGINT rather than INTEGER because
    -- the longitude range is +/-180,000,000 and INTEGER's ceiling is
    -- 2,147,483,647: it would fit, and the column beside it would be the one
    -- that had to explain why the two halves of one pair are different types.
    ADD COLUMN address_latitude_microdeg BIGINT,
    ADD COLUMN address_longitude_microdeg BIGINT,
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
-- So the database states it: a row with `anonymized_at` set holds NOTHING of
-- the payer's in ANY identifier column, with no exceptions. All eleven are
-- written unconditionally — including the components the payer never had —
-- because "which fields did this payer fill in?" is itself information about
-- them, and a scheme where a NULL stays NULL leaks the shape of the record it
-- claims to have erased.
--
-- WHAT "NOTHING" IS PER COLUMN TYPE, SINCE 2026-09-11
--
-- The nine text columns carry the marker, which is a value and not an
-- absence: it says "there was a payer here and vpay erased them", which is
-- the fact an operator reading the row needs and is why the erasure writes
-- all nine rather than NULLing them.
--
-- The two coordinate columns CANNOT carry it — they are BIGINT, and there is
-- no integer that is not a possible place — so the erasure sets them NULL and
-- this CHECK requires NULL of them. That is a real asymmetry and it is stated
-- here rather than smoothed over: the marker's job on the text columns is to
-- distinguish an erased payer from a payer who never gave a name, and on the
-- coordinates that distinction is carried by `anonymized_at` itself, which is
-- non-NULL exactly when this branch of the CHECK applies. Nothing is lost;
-- what would have been lost is the only thing worth keeping about a
-- coordinate, which is the coordinate.
--
-- The literal is spelled here and in `vpay_db::customers::REDACTED`, and the
-- two must agree: `an_anonymised_customer_carries_the_marker_in_every_
-- identifier_column` in postgres_smoke.rs is what proves they do, by writing
-- the Rust constant into a real row.
--
-- WHY `IS NOT DISTINCT FROM` AND NOT `=`
--
-- (This paragraph spells the marker WITHOUT quotes, as the rest of this
-- file's prose does, because `the_redaction_marker_is_the_one_the_migration_
-- enforces` counts the quoted form and expects exactly nine — one per TEXT
-- identifier column of the CHECK below. The count stayed at nine when the two
-- coordinate columns joined that CHECK on 2026-09-11, and that is the point of
-- it rather than a coincidence: they are required to be NULL and name no
-- literal, so a coordinate written with the marker instead would be a
-- 22P02 on every erasure. A tenth quoted marker in a comment fails it.)
--
-- A CHECK is violated only when its expression evaluates to FALSE; NULL
-- passes. Comparing a NULL name to the marker with `=` yields NULL, so the
-- `=` spelling of this constraint accepted exactly the row it is written to
-- refuse: `anonymized_at` set, `address_line2` (or `name`, or any of the
-- nine text columns) left NULL. That is not a theoretical hole. It is the FIRST state a
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
        -- The coordinates, NULL and not the marker — see "WHAT `NOTHING` IS
        -- PER COLUMN TYPE" above. Spelled `IS NOT DISTINCT FROM NULL` rather
        -- than `IS NULL`, which it is exactly equivalent to, so that all
        -- eleven conjuncts read as one rule stated eleven times: a reader
        -- checking that this CHECK covers every identifier column should be
        -- able to run their eye down a single column of operators, and a
        -- twelfth column added in a different spelling is what that uniformity
        -- makes visible.
        AND address_latitude_microdeg IS NOT DISTINCT FROM NULL
        AND address_longitude_microdeg IS NOT DISTINCT FROM NULL
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
--
-- `address_coordinates_are_both_or_neither`, below, is widened by the same
-- one disjunct and for the same reason -- it is the third member of this
-- family rather than a new idea -- and its own comment says what that buys
-- beyond consistency.
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

-- THE COORDINATES: A RANGE PER AXIS, AND THE PAIR RULE.
--
-- The bounds are the definition of the units rather than a product limit:
-- latitude runs -90..=90 degrees and longitude -180..=180, and in microdegrees
-- that is -90,000,000..=90,000,000 and -180,000,000..=180,000,000. A value
-- outside them is not a place. Two separate CHECKs and not one over both,
-- because the two axes have DIFFERENT bounds and a single constraint would
-- report "the coordinates are wrong" for a longitude of 200 degrees when the
-- fact is which axis and which bound -- the same argument the six
-- `address_*_length` CHECKs above are separate for.
--
-- The API refuses both first, with a `400` naming `address` (`vpay_api::v1::
-- customers`), so these are the backstop a second writer -- a backfill, a
-- repair script, a psql session -- reaches without passing the API at all.
-- Each is single-column, so each costs one `[safe] CHECK ... is not declared`
-- drift line, exactly as the six length bounds do; `model Customer` declares
-- the matching `@range` without `@db_enforce` for the reason every validator
-- in that file carries, and `inputs.rs::validate_impl_tokens` still builds
-- `input.validate()` from it.
ALTER TABLE customers ADD CONSTRAINT address_latitude_microdeg_range CHECK (
    address_latitude_microdeg IS NULL
    OR address_latitude_microdeg BETWEEN -90000000 AND 90000000
);
ALTER TABLE customers ADD CONSTRAINT address_longitude_microdeg_range CHECK (
    address_longitude_microdeg IS NULL
    OR address_longitude_microdeg BETWEEN -180000000 AND 180000000
);

-- BOTH OR NEITHER. Half a coordinate names no place at all: a latitude alone
-- is a line right round the planet, and storing one is strictly worse than
-- storing nothing, because a reader has no way to tell a half-written point
-- from a point at longitude zero once a later writer "completes" it. The pair
-- is the value; neither column is one on its own.
--
-- `(a IS NULL) = (b IS NULL)` rather than a disjunction of two `AND`s,
-- because both sides of that `=` are booleans that are never NULL -- which is
-- `anonymized_customers_carry_the_marker`'s own lesson from the other
-- direction: a CHECK whose expression can be NULL is a CHECK that passes.
--
-- MULTI-COLUMN, so it is invisible to `cratestack migrate baseline` in both
-- directions and costs no drift line; the proof is
-- `a_half_written_coordinate_is_refused_by_the_database` against a real
-- Postgres.
--
-- Widened by `anonymized_at IS NOT NULL`, exactly as the two shape CHECKs
-- above are and for the same reason rather than a new one. An anonymised row
-- holds NULL in both columns, so the pair rule is satisfied anyway and the
-- disjunct buys no behaviour on its own; what it buys is that
-- `anonymized_customers_carry_the_marker` is the ONLY constraint that can
-- fire on a row claiming to be erased. That is what lets the marker CHECK be
-- tested one coordinate at a time -- hold latitude back and Postgres names
-- the marker, not this -- and a constraint a test cannot attribute is a
-- constraint that stops being checked the first time somebody widens another.
ALTER TABLE customers ADD CONSTRAINT address_coordinates_are_both_or_neither CHECK (
    anonymized_at IS NOT NULL
    OR (address_latitude_microdeg IS NULL) = (address_longitude_microdeg IS NULL)
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
COMMENT ON COLUMN customers.address_latitude_microdeg IS
    'Latitude in MICRODEGREES -- millionths of a degree, so 4.061 deg N is 4061000 (the maintainer, 2026-09-11: "address in our system means both formal as well as GPS"). An integer and never a float: CrateStack''s Value::from_plain_json demotes any JSON number outside i64 to f64, and vpay''s own precedent for a fixed-scale quantity is the money layer''s integer minor units. About 0.11 m of resolution, far finer than consumer GPS. NULL, or a value in -90000000..=90000000, and never without address_longitude_microdeg beside it.';
COMMENT ON COLUMN customers.address_longitude_microdeg IS
    'Longitude in MICRODEGREES; see address_latitude_microdeg for the unit and why it is an integer. NULL, or a value in -180000000..=180000000, and never without a latitude beside it -- address_coordinates_are_both_or_neither, because half a coordinate names no place.';
COMMENT ON CONSTRAINT address_coordinates_are_both_or_neither ON customers IS
    'A customer has both coordinates or neither: a latitude alone is a line right round the planet, and a half-written point is worse than none because a later writer completing it produces a plausible wrong place. Widened by anonymized_at IS NOT NULL exactly as the two shape CHECKs are, so that anonymized_customers_carry_the_marker is the only constraint that can fire on a row claiming to be erased. Multi-column, therefore invisible to cratestack migrate baseline in both directions.';
COMMENT ON COLUMN customers.anonymized_at IS
    'When DELETE /v1/customers/{id} or sweep_idle_customers erased this payer''s identifiers, or NULL for a live customer. NOT a soft-delete flag: by the time this is non-NULL every identifier column on the row is the literal [redacted], which anonymized_customers_carry_the_marker enforces. A customer with no payment history is still hard-deleted and never reaches this state; one an intent, session or invoice references cannot be deleted (NO ACTION) and is anonymised instead, so the payment record survives with no payer on it.';
COMMENT ON CONSTRAINT anonymized_customers_carry_the_marker ON customers IS
    'A row that says the payer was erased holds no identifier of theirs: the nine TEXT identifier columns are the literal [redacted] and none of them is NULL, and the two coordinate columns are NULL -- a BIGINT cannot carry the marker, and there is no integer that is not a possible place, so for those two the erasure is the absence. Written unconditionally, including components the payer never filled in, because which fields a record had is itself information about the person. Spelled with IS NOT DISTINCT FROM rather than =, because a CHECK passes when its expression is NULL and the = spelling admitted every partial erasure that left a column NULL. Multi-column, therefore invisible to cratestack migrate baseline in both directions; postgres_smoke.rs asserts it directly, against the Rust constant vpay_db::customers::REDACTED, holding back each text column first as a value and then as a NULL, and each coordinate column as a value in range.';
COMMENT ON TABLE customers IS
    'A merchant-owned record of a payer they expect to see again (S4a). Personal data. DELETE /v1/customers/{id} and the twelve-month sweep_idle_customers job hard-delete a customer with no payment history and ANONYMISE one that has any (0041): every text identifier column becomes [redacted] and the two coordinate columns become NULL, anonymized_at is stamped, metadata (the merchant''s own data) stays, and the NO ACTION foreign keys keep the payment record intact with no payer on it. Never soft-deleted — an anonymised row is not the record of the person. No cross-merchant identity: nothing here is unique across merchant_id, deliberately.';
