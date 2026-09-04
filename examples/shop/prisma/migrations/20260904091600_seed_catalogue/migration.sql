-- The catalogue, as a data migration.
--
-- D13 makes products seed data with no admin surface, so there is exactly one
-- place they can come from and this is it: `prisma migrate deploy` in the
-- container's entrypoint applies this the first time the database is empty,
-- and never again. Deliberately NOT a `prisma db seed` script — that would be
-- a second thing to run, and a shop whose catalogue depends on whether
-- somebody remembered to run it is a shop that shows an empty page in the
-- demo.
--
-- Prices are INTEGER MINOR UNITS in XAF, which is zero-decimal
-- (docs/flows/money.md): 12000 is 12,000 FCFA, not 120.00.
--
-- `ON CONFLICT DO NOTHING` so that re-baselining a database that already
-- carries these rows is not a failed deploy.

INSERT INTO "products" ("id", "name", "description", "price_minor", "currency")
VALUES
  ('mbanga-coffee-1kg', 'Mbanga highland coffee, 1 kg', 'Washed arabica from the slopes above Mbanga, roasted the week it ships.', 7500, 'xaf'),
  ('douala-harbour-tee', 'Douala harbour T-shirt', 'Heavy cotton, screen-printed with the 1901 harbour plan.', 9000, 'xaf'),
  ('njangi-tote', 'Njangi tote bag', 'Woven raffia with a canvas lining, big enough for a market run.', 12000, 'xaf'),
  ('kribi-hammock', 'Kribi beach hammock', 'Cotton weave, two-person, with the tree straps.', 28000, 'xaf'),
  ('bamenda-stool', 'Bamenda carved stool', 'One piece of iroko, carved in the Northwest, oiled not lacquered.', 45000, 'xaf')
ON CONFLICT ("id") DO NOTHING;
