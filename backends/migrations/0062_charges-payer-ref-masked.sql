-- charges.payer_ref_masked: backfilled, and from now on a function of
-- charges.payer_ref (issue #79, decision 4).
--
-- The column has existed since 0004 and nothing ever wrote it: the confirm
-- path stored NULL, so the staff dashboard's payment detail rendered `—` for
-- every payer. The decision is that it holds the payer's NORMALISED MSISDN
-- masked to its first three and last two digits, written by the confirm path
-- in the same INSERT as `payer_ref` (`vpay_api::v1::payment_intents`'
-- `open_charge`, through `vpay_core::privacy::payer_msisdn_mask`), and
-- backfilled here for the rows that already exist.
--
-- THE SHAPE: `+` + the first three digits + four bullets (U+2022) + the last
-- two digits, e.g. `+237••••67` for `237671234567`. Four bullets however many
-- digits they hide, so a mask's length is not an oracle for the number's.
--
-- WHICH ROWS GET ONE: exactly those whose `payer_ref` is a normalised number —
-- 8 to 15 ASCII digits, no `+`, no separator — the form `payer_fields` gives
-- every push rail. Every other row stays NULL, and that is the point:
--
--   * a redirect rail (Orange Money) has `payer_ref IS NULL`: vpay never learnt
--     the number, so there is nothing to mask;
--   * a row erasure reached has `payer_ref = '[redacted]'` and
--     `payer_ref_masked` already NULL (`vpay_db::customers::erase_in_tx`,
--     ADR-0027). A mask derived from the marker would be a rendering of a
--     value erasure removed, and one recovered from anywhere else would be
--     un-erasing it. It stays NULL;
--   * a row written before `payer_fields` normalised the number (a verbatim
--     `+237 6…` the merchant sent) is NOT the normalised MSISDN, and a mask of
--     it would be a mask of something else. Normalising in SQL here would be a
--     second normaliser with nothing holding it to the first. It stays NULL,
--     and the dashboard renders `—` for it, which is true.
--
-- FIRST, a value already in the column that is not the derived mask is
-- cleared. No shipped code wrote this column before 0062, but a database can
-- hold anything a hand-run statement or a fixture put there, and one such row
-- would make the ADD CONSTRAINT below fail — and with it this migration, and
-- every boot that applies it. Clearing rather than keeping is the safe
-- direction: the value is either replaced by the derived mask on the next
-- statement (a normalised `payer_ref`) or left NULL (anything else). The
-- predicate is the CHECK's own condition, negated.
UPDATE charges
   SET payer_ref_masked = NULL
 WHERE payer_ref_masked IS NOT NULL
   AND (payer_ref IS NULL
        OR payer_ref !~ '^[0-9]{8,15}$'
        OR payer_ref_masked IS DISTINCT FROM
           '+' || left(payer_ref, 3) || '••••' || right(payer_ref, 2));

-- `WHERE payer_ref_masked IS NULL` leaves alone a row whose value already is
-- the derived mask.
UPDATE charges
   SET payer_ref_masked = '+' || left(payer_ref, 3) || '••••' || right(payer_ref, 2)
 WHERE payer_ref_masked IS NULL
   AND payer_ref ~ '^[0-9]{8,15}$';

-- And the rule the backfill applied, held for every later write.
--
-- A mask is either absent or EXACTLY the mask of this row's `payer_ref`. That
-- one CHECK refuses three different mistakes:
--
--   * an unmasked number written into the masked column — the failure mode
--     `docs/reference/vpay-api/dashboard-surface.md` names for this field;
--   * a mask left behind by an erasure that redacted `payer_ref` and forgot
--     `payer_ref_masked` (the marker does not match the digit pattern, so any
--     non-NULL mask beside it is refused);
--   * a Rust mask that drifted from this SQL one — the confirm path's INSERT
--     fails rather than storing a second shape.
--
-- Two columns, so `cratestack migrate baseline` cannot see it in either
-- direction (its introspection keeps single-column CHECKs only);
-- `postgres_smoke.rs` lists it with the other multi-column CHECKs.
--
-- COST: `ADD CONSTRAINT ... CHECK` without `NOT VALID` scans every row of
-- `charges` to validate it, holding an ACCESS EXCLUSIVE lock on the table for
-- the whole scan — no confirm, settlement or dashboard read of `charges`
-- proceeds meanwhile. The two UPDATEs above also rewrite the rows they touch.
-- On a large table, schedule this migration for a quiet window.
--
-- `payer_ref IS NOT NULL` is spelled out although the pattern match implies
-- it: a CHECK passes when its expression is NULL, and `NULL ~ '...'` is NULL,
-- so without it a mask beside no number at all would be admitted.
ALTER TABLE charges
    ADD CONSTRAINT payer_ref_masked_derives_from_payer_ref CHECK (
        payer_ref_masked IS NULL
        OR (
            payer_ref IS NOT NULL
            AND payer_ref ~ '^[0-9]{8,15}$'
            AND payer_ref_masked = '+' || left(payer_ref, 3) || '••••' || right(payer_ref, 2)
        )
    );
