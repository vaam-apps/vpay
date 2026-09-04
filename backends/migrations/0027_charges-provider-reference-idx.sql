-- Step 8 lane C: the rail callback route needs to find a charge by the
-- reference a rail names, and that lookup is served to an UNAUTHENTICATED
-- caller.
--
-- `POST /provider/{code}/callback` (vpay_api::provider_callback) resolves
-- `CallbackRef::reference_id` to a charge through
-- `vpay_db::Charges::get_by_provider_reference`. Without this index that read
-- is a sequential scan over every charge the deployment has ever taken, and
-- anyone who can reach the callback URL can ask for one per request — which
-- is a denial-of-service surface that grows with the deployment's own
-- success. The rails sign nothing, so there is no credential to rate-limit
-- behind (docs/flows/adapter-mtn-momo.md, "the callback is unsigned and
-- unauthenticated"); the index is what keeps the read O(log n).
--
-- Leading with `provider_code` because that is how the route reads it: the
-- path segment says which rail is speaking, and scoping the lookup by it is
-- what stops a body posted to one rail's callback path from naming a charge
-- on another. It also keeps the index useful for the ordinary
-- "everything on this rail" operator query.
--
-- Deliberately NOT UNIQUE. Every insert path mints
-- `provider_reference_id` with `Uuid::new_v4()` before committing
-- (vpay_api::v1::payment_intents, docs/flows/crash-safety.md), so a
-- collision would be a vpay bug rather than anything a rail or a merchant
-- can cause — but "one charge per rail reference" is a schema-level
-- invariant this repository has not previously claimed, and adding it under
-- a route change would be deciding it rather than proposing it. See
-- docs/plans/step8-notes/lane-c.md.
CREATE INDEX charges_provider_reference_idx
    ON charges (provider_code, provider_reference_id);

COMMENT ON INDEX charges_provider_reference_idx IS
    'Serves vpay_db::Charges::get_by_provider_reference, the lookup behind the unauthenticated POST /provider/{code}/callback route. See docs/flows/reconciler.md.';
